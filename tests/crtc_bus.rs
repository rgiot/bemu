//! The CRTC's CPU-facing register interface is asynchronous to its character
//! clock: a register read must return the register, whatever CCLK is doing
//! when the I/O cycle happens (it used to return whatever the CPU had left on
//! the data bus, e.g. the &78 of the `IN A,(C)` opcode itself).

use amsrust::crtc::CrtcType;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16};

/// Select R12, write `value` to it, then read it back through &BFxx.
fn write_then_read_r12(crtc: CrtcType, value: u8) -> Motherboard {
    let program = [
        0x01, 0x0c, 0xbc, // LD BC,&BC0C
        0xed, 0x49, // OUT (C),C   select R12
        0x01, value, 0xbd, // LD BC,&BD<value>
        0xed, 0x49, // OUT (C),C   write it
        0x06, 0xbf, // LD B,&BF
        0xed, 0x78, // IN A,(C)   read the selected register
        0x32, 0x00, 0x80, // LD (&8000),A
    ];
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .set_crtc(crtc)
        .build();
    motherboard.write_buffer(0x4000, &program);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
    for _ in 0..8 {
        motherboard.execute_next_opcode_no_video();
    }
    motherboard
}

#[test]
fn register_read_returns_the_register_not_the_floating_bus() {
    let motherboard = write_then_read_r12(CrtcType::Crtc0, 0x34);
    assert_eq!(motherboard.crtc().r12(), 0x34);
    assert_eq!(
        motherboard.read_byte(0x8000),
        0x34,
        "IN A,(C) at &BF00 returned something other than R12"
    );
}

#[test]
fn register_read_is_stable_across_clock_phases() {
    // vary how many instructions precede the access so the I/O cycle lands
    // on different phases of the character clock
    for padding in 0..16 {
        let mut program = vec![0x00; padding];
        program.extend_from_slice(&[
            0x01, 0x0c, 0xbc, 0xed, 0x49, 0x01, 0x2a, 0xbd, 0xed, 0x49, 0x06, 0xbf, 0xed, 0x78,
            0x32, 0x00, 0x80,
        ]);
        let mut motherboard = MotherboardBuilder::default()
            .disable_lower_rom()
            .disable_upper_rom()
            .build();
        motherboard.write_buffer(0x4000, &program);
        motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
        motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
        for _ in 0..padding + 8 {
            motherboard.execute_next_opcode_no_video();
        }
        assert_eq!(
            motherboard.read_byte(0x8000),
            0x2a,
            "with {padding} NOPs before the access"
        );
    }
}
