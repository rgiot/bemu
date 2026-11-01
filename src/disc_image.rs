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

    /// The AMSDOS directory of side 0/track 0 (the layout every CPC disc
    /// uses, `|CPM` or not): 2048 bytes, the first 4 512-byte sectors in
    /// logical (R) order regardless of the physical interleave, decoded as
    /// 64 32-byte entries. Deleted entries (user number &E5) are left out;
    /// entries whose extent number isn't 0 (a file bigger than 16 blocks
    /// continues onto more entries) are merged into their first one, since
    /// that's what "the file exists" means to a catalog listing.
    pub fn amsdos_catalog(&self) -> Vec<AmsdosEntry> {
        let Some(track) = self.track(0, 0) else { return Vec::new() };
        let mut sectors: Vec<&Sector> = track.sectors.iter().collect();
        sectors.sort_by_key(|s| s.r);

        let mut directory = Vec::with_capacity(2048);
        for sector in sectors {
            directory.extend_from_slice(&sector.data);
            if directory.len() >= 2048 {
                break;
            }
        }
        directory.resize(2048, 0xe5);

        let mut entries: Vec<AmsdosEntry> = Vec::new();
        for raw in directory.chunks_exact(32) {
            let user = raw[0];
            if user == 0xe5 || user > 15 {
                continue; // empty slot
            }
            let extent = raw[12];
            let name: String = raw[1..9].iter().map(|&b| (b & 0x7f) as char).collect();
            let extension: String = raw[9..12].iter().map(|&b| (b & 0x7f) as char).collect();
            let hidden = raw[10] & 0x80 != 0;
            let entry = AmsdosEntry {
                user,
                name: name.trim_end().to_owned(),
                extension: extension.trim_end().to_owned(),
                hidden,
                records: raw[15]
            };
            if extent == 0 {
                entries.push(entry);
            }
            // extent != 0: a continuation of the entry already pushed for
            // this name - nothing more to record for a catalog listing.
        }
        entries
    }
}

/// One AMSDOS directory entry, as `CAT` would list it (`user:name.extension`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmsdosEntry {
    pub user: u8,
    pub name: String,
    pub extension: String,
    /// The "system" attribute (extension bit 7 of its 2nd character):
    /// `CAT` itself does not list these.
    pub hidden: bool,
    /// Records used in the entry's first extent (at most 128, one per 128
    /// bytes): not the file's real length for anything bigger than 16KB,
    /// but enough to rank entries by roughly how big they are.
    pub records: u8
}

impl AmsdosEntry {
    /// `NAME.EXT` the way `RUN"..."` expects it.
    pub fn filename(&self) -> String {
        if self.extension.is_empty() {
            self.name.clone()
        }
        else {
            format!("{}.{}", self.name, self.extension)
        }
    }

    /// Whether this looks like an actual file rather than a "CATART"
    /// directory entry - many demos replace their catalog with entries
    /// whose name/extension bytes are cursor-control codes and colour
    /// changes rather than text, so that `CAT` itself draws a picture (the
    /// firmware's `PRINT` prints those bytes as VDU commands, not
    /// characters). A real AMSDOS filename never contains one of those
    /// (0x00-0x1F): only space-padding and printable characters.
    pub fn is_plausible_filename(&self) -> bool {
        let printable = |s: &str| s.bytes().all(|b| (0x20..0x7f).contains(&b));
        !self.name.is_empty() && printable(&self.name) && printable(&self.extension)
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
            // cpclib-disc knows HFE images on the desktop only (its `hfe` feature wraps a C library)
            #[cfg(not(target_arch = "wasm32"))]
            AnyDisc::Hfe(_) => Err("HFE images are not supported by the FDC yet".to_owned())
        }
    }
}

/// Load a standard or extended DSK image.
pub fn load_dsk(buffer: &[u8]) -> Result<DiscImage, String> {
    DiscImage::from_bytes(buffer)
}

/// The extensions of the disc images an archive is searched for.
const DISC_EXTENSIONS: [&str; 2] = ["dsk", "hfe"];

fn has_extension(name: &str, extensions: &[&str]) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| extensions.iter().any(|wanted| ext.eq_ignore_ascii_case(wanted)))
}

/// A disc image from the bytes of a file named `name`: a DSK or extended DSK, an HFE image
/// (converted to DSK, desktop only), or a zip archive holding one of them. In an archive, `entry`
/// names the image (its path in the archive, the case ignored); without it the first DSK or HFE
/// file is taken.
pub fn load_disc_bytes(bytes: &[u8], name: &str, entry: Option<&str>) -> Result<DiscImage, String> {
    if bytes.starts_with(b"PK\x03\x04") {
        let (entry_name, data) = zip_entry(bytes, entry).map_err(|e| format!("{name}: {e}"))?;
        return load_disc_bytes(&data, &entry_name, None).map_err(|e| format!("{name}#{entry_name}: {e}"));
    }
    if let Some(entry) = entry {
        return Err(format!("{name} is not a zip archive, it has no {entry}"));
    }
    if bytes.starts_with(b"HXCPICFE") || bytes.starts_with(b"HXCHFEV3") || has_extension(name, &["hfe"]) {
        return hfe_to_dsk(bytes).and_then(|dsk| load_dsk(&dsk));
    }
    load_dsk(bytes)
}

/// The name and content of the archive entry `wanted`, or of its first disc image.
fn zip_entry(bytes: &[u8], wanted: Option<&str>) -> Result<(String, Vec<u8>), String> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let files = || names.iter().filter(|n| !n.ends_with('/'));
    let chosen = match wanted {
        Some(wanted) => files()
            .find(|n| n.as_str() == wanted)
            .or_else(|| files().find(|n| n.eq_ignore_ascii_case(wanted)))
            .ok_or_else(|| format!("no {wanted} in the archive (it holds: {})", files().cloned().collect::<Vec<_>>().join(", ")))?,
        None => files()
            .find(|n| has_extension(n, &DISC_EXTENSIONS))
            .ok_or_else(|| format!("no disc image (.dsk, .hfe) in the archive (it holds: {})", files().cloned().collect::<Vec<_>>().join(", ")))?
    }
    .clone();
    let mut file = archive.by_name(&chosen).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(|e| e.to_string())?;
    Ok((chosen, data))
}

/// An HFE image converted to an extended DSK by the HxC floppy library.
#[cfg(not(target_arch = "wasm32"))]
fn hfe_to_dsk(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    // the library loads through a file of that name in the temporary directory
    let name = format!("amsrust_{}_{}.hfe", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
    let img = hxcfe::Hxcfe::get().load_from_buffer(bytes, &name).map_err(|e| format!("HFE: {e}"))?;
    img.save_to_buffer(hxcfe::ImageFormat::AmstradcpcDsk).map_err(|e| format!("HFE to DSK: {e}"))
}

#[cfg(target_arch = "wasm32")]
fn hfe_to_dsk(_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err("HFE images need the desktop version (the HxC floppy library)".to_owned())
}

/// Split a disc argument into the file and, for a zip archive, the entry after a `#`
/// (`games.zip#side b.dsk`). A `#` in a name that is not an archive's stays in the path.
pub fn split_disc_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.rsplit_once('#') {
        Some((path, entry)) if has_extension(path, &["zip"]) && !entry.is_empty() => (path, Some(entry)),
        _ => (spec, None)
    }
}

/// A disc image file: a DSK, an HFE, `archive.zip` (its first disc image) or `archive.zip#entry`.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_disc_file(spec: &str) -> Result<DiscImage, String> {
    let (path, entry) = split_disc_spec(spec);
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
    load_disc_bytes(&bytes, name, entry).map_err(|e| if e.starts_with(name) { e } else { format!("{name}: {e}") })
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

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn same_sectors(a: &DiscImage, b: &DiscImage) {
        assert_eq!((a.nb_tracks(), a.nb_heads()), (b.nb_tracks(), b.nb_heads()));
        for track in 0..a.nb_tracks() {
            let (ta, tb) = (a.track(0, track).unwrap(), b.track(0, track).unwrap());
            let ids = |t: &Track| t.sectors.iter().map(|s| (s.c, s.h, s.r, s.n, s.data.clone())).collect::<Vec<_>>();
            assert_eq!(ids(ta), ids(tb), "track {track}");
        }
    }

    #[test]
    fn a_zip_gives_its_first_disc_image_or_the_one_named() {
        let rhino = std::fs::read("validation/RHINO.DSK").unwrap();
        let other = std::fs::read("validation/demos/skoh/skoh.dsk").unwrap();
        let zip = zip_of(&[("readme.txt", b"hello"), ("dir/Rhino.dsk", &rhino), ("S&KOH.DSK", &other)]);

        let first = load_disc_bytes(&zip, "pack.zip", None).unwrap();
        same_sectors(&first, &load_dsk(&rhino).unwrap());
        let named = load_disc_bytes(&zip, "pack.zip", Some("s&koh.dsk")).unwrap();
        same_sectors(&named, &load_dsk(&other).unwrap());

        let missing = load_disc_bytes(&zip, "pack.zip", Some("nope.dsk")).unwrap_err();
        assert!(missing.contains("nope.dsk") && missing.contains("readme.txt"), "{missing}");
        assert!(load_disc_bytes(&zip_of(&[("a.txt", b"x")]), "p.zip", None).is_err());
        assert!(load_disc_bytes(&rhino, "RHINO.DSK", Some("x.dsk")).is_err(), "only an archive has entries");
    }

    #[test]
    fn a_hash_names_an_entry_of_a_zip_only() {
        assert_eq!(split_disc_spec("games.zip#side b.dsk"), ("games.zip", Some("side b.dsk")));
        assert_eq!(split_disc_spec("GAMES.ZIP#a.dsk"), ("GAMES.ZIP", Some("a.dsk")));
        assert_eq!(split_disc_spec("games.zip"), ("games.zip", None));
        assert_eq!(split_disc_spec("disc #2.dsk"), ("disc #2.dsk", None));
        assert_eq!(split_disc_spec("games.zip#"), ("games.zip#", None));
    }

    #[test]
    fn an_hfe_image_gives_the_sectors_of_the_disc_it_was_made_from() {
        let rhino = std::fs::read("validation/RHINO.DSK").unwrap();
        let hxc = hxcfe::Hxcfe::get();
        let img = hxc.load_from_buffer(&rhino, "amsrust_test_rhino.dsk").unwrap();
        let hfe = img.save_to_buffer(hxcfe::ImageFormat::HxcHfe).unwrap();
        assert!(hfe.starts_with(b"HXCPICFE"));

        let back = load_disc_bytes(&hfe, "rhino.hfe", None).unwrap();
        same_sectors(&back, &load_dsk(&rhino).unwrap());
        // and from a zip
        let zipped = load_disc_bytes(&zip_of(&[("rhino.hfe", &hfe)]), "rhino.zip", None).unwrap();
        same_sectors(&zipped, &load_dsk(&rhino).unwrap());
    }

    #[test]
    fn amsdos_catalog_reads_rhino_dsk() {
        let bytes = std::fs::read("validation/RHINO.DSK").unwrap();
        let image = load_dsk(&bytes).unwrap();
        let catalog = image.amsdos_catalog();
        assert!(!catalog.is_empty(), "RHINO.DSK must have at least one file");
        assert!(
            catalog.iter().any(|e| e.name == "RHINO"),
            "expected a RHINO entry, got {catalog:?}"
        );
    }

    #[test]
    fn filename_joins_name_and_extension_only_when_there_is_one() {
        let with_ext = amsdos_entry("RHINO", "BIN");
        assert_eq!(with_ext.filename(), "RHINO.BIN");
        let without_ext = amsdos_entry("DISC", "");
        assert_eq!(without_ext.filename(), "DISC");
    }

    fn amsdos_entry(name: &str, extension: &str) -> AmsdosEntry {
        AmsdosEntry { user: 0, name: name.into(), extension: extension.into(), hidden: false, records: 0 }
    }

    #[test]
    fn catart_entries_are_told_apart_from_real_filenames() {
        assert!(amsdos_entry("HBL", "BAS").is_plausible_filename());
        assert!(amsdos_entry("DISC", "").is_plausible_filename());
        // control bytes (cursor moves, colour changes) used to draw a
        // picture when `CAT` prints the directory, not a real filename
        assert!(!amsdos_entry("K\n B.", "").is_plausible_filename());
        assert!(!amsdos_entry("N2026.", "\n\n\n").is_plausible_filename());
        assert!(!amsdos_entry("", "").is_plausible_filename());
    }

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
