//! The undocumented OUT (C),0 (ED 71) really outputs 0 and consumes no byte
//! after the opcode (the Dsc4 demo uses it to select and write CRTC
//! registers).

use amsrust::motherboard::Motherboard;
use amsrust::z80::{HasValue, Reg16};

fn run(motherboard: &mut Motherboard, load: u16, code: &[u8]) {
    motherboard.write_buffer(load, code);
    // JR $ after the code: nothing else is executed while the last I/O cycle ends
    motherboard.write_buffer(load + code.len() as u16, &[0x18, 0xfe]);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(load);
    let end = load + code.len() as u16;
    while motherboard.pc() != end {
        motherboard.execute_next_opcode();
    }
    // let the last instruction's I/O cycle finish (JR $ spins in place)
    for _ in 0..4 {
        motherboard.execute_next_opcode();
    }
    assert_eq!(motherboard.pc(), end);
}

#[test]
fn out_c_0_selects_crtc_register_0() {
    let mut mb = Motherboard::default();
    // LD BC,#BC05 ; OUT (C),C        -> select R5
    // LD BC,#BC09 ; OUT (C),0        -> select R0 (data 0 on the select port)
    run(
        &mut mb,
        0x4000,
        &[0x01, 0x05, 0xbc, 0xed, 0x49, 0x01, 0x09, 0xbc, 0xed, 0x71],
    );
    assert_eq!(mb.crtc().selected_register(), 0);
}

#[test]
fn out_c_0_writes_zero_in_the_selected_register() {
    let mut mb = Motherboard::default();
    assert_ne!(mb.crtc().r4(), 0);
    // select R4, then OUT (C),0 on the data port
    run(
        &mut mb,
        0x4000,
        &[0x01, 0x04, 0xbc, 0xed, 0x49, 0x01, 0x09, 0xbd, 0xed, 0x71],
    );
    assert_eq!(mb.crtc().r4(), 0);
}

#[test]
fn out_n_a_with_the_register_number_as_high_byte_selects_that_crtc_register() {
    // LD A,12 ; OUT (#FF),A -> port #0CFF: A14=0 selects the CRTC, A9=A8=0
    // is the register select, the data (12) is the register number.
    let mut mb = Motherboard::default();
    run(&mut mb, 0x4000, &[0x3e, 0x0c, 0xd3, 0xff]);
    assert_eq!(mb.crtc().selected_register(), 12);
}

#[test]
fn out_n_a_is_an_io_write_not_a_memory_write() {
    let mut mb = Motherboard::default();
    // RAM at #04FF must stay untouched by OUT (#FF),A with A=4
    let before = mb.read_byte(0x04ff);
    run(&mut mb, 0x4000, &[0x3e, 0x04, 0xd3, 0xff]);
    assert_eq!(mb.read_byte(0x04ff), before);
    assert_eq!(mb.crtc().selected_register(), 4);
}

#[test]
fn out_c_0_puts_bc_on_the_address_bus_and_0_on_the_data_bus() {
    use amsrust::chip::{Chip, ReadAddressChip, ReadDataChip};
    use amsrust::z80::Z80Pin;
    let mut mb = Motherboard::default();
    mb.write_buffer(0x4000, &[0x01, 0x09, 0xbc, 0xed, 0x71, 0x18, 0xfe]);
    mb.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    let mut seen = None;
    for _ in 0..400 {
        mb.tick();
        let cpu = mb.cpu();
        if !cpu.get_pin(Z80Pin::Neg_IORQ) && !cpu.get_pin(Z80Pin::Neg_WR) {
            seen = Some((cpu.get_address_from_pins(), cpu.get_data_from_pins()));
        }
    }
    assert_eq!(seen, Some((0xbc09, 0x00)));
}
