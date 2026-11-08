//! Loads and saves `.SNA` snapshots (the CPCEMU/WinAPE format `cpclib-sna` reads)
//! into a running [`Motherboard`]: CPU registers, the base 64K of RAM, the
//! CRTC's registers, the gate array's ink/mode/ROM-paging state and the
//! PSG's registers - replayed through each chip's own real register-write
//! path (see [`crate::ga::GateArray::write_register`]) rather than by
//! reconstructing the resulting state by hand, wherever that path exists.
//!
//! The CRTC takes its registers as stored (not written: a write has side
//! effects in the middle of a frame), its selected register and, from version
//! 3, its counters.
//!
//! The PPI gets its control word, port A's output latch and port C.
//!
//! Not restored: the FDD motor/track flags (a disc given on its own command
//! line is inserted stopped at track 0 already).
//! The extra 64K pages of a 128K+ snapshot go to the extra RAM (a RAM
//! expansion on a 464/664), banked as the MMR says. From version 2, the
//! header's CPC type picks the model (ROMs, RAM banking).

use cpclib::sna::{Snapshot, SnapshotFlag, SnapshotVersion};

use crate::breakpoints::AdvancedBreakpoint;
use crate::crtc::CrtcCounters;
use crate::motherboard::{CpcModel, Motherboard};

/// Load `path` into `motherboard`. Registers, RAM and the CRTC/GA/PSG are
/// all overwritten; anything not covered by the snapshot format is left as
/// the caller set it up (so loading a snapshot after choosing `--crtc` and
/// inserting a disc still leaves those in place).
pub fn load_sna(motherboard: &mut Motherboard, path: &str) -> Result<(), String> {
    let snapshot = Snapshot::load(path).map_err(|e| format!("{path}: {e}"))?;
    load_snapshot(motherboard, &snapshot)
}

/// [`load_sna`] from the bytes of a `.SNA` file; also returns the
/// breakpoints it carries (see [`load_breakpoints`]).
pub fn load_sna_bytes(
    motherboard: &mut Motherboard,
    bytes: &[u8],
) -> Result<SnapshotBreakpoints, String> {
    let snapshot = Snapshot::from_buffer(bytes.to_vec())?;
    load_snapshot(motherboard, &snapshot)?;
    Ok(breakpoints_of_snapshot(&snapshot))
}

/// The state of `motherboard` as a version 3 `.SNA` file (memory in compressed MEM chunks), the
/// inverse of [`load_sna_bytes`]: CPU, RAM (the main 64K and the machine's extra pages, not a
/// large expansion card's), CRTC registers and counters, Gate Array, PSG, PPI, upper ROM, MMR,
/// the model and the CRTC type, the disc motor and drive A's track.
pub fn save_sna_bytes(motherboard: &Motherboard) -> Result<Vec<u8>, String> {
    save_sna_bytes_version(motherboard, 3)
}

/// [`save_sna_bytes`] as a snapshot of `version` 1, 2 or 3 (the older ones keep less: no CRTC
/// counters, no CPC+ chunk...).
pub fn save_sna_bytes_version(motherboard: &Motherboard, version: u8) -> Result<Vec<u8>, String> {
    let version = match version {
        1 => SnapshotVersion::V1,
        2 => SnapshotVersion::V2,
        3 => SnapshotVersion::V3,
        other => return Err(format!("snapshot version {other}: 1, 2 or 3")),
    };
    let mut bytes = Vec::new();
    snapshot_of(motherboard)?
        .write_all(&mut bytes, version)
        .map_err(|e| format!("cannot write the snapshot: {e}"))?;
    Ok(bytes)
}

/// [`save_sna_bytes`] into the file `path`.
pub fn save_sna(motherboard: &Motherboard, path: &str) -> Result<(), String> {
    std::fs::write(path, save_sna_bytes(motherboard)?).map_err(|e| format!("{path}: {e}"))
}

fn snapshot_of(motherboard: &Motherboard) -> Result<Snapshot, String> {
    use crate::z80::{HasValue, InterruptMode, Reg16, Reg8};
    use SnapshotFlag::*;

    let mut snapshot = Snapshot::default();
    let mut values: Vec<(SnapshotFlag, u16)> = Vec::new();
    let mut set = |flag: SnapshotFlag, value: u16| values.push((flag, value));

    let cpu = motherboard.cpu();
    for (flag, register) in [
        (Z80_AF, Reg16::Af),
        (Z80_BC, Reg16::Bc),
        (Z80_DE, Reg16::De),
        (Z80_HL, Reg16::Hl),
        (Z80_AFX, Reg16::Af2),
        (Z80_BCX, Reg16::Bc2),
        (Z80_DEX, Reg16::De2),
        (Z80_HLX, Reg16::Hl2),
        (Z80_IX, Reg16::Ix),
        (Z80_IY, Reg16::Iy),
        (Z80_SP, Reg16::Sp),
        (Z80_PC, Reg16::Pc),
    ] {
        set(flag, cpu.reg16(register).value());
    }
    set(Z80_I, cpu.reg8(Reg8::I).value().into());
    set(Z80_R, cpu.reg8(Reg8::R).value().into());
    set(Z80_IFF0, cpu.iff1().into());
    set(Z80_IFF1, cpu.iff2().into());
    set(
        Z80_IM,
        match cpu.im() {
            InterruptMode::Mode0 => 0,
            InterruptMode::Mode1 => 1,
            InterruptMode::Mode2 => 2,
        },
    );

    let ga = motherboard.ga();
    for (pen, ink) in ga.palette().iter().enumerate() {
        set(GA_PAL(Some(pen)), ink.hardware_number().into());
    }
    set(GA_PEN, ga.selected_pen().into());
    set(GA_ROMCFG, ga.rmr().into());
    set(GA_RAMCFG, motherboard.mmr().into());
    set(ROM_UP, motherboard.upper_rom_number().into());
    set(GA_ISC, ga.r52().into());

    let ppi = motherboard.ppi();
    set(PPI_A, ppi.port_a_latch().into());
    set(PPI_B, ppi.get_data_b_from_pins().into());
    set(PPI_C, ppi.get_data_c_from_pins().into());
    set(PPI_CTL, ppi.control_word().into());

    let psg = motherboard.psg();
    set(PSG_SEL, psg.selected_register().unwrap_or(0).into());
    for register in 0..=15u8 {
        set(
            PSG_REG(Some(register as usize)),
            psg.register(register).into(),
        );
    }

    set(
        CPC_TYPE,
        match motherboard.model() {
            CpcModel::Cpc464 => 0,
            CpcModel::Cpc664 => 1,
            CpcModel::Cpc6128 => 2,
            CpcModel::Cpc6128Plus => 4,
            CpcModel::Cpc464Plus => 5,
            CpcModel::Gx4000 => 6,
        },
    );

    // the CRTC as `load_crtc` reads it back
    let crtc = motherboard.crtc();
    for (register, value) in crtc.registers().iter().enumerate() {
        set(CRTC_REG(Some(register)), (*value).into());
    }
    set(CRTC_SEL, crtc.selected_register().into());
    set(CRTC_TYPE, crtc.crtc_type() as u16);
    let c = crtc.counters();
    set(CRTC_HCC, c.hcc.into());
    set(CRTC_CLC, c.vcc.into());
    set(CRTC_RLC, c.vlc.into());
    set(CRTC_VAC, c.vtac.into());
    set(CRTC_VSWC, c.hsc.into());
    set(CRTC_HSWC, c.vsc.into());
    set(
        CRTC_STATE,
        u16::from(c.vsync) | u16::from(c.hsync) << 1 | u16::from(c.in_vertical_adjust) << 7,
    );

    set(FDD_MOTOR, motherboard.fdc().motor().into());
    set(FDD_TRACK, motherboard.fdc().track(0).into());

    for (flag, value) in values {
        snapshot
            .set_value(flag, value)
            .map_err(|e| format!("{flag:?}: {e:?}"))?;
    }

    if let Some(chunk) = motherboard.asic_snapshot_chunk() {
        snapshot.add_chunk(cpclib::sna::UnknownChunk::new("CPC+", chunk));
    }

    let ram = motherboard.ram_dump();
    snapshot.resize(ram.len() / 0x10000);
    for (address, &value) in ram.iter().enumerate() {
        snapshot.set_byte(address as u32, value);
    }
    Ok(snapshot)
}

fn load_snapshot(motherboard: &mut Motherboard, snapshot: &Snapshot) -> Result<(), String> {
    let word = |flag: SnapshotFlag| -> u16 {
        snapshot
            .get_value(&flag)
            .as_u16()
            .unwrap_or_else(|| panic!("{flag:?} is not a plain value"))
    };
    let byte = |flag: SnapshotFlag| -> u8 { word(flag) as u8 };

    load_model(motherboard, snapshot);
    load_registers(motherboard, &word, &byte);
    load_memory(motherboard, snapshot)?;
    load_crtc(motherboard, snapshot, &byte);
    load_gate_array(motherboard, &byte);
    load_psg(motherboard, &byte);
    motherboard.ppi_mut().restore(
        byte(SnapshotFlag::PPI_CTL),
        byte(SnapshotFlag::PPI_A),
        byte(SnapshotFlag::PPI_C),
    );
    // the CPC+'s ASIC, after the Gate Array whose inks also reach its palette
    if let Some(chunk) = snapshot
        .chunks()
        .iter()
        .find(|chunk| chunk.code().as_str() == "CPC+")
    {
        motherboard.restore_asic_snapshot_chunk(chunk.data())?;
    }

    Ok(())
}

fn load_registers(
    motherboard: &mut Motherboard,
    word: &impl Fn(SnapshotFlag) -> u16,
    byte: &impl Fn(SnapshotFlag) -> u8,
) {
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
        _ => InterruptMode::Mode1,
    });
}

/// The machine the snapshot was taken on (version 2 and later): a 464,
/// 664 or 6128 gets its ROMs and RAM banking (see
/// [`Motherboard::set_model`]); the disc interface, an add-on on a 464, is
/// kept as it was; a CPC+ type gives that CPC+ (its ASIC's state is the snapshot's "CPC+" chunk).
/// The unknown type leaves the machine as it is.
pub fn model_of_snapshot(snapshot: &Snapshot) -> Option<CpcModel> {
    if snapshot.version_header() < 2 {
        return None;
    }
    match snapshot.get_value(&SnapshotFlag::CPC_TYPE).as_u16()? {
        0 => Some(CpcModel::Cpc464),
        1 => Some(CpcModel::Cpc664),
        2 => Some(CpcModel::Cpc6128),
        4 => Some(CpcModel::Cpc6128Plus),
        5 => Some(CpcModel::Cpc464Plus),
        6 => Some(CpcModel::Gx4000),
        _ => None,
    }
}

fn load_model(motherboard: &mut Motherboard, snapshot: &Snapshot) {
    if let Some(model) = model_of_snapshot(snapshot) {
        if model != motherboard.model() {
            let disc_interface = motherboard.has_disc_interface();
            motherboard.set_model(model);
            motherboard.set_disc_interface(disc_interface);
        }
    }
}

fn load_memory(motherboard: &mut Motherboard, snapshot: &Snapshot) -> Result<(), String> {
    let memory = snapshot.memory_dump()?;
    for (address, &value) in memory.iter().take(0x10000).enumerate() {
        motherboard.write_byte(address as u16, value);
    }
    // the 128K (and more) snapshots: the following 64K pages go to the
    // extra RAM - an expansion on a 464/664, banked through the MMR.
    let extra_pages = memory.len().saturating_sub(0x10000).div_ceil(0x10000);
    if extra_pages > 0
        && (!motherboard.has_ram_banking() || motherboard.extra_ram_pages() < extra_pages)
    {
        motherboard.set_extra_ram_pages(extra_pages.max(motherboard.extra_ram_pages()));
    }
    for (page, data) in memory
        .get(0x10000..)
        .unwrap_or(&[])
        .chunks(0x10000)
        .enumerate()
    {
        motherboard.load_extra_ram_page(page, data);
    }
    motherboard.force_mmr(
        snapshot
            .get_value(&SnapshotFlag::GA_RAMCFG)
            .as_u16()
            .unwrap_or(0) as u8,
    );
    Ok(())
}

/// The registers as stored, the selected one, and from version 3 the counters
/// (&A9 C0, &AB C4, &AC C9, &AD adjust, &AE HSYNC width counter, &AF VSYNC width
/// counter, &B0 flags: bit 0 VSYNC, bit 1 HSYNC, bit 7 vertical adjust). cpclib
/// names &AE VSWC and &AF HSWC: the format has them the other way round.
fn load_crtc(
    motherboard: &mut Motherboard,
    snapshot: &Snapshot,
    byte: &impl Fn(SnapshotFlag) -> u8,
) {
    use SnapshotFlag::*;

    let mut regs = [0u8; 18];
    for (register, value) in regs.iter_mut().enumerate() {
        *value = byte(CRTC_REG(Some(register)));
    }
    let counters = (snapshot.version_header() >= 3).then(|| {
        let flags = byte(CRTC_STATE);
        CrtcCounters {
            hcc: byte(CRTC_HCC),
            vcc: byte(CRTC_CLC),
            vlc: byte(CRTC_RLC),
            vtac: byte(CRTC_VAC),
            hsc: byte(CRTC_VSWC),
            vsc: byte(CRTC_HSWC),
            vsync: flags & 1 != 0,
            hsync: flags & 2 != 0,
            in_vertical_adjust: flags & 0x80 != 0,
        }
    });
    motherboard
        .crtc_mut()
        .restore(&regs, byte(CRTC_SEL), counters);
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
    // the pen selected, as it was (the palette's writes above changed it)
    ga.write_register(byte(GA_PEN) & 0x1f);
    ga.set_r52(byte(GA_ISC));

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
        psg.set_register(
            register,
            byte(SnapshotFlag::PSG_REG(Some(register as usize))),
        );
    }
    psg.select_register(byte(SnapshotFlag::PSG_SEL));
}

/// A code breakpoint stored in a snapshot chunk by an assembler (rasm,
/// basm...) or an emulator, for the debugger to stop at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotBreakpoint {
    pub address: u16,
    /// The chunk it comes from: "REMU", "BRKC" (ACE), "BRKS" (rasm/basm for
    /// WinAPE) or "WABP" (WinAPE).
    pub chunk: &'static str,
}

/// The breakpoints a snapshot carries, and its symbols.
#[derive(Debug, Default)]
pub struct SnapshotBreakpoints {
    /// Execution breakpoints on an address (banks and ROM numbers ignored).
    pub exec: Vec<SnapshotBreakpoint>,
    /// REMU `acebreak` entries: memory/IO/execution breakpoints with
    /// masks, sizes, values, stop or watch (see [`crate::breakpoints`]).
    pub advanced: Vec<AdvancedBreakpoint>,
    /// What was found but is not supported, described.
    pub ignored: Vec<String>,
    /// The labels of the `SYMB` chunk and of the REMU chunk's `label`/`alias` entries.
    pub symbols: crate::symbols::SymbolTable,
}

impl SnapshotBreakpoints {
    pub fn is_empty(&self) -> bool {
        self.exec.is_empty() && self.advanced.is_empty()
    }

    pub fn len(&self) -> usize {
        self.exec.len() + self.advanced.len()
    }
}

/// The breakpoints of the snapshot at `path`.
pub fn load_breakpoints(path: &str) -> Result<SnapshotBreakpoints, String> {
    let snapshot = Snapshot::load(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(breakpoints_of_snapshot(&snapshot))
}

fn breakpoints_of_snapshot(snapshot: &Snapshot) -> SnapshotBreakpoints {
    let mut breakpoints = SnapshotBreakpoints::default();
    for chunk in snapshot.chunks() {
        breakpoints_of_chunk(chunk.code().as_str(), chunk.data(), &mut breakpoints);
    }
    // Assemblers write the same breakpoint to several chunks (REMU, BRKS
    // and BRKC for basm and rasm): one each.
    let mut seen = std::collections::HashSet::new();
    breakpoints.exec.retain(|b| seen.insert(b.address));
    breakpoints
}

fn breakpoints_of_chunk(code: &str, data: &[u8], breakpoints: &mut SnapshotBreakpoints) {
    breakpoints.symbols.add_snapshot_chunk(code, data);
    let SnapshotBreakpoints {
        exec,
        advanced,
        ignored,
        ..
    } = breakpoints;
    match code {
        "REMU" => remu_breakpoints(data, exec, advanced, ignored),
        "BRKC" => ace_breakpoints(data, exec, ignored),
        "BRKS" => brks_breakpoints(data, exec),
        "WABP" => winape_breakpoints(data, exec, ignored),
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
        None => text.parse().ok(),
    }
}

/// REMU: text entries separated by ';' - "brk <address> <bank>",
/// "rombrk <address> <rom>", "acebreak <description>", "label ...",
/// "alias ...".
fn remu_breakpoints(
    data: &[u8],
    breakpoints: &mut Vec<SnapshotBreakpoint>,
    advanced: &mut Vec<AdvancedBreakpoint>,
    ignored: &mut Vec<String>,
) {
    for entry in String::from_utf8_lossy(data).split(';') {
        let entry = entry.trim();
        if let Some(description) = entry.strip_prefix("acebreak ") {
            match AdvancedBreakpoint::parse_remu(description) {
                Ok(brk) => advanced.push(brk),
                Err(e) => ignored.push(format!("REMU: {entry}: {e}")),
            }
            continue;
        }
        let mut words = entry.split_whitespace();
        match (words.next(), words.next().and_then(parse_number)) {
            (Some("brk" | "rombrk"), Some(address)) => breakpoints.push(SnapshotBreakpoint {
                address,
                chunk: "REMU",
            }),
            (Some("label" | "alias") | None, _) => {}
            _ => ignored.push(format!("REMU: {entry}")),
        }
    }
}

/// ACE (BRKC): 216-byte records, byte 0 the type (0 = execution), bytes
/// 4-5 the address, high byte first (cpcwiki "Snapshot#BRKC", as cpclib and
/// basm write it), the name at 144. rasm writes the address low byte first
/// and names its records "breakpoint<index>": read that way when so named.
fn ace_breakpoints(
    data: &[u8],
    breakpoints: &mut Vec<SnapshotBreakpoint>,
    ignored: &mut Vec<String>,
) {
    for (index, record) in data.chunks_exact(216).enumerate() {
        if record[0] == 0 {
            let name = &record[144..208];
            let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
            let address = if name == format!("breakpoint{index}").as_bytes() {
                u16::from_le_bytes([record[4], record[5]])
            } else {
                u16::from_be_bytes([record[4], record[5]])
            };
            breakpoints.push(SnapshotBreakpoint {
                address,
                chunk: "BRKC",
            });
        } else {
            ignored.push(format!(
                "BRKC: breakpoint of type {} (memory/port)",
                record[0]
            ));
        }
    }
}

/// BRKS (written by rasm and basm for WinAPE, unofficial): 5-byte records,
/// the address low byte first, 0 for the base 64K or 1 for the extended
/// RAM (ignored), a zeroed condition.
fn brks_breakpoints(data: &[u8], breakpoints: &mut Vec<SnapshotBreakpoint>) {
    for record in data.chunks_exact(5) {
        breakpoints.push(SnapshotBreakpoint {
            address: u16::from_le_bytes([record[0], record[1]]),
            chunk: "BRKS",
        });
    }
}

/// WinAPE (WABP): a little-endian u32 count of code breakpoints, each an
/// origin byte and a u32 whose low 16 bits are the address, bits 31/30
/// announcing pass counts/a condition that follow (variable size: parsing
/// stops there). Memory and IO breakpoints come after and are not read.
fn winape_breakpoints(
    data: &[u8],
    breakpoints: &mut Vec<SnapshotBreakpoint>,
    ignored: &mut Vec<String>,
) {
    let u32_at = |at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let Some(count) = u32_at(0) else {
        return;
    };
    for index in 0..count as usize {
        let Some(word) = u32_at(4 + index * 5 + 1) else {
            break;
        };
        if word & (3 << 30) != 0 {
            ignored.push(format!(
                "WABP: breakpoint {}/{count} and the following ones (pass count/condition)",
                index + 1
            ));
            return;
        }
        breakpoints.push(SnapshotBreakpoint {
            address: (word & 0xffff) as u16,
            chunk: "WABP",
        });
    }
}

#[cfg(test)]
mod breakpoint_test {
    use super::*;
    use cpclib::sna::*;

    fn parse(code: &str, data: &[u8]) -> (Vec<u16>, usize) {
        let mut breakpoints = SnapshotBreakpoints::default();
        breakpoints_of_chunk(code, data, &mut breakpoints);
        (
            breakpoints.exec.iter().map(|b| b.address).collect(),
            breakpoints.ignored.len(),
        )
    }

    #[test]
    fn breakpoints_of_a_saved_v3_snapshot() {
        let mut snapshot = Snapshot::default();
        let mut remu = RemuChunk::empty();
        remu.add_entry(&RemuEntry::new_breakpoint(0x4000, 0));
        let io = AdvancedRemuBreakPoint {
            brk_type: RemuBreakPointType::IO,
            access_mode: RemuBreakPointAccessMode::Write,
            run_mode: RemuBreakPointRunMode::Watch,
            addr: 0x7f00,
            mask: 0xc000,
            ..Default::default()
        };
        remu.add_entry(&RemuEntry::new_remu_breakpoint(io));
        snapshot.add_chunk(remu);
        let mut ace = AceBreakPointChunk::empty();
        ace.add_breakpoint(AceBreakPoint::new_execution(
            0x8000,
            AceBrkRuntimeMode::Break,
            AceMemMapType::Undefined,
        ));
        snapshot.add_chunk(ace);

        let path =
            std::env::temp_dir().join(format!("amsrust_breakpoints_{}.sna", std::process::id()));
        let path = path.to_str().unwrap().to_owned();
        snapshot.save(&path, SnapshotVersion::V3).unwrap();
        let breakpoints = load_breakpoints(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let exec: Vec<_> = breakpoints
            .exec
            .iter()
            .map(|b| (b.address, b.chunk))
            .collect();
        assert_eq!(exec, vec![(0x4000, "REMU"), (0x8000, "BRKC")]);
        assert_eq!(breakpoints.advanced.len(), 1);
        assert_eq!(
            breakpoints.advanced[0].run,
            crate::breakpoints::RunMode::Watch
        );
        assert!(breakpoints.ignored.is_empty(), "{:?}", breakpoints.ignored);
    }

    fn load_crtc_snapshot(version: SnapshotVersion, values: &[(SnapshotFlag, u16)]) -> Motherboard {
        let mut snapshot = Snapshot::default();
        for (flag, value) in values {
            snapshot.set_value(*flag, *value).unwrap();
        }
        let path = std::env::temp_dir().join(format!(
            "amsrust_crtc_{}_{version:?}.sna",
            std::process::id()
        ));
        let path = path.to_str().unwrap().to_owned();
        snapshot.save(&path, version).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        load_sna_bytes(&mut motherboard, &bytes).unwrap();
        motherboard
    }

    /// The CRTC registers are restored as stored (R0=0 used to swallow the R4, R5
    /// and R9 replayed after it), with the selected register and, in version 3,
    /// the counters.
    #[test]
    fn the_crtc_is_restored_as_saved() {
        use SnapshotFlag::*;
        let regs = [
            (0, 0),
            (4, 20),
            (5, 3),
            (9, 5),
            (1, 32),
            (12, 0x10),
            (7, 12),
        ];
        let mut values: Vec<_> = regs.iter().map(|&(r, v)| (CRTC_REG(Some(r)), v)).collect();
        values.push((CRTC_SEL, 12));
        values.extend([
            (CRTC_HCC, 10),
            (CRTC_CLC, 7),
            (CRTC_RLC, 2),
            (CRTC_VAC, 0),
            (CRTC_VSWC, 0),
            (CRTC_HSWC, 3),
            (CRTC_STATE, 1),
        ]);
        let motherboard = load_crtc_snapshot(SnapshotVersion::V3, &values);
        let crtc = motherboard.crtc();
        for (r, v) in regs {
            assert_eq!(crtc.regs_r(r), v as u8, "R{r}");
        }
        assert_eq!(crtc.selected_register(), 12);
        let counters = crtc.counters();
        assert_eq!(
            (counters.hcc, counters.vcc, counters.vlc, counters.vsc),
            (10, 7, 2, 3)
        );
        assert!(counters.vsync && !counters.hsync && !counters.in_vertical_adjust);

        // version 2 has no counters: the top of a frame
        let motherboard = load_crtc_snapshot(SnapshotVersion::V2, &values);
        let counters = motherboard.crtc().counters();
        assert_eq!((counters.hcc, counters.vcc, counters.vlc), (0, 0, 0));
        assert_eq!(motherboard.crtc().regs_r(4), 20);
    }

    /// A snapshot saved as `version`, its CPC type and MMR set, loaded
    /// into a 6128.
    fn load_typed(cpc_type: u16, mmr: u16, version: SnapshotVersion) -> (Motherboard, Snapshot) {
        let mut snapshot = Snapshot::default();
        snapshot
            .set_value(SnapshotFlag::CPC_TYPE, cpc_type)
            .unwrap();
        snapshot.set_value(SnapshotFlag::GA_RAMCFG, mmr).unwrap();
        let path = std::env::temp_dir().join(format!(
            "amsrust_model_{}_{cpc_type}_{mmr}.sna",
            std::process::id()
        ));
        let path = path.to_str().unwrap().to_owned();
        snapshot.save(&path, version).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        load_sna_bytes(&mut motherboard, &bytes).unwrap();
        (motherboard, Snapshot::from_buffer(bytes).unwrap())
    }

    #[test]
    fn the_snapshot_cpc_type_picks_the_model() {
        let (motherboard, snapshot) = load_typed(0, 0xc0, SnapshotVersion::V2);
        assert_eq!(model_of_snapshot(&snapshot), Some(CpcModel::Cpc464));
        assert_eq!(motherboard.model(), CpcModel::Cpc464);
        assert!(
            motherboard.has_disc_interface(),
            "the disc interface is kept, as a DDI-1"
        );
        let extra = snapshot.memory_size_header() > 64;
        assert_eq!(
            motherboard.has_ram_banking(),
            extra,
            "banked only with the snapshot's extra RAM"
        );

        let (motherboard, _) = load_typed(1, 0xc0, SnapshotVersion::V2);
        assert_eq!(motherboard.model(), CpcModel::Cpc664);
        // Version 1 has no CPC type, it leaves the model; the CPC+ types give a CPC+.
        assert_eq!(
            load_typed(0, 0xc0, SnapshotVersion::V1).0.model(),
            CpcModel::Cpc6128
        );
        assert_eq!(
            load_typed(4, 0xc0, SnapshotVersion::V2).0.model(),
            CpcModel::Cpc6128Plus
        );
    }

    #[test]
    fn the_snapshot_mmr_is_restored() {
        let (motherboard, _) = load_typed(2, 0xc4, SnapshotVersion::V2);
        assert_eq!(motherboard.model(), CpcModel::Cpc6128);
        assert_eq!(motherboard.mmr(), 0xc4);
        assert_eq!(
            motherboard.physical_address(0x4000),
            crate::memory::PhysicalAddress::Ram { bank: 4, offset: 0 }
        );
    }

    #[test]
    fn remu_advanced_entries_written_by_cpclib() {
        let mut chunk = RemuChunk::empty();
        let brk = AdvancedRemuBreakPoint {
            brk_type: RemuBreakPointType::Mem,
            access_mode: RemuBreakPointAccessMode::Write,
            addr: 0xc000,
            size: 0x4000,
            ..Default::default()
        };
        chunk.add_entry(&RemuEntry::new_remu_breakpoint(brk));
        let mut breakpoints = SnapshotBreakpoints::default();
        breakpoints_of_chunk("REMU", chunk.data(), &mut breakpoints);
        assert_eq!(breakpoints.ignored, Vec::<String>::new());
        let advanced = &breakpoints.advanced[0];
        assert_eq!(advanced.kind, crate::breakpoints::BreakpointKind::Memory);
        assert_eq!((advanced.address, advanced.size), (0xc000, 0x4000));
    }

    #[test]
    fn remu_entries_written_by_cpclib() {
        let mut chunk = RemuChunk::empty();
        chunk.add_entry(&RemuEntry::new_breakpoint(0x4000, 0));
        chunk.add_entry(&RemuEntry::new_label("start".to_owned(), 0x4000, 0));
        chunk.add_entry(&RemuEntry::new_rom_breakpoint(0xc006, 7));
        assert_eq!(parse("REMU", chunk.data()), (vec![0x4000, 0xc006], 0));
        assert_eq!(
            parse("REMU", b"brk #8000 0;brk &10 0;watch 1;"),
            (vec![0x8000, 0x10], 1)
        );
    }

    #[test]
    fn symbols_written_by_cpclib() {
        let mut remu = RemuChunk::empty();
        remu.add_entry(&RemuEntry::new_label("start".to_owned(), 0x4000, 0));
        remu.add_entry(&RemuEntry::new_alias("SCREEN".to_owned(), 0xc000));
        let mut symb = AceSymbolChunk::empty();
        symb.add_symbol(AceSymbol::new(
            "loop",
            0x4010,
            AceMemMapType::Undefined,
            AceSymbolType::Absolute,
        ));
        symb.add_symbol(AceSymbol::new(
            "data",
            0x8000,
            AceMemMapType::Undefined,
            AceSymbolType::Absolute,
        ));
        let mut breakpoints = SnapshotBreakpoints::default();
        breakpoints_of_chunk("REMU", remu.data(), &mut breakpoints);
        breakpoints_of_chunk("SYMB", symb.data(), &mut breakpoints);
        let symbols = &breakpoints.symbols;
        assert_eq!(symbols.address_of("start"), Some(0x4000));
        assert_eq!(symbols.address_of("SCREEN"), Some(0xc000));
        assert_eq!(symbols.address_of("loop"), Some(0x4010));
        assert_eq!(symbols.address_of("data"), Some(0x8000));
        assert!(
            breakpoints.ignored.is_empty() && breakpoints.is_empty(),
            "labels are not breakpoints"
        );
    }

    #[test]
    fn ace_execution_breakpoints_written_by_cpclib() {
        let mut chunk = AceBreakPointChunk::empty();
        chunk.add_breakpoint(AceBreakPoint::new_execution(
            0x1234,
            AceBrkRuntimeMode::Break,
            AceMemMapType::Undefined,
        ));
        chunk.add_breakpoint(AceBreakPoint::new_execution(
            0xbeef,
            AceBrkRuntimeMode::Break,
            AceMemMapType::Undefined,
        ));
        assert_eq!(parse("BRKC", chunk.data()), (vec![0x1234, 0xbeef], 0));
    }

    #[test]
    fn ace_execution_breakpoints_written_by_rasm() {
        // rasm -sb: the address low byte first, named "breakpoint<index>".
        let mut chunk = Vec::new();
        for (index, address) in [0x4000u16, 0x1234].into_iter().enumerate() {
            let mut record = [0u8; 216];
            record[4..6].copy_from_slice(&address.to_le_bytes());
            let name = format!("breakpoint{index}");
            record[144..144 + name.len()].copy_from_slice(name.as_bytes());
            chunk.extend_from_slice(&record);
        }
        assert_eq!(parse("BRKC", &chunk), (vec![0x4000, 0x1234], 0));
        // Another name: high byte first.
        chunk[144..155].copy_from_slice(b"breakpoint7");
        assert_eq!(parse("BRKC", &chunk), (vec![0x0040, 0x1234], 0));
    }

    #[test]
    fn brks_breakpoints_written_by_rasm_and_basm() {
        assert_eq!(
            parse("BRKS", &[0x00, 0x40, 0, 0, 0, 0x38, 0x00, 1, 0, 0]),
            (vec![0x4000, 0x0038], 0)
        );
    }

    /// `tests/fixtures/basm_breakpoints.asm` assembled by basm (cpclib):
    /// an execution breakpoint in REMU, BRKS and BRKC, a memory watch and
    /// an IO breakpoint with a condition as REMU `acebreak` entries.
    #[test]
    fn breakpoints_of_a_basm_snapshot() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basm_breakpoints.sna"
        );
        let breakpoints = load_breakpoints(path).unwrap();
        assert!(breakpoints.ignored.is_empty(), "{:?}", breakpoints.ignored);
        let exec: Vec<_> = breakpoints.exec.iter().map(|b| b.address).collect();
        assert_eq!(exec, vec![0x4000]);
        let advanced: Vec<_> = breakpoints
            .advanced
            .iter()
            .map(|b| {
                (
                    b.kind,
                    b.address,
                    b.size,
                    b.run,
                    b.name.as_deref(),
                    b.condition.is_some(),
                )
            })
            .collect();
        use crate::breakpoints::{BreakpointKind, RunMode};
        assert_eq!(
            advanced,
            vec![
                (
                    BreakpointKind::Memory,
                    0xc000,
                    16,
                    RunMode::Watch,
                    Some("screen"),
                    false
                ),
                (
                    BreakpointKind::Io,
                    0x7f00,
                    1,
                    RunMode::Stop,
                    Some("ink"),
                    true
                ),
            ]
        );
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

    /// A booted machine saved and loaded into another has the same state, and both then run
    /// the same way.
    #[test]
    fn a_saved_snapshot_loads_back() {
        use crate::z80::{HasValue, Reg16};
        let booted = || {
            let mut builder = crate::motherboard::MotherboardBuilder::default();
            builder.enable_interruptions();
            builder.build()
        };
        let mut machine = booted();
        let t_states = |mb: &Motherboard| mb.t_state_counter().value();
        while t_states(&machine) < 2_000_000 {
            machine.execute_next_opcode_no_video();
        }
        machine.psg_mut().set_register(8, 13);
        let bytes = save_sna_bytes(&machine).unwrap();
        assert_eq!(
            Snapshot::from_buffer(bytes.clone())
                .unwrap()
                .version_header(),
            3
        );

        let mut copy = booted();
        load_sna_bytes(&mut copy, &bytes).unwrap();
        for register in [
            Reg16::Af,
            Reg16::Bc,
            Reg16::De,
            Reg16::Hl,
            Reg16::Ix,
            Reg16::Iy,
            Reg16::Sp,
            Reg16::Pc,
            Reg16::Af2,
        ] {
            assert_eq!(
                copy.cpu().reg16(register).value(),
                machine.cpu().reg16(register).value(),
                "{register:?}"
            );
        }
        assert_eq!(copy.ram_dump(), machine.ram_dump());
        assert_eq!(copy.mmr(), machine.mmr());
        assert_eq!(copy.ga().palette(), machine.ga().palette());
        assert_eq!(copy.ga().rmr(), machine.ga().rmr());
        assert_eq!(copy.crtc().registers(), machine.crtc().registers());
        assert_eq!(copy.crtc().counters(), machine.crtc().counters());
        assert_eq!(copy.psg().register(8), 13);
        assert_eq!(copy.ppi().control_word(), machine.ppi().control_word());
        assert_eq!(
            copy.ppi().get_data_c_from_pins(),
            machine.ppi().get_data_c_from_pins()
        );
        // and a saved copy saves the same
        let again = save_sna_bytes(&copy).unwrap();
        let differ: Vec<_> = again
            .iter()
            .zip(&bytes)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i, *a, *b))
            .take(10)
            .collect();
        assert!(
            differ.is_empty() && again.len() == bytes.len(),
            "{differ:x?}"
        );

        // the firmware goes on the same way on both
        let start = t_states(&copy);
        while t_states(&copy) < start + 1_000_000 {
            copy.execute_next_opcode_no_video();
        }
        let start = t_states(&machine);
        while t_states(&machine) < start + 1_000_000 {
            machine.execute_next_opcode_no_video();
        }
        assert_eq!(copy.ram_dump(), machine.ram_dump());
    }
}
