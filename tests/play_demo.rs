//! Smoke tests for playing real demos (see `examples/play_demo.rs`) against
//! the two small ones vendored for this purpose, from
//! <http://benediction.cpcscene.net/releases.html>.

use amsrust::csl_runner::CslRunner;
use amsrust::disc_image::load_dsk;
use amsrust::ga::VideoMode;

const BND4_PACKAGE: &str = "validation/Benediction/bnd4_package.dsk";
const HBL: &str = "validation/Benediction/HBL.DSK";

#[test]
fn the_catalog_reader_tells_catart_from_the_real_files_on_both_discs() {
    for (path, expected) in [(BND4_PACKAGE, "#MENU"), (HBL, "HBL.BAS")] {
        let bytes = std::fs::read(path).unwrap();
        let disc = load_dsk(&bytes).unwrap();
        let catalog = disc.amsdos_catalog();
        assert!(!catalog.is_empty(), "{path}: empty catalog");

        let real: Vec<_> = catalog.iter().filter(|e| e.is_plausible_filename()).collect();
        assert!(!real.is_empty(), "{path}: every entry looked like CATART");
        assert!(
            real.iter().any(|e| e.filename() == expected),
            "{path}: expected {expected} among the real files, got {real:?}"
        );
    }
}

/// bnd4_package.dsk's `#MENU` runs straight into a custom-font, multi-colour
/// menu screen (Mode 2, machine code) - a disc that needs no special
/// parameters, unlike HBL.DSK (see `examples/play_demo.rs`'s own doc
/// comment for that one).
#[test]
#[ignore = "slow: boots the real firmware from the bundled disc"]
fn bnd4_package_boots_into_its_menu() {
    let dir = std::env::temp_dir().join(format!("amsrust_play_demo_bnd4_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut runner = CslRunner::new(&dir);
    runner
        .run_text(
            "crtc_select 0\nreset\nwait 3000000\ndisk_insert 'bnd4_package.dsk'\n\
             key_delay 70000 70000 400000\nkey_output 'RUN\"#MENU\"\\(RET)'\nwait 8000000\n",
            "validation/Benediction"
        )
        .unwrap();

    let motherboard = runner.motherboard();
    // BASIC's own boot screen is Mode 1 with the firmware running from
    // lower ROM (PC < 0x4000); the menu takes over both.
    assert_eq!(motherboard.ga().current_mode(), VideoMode::Mode2, "the menu never took over the screen");
    assert!(motherboard.pc() >= 0x4000, "still running firmware, PC={:04x}", motherboard.pc());
}
