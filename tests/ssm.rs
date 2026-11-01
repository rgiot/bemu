//! SSM (ScreenShot Management) protocol tests - see SSM-STANDARD-EN.pdf.
//!
//! SSM repurposes 177 "neutral" (undefined) ED-prefixed suffix bytes: two
//! of them executed back to back form a 16-bit request code that the
//! emulator surfaces via `Z80::take_ssm_codes`.

use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};

fn run(load: u16, exec: u16, code: &[u8]) -> amsrust::motherboard::Motherboard {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();

    motherboard.write_buffer(load, code);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(exec);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x8000);

    for _ in 0..code.len() {
        motherboard.execute_next_opcode_no_video();
    }

    motherboard
}

#[test]
fn ssm_example_from_standard() {
    // The exact byte sequence given by SSM-STANDARD-EN.pdf: the first
    // "ED 3F" pair is discarded because it isn't immediately followed by
    // another ED (the "00" resets the expectation), then "ED 3E" / "ED 3D"
    // completes a pair. Expected code: LL=3E, HH=3D -> 0x3D3E.
    let code = [0xed, 0x3f, 0x00, 0xed, 0x3e, 0xed, 0x3d];
    let mut motherboard = run(0x4000, 0x4000, &code);

    assert_eq!(motherboard.cpu_mut().take_ssm_codes(), vec![0x3d3e]);
    // Draining is destructive: a second call returns nothing new.
    assert_eq!(motherboard.cpu_mut().take_ssm_codes(), Vec::<u16>::new());
}

#[test]
fn ssm_consecutive_pairs_each_detected() {
    // Two back-to-back complete pairs: ED 00/ED 01, then immediately
    // ED 02/ED 03. The second pair's first half isn't preceded by any
    // non-ED instruction, only by the first pair completing.
    let code = [0xed, 0x00, 0xed, 0x01, 0xed, 0x02, 0xed, 0x03];
    let mut motherboard = run(0x4000, 0x4000, &code);

    assert_eq!(motherboard.cpu_mut().take_ssm_codes(), vec![0x0100, 0x0302]);
}

#[test]
fn ssm_real_ed_instruction_breaks_pending_pair() {
    // ED C1 is a real instruction (OUT (C),B is 0x41; use LDI at 0xA0
    // which is unambiguously real and not in any neutral range) between
    // two neutral ED suffixes: it must reset the pending low byte instead
    // of being mistaken for the high byte of a pair.
    let code = [0xed, 0x01, 0xed, 0xa0, 0xed, 0x02, 0xed, 0x03];
    let mut motherboard = run(0x4000, 0x4000, &code);

    // Only the second pair (02/03) should be detected.
    assert_eq!(motherboard.cpu_mut().take_ssm_codes(), vec![0x0302]);
}

#[test]
fn ssm_plain_instruction_breaks_pending_pair() {
    // A totally unrelated one-byte instruction (NOP) between two neutral
    // ED suffixes must also reset the pending low byte.
    let code = [0xed, 0x01, 0x00, 0xed, 0x02, 0xed, 0x03];
    let mut motherboard = run(0x4000, 0x4000, &code);

    assert_eq!(motherboard.cpu_mut().take_ssm_codes(), vec![0x0302]);
}

#[test]
fn ssm_no_codes_when_no_pairs() {
    let code = [0x00, 0x00, 0xed, 0x40 /* IN B,(C), real instruction */];
    let mut motherboard = run(0x4000, 0x4000, &code);

    assert!(motherboard.cpu_mut().take_ssm_codes().is_empty());
}
