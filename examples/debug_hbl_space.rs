//! Checks whether a `key_output ' '` sent well after HBL.DSK's boot (once
//! it's reached the idle "wait for keypress" OS ROM loop, PC=0x1BC2 - see
//! `project_hbl_boot_failure` memory) actually gets delivered and does
//! anything, by watching PC and the keyboard matrix directly around the
//! moment the space is scheduled.
//!
//! `cargo run --release --example debug_hbl_space -- <HBL.DSK path>`

use amsrust::csl_runner::CslRunner;
use amsrust::keyboard::CpcKey;

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_hbl_space <HBL.DSK path>");
        std::process::exit(2);
    });

    let mut runner = CslRunner::new("/tmp/debug_hbl_space_shots");
    runner.set_verbose(true);

    let boot = format!(
        "csl_version 1.0\n\
         crtc_select 0\n\
         reset\n\
         disk_insert '{dsk}'\n\
         wait 3000000\n\
         key_delay 70000 70000 400000\n\
         key_output 'RUN\"HBL\\(RET)'\n\
         wait 60000000\n"
    );
    runner
        .run_text(&boot, ".")
        .unwrap_or_else(|e| panic!("boot failed: {e}"));

    eprintln!("after boot+60s wait: PC={:04X}", runner.motherboard().pc());
    eprintln!(
        "space pressed? {}",
        runner.motherboard().keyboard().is_pressed(CpcKey::Space)
    );

    // Now send the space and watch closely in small increments.
    for i in 0..10 {
        let step = if i == 0 {
            "key_output ' '\nwait 100000\n".to_owned()
        } else {
            "wait 100000\n".to_owned()
        };
        runner
            .run_text(&step, ".")
            .unwrap_or_else(|e| panic!("step failed: {e}"));
        eprintln!(
            "[+{}ms] PC={:04X} space_pressed={} keyboard_line5={:02X}",
            (i + 1) * 100,
            runner.motherboard().pc(),
            runner.motherboard().keyboard().is_pressed(CpcKey::Space),
            runner.motherboard().keyboard().line(5),
        );
    }
}
