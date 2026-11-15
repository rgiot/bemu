//! The machine's state as the debugger's windows and its API show it: the API's reply types
//! (`bemu-api`), read from the machine here. The windows draw them and the API serves them, so
//! what a window learns to show reaches the API (and any other interface) too.

pub use bemu_api::{
    Asic, AsicDma, AsicSprite, CatalogueEntry, Disc, DiscSector, Fdc, FdcDrive, Psg, PsgChannel,
    TrackKind, Z80Registers,
};

use crate::motherboard::Motherboard;
use crate::z80::{HasValue, InterruptMode, Reg16, Reg8};

// ------------------------------------------------------------------- Z80

/// `GET /api/z80`: the registers, the interrupts, the time, WZ, the flags by name, HALT.
pub fn z80(mb: &Motherboard) -> Z80Registers {
    let cpu = mb.cpu();
    let r8 = |reg: Reg8| cpu.reg8(reg).value();
    let r16 = |reg: Reg16| cpu.reg16(reg).value();
    let pc = r16(Reg16::Pc);
    Z80Registers {
        PC: pc,
        SP: r16(Reg16::Sp),
        IX: r16(Reg16::Ix),
        IY: r16(Reg16::Iy),
        A: r8(Reg8::A),
        F: r8(Reg8::F),
        B: r8(Reg8::B),
        C: r8(Reg8::C),
        D: r8(Reg8::D),
        E: r8(Reg8::E),
        H: r8(Reg8::H),
        L: r8(Reg8::L),
        A2: r8(Reg8::A2),
        F2: r8(Reg8::F2),
        B2: r8(Reg8::B2),
        C2: r8(Reg8::C2),
        D2: r8(Reg8::D2),
        E2: r8(Reg8::E2),
        H2: r8(Reg8::H2),
        L2: r8(Reg8::L2),
        I: r8(Reg8::I),
        R: r8(Reg8::R),
        IFF1: cpu.iff1() as u8,
        IFF2: cpu.iff2() as u8,
        IM: match cpu.im() {
            InterruptMode::Mode0 => 0,
            InterruptMode::Mode1 => 1,
            InterruptMode::Mode2 => 2,
        },
        ticks: (mb.t_state_counter().value() / 4) as u64,
        WZ: r16(Reg16::Wz),
        flags: flag_names(r8(Reg8::F)),
        // a HALT runs again until an interrupt: the PC stays on it
        halted: mb.read_byte(pc) == 0x76,
    }
}

/// The flags of `f` by name: `SZYHXPNC` (bits 7-0), a dash for a clear one.
pub fn flag_names(f: u8) -> String {
    "SZYHXPNC"
        .chars()
        .enumerate()
        .map(|(bit, name)| if f & (0x80 >> bit) != 0 { name } else { '-' })
        .collect()
}

// ------------------------------------------------------------------- PSG

/// The AY's clock on a CPC: 1 MHz.
pub const PSG_HZ: f64 = 1_000_000.0;

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A tone's period (R0-R5) as a frequency.
pub fn tone_hz(period: u16) -> f64 {
    PSG_HZ / (16.0 * f64::from(period.max(1)))
}

/// The nearest note (MIDI number) and how far from it, in cents; `None` out of hearing.
pub fn nearest_note(frequency: f64) -> Option<(i32, i32)> {
    if !(16.0..20_000.0).contains(&frequency) {
        return None;
    }
    let exact = 69.0 + 12.0 * (frequency / 440.0).log2();
    let midi = exact.round();
    Some((midi as i32, ((exact - midi) * 100.0).round() as i32))
}

/// A MIDI note's name (`A4`).
pub fn note_name(midi: i32) -> String {
    format!(
        "{}{}",
        NOTE_NAMES[midi.rem_euclid(12) as usize],
        midi / 12 - 1
    )
}

/// `GET /api/psg`: AMSpiriT's fields, the 16 registers, and the channels read.
pub fn psg(mb: &Motherboard) -> Psg {
    let psg = mb.psg();
    let reg = |r: u8| psg.register(r);
    let word = |low: u8| u16::from_le_bytes([reg(low), reg(low + 1)]);
    let mixer = reg(7);
    let channels = (0..3u8)
        .map(|channel| {
            let period = word(channel * 2) & 0xfff;
            let frequency_hz = tone_hz(period);
            let note = nearest_note(frequency_hz);
            let volume = reg(8 + channel);
            PsgChannel {
                period,
                frequency_hz,
                note: note.map(|(midi, _)| note_name(midi)),
                cents: note.map(|(_, cents)| cents),
                midi: note.map(|(midi, _)| midi),
                tone: mixer & (1 << channel) == 0,
                noise: mixer & (8 << channel) == 0,
                volume: volume & 0x0f,
                envelope: volume & 0x10 != 0,
            }
        })
        .collect();
    let noise = reg(6) & 0x1f;
    let env_period = word(11);
    Psg {
        period_a: word(0) & 0xfff,
        vol_a: reg(8) & 0x1f,
        period_b: word(2) & 0xfff,
        vol_b: reg(9) & 0x1f,
        period_c: word(4) & 0xfff,
        vol_c: reg(10) & 0x1f,
        mixer,
        noise,
        env_period,
        env_shape: reg(13) & 0x0f,
        regs: (0..16).map(reg).collect(),
        selected_reg: psg.selected_register(),
        channels,
        noise_hz: PSG_HZ / (16.0 * f64::from(noise.max(1))),
        envelope_hz: PSG_HZ / (256.0 * f64::from(env_period.max(1))),
        port_a_input: psg.port_a_is_input(),
    }
}

// ------------------------------------------------------------------- FDC

/// The format a track's sector IDs name: AMSDOS's Data (&C1-&C9), System (&41-&49) or IBM
/// (&01-&08), else its sectors and their IDs ("10 sectors &C1-&CA").
pub fn format_name(track: &crate::disc_image::Track) -> String {
    let ids: Vec<u8> = track.sectors.iter().map(|s| s.r).collect();
    let all = |range: std::ops::RangeInclusive<u8>| {
        ids.len() == range.clone().count() && ids.iter().all(|r| range.contains(r))
    };
    if all(0xc1..=0xc9) {
        "Data".to_owned()
    } else if all(0x41..=0x49) {
        "System".to_owned()
    } else if all(0x01..=0x08) {
        "IBM".to_owned()
    } else if let (Some(first), Some(last)) = (ids.iter().min(), ids.iter().max()) {
        format!("{} sectors &{first:02X}-&{last:02X}", ids.len())
    } else {
        "no sector".to_owned()
    }
}

/// A track in a word: unformatted, the standard 9 sectors of 512 bytes, or another layout (a
/// protection, a more packed format), weak sectors (several copies of their data) above all.
pub fn track_kind(track: Option<&crate::disc_image::Track>) -> TrackKind {
    match track {
        None => TrackKind::Unformatted,
        Some(track) if track.sectors.iter().any(|s| s.copies() > 1) => TrackKind::WeakSectors,
        Some(track) if track.sectors.len() == 9 && track.sectors.iter().all(|s| s.n == 2) => {
            TrackKind::Standard
        }
        Some(track) if track.sectors.is_empty() => TrackKind::NoSector,
        Some(_) => TrackKind::AnotherLayout,
    }
}

/// The disc of drive `unit` as the head sees it (`None`: no disc).
pub fn disc(mb: &Motherboard, unit: usize) -> Option<Disc> {
    use crate::fdc::FdcDrive as Drive;
    let drive = if unit == 0 { Drive::A } else { Drive::B };
    let fdc = mb.fdc();
    let disc = fdc.image(drive)?;
    let side = fdc.disc_side(drive).min(disc.nb_heads().saturating_sub(1));
    let head_track = fdc.track(unit);
    let under = disc.track(side, head_track);
    let sectors: Vec<DiscSector> = under
        .map(|track| {
            track
                .sectors
                .iter()
                .map(|sector| {
                    let mut notes = Vec::new();
                    if sector.copies() > 1 {
                        notes.push(format!("weak: {} copies", sector.copies()));
                    }
                    if sector.is_deleted() {
                        notes.push("deleted".to_owned());
                    }
                    if sector.c != head_track {
                        notes.push(format!("ID of track {}", sector.c));
                    }
                    if sector.data.len() != sector.nominal_len() && sector.copies() <= 1 {
                        notes.push("odd size".to_owned());
                    }
                    DiscSector {
                        c: sector.c,
                        h: sector.h,
                        r: sector.r,
                        n: sector.n,
                        size: sector.nominal_len(),
                        data_len: sector.data.len(),
                        notes,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let next_sector = (!sectors.is_empty()).then(|| fdc.sector_under_head(unit) % sectors.len());
    Some(Disc {
        image: if disc.is_bitstream() { "HFE" } else { "DSK" }.to_owned(),
        format: under.map_or("unformatted".to_owned(), format_name),
        tracks: disc.nb_tracks(),
        double_sided: disc.is_double_sided(),
        side: ["A", "B"][usize::from(side & 1)].to_owned(),
        head_track,
        track_kinds: (0..disc.nb_tracks())
            .map(|track| track_kind(disc.track(side, track)))
            .collect(),
        track_sectors: (0..disc.nb_tracks())
            .map(|track| {
                disc.track(side, track)
                    .map(|t| t.sectors.iter().map(|s| s.r).collect())
                    .unwrap_or_default()
            })
            .collect(),
        sectors,
        next_sector,
        catalogue: disc
            .amsdos_catalog()
            .iter()
            .map(|entry| CatalogueEntry {
                name: format!("{}.{}", entry.name.trim(), entry.extension.trim()),
                user: entry.user,
                hidden: entry.hidden,
                kilobytes: (u32::from(entry.records) * 128).div_ceil(1024),
            })
            .collect(),
    })
}

/// `GET /api/fdc`: the status registers, the phase, the drives and their discs.
pub fn fdc(mb: &Motherboard) -> Fdc {
    use crate::fdc::FdcDrive as Drive;
    let fdc = mb.fdc();
    let [st0, st1, st2] = fdc.status_registers();
    Fdc {
        msr: fdc.read_status(),
        sr0: st0,
        sr1: st1,
        sr2: st2,
        motor: fdc.motor(),
        phase: fdc.phase_name().to_owned(),
        command: fdc.command_name().to_string(),
        drives: [Drive::A, Drive::B]
            .iter()
            .enumerate()
            .map(|(unit, &drive)| FdcDrive {
                connected: true,
                kind: if fdc.drive_kind(drive).is_double_sided() {
                    "3.5"
                } else {
                    "3"
                }
                .to_owned(),
                disk: fdc.has_disk(drive),
                side: if fdc.disc_side(drive) == 0 { "A" } else { "B" }.to_owned(),
                track: fdc.track(unit),
                disc: disc(mb, unit),
            })
            .collect(),
    }
}

// ------------------------------------------------------------------- ASIC

/// `GET /api/asic`: a CPC+'s ASIC (`None` on a CPC).
pub fn asic(mb: &Motherboard) -> Option<Asic> {
    let (asic, video) = (mb.asic()?, mb.ga().plus()?);
    Some(Asic {
        unlocked: video.unlocked,
        palette: (0..32).map(|i| video.palette_entry(i)).collect(),
        pri: asic.pri(),
        splt: asic.splt(),
        sscr: video.sscr(),
        ivr: asic.ivr(),
        rmr2: video.rmr2(),
        dma: asic
            .dma_channels()
            .iter()
            .map(|c| AsicDma {
                address: c.address,
                prescaler: c.prescaler,
                enabled: c.enabled,
                pause: c.pause,
                loop_count: c.loop_count,
                interrupt: c.interrupt,
            })
            .collect(),
        sprites: (0..16)
            .map(|n| {
                let sprite = video.sprite(n);
                AsicSprite {
                    x: sprite.x,
                    y: sprite.y,
                    zoom_x: sprite.zoom_x,
                    zoom_y: sprite.zoom_y,
                    shown: sprite.zoom_x != 0 && sprite.zoom_y != 0,
                    pixels: sprite.pixels.to_vec(),
                }
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_is_named_by_its_sectors() {
        use crate::disc_image::{Sector, Track};
        let track = |ids: &[u8], n: u8| Track {
            sectors: ids.iter().map(|&r| Sector::new(0, 0, r, n, 0xe5)).collect(),
            track: 0,
            head: 0,
            gap3: 0x4e,
            filler: 0xe5,
        };
        let data = track(&[0xc1, 0xc6, 0xc2, 0xc7, 0xc3, 0xc8, 0xc4, 0xc9, 0xc5], 2);
        assert_eq!(format_name(&data), "Data");
        assert_eq!(track_kind(Some(&data)), TrackKind::Standard);
        let packed = track(
            &[0xc1, 0xc6, 0xc2, 0xc7, 0xc3, 0xc8, 0xc4, 0xc9, 0xc5, 0xca],
            2,
        );
        assert_eq!(format_name(&packed), "10 sectors &C1-&CA");
        assert_eq!(track_kind(Some(&packed)), TrackKind::AnotherLayout);
        assert_eq!(
            format_name(&track(
                &[0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49],
                2
            )),
            "System"
        );
        assert_eq!(track_kind(None), TrackKind::Unformatted);
    }

    #[test]
    fn flags_are_named() {
        assert_eq!(flag_names(0x00), "--------");
        assert_eq!(flag_names(0x41), "-Z-----C");
    }

    #[test]
    fn a_tone_is_its_note() {
        // period 142 at 1 MHz: 440.1 Hz, A4
        let (midi, cents) = nearest_note(tone_hz(142)).unwrap();
        assert_eq!(note_name(midi), "A4");
        assert!(cents.abs() < 5);
    }

    /// The API's names stay those of AMSpiriT (and the windows read the same values).
    #[test]
    fn the_z80_keeps_its_api_names() {
        let value = serde_json::to_value(z80(&Motherboard::default())).unwrap();
        for key in [
            "PC", "SP", "A2", "IFF1", "IM", "ticks", "WZ", "flags", "halted",
        ] {
            assert!(value.get(key).is_some(), "{key}: {value}");
        }
    }

    #[test]
    fn a_cpc_has_no_asic_a_plus_has_one() {
        assert!(asic(&Motherboard::default()).is_none());
        let mut builder = crate::motherboard::MotherboardBuilder::default();
        builder.set_model(crate::motherboard::CpcModel::Cpc6128Plus);
        let asic = asic(&builder.build()).expect("an ASIC");
        assert_eq!((asic.palette.len(), asic.sprites.len()), (32, 16));
    }
}
