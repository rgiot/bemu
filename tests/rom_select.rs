//! The upper ROM select port is &DFxx (A13 low). It used to latch on A13
//! *high* instead, so every gate array write (&7Fxx) overwrote the selected
//! ROM number with its command byte, and no real ROM select - AMSDOS in
//! slot 7 included - ever took effect.

use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16};

/// LD BC,port; LD A,value; OUT (C),A
fn emit_out(prog: &mut Vec<u8>, port: u16, value: u8) {
    prog.extend_from_slice(&[0x01, (port & 0xff) as u8, (port >> 8) as u8]);
    prog.extend_from_slice(&[0x3e, value]);
    prog.extend_from_slice(&[0xed, 0x79]);
}

fn run(prog: &[u8]) -> Motherboard {
    let mut motherboard = MotherboardBuilder::default().build();
    motherboard.write_buffer(0x4000, prog);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x9000);
    for _ in 0..prog.len() * 2 {
        motherboard.execute_next_opcode_no_video();
    }
    motherboard
}

#[test]
fn writing_port_df_selects_the_upper_rom() {
    let mut prog = Vec::new();
    emit_out(&mut prog, 0xdf00, 7);
    assert_eq!(run(&prog).memory_config().0.rom_number, 7);
}

#[test]
fn gate_array_writes_do_not_change_the_selected_rom() {
    let mut prog = Vec::new();
    emit_out(&mut prog, 0xdf00, 7);
    emit_out(&mut prog, 0x7f00, 0x85); // gate array: mode/ROM control command
    emit_out(&mut prog, 0x7f00, 0x8d);
    assert_eq!(
        run(&prog).memory_config().0.rom_number,
        7,
        "a gate array command byte was latched as a ROM number"
    );
}

#[test]
fn only_a_write_selects_the_rom() {
    // an ordinary memory fetch has A13 low half the time: it must not select
    let prog = vec![0x00; 64]; // NOPs
    let motherboard = run(&prog);
    assert_eq!(motherboard.memory_config().0.rom_number, 0);
}
