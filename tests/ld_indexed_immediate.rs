//! `LD (IX+d),n` / `LD (IY+d),n` used to store the displacement `d` instead
//! of the immediate `n` (the value was taken from the first operand byte,
//! which for an indexed instruction is the displacement). AMSDOS initialises
//! its state with `LD (IY+8),&FF`, so files looked permanently "already open".

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16};

fn run(program: &[u8], instructions: usize) -> Motherboard {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    motherboard.write_buffer(0x4000, program);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
    for _ in 0..instructions {
        motherboard.execute_next_opcode_no_video();
    }
    motherboard
}

#[test]
fn ld_iy_plus_d_n_stores_the_immediate() {
    // LD IY,&8000 ; LD (IY+8),&FF
    let motherboard = run(&[0xfd, 0x21, 0x00, 0x80, 0xfd, 0x36, 0x08, 0xff], 2);
    assert_eq!(motherboard.read_byte(0x8008), 0xff);
    assert_eq!(motherboard.cpu().reg16(Reg16::Pc).value(), 0x4008);
}

#[test]
fn ld_ix_plus_d_n_stores_the_immediate() {
    // LD IX,&8100 ; LD (IX+5),&AA
    let motherboard = run(&[0xdd, 0x21, 0x00, 0x81, 0xdd, 0x36, 0x05, 0xaa], 2);
    assert_eq!(motherboard.read_byte(0x8105), 0xaa);
}

#[test]
fn ld_indexed_n_with_a_negative_displacement() {
    // LD IX,&8100 ; LD (IX-2),&5A
    let motherboard = run(&[0xdd, 0x21, 0x00, 0x81, 0xdd, 0x36, 0xfe, 0x5a], 2);
    assert_eq!(motherboard.read_byte(0x80fe), 0x5a);
    // and nothing was written where the displacement byte would have gone
    assert_eq!(motherboard.read_byte(0x8100), 0x00);
}

#[test]
fn ld_hl_indirect_n_still_stores_the_immediate() {
    // LD HL,&8200 ; LD (HL),&77
    let motherboard = run(&[0x21, 0x00, 0x82, 0x36, 0x77], 2);
    assert_eq!(motherboard.read_byte(0x8200), 0x77);
}
