//! HFE floppy images kept as what they are, the MFM bit cells of each track, read and written by
//! the HxC floppy library (the `hxcfe` crate, a wrapper of HxC's own C library).
//!
//! The library loads the image (any HFE version), gives the cells of each track and finds the
//! sectors on them (IDs, data marks, CRCs, where they lie): the FDC reads those, at those places.
//! A sector the FDC writes is written through the library when the disc is saved (it encodes the
//! field and its CRC again), as HFE version 3. The emulator only encodes cells itself to format a
//! track ([`BitTrack::formatted`], the uPD765's layout) and to give a sector a deleted data mark.

use std::fmt;

/// The bit cells of one side of a track, LSB first in each byte.
#[derive(Clone, PartialEq, Eq)]
pub struct BitTrack {
    pub cells: Vec<u8>,
    /// Cells that read as random each time (weak bits), same layout as `cells`.
    pub weak: Vec<u8>,
    /// Number of valid cells.
    pub len: usize,
}

impl fmt::Debug for BitTrack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BitTrack({} cells)", self.len)
    }
}

impl BitTrack {
    pub fn new(len: usize) -> Self {
        Self {
            cells: vec![0; len.div_ceil(8)],
            weak: vec![0; len.div_ceil(8)],
            len,
        }
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
        } else {
            self.cells[i / 8] &= !(1 << (i % 8));
        }
        self.weak[i / 8] &= !(1 << (i % 8));
    }

    /// The data byte of the 16 cells from `at` (every other cell from the second); weak cells read
    /// as `noise` gives them.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn byte(&self, at: usize, noise: &mut impl FnMut() -> bool) -> u8 {
        (0..8).fold(0, |b, k| {
            let i = at + 2 * k + 1;
            let bit = if self.is_weak(i) {
                noise()
            } else {
                self.cell(i)
            };
            (b << 1) | u8::from(bit)
        })
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn any_weak(&self, from: usize, cells: usize) -> bool {
        (from..from + cells).any(|i| self.is_weak(i))
    }

    /// Write `bytes` MFM encoded from cell `at`, the clock of the first following the data bit
    /// before it on the track.
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
        let mut field = vec![if deleted { 0xf8 } else { 0xfb }];
        field.extend_from_slice(data);
        let crc = crc16(&[[0xa1, 0xa1, 0xa1].as_slice(), &field].concat());
        field.extend_from_slice(&crc.to_be_bytes());
        self.write_bytes(mark_at, &field);
    }

    /// A track of `len` cells formatted the way the uPD765's Format Track writes it: gap 4a, sync,
    /// index mark, gap 1, then for each sector its ID (`ids`: C, H, R, N) and a data field of
    /// `filler` (N `n`), separated by gaps of `gap3`; 4E up to the end (cut if it does not fit).
    pub fn formatted(len: usize, ids: &[[u8; 4]], n: u8, gap3: u8, filler: u8) -> Self {
        let mut bytes = Vec::new();
        let mut marks = Vec::new(); // (byte index, 16-bit mark word)
        let gap = |bytes: &mut Vec<u8>, count: usize, value: u8| {
            bytes.extend(std::iter::repeat_n(value, count))
        };
        let sync_and_mark =
            |bytes: &mut Vec<u8>, marks: &mut Vec<(usize, u16)>, sync: u8, word: u16| {
                for _ in 0..3 {
                    marks.push((bytes.len(), word));
                    bytes.push(sync);
                }
            };
        gap(&mut bytes, 80, 0x4e);
        gap(&mut bytes, 12, 0x00);
        sync_and_mark(&mut bytes, &mut marks, 0xc2, 0x5224);
        bytes.push(0xfc);
        gap(&mut bytes, 50, 0x4e);
        for id in ids {
            for (mark, payload) in [
                (0xfe, id.to_vec()),
                (0xfb, vec![filler; crate::disc_image::size_from_code(n)]),
            ] {
                gap(&mut bytes, 12, 0x00);
                sync_and_mark(&mut bytes, &mut marks, 0xa1, 0x4489);
                let mut field = vec![mark];
                field.extend(payload);
                let crc = crc16(&[[0xa1, 0xa1, 0xa1].as_slice(), &field].concat());
                field.extend_from_slice(&crc.to_be_bytes());
                bytes.extend(field);
                gap(
                    &mut bytes,
                    if mark == 0xfe { 22 } else { usize::from(gap3) },
                    0x4e,
                );
            }
        }
        let missing = (len / 16).saturating_sub(bytes.len());
        gap(&mut bytes, missing, 0x4e);
        bytes.truncate(len.div_ceil(16));
        let mut track = BitTrack::new(len);
        track.write_bytes(0, &bytes);
        for (at, word) in marks {
            if at * 16 + 16 <= len {
                track.write_word(at * 16, word);
            }
        }
        track
    }

    /// The cells as the library stores them (MSB first).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn to_library(&self) -> (Vec<u8>, Vec<u8>) {
        (
            self.cells.iter().map(|b| b.reverse_bits()).collect(),
            self.weak.iter().map(|b| b.reverse_bits()).collect(),
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn from_library(data: &[u8], flaky: &[u8], len: usize) -> Self {
        let bytes = len.div_ceil(8);
        let mut weak: Vec<u8> = flaky.iter().map(|b| b.reverse_bits()).collect();
        weak.resize(bytes, 0);
        let mut cells: Vec<u8> = data.iter().map(|b| b.reverse_bits()).collect();
        cells.resize(bytes, 0);
        Self { cells, weak, len }
    }
}

/// CRC-CCITT as the uPD765 computes it (polynomial &1021, preset &FFFF), from the first A1 sync.
pub fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0xffff, |mut crc: u16, &byte| {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
        crc
    })
}

/// A sector the library found on a track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundSector {
    pub id: [u8; 4],
    pub id_crc_ok: bool,
    /// First cell of the ID's C byte, and of the first data byte (None without a data field).
    pub id_at: usize,
    pub data_at: Option<usize>,
    pub deleted: bool,
    /// The data, as many copies as weak cells make different (1 otherwise).
    pub copies: Vec<Vec<u8>>,
    pub data_crc_ok: bool,
}

/// A sector write waiting to go to the image through the library (see [`Bitstream::save`]).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingWrite {
    track: usize,
    side: usize,
    r: u8,
    data: Vec<u8>,
}

/// The medium of a disc loaded from an HFE image: its cells, and what changed since it was loaded
/// (the source file is what the library loads again to save).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitstream {
    source: Vec<u8>,
    pub tracks: Vec<[BitTrack; 2]>,
    pub sides: u8,
    /// Bits per second of the data (250 000 on a CPC: a cell lasts 2 us).
    pub bitrate: u32,
    /// Tracks whose cells the emulator wrote itself (formatted, deleted marks): they replace the
    /// source's when saving.
    rebuilt: Vec<(usize, usize)>,
    writes: Vec<PendingWrite>,
}

impl Bitstream {
    /// Microseconds a cell lasts.
    pub fn cell_us(&self) -> f64 {
        500_000.0 / f64::from(self.bitrate.max(1))
    }

    pub fn is_hfe(bytes: &[u8]) -> bool {
        bytes.starts_with(b"HXCPICFE") || bytes.starts_with(b"HXCHFEV3")
    }

    /// Record that the FDC wrote sector `r` (its ID's R) of a track: the library writes it when
    /// saving. A deleted data mark is encoded here, in the cells (`mark_at`: its first cell).
    pub fn sector_written(
        &mut self,
        track: usize,
        side: usize,
        r: u8,
        mark_at: usize,
        deleted: bool,
        data: &[u8],
    ) {
        if deleted {
            self.tracks[track][side].write_data_field(mark_at, true, data);
            self.rebuild(track, side);
        } else {
            self.writes
                .retain(|w| !(w.track == track && w.side == side && w.r == r));
            self.writes.push(PendingWrite {
                track,
                side,
                r,
                data: data.to_vec(),
            });
        }
    }

    /// The cells of a track were written by the emulator: they replace the source's when saving.
    pub fn rebuild(&mut self, track: usize, side: usize) {
        if !self.rebuilt.contains(&(track, side)) {
            self.rebuilt.push((track, side));
        }
        self.writes
            .retain(|w| !(w.track == track && w.side == side));
    }
}

/// The tests that use the library themselves hold this while they do.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) use library::lock as library_lock;

#[cfg(not(target_arch = "wasm32"))]
mod library {
    use super::*;
    use hxcfe::{HeadId, Hxcfe, Img, SectorAccess, SectorId, TrackEncoding, TrackId};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    /// libhxcfe keeps global state: its images are used one call at a time, from whatever thread
    /// (the tests run in parallel, and got images and sectors mixed up).
    static LIBRARY: Mutex<()> = Mutex::new(());

    pub fn lock() -> MutexGuard<'static, ()> {
        LIBRARY.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Load `bytes` in the library, under a unique temporary name (it loads through a file).
    fn load(bytes: &[u8]) -> Result<Img, String> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "amsrust_{}_{}.hfe",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        );
        Hxcfe::get()
            .load_from_buffer(bytes, &name)
            .map_err(|e| format!("HFE: {e}"))
    }

    /// The sectors the library finds on a side of a track (`cells`: the same track, for the
    /// copies of the weak ones), in the order they pass under the head.
    fn sectors(img: &Img, track: usize, side: usize, cells: &BitTrack) -> Vec<FoundSector> {
        let Some(access) = SectorAccess::new(img) else {
            return Vec::new();
        };
        let Some(all) = access.all_track_sectors(
            HeadId::new(side as i32),
            TrackId::new(track as i32),
            TrackEncoding::IsoibmMfm,
        ) else {
            return Vec::new();
        };
        let mut rng = 0x9e37_79b9u32;
        let mut noise = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng & 1 != 0
        };
        let mut found = Vec::new();
        for i in 0..all.nb_sectors() {
            let s = all.sector_config(i);
            // the indexes are the first cell of the A1 syncs: the field's bytes start 4 bytes on
            let id_at = (s.start_sector_index().max(0) as usize + 64) % cells.len.max(1);
            let mark = s.data_mark();
            let data_at =
                mark.map(|_| (s.start_data_index().max(0) as usize + 64) % cells.len.max(1));
            let mut copies = vec![s.read().to_vec()];
            if let Some(at) = data_at {
                let size = copies[0].len();
                if cells.any_weak(at, size * 16) {
                    for _ in 0..2 {
                        copies.push(
                            (0..size)
                                .map(|j| cells.byte(at + 16 * j, &mut noise))
                                .collect(),
                        );
                    }
                }
            }
            found.push(FoundSector {
                id: [
                    s.cylinder() as u8,
                    s.head().get() as u8,
                    s.sector_id().get() as u8,
                    s.size_id(),
                ],
                id_crc_ok: s.header_crc_ok(),
                id_at,
                data_at,
                deleted: mark == Some(0xf8),
                copies,
                data_crc_ok: mark.is_some() && s.data_crc_ok(),
            });
        }
        found.sort_by_key(|f| f.id_at);
        found
    }

    /// The library's image of `bitstream`: its source with the tracks the emulator rebuilt.
    fn current(bitstream: &Bitstream) -> Result<Img, String> {
        let mut img = load(&bitstream.source)?;
        for &(track, side) in &bitstream.rebuilt {
            let (data, flaky) = bitstream.tracks[track][side].to_library();
            img.set_track_bits(track as i32, side as i32, &data, Some(&flaky))?;
        }
        Ok(img)
    }

    impl Bitstream {
        /// An HFE image (any version) loaded by the library, with the sectors of each side of
        /// each track (`found[track][side]`).
        pub fn load(bytes: &[u8]) -> Result<(Self, Vec<[Vec<FoundSector>; 2]>), String> {
            let _library = lock();
            let img = load(bytes)?;
            let nb_tracks = img.nb_tracks_per_head().max(0) as usize;
            let sides = img.nb_sides().clamp(1, 2) as u8;
            let mut tracks = Vec::with_capacity(nb_tracks);
            let mut found = Vec::with_capacity(nb_tracks);
            let mut bitrate = 250_000;
            for track in 0..nb_tracks {
                let mut cells = [BitTrack::new(0), BitTrack::new(0)];
                let mut on_track = [Vec::new(), Vec::new()];
                for side in 0..usize::from(sides) {
                    if let Some(bits) = img.track_bits(track as i32, side as i32) {
                        if bits.bitrate > 0 {
                            bitrate = bits.bitrate;
                        }
                        cells[side] = BitTrack::from_library(&bits.data, &bits.flaky, bits.len);
                        on_track[side] = sectors(&img, track, side, &cells[side]);
                    }
                }
                tracks.push(cells);
                found.push(on_track);
            }
            let bitstream = Self {
                source: bytes.to_vec(),
                tracks,
                sides,
                bitrate: bitrate as u32,
                rebuilt: Vec::new(),
                writes: Vec::new(),
            };
            Ok((bitstream, found))
        }

        /// The sectors the library finds on a track as it is now (after a format).
        pub fn sectors(&self, track: usize, side: usize) -> Result<Vec<FoundSector>, String> {
            let _library = lock();
            let img = current(self)?;
            Ok(sectors(&img, track, side, &self.tracks[track][side]))
        }

        /// The image as HFE version 3, written by the library: the source with the rebuilt tracks
        /// and the sectors the FDC wrote. It becomes the new source.
        pub fn save(&mut self) -> Result<Vec<u8>, String> {
            let _library = lock();
            let img = current(self)?;
            {
                let access = SectorAccess::new(&img).ok_or("the library's sector access")?;
                for w in &self.writes {
                    let mut sector = access
                        .search_sector(
                            HeadId::new(w.side as i32),
                            TrackId::new(w.track as i32),
                            SectorId::new(i32::from(w.r)),
                            TrackEncoding::IsoibmMfm,
                        )
                        .ok_or_else(|| {
                            format!(
                                "sector {:02X} of track {} side {} not found to write it",
                                w.r, w.track, w.side
                            )
                        })?;
                    let mut data = w.data.clone();
                    data.resize(sector.len().max(0) as usize, 0);
                    sector.write(TrackEncoding::IsoibmMfm, &data);
                }
            }
            let bytes = img.save_to_buffer(hxcfe::ImageFormat::HxcHfev3)?;
            // what the library wrote is the medium from now on
            let sides = usize::from(self.sides);
            for (track, cells) in self.tracks.iter_mut().enumerate() {
                for (side, cells) in cells.iter_mut().enumerate().take(sides) {
                    if let Some(bits) = img.track_bits(track as i32, side as i32) {
                        *cells = BitTrack::from_library(&bits.data, &bits.flaky, bits.len);
                    }
                }
            }
            self.source = bytes.clone();
            self.rebuilt.clear();
            self.writes.clear();
            Ok(bytes)
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Bitstream {
    pub fn load(_bytes: &[u8]) -> Result<(Self, Vec<[Vec<FoundSector>; 2]>), String> {
        Err("HFE images need the desktop version (the HxC floppy library)".to_owned())
    }

    pub fn sectors(&self, _track: usize, _side: usize) -> Result<Vec<FoundSector>, String> {
        Err("HFE images need the desktop version".to_owned())
    }

    pub fn save(&mut self) -> Result<Vec<u8>, String> {
        Err("HFE images need the desktop version".to_owned())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn crc_after_the_three_syncs_is_the_well_known_cdb4() {
        assert_eq!(crc16(&[0xa1, 0xa1, 0xa1]), 0xcdb4);
    }

    #[test]
    fn cells_go_to_the_library_msb_first_and_back() {
        let track = BitTrack::formatted(100_000, &[[0, 0, 0xc1, 2]], 2, 0x4e, 0xe5);
        let (data, flaky) = track.to_library();
        assert_eq!(BitTrack::from_library(&data, &flaky, track.len), track);
        // the first cell is the top bit of the library's first byte
        assert_eq!(data[0] >> 7 == 1, track.cell(0));
    }
}
