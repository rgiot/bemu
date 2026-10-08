//! Helpers shared by the integration tests that drive the whole machine
//! (real firmware, real keyboard scan, real disc I/O) and need to look at
//! what ended up on screen.
#![allow(dead_code)]

use amsrust::chip::*;
use amsrust::crtc::*;
use amsrust::ga::*;
use amsrust::keyboard::CpcKey;
use amsrust::monitor::*;
use amsrust::motherboard::Motherboard;

/// A keypress must outlast at least one keyboard scan (every 6th interrupt,
/// i.e. once per 50Hz frame) *and* the firmware's debounce, whatever the
/// phase it lands on: generous rather than tight.
pub const KEY_HOLD_OPCODES: u64 = 200_000;
pub const KEY_GAP_OPCODES: u64 = 100_000;

pub fn run_opcodes(motherboard: &mut Motherboard, count: u64) {
    for _ in 0..count {
        motherboard.execute_next_opcode_no_video();
    }
}

pub fn press_and_release(motherboard: &mut Motherboard, key: CpcKey) {
    motherboard.keyboard_mut().set_pressed(key, true);
    run_opcodes(motherboard, KEY_HOLD_OPCODES);
    motherboard.keyboard_mut().set_pressed(key, false);
    run_opcodes(motherboard, KEY_GAP_OPCODES);
}

/// Press several keys together (e.g. Shift + 2 for `"`), hold them, release.
pub fn press_chord(motherboard: &mut Motherboard, keys: &[CpcKey]) {
    for &key in keys {
        motherboard.keyboard_mut().set_pressed(key, true);
    }
    run_opcodes(motherboard, KEY_HOLD_OPCODES);
    for &key in keys {
        motherboard.keyboard_mut().set_pressed(key, false);
    }
    run_opcodes(motherboard, KEY_GAP_OPCODES);
}

pub fn type_keys(motherboard: &mut Motherboard, keys: &[CpcKey]) {
    for &key in keys {
        press_and_release(motherboard, key);
    }
}

/// Number of text rows of the default screens.
const TEXT_ROWS: u16 = 25;

/// Address the gate array puts on the RAM bus for a given CRTC memory
/// address (MA), scan line (RA) and byte within the character: MA13/MA12
/// select the 16K bank, RA the 2K line stripe, MA9-MA0 the offset inside it -
/// which is why hardware scrolling wraps inside a 2K window.
fn video_address(ma: u16, ra: u16, byte: u16) -> u16 {
    ((ma & 0x3000) << 2) | ((ra & 7) << 11) | (((ma & 0x3ff) << 1) + byte)
}

/// Start address currently programmed in the CRTC (R12/R13).
fn crtc_start(motherboard: &Motherboard) -> u16 {
    ((u16::from(motherboard.crtc().r12()) << 8) | u16::from(motherboard.crtc().r13())) & 0x3fff
}

/// Decode a mode 1 or mode 2 text screen back into text by matching every
/// 8x8 cell against the firmware's own character matrix (lower ROM at
/// &3800, 8 bytes per code), so a test can assert on what BASIC (or a
/// program using the firmware font) actually printed rather than on pixels.
/// Cells matching no glyph (e.g. the cursor block) are '?'.
pub fn screen_lines(motherboard: &Motherboard) -> Vec<String> {
    let mode = motherboard.ga().current_mode();
    // (characters per line, bytes per character, pixels per byte)
    let (columns, bytes_per_char) = match mode {
        VideoMode::Mode1 => (40u16, 2u16),
        VideoMode::Mode2 => (80u16, 1u16),
        other => panic!("text decoding only implemented for modes 1 and 2, not {other:?}")
    };

    let font: Vec<[u8; 8]> = (0..=255u16)
        .map(|code| {
            let mut glyph = [0u8; 8];
            for (row, byte) in glyph.iter_mut().enumerate() {
                *byte = motherboard.read_lower_rom_byte(0x3800 + code * 8 + row as u16);
            }
            glyph
        })
        .collect();

    let start = crtc_start(motherboard);
    (0..TEXT_ROWS)
        .map(|row| {
            (0..columns)
                .map(|col| {
                    // one MA covers two bytes (a "character" of the CRTC)
                    let first_byte = col * bytes_per_char;
                    let ma = start.wrapping_add(row * 40 + first_byte / 2);
                    let mut glyph = [0u8; 8];
                    for (scan, line) in glyph.iter_mut().enumerate() {
                        for byte in 0..bytes_per_char {
                            let value =
                                motherboard.read_video_ram_byte(video_address(ma, scan as u16, (first_byte + byte) % 2));
                            match mode {
                                VideoMode::Mode2 => *line = value,
                                _ => {
                                    for pixel in 0..4 {
                                        // mode 1: pixel p is bit (7-p) (pen bit 0) and bit (3-p) (pen bit 1)
                                        let pen = ((value >> (7 - pixel)) & 1) | (((value >> (3 - pixel)) & 1) << 1);
                                        if pen != 0 {
                                            *line |= 0x80 >> (byte * 4 + pixel);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if glyph == [0; 8] {
                        ' '
                    }
                    else {
                        font.iter()
                            .position(|candidate| *candidate == glyph)
                            .filter(|code| (0x20..0x7f).contains(code))
                            .map_or('?', |code| code as u8 as char)
                    }
                })
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
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

/// Render the machine's current screen memory/palette/mode as one clean
/// frame out of a fresh CRTC/GA (a live machine's own timing is too
/// chaotic to catch a whole frame reliably), for a human to look at.
pub fn render_screen(motherboard: &Motherboard, out_path: &str) {
    let mut bank = [0u8; 0x10000];
    for (addr, byte) in bank.iter_mut().enumerate() {
        *byte = motherboard.read_video_ram_byte(addr as u16);
    }
    let palette = *motherboard.ga().palette();
    let mode = motherboard.ga().current_mode();

    let mut crtc = Crtc::default();
    crtc.do_write_register(12, motherboard.crtc().r12());
    crtc.do_write_register(13, motherboard.crtc().r13());
    crtc.vlc = 7.into();
    crtc.vcc = 29.into();
    crtc.hcc = 63;
    crtc.seed_vma_for_test(crtc_start(motherboard), 29);
    crtc.set_address_on_pins();

    let mut monitor = Monitor::wait_vsync();

    let mut ga = GateArray::default();
    ga.set_palette(palette);
    ga.force_active_video_mode(mode);
    ga.set_video_mode(mode);
    ga.set_pin(GateArrayPin::CCLK, false);
    ga.seed_clock_for_test();

    for _char_line in 0..=crtc.r4() {
        for _scan_line in 0..=crtc.r9() {
            for _char in 0..=crtc.r0() {
                for _pixel in 0..16 {
                    beginning_steps(&mut ga, &mut crtc);
                    if ga.need_memory_byte() {
                        let fetch_address = ga.video_fetch_address(crtc.get_address_from_pins());
                        ga.set_memory_byte(bank[fetch_address as usize]);
                    }
                    end_steps(&mut ga, &mut crtc, &mut monitor);
                }
            }
        }
    }

    monitor.image().save(out_path).unwrap();
}
