use amsrust::{chip::{Activable, Chip, ReadDataChip}, crtc::CrtcPin, debug::Monitor, ga::GateArrayPin, motherboard::MotherboardBuilder, ppi::{Ppi, PpiPin}};

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

;


	while !motherboard.crtc().vsync() {
		motherboard.tick();
	}
	assert!(check_vsync_from_ppi(motherboard.ppi_mut()));


	while motherboard.crtc().vsync() {
		motherboard.tick();
	}
	assert!(!check_vsync_from_ppi(motherboard.ppi_mut()));


}

fn check_vsync_from_ppi(ppi: &mut Ppi) -> bool {
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
	}

#[test]
fn test_ppi_vsync_timing() {
	let mut motherboard = MotherboardBuilder::default()
												.disable_lower_rom()
												.disable_upper_rom()	
												.build();	

	while !check_vsync_from_ppi(motherboard.ppi_mut()) {
		motherboard.tick();
	}

	let mut nb_ticks = 0;
	// consume time within the vsync signal
	while check_vsync_from_ppi(motherboard.ppi_mut()) {
		nb_ticks += 1;
		motherboard.tick();
	}	

	// consume time when vysnc is not generated
	while !check_vsync_from_ppi(motherboard.ppi_mut()) {
		nb_ticks += 1;
		motherboard.tick();
	}		

	// assume one frame as elapsed
	assert_eq!(
		nb_ticks,
		19968*16
	);
}


#[test]
fn test_z80_vsync_activated() {
	let mut motherboard = MotherboardBuilder::default()
	.disable_lower_rom()
	.disable_upper_rom()	
	.build();	

	// Wait new vsync
	while !check_vsync_from_ppi(motherboard.ppi_mut()) {
		motherboard.tick();
	}
	assert!(check_vsync_from_ppi(motherboard.ppi_mut()), "PPI does not provide vsync");
	assert!(motherboard.crtc().get_pin(CrtcPin::VS));

	let mut motherboard = MotherboardBuilder::default()
		.disable_lower_rom()
		.disable_upper_rom()	
		.build();	

		let prog = dbg!(cpclib_macros::assemble!("
		org 0x4000
		ld b, 0xf5
		in a, (c)
		rra
end
	assert end == 0x4000 + (2+2+1)
	"));
	motherboard.set_pc(0x4000);
	motherboard.set_sp(0x4000);
	motherboard.write_buffer(0x4000, prog);

	while motherboard.pc() != 0x4000 + (2+2+1) {
		Monitor::monitor(&motherboard);
		motherboard.execute_next_opcode();
	}

	assert!(motherboard.crtc().get_pin(CrtcPin::VS), "Vsync stopped too soon");
	assert!(motherboard.ppi().get_pin(PpiPin::B0));
	assert!(motherboard.cpu().test_flag(cpclib_asm::preamble::FlagTest::C), "CPU has not retrieved the vsync signal");

}

#[test]
fn test_z80_vsync_full_frame() {
	let mut motherboard = MotherboardBuilder::default()
	.disable_lower_rom()
	.disable_upper_rom()	
	.build();	

	// Wait new vsync
	while !check_vsync_from_ppi(motherboard.ppi_mut()) {
		motherboard.tick();
	}
	
	// Wait its end
	let mut nb_ticks = 0;
	// consume time within the vsync signal
	while check_vsync_from_ppi(motherboard.ppi_mut()) {
		nb_ticks += 1;
		motherboard.tick();
	}	

	// inject vsync test codec
	let prog = dbg!(cpclib_macros::assemble!("
		org 0x4000
		ld b, 0xf5
loop
		in a, (c)
		rra
		jr nc, loop
end
	assert end == 0x4000 + (2+2+1+2)
	"));
	motherboard.set_pc(0x4000);
	motherboard.set_sp(0x4000);
	motherboard.write_buffer(0x4000, prog);
	

	// wait the detection of vsync by the z80 code
	// even if z80 timings are wrong the test shall pass as we are sticked with the ticks
	loop {
		if motherboard.cpu().is_activated() &&  motherboard.is_aligned_with_new_instruction(){
			println!();
			Monitor::monitor(&motherboard);

			if motherboard.pc() == 0x4000 + (2+2+1+2) {
				break;
			}
		}

		nb_ticks += 1;
		motherboard.tick();

		// safe guard to not indefinitively loop
		assert!(nb_ticks < (19968 + 64 *2)*16);
	}
	

	assert_eq!(
		motherboard.pc(),
		0x4000 + (2+2+1+2) 
	);

	// assume one frame as elapsed
	assert_eq!(
		nb_ticks,
		19968*16
	);
}