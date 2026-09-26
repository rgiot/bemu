//! The disc as the FDC sees it: tracks holding sectors, each with the ID
//! (C/H/R/N), the ST1/ST2 flags recorded on the medium and its data.
//!
//! `cpclib-disc` keeps most of that private (a sector's C/H/N and stored
//! ST1/ST2 are not reachable, and a track cannot be built from outside),
//! and only reads *extended* DSK images. The FDC needs all of it - to compare
//! full sector IDs, to report CRC errors and deleted marks the way the
//! recorded flags say, to format - so it works on this model, loaded from
//! standard or extended DSK bytes and written back as extended DSK bytes.

use cpclib_disc::AnyDisc;

const DISC_INFO_LEN: usize = 0x100;
const TRACK_INFO_LEN: usize = 0x100;
const STANDARD_SIGNATURE: &[u8] = b"MV - CPC";
const EXTENDED_SIGNATURE: &[u8] = b"EXTENDED CPC DSK File\r\nDisk-Info\r\n";
const TRACK_SIGNATURE: &[u8] = b"Track-Info\r\n";
const CREATOR: &[u8; 14] = b"cpc-emulator  ";

/// Offsets inside the disc information block.
const NB_TRACKS: usize = 0x30;
const NB_HEADS: usize = 0x31;
const TRACK_SIZE: usize = 0x32;
const TRACK_SIZE_TABLE: usize = 0x34;
/// Offsets inside a track information block.
const TRACK_NUMBER: usize = 0x10;
const TRACK_HEAD: usize = 0x11;
const TRACK_SIZE_CODE: usize = 0x14;
const TRACK_NB_SECTORS: usize = 0x15;
const TRACK_GAP3: usize = 0x16;
const TRACK_FILLER: usize = 0x17;
const TRACK_SECTOR_INFOS: usize = 0x18;
const SECTOR_INFO_LEN: usize = 8;
/// A track information block holds at most this many sector descriptions.
const MAX_SECTORS_PER_TRACK: usize = (TRACK_INFO_LEN - TRACK_SECTOR_INFOS) / SECTOR_INFO_LEN;

/// ST1/ST2 flags a sector can carry from the medium.
pub mod flag {
    /// ST1: CRC error in the ID or data field.
    pub const ST1_DATA_ERROR: u8 = 1 << 5;
    /// ST2: CRC error in the data field.
    pub const ST2_DATA_ERROR_IN_DATA: u8 = 1 << 5;
    /// ST2: the sector carries a deleted data address mark.
    pub const ST2_CONTROL_MARK: u8 = 1 << 6;
}

/// `128 << n`: the size the N field of an ID announces.
pub const fn size_from_code(n: u8) -> usize {
    128usize << if n > 7 { 7 } else { n }
}

/// Smallest N whose announced size holds `len` bytes.
pub fn size_code_from_len(len: usize) -> u8 {
    let mut n = 0u8;
    while n < 7 && size_from_code(n) < len {
        n += 1;
    }
    n
}

/// One sector: its ID as recorded, the flags of the medium and the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sector {
    pub c: u8,
    pub h: u8,
    pub r: u8,
    pub n: u8,
    /// ST1 as recorded (data/ID CRC error, ...)
    pub st1: u8,
    /// ST2 as recorded (data CRC error, deleted mark, ...)
    pub st2: u8,
    /// Raw data. Longer than the nominal size when the image stores several
    /// copies of a sector (weak/fuzzy sectors): see [`Sector::copies`].
    pub data: Vec<u8>
}

impl Sector {
    /// A sector of the size its N announces, filled with `filler`.
    pub fn new(c: u8, h: u8, r: u8, n: u8, filler: u8) -> Self {
        Self {
            c,
            h,
            r,
            n,
            st1: 0,
            st2: 0,
            data: vec![filler; size_from_code(n)]
        }
    }

    /// Size the ID announces.
    pub fn nominal_len(&self) -> usize {
        size_from_code(self.n)
    }

    /// Number of complete copies of the data stored (1 for a normal sector).
    pub fn copies(&self) -> usize {
        (self.data.len() / self.nominal_len().max(1)).max(1)
    }

    /// The nominal-size window of copy `copy` (modulo the copies present),
    /// padded with `filler` if the image stored less than the ID announces.
    pub fn read_copy(&self, copy: usize, filler: u8) -> Vec<u8> {
        let len = self.nominal_len();
        let start = (copy % self.copies()) * len;
        let mut bytes: Vec<u8> = self.data.iter().skip(start).take(len).copied().collect();
        bytes.resize(len, filler);
        bytes
    }

    pub fn is_deleted(&self) -> bool {
        self.st2 & flag::ST2_CONTROL_MARK != 0
    }
}

/// A formatted track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub track: u8,
    pub head: u8,
    pub gap3: u8,
    pub filler: u8,
    /// In the order they pass under the head after the index hole.
    pub sectors: Vec<Sector>
}

/// A whole disc, indexed by (head, track).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscImage {
    nb_tracks: u8,
    nb_heads: u8,
    /// `tracks[track * nb_heads + head]`; `None` = unformatted
    tracks: Vec<Option<Track>>
}

impl DiscImage {
    /// An unformatted disc.
    pub fn new(nb_tracks: u8, nb_heads: u8) -> Self {
        let nb_heads = nb_heads.clamp(1, 2);
        Self {
            nb_tracks,
            nb_heads,
            tracks: vec![None; usize::from(nb_tracks) * usize::from(nb_heads)]
        }
    }

    pub fn nb_tracks(&self) -> u8 {
        self.nb_tracks
    }

    pub fn nb_heads(&self) -> u8 {
        self.nb_heads
    }

    pub fn is_double_sided(&self) -> bool {
        self.nb_heads > 1
    }

    fn slot(&self, head: u8, track: u8) -> Option<usize> {
        (track < self.nb_tracks && head < self.nb_heads)
            .then(|| usize::from(track) * usize::from(self.nb_heads) + usize::from(head))
    }

    pub fn track(&self, head: u8, track: u8) -> Option<&Track> {
        self.slot(head, track)
            .and_then(|slot| self.tracks[slot].as_ref())
    }

    pub fn track_mut(&mut self, head: u8, track: u8) -> Option<&mut Track> {
        let slot = self.slot(head, track)?;
        self.tracks[slot].as_mut()
    }

    /// Replace (format) a track. Fails when the position is outside the disc.
    pub fn set_track(&mut self, head: u8, track: u8, value: Track) -> Result<(), String> {
        let slot = self
            .slot(head, track)
            .ok_or_else(|| format!("track {track} head {head} is outside the disc"))?;
        self.tracks[slot] = Some(value);
        Ok(())
    }

    /// Load a standard or extended DSK image.
    pub fn from_bytes(buffer: &[u8]) -> Result<Self, String> {
        if buffer.len() < DISC_INFO_LEN {
            return Err(format!(
                "DSK file is only {} bytes, too short for its header",
                buffer.len()
            ));
        }
        let extended = starts_with_ignore_case(buffer, b"EXTENDED");
        if !extended && !starts_with_ignore_case(buffer, STANDARD_SIGNATURE) {
            return Err("Not a DSK file (unknown signature)".to_owned());
        }

        let nb_tracks = buffer[NB_TRACKS];
        let nb_heads = buffer[NB_HEADS];
        if nb_heads != 1 && nb_heads != 2 {
            return Err(format!("Unsupported number of heads: {nb_heads}"));
        }
        let nb_slots = usize::from(nb_tracks) * usize::from(nb_heads);
        let mut image = Self::new(nb_tracks, nb_heads);

        let standard_track_size =
            usize::from(u16::from_le_bytes([buffer[TRACK_SIZE], buffer[TRACK_SIZE + 1]]));
        if !extended && (standard_track_size < TRACK_INFO_LEN || standard_track_size % 256 != 0) {
            return Err(format!("Invalid standard DSK track size {standard_track_size}"));
        }
        if extended && TRACK_SIZE_TABLE + nb_slots > DISC_INFO_LEN {
            return Err(format!(
                "{nb_tracks} tracks x {nb_heads} heads do not fit the disc information block"
            ));
        }

        let mut offset = DISC_INFO_LEN;
        for slot in 0..nb_slots {
            let size = if extended {
                usize::from(buffer[TRACK_SIZE_TABLE + slot]) * 256
            }
            else {
                standard_track_size
            };
            if size == 0 {
                continue; // unformatted
            }
            let block = buffer
                .get(offset..offset + size)
                .ok_or_else(|| format!("DSK file ends inside track slot {slot}"))?;
            image.tracks[slot] = Some(parse_track(block, extended)?);
            offset += size;
        }
        Ok(image)
    }

    /// Serialise as an extended DSK image.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![0u8; DISC_INFO_LEN];
        out[..EXTENDED_SIGNATURE.len()].copy_from_slice(EXTENDED_SIGNATURE);
        out[0x22..0x30].copy_from_slice(CREATOR);
        out[NB_TRACKS] = self.nb_tracks;
        out[NB_HEADS] = self.nb_heads;

        for (slot, track) in self.tracks.iter().enumerate() {
            let Some(track) = track
            else {
                continue;
            };
            let start = out.len();
            write_track(track, &mut out);
            out[TRACK_SIZE_TABLE + slot] = ((out.len() - start) / 256) as u8;
        }
        out
    }
}

impl TryFrom<&AnyDisc> for DiscImage {
    type Error = String;

    /// Convert a `cpclib-disc` disc by way of its own DSK serialisation.
    fn try_from(disc: &AnyDisc) -> Result<Self, String> {
        match disc {
            AnyDisc::Edsk(dsk) => {
                let mut bytes = Vec::new();
                dsk.to_buffer(&mut bytes);
                Self::from_bytes(&bytes)
            },
            #[cfg(feature = "hfe")]
            AnyDisc::Hfe(_) => Err("HFE images are not supported by the FDC yet".to_owned())
        }
    }
}

/// Load a standard or extended DSK image.
pub fn load_dsk(buffer: &[u8]) -> Result<DiscImage, String> {
    DiscImage::from_bytes(buffer)
}

fn starts_with_ignore_case(buffer: &[u8], prefix: &[u8]) -> bool {
    buffer
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

fn parse_track(block: &[u8], extended: bool) -> Result<Track, String> {
    if block.len() < TRACK_INFO_LEN || !starts_with_ignore_case(block, b"Track-Info") {
        return Err("Track information block expected".to_owned());
    }
    let nb_sectors = usize::from(block[TRACK_NB_SECTORS]);
    if nb_sectors > MAX_SECTORS_PER_TRACK {
        return Err(format!(
            "Track declares {nb_sectors} sectors, at most {MAX_SECTORS_PER_TRACK} fit its information block"
        ));
    }

    let mut sectors = Vec::with_capacity(nb_sectors);
    let mut data_at = TRACK_INFO_LEN;
    for index in 0..nb_sectors {
        let info = &block[TRACK_SECTOR_INFOS + index * SECTOR_INFO_LEN..][..SECTOR_INFO_LEN];
        let n = info[3];
        // A standard image gives every sector its nominal size; an extended
        // one records the real length (0 = nominal by convention).
        let recorded = usize::from(u16::from_le_bytes([info[6], info[7]]));
        let len = if !extended || recorded == 0 {
            size_from_code(n)
        }
        else {
            recorded
        };
        let data = block
            .get(data_at..data_at + len)
            .ok_or_else(|| format!("Track data ends inside sector {index}"))?
            .to_vec();
        data_at += len;
        sectors.push(Sector {
            c: info[0],
            h: info[1],
            r: info[2],
            n,
            st1: info[4],
            st2: info[5],
            data
        });
    }

    Ok(Track {
        track: block[TRACK_NUMBER],
        head: block[TRACK_HEAD],
        gap3: block[TRACK_GAP3],
        filler: block[TRACK_FILLER],
        sectors
    })
}

fn write_track(track: &Track, out: &mut Vec<u8>) {
    let start = out.len();
    out.resize(start + TRACK_INFO_LEN, 0);
    out[start..start + TRACK_SIGNATURE.len()].copy_from_slice(TRACK_SIGNATURE);
    out[start + TRACK_NUMBER] = track.track;
    out[start + TRACK_HEAD] = track.head;
    out[start + TRACK_SIZE_CODE] = track.sectors.first().map_or(2, |sector| sector.n);
    out[start + TRACK_NB_SECTORS] = track.sectors.len().min(MAX_SECTORS_PER_TRACK) as u8;
    out[start + TRACK_GAP3] = track.gap3;
    out[start + TRACK_FILLER] = track.filler;

    for (index, sector) in track.sectors.iter().take(MAX_SECTORS_PER_TRACK).enumerate() {
        let info = start + TRACK_SECTOR_INFOS + index * SECTOR_INFO_LEN;
        out[info..info + 6].copy_from_slice(&[
            sector.c, sector.h, sector.r, sector.n, sector.st1, sector.st2
        ]);
        out[info + 6..info + 8].copy_from_slice(&(sector.data.len() as u16).to_le_bytes());
    }
    for sector in track.sectors.iter().take(MAX_SECTORS_PER_TRACK) {
        out.extend_from_slice(&sector.data);
    }
    while (out.len() - start) % 256 != 0 {
        out.push(0);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// One track, three 512 byte sectors, in the standard layout (sector
    /// length field left at 0, as most writers do).
    fn standard_image() -> Vec<u8> {
        let track_size = TRACK_INFO_LEN + 3 * 512;
        let mut image = vec![0u8; DISC_INFO_LEN + track_size];
        image[..34].copy_from_slice(b"MV - CPCEMU Disk-File\r\nDisk-Info\r\n");
        image[NB_TRACKS] = 1;
        image[NB_HEADS] = 1;
        image[TRACK_SIZE..TRACK_SIZE + 2].copy_from_slice(&(track_size as u16).to_le_bytes());

        let track = DISC_INFO_LEN;
        image[track..track + 12].copy_from_slice(b"Track-Info\r\n");
        image[track + TRACK_SIZE_CODE] = 2;
        image[track + TRACK_NB_SECTORS] = 3;
        for (i, id) in [0xc1u8, 0xc2, 0xc3].into_iter().enumerate() {
            let info = track + TRACK_SECTOR_INFOS + i * SECTOR_INFO_LEN;
            image[info..info + 4].copy_from_slice(&[0, 0, id, 2]);
            image[track + TRACK_INFO_LEN + i * 512..][..512].fill(id);
        }
        image
    }

    #[test]
    fn standard_dsk_gets_nominal_sector_sizes() {
        let image = DiscImage::from_bytes(&standard_image()).unwrap();
        let track = image.track(0, 0).unwrap();
        let ids: Vec<u8> = track.sectors.iter().map(|sector| sector.r).collect();
        assert_eq!(ids, [0xc1, 0xc2, 0xc3]);
        for sector in &track.sectors {
            assert_eq!(sector.data, vec![sector.r; 512]);
        }
    }

    #[test]
    fn signature_is_matched_case_insensitively() {
        let mut image = standard_image();
        image[..8].copy_from_slice(b"mv - cpc");
        assert!(DiscImage::from_bytes(&image).is_ok());
    }

    #[test]
    fn truncated_dsk_is_rejected() {
        let mut image = standard_image();
        image.truncate(image.len() - 1);
        assert!(DiscImage::from_bytes(&image).is_err());
        assert!(DiscImage::from_bytes(b"not a dsk at all, and far too short").is_err());
    }

    #[test]
    fn extended_roundtrip_keeps_ids_flags_and_data() {
        let mut image = DiscImage::new(2, 1);
        let mut sector = Sector::new(1, 0, 0x41, 2, 0xe5);
        sector.st1 = flag::ST1_DATA_ERROR;
        sector.st2 = flag::ST2_CONTROL_MARK | flag::ST2_DATA_ERROR_IN_DATA;
        sector.data[7] = 0x99;
        let mut second = Sector::new(1, 0, 0x42, 1, 0x00);
        second.data.extend_from_slice(&[0xaa; 256]); // two copies: a weak sector
        image
            .set_track(0, 1, Track {
                track: 1,
                head: 0,
                gap3: 0x4e,
                filler: 0xe5,
                sectors: vec![sector, second]
            })
            .unwrap();

        let reloaded = DiscImage::from_bytes(&image.to_bytes()).unwrap();
        assert_eq!(reloaded, image);
        assert!(reloaded.track(0, 0).is_none(), "unformatted track stays unformatted");
        assert_eq!(reloaded.track(0, 1).unwrap().sectors[1].copies(), 2);
    }

    #[test]
    fn setting_a_track_outside_the_disc_fails() {
        let mut image = DiscImage::new(2, 1);
        let track = Track {
            track: 5,
            head: 0,
            gap3: 0,
            filler: 0,
            sectors: Vec::new()
        };
        assert!(image.set_track(0, 5, track.clone()).is_err());
        assert!(image.set_track(1, 0, track).is_err());
    }

    #[test]
    fn weak_sector_copies_cycle() {
        let mut sector = Sector::new(0, 0, 1, 0, 0);
        sector.data = [vec![1u8; 128], vec![2u8; 128]].concat();
        assert_eq!(sector.read_copy(0, 0)[0], 1);
        assert_eq!(sector.read_copy(1, 0)[0], 2);
        assert_eq!(sector.read_copy(2, 0)[0], 1);
    }

    #[test]
    fn size_codes() {
        assert_eq!(size_from_code(2), 512);
        assert_eq!(size_code_from_len(512), 2);
        assert_eq!(size_code_from_len(513), 3);
        assert_eq!(size_code_from_len(1), 0);
    }
}
