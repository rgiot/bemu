use amsrust::{debug::Monitor, motherboard::MotherboardBuilder, z80::{HasValue, Reg16}};
use amsrust::crtc;

#[test]
fn test_crtc_registers_modification(){
	let mut motherboard = MotherboardBuilder::default()
		.disable_lower_rom()
		.disable_upper_rom()	
		.build();	

	let prog = dbg!(cpclib_macros::assemble!("
		org 0x4000
		ld bc, 0xbc00 + 6
		out (c), c
		ld bc, 0xbd00 + 0
		out (c), c
		jr $
	"));	

	motherboard.set_pc(0x4000);
	motherboard.set_sp(0x4000);
	motherboard.write_buffer(0x4000, prog);
	

	let end_pattern = cpclib_macros::assemble!(" jr $");
	assert_eq!(
		motherboard.crtc().get_r6(),
		25
	);
	loop {
		Monitor::monitor(&motherboard);
		motherboard.execute_next_opcode();

		let pc = motherboard.cpu().reg16(Reg16::Pc).value();

		// detect if it is the end
		let current_pattern = [motherboard.read_byte(pc + 0), motherboard.read_byte(pc + 1)];

		if current_pattern == *end_pattern {
			break;
		}
	}
	Monitor::monitor(&motherboard);
	

	assert_eq!(
		motherboard.crtc().get_r6(),
		0
	);
}