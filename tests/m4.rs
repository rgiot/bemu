//! The M4 board on the expansion port, driven by Z80 code: its ROM in slot 6 through ROMDIS,
//! a command through &FE00/&FC00, the response read from its ROM.

use amsrust::m4::M4Board;
use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};

#[test]
fn the_cpu_reads_the_m4_rom_and_its_responses() {
    let sd = std::env::temp_dir().join(format!("bemu-m4-bus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sd);
    std::fs::create_dir_all(&sd).unwrap();
    let mut builder = MotherboardBuilder::default();
    builder.disable_lower_rom();
    let mut motherboard = builder.build();
    motherboard.plug_expansion(Box::new(M4Board::with_bundled_rom(&sd).unwrap()));
    // upper ROM on (RMR: lower ROM off), ROM 6 selected; copy its first byte and its name;
    // C_VERSION; copy the response
    let program = cpclib_macros::assemble!(
        "
        di : ld sp,0x4000
        ld bc,0x7f84 : out (c),c
        ld bc,0xdf06 : out (c),c
        ld hl,0xc000 : ld de,0x8000 : ld bc,0x10 : ldir
        ld hl,0xc093 : ld de,0x8010 : ld bc,8 : ldir
        ld bc,0xfe00 : ld a,2 : out (c),a : ld a,0x26 : out (c),a : ld a,0x43 : out (c),a
        ld b,0xfc : out (c),c
        ld hl,0xe800 : ld de,0x8100 : ld bc,0x40 : ldir
        ld bc,0xdf00 : out (c),c
        ld a,(0xc000) : ld (0x8200),a
        loop: jr loop
        "
    );
    motherboard.write_buffer(0x100, program);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);
    for _ in 0..5000 {
        motherboard.execute_next_opcode_no_video();
    }
    assert_eq!(motherboard.read_byte(0x8000), 0x01, "a background ROM's header");
    let name: Vec<u8> = (0..8).map(|i| motherboard.read_byte(0x8010 + i)).collect();
    assert_eq!(&name, b"M4 BOAR\xc4");
    // the response: the length, the command echoed, the version
    assert_eq!((motherboard.read_byte(0x8101), motherboard.read_byte(0x8102)), (0x26, 0x43));
    let length = usize::from(motherboard.read_byte(0x8100)) - 2;
    let text: Vec<u8> = (0..length).map(|i| motherboard.read_byte(0x8103 + i as u16)).collect();
    assert!(text.starts_with(b"bemu's M4"), "{}", String::from_utf8_lossy(&text));
    // ROM 0 (BASIC) is the CPC's again
    assert_eq!(motherboard.read_byte(0x8200), 0x80, "BASIC's first byte");
}
