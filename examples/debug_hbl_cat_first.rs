//! Reproduces the user's own exact repro path (not the earlier, apparently
//! incomplete `debug_hbl_boot`/`debug_hbl_space` one): `CAT` first to show
//! the catart, *then* `RUN"HBL`+RET - confirmed by hand to actually reach
//! the demo's own code and reboot "during the amsdos leaving part"
//! (`~/Perso/CPC/current_projects/demo.bnd5/linking/src/hbl_inner.asm`'s
//! `.execute_leave_amsdos`/`.kill_system`, which replaces the IM1 vector
//! at 0x0038 with a bare EI:RET and takes over the machine). Traces PC
//! continuously and stops the instant it hits 0x0000 (a real RST 0 /
//! reset, not an ordinary IM1 interrupt at 0x0038), printing full state
//! and a disassembly of what led there. See `project_hbl_boot_failure`
//! memory.
//!
//! `cargo run --release --example debug_hbl_cat_first -- <HBL.DSK path>`

use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16};

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_hbl_cat_first <HBL.DSK path>");
        std::process::exit(2);
    });

    let mut runner = CslRunner::new("/tmp/debug_hbl_cat_first_shots");
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
         key_output 'RUN\"HBL\\(RET)'\n\
         wait 3000000\n"
    );
    runner.run_text(&boot, ".").unwrap_or_else(|e| panic!("boot failed: {e}"));

    eprintln!("boot script done (PC={:04X}), switching to raw single-stepping", runner.motherboard().pc());

    let mb = runner.motherboard_mut();
    let mut last_pc = mb.pc();
    let mut steps: u64 = 0;
    const MAX_STEPS: u64 = 200_000_000;

    loop {
        mb.execute_next_opcode_no_video();
        steps += 1;
        let pc = mb.pc();

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
            let sp = mb.cpu().reg16(Reg16::Sp).value();
            eprint!("stack:");
            for i in 0..16u16 {
                let addr = sp.wrapping_add(i.wrapping_mul(2));
                eprint!(" {:04X}={:04X}", addr, mb.read_word(addr));
            }
            eprintln!();
            eprintln!("\ndisassembly leading up to it ({:04X}-{:04X}):", last_pc.saturating_sub(32), last_pc);
            for insn in mb.disassemble_instructions_from(last_pc.saturating_sub(32), 20) {
                eprintln!("{}", insn);
            }
            break;
        }
        last_pc = pc;

        if steps.is_multiple_of(10_000_000) {
            eprintln!("[step {steps}] PC={pc:04X}");
        }
        if steps >= MAX_STEPS {
            eprintln!("gave up after {MAX_STEPS} steps without hitting 0000 (PC={pc:04X})");
            break;
        }
    }
}
