///! This file contains some tests that aim at chacking the impact of the execution of an instraction on the machine

use amsrust::crtc::{Crtc, CrtcType};
use amsrust::motherboard::Motherboard;
use amsrust::z80::{HasValue, Reg16, Reg8, Z80, Z80Flags};
use amsrust::ga::{VideoMode, Ink};

/// Build a standard machine and fill it with the code  and execute it until arriving to jr $
/// Attention, do not put the code in area where ROMs are activated ...
fn execute_program(load: u16, exec: u16, test: &[u8]) -> Motherboard {
	dbg!(
		cpclib_asm::disass::disassemble(test)
	);

    let end_pattern = cpclib_macros::assemble!(" jr $");


    let mut motherboard = Motherboard::default();

	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().active_video_mode()
	);
	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().next_video_mode()
	);

    motherboard.write_buffer(load, test);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(exec);

	// execute it until it is finished
	loop {
		let previous_pc = motherboard.cpu().reg16(Reg16::Pc).value();
		motherboard.execute_next_opcode();
		let pc = motherboard.cpu().reg16(Reg16::Pc).value();

		// detect if it is the end
		let current_pattern = [motherboard.read_byte(pc + 0), motherboard.read_byte(pc + 1)];

		if current_pattern == *end_pattern {
			break;
		}
	}

	motherboard

}



#[test]
fn execute_call() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		call end
		jr $ ; never reached if not buggy

		org 0x5000
end
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0x5000,
		motherboard.cpu().reg16(Reg16::Pc).value()
	);
}


#[test]
fn execute_dec() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0x7f8d
		dec b
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0x7e,
		motherboard.cpu().reg8(Reg8::B).value()
	);
}


#[test]
fn execute_and1() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld a, 0xff
		and 0
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	let a = motherboard.cpu().reg8(Reg8::A).value();
	let f = motherboard.cpu().reg8(Reg8::F).value();
	assert_eq!(
		0x0,
		a
	);

	assert!(
		Z80Flags::Z.is_on(f)
	);
}


#[test]
fn execute_and2() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld a, 0xff
		and 0b01
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	let a = motherboard.cpu().reg8(Reg8::A).value();
	let f = motherboard.cpu().reg8(Reg8::F).value();
	assert_eq!(
		0x01,
		a
	);

	assert!(
		Z80Flags::Z.is_off(f)
	);
}

#[test]
fn execute_in1() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0xf5
		in a, (c)
		jr $ ; end of test
	"));

	// wait (too much) to be sure screen mode changed
	let mut motherboard = execute_program(0x4000,0x4000, code);


	// TODO add the tests to check the machinery really work
}



#[test]
fn execute_ld() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0x7f8d
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0x7f8d,
		motherboard.cpu().reg16(Reg16::Bc).value()
	);
}

#[test]
fn execute_jr1() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		jr $ ; real jr
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code);

	assert_eq!(
		0x4000,
		motherboard.cpu().reg16(Reg16::Pc).value()
	);
}


#[test]
fn execute_jr2() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		jr 0x4010 ; real jr
		org 0x4010
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code);

	assert_eq!(
		0x4010,
		motherboard.cpu().reg16(Reg16::Pc).value()
	);
}

#[test]
/// This test currently ... I have to find if it comes from assembler or disassembler
fn execute_jr3() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x3ff0
label
		jr $ ; end of test

		org 0x4000
		jr label ; real jr
	"));

	let motherboard = execute_program(0x3ff0, 0x4000, code);

	assert_eq!(
		0x3ff0,
		motherboard.cpu().reg16(Reg16::Pc).value(),

		"{:x} != {:x}", 0x3ff0,motherboard.cpu().reg16(Reg16::Pc).value()
	);
}


#[test]
fn execute_out1() {

	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		jr $ ; end of test
	"));

	// wait (too much) to be sure screen mode changed
	let mut motherboard = execute_program(0x4000,0x4000, code);

	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().active_video_mode()
	);
	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().next_video_mode()
	);


	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0x7f8e
		jr $ ; end of test
	"));

	// wait (too much) to be sure screen mode changed
	let mut motherboard = execute_program(0x4000,0x4000, code);

	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().active_video_mode()
	);
	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().next_video_mode()
	);


	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0x7f8e
		out (c), c
		jr $ ; end of test
	"));

	// wait (too much) to be sure screen mode changed
	let mut motherboard = execute_program(0x4000,0x4000, code);

	assert_eq!(
		VideoMode::Mode0,
		motherboard.ga().active_video_mode()
	);
	assert_eq!(
		VideoMode::Mode2,
		motherboard.ga().next_video_mode()
	);
}


#[test]
// Test a palette change
fn execute_out2() {

	let code = cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0x7f00
		out (c), c ; select pen 0

		ld bc, 0x7f4b
		out (c), c ; select ink white

		nop
		jr $
		");

	let mut motherboard = execute_program(0x4000, 0x4000, code);
	assert_eq!(
		Ink::from(26),
		motherboard.ga().palette()[0]
	);
}



#[test]
fn execute_outd() {

	let code = cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0x7f00
		out (c), c ; select pen 0

		ld b, 0x7f + 1
		ld hl, table
		outd ; put white ink

		jr $ ; end of test
table
		db 0x4b
	");

	let mut motherboard = execute_program(0x4000, 0x4000, code);
	assert_eq!(
		Ink::from(26),
		motherboard.ga().palette()[0]
	);
}


#[test]
fn execute_ret() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		call end
return
		assert return == 0x4000 + 3
		jr $ ; really the final destination

		org 0x5000
end
		ret
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0x4000+3,
		motherboard.cpu().reg16(Reg16::Pc).value()
	);
}