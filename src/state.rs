//! The machine's state as the debugger's windows and its API show it: one typed snapshot a
//! chip, read from the machine, that the windows draw and the API serves as JSON
//! (`serde_json::to_value`). What a window learns to show goes here, and so reaches the API (and
//! any other interface) too.

use serde::Serialize;

use crate::motherboard::Motherboard;
use crate::z80::{HasValue, InterruptMode, Reg16, Reg8};

// ------------------------------------------------------------------- Z80

/// `GET /api/z80`: the registers (AMSpiriT's names), the interrupts, the time.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub struct Z80State {
    pub pc: u16,
    pub sp: u16,
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub a2: u8,
    pub f2: u8,
    pub b2: u8,
    pub c2: u8,
    pub d2: u8,
    pub e2: u8,
    pub h2: u8,
    pub l2: u8,
    pub ix: u16,
    pub iy: u16,
    pub i: u8,
    pub r: u8,
    pub iff1: u8,
    pub iff2: u8,
    pub im: u8,
    /// NOPs since the machine started.
    #[serde(rename = "ticks")]
    pub ticks: usize,
    /// WZ (MEMPTR).
    pub wz: u16,
    /// The flags set, by name (`SZ-H-PNC`, a dash for a clear one).
    #[serde(rename = "flags")]
    pub flags: String,
    /// On a HALT, waiting for an interrupt.
    #[serde(rename = "halted")]
    pub halted: bool,
}

impl Z80State {
    pub fn of(mb: &Motherboard) -> Self {
        let cpu = mb.cpu();
        let r8 = |reg: Reg8| cpu.reg8(reg).value();
        let r16 = |reg: Reg16| cpu.reg16(reg).value();
        let pc = r16(Reg16::Pc);
        Self {
            pc,
            sp: r16(Reg16::Sp),
            a: r8(Reg8::A),
            f: r8(Reg8::F),
            b: r8(Reg8::B),
            c: r8(Reg8::C),
            d: r8(Reg8::D),
            e: r8(Reg8::E),
            h: r8(Reg8::H),
            l: r8(Reg8::L),
            a2: r8(Reg8::A2),
            f2: r8(Reg8::F2),
            b2: r8(Reg8::B2),
            c2: r8(Reg8::C2),
            d2: r8(Reg8::D2),
            e2: r8(Reg8::E2),
            h2: r8(Reg8::H2),
            l2: r8(Reg8::L2),
            ix: r16(Reg16::Ix),
            iy: r16(Reg16::Iy),
            i: r8(Reg8::I),
            r: r8(Reg8::R),
            iff1: cpu.iff1() as u8,
            iff2: cpu.iff2() as u8,
            im: match cpu.im() {
                InterruptMode::Mode0 => 0,
                InterruptMode::Mode1 => 1,
                InterruptMode::Mode2 => 2,
            },
            ticks: mb.t_state_counter().value() / 4,
            wz: r16(Reg16::Wz),
            flags: flag_names(r8(Reg8::F)),
            // a HALT runs again until an interrupt: the PC stays on it
            halted: mb.read_byte(pc) == 0x76,
        }
    }
}

/// The flags of `f` by name: `SZ-H-PNC` order (bits 7-0), a dash for a clear one.
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

/// One tone channel as a musician reads it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PsgChannel {
    pub period: u16,
    pub frequency_hz: f64,
    /// The nearest note, how far from it (cents), its MIDI number: `None` out of hearing.
    pub note: Option<String>,
    pub cents: Option<i32>,
    pub midi: Option<i32>,
    /// Its tone and the noise are mixed in.
    pub tone: bool,
    pub noise: bool,
    /// Its volume (0-15), or the envelope's.
    pub volume: u8,
    pub envelope: bool,
}

impl PsgChannel {
    /// It sounds a note: its tone on, a volume.
    pub fn sounding(&self) -> Option<i32> {
        self.midi
            .filter(|_| self.tone && (self.envelope || self.volume != 0))
    }
}

/// `GET /api/psg`: AMSpiriT's fields, the 16 registers, and the channels read.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PsgState {
    pub period_a: u16,
    pub vol_a: u8,
    pub period_b: u16,
    pub vol_b: u8,
    pub period_c: u16,
    pub vol_c: u8,
    pub mixer: u8,
    pub noise: u8,
    pub env_period: u16,
    pub env_shape: u8,
    pub regs: Vec<u8>,
    pub selected_reg: Option<u8>,
    pub channels: Vec<PsgChannel>,
    pub noise_hz: f64,
    pub envelope_hz: f64,
    /// Port A reads (the keyboard), else writes.
    pub port_a_input: bool,
}

impl PsgState {
    pub fn of(mb: &Motherboard) -> Self {
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
        Self {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Unformatted,
    WeakSectors,
    Standard,
    NoSector,
    AnotherLayout,
}

impl TrackKind {
    pub fn of(track: Option<&crate::disc_image::Track>) -> Self {
        match track {
            None => Self::Unformatted,
            Some(track) if track.sectors.iter().any(|s| s.copies() > 1) => Self::WeakSectors,
            Some(track) if track.sectors.len() == 9 && track.sectors.iter().all(|s| s.n == 2) => {
                Self::Standard
            }
            Some(track) if track.sectors.is_empty() => Self::NoSector,
            Some(_) => Self::AnotherLayout,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unformatted => "unformatted",
            Self::WeakSectors => "weak sectors",
            Self::Standard => "9 x 512",
            Self::NoSector => "no sector",
            Self::AnotherLayout => "another layout",
        }
    }
}

/// A sector of the track under the head.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SectorState {
    pub c: u8,
    pub h: u8,
    pub r: u8,
    pub n: u8,
    /// The size its N announces, and the data the image holds.
    pub size: usize,
    pub data_len: usize,
    /// What is special about it: weak (several copies), deleted, another track's ID, odd size.
    pub notes: Vec<String>,
}

/// A file of the disc's AMSDOS catalogue.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CatalogueEntry {
    pub name: String,
    pub user: u8,
    pub hidden: bool,
    pub kilobytes: u32,
}

/// The disc in a drive as the head sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiscState {
    /// DSK or HFE.
    pub image: &'static str,
    /// The format of the track under the head (see [`format_name`]).
    pub format: String,
    pub tracks: u8,
    pub double_sided: bool,
    /// The side read: A or B.
    pub side: &'static str,
    pub head_track: u8,
    /// Each track of the side read.
    pub track_kinds: Vec<TrackKind>,
    /// Each track's sector IDs (R), in their order on the track.
    pub track_sectors: Vec<Vec<u8>>,
    /// The track under the head, in the order its sectors pass, the next one's index.
    pub sectors: Vec<SectorState>,
    pub next_sector: Option<usize>,
    pub catalogue: Vec<CatalogueEntry>,
}

impl DiscState {
    /// The disc of drive `unit` (`None`: no disc).
    pub fn of(mb: &Motherboard, unit: usize) -> Option<Self> {
        use crate::fdc::FdcDrive;
        let drive = if unit == 0 { FdcDrive::A } else { FdcDrive::B };
        let fdc = mb.fdc();
        let disc = fdc.image(drive)?;
        let side = fdc.disc_side(drive).min(disc.nb_heads().saturating_sub(1));
        let head_track = fdc.track(unit);
        let under = disc.track(side, head_track);
        let sectors: Vec<SectorState> = under
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
                        SectorState {
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
        let next_sector =
            (!sectors.is_empty()).then(|| fdc.sector_under_head(unit) % sectors.len());
        Some(Self {
            image: if disc.is_bitstream() { "HFE" } else { "DSK" },
            format: under.map_or("unformatted".to_owned(), format_name),
            tracks: disc.nb_tracks(),
            double_sided: disc.is_double_sided(),
            side: ["A", "B"][usize::from(side & 1)],
            head_track,
            track_kinds: (0..disc.nb_tracks())
                .map(|track| TrackKind::of(disc.track(side, track)))
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
}

/// A drive of `GET /api/fdc`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DriveState {
    pub connected: bool,
    /// 3 or 3.5 (inches).
    pub kind: &'static str,
    pub disk: bool,
    pub side: &'static str,
    pub track: u8,
    /// Its disc as the head sees it.
    pub disc: Option<DiscState>,
}

/// `GET /api/fdc`: the status registers, the phase, the drives and their discs.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FdcState {
    pub msr: u8,
    pub sr0: u8,
    pub sr1: u8,
    pub sr2: u8,
    pub motor: bool,
    pub phase: &'static str,
    pub command: String,
    pub drives: Vec<DriveState>,
}

impl FdcState {
    pub fn of(mb: &Motherboard) -> Self {
        use crate::fdc::FdcDrive;
        let fdc = mb.fdc();
        let [st0, st1, st2] = fdc.status_registers();
        Self {
            msr: fdc.read_status(),
            sr0: st0,
            sr1: st1,
            sr2: st2,
            motor: fdc.motor(),
            phase: fdc.phase_name(),
            command: fdc.command_name(),
            drives: [FdcDrive::A, FdcDrive::B]
                .iter()
                .enumerate()
                .map(|(unit, &drive)| DriveState {
                    connected: true,
                    kind: if fdc.drive_kind(drive).is_double_sided() {
                        "3.5"
                    } else {
                        "3"
                    },
                    disk: fdc.has_disk(drive),
                    side: if fdc.disc_side(drive) == 0 { "A" } else { "B" },
                    track: fdc.track(unit),
                    disc: DiscState::of(mb, unit),
                })
                .collect(),
        }
    }
}

// ------------------------------------------------------------------- ASIC

/// A DMA channel of the ASIC.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DmaChannel {
    pub address: u16,
    pub prescaler: u8,
    pub enabled: bool,
    pub pause: u16,
    pub loop_count: u16,
    pub interrupt: bool,
}

/// A hardware sprite of the ASIC.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sprite {
    pub x: u16,
    pub y: u16,
    pub zoom_x: u8,
    pub zoom_y: u8,
    pub shown: bool,
    /// Its 16 x 16 pens, row by row (0: transparent).
    pub pixels: Vec<u8>,
}

/// `GET /api/asic` (a CPC+): the lock, the palette, its registers, the DMA and the sprites.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AsicState {
    pub unlocked: bool,
    /// The 32 palette entries (12 bits, &GRB): pens 0-15, the border, sprite colours 1-15.
    pub palette: Vec<u16>,
    pub pri: u8,
    pub splt: u8,
    pub sscr: u8,
    pub ivr: u8,
    pub rmr2: u8,
    pub dma: Vec<DmaChannel>,
    pub sprites: Vec<Sprite>,
}

impl AsicState {
    /// `None` on a CPC.
    pub fn of(mb: &Motherboard) -> Option<Self> {
        let (asic, video) = (mb.asic()?, mb.ga().plus()?);
        Some(Self {
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
                .map(|c| DmaChannel {
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
                    Sprite {
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
        assert_eq!(TrackKind::of(Some(&data)), TrackKind::Standard);
        let packed = track(
            &[0xc1, 0xc6, 0xc2, 0xc7, 0xc3, 0xc8, 0xc4, 0xc9, 0xc5, 0xca],
            2,
        );
        assert_eq!(format_name(&packed), "10 sectors &C1-&CA");
        assert_eq!(TrackKind::of(Some(&packed)), TrackKind::AnotherLayout);
        assert_eq!(
            format_name(&track(
                &[0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49],
                2
            )),
            "System"
        );
        assert_eq!(TrackKind::of(None), TrackKind::Unformatted);
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
        let value = serde_json::to_value(Z80State::of(&Motherboard::default())).unwrap();
        for key in [
            "PC", "SP", "A2", "IFF1", "IM", "ticks", "WZ", "flags", "halted",
        ] {
            assert!(value.get(key).is_some(), "{key}: {value}");
        }
    }

    #[test]
    fn a_cpc_has_no_asic_a_plus_has_one() {
        assert!(AsicState::of(&Motherboard::default()).is_none());
        let mut builder = crate::motherboard::MotherboardBuilder::default();
        builder.set_model(crate::motherboard::CpcModel::Cpc6128Plus);
        let asic = AsicState::of(&builder.build()).expect("an ASIC");
        assert_eq!((asic.palette.len(), asic.sprites.len()), (32, 16));
    }
}
