use amsrust::motherboard::{MotherboardBuilder, Motherboard};
use amsrust::z80::{HasValue, Reg16, Reg8};
use amsrust::memory::*;
use amsrust::debug::*;
use std::cell::RefCell;
use std::io::Write;
use std::time::Instant;

/// `zexall` is the stricter sibling of `zexdoc.rs`'s exerciser: it also
/// checks the Z80's *undocumented* flag behaviour (bits 3/5 of F, the
/// undocumented half of the overflow/parity flag on a few opcodes), not
/// just the documented subset `zexdoc` covers. Same harness shape as
/// `zexdoc.rs` (CP/M BDOS calls 2/9 trapped, IM1 vector patched to
/// `EI:RET`, PC==0 means "test program exited") - diverges by capturing
/// the exerciser's own printed output and asserting on it directly
/// (`zexdoc.rs` only ever printed this for a human to read), since the
/// user explicitly asked to "ensure the zexall tests pass" rather than
/// just "run to completion".
#[test]
#[ignore = "hours: the full Z80 exerciser; tests/z80_reference.rs is the quick check"]
fn zexall() {
    let debug_every = std::env::var("ZEXALL_DEBUG_EVERY")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let max_operations = std::env::var("ZEXALL_MAX_OPS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());

    let mut motherboard = MotherboardBuilder::default().build();
    motherboard.force_rom_config(
        RomConfig{
            lower_rom: Flag::Disable,
            upper_rom: Flag::Disable,
            rom_number: 0
        }
    );

    let prg = include_bytes!("zexall.com");

    motherboard.write_buffer(0x100, prg);
    // zexall, like zexdoc, expects interrupts re-enabled from the IM1 handler.
    motherboard.write_buffer(0x38, cpclib_macros::assemble!(" ei : ret"));
    motherboard.write_byte(5, 0xc9); // need to return from cpm call
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);
    motherboard.write_word(6, 0xF606); // value on cpc - corresponds to the address jumped from 05

    assert_eq!(0xc9, motherboard.read_byte(0x5));

    let output = RefCell::new(String::new());

    // Handle the specificities of the test
    let mut handle = |motherboard: &mut Motherboard| {
        let cpu = motherboard.cpu();

        match cpu.reg16(Reg16::Pc).value() {
            0x0005 => {
                cpm_bdos(motherboard, &mut output.borrow_mut());
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
    let start = Instant::now();
    let mut operations = 0u64;

    loop {
        operations += 1;
        let stop = ctx.execute_next(&mut motherboard);
        if stop {
            let elapsed = start.elapsed();
            eprintln!("[zexall] finished after {} operations in {:.2?}", operations, elapsed);
            break;
        }

        if debug_every > 0 && operations % debug_every == 0 {
            let pc = motherboard.cpu().reg16(Reg16::Pc).value();
            eprintln!(
                "[zexall] ops={} pc=0x{:04x} next={}",
                operations,
                pc,
                motherboard.disassemble_next_instruction().to_string()
            );
        }

        if let Some(max_operations) = max_operations {
            if operations >= max_operations {
                panic!(
                    "[zexall] reached max operations ({}) at pc=0x{:04x} without the test program exiting",
                    max_operations,
                    motherboard.cpu().reg16(Reg16::Pc).value(),
                );
            }
        }
    }

    let output = output.into_inner();

    assert!(
        output.contains("Tests complete"),
        "zexall exited without printing its own completion banner - it likely crashed or looped instead of finishing"
    );
    assert!(
        !output.contains("ERROR"),
        "zexall reported at least one flag/behaviour mismatch - see the printed output above for which instruction(s)"
    );
}

// Code to execute when a call is made to CPM
//
// Prints each character as it arrives (not just accumulating into
// `output` for the final assertion) so a `--nocapture` run shows
// zexall's own per-test-case "  OK"/"  ERROR ..." lines live, in a long
// run's log - otherwise there's no way to tell how many test cases have
// actually passed until the whole multi-hour run finishes or hangs.
fn cpm_bdos(motherboard: &Motherboard, output: &mut String) {
    let cpu = motherboard.cpu();
    let c_value = cpu.reg8(Reg8::C).value();
    match c_value {
        2 => {
            // output character in register E
            let c = cpu.reg8(Reg8::E).value() as u8 as char;
            output.push(c);
            print!("{c}");
        },
        9 => {
            // output a string at register DE until '$'
            let mut addr = cpu.reg16(Reg16::De).value();
            loop {
                let c = motherboard.read_byte(addr) as char;
                addr = addr.wrapping_add(1);
                if c != '$' {
                    output.push(c);
                    print!("{c}");
                }
                else {
                    break;
                }
            }
        },
        _ => {
            panic!("Unknown CP/M call {}!", c_value);
        }
    }
    std::io::stdout().flush().unwrap();
}
