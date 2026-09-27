//! The CSL runner (`amsrust::csl_runner`) plays the scripts of the Shaker
//! test-suite; these tests check its own behaviour, then use Shaker's
//! Module E menu - which detects the CRTC by itself and prints its number -
//! as an end-to-end check of the whole chain: script, reset, disc, typing
//! through the keyboard matrix, firmware, program, video.

mod common;

use amsrust::crtc::CrtcType;
use amsrust::csl_runner::CslRunner;
use common::screen_lines;
use std::path::PathBuf;

const SHAKER_DIR: &str = "../rust.cpcdemotools/cpclib-csl/tests/CSL/Shaker_CSL/MODULE_E";

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
        "csl_version 1.0\ncrtc_select 3\nreset\nwait 1000\ncsl_load 'SECOND'\n"
    )
    .unwrap();
    // the chained name has another case and no extension, as in the Shaker files
    std::fs::write(
        dir.join("second.CSL"),
        "csl_version 1.0\ncrtc_select 4\nreset\ntape_play\nwait 500\n"
    )
    .unwrap();

    let mut runner = CslRunner::new(&dir);
    let report = runner.run_file(dir.join("first.csl")).unwrap();

    assert_eq!(runner.motherboard().crtc().crtc_type(), CrtcType::Crtc4);
    assert_eq!(report.ignored, vec!["tape_play".to_owned()]);
    // the time is the script's: 1000 + 500 microseconds, to an instruction
    assert!((1500..1520).contains(&report.elapsed_us), "{} µs", report.elapsed_us);
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
            "."
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

/// Shaker module E prints "CRTC <n>" (its own detection) under its menu.
#[test]
#[ignore = "slow: about a minute per CRTC type"]
fn shaker_module_e_menu_detects_every_crtc_type() {
    let dir = scratch_dir("shaker_e");
    let mut failures = Vec::new();
    for crtc in CrtcType::ALL {
        let mut runner = CslRunner::new(&dir);
        runner
            .run_text(
                &format!(
                    "crtc_select {}\nreset\nwait 3000000\ndisk_insert 'shaker26.dsk'\n\
                     key_delay 70000 70000 400000\nkey_output 'RUN\"SHAKE26E\"\\(RET)'\n\
                     wait 10000000\nscreenshot_name 'menu_{0}'\nscreenshot\n",
                    crtc as u8
                ),
                "../rust.cpcdemotools/cpclib-csl/tests/CSL/Shaker_CSL"
            )
            .unwrap();

        let screen = screen_lines(runner.motherboard());
        let detected = screen
            .iter()
            .rev()
            .find_map(|line| line.trim().strip_prefix("CRTC ").map(str::to_owned));
        eprintln!("CRTC {} -> Shaker says {detected:?}", crtc as u8);
        if detected.as_deref() != Some(&(crtc as u8).to_string()) {
            failures.push(format!("{crtc:?}: {detected:?}\n{}", screen.join("\n")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let _ = SHAKER_DIR;
}
