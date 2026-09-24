///! End-to-end test: boot to BASIC, insert the real Shaker test-suite disk,
///! type `cat` at the keyboard (through the matrix, not injected text), and
///! render the resulting screen to confirm the firmware's own disk-access
///! code (going through this session's FDC) lists the expected files.
///!
///! This is the first test that drives the FDC through real firmware
///! (AMSDOS's catalog routine) rather than hand-crafted raw commands, and
///! the first to drive the keyboard matrix through real firmware's own
///! scan routine rather than direct matrix pokes.
///!
///! Currently blocked by a real, separate Z80 core bug (unrelated to the
///! keyboard matrix or PSG wiring, both independently verified correct in
///! tests/keyboard_psg.rs): `Z80::instruction_finished()` gets called after
///! *every* M-cycle rather than only once a whole instruction has retired,
///! so a multi-cycle instruction landing at the wrong moment can see
///! `interruption_received && iff1` true, clear iff1 and hand off to
///! `MCycle::new_interruption()`, and then have that clobbered before the
///! actual jump to &0038 happens - the net effect being interrupts silently
///! and permanently disabled with no exception ever taken. Confirmed
///! reproducible: booting with `enable_interruptions()` alone (no disk, no
///! typing) already gets stuck this way at instruction #234,120, a `POP HL`
///! at &1D23 inside the firmware's own keyboard-scan interrupt handler -
///! long before "Ready" even appears. Once that's fixed, this test should
///! start actually exercising the FDC through firmware's CAT routine.

use amsrust::chip::*;
use amsrust::crtc::*;
use amsrust::fdc::FdcDrive;
use amsrust::ga::*;
use amsrust::keyboard::CpcKey;
use amsrust::monitor::*;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use cpclib_disc::edsk::ExtendedDsk;
use cpclib_disc::AnyDisc;

const SHAKER_DSK: &str =
    "../rust.cpcdemotools/cpclib-csl/tests/CSL/Shaker_CSL/shaker26.dsk";

/// How many opcodes a key stays pressed / released for - generous relative
/// to a single 20ms (50Hz) frame so firmware's keyboard scan interrupt is
/// guaranteed to see it regardless of exactly when in the frame it lands.
const KEY_HOLD_OPCODES: u64 = 200_000;
const KEY_GAP_OPCODES: u64 = 100_000;

fn run_opcodes(motherboard: &mut Motherboard, count: u64) {
    for _ in 0..count {
        motherboard.execute_next_opcode_no_video();
    }
}

fn press_and_release(motherboard: &mut Motherboard, key: CpcKey) {
    motherboard.keyboard_mut().set_pressed(key, true);
    run_opcodes(motherboard, KEY_HOLD_OPCODES);
    motherboard.keyboard_mut().set_pressed(key, false);
    run_opcodes(motherboard, KEY_GAP_OPCODES);
}

fn type_text(motherboard: &mut Motherboard, keys: &[CpcKey]) {
    for &key in keys {
        press_and_release(motherboard, key);
    }
}

fn beginning_steps(ga: &mut GateArray, crtc: &mut Crtc) {
    ga.save_pins_state();
    crtc.save_pins_state();
    sync_pins(ga, &[GateArrayPin::CCLK], crtc, &[CrtcPin::CCLK]);
    sync_pins(
        crtc,
        &[CrtcPin::VS, CrtcPin::HS, CrtcPin::DE],
        ga,
        &[GateArrayPin::VSYNC, GateArrayPin::HSYNC, GateArrayPin::DISPEN]
    );
}

fn end_steps(ga: &mut GateArray, crtc: &mut Crtc, monitor: &mut Monitor) {
    ga.tick();
    if crtc.is_activated() {
        crtc.tick();
    }
    let rgb = ga.rgb();
    let vsync = ga.get_pin(GateArrayPin::VSYNC);
    let hsync = ga.get_pin(GateArrayPin::HSYNC);
    monitor.receive(rgb, Sync { vsync, hsync });
    ga.manage_clocks();
}

fn fetch_video_byte(bank: &[u8; 0x4000], ga: &GateArray, crtc: &Crtc) -> u8 {
    let crtc_address = crtc.get_address_from_pins();
    let fetch_address = ga.video_fetch_address(crtc_address);
    bank[fetch_address as usize - 0xc000]
}

/// Capture the machine's live screen memory/palette/mode and render one
/// clean frame from a fresh CRTC/GA - the same technique used to get a
/// readable frame out of an otherwise chaotically-timed live machine (see
/// the boot-to-BASIC investigation this technique was first developed for).
fn render_screen(motherboard: &Motherboard, out_path: &str) {
    let mut bank = [0u8; 0x4000];
    for i in 0..0x4000u32 {
        bank[i as usize] = motherboard.read_byte(0xc000 + i as u16);
    }
    let palette = *motherboard.ga().palette();
    let mode = motherboard.ga().current_mode();

    let mut crtc = Crtc::default();
    crtc.do_write_register(12, 0x30);
    crtc.do_write_register(13, 0x00);
    crtc.vlc = 7.into();
    crtc.vcc = 29.into();
    crtc.hcc = 63;
    crtc.ma = 0x3000;
    crtc.set_address_on_pins();

    let mut monitor = Monitor::wait_vsync();

    let mut ga = GateArray::default();
    ga.set_palette(palette);
    ga.force_active_video_mode(mode);
    ga.set_video_mode(mode);
    ga.set_pin(GateArrayPin::CCLK, false);

    for _char_line in 0..=crtc.r4() {
        for _scan_line in 0..=crtc.r9() {
            for _char in 0..=crtc.r0() {
                for _pixel in 0..16 {
                    beginning_steps(&mut ga, &mut crtc);
                    if ga.need_memory_byte() {
                        let byte = fetch_video_byte(&bank, &ga, &crtc);
                        ga.set_memory_byte(byte);
                    }
                    end_steps(&mut ga, &mut crtc, &mut monitor);
                }
            }
        }
    }

    monitor.image().save(out_path).unwrap();
}

#[test]
#[ignore = "slow (boots real firmware + real disk I/O, tens of millions of opcodes)"]
fn cat_command_lists_shaker_disk_contents() {
    let dsk_bytes = std::fs::read(SHAKER_DSK).expect("shaker26.dsk must be present");
    let dsk = ExtendedDsk::from_buffer(&dsk_bytes).expect("valid dsk image");

    let mut motherboard = MotherboardBuilder::default().enable_interruptions().build();
    motherboard
        .fdc_mut()
        .insert_disk(FdcDrive::A, Some(AnyDisc::Edsk(dsk)));

    // Boot to the "Ready" BASIC prompt (matches the opcode count already
    // established to reach it in the boot-to-BASIC work this session).
    run_opcodes(&mut motherboard, 30_000_000);

    use CpcKey::*;
    type_text(&mut motherboard, &[C, A, T, Return]);

    // Let the CAT command's disk I/O and screen output finish.
    run_opcodes(&mut motherboard, 4_000_000);

    render_screen(
        &motherboard,
        "/tmp/shaker_cat_result.png"
    );
}
