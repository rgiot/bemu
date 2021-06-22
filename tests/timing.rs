use amsrust::{crtc::{Crtc, CrtcType}, motherboard};
use amsrust::motherboard::Motherboard;
use amsrust::z80::{HasValue, Reg16, Reg8, Z80};
use amsrust::ga::{VideoMode, Sync, RgbLevels};

/// Build a standard machine and fill it with the code  and execute it until arriving to jr $
/// Attention, do not put the code in area where ROMs are activated ...
fn execute_instruction(load: u16, test: &[u8]) -> (Vec<(RgbLevels, Sync)> , Motherboard) {
	dbg!(
		cpclib_asm::disass::disassemble(test)
	);


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
    motherboard.cpu_mut().reg16_mut(Reg16::Pc).set(load);

	// execute it until it is finished
	let output = motherboard.execute_next_opcode();

	(output, motherboard)

}


#[test]
fn timing_nop() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		nop
	"));

	let (output, motherboard) = dbg!(execute_instruction(0x4000, code)); 

	dbg!(output.len(), motherboard.t_state_counter());

	assert_eq!(
		motherboard.cpu().reg16(Reg16::Pc).value(),
		0x4001
	);
	assert_eq!(
		motherboard.t_state_counter().value(),
		1*4
	);

	assert_eq!(
		motherboard.t_state_counter().value()%4,
		0
	);

	assert_eq!(
		motherboard.nops_counter().value(),
		1
	);

	assert_eq!(
		output.len(),
		16
	);
}



#[test]
fn timing_call() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		call 0xdead
	"));

	let (output, motherboard) = dbg!(execute_instruction(0x4000, code)); 

	dbg!(output.len(), motherboard.t_state_counter(), motherboard.nops_counter());

	assert_eq!(
		motherboard.cpu().reg16(Reg16::Pc).value(),
		0xdead
	);

	assert_eq!(
		motherboard.nops_counter().value(),
		5
	);
}



#[test]
fn timing_ldd() {
	let code = dbg!(cpclib_macros::assemble!(
		"
		ldd
	"));

	let (output, motherboard) = execute_instruction(0x4000, code); 

	dbg!(output.len(), motherboard.t_state_counter(), motherboard.nops_counter());

	assert_eq!(
		motherboard.nops_counter().value(),
		5
	);
}