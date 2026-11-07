use amsrust::{
    chip::{Activable, Chip, ReadDataChip},
    crtc::CrtcPin,
    debug::Monitor,
    ga::GateArrayPin,
    motherboard::MotherboardBuilder,
    ppi::{Ppi, PpiPin},
};

/// A CPC whose RAM holds only NOPs, the ROMs off: the RAM powers up as #00/#FF blocks, whose
/// RST &38s would loop at &0038 (&FF) and finish over a PC the test sets.
fn nop_machine() -> amsrust::motherboard::Motherboard {
    let mut motherboard = MotherboardBuilder::default()
        .disable_lower_rom()
        .disable_upper_rom()
        .build();
    motherboard.write_buffer(0x0000, &[0; 0x8000]);
    motherboard.write_buffer(0x8000, &[0; 0x8000]);
    motherboard
}

/// CRTC module is supposed to generate properly the vsync
/// This file aims at testing all the wirings related to CRTC signal

#[test]
fn test_crtc_vsync_distribution() {
    // build a CPC that only execute NOP
    let mut motherboard = nop_machine();

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

/// The monitor's C-VSYNC is the Gate Array's (ACCC 16.2.3): it begins at the end of the 2nd HSYNC of
/// the CRTC's VSYNC and ends at the end of the 6th, while the CRTC's VSYNC (16 lines) goes on.
#[test]
fn test_ga_vsync_generation() {
    let mut motherboard = nop_machine();

    while !motherboard.crtc().vsync() {
        motherboard.tick();
    }
    let mut hsync_ends = 0;
    let mut hsync_before = motherboard.crtc().hsync();
    let mut c_vsync_seen = Vec::new();
    while motherboard.crtc().vsync() {
        let (_, sync) = motherboard.tick();
        let hsync = motherboard.crtc().hsync();
        if hsync_before && !hsync {
            hsync_ends += 1;
        }
        hsync_before = hsync;
        c_vsync_seen.push((hsync_ends, sync.vsync));
    }
    for (ends, c_vsync) in c_vsync_seen {
        match ends {
            0 | 1 => assert!(!c_vsync, "no C-VSYNC before the end of the 2nd HSYNC"),
            3..=5 => assert!(c_vsync, "C-VSYNC from the 2nd to the 6th HSYNC end"),
            7.. => assert!(!c_vsync, "no C-VSYNC after the 6th HSYNC end"),
            _ => {} // the GA hears of the HSYNC end a few ticks after the CRTC
        }
    }
}

#[test]
fn test_ppi_vsync_test() {
    // build a CPC that only execute NOP
    let mut motherboard = nop_machine();

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
    let mut motherboard = nop_machine();

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
    assert_eq!(nb_ticks, 19968 * 16);
}

#[test]
fn test_z80_vsync_activated() {
    let mut motherboard = nop_machine();

    // Wait new vsync
    while !check_vsync_from_ppi(motherboard.ppi_mut()) {
        motherboard.tick();
    }

    // The next 128  nops, we still have a vsync
    for _ in 0..2 * 64 * 16 {
        motherboard.tick();
        assert!(
            check_vsync_from_ppi(motherboard.ppi_mut()),
            "PPI does not provide vsync"
        );
        assert!(motherboard.crtc().get_pin(CrtcPin::VS));
        Monitor::monitor_crtc_counters(motherboard.crtc());
    }

    let prog = dbg!(cpclib_macros::assemble!(
        "
		org 0x4000
		ld b, 0xf5
		in a, (c)
	done:
		rra
	"
    ));
    // on an instruction's boundary, or the one in flight finishes over the PC set
    motherboard.tick_until_next_instruction();
    motherboard.set_pc(0x4000);
    motherboard.set_sp(0x4000);
    motherboard.write_buffer(0x4000, prog);

    while motherboard.pc() != 0x4000 + (2 + 2) {
        Monitor::monitor(&motherboard);
        motherboard.execute_next_opcode();
        dbg!(motherboard.ppi().get_data_b_from_pins());
        dbg!(motherboard.ppi().get_data_from_pins());
    }
    Monitor::monitor(&motherboard);
    assert!(
        motherboard.crtc().get_pin(CrtcPin::VS),
        "Vsync stopped too soon"
    );
    assert!(motherboard.ppi().get_pin(PpiPin::B0));
    assert!(motherboard.ppi().is_port_b_addressed());
    assert_eq!(dbg!(motherboard.ppi().get_data_b_from_pins()) & 1, 1);
    assert_eq!(
        dbg!(motherboard.cpu().a()) & 1,
        1,
        "CPU did not get PPI sync signal"
    );

    motherboard.execute_next_opcode();
    assert!(motherboard.cpu().flag(amsrust::z80::Z80Flags::C));

    // Wait end of vsync
    while check_vsync_from_ppi(motherboard.ppi_mut()) {
        motherboard.tick();
    }

    // redo the vsync test
    motherboard.tick_until_next_instruction();
    motherboard.set_pc(0x4000);
    while motherboard.pc() != 0x4000 + (2 + 2) {
        Monitor::monitor(&motherboard);
        motherboard.execute_next_opcode();
        dbg!(motherboard.ppi().get_data_b_from_pins());
        dbg!(motherboard.ppi().get_data_from_pins());
    }
    Monitor::monitor(&motherboard);

    assert!(
        !motherboard.crtc().get_pin(CrtcPin::VS),
        "Vsync stopped too soon"
    );
    assert!(!motherboard.ppi().get_pin(PpiPin::B0));
    assert!(motherboard.ppi().is_port_b_addressed());
    assert_eq!(dbg!(motherboard.ppi().get_data_b_from_pins()) & 1, 0);
    assert_eq!(dbg!(motherboard.cpu().a()) & 1, 0);
}

#[test]
fn test_z80_vsync_full_frame() {
    let mut motherboard = nop_machine();

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
    let prog = dbg!(cpclib_macros::assemble!(
        "
		org 0x4000
		ld b, 0xf5
	loop:
		in a, (c)
		rra
		jr nc, loop
	done:
	"
    ));
    motherboard.set_pc(0x4000);
    motherboard.set_sp(0x4000);
    motherboard.write_buffer(0x4000, prog);

    // wait the detection of vsync by the z80 code
    // even if z80 timings are wrong the test shall pass as we are sticked with the ticks
    loop {
        if motherboard.cpu().is_activated() && motherboard.is_aligned_with_new_instruction() {
            println!();
            Monitor::monitor(&motherboard);

            if motherboard.pc() == 0x4000 + (2 + 2 + 1 + 2) {
                break;
            }
        }

        nb_ticks += 1;
        motherboard.tick();

        // safe guard to not indefinitively loop
        assert!(nb_ticks < (19968 + 64 * 2) * 16);
    }

    assert_eq!(motherboard.pc(), 0x4000 + (2 + 2 + 1 + 2));

    // one frame has elapsed, as the Z80 sees it: its loop (IN, RRA, JR NC: 8 us) reads the
    // VSYNC up to a pass late, then ends its RRA and JR
    assert!(
        (19968 * 16..=(19968 + 8 + 4) * 16).contains(&nb_ticks),
        "{nb_ticks} ticks, a frame is {}",
        19968 * 16
    );
}
