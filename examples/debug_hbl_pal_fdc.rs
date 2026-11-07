//! Traces PAL/MMR RAM-config switches and FDC activity throughout the
//! HBL.DSK CAT+RUN"HBL repro, to correlate them with the stack-corruption
//! crash at step ~10994876 (RET popping 0x0000 from SP=0xB1F0) - the user
//! suspects either a RAM config switch (0xC1/0xC2/0xC3) or an FDC read
//! issue. See `project_hbl_boot_failure` memory.
//!
//! `cargo run --release --example debug_hbl_pal_fdc -- <HBL.DSK path>`

use amsrust::csl_runner::CslRunner;
use amsrust::memory::RamConfig;
use amsrust::z80::{HasValue, Reg16};

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_hbl_pal_fdc <HBL.DSK path>");
        std::process::exit(2);
    });

    let mut runner = CslRunner::new("/tmp/debug_hbl_pal_fdc_shots");
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
    runner
        .run_text(&boot, ".")
        .unwrap_or_else(|e| panic!("boot failed: {e}"));

    eprintln!("boot script done (PC={:04X})", runner.motherboard().pc());

    let mb = runner.motherboard_mut();
    let mut steps: u64 = 0;
    const CRASH_STEP: u64 = 10_994_876;

    let mut last_ram_config: Option<RamConfig> = None;
    let mut last_motor: Option<bool> = None;
    let mut mmr_switch_count: u64 = 0;
    let mut last_mmr_switch_step: u64 = 0;
    let mut motor_on_count: u64 = 0;

    loop {
        let ram_config = mb.memory_config().1;
        if last_ram_config != Some(ram_config) {
            mmr_switch_count += 1;
            let gap = steps - last_mmr_switch_step;
            if steps + 2_000_000 > CRASH_STEP && steps < CRASH_STEP + 10 {
                eprintln!(
                    "[step {steps}] RAM config changed: {:?} -> {:?} (gap since last switch: {gap} steps)",
                    last_ram_config,
                    ram_config
                );
            }
            last_ram_config = Some(ram_config);
            last_mmr_switch_step = steps;
        }

        let motor = mb.fdc().motor();
        if last_motor != Some(motor) {
            if motor {
                motor_on_count += 1;
            }
            if steps + 2_000_000 > CRASH_STEP && steps < CRASH_STEP + 10 {
                eprintln!("[step {steps}] FDC motor -> {motor}");
            }
            last_motor = Some(motor);
        }

        if steps == CRASH_STEP.saturating_sub(2_000_000) {
            eprintln!(
                "--- entering final 2M-step window before the known crash point (mmr_switches_so_far={mmr_switch_count}, motor_on_count_so_far={motor_on_count}) ---"
            );
        }

        mb.execute_next_opcode_no_video();
        steps += 1;

        if steps > CRASH_STEP + 5 {
            eprintln!(
                "\nstopped at step {steps}, PC={:04X}, total mmr switches={mmr_switch_count}, total motor-on events={motor_on_count}",
                mb.pc()
            );
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
    }
}
