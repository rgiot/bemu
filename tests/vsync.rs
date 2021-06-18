use amsrust::{chip::Chip, ga::GateArrayPin, motherboard::MotherboardBuilder};

/// CRTC module is supposed to generate properly the vsync
/// This file aims at testing all the wirings related to CRTC signal

/// Here we ant to test all stuffs related to GA and its VSYNC generation
#[test]
fn test_ga_vsync() {
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
}