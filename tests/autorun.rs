//! Autorun (`amsrust::autorun`): what is typed to start a disc, picked from its catalogue, and a
//! disc that starts by itself on the machine of the Android frontend (UniDOS, the M4 and
//! RulezCharge plugged).

use amsrust::autorun::{
    boot_seconds, disc_command, media_kind, MediaKind, AUTORUN_AT_SECONDS, AUTORUN_EXPANDED_AT_SECONDS,
};
use amsrust::disc_image::load_dsk;
use amsrust::fdc::FdcDrive;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::typist::{Typist, T_STATES_PER_SECOND};


/// A data disc holding `files` (name, load and run address, bytes) as AMSDOS binaries.
fn disc_with(files: &[(&str, u16, &[u8])]) -> Vec<u8> {
    use cpclib_disc::amsdos::{AmsdosAddBehavior, AmsdosFile, AmsdosFileName, AmsdosManagerMut};
    let mut disc = cpclib_disc::builder::single_head_data_dsk();
    for (name, address, code) in files {
        let filename = AmsdosFileName::try_from(*name).unwrap();
        let file = AmsdosFile::binary_file_from_buffer(&filename, *address, *address, code).unwrap();
        AmsdosManagerMut::new_from_disc(&mut disc, cpclib_disc::edsk::Head::A)
            .add_file(&file, None, false, false, AmsdosAddBehavior::FailIfPresent)
            .unwrap();
    }
    let mut bytes = Vec::new();
    disc.to_buffer(&mut bytes);
    bytes
}

/// LD A,&42 : LD (&9000),A : DI : JR $ — a mark in RAM, the CPU then held.
const MARKER: [u8; 8] = [0x3e, 0x42, 0x32, 0x00, 0x90, 0xf3, 0x18, 0xfe];

#[test]
fn the_program_of_a_disc_is_found() {
    let bytes = disc_with(&[
        ("DATA.DAT", 0x4000, &[0u8; 3000]),
        ("GAME.BIN", 0x4000, &MARKER),
        ("LOADER.BAS", 0x0170, &[0u8; 100]),
    ]);
    let disc = load_dsk(&bytes).unwrap();
    assert_eq!(
        disc_command(&disc).as_deref(),
        Some("RUN\"LOADER.BAS\\(RET)")
    );
    assert_eq!(media_kind("whatever", &bytes), Some(MediaKind::Disc));
}

#[test]
fn the_benediction_discs_run_their_menus() {
    for (path, expected) in [
        ("validation/Benediction/bnd4_package.dsk", "RUN\"#MENU\\(RET)"),
        ("validation/Benediction/HBL.DSK", "RUN\"HBL.BAS\\(RET)"),
    ] {
        let disc = load_dsk(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(disc_command(&disc).as_deref(), Some(expected), "{path}");
    }
}

/// The machine bemu-droid builds: a 6128, the M4 (SD card in memory), UniDOS and RulezCharge.
#[cfg(all(feature = "m4", feature = "unidos"))]
fn droid_machine() -> Motherboard {
    use amsrust::roms::load_rom;
    let mut motherboard = MotherboardBuilder::default().build();
    let m4 = amsrust::m4::M4Board::with_bundled_rom_in_memory(amsrust::m4::MemoryCard::new())
        .unwrap()
        .in_slot(amsrust::m4::M4_ROM_SLOT_WITH_UNIDOS);
    motherboard.plug_expansion(Box::new(m4));
    let mut slots = vec![
        (1, "inner://unidos-m4"),
        (7, "inner://unidos"),
        (14, "inner://unitools"),
        (15, "inner://amsdos"),
    ];
    if cfg!(feature = "m4-wifi") {
        slots.push((5, "inner://rulezcharge"));
    }
    for (slot, rom) in slots {
        motherboard
            .set_upper_rom(slot, Some(&load_rom(rom).unwrap()))
            .unwrap();
    }
    motherboard.restart();
    motherboard
}

/// Runs the machine and its typist until the typing begins: when, in seconds.
fn run_until_started(motherboard: &mut Motherboard, typist: &mut Typist) -> f64 {
    while !typist.started() {
        assert!(motherboard.t_state_counter().value() < 10 * T_STATES_PER_SECOND, "never started");
        run(motherboard, typist, 0.001);
    }
    motherboard.t_state_counter().value() as f64 / T_STATES_PER_SECOND as f64
}

/// Runs the machine and its typist for `seconds` of emulated time.
fn run(motherboard: &mut Motherboard, typist: &mut Typist, seconds: f64) {
    let end = motherboard.t_state_counter().value() + (seconds * T_STATES_PER_SECOND as f64) as usize;
    while motherboard.t_state_counter().value() < end {
        typist.update(motherboard);
        for _ in 0..1000 {
            motherboard.execute_next_opcode_no_video();
        }
    }
}

#[test]
#[cfg(all(feature = "m4", feature = "unidos"))]
fn a_disc_starts_by_itself_on_unidos_and_the_m4() {
    let bytes = disc_with(&[("DATA.DAT", 0x4000, &[0u8; 3000]), ("GAME.BIN", 0x4000, &MARKER)]);
    let disc = load_dsk(&bytes).unwrap();
    let text = disc_command(&disc).unwrap();
    assert_eq!(text, "RUN\"GAME.BIN\\(RET)");

    let mut motherboard = droid_machine();
    motherboard.fdc_mut().insert_image(FdcDrive::A, Some(disc));
    assert_eq!(boot_seconds(&motherboard), AUTORUN_EXPANDED_AT_SECONDS);
    let mut typist = amsrust::autorun::typist(&text, &motherboard);
    let started = run_until_started(&mut motherboard, &mut typist);
    // UniDOS and the M4 boot until 3.7 s: keys typed before were lost
    assert!((3.0..AUTORUN_EXPANDED_AT_SECONDS).contains(&started), "typing started at {started:.2} s");
    run(&mut motherboard, &mut typist, 4.0);
    assert!(typist.is_done(), "everything was typed");
    assert_eq!(
        motherboard.read_bank_byte(2, 0x1000),
        Some(0x42),
        "GAME.BIN ran (its mark at &9000)"
    );
}


#[test]
fn a_disc_starts_by_itself_on_a_stock_6128() {
    let bytes = disc_with(&[("GAME.BIN", 0x4000, &MARKER)]);
    let disc = load_dsk(&bytes).unwrap();
    let mut motherboard = MotherboardBuilder::default().build();
    motherboard.fdc_mut().insert_image(FdcDrive::A, Some(disc.clone()));
    assert_eq!(boot_seconds(&motherboard), AUTORUN_AT_SECONDS);
    let mut typist = amsrust::autorun::typist(&disc_command(&disc).unwrap(), &motherboard);
    let started = run_until_started(&mut motherboard, &mut typist);
    // BASIC's prompt waits for a key at 0.8 s: not the 3 s a fixed delay takes
    assert!((0.5..1.5).contains(&started), "typing started at {started:.2} s");
    run(&mut motherboard, &mut typist, 5.0);
    assert_eq!(motherboard.read_bank_byte(2, 0x1000), Some(0x42), "GAME.BIN ran");
}

#[test]
fn the_typing_waits_for_basic_on_a_464_too() {
    let mut builder = MotherboardBuilder::default();
    builder.set_model("464".parse().unwrap());
    let mut motherboard = builder.build();
    let mut typist = amsrust::autorun::typist("PRINT 1\\(RET)", &motherboard);
    let started = run_until_started(&mut motherboard, &mut typist);
    assert!((0.5..1.5).contains(&started), "typing started at {started:.2} s");
    run(&mut motherboard, &mut typist, 2.0);
    assert!(typist.is_done());
}
