/// Check that the CRTC detection methods DO NOT detect an emulator but the right CRTC emulated
/// 
/// 


use amsrust::crtc::{CrtcType, Crtc};
use amsrust::z80::{Z80, Reg16, Reg8, HasValue};
use amsrust::motherboard::MotherboardBuilder;

use cpclib_asm::preamble::*;
use cpclib_macros;

enum CrtcTest {
	MadramAmslive,
	Offset
}

impl CrtcTest {

	/// Return the address where to assemble/load/run the test
	fn address(&self) -> u16 {
		match self {
			CrtcTest::MadramAmslive => 0x9001,
			_ => unimplemented!()
		}
	}

	/// Return the assembled test
	fn test(&self) -> (&[u8], u16) {
		match self {
			CrtcTest::MadramAmslive => (
				cpclib_macros::assemble!("madram_crtc_test.asm"),
				self.address()
			),
			_ => unimplemented!()
		}
	}
}


fn test_crtc(crtc_type: CrtcType, crtc_test: CrtcTest) {

	// build the CPC motherboard
	let mut motherboard = MotherboardBuilder::default()
								.set_crtc(crtc_type)
								.build();

	// build the test
	let (test, addr) = crtc_test.test();
	let end_pattern = cpclib_macros::assemble!(" jr $");

	// inject it in memory
	motherboard.write_buffer(addr, &test);

	// execute it until it is finished
	loop {
		motherboard.execute_next_opcode();

		// detect if it is the end
		let pc = motherboard.cpu().reg16(Reg16::Pc).value();
		let current_pattern = [
			motherboard.read_byte(pc + 0),
			motherboard.read_byte(pc + 1)
		];

		if current_pattern == *end_pattern {
			break
		}
	}

	// retreive the crtc number
	let obtained_crtc = motherboard.cpu().reg8(Reg8::A).value();
	let expected_crtc = crtc_type as u8;

	// and make the comparison
	assert_eq!(
		obtained_crtc,
		expected_crtc
	)

}


#[test]
fn crtc0_madram() {
	test_crtc(CrtcType::Crtc0, CrtcTest::MadramAmslive);
}