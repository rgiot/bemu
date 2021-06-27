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
	Monitor::monitor(&motherboard);
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
fn execute_dec_mem_hl() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld hl, mem
		dec (hl)
		jr $ ; end of test
		org 0x4010
mem db 2
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		1,
		motherboard.read_byte(0x4010)
	);
}


#[test]
fn execute_ex_mem_sp_hl() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld sp, stack
		ld hl, 0xbeef
		ex (sp), hl
		jr $ ; end of test
		org 0x4010

stack
		dw 0xdead
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0xdead,
		motherboard.cpu().reg16(Reg16::Hl).value(),
		"{:x} != {:x}", 0xdead,
		motherboard.cpu().reg16(Reg16::Hl).value(),
	);
	assert_eq!(
		0xbeef,
		motherboard.read_word(0x4010),
		"{:x} != {:x}",0xbeef,
		motherboard.read_word(0x4010),
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
fn ld_a_mem_ix_delta() {
let code = dbg!(cpclib_macros::assemble!(
	"
	org 0x4000
	ld ix, mem
	ld a, (ix+1)
	ld b, (ix-1)
	jr $ ; end of test

	org 0x4010
	db 0xbe
mem
	db 0xad
	db 0xde
"));

let motherboard = execute_program(0x4000, 0x4000, code); 

assert_eq!(
	0xde,
	motherboard.cpu().reg8(Reg8::A).value()
);

assert_eq!(
	0xbe,
	motherboard.cpu().reg8(Reg8::B).value()
);
}



#[test]
fn ld_mem_hl_a() {
let code = dbg!(cpclib_macros::assemble!(
	"
	org 0x4000
	ld hl, mem
	ld a, 0xff
	ld (hl), a
	jr $ ; end of test
	org 0x4010
mem
	db 0x00
"));

let motherboard = execute_program(0x4000, 0x4000, code); 

assert_eq!(
	0xff,
	motherboard.read_byte(0x4010)
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
		ld sp, 0xbeef
		ld hl, 0xdead
		ld sp, hl
		jr $
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
fn execute_ldd() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0
		ld de, output
		ld hl, input
		ldd
		jr $

		org 0x4010
output db 1

		org 0x4020
input db 2
	"));

	// wait (too much) to be sure screen mode changed
	let mut motherboard = execute_program(0x4000,0x4000, code);

	assert_eq!(
		motherboard.bc(),
		0xffff,
		"0x{:x} != 0x{:x}", motherboard.bc(),	0xffff
	);
	assert_eq!(
		motherboard.hl(),
		0x4020-1
	);
	assert_eq!(
		motherboard.de(),
		0x4010-1
	);
	assert_eq!(
		motherboard.read_byte(0x4010),
		motherboard.read_byte(0x4020),
	);
	assert_eq!(
		motherboard.read_byte(0x4010),
		2,
	);
}



#[test]
fn execute_ldir() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 2
		ld de, output
		ld hl, input
		ldir
		jr $

		org 0x4010
output db 0
		db 0
		db 0

		org 0x4020
input db 1
		db 2
		db 3
	"));

	// wait (too much) to be sure screen mode changed
	let mut motherboard = execute_program(0x4000,0x4000, code);

	assert_eq!(
		motherboard.bc(),
		0x00,
		"0x{:x} != 0x{:x}", motherboard.bc(),	0xffff
	);
	assert_eq!(
		motherboard.hl(),
		0x4020+2
	);
	assert_eq!(
		motherboard.de(),
		0x4010+2
	);
	assert_eq!(
		motherboard.read_byte(0x4010),
		motherboard.read_byte(0x4020),
	);
	assert_eq!(
		motherboard.read_byte(0x4011),
		motherboard.read_byte(0x4021),
	);
	assert_ne!(
		motherboard.read_byte(0x4012),
		motherboard.read_byte(0x4022),
	);
}



#[test]
fn execute_lddr() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 2
		ld de, output
		ld hl, input
		lddr
		jr $

		org 0x4010
 		db 0
		db 0
output		db 0

		org 0x4020
	 db 1
		db 2
input		db 3
	"));

	// wait (too much) to be sure screen mode changed
	let mut motherboard = execute_program(0x4000,0x4000, code);

	assert_eq!(
		motherboard.bc(),
		0x00,
		"0x{:x} != 0x{:x}", motherboard.bc(),	0xffff
	);
	assert_eq!(
		motherboard.hl(),
		0x4020
	);
	assert_eq!(
		motherboard.de(),
		0x4010
	);
	assert_eq!(
		motherboard.read_byte(0x4012),
		motherboard.read_byte(0x4022),
	);
	assert_eq!(
		motherboard.read_byte(0x4011),
		motherboard.read_byte(0x4021),
	);
	assert_ne!(
		motherboard.read_byte(0x4010),
		motherboard.read_byte(0x4020),
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
// Test a palette change
fn execute_out3() {

	let code = cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0xbc00 + 6
		out (c), c ; select register 6

		nop ; just to be sure it is taken into account to avoid false alarms due to wrong timings
		jr $
		");

	let mut motherboard = execute_program(0x4000, 0x4000, code);
	assert_eq!(
		6,
		motherboard.crtc().ar()
	);
}


#[test]
// Test a palette change
fn execute_out4() {

	let code = cpclib_macros::assemble!(
		"
		org 0x4000
		ld bc, 0xbc00 + 6
		out (c), c ; select register 6

		ld bc, 0xbd00 + 10
		out (c), c 

		nop ; just to be sure it is taken into account to avoid false alarms due to wrong timings
		jr $
		");

	let mut motherboard = execute_program(0x4000, 0x4000, code);
	assert_eq!(
		10,
		motherboard.crtc().r6()
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
		org 0x5000
table
		db 0x4b
	");

	let mut motherboard = execute_program(0x4000, 0x4000, code);
	assert_eq!(
		Ink::from(26),
		motherboard.ga().palette()[0]
	);
	assert_eq!(
		motherboard.cpu().b(),
		0x7f
	);

	assert_eq!(
		motherboard.cpu().hl(),
		0x5000-1
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
		motherboard.cpu().reg16(Reg16::Sp).value(),
		"{:x} != {:x}", 0x8000-2,
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
#[test]
fn execute_rlc_a() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld a, 0b10000001
	rlc a
	jr $
");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0b11,
		motherboard.a()
	);

	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::C));
}


#[test]
fn execute_rlc_mem_hl() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld hl, 0x4010
	rlc (hl)
	jr $
	org 0x4010
	db 0b10000001
");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0b11,
		motherboard.read_byte(0x4010)
	);

	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::C));
}

#[test]
fn execute_rlc_indexed() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld ix, data
	rlc (ix+0)
	jr $
	org 0x4010
data
	db 0b10000001");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0b11,
		motherboard.read_byte(0x4010)
	);

	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::C));
}

#[test]
fn execute_rra1() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld hl, 0x0100
	push hl
	pop af

	rra
	");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0,
		motherboard.a()
	);

	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::C));
}

#[test]
fn execute_rra2() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld hl, 0x0200
	push hl
	pop af

	rra
	");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		1,
		motherboard.a()
	);

	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::NC));
}


#[test]
fn execute_rra3() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld hl, 0x0200
	push hl
	pop af
	scf
	rra
	");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0b10000001,
		motherboard.a()
	);

	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::NC));
}

#[test]
fn execute_rst() {
	let code = dbg!(cpclib_macros::assemble!("

	org 0x4000
	ld a, 0xc9
	ld (0x10), a

	rst 0x10
	org 0x4008
	jr $



	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0x4008,
		motherboard.pc()
	);

}


#[test]
fn execute_srl1() {
	let code = dbg!(cpclib_macros::assemble!("
	org 0x4000
	ld hl, data
	srl (hl)
	jr $
	org 0x4010
data db 1
	"));

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0b00000000,
		motherboard.read_byte(0x4010)
	);

	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::C));
}

#[test]
fn execute_srl2() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld a, 1
	srl a
	jr $
	");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0b00000000,
		motherboard.a()
	);

	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::C));
}

#[test]
fn execute_sbc1() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld hl, 3
	ld de, 2
	or a
	sbc hl, de
	jr $
	");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		1,
		motherboard.hl()
	);
}

	#[test]
	fn execute_sbc2() {
		let code = cpclib_macros::assemble!("
		org 0x4000
		ld hl, 3
		ld de, 2
		scf
		sbc hl, de
		jr $
		");
	
		let motherboard = execute_program(0x4000, 0x4000, code); 
	
		assert_eq!(
			0,
			motherboard.hl()
		);
	}



	#[test]
	fn execute_sub1() {
		let code = cpclib_macros::assemble!("
		org 0x4000
		ld a, 0xff
		sub 0xff
		jr $
		");
	
		let motherboard = execute_program(0x4000, 0x4000, code); 
	
		assert_eq!(
			0,
			motherboard.a()
		);
	}

#[test]
fn execute_set() {
	let code = cpclib_macros::assemble!("
	org 0x4000
	ld hl, data
	set 7, (hl)
	jr $
	org 0x4010
data db 1
	");

	let motherboard = execute_program(0x4000, 0x4000, code); 

	assert_eq!(
		0b10000001,
		motherboard.read_byte(0x4010)
	);
}