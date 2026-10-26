//! Willy (PulkoMandy's): the expansion-port adapter for Serdaco's parallel-port sound cards.
//! - With an OPL3LPT, a Yamaha YMF262 (OPL3): write-only, base+0 selects a register of the first
//!   bank, base+2 of the second, base+1 and base+3 write the register selected.
//! - With an S2P, a SAM2695 General MIDI synthesizer: base+0 the MIDI data (written: to the
//!   synth; read: its answers), base+1 the control (written) and the status (read: bit 7 ready to
//!   take a byte, bit 6 a byte to read). Writing &3F to the control puts it in UART mode: it
//!   answers &FE. Its sound here: a SoundFont (rustysynth) given on the command line.
//!
//! A card takes 4 ports from its base, set by a jumper: &FEBC (the OPL3LPT's usual one), &FEAC
//! (the S2P's), &FEA4, &FE9C, &FE94, &FE8C or &FE84; several Willys can be plugged at once.

use std::any::Any;
use std::sync::Arc;

use nuked_opl3::{Opl3Device, OplRegisterFile};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

use crate::expansion_port::{ExpansionDevice, ExpansionPort};

/// The bases a jumper can set.
pub const BASES: [u16; 7] = [0xfebc, 0xfeac, 0xfea4, 0xfe9c, 0xfe94, 0xfe8c, 0xfe84];
pub const OPL3_BASE: u16 = 0xfebc;
pub const S2P_BASE: u16 = 0xfeac;

/// The OPL3's own rate (its 14.318 MHz clock / 288).
const OPL3_RATE: u32 = 49_716;
/// The S2P's rendering rate.
const S2P_RATE: i32 = 44_100;
/// Ticks of the port (16 MHz) in a microsecond.
const TICKS_PER_US: u32 = 16;

/// `&FEBC`, `#FEBC`, `$FEBC`, `0xFEBC` or `FEBC`: one of [`BASES`].
pub fn parse_base(text: &str) -> Result<u16, String> {
    let digits = text.trim_start_matches(['&', '#', '$']).trim_start_matches("0x").trim_start_matches("0X");
    let base = u16::from_str_radix(digits, 16).map_err(|_| format!("{text}: not an address"))?;
    if BASES.contains(&base) {
        Ok(base)
    }
    else {
        Err(format!("{text}: Willy's jumper sets &FEBC, &FEAC, &FEA4, &FE9C, &FE94, &FE8C or &FE84"))
    }
}

/// Willy with an OPL3LPT.
pub struct Opl3Lpt {
    base: u16,
    chip: Opl3Device,
    /// The register selected, and its bank (the second: true).
    selected: (u8, bool),
    ticks: u32,
    sample: [f32; 2],
    /// Register writes so far (for the tests and the debugger).
    writes: u64
}

impl Opl3Lpt {
    pub fn new(base: u16) -> Self {
        Self {
            base,
            chip: Opl3Device::new(OPL3_RATE),
            selected: (0, false),
            ticks: 0,
            sample: [0.0; 2],
            writes: 0
        }
    }

    pub fn base(&self) -> u16 {
        self.base
    }

    pub fn register_writes(&self) -> u64 {
        self.writes
    }

    /// A write to one of the card's 4 ports (0-3).
    pub fn write(&mut self, port: u16, value: u8) {
        match port & 3 {
            0 => self.selected = (value, false),
            2 => self.selected = (value, true),
            // the data goes to the register selected last, whichever bank
            _ => {
                let (register, second) = self.selected;
                let bank = || if second { OplRegisterFile::Secondary } else { OplRegisterFile::Primary };
                let _ = self.chip.write_address(register, bank());
                let _ = self.chip.write_data(value, bank(), false);
                self.writes += 1;
            }
        }
    }

    /// One microsecond of the chip: the samples it makes in it, the last one kept.
    fn run_microsecond(&mut self) {
        let mut frame = [0i16; 2];
        for _ in 0..self.chip.run(1.0) {
            let _ = self.chip.generate(&mut frame);
            self.sample = [f32::from(frame[0]) / 32768.0, f32::from(frame[1]) / 32768.0];
        }
    }
}

impl ExpansionDevice for Opl3Lpt {
    fn tick(&mut self, port: &mut ExpansionPort) {
        if let Some((address, value)) = port.io_write_started() {
            if address & 0xfffc == self.base {
                self.write(address, value);
            }
        }
        self.ticks += 1;
        if self.ticks == TICKS_PER_US {
            self.ticks = 0;
            self.run_microsecond();
        }
    }

    fn reset(&mut self) {
        let _ = self.chip.reset(None);
        self.selected = (0, false);
        self.sample = [0.0; 2];
    }

    fn sound(&self) -> [f32; 2] {
        self.sample
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The MIDI bytes put together into messages (running status; SysEx and real-time skipped).
#[derive(Default)]
struct MidiParser {
    status: u8,
    data: Vec<u8>,
    in_sysex: bool
}

impl MidiParser {
    /// The message a byte completes: (status, data 1, data 2).
    fn push(&mut self, byte: u8) -> Option<(u8, u8, u8)> {
        match byte {
            0xf8..=0xff => None,
            0xf0 => {
                self.in_sysex = true;
                None
            },
            0xf7 => {
                self.in_sysex = false;
                None
            },
            0xf1..=0xf6 => {
                self.status = 0;
                None
            },
            0x80..=0xef => {
                self.in_sysex = false;
                self.status = byte;
                self.data.clear();
                None
            },
            _ if self.in_sysex || self.status == 0 => None,
            _ => {
                self.data.push(byte);
                let length = if matches!(self.status & 0xf0, 0xc0 | 0xd0) { 1 } else { 2 };
                (self.data.len() == length).then(|| {
                    let message = (self.status, self.data[0], self.data.get(1).copied().unwrap_or(0));
                    self.data.clear();
                    message
                })
            }
        }
    }
}

/// Willy with an S2P.
pub struct S2p {
    base: u16,
    synth: Synthesizer,
    parser: MidiParser,
    /// What the card answers, read from base+0.
    answers: std::collections::VecDeque<u8>,
    left: Vec<f32>,
    right: Vec<f32>,
    /// The sample played in the block, and the time towards the next (in 1/1000000 of a sample).
    position: usize,
    fraction: u32,
    ticks: u32,
    messages: u64,
    /// The byte the card puts on the bus for the IN being answered (taken when it began).
    answering: Option<u8>
}

impl S2p {
    /// The S2P, its instruments the SoundFont's.
    pub fn new(base: u16, soundfont: &[u8]) -> Result<Self, String> {
        let font = SoundFont::new(&mut std::io::Cursor::new(soundfont)).map_err(|e| format!("the SoundFont: {e:?}"))?;
        let settings = SynthesizerSettings::new(S2P_RATE);
        let synth = Synthesizer::new(&Arc::new(font), &settings).map_err(|e| format!("the synthesizer: {e:?}"))?;
        let block = synth.get_block_size();
        Ok(Self {
            base,
            synth,
            parser: MidiParser::default(),
            answers: Default::default(),
            left: vec![0.0; block],
            right: vec![0.0; block],
            position: block,
            fraction: 0,
            ticks: 0,
            messages: 0,
            answering: None
        })
    }

    pub fn open(base: u16, path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::new(base, &bytes)
    }

    pub fn base(&self) -> u16 {
        self.base
    }

    /// The MIDI messages played so far.
    pub fn messages(&self) -> u64 {
        self.messages
    }

    pub fn status(&self) -> u8 {
        0x80 | if self.answers.is_empty() { 0 } else { 0x40 }
    }

    pub fn write(&mut self, port: u16, value: u8) {
        match port & 3 {
            0 => {
                if let Some((status, data1, data2)) = self.parser.push(value) {
                    self.synth.process_midi_message(i32::from(status & 0x0f), i32::from(status & 0xf0), i32::from(data1), i32::from(data2));
                    self.messages += 1;
                }
            },
            1 if value == 0x3f => {
                // UART mode, acknowledged
                self.answers.clear();
                self.answers.push_back(0xfe);
            },
            _ => {}
        }
    }

    /// A read of one of the card's ports; `consume` takes the byte read off the data port.
    pub fn read(&mut self, port: u16, consume: bool) -> u8 {
        match port & 3 {
            0 if consume => self.answers.pop_front().unwrap_or(0xff),
            0 => self.answers.front().copied().unwrap_or(0xff),
            1 => self.status(),
            _ => 0xff
        }
    }

    fn run_microsecond(&mut self) {
        // 0.0441 samples a microsecond at 44.1 kHz
        self.fraction += S2P_RATE as u32;
        while self.fraction >= 1_000_000 {
            self.fraction -= 1_000_000;
            self.position += 1;
            if self.position >= self.left.len() {
                self.synth.render(&mut self.left, &mut self.right);
                self.position = 0;
            }
        }
    }
}

impl ExpansionDevice for S2p {
    fn tick(&mut self, port: &mut ExpansionPort) {
        if let Some((address, value)) = port.io_write_started() {
            if address & 0xfffc == self.base {
                self.write(address, value);
            }
        }
        // the byte is read once, when the IN begins, and held on the bus for the whole cycle
        match port.io_read() {
            Some(address) if address & 0xfffc == self.base => {
                if port.io_read_started() || self.answering.is_none() {
                    self.answering = Some(self.read(address, true));
                }
                port.drive_data(self.answering.unwrap());
            }
            _ => self.answering = None
        }
        self.ticks += 1;
        if self.ticks == TICKS_PER_US {
            self.ticks = 0;
            self.run_microsecond();
        }
    }

    fn reset(&mut self) {
        self.synth.reset();
        self.parser = MidiParser::default();
        self.answers.clear();
    }

    fn sound(&self) -> [f32; 2] {
        match (self.left.get(self.position), self.right.get(self.position)) {
            (Some(&left), Some(&right)) => [left, right],
            _ => [0.0; 2]
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_bases_of_the_jumper() {
        assert_eq!(parse_base("&FEBC"), Ok(0xfebc));
        assert_eq!(parse_base("0xfeac"), Ok(0xfeac));
        assert_eq!(parse_base("FE84"), Ok(0xfe84));
        assert!(parse_base("&FE80").is_err());
    }

    #[test]
    fn midi_bytes_make_messages() {
        let mut parser = MidiParser::default();
        assert_eq!(parser.push(0x90), None);
        assert_eq!(parser.push(0x3c), None);
        assert_eq!(parser.push(0x7f), Some((0x90, 0x3c, 0x7f)));
        // running status
        assert_eq!(parser.push(0x40), None);
        assert_eq!(parser.push(0x00), Some((0x90, 0x40, 0x00)));
        // a program change has one data byte; real-time bytes go through it unseen
        assert_eq!(parser.push(0xc1), None);
        assert_eq!(parser.push(0xf8), None);
        assert_eq!(parser.push(0x05), Some((0xc1, 0x05, 0)));
        // SysEx skipped
        for byte in [0xf0, 0x41, 0x10, 0x42, 0xf7] {
            assert_eq!(parser.push(byte), None);
        }
    }

    /// The tone of the OPL3 tutorial (shipbrook.net's "Programming the AdLib/Sound Blaster FM
    /// Music Chips"): channel 1, a 440 Hz sine-ish note.
    pub(crate) const TONE: [(u8, u8); 9] = [
        (0x20, 0x01),
        (0x40, 0x10),
        (0x60, 0xf0),
        (0x80, 0x77),
        (0xa0, 0x98),
        (0x23, 0x01),
        (0x63, 0xf0),
        (0x83, 0x77),
        (0xb0, 0x31)
    ];

    fn loudness(card: &mut Opl3Lpt, microseconds: usize) -> f32 {
        (0..microseconds)
            .map(|_| {
                card.run_microsecond();
                card.sound()[0].abs()
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_opl3_plays_a_tone_then_stops() {
        let mut card = Opl3Lpt::new(OPL3_BASE);
        assert_eq!(loudness(&mut card, 2000), 0.0, "silent at first");
        for (register, value) in TONE {
            card.write(0xfebc, register);
            card.write(0xfebd, value);
        }
        assert!(loudness(&mut card, 20_000) > 0.01, "the note");
        // key off (B0 bit 5), then the release
        card.write(0xfebc, 0xb0);
        card.write(0xfebd, 0x11);
        loudness(&mut card, 500_000);
        assert!(loudness(&mut card, 2000) < 0.001, "released");
    }

    #[test]
    fn the_second_bank_is_selected_at_base_plus_2() {
        let mut card = Opl3Lpt::new(OPL3_BASE);
        card.write(0xfebe, 0x05);
        card.write(0xfebd, 0x01);
        assert_eq!(card.chip.read_register(0x05, OplRegisterFile::Secondary), 0x01);
        assert_eq!(card.chip.read_register(0x05, OplRegisterFile::Primary), 0x00);
    }
}
