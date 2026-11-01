//! Raw tick-by-tick trace of the CPU->PAL pin flow (A14/A15/data/IOWR)
//! around the rapid RAM-config switches found at instruction-steps
//! ~10975548-10987934 by debug_hbl_pal_fdc.rs, to check whether the byte/
//! pulse the PAL actually sees matches what the CPU intends - not just
//! whether the final decode table is right. See `project_hbl_boot_failure`
//! memory.
//!
//! Two different units matter here: `debug_hbl_pal_fdc.rs` counts
//! *instructions* (`execute_next_opcode_no_video()` calls); this example
//! steps by instruction up to the window, then switches to raw
//! `Motherboard::tick()` calls (the GA's own 16MHz-equivalent granularity)
//! for fine detail *inside* the window, since a single OUT instruction's
//! write phase spans several ticks and that's exactly the resolution
//! needed to see any pin-flow timing issue.
//!
//! `cargo run --release --example debug_hbl_pal_pinflow -- <HBL.DSK path>`

use amsrust::chip::{Chip, ReadDataChip};
use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16, Z80Pin};

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_hbl_pal_pinflow <HBL.DSK path>");
        std::process::exit(2);
    });

    let mut runner = CslRunner::new("/tmp/debug_hbl_pal_pinflow_shots");
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

    let mb = runner.motherboard_mut();

    // Phase 1: step by instruction up to just before the window (fast).
    const INSTR_WINDOW_START: u64 = 10_975_400;
    let mut instr_steps: u64 = 0;
    while instr_steps < INSTR_WINDOW_START {
        mb.execute_next_opcode_no_video();
        instr_steps += 1;
    }
    eprintln!("reached instruction-step {instr_steps}, PC={:04X} - switching to raw tick tracing", mb.pc());

    // Phase 2: raw tick-level tracing through the window of interest,
    // covering well past the last observed config change (instr-step
    // 10987934) and the eventual crash (10994876) - generous raw-tick
    // budget since each instruction spans several ticks.
    const RAW_TICKS: u64 = 300_000_000;
    let mut ticks: u64 = 0;
    let mut last_a = (false, false);
    let mut last_iowr_active = false;
    let mut last_ram_config = mb.memory_config().1;
    let mut last_pc = mb.pc();

    loop {
        let pc = mb.pc();
        let a14 = mb.cpu().get_pin(Z80Pin::A14);
        let a15 = mb.cpu().get_pin(Z80Pin::A15);
        let iowr_active = !mb.cpu().get_pin(Z80Pin::Neg_WR) || !mb.cpu().get_pin(Z80Pin::Neg_IORQ);
        let data = mb.cpu().get_data_from_pins();

        if (a14, a15) != last_a || iowr_active != last_iowr_active {
            // Only print transitions where address lines look like a
            // PAL-addressed access (A15=0,A14=1) - the interesting ones -
            // to avoid flooding the log over a 300M-tick run.
            if !a15 && a14 {
                eprintln!(
                    "[tick {ticks}] PC={pc:04X} A14={a14} A15={a15} IOWR_active={iowr_active} data={data:02X}"
                );
            }
            last_a = (a14, a15);
            last_iowr_active = iowr_active;
            last_pc = pc;
        }
        let _ = last_pc;

        let ram_config = mb.memory_config().1;
        if ram_config != last_ram_config {
            eprintln!("[tick {ticks}]     *** RAM config -> {ram_config:?} ***");
            last_ram_config = ram_config;
        }

        if pc == 0x0000 && last_pc != 0x0000 {
            eprintln!("\n*** PC hit 0x0000 (the known crash) at tick {ticks} - last PC was {last_pc:04X} ***");
            eprintln!(
                "AF={:04X} BC={:04X} DE={:04X} HL={:04X} SP={:04X}",
                mb.cpu().reg16(Reg16::Af).value(),
                mb.cpu().reg16(Reg16::Bc).value(),
                mb.cpu().reg16(Reg16::De).value(),
                mb.cpu().reg16(Reg16::Hl).value(),
                mb.cpu().reg16(Reg16::Sp).value(),
            );
            break;
        }

        mb.tick();
        ticks += 1;

        if ticks.is_multiple_of(50_000_000) {
            eprintln!("[progress] tick {ticks}, PC={:04X}", mb.pc());
        }

        if ticks > RAW_TICKS {
            eprintln!("stopped after {ticks} raw ticks, PC={:04X}", mb.pc());
            break;
        }
    }

    eprintln!(
        "\nfinal AF={:04X} SP={:04X} PC={:04X}",
        mb.cpu().reg16(Reg16::Af).value(),
        mb.cpu().reg16(Reg16::Sp).value(),
        mb.pc(),
    );
}
