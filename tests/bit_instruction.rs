//! `BIT n,r` / `BIT n,(HL)` / `BIT n,(IX+d)` flag behaviour. Z was set when
//! the tested bit was *set* (inverted), and S/P/X/Y were not handled at all:
//! the firmware's "is ESC held?" test (`BIT 2,(HL) : RET Z`) fell through
//! and raised a spurious break, pausing BASIC in the middle of `RUN`.
//!
//! Documented behaviour: Z = tested bit is clear, P/V = Z, S = bit 7 tested
//! and set, H = 1, N = 0, C unchanged; X/Y are bits 3/5 of the tested
//! register (for a memory operand they come from an internal register, so
//! only the documented flags are checked there).

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16};

const S: u8 = 0x80;
const Z: u8 = 0x40;
const XY: u8 = 0x28;
const H: u8 = 0x10;
const P: u8 = 0x04;
const N: u8 = 0x02;
const C: u8 = 0x01;

const VALUES: [u8; 8] = [0x00, 0xff, 0x55, 0xaa, 0x80, 0x08, 0x20, 0x7f];

fn run(program: &[u8], instructions: usize, memory: Option<(u16, u8)>) -> Motherboard {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    if let Some((address, value)) = memory {
        motherboard.write_buffer(address, &[value]);
    }
    motherboard.write_buffer(0x4000, program);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
    for _ in 0..instructions {
        motherboard.execute_next_opcode_no_video();
    }
    motherboard
}

fn flags(motherboard: &Motherboard) -> u8 {
    (motherboard.cpu().reg16(Reg16::Af).value() & 0xff) as u8
}

/// The documented flags BIT must produce (X/Y left out).
fn expected(value: u8, bit: u8, carry: bool) -> u8 {
    let bit_set = value & (1 << bit) != 0;
    let mut f = H;
    if !bit_set {
        f |= Z | P;
    }
    if bit == 7 && bit_set {
        f |= S;
    }
    if carry {
        f |= C;
    }
    f
}

#[test]
fn bit_on_a_register_sets_the_documented_flags() {
    for value in VALUES {
        for bit in 0..8u8 {
            for carry in [false, true] {
                // LD B,value ; (SCF | OR A) ; BIT bit,B
                let set_carry = if carry { 0x37 } else { 0xb7 };
                let motherboard = run(&[0x06, value, set_carry, 0xcb, 0x40 + bit * 8], 3, None);
                let f = flags(&motherboard);
                assert_eq!(
                    f & !XY,
                    expected(value, bit, carry),
                    "BIT {bit},B with B={value:#04x} carry={carry}: F={f:#04x}"
                );
                assert_eq!(f & XY, value & XY, "X/Y come from the register");
                assert_eq!(f & N, 0, "N is reset");
            }
        }
    }
}

#[test]
fn bit_on_hl_indirect_sets_the_documented_flags() {
    for value in VALUES {
        for bit in 0..8u8 {
            // LD HL,&8000 ; OR A ; BIT bit,(HL)
            let motherboard = run(
                &[0x21, 0x00, 0x80, 0xb7, 0xcb, 0x46 + bit * 8],
                3,
                Some((0x8000, value)),
            );
            assert_eq!(
                flags(&motherboard) & !XY,
                expected(value, bit, false),
                "BIT {bit},(HL) with (HL)={value:#04x}"
            );
        }
    }
}

#[test]
fn bit_on_an_indexed_operand_sets_the_documented_flags() {
    for value in VALUES {
        for bit in 0..8u8 {
            // LD IX,&8000 ; SCF ; BIT bit,(IX+3)
            let motherboard = run(
                &[
                    0xdd,
                    0x21,
                    0x00,
                    0x80,
                    0x37,
                    0xdd,
                    0xcb,
                    0x03,
                    0x46 + bit * 8,
                ],
                3,
                Some((0x8003, value)),
            );
            assert_eq!(
                flags(&motherboard) & !XY,
                expected(value, bit, true),
                "BIT {bit},(IX+3) with (IX+3)={value:#04x}"
            );
        }
    }
}

#[test]
fn bit_never_modifies_its_operand() {
    let motherboard = run(&[0x21, 0x00, 0x80, 0xcb, 0x46], 2, Some((0x8000, 0xa5)));
    assert_eq!(motherboard.read_byte(0x8000), 0xa5);
}

#[test]
fn the_firmware_esc_test_returns_when_esc_is_not_held() {
    // BIT 2,(HL) : RET Z  - with the bit clear the RET must be taken
    // LD HL,&8000 ; CALL &4010 ; (returns here) LD A,&77 ...
    let mut program = vec![0x21, 0x00, 0x80, 0xcd, 0x10, 0x40, 0x3e, 0x77];
    program.resize(0x10, 0x00);
    program.extend_from_slice(&[0xcb, 0x56, 0xc8, 0x3e, 0x11, 0xc9]); // BIT 2,(HL) ; RET Z ; LD A,&11 ; RET
    let motherboard = run(&program, 6, Some((0x8000, 0x00)));
    // instructions: LD HL, CALL, BIT, RET Z (taken), LD A,&77 -> A = &77
    assert_eq!(
        motherboard.cpu().reg16(Reg16::Af).value() >> 8,
        0x77,
        "RET Z was not taken"
    );
}
