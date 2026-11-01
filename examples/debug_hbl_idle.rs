//! Disassembles around the PC addresses `debug_hbl_space` observed HBL.DSK
//! sitting at well after boot, to understand what it's actually doing
//! (genuinely blocking on a keypress vs. something else) - see
//! `project_hbl_boot_failure` memory.
//!
//! `cargo run --release --example debug_hbl_idle -- <HBL.DSK path>`

use amsrust::csl_runner::CslRunner;

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_hbl_idle <HBL.DSK path>");
        std::process::exit(2);
    });

    let mut runner = CslRunner::new("/tmp/debug_hbl_idle_shots");
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
    runner.run_text(&boot, ".").unwrap_or_else(|e| panic!("boot failed: {e}"));

    let mb = runner.motherboard();
    let pc = mb.pc();
    eprintln!("PC={pc:04X}");
    eprintln!("disassembly from {:04X}:", pc.saturating_sub(16));
    for insn in mb.disassemble_instructions_from(pc.saturating_sub(16), 20) {
        eprintln!("{}", insn);
    }
}
