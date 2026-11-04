//! Kevin Thacker's CPC "Acid tests" (from the Arnold emulator's distribution, see
//! `tests/fixtures/acid/README.md`): the automatic ones, built with his test library, which print
//! `name-PASS` or `name-FAIL` for each test through the firmware.
//!
//! Each binary runs as on a real CPC: BASIC up, the binary in memory, `CALL &xxxx` typed. The
//! harness keeps what the firmware prints (TXT OUTPUT, &BB5A) and answers the key waits (KM WAIT
//! CHAR, &BB06: `n`, which is also "no" to the questions; KM READ CHAR, &BB09: no key) by
//! returning from them at once. The binaries aren't changed.
//!
//! The tests are slow (minutes of emulated time each): they are ignored by default, run them
//! with `cargo test --release --test acid -- --ignored`. Each one lists the tests bemu still
//! fails; any other failure fails it, and a listed one that passes is reported (to be removed
//! from its list).

use amsrust::crtc::CrtcType;
use amsrust::keyboard::CpcKey;
use amsrust::motherboard::{CpcModel, Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16, Reg8, Z80Flags};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/acid");

fn run_for(motherboard: &mut Motherboard, t_states: usize) {
    let end = motherboard.t_state_counter().value() + t_states;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_no_video();
    }
}

fn press(motherboard: &mut Motherboard, keys: &[CpcKey]) {
    for &key in keys {
        motherboard.keyboard_mut().set_pressed(key, true);
    }
    run_for(motherboard, 240_000);
    for &key in keys {
        motherboard.keyboard_mut().set_pressed(key, false);
    }
    run_for(motherboard, 160_000);
}

/// Returns from the firmware routine the CPU is at the entry of, with A and the carry.
fn return_with(motherboard: &mut Motherboard, a: u8, carry: bool) {
    let sp = motherboard.cpu().reg16(Reg16::Sp).value();
    let pc = motherboard.read_word(sp);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(sp.wrapping_add(2));
    motherboard.cpu_mut().reg8_mut(Reg8::A).set(a);
    motherboard.cpu_mut().set_flag(Z80Flags::C, carry);
    motherboard.set_pc(pc);
}

/// Runs `bin` (an AMSDOS binary) on a 6128 with `crtc`, or a 6128+ for CRTC 3: what it printed,
/// up to "Done." or `seconds` of emulated time. The key waits get `answers`, then `n`.
fn printed(bin: &str, crtc: CrtcType, seconds: usize, answers: &[u8]) -> String {
    let mut answers = answers.iter().copied();
    let file = std::fs::read(format!("{FIXTURES}/{bin}")).unwrap_or_else(|e| panic!("{bin}: {e}"));
    let (header, body) = file.split_at(128);
    let load = u16::from_le_bytes([header[0x15], header[0x16]]);
    let exec = u16::from_le_bytes([header[0x1a], header[0x1b]]);

    let mut builder = MotherboardBuilder::default();
    if crtc == CrtcType::Crtc3 {
        builder.set_model("6128plus".parse::<CpcModel>().unwrap());
    }
    builder.set_crtc(crtc);
    let mut motherboard = builder.build();
    // nobody looks at the picture (the CPC+'s ASIC does depend on the video fetches)
    motherboard.set_video(crtc == CrtcType::Crtc3);
    run_for(&mut motherboard, 3 * 4_000_000);
    motherboard.write_buffer(load, body);
    for c in format!("CALL &{exec:04X}").chars() {
        let keys = motherboard.keyboard_layout().keys_of_char(c).unwrap();
        press(&mut motherboard, &keys);
    }
    motherboard.keyboard_mut().set_pressed(CpcKey::Return, true);
    run_for(&mut motherboard, 240_000);
    motherboard.keyboard_mut().set_pressed(CpcKey::Return, false);

    let mut log = String::new();
    let end = motherboard.t_state_counter().value() + seconds * 4_000_000;
    while motherboard.t_state_counter().value() < end {
        match motherboard.pc() {
            0xbb5a => match motherboard.cpu().reg8(Reg8::A).value() {
                13 => log.push('\n'),
                c @ 32..=126 => log.push(c as char),
                _ => ()
            },
            0xbb06 => {
                if log.contains("Done.") {
                    break;
                }
                let key = answers.next().unwrap_or(b'n');
                return_with(&mut motherboard, key, true);
                continue;
            }
            0xbb09 => {
                return_with(&mut motherboard, 0, false);
                continue;
            }
            _ => ()
        }
        motherboard.execute_next_opcode_no_video();
    }
    log
}

/// The tests' names and whether they passed, from what the library printed.
fn results(log: &str) -> Vec<(String, bool)> {
    log.lines()
        .filter_map(|line| {
            // the first one follows the "Press a key to start" of the introduction
            let line = line.rsplit("Press a key to start").next().unwrap_or(line);
            line.strip_suffix("-PASS")
                .map(|name| (name.to_owned(), true))
                .or_else(|| line.strip_suffix("-FAIL").map(|name| (name.to_owned(), false)))
        })
        .collect()
}

/// Runs `bin` and checks its results: every test it ran passed but the `known_failures`, and it
/// ran at least `expected` tests.
fn check(bin: &str, crtc: CrtcType, seconds: usize, answers: &[u8], expected: usize, known_failures: &[&str]) {
    let log = printed(bin, crtc, seconds, answers);
    let results = results(&log);
    let new: Vec<&str> = results
        .iter()
        .filter(|(name, passed)| !passed && !known_failures.contains(&name.as_str()))
        .map(|(name, _)| name.as_str())
        .collect();
    let fixed: Vec<&&str> = known_failures
        .iter()
        .filter(|known| results.iter().any(|(name, passed)| *passed && name == *known))
        .collect();
    if !fixed.is_empty() {
        eprintln!("{bin}: these pass now, remove them from the known failures: {fixed:?}");
    }
    assert!(new.is_empty(), "{bin}: new failures {new:?}\n{log}");
    assert!(results.len() >= expected, "{bin}: {} tests ran, {expected} expected\n{log}", results.len());
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn cpc() {
    check("cpctest.bin", CrtcType::Crtc0, 120, &[], 4, &[
        // On Kevin's type 0 CPCs the VSYNC, read on the PPI, still runs 1 us later than bemu ends it
        "CPC int (HSYNC width)",
        // "GA I/O decode test" isn't listed: it gives io_decode an uninitialised BC as its restore
        // hook (every other caller sets it), and bemu jumps to &0000 with it
    ]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn ppi() {
    check("ppi.bin", CrtcType::Crtc0, 900, &[], 23, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn psg() {
    check("psg.bin", CrtcType::Crtc0, 900, &[], 20, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn z80() {
    check("cpu.bin", CrtcType::Crtc0, 300, &[], 8, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn z80_in_flags() {
    check("inout.bin", CrtcType::Crtc0, 120, &[], 8, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn asic_crtc_status() {
    check("crtc3_4_status.bin", CrtcType::Crtc3, 300, &[], 5, &[
        // both fail on Kevin's 6128+ and type 4 CPC too, the same way ("failed 4 times, remainder
        // ok", "ok, then failed")
        "Vsync end",
        "Cursor active on scanline",
    ]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn crtc1_status() {
    check("type1ctrl.bin", CrtcType::Crtc1, 300, &[], 5, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn rom_decoding() {
    // it asks the machine: 3, a 6128
    check("romtest.bin", CrtcType::Crtc0, 300, b"3", 1, &[]);
}

/// The CRTC test, which asks the type it runs on.
fn crtc(crtc: CrtcType, known_failures: &[&str]) {
    let digit = b'0' + crtc as u8;
    // Its I/O decode test scans the 65536 ports, each one waiting for a VSYNC (or its timeout),
    // so it takes hours of emulated time: only worth running optimised.
    let seconds = if cfg!(debug_assertions) { 3600 } else { 100_000 };
    check("crtctest.bin", crtc, seconds, &[digit], 20, known_failures);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn crtc0() {
    crtc(CrtcType::Crtc0, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn crtc1() {
    crtc(CrtcType::Crtc1, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn crtc2() {
    crtc(CrtcType::Crtc2, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn crtc3() {
    crtc(CrtcType::Crtc3, &[]);
}

#[test]
#[ignore = "slow: minutes of emulated time"]
fn crtc4() {
    crtc(CrtcType::Crtc4, &[]);
}
