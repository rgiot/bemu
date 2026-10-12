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

/// A code breakpoint stored in a snapshot chunk by an assembler (rasm,
/// basm...) or an emulator, for the debugger to stop at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotBreakpoint {
    pub address: u16,
    /// The chunk it comes from: "REMU", "BRKC" (ACE) or "WABP" (WinAPE).
    pub chunk: &'static str
}

/// The code breakpoints of the snapshot at `path`, and a description of
/// what was found but is not supported (memory/IO/conditional breakpoints,
/// unknown entries). The emulator's debugger only knows execution
/// breakpoints on an address, whatever the memory configuration: banks and
/// ROM numbers are ignored.
pub fn load_breakpoints(path: &str) -> Result<(Vec<SnapshotBreakpoint>, Vec<String>), String> {
    let snapshot = Snapshot::load(path).map_err(|e| format!("{path}: {e}"))?;
    let mut breakpoints = Vec::new();
    let mut ignored = Vec::new();
    for chunk in snapshot.chunks() {
        breakpoints_of_chunk(chunk.code().as_str(), chunk.data(), &mut breakpoints, &mut ignored);
    }
    Ok((breakpoints, ignored))
}

fn breakpoints_of_chunk(code: &str, data: &[u8], breakpoints: &mut Vec<SnapshotBreakpoint>, ignored: &mut Vec<String>) {
    match code {
        "REMU" => remu_breakpoints(data, breakpoints, ignored),
        "BRKC" => ace_breakpoints(data, breakpoints, ignored),
        "WABP" => winape_breakpoints(data, breakpoints, ignored),
        _ => {}
    }
}

/// Parse a number written by an assembler: decimal, or 0x/#/& hexadecimal.
fn parse_number(text: &str) -> Option<u16> {
    let text = text.trim();
    let hex = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .or_else(|| text.strip_prefix('#'))
        .or_else(|| text.strip_prefix('&'));
    match hex {
        Some(digits) => u16::from_str_radix(digits, 16).ok(),
        None => text.parse().ok()
    }
}

/// REMU: text entries separated by ';' - "brk <address> <bank>",
/// "rombrk <address> <rom>", "acebreak ...", "label ...", "alias ...".
fn remu_breakpoints(data: &[u8], breakpoints: &mut Vec<SnapshotBreakpoint>, ignored: &mut Vec<String>) {
    for entry in String::from_utf8_lossy(data).split(';') {
        let mut words = entry.split_whitespace();
        match (words.next(), words.next().and_then(parse_number)) {
            (Some("brk" | "rombrk"), Some(address)) => {
                breakpoints.push(SnapshotBreakpoint { address, chunk: "REMU" })
            },
            (Some("label" | "alias") | None, _) => {},
            _ => ignored.push(format!("REMU: {}", entry.trim()))
        }
    }
}

/// ACE (BRKC): 216-byte records, byte 0 the type (0 = execution), bytes
/// 4-5 the address, high byte first (cpcwiki "Snapshot#BRKC").
fn ace_breakpoints(data: &[u8], breakpoints: &mut Vec<SnapshotBreakpoint>, ignored: &mut Vec<String>) {
    for record in data.chunks_exact(216) {
        if record[0] == 0 {
            let address = u16::from_be_bytes([record[4], record[5]]);
            breakpoints.push(SnapshotBreakpoint { address, chunk: "BRKC" });
        }
        else {
            ignored.push(format!("BRKC: breakpoint of type {} (memory/port)", record[0]));
        }
    }
}

/// WinAPE (WABP): a little-endian u32 count of code breakpoints, each an
/// origin byte and a u32 whose low 16 bits are the address, bits 31/30
/// announcing pass counts/a condition that follow (variable size: parsing
/// stops there). Memory and IO breakpoints come after and are not read.
fn winape_breakpoints(data: &[u8], breakpoints: &mut Vec<SnapshotBreakpoint>, ignored: &mut Vec<String>) {
    let u32_at = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let Some(count) = u32_at(0)
    else {
        return;
    };
    for index in 0..count as usize {
        let Some(word) = u32_at(4 + index * 5 + 1)
        else {
            break;
        };
        if word & (3 << 30) != 0 {
            ignored.push(format!(
                "WABP: breakpoint {}/{count} and the following ones (pass count/condition)",
                index + 1
            ));
            return;
        }
        breakpoints.push(SnapshotBreakpoint { address: (word & 0xffff) as u16, chunk: "WABP" });
    }
}

#[cfg(test)]
mod breakpoint_test {
    use super::*;
    use cpclib::sna::*;

    fn parse(code: &str, data: &[u8]) -> (Vec<u16>, usize) {
        let (mut breakpoints, mut ignored) = (Vec::new(), Vec::new());
        breakpoints_of_chunk(code, data, &mut breakpoints, &mut ignored);
        (breakpoints.iter().map(|b| b.address).collect(), ignored.len())
    }

    #[test]
    fn remu_entries_written_by_cpclib() {
        let mut chunk = RemuChunk::empty();
        chunk.add_entry(&RemuEntry::new_breakpoint(0x4000, 0));
        chunk.add_entry(&RemuEntry::new_label("start".to_owned(), 0x4000, 0));
        chunk.add_entry(&RemuEntry::new_rom_breakpoint(0xc006, 7));
        assert_eq!(parse("REMU", chunk.data()), (vec![0x4000, 0xc006], 0));
        assert_eq!(parse("REMU", b"brk #8000 0;brk &10 0;watch 1;"), (vec![0x8000, 0x10], 1));
    }

    #[test]
    fn ace_execution_breakpoints_written_by_cpclib() {
        let mut chunk = AceBreakPointChunk::empty();
        chunk.add_breakpoint(AceBreakPoint::new_execution(0x1234, AceBrkRuntimeMode::Break, AceMemMapType::Undefined));
        chunk.add_breakpoint(AceBreakPoint::new_execution(0xbeef, AceBrkRuntimeMode::Break, AceMemMapType::Undefined));
        assert_eq!(parse("BRKC", chunk.data()), (vec![0x1234, 0xbeef], 0));
    }

    #[test]
    fn winape_code_breakpoints_written_by_cpclib() {
        // The payload as cpclib encodes it (`WabpChunk::add_breakpoint`
        // itself re-reads that payload as a whole RIFF chunk and panics).
        let mut wabp = Wabp::new();
        wabp.add_breakpoint(WabpAnyBreakpoint::new(0x9000));
        wabp.add_breakpoint(WabpAnyBreakpoint::new(0x0038));
        assert_eq!(parse("WABP", &wabp.to_bytes()), (vec![0x9000, 0x0038], 0));
    }
}
