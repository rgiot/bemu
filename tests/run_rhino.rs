//! `RUN"RHINO` from validation/RHINO.DSK: AMSDOS finds the file in the
//! directory, loads it through the FDC, BASIC runs the loader and the
//! embedded Z80 code prints "CRTC" followed by the detected CRTC type.
//!
//! Only the *presence* of a type digit is checked for now: the digit itself
//! depends on the CRTC variant being modelled faithfully, which is tracked
//! by the CRTC variant tests (see tests/crtc_variants.rs).

mod common;

use amsrust::fdc::FdcDrive;
use amsrust::keyboard::CpcKey::*;
use amsrust::motherboard::MotherboardBuilder;
use common::*;

#[test]
#[ignore = "slow: boots the real firmware"]
fn run_rhino_loads_and_prints_the_crtc_type() {
    let bytes = std::fs::read("validation/RHINO.DSK").expect("validation/RHINO.DSK");
    let disc = amsrust::disc_image::load_dsk(&bytes).expect("a valid standard DSK");

    let mut motherboard = MotherboardBuilder::default().enable_interruptions().build();
    motherboard.fdc_mut().insert_image(FdcDrive::A, Some(disc));
    motherboard.fdc_mut().start_command_log();
    run_opcodes(&mut motherboard, 1_000_000);

    type_keys(&mut motherboard, &[R, U, N]);
    press_chord(&mut motherboard, &[Shift, Digit2]); // "
    type_keys(&mut motherboard, &[R, H, I, N, O, Return]);
    run_opcodes(&mut motherboard, 3_000_000);

    let screen = screen_lines(&motherboard);
    let text = screen.join("\n");
    eprintln!("{text}");
    assert!(!text.contains("File already open"), "AMSDOS refused to open the file:\n{text}");

    // the file data really was read from the disc (sector &C5 = block 2)
    assert!(
        motherboard
            .fdc()
            .command_log()
            .iter()
            .any(|record| record.command.first() == Some(&0x66) && record.command.get(4) == Some(&0xc5)),
        "the FDC never read the file's data sector: {:02x?}",
        motherboard.fdc().command_log()
    );

    assert!(
        screen.iter().any(|line| {
            line.strip_prefix("CRTC")
                .is_some_and(|rest| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit()))
        }),
        "RHINO did not print CRTC<n>:\n{text}"
    );
}
