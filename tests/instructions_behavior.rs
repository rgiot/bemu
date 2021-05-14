///! This file contains some tests that aim at chacking the impact of the execution of an instraction on the machine

use amsrust::crtc::{Crtc, CrtcType};
use amsrust::motherboard::Motherboard;
use amsrust::z80::{HasValue, Reg16, Reg8, Z80};


/// Build a standard machine and fill it with the code  and execute it until arriving to jr $
fn execute_program(addr: u16, test: &[u8]) -> Motherboard {
	dbg!(
		cpclib_asm::disass::disassemble(test)
	);

    let end_pattern = cpclib_macros::assemble!(" jr $");


    let mut motherboard = Motherboard::default();
    motherboard.write_buffer(addr, test);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(addr);

	// execute it until it is finished
	loop {
		let previous_pc = motherboard.cpu().reg16(Reg16::Pc).value();
		motherboard.execute_next_opcode();
		let pc = motherboard.cpu().reg16(Reg16::Pc).value();

		assert!(previous_pc < pc);

		// detect if it is the end
		let current_pattern = [motherboard.read_byte(pc + 0), motherboard.read_byte(pc + 1)];

		if current_pattern == *end_pattern {
			break;
		}
	}

	motherboard

}

#[test]
fn execute_ld() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x100
		ld bc, 0x7f8e
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x100, code);

	assert_eq!(
		0x7f82,
		motherboard.cpu().reg16(Reg16::Bc).value()
	)
}

#[test]
fn execute_outd() {

}