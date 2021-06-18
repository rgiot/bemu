use amsrust::{chip::Chip, ga::GateArrayPin, motherboard::MotherboardBuilder, ppi::PpiPin};

/// CRTC module is supposed to generate properly the vsync
/// This file aims at testing all the wirings related to CRTC signal


#[test]
fn test_crtc_vsync_distribution() {
	// build a CPC that only execute NOP
	let mut motherboard = MotherboardBuilder::default()
												.disable_lower_rom()
												.disable_upper_rom()	
												.build();
	;


	while !motherboard.crtc().vsync() {
		motherboard.tick();
	}

	assert!(motherboard.crtc().vsync());
	assert!(motherboard.ga().get_pin(GateArrayPin::VSYNC));
	assert!(motherboard.ppi().get_pin(PpiPin::B0));

	while motherboard.crtc().vsync() {
		motherboard.tick();
	}
	assert!(!motherboard.crtc().vsync());
	assert!(!motherboard.ga().get_pin(GateArrayPin::VSYNC));
	assert!(!motherboard.ppi().get_pin(PpiPin::B0));

}

#[test]
fn test_ga_vsync_generation() {
		// build a CPC that only execute NOP
		let mut motherboard = MotherboardBuilder::default()
		.disable_lower_rom()
		.disable_upper_rom()	
		.build();
;


	while !motherboard.crtc().vsync() {
		motherboard.tick();
	}

	let res = motherboard.tick();
	assert!(res.1.vsync);


	while motherboard.crtc().vsync() {
		motherboard.tick();
	}

	let res = motherboard.tick();
	assert!(!res.1.vsync);

}