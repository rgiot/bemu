//! Loading disc images for the [`crate::fdc::Fdc`].
//!
//! `cpclib-disc` only reads *extended* DSK images (standard DSK support is a
//! known TODO there), yet standard images are everywhere (e.g. the ones
//! written by CPC2DSK). A standard image is an extended one with two
//! differences, so it is upgraded on load: the per-track size is one global
//! value instead of a table, and every sector is exactly `128 << N` bytes
//! long instead of carrying its own length.

use cpclib_disc::edsk::ExtendedDsk;
use cpclib_disc::AnyDisc;

const DISC_INFO_LEN: usize = 0x100;
const TRACK_INFO_LEN: usize = 0x100;
const STANDARD_SIGNATURE: &[u8] = b"MV - CPC";
const EXTENDED_SIGNATURE: &[u8] = b"EXTENDED CPC DSK File\r\nDisk-Info\r\n";
/// Offsets inside the disc information block.
const NB_TRACKS: usize = 0x30;
const NB_HEADS: usize = 0x31;
const TRACK_SIZE: usize = 0x32;
const TRACK_SIZE_TABLE: usize = 0x34;
/// Offsets inside a track information block.
const TRACK_NB_SECTORS: usize = 0x15;
const TRACK_SECTOR_INFOS: usize = 0x18;
const SECTOR_INFO_LEN: usize = 8;
const SECTOR_INFO_SIZE_CODE: usize = 3;
const SECTOR_INFO_DATA_LEN: usize = 6;

/// Load a standard or extended DSK image.
pub fn load_dsk(buffer: &[u8]) -> Result<AnyDisc, String> {
    let extended;
    let buffer = if is_standard(buffer) {
        extended = standard_to_extended(buffer)?;
        &extended
    }
    else {
        buffer
    };
    ExtendedDsk::from_buffer(buffer).map(AnyDisc::Edsk)
}

fn is_standard(buffer: &[u8]) -> bool {
    buffer
        .get(..STANDARD_SIGNATURE.len())
        .is_some_and(|signature| signature.eq_ignore_ascii_case(STANDARD_SIGNATURE))
}

/// Rewrite a standard DSK image as the equivalent extended one.
fn standard_to_extended(standard: &[u8]) -> Result<Vec<u8>, String> {
    if standard.len() < DISC_INFO_LEN {
        return Err(format!("DSK file is only {} bytes, too short for its header", standard.len()));
    }
    let nb_tracks = usize::from(standard[NB_TRACKS]);
    let nb_heads = usize::from(standard[NB_HEADS]);
    let track_size = usize::from(u16::from_le_bytes([standard[TRACK_SIZE], standard[TRACK_SIZE + 1]]));
    if track_size < TRACK_INFO_LEN || track_size % 256 != 0 {
        return Err(format!("Invalid standard DSK track size {track_size}"));
    }
    let nb_track_slots = nb_tracks * nb_heads;
    if TRACK_SIZE_TABLE + nb_track_slots > DISC_INFO_LEN {
        return Err(format!("{nb_tracks} tracks x {nb_heads} heads do not fit the disc information block"));
    }
    let wanted_len = DISC_INFO_LEN + nb_track_slots * track_size;
    if standard.len() < wanted_len {
        return Err(format!("DSK file is {} bytes, {wanted_len} needed for its {nb_track_slots} tracks", standard.len()));
    }

    let mut extended = vec![0u8; DISC_INFO_LEN];
    extended[..EXTENDED_SIGNATURE.len()].copy_from_slice(EXTENDED_SIGNATURE);
    // creator (0x22..0x30) is kept as is
    extended[0x22..NB_TRACKS].copy_from_slice(&standard[0x22..NB_TRACKS]);
    extended[NB_TRACKS] = standard[NB_TRACKS];
    extended[NB_HEADS] = standard[NB_HEADS];
    extended[TRACK_SIZE_TABLE..TRACK_SIZE_TABLE + nb_track_slots].fill((track_size / 256) as u8);
    extended.extend_from_slice(&standard[DISC_INFO_LEN..wanted_len]);

    // every sector of a standard image is 128 << N bytes long
    for slot in 0..nb_track_slots {
        let track = DISC_INFO_LEN + slot * track_size;
        let nb_sectors = usize::from(extended[track + TRACK_NB_SECTORS]);
        for sector in 0..nb_sectors {
            let info = track + TRACK_SECTOR_INFOS + sector * SECTOR_INFO_LEN;
            if info + SECTOR_INFO_LEN > track + TRACK_INFO_LEN {
                return Err(format!("Track {slot} declares {nb_sectors} sectors, too many for its information block"));
            }
            let len = 128u32 << extended[info + SECTOR_INFO_SIZE_CODE].min(8);
            extended[info + SECTOR_INFO_DATA_LEN..info + SECTOR_INFO_DATA_LEN + 2]
                .copy_from_slice(&(len as u16).to_le_bytes());
        }
    }
    Ok(extended)
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
        image[track + 0x14] = 2; // N
        image[track + TRACK_NB_SECTORS] = 3;
        for (i, id) in [0xc1u8, 0xc2, 0xc3].into_iter().enumerate() {
            let info = track + TRACK_SECTOR_INFOS + i * SECTOR_INFO_LEN;
            image[info..info + 4].copy_from_slice(&[0, 0, id, 2]);
            image[track + TRACK_INFO_LEN + i * 512..][..512].fill(id);
        }
        image
    }

    #[test]
    fn standard_dsk_is_upgraded_and_readable() {
        use cpclib_disc::disc::Disc;
        use cpclib_disc::edsk::Head;

        let AnyDisc::Edsk(dsk) = load_dsk(&standard_image()).unwrap()
        else {
            panic!("expected an extended dsk")
        };
        for id in [0xc1u8, 0xc2, 0xc3] {
            let bytes = dsk.sector_read_bytes(Head::A, 0, id).expect("sector present");
            assert_eq!(bytes, vec![id; 512], "sector &{id:02X}");
        }
    }

    #[test]
    fn signature_is_matched_case_insensitively() {
        let mut image = standard_image();
        image[..8].copy_from_slice(b"mv - cpc");
        assert!(is_standard(&image));
    }

    #[test]
    fn truncated_standard_dsk_is_rejected() {
        let mut image = standard_image();
        image.truncate(image.len() - 1);
        assert!(load_dsk(&image).is_err());
    }
}
