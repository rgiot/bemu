//! End-to-end test: boot to BASIC, insert the real Shaker test-suite disk,
//! type `cat` at the keyboard (through the matrix, not injected text) and
//! check the firmware's own catalog routine - reading the disc through the
//! FDC - lists the files that are really on it.
//!
//! This is the first test that drives the FDC through real firmware (AMSDOS)
//! rather than hand-crafted raw commands, and the first to drive the
//! keyboard matrix through the firmware's own interrupt-driven scan.

mod common;

use amsrust::fdc::FdcDrive;
use amsrust::keyboard::CpcKey;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use common::*;
use cpclib_disc::edsk::ExtendedDsk;
use cpclib_disc::AnyDisc;

const SHAKER_DSK: &str = "../rust.cpcdemotools/cpclib-csl/tests/CSL/Shaker_CSL/shaker26.dsk";

/// Opcodes needed for the firmware to reach the BASIC "Ready" prompt.
const BOOT_OPCODES: u64 = 1_000_000;

/// Opcodes AMSDOS needs, after Enter, to read the directory and print it.
const CAT_OPCODES: u64 = 2_500_000;

fn shaker_machine() -> Motherboard {
    let dsk_bytes = std::fs::read(SHAKER_DSK).expect("shaker26.dsk must be present");
    let dsk = ExtendedDsk::from_buffer(&dsk_bytes).expect("valid dsk image");

    let mut motherboard = MotherboardBuilder::default().enable_interruptions().build();
    motherboard
        .fdc_mut()
        .insert_disk(FdcDrive::A, Some(AnyDisc::Edsk(dsk)))
        .expect("the disc can be loaded in the FDC");
    motherboard
}

#[test]
#[ignore = "slow: boots the real firmware"]
fn cat_command_lists_shaker_disk_contents() {
    let mut motherboard = shaker_machine();
    run_opcodes(&mut motherboard, BOOT_OPCODES);

    let before = screen_lines(&motherboard);
    assert!(
        before.iter().any(|line| line.contains("Ready")),
        "BASIC never reached its prompt:\n{}",
        before.join("\n")
    );

    use CpcKey::*;
    type_keys(&mut motherboard, &[C, A, T, Return]);
    // Let the catalog's disc I/O and screen output finish.
    run_opcodes(&mut motherboard, CAT_OPCODES);

    if let Ok(path) = std::env::var("SHAKER_CAT_PNG") {
        render_screen(&motherboard, &path);
    }
    let after = screen_lines(&motherboard);

    // What AMSDOS prints, with the columns' padding collapsed. Names and
    // sizes are the ones the disc's catalog really holds (cross-checked
    // against the directory as parsed independently by cpclib-disc).
    let screen = after
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for expected in [
        "Drive A: user 0",
        "SHAKE26A.BIN 27K",
        "SHAKE26B.BIN 27K",
        "SHAKE26C.BIN 28K",
        "SHAKE26D.BIN 27K",
        "SHAKE26E.BIN 8K",
        "61K free",
    ] {
        assert!(
            screen.contains(expected),
            "{expected:?} missing from the CAT output:\n{}",
            after.join("\n")
        );
    }
}
