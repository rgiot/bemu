#![cfg(feature = "dandanator")]
//! The Dandanator on the expansion port, driven by the Z80 as a romset drives it: its slot 0
//! boots in place of the firmware, switches zone 0 to slot 3 with Dandare's trigger
//! (`FD FD` + `LD (IY+d),B`, from RAM), slot 3 then asks for zone 0's flash off at the next RET.

use amsrust::dandanator::{Dandanator, SIZE};
use amsrust::motherboard::{Motherboard, MotherboardBuilder};

/// Slot 0: marks &8000, copies its stub to &4000 and runs it; the stub switches zone 0 to slot 3
/// and jumps to &0000.
const SLOT0: [u8; 23] = [
    0xf3, // DI
    0x31, 0x00, 0xc0, // LD SP,&C000
    0x3e, 0xa5, // LD A,&A5
    0x32, 0x00, 0x80, // LD (&8000),A
    0x21, 0x20, 0x00, // LD HL,&0020 (the stub)
    0x11, 0x00, 0x40, // LD DE,&4000
    0x01, 0x0e, 0x00, // LD BC,14
    0xed, 0xb0, // LDIR
    0xc3, 0x00, 0x40 // JP &4000
];
const STUB0: [u8; 14] = [
    0xfd, 0x21, 0x00, 0x81, // LD IY,&8100
    0x06, 0x03, // LD B,3
    0xfd, 0xfd, 0xfd, 0x70, 0x00, // the trigger, LD (IY+0),B: zone 0 = slot 3
    0xc3, 0x00, 0x00 // JP &0000
];

/// Slot 3: marks &8001, then from RAM (&4100): the configuration "zone 0's flash off, at the next
/// RET"; &0000 read before the RET (still slot 3) and after it (the firmware's ROM).
const SLOT3: [u8; 19] = [
    0x3e, 0x5a, // LD A,&5A
    0x32, 0x01, 0x80, // LD (&8001),A
    0x21, 0x20, 0x00, // LD HL,&0020
    0x11, 0x00, 0x41, // LD DE,&4100
    0x01, 0x1e, 0x00, // LD BC,30
    0xed, 0xb0, // LDIR
    0xc3, 0x00, 0x41 // JP &4100
];
const STUB3: [u8; 30] = [
    0xfd, 0x21, 0x00, 0x81, // LD IY,&8100
    0x3e, 0xc1, // LD A,&C1: function 1, wait for RET, zone 0's flash off
    0xfd, 0xfd, 0xfd, 0x77, 0x00, // the trigger, LD (IY+0),A
    0x3a, 0x00, 0x00, // LD A,(&0000)
    0x32, 0x02, 0x80, // LD (&8002),A
    0xcd, 0x1d, 0x41, // CALL &411D
    0x3a, 0x00, 0x00, // LD A,(&0000)
    0x32, 0x03, 0x80, // LD (&8003),A
    0x18, 0xfe, // JR $
    0x00, // NOP
    0xc9 // RET
];

fn romset() -> Vec<u8> {
    let mut romset = vec![0xff; SIZE];
    romset[..SLOT0.len()].copy_from_slice(&SLOT0);
    romset[0x20..0x20 + STUB0.len()].copy_from_slice(&STUB0);
    let slot3 = 3 * 0x4000;
    romset[slot3..slot3 + SLOT3.len()].copy_from_slice(&SLOT3);
    romset[slot3 + 0x20..slot3 + 0x20 + STUB3.len()].copy_from_slice(&STUB3);
    romset
}

fn run(motherboard: &mut Motherboard, microseconds: usize) {
    let end = motherboard.t_state_counter().value() + microseconds * 4;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_no_video();
    }
}

#[test]
fn a_romset_boots_switches_slots_and_waits_for_a_ret() {
    let mut motherboard = MotherboardBuilder::default().build();
    motherboard.plug_expansion(Box::new(Dandanator::new(&romset())));
    run(&mut motherboard, 2000);
    assert_eq!(motherboard.read_byte(0x8000), 0xa5, "slot 0 booted");
    assert_eq!(motherboard.read_byte(0x8001), 0x5a, "slot 3 runs after the trigger");
    assert_eq!(motherboard.read_byte(0x8002), 0x3e, "before the RET: slot 3's first byte");
    assert_eq!(motherboard.read_byte(0x8003), 0x01, "after it: the firmware's (LD BC,&7F89)");
    let card = motherboard.expansion_port().unwrap().device::<Dandanator>().unwrap();
    assert_eq!(card.configuration()[1], 0x81, "function 1, the wait over");
}

#[test]
fn what_is_programmed_goes_back_to_a_writable_file_only() {
    let path = std::env::temp_dir().join(format!("bemu-dandanator-{}.rom", std::process::id()));
    for writable in [false, true] {
        std::fs::write(&path, romset()).unwrap();
        let mut card = Dandanator::open(&path, writable).unwrap();
        // writes allowed, then a byte programmed in slot 31 (5555 in slot 1, 2AAA in slot 0)
        card.command(0, 0x02);
        for (slot, address, value) in [(1, 0x1555, 0xaa), (0, 0x2aaa, 0x55), (1, 0x1555, 0xa0), (31, 0x0010, 0x12)] {
            card.command(2, slot);
            card.write(address, value);
        }
        assert_eq!(card.flash()[31 * 0x4000 + 0x10], 0x12);
        drop(card);
        let saved = std::fs::read(&path).unwrap();
        assert_eq!(saved[31 * 0x4000 + 0x10], if writable { 0x12 } else { 0xff }, "writable: {writable}");
    }
    let _ = std::fs::remove_file(&path);
}
