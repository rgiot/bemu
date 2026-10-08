//! Generic DSK repro tool for the BLIGHT demo (Benediction,
//! https://github.com/rgiot/demo.bnd5.blight): boots, types `CAT` then
//! `RUN"BLIGHT`+RET, then single-steps watching for a cold reset
//! (PC hits 0x0000, the same crash signature chased in `debug_hbl_cat_first`)
//! or a wedged PC (same address for a very long time). Prints progress
//! periodically so a run can be judged "still running fine" vs "stuck"
//! without waiting for the full step budget.
//!
//! `cargo run --release --example debug_blight -- <BLIGHT.DSK path>`

use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16};

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_blight <BLIGHT.DSK path>");
        std::process::exit(2);
    });

    let mut runner = CslRunner::new("/tmp/debug_blight_shots");
    runner.set_verbose(true);

    let boot = format!(
        "csl_version 1.0\n\
         crtc_select 0\n\
         reset\n\
         disk_insert '{dsk}'\n\
         wait 3000000\n\
         key_delay 70000 70000 400000\n\
         key_output 'CAT\\(RET)'\n\
         wait 6000000\n\
         key_output 'RUN\"BLIGHT\\(RET)'\n\
         wait 3000000\n"
    );
    runner.run_text(&boot, ".").unwrap_or_else(|e| panic!("boot failed: {e}"));

    eprintln!("boot script done (PC={:04X}), switching to raw single-stepping", runner.motherboard().pc());

    let mut last_pc = runner.motherboard().pc();
    let mut steps: u64 = 0;
    let mut same_pc_count: u64 = 0;
    const MAX_STEPS: u64 = 300_000_000;
    const PRINT_EVERY: u64 = 10_000_000;
    const STUCK_THRESHOLD: u64 = 50_000_000;
    // Every PRINT_EVERY steps, also drop a screenshot so a run can be
    // judged visually, not just "PC kept moving".
    const SHOT_EVERY: u64 = 50_000_000;

    loop {
        let mb = runner.motherboard_mut();
        mb.execute_next_opcode_no_video();
        steps += 1;
        let pc = mb.pc();

        if pc == 0x0000 && last_pc != 0x0000 {
            eprintln!("\n=== PC hit the cold-reset entry point: {last_pc:04X} -> 0000 at step {steps} ===");
            print_state(mb);
            eprintln!("\ndisassembly leading up to it ({:04X}-{:04X}):", last_pc.saturating_sub(32), last_pc);
            for insn in mb.disassemble_instructions_from(last_pc.saturating_sub(32), 20) {
                eprintln!("{}", insn.to_string());
            }
            break;
        }

        if pc == last_pc {
            same_pc_count += 1;
            if same_pc_count == STUCK_THRESHOLD {
                eprintln!("\n=== PC has not moved from {pc:04X} for {STUCK_THRESHOLD} steps (likely wedged) at step {steps} ===");
                print_state(mb);
                break;
            }
        } else {
            same_pc_count = 0;
        }

        last_pc = pc;

        if steps % PRINT_EVERY == 0 {
            eprintln!("[step {steps}] PC={pc:04X}");
        }
        if steps % SHOT_EVERY == 0 {
            let name = format!("step_{steps:010}");
            if let Err(e) = runner.run_text(&format!("screenshot_name '{name}'\nscreenshot\n"), ".") {
                eprintln!("(screenshot at step {steps} failed: {e})");
            }
        }

        if steps >= MAX_STEPS {
            eprintln!("\n=== reached MAX_STEPS ({MAX_STEPS}) without a cold reset or getting stuck ===");
            print_state(runner.motherboard());
            break;
        }
    }

    let name = format!("final_{steps:010}");
    if let Err(e) = runner.run_text(&format!("screenshot_name '{name}'\nscreenshot\n"), ".") {
        eprintln!("(final screenshot failed: {e})");
    }
}

fn print_state(mb: &amsrust::motherboard::Motherboard) {
    eprintln!(
        "AF={:04X} BC={:04X} DE={:04X} HL={:04X} IX={:04X} IY={:04X} SP={:04X} PC={:04X}",
        mb.cpu().reg16(Reg16::Af).value(),
        mb.cpu().reg16(Reg16::Bc).value(),
        mb.cpu().reg16(Reg16::De).value(),
        mb.cpu().reg16(Reg16::Hl).value(),
        mb.cpu().reg16(Reg16::Ix).value(),
        mb.cpu().reg16(Reg16::Iy).value(),
        mb.cpu().reg16(Reg16::Sp).value(),
        mb.pc(),
    );
    let sp = mb.cpu().reg16(Reg16::Sp).value();
    eprint!("stack:");
    for i in 0..16u16 {
        let addr = sp.wrapping_add(i.wrapping_mul(2));
        eprint!(" {:04X}={:04X}", addr, mb.read_word(addr));
    }
    eprintln!();
}
