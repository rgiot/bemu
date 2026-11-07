//! Diagnostic companion to `debug_blight`: boots BLIGHT.DSK the same way,
//! runs for a fixed, shorter step budget (enough to land well past the
//! loader's screen decompression), then dumps the Gate Array's video mode,
//! the CRTC's screen-start registers (R12/R13) and a raw hex sample of
//! video memory - to tell whether the noisy screenshots `debug_blight`
//! produces come from genuinely random *source* bytes (a corruption bug
//! upstream) or from structured bytes being mis-decoded for display (a
//! rendering/mode bug).
//!
//! `cargo run --release --example debug_blight_screen -- <BLIGHT.DSK path> [steps]`

use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16};

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_blight_screen <BLIGHT.DSK path> [steps]");
        std::process::exit(2);
    });
    let steps: u64 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60_000_000);

    let mut runner = CslRunner::new("/tmp/debug_blight_screen_shots");
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
    runner
        .run_text(&boot, ".")
        .unwrap_or_else(|e| panic!("boot failed: {e}"));

    eprintln!(
        "boot script done (PC={:04X}), stepping {steps} instructions",
        runner.motherboard().pc()
    );

    for i in 0..steps {
        runner.motherboard_mut().execute_next_opcode_no_video();
        if i % 10_000_000 == 0 {
            eprintln!("[step {i}] PC={:04X}", runner.motherboard().pc());
        }
    }

    let mb = runner.motherboard();
    eprintln!(
        "\nPC={:04X} AF={:04X} BC={:04X} DE={:04X} HL={:04X}",
        mb.pc(),
        mb.cpu().reg16(Reg16::Af).value(),
        mb.cpu().reg16(Reg16::Bc).value(),
        mb.cpu().reg16(Reg16::De).value(),
        mb.cpu().reg16(Reg16::Hl).value(),
    );
    eprintln!("GA mode: {:?}", mb.ga().current_mode());
    eprintln!(
        "CRTC R12/R13 (screen start address high/low): {:02X}/{:02X} -> base {:04X}",
        mb.crtc().r12(),
        mb.crtc().r13(),
        (mb.crtc().r12() as u16) << 8 | mb.crtc().r13() as u16
    );

    eprintln!("\nFirst 256 bytes at 0xC000 (loader's documented decompress target):");
    dump(mb, 0xC000);

    eprintln!("\nFirst 256 bytes at 0x4000 (lower standard screen base, in case GA ram/screen mapping differs):");
    dump(mb, 0x4000);

    let mut shot_runner = runner;
    if let Err(e) = shot_runner.run_text("screenshot_name 'diag'\nscreenshot\n", ".") {
        eprintln!("(screenshot failed: {e})");
    }
}

fn dump(mb: &amsrust::motherboard::Motherboard, base: u16) {
    for row in 0..16u16 {
        let addr = base.wrapping_add(row * 16);
        eprint!("{addr:04X}:");
        for col in 0..16u16 {
            eprint!(" {:02X}", mb.read_byte(addr.wrapping_add(col)));
        }
        eprintln!();
    }
}
