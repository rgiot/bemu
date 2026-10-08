#![feature(exclusive_range_pattern)]
#![feature(half_open_range_patterns)]
	
use amsrust::{debug::Monitor, motherboard::{self, MotherboardBuilder}, z80::{HasValue, Reg16}};
use cpclib_asm::preamble::ListingElement;
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
	assert!(motherboard.cpu().iff1());

	let infinite_loop = dbg!(cpclib_macros::assemble!(" halt : jr $"));
    let interrupt_handler = dbg!(cpclib_macros::assemble!(" ei : ret"));

	motherboard.write_buffer(0x38, interrupt_handler);
	motherboard.write_buffer(0x100, infinite_loop);
	motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);


	motherboard.nops_counter_mut().reset();
	assert!(motherboard.cpu().iff1());
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
		dbg!(motherboard.nops_counter().value() ) <= dbg!(19968/6 +3)
	);

	assert!(
		dbg!(motherboard.nops_counter().value() ) >= dbg!(19968/6 -3)
	);

	motherboard.nops_counter_mut().reset(); // reset it for next frame (reset is done a bit too late however)

	let token = motherboard.disassemble_next_instruction().token;
	assert_eq!(
		token.mnemonic(),
		Some(&Mnemonic::Ei)
	);
	assert_eq!(
		motherboard.pc(),
		0x38
	);
	motherboard.execute_next_opcode();

	let token = motherboard.disassemble_next_instruction().token;
	assert_eq!(
		motherboard.pc(),
		0x39
	);
	assert_eq!(
		token.mnemonic(),
		Some(&Mnemonic::Ret)
	);
	motherboard.execute_next_opcode();

	let token = motherboard.disassemble_next_instruction().token;
	assert_eq!(
		motherboard.pc(),
		0x101
	);
	assert_eq!(
		token.mnemonic(),
		Some(&Mnemonic::Jr)
	);


	// Replace the JR by a halt
	motherboard.write_buffer(0x101, infinite_loop);

	// Check that next interruption appears at the right time
	assert!(motherboard.cpu().iff1());
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
			0x101 | 0x38 => {},
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
		dbg!(motherboard.nops_counter().value() ) <= dbg!(19968/6 +3)
	);
	assert!(
		dbg!(motherboard.nops_counter().value() ) >= dbg!(19968/6 -3)
	);

}

#[test]
fn test_interruption_mode1_one_frame() {
	// build the CPC motherboard
	let mut motherboard = MotherboardBuilder::default()
		.disable_lower_rom()
		.disable_upper_rom()
		.enable_interruptions()
		.build();
		assert!(motherboard.cpu().iff1());

	let infinite_loop = dbg!(cpclib_macros::assemble!(" halt : halt : halt : halt : halt : halt "));
    let interrupt_handler = dbg!(cpclib_macros::assemble!(" ei : ret"));

	// RAM does not power up zeroed (it holds #00/#FF blocks): make the memory
	// walked beyond the test's own code a field of nops
	motherboard.write_buffer(0x0, &[0u8; 0x2100]);
	motherboard.write_buffer(0x38, interrupt_handler);
	motherboard.write_buffer(0x100, infinite_loop);
	motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);

	/// memory is full of nop
	let mut interrupt_count = 0;
	let mut prev_interr_time = None;
	let mut gaps = Vec::new();
	motherboard.nops_counter_mut().reset();
	assert!(motherboard.cpu().iff1());

	while motherboard.nops_counter().value() < 19968  {

		let pc = motherboard.cpu().reg16(Reg16::Pc).value();
		let token = motherboard.disassemble_next_instruction().token;

		match pc {
			0x38..0x40=> {
						},
			0x100..0x106 => {
				assert!(motherboard.cpu().iff1());
			},
			// A maskable interrupt accepted during HALT advances PC past
			// it before pushing the return address (real Z80 behaviour),
			// so RET from the 6th (and last) interrupt lands here, one
			// past the last of the six HALTs - from here on it's just the
			// zero-filled ("full of nop") memory beyond the test's own
			// code, walked straight through NOP by NOP until the loop's
			// own 19968-nop budget runs out.
			0x106..=0x2000 => {
				assert!(motherboard.cpu().iff1());
			},
			_ => unreachable!("{:04x}={:?}", pc, token)
		}
		match token.mnemonic() {
			Some(Mnemonic::Ei | Mnemonic::Ret | Mnemonic::Halt | Mnemonic::Nop) => {},
			_ => unreachable!("{:04x}={:?}", pc, token)
		}
		motherboard.execute_next_opcode();

		if motherboard.cpu().reg16(Reg16::Pc).value() == 0x38 {
			interrupt_count += 1;
			let nops = motherboard.nops_counter().value();
			if let Some(time) = prev_interr_time {
				gaps.push(nops - time);
			}
			prev_interr_time = Some(nops);
		}

		assert!(
			motherboard.ga().r52() <= 52
		);
	}

	// 312 lines / 52 lines-per-interrupt = 6 interrupts per 50Hz frame -
	// the well-known real-hardware count (the earlier "XXX check if it is
	// not supposed to be 6" doubt was warranted: the previously-inverted
	// R52-bit5 condition silently dropped one of the six by suppressing
	// the interrupt right at the VSYNC boundary).
	assert_eq!(interrupt_count, 6);
	// Gaps between interrupts are normally exactly 52 lines (19968/6 = 3328
	// nops) apart, but ACCC §27.3.2's R52-bit5 mechanism legitimately
	// shortens the one gap that straddles a VSYNC boundary - confirmed for
	// real (not just by Pierre Guerrier's GAINTS.HTM, which already
	// documented spacing as low as ~20 lines as legitimate here) against
	// AMSpiriT's own Shaker self-test, which produces real, matching-its-
	// own-expectation interrupts right at this transition - see
	// `validation/Shaker_CSL/MODULE_B/reference_screenshots/
	// BR_B_r52reset.png` and the `GateArray::manage_hsync_end` comment
	// this corresponds to. 2176 nops = 34 lines, the shortened gap here.
	assert_eq!(gaps, vec![3328, 3328, 3328, 2176, 3328]);
}

/// An interrupt must never be accepted between the compute step of a
/// read-modify-write instruction and its write cycle: that used to leave
/// `DEC (HL)`'s flags computed but its result never written back, corrupting
/// memory (in AMSDOS/the firmware key queue: a phantom key).
#[test]
fn test_interruption_never_splits_a_read_modify_write_instruction() {
	let mut motherboard = MotherboardBuilder::default()
		.disable_lower_rom()
		.disable_upper_rom()
		.enable_interruptions()
		.build();

	// &0100: LD HL,&8000
	// &0103: DEC (HL) ; INC (HL) ; JR &0103    (byte at &8000 starts at 0)
	motherboard.write_buffer(0x100, &[0x21, 0x00, 0x80, 0x35, 0x34, 0x18, 0xfc]);
	motherboard.write_buffer(0x38, cpclib_macros::assemble!(" ei : ret"));
	motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(0x100);
	motherboard.nops_counter_mut().reset();

	let mut interrupt_count = 0;
	while motherboard.nops_counter().value() < 19968 * 3 {
		motherboard.execute_next_opcode();

		// DEC (HL) then INC (HL): the byte is only ever 0 or 0xff. A lost
		// write leaves it at 1 or 0xfe after the INC / DEC pair.
		let value = motherboard.read_byte(0x8000);
		assert!(
			value == 0x00 || value == 0xff,
			"byte at &8000 is {value:#04x} - a write was lost around an interrupt"
		);

		if motherboard.cpu().reg16(Reg16::Pc).value() == 0x38 {
			interrupt_count += 1;
		}
	}

	// interrupts kept being accepted (they used to stop for good after the
	// first one that landed on a multi-cycle instruction)
	assert!(interrupt_count >= 12, "only {interrupt_count} interrupts were taken");
}
