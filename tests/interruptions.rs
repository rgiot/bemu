#![feature(exclusive_range_pattern)]
#![feature(half_open_range_patterns)]
	
use amsrust::{debug::Monitor, motherboard::MotherboardBuilder, z80::{HasValue, Reg16}};
use cpclib_asm::preamble::Mnemonic;






#[test]
fn test_interruption_mode1_halt_di() {
	// build the CPC motherboard
	let mut motherboard = MotherboardBuilder::default()
		.disable_lower_rom()
		.disable_upper_rom()
		.enable_interruptions()
		.build();


	let infinite_loop = dbg!(cpclib_macros::assemble!(" di : halt"));
    let interrupt_handler = dbg!(cpclib_macros::assemble!(" ei : ret"));

	motherboard.write_buffer(0x38, interrupt_handler);
	motherboard.write_buffer(0x100, infinite_loop);
	motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);

	/// memory is full of nop
	let mut interrupt_count = 0;
	while motherboard.nops_counter().value() < 19968 {

		let pc = motherboard.cpu().reg16(Reg16::Pc).value();
		let token = motherboard.disassemble_next_instruction().token;

		if motherboard.nops_counter().value() < 64 {
			eprintln!(
				"nop {:#?}", motherboard.nops_counter()
			);
			Monitor::monitor(&motherboard);
		}

		match pc {
			0x100..=0x101 => {},
			_ => unreachable!("{:04x}={:?}", pc, token)
		}
		match token.mnemonic() {
			Some(Mnemonic::Di | Mnemonic::Halt) => {},
			_ => unreachable!("{:04x}={:?}", pc, token)
		}
		motherboard.execute_next_opcode();


		assert!(
			motherboard.ga().r52() <= 52
		);
	}

	assert_eq!(interrupt_count, 0);
}


#[test]
fn test_interruption_mode1_halt_ei() {
	// build the CPC motherboard
	let mut motherboard = MotherboardBuilder::default()
		.disable_lower_rom()
		.disable_upper_rom()
		.enable_interruptions()
		.build();


	let infinite_loop = dbg!(cpclib_macros::assemble!(" halt"));
    let interrupt_handler = dbg!(cpclib_macros::assemble!(" ei : ret"));

	motherboard.write_buffer(0x38, interrupt_handler);
	motherboard.write_buffer(0x100, infinite_loop);
	motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);


	motherboard.nops_counter_mut().reset();
	while motherboard.pc() != 0x38 && motherboard.nops_counter().value() < 19968{

		let pc = motherboard.cpu().reg16(Reg16::Pc).value();
		let token = motherboard.disassemble_next_instruction().token;

		if motherboard.nops_counter().value() < 64 {
			eprintln!(
				"nop {:#?}", motherboard.nops_counter()
			);
			Monitor::monitor(&motherboard);
		}

		match pc {
			0x100 | 0x38 => {},
			_ => unreachable!("{:04x}={:?}", pc, token)
		}
		match token.mnemonic() {
			Some(Mnemonic::Ei | Mnemonic::Halt) => {},
			_ => unreachable!("{:04x}={:?}", pc, token)
		}
		motherboard.execute_next_opcode();


		assert!(
			motherboard.ga().r52() <= 52
		);
	}

	assert!(
		dbg!(motherboard.nops_counter().value() + 3) <= dbg!(19968/6)
	);
}

#[test]
fn test_interruption_mode1() {
	// build the CPC motherboard
	let mut motherboard = MotherboardBuilder::default()
		.disable_lower_rom()
		.disable_upper_rom()
		.enable_interruptions()
		.build();


	let infinite_loop = dbg!(cpclib_macros::assemble!(" ei : jr $"));
    let interrupt_handler = dbg!(cpclib_macros::assemble!(" ei : ret"));

	motherboard.write_buffer(0x38, interrupt_handler);
	motherboard.write_buffer(0x100, infinite_loop);
	motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);

	/// memory is full of nop
	let mut interrupt_count = 0;
	while motherboard.nops_counter().value() < 19968 {

		let pc = motherboard.cpu().reg16(Reg16::Pc).value();
		let token = motherboard.disassemble_next_instruction().token;

		if dbg!(motherboard.nops_counter().value()) < 64 {
			eprintln!(
				"nop {:#?}", motherboard.nops_counter()
			);
			Monitor::monitor(&motherboard);
		}

		match pc {
			0x38..0x40=> {},
			0x100..0x103 => {},
			_ => unreachable!("{:04x}={:?}", pc, token)
		}
		match token.mnemonic() {
			Some(Mnemonic::Ei | Mnemonic::Ret | Mnemonic::Jr) => {},
			_ => unreachable!("{:04x}={:?}", pc, token)
		}
		motherboard.execute_next_opcode();
		if motherboard.cpu().reg16(Reg16::Pc).value() == 0x38 {
			interrupt_count += 1;
		}

		assert!(
			motherboard.ga().r52() <= 52
		);
	}

	assert_eq!(interrupt_count, 6);
}