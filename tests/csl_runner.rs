//! The CSL runner (`amsrust::csl_runner`) plays the scripts of the Shaker
//! test-suite; these tests check its own behaviour on small synthetic
//! scripts. `tests/shaker_suite.rs` and `tests/play_demo.rs` are the
//! end-to-end checks against the real, vendored Shaker/Benediction discs.

mod common;

use amsrust::crtc::CrtcType;
use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16};
use common::screen_lines;
use std::path::PathBuf;

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("amsrust_csl_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn csl_load_chains_scripts_and_the_last_crtc_select_wins() {
    let dir = scratch_dir("chain");
    std::fs::write(
        dir.join("first.csl"),
        "csl_version 1.0\ncrtc_select 3\nreset\nwait 1000\ncsl_load 'SECOND'\n",
    )
    .unwrap();
    // the chained name has another case and no extension, as in the Shaker files
    std::fs::write(
        dir.join("second.CSL"),
        "csl_version 1.0\ncrtc_select 4\nreset\ntape_play\nwait 500\n",
    )
    .unwrap();

    let mut runner = CslRunner::new(&dir);
    let report = runner.run_file(dir.join("first.csl")).unwrap();

    assert_eq!(runner.motherboard().crtc().crtc_type(), CrtcType::Crtc4);
    assert_eq!(
        report.ignored,
        vec!["tape_play: no tape inserted".to_owned()]
    );
    // the time is the script's: 1000 + 500 microseconds, to an instruction
    assert!(
        (1500..1520).contains(&report.elapsed_us),
        "{} µs",
        report.elapsed_us
    );
}

#[test]
fn reset_rebuilds_the_machine_with_the_selected_crtc() {
    let mut runner = CslRunner::new(scratch_dir("reset"));
    for crtc in CrtcType::ALL {
        let script = format!("crtc_select {}\nreset\nwait 100\n", crtc as u8);
        runner.run_text(&script, ".").unwrap();
        assert_eq!(runner.motherboard().crtc().crtc_type(), crtc);
    }
}

#[test]
fn unknown_scripts_and_discs_are_errors_not_panics() {
    let mut runner = CslRunner::new(scratch_dir("missing"));
    assert!(runner.run_text("disk_insert 'nowhere.dsk'\n", ".").is_err());
    assert!(runner.run_text("csl_load 'nowhere'\n", ".").is_err());
    assert!(runner.run_text("this is not csl\n", ".").is_err());
}

#[test]
#[ignore = "slow: boots the firmware and types on it"]
fn typing_reaches_basic_and_screenshot_saves_the_frame() {
    let dir = scratch_dir("typing");
    let mut runner = CslRunner::new(&dir);
    let report = runner
        .run_text(
            "crtc_select 2\nreset\nwait 3000000\nkey_delay 70000 70000 400000\n\
             key_output 'PRINT 6*7\\(RET)'\nwait 2000000\nscreenshot\n",
            ".",
        )
        .unwrap();

    let screen = screen_lines(runner.motherboard()).join("\n");
    assert!(screen.lines().any(|line| line.trim() == "42"), "{screen}");

    let shot = &report.screenshots[0];
    assert_eq!(shot.crtc, CrtcType::Crtc2);
    use image::GenericImageView;
    let (width, height) = image::open(&shot.path).unwrap().dimensions();
    assert_eq!(width, 832);
    assert!((300..=320).contains(&height), "{height} lines");
}

/// A tiny program that executes exactly one SSM pair (`ED <low> ED <high>`)
/// then parks on `JR $`, at a fixed address - enough to drive `wait_ssm`
/// without booting real firmware. `code`'s low and high bytes must each be
/// one of the SSM standard's 177 "neutral" codes (`SSM-STANDARD-EN.pdf`) -
/// simplest is to keep both within `0x00-0x3F`, as the standard's own
/// worked example does (`ED 3E ED 3D` -> code `0x3D3E`). A byte outside the
/// neutral ranges is a real Z80 instruction (e.g. `0xAB` is `OUTD`) and is
/// correctly never treated as part of a pair - the CSL standard's own
/// `wait_ssm 0xABCD` example is illustrative only, `0xAB` is not neutral.
fn load_one_ssm_pair(runner: &mut CslRunner, code: u16) {
    let motherboard = runner.motherboard_mut();
    let [low, high] = [(code & 0xff) as u8, (code >> 8) as u8];
    // DI first: CslRunner's machine runs with interrupts enabled (real
    // scripts need that), and a HSYNC interrupt landing between the two ED
    // <suffix> instructions - Z80 only accepts one between whole
    // instructions, but that is exactly where this pair's boundary is -
    // would execute other instructions in between and break the pairing
    // (any non-ED-suffix instruction resets it, correctly, per the
    // standard's own "or any other code <> ED resets the expectation").
    motherboard.write_buffer(0x4000, &[0xf3, 0xed, low, 0xed, high, 0x18, 0xfe]); // DI; ED ll ED hh; JR $
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x8000);
}

#[test]
fn wait_ssm_returns_as_soon_as_its_own_code_is_raised() {
    let mut runner = CslRunner::new(scratch_dir("wait_ssm"));
    // the SSM standard's own worked example: ED 3E ED 3D -> code 0x3D3E
    load_one_ssm_pair(&mut runner, 0x3d3e);
    let report = runner
        .run_text("csl_version 1.5\nwait_ssm 0x3D3E\n", ".")
        .unwrap();
    assert_eq!(report.ssm_codes, vec![0x3d3e]);
    assert!(report.ignored.is_empty(), "{:?}", report.ignored);
}

#[test]
fn wait_ssm_gives_up_on_a_code_that_never_comes() {
    let mut runner = CslRunner::new(scratch_dir("wait_ssm_miss"));
    runner.set_wait_timeout_us(10_000); // keep this test fast; the JR $ it parks on never raises anything more
    load_one_ssm_pair(&mut runner, 0x3d3e); // raises 0x3d3e, not the 0x0102 we wait for
    let report = runner
        .run_text("csl_version 1.5\nwait_ssm 0x0102\n", ".")
        .unwrap();
    assert_eq!(
        report.ssm_codes,
        vec![0x3d3e],
        "the other code was still seen and recorded"
    );
    assert_eq!(report.ignored.len(), 1, "{:?}", report.ignored);
    assert!(
        report.ignored[0].contains("wait_ssm"),
        "{:?}",
        report.ignored
    );
}

#[test]
fn wait_driveonoff_returns_once_the_motor_has_cycled() {
    let mut runner = CslRunner::new(scratch_dir("wait_drive"));
    let motherboard = runner.motherboard_mut();
    // Through the real bus, the way firmware does it (see
    // Motherboard::is_fdc_motor_addr): OUT (C),1 to &FAxx turns the drive
    // motor on, OUT (C),0 turns it off. LD BC,&FA01 / OUT (C),C / a short
    // delay / LD BC,&FA00 / OUT (C),C / JR $.
    motherboard.write_buffer(
        0x4000,
        &[
            0x01, 0x01, 0xfa, 0xed, 0x49, 0x06, 0x10, 0x10, 0xfe, 0x01, 0x00, 0xfa, 0xed, 0x49,
            0x18, 0xfe,
        ],
    );
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x8000);
    assert!(
        !motherboard.fdc().motor(),
        "starts off, as the program assumes"
    );

    let report = runner
        .run_text("csl_version 1.0\nwait_driveonoff\n", ".")
        .unwrap();
    assert!(report.ignored.is_empty(), "{:?}", report.ignored);
    assert!(
        !runner.motherboard().fdc().motor(),
        "off again once wait_driveonoff returns"
    );
}
