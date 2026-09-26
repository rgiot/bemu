///! Keyboard matrix / PSG wiring tests: drives the real port A (&F4xx) and
///! port C (&F6xx) PPI addresses with real Z80 `OUT`/`IN` instructions,
///! reproducing the exact bus sequence CPC firmware uses to read a
///! keyboard row through PSG register 14 (see src/psg.rs and
///! src/keyboard.rs).

use amsrust::keyboard::CpcKey;
use amsrust::motherboard::MotherboardBuilder;
use amsrust::z80::{HasValue, Reg16};

const PORT_A: u8 = 0xf4;
const PORT_C: u8 = 0xf6;

/// LD BC,B<<8; LD A,val; OUT (C),A
fn emit_out(prog: &mut Vec<u8>, port_high: u8, value: u8) {
    prog.extend_from_slice(&[0x01, 0x00, port_high]); // LD BC,port_high*256
    prog.extend_from_slice(&[0x3e, value]); // LD A,value
    prog.extend_from_slice(&[0xed, 0x79]); // OUT (C),A
}

/// LD BC,B<<8; IN A,(C); LD (dest),A
fn emit_in(prog: &mut Vec<u8>, port_high: u8, dest: u16) {
    prog.extend_from_slice(&[0x01, 0x00, port_high]);
    prog.extend_from_slice(&[0xed, 0x78]); // IN A,(C)
    prog.extend_from_slice(&[0x32, (dest & 0xff) as u8, (dest >> 8) as u8]);
}

/// The exact bus sequence firmware uses to read one keyboard row: select
/// PSG register 14, then trigger a read with that row selected on port C's
/// low nibble, then read the byte back from port A.
fn emit_read_row(prog: &mut Vec<u8>, row: u8, dest: u16) {
    emit_out(prog, PORT_A, 14); // register 14 = I/O port A
    emit_out(prog, PORT_C, 0xc0); // BDIR=1,BC1=1 -> latch address
    emit_out(prog, PORT_C, 0x00); // back to inactive
    emit_out(prog, PORT_C, 0x40 | (row & 0x0f)); // BDIR=0,BC1=1 -> read, row selected
    emit_in(prog, PORT_A, dest);
    emit_out(prog, PORT_C, 0x00); // back to inactive
}

fn run(prog: &[u8]) -> amsrust::motherboard::Motherboard {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();

    motherboard.write_buffer(0x4000, prog);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);

    // OUT/IN each take 3 instructions; over-provision generously since
    // extra iterations just re-run harmless NOPs past the program.
    for _ in 0..prog.len() * 2 {
        motherboard.execute_next_opcode_no_video();
    }
    motherboard
}

#[test]
fn reading_an_untouched_row_returns_all_released() {
    let mut prog = Vec::new();
    emit_read_row(&mut prog, 8, 0x8000);
    let motherboard = run(&prog);
    assert_eq!(motherboard.read_byte(0x8000), 0xff);
}

#[test]
fn pressed_key_shows_up_as_a_clear_bit_on_its_row() {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    motherboard.keyboard_mut().set_pressed(CpcKey::A, true);

    let mut prog = Vec::new();
    emit_read_row(&mut prog, 8, 0x8000);
    motherboard.write_buffer(0x4000, &prog);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
    for _ in 0..prog.len() * 2 {
        motherboard.execute_next_opcode_no_video();
    }

    // CpcKey::A is line 8, bit 5 - active low.
    assert_eq!(motherboard.read_byte(0x8000), !(1 << 5));
}

#[test]
fn reading_the_wrong_row_does_not_see_the_pressed_key() {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    motherboard.keyboard_mut().set_pressed(CpcKey::A, true); // line 8

    let mut prog = Vec::new();
    emit_read_row(&mut prog, 5, 0x8000); // a different row
    motherboard.write_buffer(0x4000, &prog);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
    for _ in 0..prog.len() * 2 {
        motherboard.execute_next_opcode_no_video();
    }

    assert_eq!(motherboard.read_byte(0x8000), 0xff);
}

#[test]
fn keyboard_write_style_rows_are_read_back_correctly() {
    // Exercise the CslInstruction::KeyboardWrite wire format directly.
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    let mut lines = [0xffu8; 10];
    lines[3] = 0b1110_1111; // bit 4 pressed on line 3
    motherboard.keyboard_mut().set_lines(lines);

    let mut prog = Vec::new();
    emit_read_row(&mut prog, 3, 0x8000);
    motherboard.write_buffer(0x4000, &prog);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
    for _ in 0..prog.len() * 2 {
        motherboard.execute_next_opcode_no_video();
    }

    assert_eq!(motherboard.read_byte(0x8000), 0b1110_1111);
}
