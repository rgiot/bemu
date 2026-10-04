//! Diagnoses why `RUN"HBL` (the Benediction demo `HBL.DSK`) ends up back
//! at a cold-boot-looking BASIC `Ready` prompt within a few emulated
//! seconds instead of playing the demo, instead of `debug_hbl_boot`'s
//! coarser "where does PC hit 0x0000" trap. Watches two small RAM buffers
//! (0xB635-0xB63E, whose checksum - computed at 0x1DD0 - deliberately
//! triggers a RST 0 if it equals 0xA4; and 0xB649-0xB652, the buffer it's
//! built from) for writes, and the PSG's bus-protocol state (register
//! select, port A input mode) each time the firmware's generic
//! read-row-via-PSG-register-14 helper (0x0890-0x08BC) is hit.
//!
//! Finding so far (see `project_hbl_boot_failure` memory for the full
//! writeup): the buffer ends up as ten identical 0xF6 bytes - the PSG's
//! "selected register" is 0 (or some other non-14 register) by the time
//! the read loop runs, not 14, so `Motherboard::sync_psg`'s "keep
//! register 14 live" guard never fires and the read returns stale,
//! unrelated PSG register content instead of real keyboard-line data.
//! `IFF1` is false the whole time (confirmed via this tool), ruling out
//! an interrupt landing mid-sequence - the likely cause is a general
//! main-loop/VSYNC pacing discrepancy (see
//! `project_crtc_timing_core_rewrite`), not a one-line bug.
//!
//! `cargo run --release --example debug_hbl_boot2 -- <HBL.DSK path>`

use amsrust::csl_runner::CslRunner;
use amsrust::z80::HasValue;
use amsrust::z80::Reg16;

const WATCH_START: u16 = 0xB635;
const WATCH_END: u16 = 0xB63E;
const SOURCE_START: u16 = 0xB649;
const SOURCE_END: u16 = 0xB652;
/// `REG_IO_PORT_A` in `motherboard.rs` - the PSG register that doubles as
/// the live keyboard-matrix line read-back.
const REG_IO_PORT_A: u8 = 14;

fn main() {
    let mut args = std::env::args().skip(1);
    let dsk = args.next().unwrap_or_else(|| {
        eprintln!("usage: debug_hbl_boot2 <HBL.DSK path>");
        std::process::exit(2);
    });

    let mut runner = CslRunner::new("/tmp/debug_hbl_boot2_shots");
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
    runner.run_text(&script, ".").unwrap_or_else(|e| panic!("boot script failed: {e}"));

    eprintln!("boot script done, watching the checksum buffer and PSG state while single-stepping");

    let mb = runner.motherboard_mut();
    let mut snapshot: Vec<u8> = (WATCH_START..=WATCH_END).map(|a| mb.read_byte(a)).collect();
    let mut source_snapshot: Vec<u8> = (SOURCE_START..=SOURCE_END).map(|a| mb.read_byte(a)).collect();
    eprintln!("initial: {:02X?} / source: {:02X?}", snapshot, source_snapshot);

    let mut steps: u64 = 0;
    const MAX_STEPS: u64 = 2_500_000;
    let mut last_pc = mb.pc();
    let mut im1_count: u64 = 0;

    loop {
        mb.execute_next_opcode_no_video();
        steps += 1;
        let pc = mb.pc();

        if pc == 0x0038 && last_pc != 0x0037 {
            im1_count += 1;
        }

        // Right after the IN A,(C) that reads back register 14 (or
        // whatever's actually selected) in the generic row-read helper.
        if last_pc == 0x08A3 && pc == 0x08A5 {
            let selected = mb.psg().selected_register();
            eprintln!(
                "[step {steps}] IN A,(C): A={:02X} PC_before=08A3 IFF1={} psg.selected_register={:?} (want Some({REG_IO_PORT_A})) port_a_is_input={} port_c={:02X}",
                mb.cpu().reg16(Reg16::Af).value() >> 8,
                mb.cpu().iff1(),
                selected,
                mb.psg().port_a_is_input(),
                mb.ppi().get_data_c_from_pins(),
            );
        }

        let mut changed = false;
        for (i, addr) in (WATCH_START..=WATCH_END).enumerate() {
            let v = mb.read_byte(addr);
            if v != snapshot[i] {
                eprintln!(
                    "[step {steps}] {addr:04X}: {:02X} -> {:02X}  (written at {last_pc:04X}) im1_count={im1_count}",
                    snapshot[i], v
                );
                snapshot[i] = v;
                changed = true;
            }
        }
        for (i, addr) in (SOURCE_START..=SOURCE_END).enumerate() {
            let v = mb.read_byte(addr);
            if v != source_snapshot[i] {
                eprintln!(
                    "[step {steps}] SOURCE {addr:04X}: {:02X} -> {:02X}  (written at {last_pc:04X}) im1_count={im1_count}",
                    source_snapshot[i], v
                );
                source_snapshot[i] = v;
                changed = true;
            }
        }
        let _ = changed;
        last_pc = pc;

        if steps >= MAX_STEPS {
            eprintln!(
                "stopping after {MAX_STEPS} steps (PC={pc:04X}); final snapshot: {:02X?} / source: {:02X?} / total im1 interrupts: {im1_count}",
                snapshot, source_snapshot
            );
            break;
        }
    }
}
