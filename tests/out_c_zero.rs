//! The undocumented OUT (C),0 (ED 71) really outputs 0 and consumes no byte
//! after the opcode (the Dsc4 demo uses it to select and write CRTC
//! registers).

use amsrust::motherboard::Motherboard;
use amsrust::z80::{HasValue, Reg16};

fn run(motherboard: &mut Motherboard, load: u16, code: &[u8]) {
    motherboard.write_buffer(load, code);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(load);
    let end = load + code.len() as u16;
    while motherboard.pc() != end {
        motherboard.execute_next_opcode();
    }
}

#[test]
fn out_c_0_selects_crtc_register_0() {
    let mut mb = Motherboard::default();
    // LD BC,#BC05 ; OUT (C),C        -> select R5
    // LD BC,#BC09 ; OUT (C),0        -> select R0 (data 0 on the select port)
    run(&mut mb, 0x4000, &[0x01, 0x05, 0xbc, 0xed, 0x49, 0x01, 0x09, 0xbc, 0xed, 0x71]);
    assert_eq!(mb.pc(), 0x400a, "OUT (C),0 is a 2 bytes instruction");
    assert_eq!(mb.crtc().selected_register(), 0);
}

#[test]
fn out_c_0_writes_zero_in_the_selected_register() {
    let mut mb = Motherboard::default();
    assert_ne!(mb.crtc().r4(), 0);
    // select R4, then OUT (C),0 on the data port
    run(&mut mb, 0x4000, &[0x01, 0x04, 0xbc, 0xed, 0x49, 0x01, 0x09, 0xbd, 0xed, 0x71]);
    assert_eq!(mb.pc(), 0x400a);
    assert_eq!(mb.crtc().r4(), 0);
}
