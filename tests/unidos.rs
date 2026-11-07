#![cfg(all(feature = "unidos", feature = "nova", feature = "albireo"))]
//! UniDOS and its nodes, from the ROMs embedded in the executable, on a Nova and an Albireo
//! (its key a folder): booted, UniDOS keeps its context in the Nova's NVRAM.

use amsrust::ch376::Albireo;
use amsrust::motherboard::MotherboardBuilder;
use amsrust::nova::Nova;
use amsrust::roms::load_rom;

#[test]
fn unidos_boots_on_its_nodes() {
    let usb = std::env::temp_dir().join(format!("bemu-unidos-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&usb);
    std::fs::create_dir_all(&usb).unwrap();
    let mut motherboard = MotherboardBuilder::default().build();
    motherboard.plug_expansion(Box::new(Nova::new(None).unwrap()));
    motherboard.plug_expansion(Box::new(Albireo::open(&usb).unwrap()));
    // the manual's layout
    for (slot, rom) in [
        (1, "inner://unidos-nova"),
        (7, "inner://unidos"),
        (14, "inner://unitools"),
        (15, "inner://amsdos"),
        (27, "inner://unidos-albireo"),
    ] {
        motherboard
            .set_upper_rom(slot, Some(&load_rom(rom).unwrap()))
            .unwrap();
    }
    // two seconds: the firmware, UniDOS and its nodes initialised
    while motherboard.t_state_counter().value() < 8_000_000 {
        motherboard.execute_next_opcode_no_video();
    }
    let nova = motherboard
        .expansion_port()
        .unwrap()
        .device::<Nova>()
        .unwrap();
    let written = nova.ram().iter().filter(|&&b| b != 0).count();
    assert!(written > 0, "UniDOS found its NVRAM and wrote its context");
    let _ = std::fs::remove_dir_all(&usb);
}
