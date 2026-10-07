//! Stack instructions (PUSH/POP/CALL/RET/EX (SP),HL/LD SP.../LD (nn),SP) must
//! hit exactly the bytes `SP-1`/`SP-2`/`SP`/`SP+1` (16-bit wrapping) of the
//! *current* memory map, whatever RAM config port &7Fxx selected (including
//! the extra 64K pages of a 512K machine) and wherever the stack sits
//! (bank boundaries, 0x0000/0xFFFF wrap-around).

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16};

const CODE: u16 = 0x9000;
/// Stack positions worth probing: wrap-around, 16K bank boundaries, plain.
const STACKS: [u16; 12] = [
    0x0000, 0x0001, 0x0002, 0x0100, 0x3fff, 0x4001, 0x7fff, 0x8001, 0xbfff, 0xc001, 0xfffe, 0xffff
];

fn machine() -> Motherboard {
    MotherboardBuilder::default()
        .set_extra_ram_pages(8)
        .disable_lower_rom()
        .disable_upper_rom()
        .build()
}

/// Selects a RAM config via a real `OUT (C),A`, then returns with code
/// space ready for `run`.
fn select_config(mb: &mut Motherboard, data: u8) {
    mb.write_buffer(CODE, &[0x01, 0xff, 0x7f, 0x3e, data, 0xed, 0x79, 0, 0, 0]);
    mb.cpu_mut().set_pc(CODE);
    for _ in 0..4 {
        mb.execute_next_opcode_no_video();
    }
}

fn run(mb: &mut Motherboard, program: &[u8], sp: u16, instructions: usize) {
    mb.write_buffer(CODE, program);
    mb.cpu_mut().set_pc(CODE);
    mb.cpu_mut().reg16_mut(Reg16::Sp).set(sp);
    for _ in 0..instructions {
        mb.execute_next_opcode_no_video();
    }
}

fn reg(mb: &Motherboard, r: Reg16) -> u16 {
    mb.cpu().reg16(r).value()
}

/// All 8 RAM configs, with an extra-page selection for each.
fn configs() -> impl Iterator<Item = u8> {
    (0u8..8).map(|c| 0b1100_0000 | ((c ^ 5) & 7) << 3 | c)
}

fn for_each_case(mut f: impl FnMut(&mut Motherboard, u8, u16)) {
    for data in configs() {
        for sp in STACKS {
            let mut mb = machine();
            select_config(&mut mb, data);
            f(&mut mb, data, sp);
        }
    }
}

#[test]
fn push_writes_high_then_low_below_sp() {
    for_each_case(|mb, data, sp| {
        // LD BC,&1234 ; PUSH BC
        run(mb, &[0x01, 0x34, 0x12, 0xc5], sp, 2);
        let ctx = format!("cfg {data:#04x} sp {sp:#06x}");
        assert_eq!(reg(mb, Reg16::Sp), sp.wrapping_sub(2), "{ctx}");
        assert_eq!(mb.read_byte(sp.wrapping_sub(1)), 0x12, "{ctx}");
        assert_eq!(mb.read_byte(sp.wrapping_sub(2)), 0x34, "{ctx}");
    });
}

#[test]
fn pop_reads_low_then_high_at_sp() {
    for_each_case(|mb, data, sp| {
        run(mb, &[0x00], sp, 0);
        mb.write_byte(sp, 0xcd);
        mb.write_byte(sp.wrapping_add(1), 0xab);
        // POP DE
        run(mb, &[0xd1], sp, 1);
        let ctx = format!("cfg {data:#04x} sp {sp:#06x}");
        assert_eq!(reg(mb, Reg16::De), 0xabcd, "{ctx}");
        assert_eq!(reg(mb, Reg16::Sp), sp.wrapping_add(2), "{ctx}");
    });
}

#[test]
fn call_then_ret_round_trips() {
    for_each_case(|mb, data, sp| {
        // CALL &9010 ... at 9010: RET
        mb.write_buffer(0x9010, &[0xc9]);
        run(mb, &[0xcd, 0x10, 0x90, 0x00], sp, 0);
        mb.write_buffer(0x9010, &[0xc9]);
        mb.cpu_mut().set_pc(CODE);
        mb.cpu_mut().reg16_mut(Reg16::Sp).set(sp);
        mb.execute_next_opcode_no_video();
        let ctx = format!("cfg {data:#04x} sp {sp:#06x}");
        assert_eq!(reg(mb, Reg16::Pc), 0x9010, "{ctx}");
        assert_eq!(reg(mb, Reg16::Sp), sp.wrapping_sub(2), "{ctx}");
        assert_eq!(mb.read_byte(sp.wrapping_sub(1)), 0x90, "{ctx}");
        assert_eq!(mb.read_byte(sp.wrapping_sub(2)), 0x03, "{ctx}");
        mb.execute_next_opcode_no_video();
        assert_eq!(reg(mb, Reg16::Pc), 0x9003, "{ctx}");
        assert_eq!(reg(mb, Reg16::Sp), sp, "{ctx}");
    });
}

#[test]
fn ex_sp_hl_and_ld_sp_forms() {
    for_each_case(|mb, data, sp| {
        run(mb, &[0x00], sp, 0);
        mb.write_byte(sp, 0x11);
        mb.write_byte(sp.wrapping_add(1), 0x22);
        // LD HL,&5678 ; EX (SP),HL
        run(mb, &[0x21, 0x78, 0x56, 0xe3], sp, 2);
        let ctx = format!("cfg {data:#04x} sp {sp:#06x}");
        assert_eq!(reg(mb, Reg16::Hl), 0x2211, "{ctx}");
        assert_eq!(mb.read_byte(sp), 0x78, "{ctx}");
        assert_eq!(mb.read_byte(sp.wrapping_add(1)), 0x56, "{ctx}");
        assert_eq!(reg(mb, Reg16::Sp), sp, "{ctx}");

        // LD (&A000),SP ; LD SP,&1234 ; LD SP,(&A000) ; LD SP,HL
        run(mb, &[0xed, 0x73, 0x00, 0xa0, 0x31, 0x34, 0x12, 0xed, 0x7b, 0x00, 0xa0], sp, 3);
        assert_eq!(reg(mb, Reg16::Sp), sp, "{ctx} LD SP,(nn)");
        run(mb, &[0x21, 0xfe, 0xff, 0xf9], sp, 2);
        assert_eq!(reg(mb, Reg16::Sp), 0xfffe, "{ctx} LD SP,HL");
    });
}

#[test]
fn inc_dec_add_sp_wrap() {
    for_each_case(|mb, data, sp| {
        // DEC SP ; INC SP ; INC SP ; LD HL,1 ; ADD HL,SP
        run(mb, &[0x3b, 0x33, 0x33, 0x21, 0x01, 0x00, 0x39], sp, 5);
        let ctx = format!("cfg {data:#04x} sp {sp:#06x}");
        assert_eq!(reg(mb, Reg16::Sp), sp.wrapping_add(1), "{ctx}");
        assert_eq!(reg(mb, Reg16::Hl), sp.wrapping_add(2), "{ctx}");
    });
}

/// With the lower ROM on, writes still reach the RAM underneath and a stack
/// there works as plain RAM once the ROM is paged out (RMR bit 2 set).
#[test]
fn push_under_rom_lands_in_ram_below() {
    let mut mb = MotherboardBuilder::default().disable_upper_rom().build();
    // LD BC,&1234 ; PUSH BC  with the lower ROM still enabled
    run(&mut mb, &[0x01, 0x34, 0x12, 0xc5], 0x0102, 2);
    // switch ROMs off through the GA (RMR: 10x with bit2 = lower ROM off)
    mb.write_buffer(CODE, &[0x01, 0x00, 0x7f, 0x3e, 0x8d, 0xed, 0x79]);
    mb.cpu_mut().set_pc(CODE);
    for _ in 0..4 {
        mb.execute_next_opcode_no_video();
    }
    assert_eq!(mb.read_byte(0x0101), 0x12);
    assert_eq!(mb.read_byte(0x0100), 0x34);
}

/// Guards the tests above against passing vacuously: the config switch
/// really changes what lives at a stack address.
#[test]
fn config_switch_really_remaps_the_stack_area() {
    let mut mb = machine();
    mb.write_byte(0x4001, 0xaa);
    select_config(&mut mb, 0b1100_0000 | (3 << 3) | 0b100); // config 4: 0x4000 block <- extra page 3
    assert_ne!(mb.read_byte(0x4001), 0xaa);
    select_config(&mut mb, 0b1100_0000); // config 0: back to base RAM
    assert_eq!(mb.read_byte(0x4001), 0xaa);
}
