//! HFE floppy images (HxC Floppy Emulator) kept as what they are: the MFM bit cells of each track.
//!
//! Version 1 (`HXCPICFE`) and 3 (`HXCHFEV3`, with its opcodes: index, bit rate, skipped bits and
//! random "weak" bits) are read; version 1 is written back. Each track side becomes a [`BitTrack`]:
//! the cells in the order they pass under the head, from the index hole. [`decode_track`] finds the
//! sectors on it (sync marks, ID and data address marks, CRCs) with where they lie; the FDC reads
//! those, so the timing of a track is the one of the actual bits. A sector written by the FDC is
//! encoded back in place ([`BitTrack::write_data_field`]), a formatted track rebuilt
//! ([`BitTrack::formatted`]), and the image can be saved again as HFE.

use std::fmt;

/// The bit cells of one side of a track, LSB first in each byte (as HFE stores them).
#[derive(Clone, PartialEq, Eq)]
pub struct BitTrack {
    pub cells: Vec<u8>,
    /// Cells that read as random each time (HFE v3 RAND), same layout as `cells`.
    pub weak: Vec<u8>,
    /// Number of valid cells.
    pub len: usize
}

impl fmt::Debug for BitTrack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BitTrack({} cells)", self.len)
    }
}

impl BitTrack {
    pub fn new(len: usize) -> Self {
        Self { cells: vec![0; len.div_ceil(8)], weak: vec![0; len.div_ceil(8)], len }
    }

    pub fn cell(&self, i: usize) -> bool {
        let i = i % self.len.max(1);
        self.cells[i / 8] & (1 << (i % 8)) != 0
    }

    fn is_weak(&self, i: usize) -> bool {
        let i = i % self.len.max(1);
        self.weak[i / 8] & (1 << (i % 8)) != 0
    }

    fn set_cell(&mut self, i: usize, value: bool) {
        let i = i % self.len.max(1);
        if value {
            self.cells[i / 8] |= 1 << (i % 8);
        }
        else {
            self.cells[i / 8] &= !(1 << (i % 8));
        }
        self.weak[i / 8] &= !(1 << (i % 8));
    }

    fn push(&mut self, value: bool, weak: bool) {
        if self.len % 8 == 0 {
            self.cells.push(0);
            self.weak.push(0);
        }
        let i = self.len;
        if value {
            self.cells[i / 8] |= 1 << (i % 8);
        }
        if weak {
            self.weak[i / 8] |= 1 << (i % 8);
        }
        self.len += 1;
    }

    /// The 16 cells from `at` as a word, first cell in the top bit (the way MFM marks are written).
    fn word(&self, at: usize) -> u16 {
        (0..16).fold(0, |w, k| (w << 1) | u16::from(self.cell(at + k)))
    }

    /// The data byte of the 16 cells from `at` (the data cells, every other one from the second).
    /// Weak cells read as `noise` gives them.
    fn byte(&self, at: usize, noise: &mut impl FnMut() -> bool) -> u8 {
        (0..8).fold(0, |b, k| {
            let i = at + 2 * k + 1;
            let bit = if self.is_weak(i) { noise() } else { self.cell(i) };
            (b << 1) | u8::from(bit)
        })
    }

    fn any_weak(&self, from: usize, cells: usize) -> bool {
        (from..from + cells).any(|i| self.is_weak(i))
    }

    /// Write `bytes` MFM encoded from cell `at`, the clock of the first one following the data
    /// bit that precedes it on the track.
    fn write_bytes(&mut self, at: usize, bytes: &[u8]) {
        let mut previous = at.checked_sub(1).map(|i| self.cell(i)).unwrap_or(false);
        let mut i = at;
        for &byte in bytes {
            for k in (0..8).rev() {
                let bit = byte & (1 << k) != 0;
                self.set_cell(i, !bit && !previous);
                self.set_cell(i + 1, bit);
                previous = bit;
                i += 2;
            }
        }
    }

    fn write_word(&mut self, at: usize, word: u16) {
        for k in 0..16 {
            self.set_cell(at + k, word & (0x8000 >> k) != 0);
        }
    }

    /// Encode the data field of a sector again: its address mark (`deleted`: &F8, else &FB),
    /// `data` and their CRC. `mark_at` is the first cell of the mark byte (after the A1 syncs).
    pub fn write_data_field(&mut self, mark_at: usize, deleted: bool, data: &[u8]) {
        let mark = if deleted { 0xf8 } else { 0xfb };
        let mut field = vec![mark];
        field.extend_from_slice(data);
        let crc = crc16(&[[0xa1, 0xa1, 0xa1].as_slice(), &field].concat());
        field.extend_from_slice(&crc.to_be_bytes());
        self.write_bytes(mark_at, &field);
    }

    /// A track of `len` cells (or as long as needed) formatted the way the uPD765's Format Track
    /// writes it: gap 4a, sync, index mark, gap 1, then for each sector its ID (`ids`: C, H, R, N)
    /// and a data field of `filler` (N `n`), separated by gaps of `gap3`; 4E up to the end.
    pub fn formatted(len: usize, ids: &[[u8; 4]], n: u8, gap3: u8, filler: u8) -> Self {
        let mut bytes = Vec::new();
        let mut marks = Vec::new(); // (byte index, 16-bit mark word)
        let gap = |bytes: &mut Vec<u8>, count: usize, value: u8| bytes.extend(std::iter::repeat_n(value, count));
        gap(&mut bytes, 80, 0x4e);
        gap(&mut bytes, 12, 0x00);
        for _ in 0..3 {
            marks.push((bytes.len(), 0x5224));
            bytes.push(0xc2);
        }
        bytes.push(0xfc);
        gap(&mut bytes, 50, 0x4e);
        for id in ids {
            gap(&mut bytes, 12, 0x00);
            for _ in 0..3 {
                marks.push((bytes.len(), 0x4489));
                bytes.push(0xa1);
            }
            let mut field = vec![0xfe];
            field.extend_from_slice(id);
            let crc = crc16(&[[0xa1, 0xa1, 0xa1].as_slice(), &field].concat());
            field.extend_from_slice(&crc.to_be_bytes());
            bytes.extend(field);
            gap(&mut bytes, 22, 0x4e);
            gap(&mut bytes, 12, 0x00);
            for _ in 0..3 {
                marks.push((bytes.len(), 0x4489));
                bytes.push(0xa1);
            }
            let mut field = vec![0xfb];
            field.extend(std::iter::repeat_n(filler, crate::disc_image::size_from_code(n)));
            let crc = crc16(&[[0xa1, 0xa1, 0xa1].as_slice(), &field].concat());
            field.extend_from_slice(&crc.to_be_bytes());
            bytes.extend(field);
            gap(&mut bytes, usize::from(gap3), 0x4e);
        }
        let len = len.max(bytes.len() * 16);
        let missing = (len / 16).saturating_sub(bytes.len());
        gap(&mut bytes, missing, 0x4e);
        let mut track = BitTrack::new(len);
        track.write_bytes(0, &bytes);
        for (at, word) in marks {
            track.write_word(at * 16, word);
        }
        track
    }
}

/// CRC-CCITT as the uPD765 computes it (polynomial &1021, preset &FFFF), over the bytes from the
/// first A1 sync of a field.
pub fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0xffff, |mut crc: u16, &byte| {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
        crc
    })
}

/// A sector found on a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundSector {
    pub id: [u8; 4],
    pub id_crc_ok: bool,
    /// First cell of the ID's C byte and of its first data byte; None when no data field follows
    /// the ID closely enough (missing data address mark).
    pub id_at: usize,
    pub data_at: Option<usize>,
    pub deleted: bool,
    /// The data, as many copies as weak cells make different (1 otherwise).
    pub copies: Vec<Vec<u8>>,
    pub data_crc_ok: bool
}

/// The cells between an ID and the start of the data field's sync the uPD765 accepts (gap 2 is 22
/// bytes, the chip gives up after about 43).
const DATA_SEARCH_CELLS: usize = 43 * 16;

/// The sectors on `track`, in the order they pass under the head from the index hole. A field
/// running past the end of the track goes on from its start (the track is a circle).
pub fn decode_track(track: &BitTrack) -> Vec<FoundSector> {
    let len = track.len;
    if len < 64 {
        return Vec::new();
    }
    let mut rng = 0x1234_5678u32;
    let mut noise = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng & 1 != 0
    };
    // the A1 syncs: positions of the cell after three of them in a row
    let mut marks = Vec::new();
    let mut i = 0;
    while i < len {
        if track.word(i) == 0x4489 && track.word(i + 16) == 0x4489 && track.word(i + 32) == 0x4489 {
            marks.push(i + 48);
            i += 48;
        }
        else {
            i += 1;
        }
    }
    let mut sectors = Vec::new();
    for (k, &at) in marks.iter().enumerate() {
        if track.byte(at, &mut noise) != 0xfe {
            continue;
        }
        let id_at = at + 16;
        let id: [u8; 4] = std::array::from_fn(|j| track.byte(id_at + 16 * j, &mut noise));
        let crc = u16::from_be_bytes([track.byte(id_at + 64, &mut noise), track.byte(id_at + 80, &mut noise)]);
        let id_crc_ok = crc == crc16(&[0xa1, 0xa1, 0xa1, 0xfe, id[0], id[1], id[2], id[3]]);
        // the data field: the next sync after the ID, if close enough
        let id_end = id_at + 96;
        let data = marks
            .iter()
            .chain(marks.iter().map(|m| m + len).collect::<Vec<_>>().iter())
            .skip(k + 1)
            .find(|&&m| m > id_end)
            .copied()
            .filter(|&m| m - 48 <= id_end + DATA_SEARCH_CELLS)
            .filter(|&m| matches!(track.byte(m, &mut noise), 0xfb | 0xf8));
        let mut found = FoundSector {
            id,
            id_crc_ok,
            id_at: id_at % len,
            data_at: None,
            deleted: false,
            copies: vec![Vec::new()],
            data_crc_ok: false
        };
        if let Some(mark_at) = data {
            let data_at = mark_at + 16;
            let size = crate::disc_image::size_from_code(id[3] & 7);
            let weak = track.any_weak(data_at, size * 16);
            let read = |noise: &mut dyn FnMut() -> bool| -> Vec<u8> {
                let mut n = || noise();
                (0..size).map(|j| track.byte(data_at + 16 * j, &mut n)).collect()
            };
            let first = read(&mut noise);
            let mark = track.byte(mark_at, &mut noise);
            let crc_at = data_at + 16 * size;
            let crc = u16::from_be_bytes([track.byte(crc_at, &mut noise), track.byte(crc_at + 16, &mut noise)]);
            found.data_crc_ok = crc == crc16(&[[0xa1, 0xa1, 0xa1, mark].as_slice(), &first].concat());
            found.deleted = mark == 0xf8;
            found.data_at = Some(data_at % len);
            found.copies = vec![first];
            if weak {
                for _ in 0..2 {
                    let again = read(&mut noise);
                    found.copies.push(again);
                }
            }
        }
        sectors.push(found);
    }
    sectors
}

/// An HFE image: its tracks (`tracks[track][side]`) and what the header says of the drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hfe {
    pub tracks: Vec<[BitTrack; 2]>,
    pub sides: u8,
    /// Kbit/s of data (250 for a CPC: a cell lasts 2 us).
    pub bit_rate: u16,
    pub rpm: u16,
    pub interface_mode: u8,
    pub write_allowed: bool
}

const HFE_V1: &[u8; 8] = b"HXCPICFE";
const HFE_V3: &[u8; 8] = b"HXCHFEV3";
/// The interface mode of an Amstrad CPC drive (HxC's `CPC_DD_FLOPPYMODE`).
const CPC_DD_FLOPPYMODE: u8 = 0x07;

impl Hfe {
    pub fn is_hfe(bytes: &[u8]) -> bool {
        bytes.starts_with(HFE_V1) || bytes.starts_with(HFE_V3)
    }

    /// Microseconds a cell lasts.
    pub fn cell_us(&self) -> f64 {
        500.0 / f64::from(self.bit_rate.max(1))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let v3 = bytes.starts_with(HFE_V3);
        if !v3 && !bytes.starts_with(HFE_V1) {
            return Err("not an HFE image".to_owned());
        }
        if bytes.len() < 512 {
            return Err("HFE image shorter than its header".to_owned());
        }
        let nb_tracks = usize::from(bytes[9]);
        let sides = bytes[10];
        if !(1..=2).contains(&sides) {
            return Err(format!("HFE image with {sides} sides"));
        }
        let encoding = bytes[11];
        if encoding != 0 && encoding != 0xff {
            // 0: ISO/IBM MFM (the CPC's); others are FM, Amiga MFM, ...
            return Err(format!("HFE track encoding {encoding} (only MFM is supported)"));
        }
        let bit_rate = u16::from_le_bytes([bytes[12], bytes[13]]);
        let rpm = u16::from_le_bytes([bytes[14], bytes[15]]);
        let interface_mode = bytes[16];
        let lut = usize::from(u16::from_le_bytes([bytes[18], bytes[19]])) * 512;
        let write_allowed = bytes[20] != 0;
        let mut tracks = Vec::with_capacity(nb_tracks);
        for t in 0..nb_tracks {
            let entry = bytes.get(lut + 4 * t..lut + 4 * t + 4).ok_or("HFE track table out of the file")?;
            let offset = usize::from(u16::from_le_bytes([entry[0], entry[1]])) * 512;
            let len = usize::from(u16::from_le_bytes([entry[2], entry[3]]));
            let data = bytes.get(offset..offset + len.div_ceil(512) * 512).ok_or(format!("HFE track {t} out of the file"))?;
            // 512 byte blocks: 256 bytes of side 0, then 256 of side 1
            let mut raw = [Vec::with_capacity(len / 2), Vec::with_capacity(len / 2)];
            for (b, block) in data.chunks(512).enumerate() {
                let take = (len / 2).saturating_sub(b * 256).min(256);
                raw[0].extend_from_slice(&block[..take.min(block.len())]);
                raw[1].extend_from_slice(&block[256..(256 + take).min(block.len())]);
            }
            let side = |raw: &[u8]| if v3 { decode_v3(raw) } else { from_bytes(raw) };
            tracks.push([side(&raw[0]), side(&raw[1])]);
        }
        Ok(Self { tracks, sides, bit_rate, rpm, interface_mode, write_allowed })
    }

    /// The image as HFE version 1 (weak cells written as they last read: 0).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![0xffu8; 512];
        out[..8].copy_from_slice(HFE_V1);
        out[8] = 0;
        out[9] = self.tracks.len() as u8;
        out[10] = self.sides;
        out[11] = 0; // ISO/IBM MFM
        out[12..14].copy_from_slice(&self.bit_rate.to_le_bytes());
        out[14..16].copy_from_slice(&self.rpm.to_le_bytes());
        out[16] = if self.interface_mode == 0 { CPC_DD_FLOPPYMODE } else { self.interface_mode };
        out[17] = 1;
        out[18..20].copy_from_slice(&1u16.to_le_bytes()); // the track table in block 1
        out[20] = if self.write_allowed { 0xff } else { 0 };
        out[21] = 0xff; // single step
        out[22..26].copy_from_slice(&[0xff, 0, 0xff, 0]);
        let table_blocks = (self.tracks.len() * 4).div_ceil(512).max(1);
        let mut table = vec![0xffu8; table_blocks * 512];
        let mut data = Vec::new();
        let first_block = 1 + table_blocks;
        for (t, sides) in self.tracks.iter().enumerate() {
            let side_bytes = sides.iter().map(|s| s.cells.len()).max().unwrap_or(0);
            let len = side_bytes * 2;
            let offset = first_block + data.len() / 512;
            table[4 * t..4 * t + 2].copy_from_slice(&(offset as u16).to_le_bytes());
            table[4 * t + 2..4 * t + 4].copy_from_slice(&(len as u16).to_le_bytes());
            let blocks = side_bytes.div_ceil(256);
            for b in 0..blocks {
                for side in sides {
                    let chunk: Vec<u8> = (0..256).map(|k| side.cells.get(b * 256 + k).copied().unwrap_or(0x55)).collect();
                    data.extend_from_slice(&chunk);
                }
            }
        }
        out.extend(table);
        out.extend(data);
        out
    }
}

fn from_bytes(raw: &[u8]) -> BitTrack {
    BitTrack { cells: raw.to_vec(), weak: vec![0; raw.len()], len: raw.len() * 8 }
}

/// HFE v3: bytes whose bit-reversed value is &F0-&F4 are opcodes (NOP, SETINDEX, SETBITRATE + a
/// byte, SKIPBITS + a count then a byte, RAND: a byte of random cells).
fn decode_v3(raw: &[u8]) -> BitTrack {
    let mut track = BitTrack { cells: Vec::new(), weak: Vec::new(), len: 0 };
    let mut i = 0;
    let push_byte = |track: &mut BitTrack, byte: u8, from_bit: usize, weak: bool| {
        for k in from_bit..8 {
            track.push(byte & (1 << k) != 0, weak);
        }
    };
    while i < raw.len() {
        match raw[i].reverse_bits() {
            0xf0 | 0xf1 => i += 1,
            0xf2 => i += 2,
            0xf3 => {
                let skip = usize::from(raw.get(i + 1).copied().unwrap_or(0)).min(8);
                if let Some(&byte) = raw.get(i + 2) {
                    push_byte(&mut track, byte, skip, false);
                }
                i += 3;
            },
            0xf4 => {
                push_byte(&mut track, 0, 0, true);
                i += 1;
            },
            _ => {
                push_byte(&mut track, raw[i], 0, false);
                i += 1;
            }
        }
    }
    track
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn crc_after_the_three_syncs_is_the_well_known_cdb4() {
        // every MFM field starts with A1 A1 A1: the CRC is &CDB4 after them
        assert_eq!(crc16(&[0xa1, 0xa1, 0xa1]), 0xcdb4);
    }

    #[test]
    fn a_formatted_track_decodes_to_its_sectors() {
        let ids: Vec<[u8; 4]> = [0xc1, 0xc6, 0xc2, 0xc7, 0xc3, 0xc8, 0xc4, 0xc9, 0xc5].iter().map(|&r| [3, 0, r, 2]).collect();
        let track = BitTrack::formatted(100_000, &ids, 2, 0x52, 0xe5);
        let found = decode_track(&track);
        assert_eq!(found.len(), 9);
        for (sector, id) in found.iter().zip(&ids) {
            assert_eq!(&sector.id, id);
            assert!(sector.id_crc_ok && sector.data_crc_ok && !sector.deleted);
            assert_eq!(sector.copies, vec![vec![0xe5; 512]]);
        }
        // in order along the track
        assert!(found.windows(2).all(|w| w[0].id_at < w[1].id_at));
    }

    #[test]
    fn a_data_field_written_again_reads_back_with_a_good_crc() {
        let ids = [[0, 0, 1, 1], [0, 0, 2, 1]];
        let mut track = BitTrack::formatted(100_000, &ids, 1, 0x4e, 0);
        let found = decode_track(&track);
        let data: Vec<u8> = (0..=255).collect();
        track.write_data_field(found[0].data_at.unwrap() - 16, true, &data);
        let found = decode_track(&track);
        assert_eq!(found[0].copies[0], data);
        assert!(found[0].data_crc_ok && found[0].deleted);
        assert_eq!(found[1].copies[0], vec![0; 256], "the next sector untouched");
        assert!(found[1].data_crc_ok);
    }

    #[test]
    fn hfe_v1_round_trip_keeps_the_cells() {
        let ids = [[0, 0, 0xc1, 2]];
        let hfe = Hfe {
            tracks: vec![[BitTrack::formatted(100_000, &ids, 2, 0x4e, 0xe5), BitTrack::new(100_000)]],
            sides: 1,
            bit_rate: 250,
            rpm: 300,
            interface_mode: CPC_DD_FLOPPYMODE,
            write_allowed: true
        };
        let back = Hfe::from_bytes(&hfe.to_bytes()).unwrap();
        assert_eq!(decode_track(&back.tracks[0][0]), decode_track(&hfe.tracks[0][0]));
        assert_eq!(back.bit_rate, 250);
    }

    #[test]
    fn hfe_v3_random_bytes_make_weak_cells() {
        // a NOP, a data byte, a RAND opcode (both written bit-reversed, as HFE does)
        let raw = [0xf0u8.reverse_bits(), 0x55, 0xf4u8.reverse_bits()];
        let track = decode_v3(&raw);
        assert_eq!(track.len, 16);
        assert!(!track.is_weak(0) && track.is_weak(8));
    }
}
