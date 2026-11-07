//! Cassette tapes: CDT files (the TZX format, used for the CPC under that name) and the deck that
//! plays them.
//!
//! A [`Tape`] is the signal of a CDT file: the levels the deck's head reads and how long each
//! lasts, built once when the file is loaded. The [`TapeDeck`] plays it while its PLAY key is
//! down and the CPC's cassette motor (PPI port C, bit 4) runs; the level goes to PPI port B,
//! bit 7.
//!
//! Durations are kept in the file's own unit, the T-states of a 3.5 MHz ZX Spectrum, doubled so
//! that one CPC microsecond (3.5 of them) is a whole number: 7.

/// One microsecond of the CPC, in [`Segment`] units (half T-states at 3.5 MHz).
const UNITS_PER_US: i64 = 7;
/// One millisecond, in [`Segment`] units.
const UNITS_PER_MS: u32 = 7000;

/// A stretch of the signal: a level held for a while, or where the tape stops by itself (a
/// "stop the tape" pause, the PLAY key pops up).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Segment {
    Level { high: bool, units: u32 },
    Stop,
}

/// The signal of a tape, ready to be played.
#[derive(Clone, Debug, Default)]
pub struct Tape {
    segments: Vec<Segment>,
    /// What was skipped (blocks this player does not know, ...).
    pub notes: Vec<String>,
}

/// Builds the signal: each pulse inverts the level.
#[derive(Default)]
struct Signal {
    segments: Vec<Segment>,
    high: bool,
}

impl Signal {
    fn pulse(&mut self, t_states: u32) {
        self.high = !self.high;
        self.hold(t_states * 2);
    }

    /// The current level, for `units`.
    fn hold(&mut self, units: u32) {
        if units == 0 {
            return;
        }
        match self.segments.last_mut() {
            Some(Segment::Level { high, units: last }) if *high == self.high => {
                *last = last.saturating_add(units)
            }
            _ => self.segments.push(Segment::Level {
                high: self.high,
                units,
            }),
        }
    }

    fn data(&mut self, data: &[u8], used_bits: u8, zero: u32, one: u32) {
        for (i, byte) in data.iter().enumerate() {
            let bits = if i + 1 == data.len() {
                used_bits.clamp(1, 8)
            } else {
                8
            };
            for bit in 0..bits {
                let length = if byte & (0x80 >> bit) != 0 { one } else { zero };
                self.pulse(length);
                self.pulse(length);
            }
        }
    }

    /// A block's pause: the last edge held for a millisecond, then low.
    fn pause(&mut self, ms: u16) {
        if ms == 0 {
            return;
        }
        self.hold(UNITS_PER_MS);
        self.high = false;
        self.hold((u32::from(ms) - 1) * UNITS_PER_MS);
    }
}

/// Reads the blocks of a file.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let slice = self
            .bytes
            .get(self.at..self.at + n)
            .ok_or_else(|| format!("the file ends inside a block (at {:#x})", self.at))?;
        self.at += n;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u24(&mut self) -> Result<usize, String> {
        let b = self.take(3)?;
        Ok(usize::from(b[0]) | usize::from(b[1]) << 8 | usize::from(b[2]) << 16)
    }

    fn u32(&mut self) -> Result<usize, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }
}

impl Tape {
    /// The tape of a CDT (or TZX) file.
    pub fn from_cdt(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 10 || &bytes[..8] != b"ZXTape!\x1a" {
            return Err("not a CDT file (no \"ZXTape!\" header)".to_owned());
        }
        let mut reader = Reader { bytes, at: 10 };
        // the blocks, by where they start (for the loops)
        let mut signal = Signal::default();
        let mut notes = Vec::new();
        let mut loop_start: Option<(usize, u16)> = None;
        while reader.at < bytes.len() {
            let id = reader.u8()?;
            match id {
                0x10 => {
                    let pause = reader.u16()?;
                    let length = usize::from(reader.u16()?);
                    let data = reader.take(length)?;
                    let pilots = if data.first().is_some_and(|flag| *flag < 0x80) {
                        8063
                    } else {
                        3223
                    };
                    for _ in 0..pilots {
                        signal.pulse(2168);
                    }
                    signal.pulse(667);
                    signal.pulse(735);
                    signal.data(data, 8, 855, 1710);
                    signal.pause(pause);
                }
                0x11 => {
                    let pilot = u32::from(reader.u16()?);
                    let sync = (u32::from(reader.u16()?), u32::from(reader.u16()?));
                    let (zero, one) = (u32::from(reader.u16()?), u32::from(reader.u16()?));
                    let pilots = reader.u16()?;
                    let used_bits = reader.u8()?;
                    let pause = reader.u16()?;
                    let length = reader.u24()?;
                    let data = reader.take(length)?;
                    for _ in 0..pilots {
                        signal.pulse(pilot);
                    }
                    signal.pulse(sync.0);
                    signal.pulse(sync.1);
                    signal.data(data, used_bits, zero, one);
                    signal.pause(pause);
                }
                0x12 => {
                    let length = u32::from(reader.u16()?);
                    for _ in 0..reader.u16()? {
                        signal.pulse(length);
                    }
                }
                0x13 => {
                    for _ in 0..reader.u8()? {
                        let length = u32::from(reader.u16()?);
                        signal.pulse(length);
                    }
                }
                0x14 => {
                    let (zero, one) = (u32::from(reader.u16()?), u32::from(reader.u16()?));
                    let used_bits = reader.u8()?;
                    let pause = reader.u16()?;
                    let length = reader.u24()?;
                    signal.data(reader.take(length)?, used_bits, zero, one);
                    signal.pause(pause);
                }
                0x15 => {
                    let per_sample = u32::from(reader.u16()?) * 2;
                    let pause = reader.u16()?;
                    let used_bits = reader.u8()?;
                    let length = reader.u24()?;
                    let data = reader.take(length)?;
                    for (i, byte) in data.iter().enumerate() {
                        let bits = if i + 1 == data.len() {
                            used_bits.clamp(1, 8)
                        } else {
                            8
                        };
                        for bit in 0..bits {
                            signal.high = byte & (0x80 >> bit) != 0;
                            signal.hold(per_sample);
                        }
                    }
                    signal.pause(pause);
                }
                0x20 => match reader.u16()? {
                    0 => signal.segments.push(Segment::Stop),
                    ms => signal.pause(ms),
                },
                0x21 => {
                    let length = usize::from(reader.u8()?);
                    reader.take(length)?;
                }
                0x22 | 0x25 | 0x27 => {
                    if id == 0x25 {
                        if let Some((start, count)) = loop_start.take() {
                            if count > 1 {
                                // play the loop's blocks again, one time fewer
                                loop_start = Some((start, count - 1));
                                reader.at = start;
                            }
                        }
                    }
                }
                0x24 => {
                    let count = reader.u16()?;
                    loop_start = Some((reader.at, count));
                }
                0x23 => {
                    reader.u16()?;
                    notes.push("a jump block (0x23) is ignored".to_owned());
                }
                0x26 => {
                    let calls = usize::from(reader.u16()?);
                    reader.take(calls * 2)?;
                    notes.push("a call sequence (0x26) is ignored".to_owned());
                }
                0x28 | 0x32 => {
                    let length = usize::from(reader.u16()?);
                    reader.take(length)?;
                }
                0x2a => {
                    let length = reader.u32()?;
                    reader.take(length)?;
                }
                0x2b => {
                    let length = reader.u32()?;
                    let data = reader.take(length)?;
                    signal.high = data.first().is_some_and(|level| *level != 0);
                }
                0x30 => {
                    let length = usize::from(reader.u8()?);
                    reader.take(length)?;
                }
                0x31 => {
                    reader.u8()?;
                    let length = usize::from(reader.u8()?);
                    reader.take(length)?;
                }
                0x33 => {
                    let entries = usize::from(reader.u8()?);
                    reader.take(entries * 3)?;
                }
                0x35 => {
                    reader.take(16)?;
                    let length = reader.u32()?;
                    reader.take(length)?;
                }
                0x5a => {
                    reader.take(9)?;
                }
                0x18 | 0x19 => {
                    // CSW recording, generalised data: skipped (rare on the CPC)
                    let length = reader.u32()?;
                    reader.take(length)?;
                    notes.push(format!("a block {id:#04x} is not played"));
                }
                other => {
                    // TZX 1.10+: every unknown block starts with its length
                    let length = reader.u32()?;
                    reader.take(length)?;
                    notes.push(format!("an unknown block {other:#04x} is skipped"));
                }
            }
        }
        Ok(Tape {
            segments: signal.segments,
            notes,
        })
    }

    /// How long the whole tape plays, in seconds.
    pub fn duration_seconds(&self) -> f64 {
        let units: u64 = self
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Level { units, .. } => u64::from(*units),
                Segment::Stop => 0,
            })
            .sum();
        units as f64 / (UNITS_PER_MS as f64 * 1000.0)
    }
}

/// A CDT holding `data` as the CPC firmware writes a binary file on tape (as 2CDT converts
/// one): 2K blocks, each a header record then a data record, at `baud` (1000 or 2000 for the
/// firmware's own speeds), each record after 3 s of blank tape. `RUN"` loads it at `load` and
/// calls `entry`.
pub fn cdt_of_binary(name: &str, load: u16, entry: u16, data: &[u8], baud: u32) -> Vec<u8> {
    // a zero bit is two pulses of `zero` T-states (at 3.5 MHz), a one two of twice that
    let zero = (3_500_000 / (baud * 3)) as u16;
    let one = zero * 2;
    let mut cdt = b"ZXTape!\x1a\x01\x14".to_vec();
    // blank tape first, for the motor's spin-up
    cdt.extend_from_slice(&[0x20, 0xb8, 0x0b]);
    let record = |cdt: &mut Vec<u8>, sync: u8, payload: &[u8]| {
        let mut bytes = vec![sync];
        for segment in payload.chunks(256) {
            let mut segment = segment.to_vec();
            segment.resize(256, 0);
            let crc = !segment.iter().fold(0xffffu16, |mut crc, byte| {
                crc ^= u16::from(*byte) << 8;
                for _ in 0..8 {
                    crc = if crc & 0x8000 != 0 {
                        crc << 1 ^ 0x1021
                    } else {
                        crc << 1
                    };
                }
                crc
            });
            bytes.extend_from_slice(&segment);
            bytes.extend_from_slice(&crc.to_be_bytes());
        }
        bytes.extend_from_slice(&[0xff; 4]); // trailer
        cdt.push(0x11);
        for value in [one, zero, zero, zero, one, 4096] {
            cdt.extend_from_slice(&value.to_le_bytes());
        }
        cdt.push(8);
        // blank tape after each record: the firmware stops the motor after one and spins it up
        // for 2.5 s before reading the next
        cdt.extend_from_slice(&3000u16.to_le_bytes());
        cdt.extend_from_slice(&(bytes.len() as u32).to_le_bytes()[..3]);
        cdt.extend_from_slice(&bytes);
    };
    let blocks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(2048).collect()
    };
    for (i, block) in blocks.iter().enumerate() {
        let mut header = [0u8; 64];
        for (slot, byte) in header.iter_mut().zip(name.bytes().take(16)) {
            *slot = byte.to_ascii_uppercase();
        }
        header[16] = i as u8 + 1;
        header[17] = u8::from(i + 1 == blocks.len()) * 0xff;
        header[18] = 2; // binary
        header[19..21].copy_from_slice(&(block.len() as u16).to_le_bytes());
        header[21..23].copy_from_slice(&(load.wrapping_add(i as u16 * 2048)).to_le_bytes());
        header[23] = u8::from(i == 0) * 0xff;
        header[24..26].copy_from_slice(&(data.len() as u16).to_le_bytes());
        header[26..28].copy_from_slice(&entry.to_le_bytes());
        record(&mut cdt, 0x2c, &header);
        record(&mut cdt, 0x16, block);
    }
    cdt
}

/// The cassette deck: a tape, its position, and whether PLAY is down.
#[derive(Clone, Debug, Default)]
pub struct TapeDeck {
    tape: Tape,
    /// The segment under the head.
    index: usize,
    /// What remains of it, in [`Segment`] units.
    remaining: i64,
    high: bool,
    playing: bool,
}

impl TapeDeck {
    /// `tape` inserted, rewound, PLAY down.
    pub fn new(tape: Tape) -> Self {
        let mut deck = Self {
            tape,
            playing: true,
            ..Self::default()
        };
        deck.rewind();
        deck
    }

    pub fn tape(&self) -> &Tape {
        &self.tape
    }

    /// PLAY down (`true`) or STOP.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Whether PLAY is down (a stop block or the tape's end pops it up).
    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn rewind(&mut self) {
        self.index = 0;
        self.remaining = 0;
        self.high = false;
        self.enter_segment();
    }

    /// Where the head is, in seconds of tape from the start.
    pub fn position_seconds(&self) -> f64 {
        let before: u64 = self.tape.segments[..self.index.min(self.tape.segments.len())]
            .iter()
            .map(|segment| match segment {
                Segment::Level { units, .. } => u64::from(*units),
                Segment::Stop => 0,
            })
            .sum();
        let current = match self.tape.segments.get(self.index) {
            Some(Segment::Level { units, .. }) => i64::from(*units) - self.remaining,
            _ => 0,
        };
        (before as i64 + current) as f64 / (f64::from(UNITS_PER_MS) * 1000.0)
    }

    /// Whether the head is past the tape's end.
    pub fn at_end(&self) -> bool {
        self.index >= self.tape.segments.len()
    }

    /// The level the head reads.
    pub fn level(&self) -> bool {
        self.high
    }

    /// One microsecond with the motor `motor`: the tape moves if PLAY is down too.
    #[inline]
    pub fn advance_us(&mut self, motor: bool) {
        if !(motor && self.playing) {
            return;
        }
        self.remaining -= UNITS_PER_US;
        while self.remaining <= 0 && self.playing {
            self.index += 1;
            self.enter_segment();
        }
    }

    /// Take the segment under the head.
    fn enter_segment(&mut self) {
        match self.tape.segments.get(self.index) {
            Some(Segment::Level { high, units }) => {
                self.high = *high;
                self.remaining += i64::from(*units);
            }
            Some(Segment::Stop) => {
                // PLAY pops up; the next PLAY goes on after the stop
                self.playing = false;
                self.remaining = 0;
            }
            None => {
                self.playing = false;
                self.remaining = 0;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn cdt(blocks: &[u8]) -> Vec<u8> {
        let mut bytes = b"ZXTape!\x1a\x01\x14".to_vec();
        bytes.extend_from_slice(blocks);
        bytes
    }

    #[test]
    fn a_turbo_block_is_its_pulses() {
        // pilot 1000 x2, syncs 300/400, zero 500, one 1000, one byte 0b1000_0000 (1 bit used)
        let mut block = vec![0x11];
        for value in [1000u16, 300, 400, 500, 1000, 2] {
            block.extend_from_slice(&value.to_le_bytes());
        }
        block.push(1); // used bits
        block.extend_from_slice(&0u16.to_le_bytes()); // pause
        block.extend_from_slice(&[1, 0, 0, 0x80]);
        let tape = Tape::from_cdt(&cdt(&block)).unwrap();
        let levels: Vec<_> = tape
            .segments
            .iter()
            .map(|s| match s {
                Segment::Level { high, units } => (*high, *units / 2),
                Segment::Stop => panic!(),
            })
            .collect();
        assert_eq!(
            levels,
            vec![
                (true, 1000),
                (false, 1000),
                (true, 300),
                (false, 400),
                (true, 1000),
                (false, 1000)
            ]
        );
    }

    #[test]
    fn the_deck_moves_only_with_the_motor_and_stops_at_a_stop_block() {
        // a 35000 T-state tone (10 ms), stop, another
        let mut blocks = vec![0x12];
        blocks.extend_from_slice(&35000u16.to_le_bytes());
        blocks.extend_from_slice(&1u16.to_le_bytes());
        blocks.extend_from_slice(&[0x20, 0, 0]);
        blocks.extend_from_slice(&[0x12]);
        blocks.extend_from_slice(&35000u16.to_le_bytes());
        blocks.extend_from_slice(&1u16.to_le_bytes());
        let mut deck = TapeDeck::new(Tape::from_cdt(&cdt(&blocks)).unwrap());
        assert!(deck.level());
        for _ in 0..20_000 {
            deck.advance_us(false);
        }
        assert_eq!(deck.position_seconds(), 0.0);
        for _ in 0..10_000 {
            deck.advance_us(true);
        }
        assert!(!deck.playing(), "stopped by the stop block");
        deck.set_playing(true);
        deck.advance_us(true);
        assert!(!deck.level(), "the second tone's level");
        assert!(
            (deck.position_seconds() - 0.010001).abs() < 1e-6,
            "{}",
            deck.position_seconds()
        );
        for _ in 0..10_000 {
            deck.advance_us(true);
        }
        assert!(deck.at_end() && !deck.playing());
    }

    #[test]
    fn loops_repeat_their_blocks() {
        let mut blocks = vec![0x24, 3, 0, 0x13, 1];
        blocks.extend_from_slice(&100u16.to_le_bytes());
        blocks.push(0x25);
        let tape = Tape::from_cdt(&cdt(&blocks)).unwrap();
        assert_eq!(tape.segments.len(), 3);
    }

    #[test]
    fn not_a_cdt() {
        assert!(Tape::from_cdt(b"MV - SNA").is_err());
    }

    /// A 6128 (`|TAPE`) loads and runs a binary from a CDT as the firmware wrote it.
    #[test]
    fn the_firmware_loads_a_tape() {
        use crate::csl_runner::{CslRunner, MemFiles, PngFiles};
        // LD A,&42 : LD (&4100),A : JR $
        let code = [0x3e, 0x42, 0x32, 0x00, 0x41, 0x18, 0xfe];
        let mut files = MemFiles::default();
        files.insert(
            "test.cdt",
            cdt_of_binary("test", 0x4000, 0x4000, &code, 2000),
        );
        let mut runner =
            CslRunner::with_io(Box::new(files), Box::new(PngFiles), std::env::temp_dir());
        let script = "csl_version 1.0\nreset\nwait 3000000\ntape_insert 'test.cdt'\n\
                      key_output '|TAPE\\(RET)RUN\"\\(RET)'\nwait 2000000\ntape_play\nkey_output ' '\nwait 16000000\n";
        let report = runner.run_text(script, ".").unwrap();
        assert!(report.ignored.is_empty(), "{:?}", report.ignored);
        assert_eq!(
            runner.motherboard().read_byte(0x4100),
            0x42,
            "the program ran"
        );
        assert_eq!(runner.motherboard().cpu().pc(), 0x4005);
    }
}
