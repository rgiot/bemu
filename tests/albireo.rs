#![cfg(feature = "albireo")]
//! The Albireo's CH376 driven by Z80 code through &FE80/&FE81.

use amsrust::ch376::Albireo;
use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};

#[test]
fn the_cpu_talks_to_the_ch376() {
    let mut builder = MotherboardBuilder::default();
    builder.disable_lower_rom().disable_upper_rom();
    let mut motherboard = builder.build();
    motherboard.plug_expansion(Box::new(Albireo::new(None)));
    // CHECK_EXIST &57 -> &A8; GET_IC_VER; the status (no key: no interrupt, bit 7 high);
    // two INs of the data port read two bytes, not more
    let program = cpclib_macros::assemble!(
        "
        di
        ld bc,0xfe81 : ld a,0x06 : out (c),a
        ld bc,0xfe80 : ld a,0x57 : out (c),a
        in a,(c) : ld (0x8000),a
        ld bc,0xfe81 : ld a,0x01 : out (c),a
        ld bc,0xfe80 : in a,(c) : ld (0x8001),a
        ld bc,0xfe81 : in a,(c) : ld (0x8002),a
        loop: jr loop
        "
    );
    motherboard.write_buffer(0x100, program);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);
    for _ in 0..200 {
        motherboard.execute_next_opcode_no_video();
    }
    assert_eq!(
        motherboard.read_byte(0x8000),
        0xa8,
        "CHECK_EXIST: the complement"
    );
    assert_eq!(
        motherboard.read_byte(0x8001) & 0xc0,
        0x40,
        "a CH376's version"
    );
    assert_eq!(
        motherboard.read_byte(0x8002) & 0x80,
        0x80,
        "no interrupt pending"
    );
}
