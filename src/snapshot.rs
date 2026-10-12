//! Loads a `.SNA` snapshot (the CPCEMU/WinAPE format `cpclib-sna` reads)
//! into a running [`Motherboard`]: CPU registers, the base 64K of RAM, the
//! CRTC's registers, the gate array's ink/mode/ROM-paging state and the
//! PSG's registers - replayed through each chip's own real register-write
//! path (see [`crate::ga::GateArray::write_register`]) rather than by
//! reconstructing the resulting state by hand, wherever that path exists.
//!
//! Not restored: PPI port latches (no public way to force one without
//! reconstructing its mode first) and the FDD motor/track flags (a disc
//! given on its own command line is inserted stopped at track 0 already).
//! The extra 64K pages of a 128K+ snapshot go to the extra RAM.

use cpclib::sna::{Snapshot, SnapshotFlag};

use crate::motherboard::Motherboard;

/// Load `path` into `motherboard`. Registers, RAM and the CRTC/GA/PSG are
/// all overwritten; anything not covered by the snapshot format is left as
/// the caller set it up (so loading a snapshot after choosing `--crtc` and
/// inserting a disc still leaves those in place).
pub fn load_sna(motherboard: &mut Motherboard, path: &str) -> Result<(), String> {
    let snapshot = Snapshot::load(path).map_err(|e| format!("{path}: {e}"))?;
    let word = |flag: SnapshotFlag| -> u16 {
        snapshot.get_value(&flag).as_u16().unwrap_or_else(|| panic!("{flag:?} is not a plain value"))
    };
    let byte = |flag: SnapshotFlag| -> u8 { word(flag) as u8 };

    load_registers(motherboard, &word, &byte);
    load_memory(motherboard, &snapshot)?;
    load_crtc(motherboard, &byte);
    load_gate_array(motherboard, &byte);
    load_psg(motherboard, &byte);

    Ok(())
}

fn load_registers(motherboard: &mut Motherboard, word: &impl Fn(SnapshotFlag) -> u16, byte: &impl Fn(SnapshotFlag) -> u8) {
    use crate::z80::{HasValue, InterruptMode, Reg16};
    use SnapshotFlag::*;

    let cpu = motherboard.cpu_mut();
    cpu.reg16_mut(Reg16::Af).set(word(Z80_AF));
    cpu.reg16_mut(Reg16::Bc).set(word(Z80_BC));
    cpu.reg16_mut(Reg16::De).set(word(Z80_DE));
    cpu.reg16_mut(Reg16::Hl).set(word(Z80_HL));
    cpu.reg16_mut(Reg16::Af2).set(word(Z80_AFX));
    cpu.reg16_mut(Reg16::Bc2).set(word(Z80_BCX));
    cpu.reg16_mut(Reg16::De2).set(word(Z80_DEX));
    cpu.reg16_mut(Reg16::Hl2).set(word(Z80_HLX));
    cpu.reg16_mut(Reg16::Ix).set(word(Z80_IX));
    cpu.reg16_mut(Reg16::Iy).set(word(Z80_IY));
    cpu.reg16_mut(Reg16::Sp).set(word(Z80_SP));
    cpu.reg16_mut(Reg16::Pc).set(word(Z80_PC));
    cpu.set_i_and_r(byte(Z80_I), byte(Z80_R));
    cpu.set_iffs(byte(Z80_IFF0) != 0, byte(Z80_IFF1) != 0);
    cpu.set_im(match byte(Z80_IM) {
        0 => InterruptMode::Mode0,
        2 => InterruptMode::Mode2,
        _ => InterruptMode::Mode1
    });
}

fn load_memory(motherboard: &mut Motherboard, snapshot: &Snapshot) -> Result<(), String> {
    let memory = snapshot.memory_dump()?;
    for (address, &value) in memory.iter().take(0x10000).enumerate() {
        motherboard.write_byte(address as u16, value);
    }
    // the 128K (and more) snapshots: the following 64K pages go to the extra RAM
    for (page, data) in memory.get(0x10000..).unwrap_or(&[]).chunks(0x10000).enumerate() {
        motherboard.load_extra_ram_page(page, data);
    }
    Ok(())
}

fn load_crtc(motherboard: &mut Motherboard, byte: &impl Fn(SnapshotFlag) -> u8) {
    let crtc = motherboard.crtc_mut();
    for register in 0..=17u8 {
        crtc.do_write_register(register, byte(SnapshotFlag::CRTC_REG(Some(register as usize))));
    }
}

fn load_gate_array(motherboard: &mut Motherboard, byte: &impl Fn(SnapshotFlag) -> u8) {
    use crate::memory::RomConfig;
    use SnapshotFlag::*;

    let ga = motherboard.ga_mut();
    // Replay the exact bytes through the gate array's own PENR/INKR
    // dispatch (see write_register's doc), rather than computing the
    // resulting Ink/VideoMode by hand: select pen i (0x00-0x0F) or the
    // border (0x10), then its ink (INKR: 0x40 | hardware colour 0-31).
    for pen in 0..=16u8 {
        let select = if pen == 16 { 0x10 } else { pen };
        let hardware_ink = byte(GA_PAL(Some(pen as usize))) & 0x1f;
        ga.write_register(select);
        ga.write_register(0x40 | hardware_ink);
    }
    // RMR: mode and ROM paging bits, forced into its address range (bit7
    // set, bit6 clear) the way real firmware always leaves it, per the
    // format's own documented convention.
    ga.write_register(0x80 | (byte(GA_ROMCFG) & 0x1f));

    // The upper ROM page number is a separate latch (the ROM select port),
    // not one of the GA's own registers - keep whatever enable/disable the
    // RMR replay above just set, only change which ROM image is selected.
    let mut config = motherboard.memory_config().0;
    config.rom_number = byte(ROM_UP);
    motherboard.force_rom_config(RomConfig { ..config });
}

fn load_psg(motherboard: &mut Motherboard, byte: &impl Fn(SnapshotFlag) -> u8) {
    let psg = motherboard.psg_mut();
    for register in 0..=15u8 {
        psg.set_register(register, byte(SnapshotFlag::PSG_REG(Some(register as usize))));
    }
}
