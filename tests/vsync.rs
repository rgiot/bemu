use amsrust::{chip::{Chip, ReadDataChip}, ga::GateArrayPin, motherboard::MotherboardBuilder, ppi::{Ppi, PpiPin}};

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



#[test]
fn test_ppi_vsync_test() {
	// build a CPC that only execute NOP
	let mut motherboard = MotherboardBuilder::default()
												.disable_lower_rom()
												.disable_upper_rom()	
												.build();
	;

	let test_vsync = |ppi: &mut Ppi| {
		// request in 0xf5
		ppi.off(PpiPin::Neg_CS);
		ppi.on(PpiPin::Neg_WR);
		ppi.off(PpiPin::Neg_RD);
		ppi.on(PpiPin::InA0);
		ppi.off(PpiPin::InA1);

		// execute it
		ppi.execute_tick();

		// obtain the appropriate value
		let val = ppi.get_data_from_pins();

		// check that vsync value is set
		val & 1 != 0
	};


	while !motherboard.crtc().vsync() {
		motherboard.tick();
	}
	assert!(test_vsync(motherboard.ppi_mut()));


	while motherboard.crtc().vsync() {
		motherboard.tick();
	}
	assert!(!test_vsync(motherboard.ppi_mut()));


}