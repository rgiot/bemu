//! Smoke tests for the official Shaker test-suite bundle
//! (`validation/Shaker_CSL/`, from <https://shaker.logonsystem.eu/Shaker_CSL.zip>)
//! and its [`amsrust::shakerland::Manifest`]. The suite itself takes hours
//! (see `examples/run_shaker_suite.rs`, meant to be started and left
//! running); these just check the plumbing before that runs unattended.

mod common;

use amsrust::csl_runner::CslRunner;
use common::screen_lines;
use std::path::Path;

const SHAKER_CSL: &str = "validation/Shaker_CSL";

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "amsrust_shaker_suite_{name}_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn every_module_s_top_level_script_parses_and_chains_to_five_crtc_types() {
    for module in ['A', 'B', 'C', 'D', 'E'] {
        let script = Path::new(SHAKER_CSL)
            .join(format!("MODULE_{module}"))
            .join(format!("SHAKER-{module}-0.CSL"));
        let text = std::fs::read_to_string(&script)
            .unwrap_or_else(|e| panic!("{}: {e}", script.display()));
        assert!(text.contains("csl_version"), "{module}: not a CSL script");
        // each module's -0 chains through -1..-4, one crtc_select per file
        for variant in 0..=4 {
            let path = script.with_file_name(format!("SHAKER-{module}-{variant}.CSL"));
            assert!(path.exists(), "{module}: missing {}", path.display());
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                text.contains(&format!("crtc_select {variant}")),
                "{module}-{variant}: doesn't select CRTC {variant}"
            );
        }
    }
}

#[test]
fn the_disc_image_is_present_and_loadable() {
    let bytes = std::fs::read(Path::new(SHAKER_CSL).join("shaker27.dsk"))
        .expect("shaker27.dsk must be bundled");
    amsrust::disc_image::load_dsk(&bytes).expect("shaker27.dsk must be a valid DSK image");
}

#[test]
fn the_shakerland_manifest_covers_every_module_the_site_publishes() {
    let manifest = amsrust::shakerland::Manifest::bundled();
    // spot-checked against the site's own /api/tests: A1/A is "UPDATE VRAM VS
    // CRTC/GA" sub-test A, code 0001 - modules A-D are published, E is not yet.
    assert_eq!(manifest.file_stem('A', 0, 0x0001), "A1_CRTC0_A");
    assert_eq!(
        manifest.file_stem('E', 0, 0x0001),
        "E_CRTC0_0001",
        "module E isn't published yet"
    );
}

/// Boots into module E far enough to select its first test - the same
/// firmware-level keyboard path every module depends on - without waiting
/// for the hours the full suite takes.
#[test]
#[ignore = "slow: boots the real firmware from the bundled disc"]
fn module_e_boots_and_accepts_a_keypress() {
    let dir = scratch_dir("boot");
    let mut runner = CslRunner::new(&dir);
    runner
        .run_text(
            "crtc_select 0\nreset\nwait 3000000\ndisk_insert 'shaker27.dsk'\n\
             key_delay 70000 70000 400000\nkey_output 'RUN\"SHAKE27E\"\\(RET)'\nwait 10000000\n\
             key_output '1'\nwait 3000000\n",
            SHAKER_CSL,
        )
        .unwrap();

    let screen = screen_lines(runner.motherboard());
    assert!(
        screen.iter().any(|line| line.contains("R5 STORIES")),
        "test 1 never opened:\n{}",
        screen.join("\n")
    );
}
