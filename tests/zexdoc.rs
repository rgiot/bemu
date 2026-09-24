use amsrust::motherboard::{MotherboardBuilder, Motherboard};
use amsrust::z80::{HasValue, Reg16, Reg8};
use amsrust::memory::*;
use amsrust::debug::*;
use std::fs::File;
use std::io;
use std::io::{BufWriter, Write};
use std::time::Instant;
/// Took inspiration from https://floooh.github.io/2016/07/12/z80-rust-ms1.html
#[test]
fn zexdoc() {
    let interactive = std::env::var("ZEXDOC_INTERACTIVE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    let debug_every = std::env::var("ZEXDOC_DEBUG_EVERY")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let max_operations = std::env::var("ZEXDOC_MAX_OPS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    let trace_file = std::env::var("ZEXDOC_TRACE_FILE").ok();

    let mut motherboard = MotherboardBuilder::default().build();
    motherboard.force_rom_config(
        RomConfig{
            lower_rom: Flag::Disable,
            upper_rom: Flag::Disable,
            rom_number: 0
        }
    );
    
    
  // let prg = include_bytes!("CPUTEST.COM");
   let prg = include_bytes!("zexdoc.com");
   //let prg = include_bytes!("zexall.com");


    motherboard.write_buffer(0x100, prg);
    // ZEXDOC harness expects interrupts to be re-enabled from the IM1 handler.
    motherboard.write_buffer(0x38, cpclib_macros::assemble!(" ei : ret"));
    motherboard.write_byte(5, 0xc9); // need to return from cpm call
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);
   // motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0xF000);
    motherboard.write_word(6, 0xF606); //value on cpc - correspond to the adress jumped from 05


    assert_eq!(
        0xc9,
        motherboard.read_byte(0x5)
    );


    // Handle the specificities of the test
    let mut handle = |motherboard: &mut Motherboard| {

        let cpu = motherboard.cpu();

         match cpu.reg16(Reg16::Pc).value() {
            0x0005 => { 
                cpm_bdos(&motherboard); 
                false
            },
            0x0000 => { 
                println!("leave test");
                true
            },
            _ => false,
        }
    };

    let mut ctx = MonitorContext::new(Box::new(&mut handle));
    if interactive {
        eprintln!("[zexdoc] interactive mode enabled");
        Monitor::cli_monitoring(&mut motherboard, &mut ctx);
    }
    else {
        let start = Instant::now();
        let mut operations = 0u64;
        let mut iut_executions = 0u64;

        // In zexdoc.com, the instruction-under-test trampoline label `iut` resolves to 0x1d42.
        const IUT_ADDR: u16 = 0x1d42;
        // zexdoc source comment for first descriptor (<adc,sbc> hl,<bc,de,hl,sp>): 38,912 iterations.
        const FIRST_TEST_IUT_TARGET: u64 = 38_912;
        // Approximate total test-case iterations from cycle comments in tests/zexdoc.src.
        // This is a much better long-run indicator than scaling against only the first descriptor.
        const TOTAL_TEST_IUT_TARGET_ESTIMATE: u64 = 1_804_651;
        let mut last_debug_ops = 0u64;
        let mut last_debug_iut = 0u64;
        let mut last_traced_pc: Option<u16> = None;
        let mut last_traced_inst: Option<String> = None;
        let mut trace_writer = trace_file.as_ref().map(|path| {
            let file = File::create(path)
                .unwrap_or_else(|e| panic!("[zexdoc] cannot create trace file '{}': {}", path, e));
            let mut writer = BufWriter::new(file);
            writeln!(
                writer,
                "ops\tpc\tbytes\tinst\taf\tbc\tde\thl\tix\tiy\tsp"
            )
            .expect("[zexdoc] cannot write trace header");
            writer
        });

        loop {
            if let Some(writer) = trace_writer.as_mut() {
                let cpu = motherboard.cpu();
                let pc = cpu.reg16(Reg16::Pc).value();
                let af = cpu.reg16(Reg16::Af).value();
                let bc = cpu.reg16(Reg16::Bc).value();
                let de = cpu.reg16(Reg16::De).value();
                let hl = cpu.reg16(Reg16::Hl).value();
                let ix = cpu.reg16(Reg16::Ix).value();
                let iy = cpu.reg16(Reg16::Iy).value();
                let sp = cpu.reg16(Reg16::Sp).value();
                let next = motherboard.disassemble_next_instruction();
                let inst = next.token.to_string();
                let suppress_repeated = matches!(
                    inst.as_str(),
                    "LDIR" | "LDDR" | "HALT" | "OTDR" | "OTIR"
                ) && last_traced_pc == Some(pc)
                    && last_traced_inst.as_deref() == Some(inst.as_str());

                if !suppress_repeated {
                let b0 = next.memory.get(0).copied();
                let b1 = next.memory.get(1).copied();
                let b2 = next.memory.get(2).copied();
                let bytes = format!(
                    "{} {} {}",
                    b0.map(|b| format!("{:02x}", b)).unwrap_or_else(|| "--".to_string()),
                    b1.map(|b| format!("{:02x}", b)).unwrap_or_else(|| "--".to_string()),
                    b2.map(|b| format!("{:02x}", b)).unwrap_or_else(|| "--".to_string())
                );

                writeln!(
                    writer,
                    "{}\t0x{:04x}\t{}\t{}\t0x{:04x}\t0x{:04x}\t0x{:04x}\t0x{:04x}\t0x{:04x}\t0x{:04x}\t0x{:04x}",
                    operations,
                    pc,
                    bytes,
                    inst,
                    af,
                    bc,
                    de,
                    hl,
                    ix,
                    iy,
                    sp,
                ).expect("[zexdoc] cannot write instruction trace line");
                }

                last_traced_pc = Some(pc);
                last_traced_inst = Some(inst);
            }

            if motherboard.cpu().reg16(Reg16::Pc).value() == IUT_ADDR {
                iut_executions += 1;
            }

            operations += 1;
            let stop = ctx.execute_next(&mut motherboard);
            if stop {
                let elapsed = start.elapsed();
                if let Some(writer) = trace_writer.as_mut() {
                    writer.flush().expect("[zexdoc] cannot flush instruction trace file");
                }
                eprintln!(
                    "[zexdoc] finished after {} operations (iut_executions={}) in {:.2?}",
                    operations,
                    iut_executions,
                    elapsed
                );
                break;
            }

            if debug_every > 0 && operations % debug_every == 0 {
                let pc = motherboard.cpu().reg16(Reg16::Pc).value();
                let af = motherboard.cpu().reg16(Reg16::Af).value();
                let bc = motherboard.cpu().reg16(Reg16::Bc).value();
                let de = motherboard.cpu().reg16(Reg16::De).value();
                let hl = motherboard.cpu().reg16(Reg16::Hl).value();
                let first_test_equivalent = (iut_executions as f64 / FIRST_TEST_IUT_TARGET as f64) * 100.0;
                let global_progress_estimate = (iut_executions as f64 / TOTAL_TEST_IUT_TARGET_ESTIMATE as f64) * 100.0;
                let delta_ops = operations - last_debug_ops;
                let delta_iut = iut_executions - last_debug_iut;
                let iut_per_mop = if delta_ops > 0 {
                    (delta_iut as f64) * 1_000_000.0 / (delta_ops as f64)
                } else {
                    0.0
                };
                eprintln!(
                    "[zexdoc] ops={} iut_executions={} global_est={:.2}% first_equiv={:.2}% delta_iut={} iut_per_mop={:.1} pc=0x{:04x} af=0x{:04x} bc=0x{:04x} de=0x{:04x} hl=0x{:04x} next={}",
                    operations,
                    iut_executions,
                    global_progress_estimate,
                    first_test_equivalent,
                    delta_iut,
                    iut_per_mop,
                    pc,
                    af,
                    bc,
                    de,
                    hl,
                    motherboard.disassemble_next_instruction().to_string()
                );
                last_debug_ops = operations;
                last_debug_iut = iut_executions;
            }

            if let Some(max_operations) = max_operations {
                if operations >= max_operations {
                    panic!(
                        "[zexdoc] reached max operations ({}) at pc=0x{:04x} after iut_executions={} (global_est={:.2}%, first_equiv={:.2}%); note: zexdoc source 'cycles' are test-case iterations, not CPU ops",
                        max_operations,
                        motherboard.cpu().reg16(Reg16::Pc).value(),
                        iut_executions,
                        (iut_executions as f64 / TOTAL_TEST_IUT_TARGET_ESTIMATE as f64) * 100.0,
                        (iut_executions as f64 / FIRST_TEST_IUT_TARGET as f64) * 100.0
                    );
                }
            }
        }
    }
}

// Code to execute when a call is made to CPM
fn cpm_bdos(motherboard: & Motherboard) {
    let cpu = motherboard.cpu();
    let c_value = cpu.reg8(Reg8::C).value();
    match  c_value {
        2 => {
            // output character in register E
            print!("{}", cpu.reg8(Reg8::E).value() as u8 as char);
            io::stdout().flush().unwrap();
        },
        9 => {
            // output a string at register DE until '$'
            let mut addr = cpu.reg16(Reg16::De).value();
            loop {
                let c = motherboard.read_byte(addr) as char;
                addr = addr.wrapping_add(1);
                if c != '$' {
                    print!("{}", c);
                }
                else {
                    break;
                }
                io::stdout().flush().unwrap();
            }
        },
        _ => {
            panic!("Unknown CP/M call {}!", c_value);
        }
    }
}