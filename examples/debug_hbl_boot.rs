//! Single-steps past `RUN"HBL`+RET on HBL.DSK, watching for the point PC
//! lands in the ROM reset-vector range (0x0000-0x003F) - the signature
//! observed via screenshots (a full ROM boot-banner reprint) of whatever
//! sends execution back to a cold-boot-looking state within ~14 emulated
//! seconds, instead of the demo loading. See `project_hbl_boot_failure`
//! memory. Prints the offending previous PC, registers, a stack dump and
//! FDC status the instant it happens, plus periodic progress so a long
//! run can be watched.
//!
//! `cargo run --release --example debug_hbl_boot -- <HBL.DSK path>`

use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16};
use std::collections::VecDeque;

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_hbl_boot <HBL.DSK path>");
        std::process::exit(2);
    });

    let mut runner = CslRunner::new("/tmp/debug_hbl_boot_shots");
    runner.set_verbose(true);

    let script = format!(
        "csl_version 1.0\n\
         crtc_select 0\n\
         reset\n\
         disk_insert '{dsk}'\n\
         wait 3000000\n\
         key_delay 70000 70000 400000\n\
         key_output 'RUN\"HBL\\(RET)'\n\
         wait 2000000\n"
    );
    runner
        .run_text(&script, ".")
        .unwrap_or_else(|e| panic!("boot script failed: {e}"));

    eprintln!("boot script done, switching to raw single-stepping");

    let mb = runner.motherboard_mut();
    let mut last_pc = mb.pc();
    let mut last_motor = mb.fdc().motor();
    let mut steps: u64 = 0;
    let mut pc_history: VecDeque<u16> = VecDeque::with_capacity(64);
    const MAX_STEPS: u64 = 200_000_000;

    loop {
        mb.execute_next_opcode_no_video();
        steps += 1;
        let pc = mb.pc();

        let motor = mb.fdc().motor();
        if motor != last_motor {
            eprintln!("[step {steps}] FDC motor -> {motor} (PC now {pc:04X}, was entered from {last_pc:04X})");
            last_motor = motor;
        }

        if pc == 0x0000 {
            eprintln!("\n=== PC hit the cold-reset entry point: {last_pc:04X} -> 0000 at step {steps} ===");
            eprintln!(
                "AF={:04X} BC={:04X} DE={:04X} HL={:04X} IX={:04X} IY={:04X} SP={:04X}",
                mb.cpu().reg16(Reg16::Af).value(),
                mb.cpu().reg16(Reg16::Bc).value(),
                mb.cpu().reg16(Reg16::De).value(),
                mb.cpu().reg16(Reg16::Hl).value(),
                mb.cpu().reg16(Reg16::Ix).value(),
                mb.cpu().reg16(Reg16::Iy).value(),
                mb.cpu().reg16(Reg16::Sp).value(),
            );
            eprintln!("FDC motor={} ", mb.fdc().motor());
            let sp = mb.cpu().reg16(Reg16::Sp).value();
            eprint!("stack:");
            for i in 0..16u16 {
                let addr = sp.wrapping_add(i.wrapping_mul(2));
                eprint!(" {:04X}={:04X}", addr, mb.read_word(addr));
            }
            eprintln!();

            eprint!("memory 0xB630-0xB660:");
            for addr in 0xB630u16..=0xB660u16 {
                eprint!(" {:02X}", mb.read_byte(addr));
            }
            eprintln!();

            eprintln!("\ndisassembly leading up to the RST 0 (0x1D90-0x1DD4):");
            for insn in mb.disassemble_instructions_from(0x1D90, 40) {
                let addr = insn.address;
                eprintln!("{}", insn);
                if addr >= 0x1DD0 {
                    break;
                }
            }

            eprintln!(
                "\nrecent PC trace (last {} entries, oldest first):",
                pc_history.len()
            );
            for p in &pc_history {
                eprintln!("{p:04X}");
            }
            break;
        }
        last_pc = pc;
        pc_history.push_back(pc);
        if pc_history.len() > 64 {
            pc_history.pop_front();
        }

        if steps.is_multiple_of(10_000_000) {
            eprintln!("[step {steps}] PC={pc:04X} motor={motor}");
        }
        if steps >= MAX_STEPS {
            eprintln!("gave up after {MAX_STEPS} steps without entering the low vector range (PC={pc:04X})");
            break;
        }
    }
}
