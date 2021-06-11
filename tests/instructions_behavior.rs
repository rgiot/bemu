///! This file contains some tests that aim at chacking the impact of the execution of an instraction on the machine

use amsrust::crtc::{Crtc, CrtcType};
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::z80::{HasValue, Reg16, Reg8, Z80, Z80Flags};
use amsrust::ga::{VideoMode, Ink};
use amsrust::debug::Monitor;
use cpclib_asm::preamble::{DataAccess, Expr, Mnemonic, Token};

/// Build a standard machine and fill it with the code  and execute it until arriving to jr $
/// Attention, do not put the code in area where ROMs are activated ...
fn execute_program(load: u16, exec: u16, test: &[u8]) -> Motherboard {
/*	dbg!(
		cpclib_asm::disass::disassemble(test)
	);*/

    let end_pattern = cpclib_macros::assemble!(" jr $");
	const MAX_OPERATIONS: usize = 10;
	let mut nb_operations = 0;

    let mut motherboard = MotherboardBuilder::default()
													.disable_lower_rom()
													.disable_upper_rom()
													.build();

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
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x8000);

	// execute it until it is finished
	loop {
		Monitor::monitor(&motherboard);
		motherboard.execute_next_opcode();

		let pc = motherboard.cpu().reg16(Reg16::Pc).value();

		// detect if it is the end
		let current_pattern = [motherboard.read_byte(pc + 0), motherboard.read_byte(pc + 1)];

		nb_operations += 1;

		// erroneous end
		if nb_operations == MAX_OPERATIONS {
			eprintln!("Maximum number of operations executed");
			break;
		}

		// normal end
		if current_pattern == *end_pattern {
			break;
		}
	}

	motherboard

}

/// Program is in 0x4000 stack is in 0x8000
fn execute_single_instruction(test: &[u8]) -> Motherboard {
	dbg!(
		cpclib_asm::disass::disassemble(test)
	);



    let mut motherboard = Motherboard::default();
	Monitor::monitor(&motherboard);

    motherboard.write_buffer(0x4000, test);
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x4000);
    motherboard.cpu_mut().reg16_mut(Reg16::Sp).set(0x8000);

	motherboard.execute_next_opcode();
	Monitor::monitor(&motherboard);

	motherboard

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
fn execute_call() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4012
		call end
		jr $ ; never reached if not buggy

		org 0x5000
end
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4012, 0x4012, code); 


	assert_eq!(
		0x12+3,
		motherboard.read_byte(0x8000-2)
	);

	assert_eq!(
		0x40,
		motherboard.read_byte(0x8000-1)
	);

	assert_eq!(
		0x4012+3,
		motherboard.read_word(0x8000-2)
	);

	assert_eq!(
		0x5000,
		motherboard.cpu().reg16(Reg16::Pc).value()
	);

	assert_eq!(
		motherboard.cpu().reg16(Reg16::Sp).value(),
		0x8000 - 2
	);
}


#[test]
fn execute_call_conditionnal_1(){
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4012
		xor a
		or a
		call z, end
		jr $ ; never reached if not buggy

		org 0x5000
end
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4012, 0x4012, code); 

	assert_eq!(
		0x5000,
		motherboard.cpu().reg16(Reg16::Pc).value()
	);

	assert_eq!(
		motherboard.cpu().reg16(Reg16::Sp).value(),
		0x8000 - 2
	);
}


#[test]
fn execute_call_conditionnal_2(){
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4012
		ld a, 0xff
		or a 
		call z, end
		jr $ ; never reached if not buggy

		org 0x5000
end
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4012, 0x4012, code); 

	let dest = 0x4012+2+1+3;
	assert_eq!(
		dest,
		motherboard.cpu().reg16(Reg16::Pc).value(),
		"{} != {}", dest, motherboard.cpu().reg16(Reg16::Pc).value()
	);

	assert_eq!(
		motherboard.cpu().reg16(Reg16::Sp).value(),
		0x8000
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
fn execute_inc_mem_hl() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld hl, data
		inc (hl)
		jr $ ; end of test
data
		db 3
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	let address = motherboard.cpu().reg16(Reg16::Hl).value();
	let value = motherboard.read_byte(address);

	assert_eq!(
		value,
		4
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
fn execute_ld1() {
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



	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0x7f00
		ld bc, 0x7f00
		ld bc, 0x7f00
		jr $ ; end of test
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0x7f00,
		motherboard.cpu().reg16(Reg16::Bc).value()
	);
}

#[test]
fn ld_hl_mem() {
let code = dbg!(cpclib_macros::assemble!(
	"
	org 0x4000
	ld hl, (mem)
	jr $ ; end of test
mem
	db 0xad
	db 0xde
"));

let motherboard = execute_program(0x4000, 0x4000, code); 

assert_eq!(
	0xdead,
	motherboard.cpu().reg16(Reg16::Hl).value(),
	"{:x} != {:x}", 0xdead, motherboard.cpu().reg16(Reg16::Hl).value()
);
}


#[test]
fn ld_sp_mem() {
let code = dbg!(cpclib_macros::assemble!(
	"
	org 0x4000
	ld sp, (mem)
	jr $ ; end of test
mem
	db 0xad
	db 0xde
"));

let motherboard = execute_program(0x4000, 0x4000, code); 

assert_eq!(
	0xdead,
	motherboard.cpu().reg16(Reg16::Hl).value(),
	"{:x} != {:x}", 0xdead, motherboard.cpu().reg16(Reg16::Sp).value()
);
}

#[test]
fn ld_a_mem_hl() {
let code = dbg!(cpclib_macros::assemble!(
	"
	org 0x4000
	ld hl, mem
	ld a, (hl)
	jr $ ; end of test
mem
	db 0xad
	db 0xde
"));

let motherboard = execute_program(0x4000, 0x4000, code); 

assert_eq!(
	0xad,
	motherboard.cpu().reg8(Reg8::A).value(),
	"{:x} != {:x}", 0xad, motherboard.cpu().reg8(Reg8::A).value()
);
}

#[test]
fn execute_ld_sp1() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		ld sp, 0xdead
	"));

	let motherboard = execute_single_instruction( code); 

	assert_eq!(
		0xdead,
		motherboard.cpu().reg16(Reg16::Sp).value()
	);
}


#[test]
fn execute_ld_sp2() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld sp, 0
		ld hl, 0xdead
		ld sp, hl
		jp $
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0xdead,
		motherboard.cpu().reg16(Reg16::Sp).value()
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
/// TODO debug the assembler that compute a wrong value
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

	let instruction = dbg!(motherboard.disassemble_instruction_at(0x4000));
	assert_eq!(
		instruction.address,
		0x4000
	);
	assert_eq!(
		instruction.token,
		Token::OpCode(
			Mnemonic::Jr,
			None,
			Some(DataAccess::Expression(Expr::RelativeDelta(
				((0x3ff0 - (0x4000+2)) & 0xff) as i8
			)))
		)
	);

	let instruction = dbg!(motherboard.disassemble_instruction_at(0x3ff0));
	assert_eq!(
		instruction.address,
		0x3ff0
	);
	assert_eq!(
		instruction.token,
		Token::OpCode(
			Mnemonic::Jr,
			None,
			Some(DataAccess::Expression(Expr::RelativeDelta(
				((-2) & 0xff) as i8
			)))
		)
	);



	assert_eq!(
		0x3ff0,
		motherboard.cpu().reg16(Reg16::Pc).value(),

		"{:x} != {:x}", 0x3ff0,motherboard.cpu().reg16(Reg16::Pc).value()
	);
}

#[test]
fn execute_jr3_assembler_bypassed1() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x3ff0
label
		jr $ ; end of test

		org 0x4000
		db 0x18 ; jr
		db -18 
	"));

	let motherboard = execute_program(0x3ff0, 0x4000, code);



	let instruction = dbg!(motherboard.disassemble_instruction_at(0x4000));
	assert_eq!(
		instruction.address,
		0x4000
	);
	assert_eq!(
		instruction.token,
		Token::OpCode(
			Mnemonic::Jr,
			None,
			Some(DataAccess::Expression(Expr::RelativeDelta(
				((0x3ff0 - (0x4000+2)) & 0xff) as i8
			)))
		)
	);

	assert_eq!(
		0x3ff0,
		motherboard.cpu().reg16(Reg16::Pc).value(),

		"{:x} != {:x}", 0x3ff0,motherboard.cpu().reg16(Reg16::Pc).value()
	);
}
#[test]
fn execute_jr3_assembler_bypassed2() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x3ff0
label
		jr $ ; end of test

		org 0x4000
		db 0x18 ; jr
		db label-after

after
	"));

	let motherboard = execute_program(0x3ff0, 0x4000, code);



	let instruction = dbg!(motherboard.disassemble_instruction_at(0x4000));
	assert_eq!(
		instruction.address,
		0x4000
	);
	assert_eq!(
		instruction.token,
		Token::OpCode(
			Mnemonic::Jr,
			None,
			Some(DataAccess::Expression(Expr::RelativeDelta(
				((0x3ff0 - (0x4000+2)) & 0xff) as i8
			)))
		)
	);

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
fn execute_push1() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		push hl
	"));

	let motherboard = execute_single_instruction( code); 

	assert_eq!(
		0x8000-2,
		motherboard.cpu().reg16(Reg16::Sp).value()
	);

}


#[test]
fn execute_pop() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld sp, stack
		pop hl
		jr $
stack
		dw 0xdead
		assert stack == 0x4000 + 6
	"));

	let motherboard = execute_program(0x4000, 0x4000,code); 

	assert_eq!(
		0xdead,
		motherboard.cpu().reg16(Reg16::Hl).value(),
		"{:x} != {:x}", 0xdead, motherboard.cpu().reg16(Reg16::Hl).value()
	);


	assert_eq!(
		0x4000 + 6 + 2,
		motherboard.cpu().reg16(Reg16::Sp).value()
	);
}

#[test]
fn execute_ret1() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld sp, stack
		ret
stack
		dw back

back
		assert back == 0x4000 + 6
		jr $

	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0x4000+6,
		motherboard.cpu().reg16(Reg16::Pc).value()
	);
}


#[test]
fn execute_ret2() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		call end
return
		assert return == 0x4000 + 3
		jr $ ; really the final destination

		org 0x4010
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