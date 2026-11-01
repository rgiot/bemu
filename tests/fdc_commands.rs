//! uPD765A command behaviour beyond the plain Read Data path (see
//! `tests/fdc.rs`): full sector ID matching, Read ID walking the track,
//! recorded error / deleted-mark flags, SK, MT, multi-sector transfers,
//! Read Track, Format Track, the Scan commands, ready-change interrupts,
//! Sense Drive Status. Expected values come from the NEC datasheet
//! (`doc/fdc.upd765ab.pdf`).

use amsrust::disc_image::{flag, DiscImage, Sector, Track};
use amsrust::fdc::{DriveKind, Fdc, FdcDrive};

const RQM: u8 = 1 << 7;
const DIO: u8 = 1 << 6;
const CB: u8 = 1 << 4;

/// Command codes with the MFM flag every CPC driver sets.
const READ_DATA: u8 = 0x46;
const READ_DELETED_DATA: u8 = 0x4c;
const WRITE_DATA: u8 = 0x45;
const WRITE_DELETED_DATA: u8 = 0x49;
const READ_TRACK: u8 = 0x42;
const READ_ID: u8 = 0x4a;
const FORMAT_TRACK: u8 = 0x4d;
const SCAN_EQUAL: u8 = 0x51;
const SCAN_LOW_OR_EQUAL: u8 = 0x59;
const SCAN_HIGH_OR_EQUAL: u8 = 0x5d;
const SK: u8 = 0x20;
const MT: u8 = 0x80;

fn sector(c: u8, h: u8, r: u8, n: u8, fill: u8) -> Sector {
    Sector::new(c, h, r, n, fill)
}

/// Track 0, head 0 (all sectors 256 bytes, filled with their R unless said):
///   R1 R2 R3: ordinary
///   R4: deleted data mark
///   R5: CRC error recorded in the data field
///   R6: two copies of its data (a weak sector)
///   R7: recorded with C = 1 (wrong cylinder for a search on track 0)
///   R8: recorded with C = &FF (bad cylinder)
/// Track 1: unformatted.
fn disc() -> DiscImage {
    let mut deleted = sector(0, 0, 4, 1, 4);
    deleted.st2 = flag::ST2_CONTROL_MARK;
    let mut crc = sector(0, 0, 5, 1, 5);
    crc.st1 = flag::ST1_DATA_ERROR;
    crc.st2 = flag::ST2_DATA_ERROR_IN_DATA;
    let mut weak = sector(0, 0, 6, 1, 0xaa);
    weak.data.extend_from_slice(&[0xbb; 256]);

    let mut image = DiscImage::new(3, 1);
    image
        .set_track(0, 0, Track {
            track: 0,
            head: 0,
            gap3: 0x4e,
            filler: 0xe5,
            sectors: vec![
                sector(0, 0, 1, 1, 1),
                sector(0, 0, 2, 1, 2),
                sector(0, 0, 3, 1, 3),
                deleted,
                crc,
                weak,
                sector(1, 0, 7, 1, 7),
                sector(0xff, 0, 8, 1, 8),
            ]
        })
        .unwrap();
    image
}

fn fdc_with(image: DiscImage) -> Fdc {
    let mut fdc = Fdc::default();
    fdc.insert_image(FdcDrive::A, Some(image));
    fdc.set_motor(true);
    drain_interrupts(&mut fdc);
    fdc
}

fn ready_fdc() -> Fdc {
    fdc_with(disc())
}

fn drain_interrupts(fdc: &mut Fdc) {
    for _ in 0..8 {
        fdc.write_data(0x08);
        if fdc.read_data() == 0x80 {
            return;
        }
        fdc.read_data();
    }
    panic!("interrupts keep coming");
}

fn send(fdc: &mut Fdc, bytes: &[u8]) {
    for &byte in bytes {
        assert!(fdc.read_status() & RQM != 0, "FDC refused byte {byte:#04x}");
        fdc.write_data(byte);
    }
}

fn receive(fdc: &mut Fdc, count: usize) -> Vec<u8> {
    (0..count)
        .map(|_| {
            assert!(fdc.read_status() & RQM != 0, "no byte ready");
            fdc.read_data()
        })
        .collect()
}

/// `[code, unit/head, C, H, R, N, EOT, GPL, DTL]`
fn rw(code: u8, c: u8, h: u8, r: u8, n: u8, eot: u8) -> [u8; 9] {
    [code, 0x00, c, h, r, n, eot, 0x2a, 0xff]
}

/// Run a data-reading command: returns (data, 7 result bytes).
fn read(fdc: &mut Fdc, command: &[u8], data_len: usize) -> (Vec<u8>, Vec<u8>) {
    send(fdc, command);
    let data = receive(fdc, data_len);
    let result = receive(fdc, 7);
    (data, result)
}

/// Run a command that fails before any data: only the 7 result bytes.
fn read_result_only(fdc: &mut Fdc, command: &[u8]) -> Vec<u8> {
    send(fdc, command);
    receive(fdc, 7)
}

// ------------------------------------------------------------------ IDs

#[test]
fn reading_a_sector_returns_its_data_and_id() {
    let mut fdc = ready_fdc();
    let (data, result) = read(&mut fdc, &rw(READ_DATA, 0, 0, 2, 1, 2), 256);
    assert_eq!(data, vec![2; 256]);
    assert_eq!(result, [0x40, 0x80, 0x00, 0, 0, 2, 1], "ends with End of Cylinder");
}

#[test]
fn a_sector_id_must_match_on_c_h_and_n_not_only_r() {
    let mut fdc = ready_fdc();

    // right R, wrong N
    let result = read_result_only(&mut fdc, &rw(READ_DATA, 0, 0, 2, 2, 2));
    assert_eq!(result[0] & 0xc0, 0x40);
    assert_eq!(result[1] & 0x04, 0x04, "ND");

    // right R, wrong H
    let result = read_result_only(&mut fdc, &rw(READ_DATA, 0, 1, 2, 1, 2));
    assert_eq!(result[1] & 0x04, 0x04, "ND");
}

#[test]
fn a_recorded_cylinder_differing_from_the_command_sets_wrong_cylinder() {
    let mut fdc = ready_fdc();
    // R7 is recorded with C=1; we ask for C=0: ND + WC (ST2 D4)
    let result = read_result_only(&mut fdc, &rw(READ_DATA, 0, 0, 7, 1, 7));
    assert_eq!(result[1] & 0x04, 0x04, "ND");
    assert_eq!(result[2] & 0x10, 0x10, "WC");
    assert_eq!(result[2] & 0x02, 0x00, "not BC");
}

#[test]
fn a_recorded_cylinder_of_ff_sets_bad_cylinder() {
    let mut fdc = ready_fdc();
    let result = read_result_only(&mut fdc, &rw(READ_DATA, 0, 0, 8, 1, 8));
    assert_eq!(result[1] & 0x04, 0x04, "ND");
    assert_eq!(result[2] & 0x02, 0x02, "BC");
}

#[test]
fn an_unformatted_track_has_no_address_mark() {
    let mut fdc = ready_fdc();
    // seek to track 1 (unformatted)
    send(&mut fdc, &[0x0f, 0x00, 0x01]);
    drain_interrupts(&mut fdc);

    let result = read_result_only(&mut fdc, &rw(READ_DATA, 1, 0, 1, 1, 1));
    assert_eq!(result[0] & 0xc0, 0x40);
    assert_eq!(result[1] & 0x05, 0x05, "MA and ND");
}

#[test]
fn fm_commands_find_nothing_on_an_mfm_disc() {
    let mut fdc = ready_fdc();
    // 0x06 = Read Data without the MF bit
    let result = read_result_only(&mut fdc, &rw(0x06, 0, 0, 1, 1, 1));
    assert_eq!(result[0] & 0xc0, 0x40);
    assert_eq!(result[1] & 0x05, 0x05);
}

#[test]
fn st0_reports_the_unit_and_head_of_the_command() {
    let mut fdc = ready_fdc();
    // head 1 (HD bit of the unit byte) on a single-sided disc: not found
    let mut command = rw(READ_DATA, 0, 1, 1, 1, 1);
    command[1] = 0x04;
    let result = read_result_only(&mut fdc, &command);
    assert_eq!(result[0] & 0x07, 0x04, "HD bit in ST0");
}

// -------------------------------------------------------------- Read ID

#[test]
fn read_id_walks_the_track_and_wraps() {
    let mut fdc = ready_fdc();
    let mut seen = Vec::new();
    for _ in 0..10 {
        send(&mut fdc, &[READ_ID, 0x00]);
        let result = receive(&mut fdc, 7);
        assert_eq!(result[0] & 0xc0, 0x00);
        seen.push((result[3], result[5]));
    }
    // physical order, then round again
    assert_eq!(
        seen,
        [(0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0, 6), (1, 7), (0xff, 8), (0, 1), (0, 2)]
    );
}

#[test]
fn read_id_on_an_unformatted_track_fails_with_no_address_mark() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &[0x0f, 0x00, 0x01]);
    drain_interrupts(&mut fdc);

    send(&mut fdc, &[READ_ID, 0x00]);
    let result = receive(&mut fdc, 7);
    assert_eq!(result[0] & 0xc0, 0x40);
    assert_eq!(result[1] & 0x05, 0x05);
}

#[test]
fn a_seek_puts_the_head_back_at_the_start_of_the_track() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &[READ_ID, 0x00]);
    receive(&mut fdc, 7);
    send(&mut fdc, &[READ_ID, 0x00]);
    receive(&mut fdc, 7);

    send(&mut fdc, &[0x0f, 0x00, 0x00]);
    drain_interrupts(&mut fdc);
    send(&mut fdc, &[READ_ID, 0x00]);
    assert_eq!(receive(&mut fdc, 7)[5], 1);
}

// ------------------------------------------------ recorded flags / marks

#[test]
fn a_recorded_data_crc_error_terminates_the_read_abnormally() {
    let mut fdc = ready_fdc();
    // ask for sectors 5 to 6: it must stop after sector 5
    let (data, result) = read(&mut fdc, &rw(READ_DATA, 0, 0, 5, 1, 6), 256);
    assert_eq!(data, vec![5; 256], "the data is still transferred");
    assert_eq!(result[0] & 0xc0, 0x40, "IC abnormal");
    assert_eq!(result[1] & 0x20, 0x20, "ST1 DE");
    assert_eq!(result[2] & 0x20, 0x20, "ST2 DD");
    assert_eq!(result[5], 5, "stopped on the sector with the error");
}

#[test]
fn reading_a_deleted_sector_flags_the_control_mark_and_stops() {
    let mut fdc = ready_fdc();
    let (data, result) = read(&mut fdc, &rw(READ_DATA, 0, 0, 4, 1, 6), 256);
    assert_eq!(data, vec![4; 256]);
    assert_eq!(result[2] & 0x40, 0x40, "ST2 CM");
    assert_eq!(result[5], 4, "stopped after the deleted sector");
}

#[test]
fn skip_flag_steps_over_deleted_sectors() {
    let mut fdc = ready_fdc();
    // R3 then (deleted R4 skipped) R5 - the transfer contains R3 and R5 only
    // R5 carries a CRC error so it ends the command: ends on R5
    let (data, result) = read(&mut fdc, &rw(READ_DATA | SK, 0, 0, 3, 1, 5), 512);
    assert_eq!(&data[..256], vec![3; 256].as_slice());
    assert_eq!(&data[256..], vec![5; 256].as_slice());
    assert_eq!(result[2] & 0x40, 0x00, "no control mark: it was skipped");
}

#[test]
fn read_deleted_data_wants_the_deleted_sector() {
    let mut fdc = ready_fdc();
    let (data, result) = read(&mut fdc, &rw(READ_DELETED_DATA, 0, 0, 4, 1, 4), 256);
    assert_eq!(data, vec![4; 256]);
    assert_eq!(result[2] & 0x40, 0x00, "the mark is the expected one: no CM");

    // a normal sector met by Read Deleted Data raises CM
    let (_, result) = read(&mut fdc, &rw(READ_DELETED_DATA, 0, 0, 2, 1, 2), 256);
    assert_eq!(result[2] & 0x40, 0x40, "CM");
}

#[test]
fn write_deleted_data_sets_the_mark_and_write_data_clears_it() {
    let mut fdc = ready_fdc();

    send(&mut fdc, &rw(WRITE_DELETED_DATA, 0, 0, 2, 1, 2));
    send(&mut fdc, &[0x5a; 256]);
    assert_eq!(receive(&mut fdc, 7)[0] & 0xc0, 0x40, "End of Cylinder");

    // now a Read Data sees a deleted sector
    let (data, result) = read(&mut fdc, &rw(READ_DATA, 0, 0, 2, 1, 2), 256);
    assert_eq!(data, vec![0x5a; 256]);
    assert_eq!(result[2] & 0x40, 0x40, "CM");

    // a plain Write Data puts a normal mark back
    send(&mut fdc, &rw(WRITE_DATA, 0, 0, 2, 1, 2));
    send(&mut fdc, &[0x11; 256]);
    receive(&mut fdc, 7);
    let (data, result) = read(&mut fdc, &rw(READ_DATA, 0, 0, 2, 1, 2), 256);
    assert_eq!(data, vec![0x11; 256]);
    assert_eq!(result[2] & 0x40, 0x00);
}

#[test]
fn rewriting_a_sector_with_a_crc_error_repairs_it() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &rw(WRITE_DATA, 0, 0, 5, 1, 5));
    send(&mut fdc, &[0x77; 256]);
    receive(&mut fdc, 7);

    let (data, result) = read(&mut fdc, &rw(READ_DATA, 0, 0, 5, 1, 5), 256);
    assert_eq!(data, vec![0x77; 256]);
    assert_eq!(result[..3], [0x40, 0x80, 0], "End of Cylinder, nothing else");
}

#[test]
fn a_weak_sector_alternates_between_its_copies() {
    let mut fdc = ready_fdc();
    let first = read(&mut fdc, &rw(READ_DATA, 0, 0, 6, 1, 6), 256).0;
    let second = read(&mut fdc, &rw(READ_DATA, 0, 0, 6, 1, 6), 256).0;
    let third = read(&mut fdc, &rw(READ_DATA, 0, 0, 6, 1, 6), 256).0;
    assert_eq!(first, vec![0xaa; 256]);
    assert_eq!(second, vec![0xbb; 256]);
    assert_eq!(third, vec![0xaa; 256]);
}

// -------------------------------------------------- multi-sector, EOT, MT

#[test]
fn a_read_continues_through_the_sectors_up_to_eot() {
    let mut fdc = ready_fdc();
    let (data, result) = read(&mut fdc, &rw(READ_DATA, 0, 0, 1, 1, 3), 3 * 256);
    assert_eq!(&data[..256], vec![1; 256].as_slice());
    assert_eq!(&data[256..512], vec![2; 256].as_slice());
    assert_eq!(&data[512..], vec![3; 256].as_slice());
    assert_eq!(result, [0x40, 0x80, 0x00, 0, 0, 3, 1], "ends with End of Cylinder");
}

#[test]
fn a_multi_sector_read_stops_when_a_sector_is_missing() {
    let mut fdc = ready_fdc();
    // R6 is followed by R7 whose recorded C differs: the search for
    // C=0 R=7 fails
    let (data, result) = read(&mut fdc, &rw(READ_DATA, 0, 0, 6, 1, 7), 256);
    assert_eq!(data.len(), 256);
    assert_eq!(result[0] & 0xc0, 0x40);
    assert_eq!(result[1] & 0x04, 0x04, "ND on the second sector");
    assert_eq!(result[5], 7);
}

fn two_sided() -> DiscImage {
    let mut image = DiscImage::new(1, 2);
    for head in 0..2u8 {
        image
            .set_track(head, 0, Track {
                track: 0,
                head,
                gap3: 0x4e,
                filler: 0xe5,
                sectors: (1..=2).map(|r| sector(0, head, r, 1, 0x10 * head + r)).collect()
            })
            .unwrap();
    }
    image
}

/// A double-sided 3.5" drive A with `image` in it, spinning.
fn fdc_35_with(image: DiscImage) -> Fdc {
    let mut fdc = Fdc::default();
    fdc.set_drive_kind(FdcDrive::A, DriveKind::ThreeAndHalfInch);
    fdc.insert_image(FdcDrive::A, Some(image));
    fdc.set_motor(true);
    drain_interrupts(&mut fdc);
    fdc
}

#[test]
fn multi_track_carries_on_with_the_other_side() {
    let mut fdc = fdc_35_with(two_sided());
    let (data, result) = read(&mut fdc, &rw(READ_DATA | MT, 0, 0, 1, 1, 2), 4 * 256);
    assert_eq!(&data[..256], vec![0x01; 256].as_slice());
    assert_eq!(&data[256..512], vec![0x02; 256].as_slice());
    assert_eq!(&data[512..768], vec![0x11; 256].as_slice(), "side 1, sector 1");
    assert_eq!(&data[768..], vec![0x12; 256].as_slice());
    assert_eq!(result[0] & 0xc0, 0x40, "End of Cylinder");
    assert_eq!((result[3], result[4], result[5]), (0, 1, 2), "ends on H=1 R=2");
}

#[test]
fn without_multi_track_the_read_stays_on_its_side() {
    let mut fdc = fdc_with(two_sided());
    let (data, _) = read(&mut fdc, &rw(READ_DATA, 0, 0, 1, 1, 2), 2 * 256);
    assert_eq!(data.len(), 512);
    // the FDC is back in the command phase after the result
    assert_eq!(fdc.read_status(), RQM);
}

// ----------------------------------------------------------- N = 0 / DTL

#[test]
fn n_zero_limits_the_transfer_to_dtl_bytes() {
    let mut image = DiscImage::new(1, 1);
    image
        .set_track(0, 0, Track {
            track: 0,
            head: 0,
            gap3: 0x4e,
            filler: 0xe5,
            sectors: vec![sector(0, 0, 1, 0, 0x33)]
        })
        .unwrap();
    let mut fdc = fdc_with(image);

    let mut command = rw(READ_DATA, 0, 0, 1, 0, 1);
    command[8] = 16; // DTL
    let (data, _) = read(&mut fdc, &command, 16);
    assert_eq!(data, vec![0x33; 16]);
}

// ------------------------------------------------------------ Read Track

#[test]
fn read_track_returns_the_sectors_in_physical_order() {
    let mut fdc = ready_fdc();
    // EOT = 3: the first three sectors from the index hole, whatever R asked
    let (data, result) = read(&mut fdc, &rw(READ_TRACK, 0, 0, 1, 1, 3), 3 * 256);
    assert_eq!(&data[..256], vec![1; 256].as_slice());
    assert_eq!(&data[256..512], vec![2; 256].as_slice());
    assert_eq!(&data[512..], vec![3; 256].as_slice());
    assert_eq!(result[0] & 0xc0, 0x00);
}

#[test]
fn read_track_starts_at_the_index_not_at_the_current_sector() {
    let mut fdc = ready_fdc();
    // move the "head" along the track first
    send(&mut fdc, &[READ_ID, 0x00]);
    receive(&mut fdc, 7);
    send(&mut fdc, &[READ_ID, 0x00]);
    receive(&mut fdc, 7);

    let (data, _) = read(&mut fdc, &rw(READ_TRACK, 0, 0, 1, 1, 1), 256);
    assert_eq!(data, vec![1; 256]);
}

// ---------------------------------------------------------- Format Track

#[test]
fn format_track_creates_the_sectors_the_cpu_describes() {
    let mut fdc = ready_fdc();
    // go to track 1 (unformatted), format 3 sectors of 512 bytes filled with &E5
    send(&mut fdc, &[0x0f, 0x00, 0x01]);
    drain_interrupts(&mut fdc);

    send(&mut fdc, &[FORMAT_TRACK, 0x00, 0x02, 0x03, 0x4e, 0xe5]);
    assert_eq!(fdc.read_status() & (RQM | DIO), RQM, "the CPU sends the IDs");
    for id in [[1u8, 0, 0xa1, 2], [1, 0, 0xa3, 2], [1, 0, 0xa2, 2]] {
        send(&mut fdc, &id);
    }
    let result = receive(&mut fdc, 7);
    assert_eq!(result[0] & 0xc0, 0x00, "formatted normally");

    // Read ID now walks the new sectors in the order they were described
    let mut ids = Vec::new();
    for _ in 0..4 {
        send(&mut fdc, &[READ_ID, 0x00]);
        let result = receive(&mut fdc, 7);
        ids.push((result[3], result[5], result[6]));
    }
    assert_eq!(ids, [(1, 0xa1, 2), (1, 0xa3, 2), (1, 0xa2, 2), (1, 0xa1, 2)]);

    // and they hold the filler byte, 512 bytes each
    let (data, _) = read(&mut fdc, &rw(READ_DATA, 1, 0, 0xa2, 2, 0xa2), 512);
    assert_eq!(data, vec![0xe5; 512]);
}

#[test]
fn format_track_beyond_the_disc_is_not_writable() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &[0x0f, 0x00, 0x09]); // track 9 on a 3 track disc
    drain_interrupts(&mut fdc);

    send(&mut fdc, &[FORMAT_TRACK, 0x00, 0x02, 0x01, 0x4e, 0xe5]);
    send(&mut fdc, &[9, 0, 1, 2]);
    let result = receive(&mut fdc, 7);
    assert_eq!(result[0] & 0xc0, 0x40);
    assert_eq!(result[1] & 0x02, 0x02, "ST1 NW");
}

#[test]
fn a_formatted_disc_can_be_saved_and_reloaded() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &[0x0f, 0x00, 0x01]);
    drain_interrupts(&mut fdc);
    send(&mut fdc, &[FORMAT_TRACK, 0x00, 0x01, 0x01, 0x4e, 0x99]);
    send(&mut fdc, &[1, 0, 0x41, 1]);
    receive(&mut fdc, 7);

    let saved = fdc.image(FdcDrive::A).unwrap().to_bytes();
    let reloaded = DiscImage::from_bytes(&saved).unwrap();
    let sector = &reloaded.track(0, 1).unwrap().sectors[0];
    assert_eq!((sector.c, sector.r, sector.n), (1, 0x41, 1));
    assert_eq!(sector.data, vec![0x99; 256]);
}

// ------------------------------------------------------------------ Scan

/// `[code, unit, C, H, R, N, EOT, GPL, STP]`
fn scan(code: u8, r: u8, eot: u8, stp: u8) -> [u8; 9] {
    [code, 0x00, 0, 0, r, 1, eot, 0x2a, stp]
}

#[test]
fn scan_equal_hits_a_matching_sector() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &scan(SCAN_EQUAL, 1, 3, 1));
    // R1 holds 1s, R2 2s: the first sector sent matches nothing, the
    // second equals R2
    send(&mut fdc, &[9; 256]);
    send(&mut fdc, &[2; 256]);
    let result = receive(&mut fdc, 7);
    assert_eq!(result[2] & 0x08, 0x08, "SH: scan equal hit");
    assert_eq!(result[2] & 0x04, 0x00, "SN clear");
    assert_eq!(result[5], 2, "stopped on the matching sector");
}

#[test]
fn scan_equal_reports_not_satisfied_when_nothing_matches() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &scan(SCAN_EQUAL, 1, 3, 1));
    for _ in 0..3 {
        send(&mut fdc, &[0x99; 256]);
    }
    let result = receive(&mut fdc, 7);
    assert_eq!(result[2] & 0x04, 0x04, "SN: scan not satisfied");
    assert_eq!(result[2] & 0x08, 0x00, "no SH");
}

#[test]
fn scan_treats_ff_as_a_wildcard() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &scan(SCAN_EQUAL, 3, 3, 1));
    send(&mut fdc, &[0xff; 256]);
    let result = receive(&mut fdc, 7);
    assert_eq!(result[2] & 0x08, 0x08);
}

#[test]
fn scan_low_or_equal_and_high_or_equal() {
    // data are R's value (1,2,3): "<= 2" first hits R1 (1 <= 2), not an equal
    let mut fdc = ready_fdc();
    send(&mut fdc, &scan(SCAN_LOW_OR_EQUAL, 1, 3, 1));
    send(&mut fdc, &[2; 256]);
    let result = receive(&mut fdc, 7);
    assert_eq!(result[5], 1);
    assert_eq!(result[2] & 0x04, 0x00, "satisfied");
    assert_eq!(result[2] & 0x08, 0x00, "but not equal");

    // ">= 3" first hits R3
    send(&mut fdc, &scan(SCAN_HIGH_OR_EQUAL, 1, 3, 1));
    send(&mut fdc, &[3; 256]);
    send(&mut fdc, &[3; 256]);
    send(&mut fdc, &[3; 256]);
    let result = receive(&mut fdc, 7);
    assert_eq!(result[5], 3);
    assert_eq!(result[2] & 0x08, 0x08, "equal hit");
}

// ------------------------------------------------------- Sense Drive Status

#[test]
fn sense_drive_status_reflects_disc_motor_and_head_position() {
    let mut fdc = Fdc::default();
    let status = |fdc: &mut Fdc| {
        send(fdc, &[0x04, 0x00]);
        receive(fdc, 1)[0]
    };

    assert_eq!(status(&mut fdc) & 0x20, 0x00, "no disc: not ready");
    fdc.insert_image(FdcDrive::A, Some(disc()));
    assert_eq!(status(&mut fdc) & 0x20, 0x00, "motor off: not ready");
    fdc.set_motor(true);
    let st3 = status(&mut fdc);
    assert_eq!(st3 & 0x20, 0x20, "RY");
    assert_eq!(st3 & 0x10, 0x10, "T0 at track 0");
    assert_eq!(st3 & 0x08, 0x00, "single sided");

    send(&mut fdc, &[0x0f, 0x00, 0x02]);
    assert_eq!(status(&mut fdc) & 0x10, 0x00, "no longer on track 0");

    // TS is the drive's: a 3" drive has one head whatever the disc
    let mut three = fdc_with(two_sided());
    assert_eq!(status(&mut three) & 0x08, 0x00, "no TS on a 3\" drive");
    let mut double = fdc_35_with(two_sided());
    assert_eq!(status(&mut double) & 0x08, 0x08, "TS on a 3.5\" drive");
}

// ------------------------------------------------------- 3" and 3.5" drives

#[test]
fn a_3_inch_drive_reads_the_side_put_up_whatever_the_head_selected() {
    let mut fdc = fdc_with(two_sided());
    // head 1 selected: a 3" drive has only one head, it reads side A of the disc
    let (data, _) = read(&mut fdc, &rw(READ_DATA, 0, 0, 1, 1, 1).iter().enumerate().map(|(i, &b)| if i == 1 { b | 4 } else { b }).collect::<Vec<_>>(), 256);
    assert_eq!(data, vec![0x01; 256], "side A");
    // the disc turned over: side B is under the head, with head 0 selected
    assert_eq!(fdc.flip_disc(FdcDrive::A), Some(1));
    drain_interrupts(&mut fdc);
    let (data, _) = read(&mut fdc, &rw(READ_DATA, 0, 1, 1, 1, 1), 256);
    assert_eq!(data, vec![0x11; 256], "side B");
    // a multi-track read does not switch heads on a 3" drive
    let (data, result) = read(&mut fdc, &rw(READ_DATA | MT, 0, 1, 1, 1, 2), 2 * 256);
    assert_eq!(&data[256..], vec![0x12; 256].as_slice());
    assert_eq!(result[0] & 0xc0, 0x40, "End of Cylinder after the side put up");
    assert_eq!((result[3], result[4], result[5]), (0, 1, 2));
    assert_eq!(fdc.read_status(), RQM, "done: no other side to go on with");
}

#[test]
fn a_3_5_inch_drive_reads_the_side_its_head_selects_and_cannot_be_flipped() {
    let mut fdc = fdc_35_with(two_sided());
    let (data, _) = read(&mut fdc, &rw(READ_DATA, 0, 1, 1, 1, 1).iter().enumerate().map(|(i, &b)| if i == 1 { b | 4 } else { b }).collect::<Vec<_>>(), 256);
    assert_eq!(data, vec![0x11; 256], "head 1 reads side B");
    assert_eq!(fdc.flip_disc(FdcDrive::A), None);
}

#[test]
fn the_heads_stop_at_the_last_track_of_their_drive() {
    let mut fdc = fdc_with(disc());
    send(&mut fdc, &[0x0f, 0x00, 79]);
    drain_interrupts(&mut fdc);
    assert_eq!(fdc.track(0), 42, "a 3\" drive stops at track 42");
    let mut fdc = fdc_35_with(disc());
    send(&mut fdc, &[0x0f, 0x00, 79]);
    drain_interrupts(&mut fdc);
    assert_eq!(fdc.track(0), 79, "a 3.5\" drive goes on to 79");
}

#[test]
fn sense_drive_status_echoes_the_head_and_unit_bits() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &[0x04, 0x05]); // HD=1, US=01
    assert_eq!(receive(&mut fdc, 1)[0] & 0x07, 0x05);
}

// -------------------------------------------------- ready-change interrupt

#[test]
fn inserting_a_disc_raises_a_ready_change_interrupt() {
    let mut fdc = Fdc::default();
    fdc.insert_image(FdcDrive::A, Some(disc()));

    fdc.write_data(0x08);
    assert_eq!(fdc.read_data(), 0xc0, "IC=11 (ready line changed), unit 0");
    assert_eq!(fdc.read_data(), 0, "PCN");

    fdc.write_data(0x08);
    assert_eq!(fdc.read_data(), 0x80, "nothing else pending");
}

#[test]
fn ejecting_a_disc_raises_it_too_and_names_the_unit() {
    let mut fdc = Fdc::default();
    fdc.insert_image(FdcDrive::B, Some(disc()));
    drain_interrupts(&mut fdc);
    fdc.insert_image(FdcDrive::B, None);

    fdc.write_data(0x08);
    assert_eq!(fdc.read_data(), 0xc1, "ready change on unit 1");
    fdc.read_data();
}

#[test]
fn seek_end_interrupts_are_reported_lowest_unit_first() {
    let mut fdc = Fdc::default();
    fdc.insert_image(FdcDrive::A, Some(disc()));
    fdc.insert_image(FdcDrive::B, Some(disc()));
    fdc.set_motor(true);
    drain_interrupts(&mut fdc);

    send(&mut fdc, &[0x0f, 0x01, 0x02]); // unit 1 to track 2
    send(&mut fdc, &[0x0f, 0x00, 0x01]); // unit 0 to track 1

    fdc.write_data(0x08);
    assert_eq!(fdc.read_data() & 0x27, 0x20, "SE, unit 0");
    assert_eq!(fdc.read_data(), 1);
    fdc.write_data(0x08);
    assert_eq!(fdc.read_data() & 0x27, 0x21, "SE, unit 1");
    assert_eq!(fdc.read_data(), 2);
}

// ------------------------------------------------------ status register

#[test]
fn execution_phase_status_matches_the_direction() {
    let mut fdc = ready_fdc();
    send(&mut fdc, &rw(READ_DATA, 0, 0, 1, 1, 1));
    assert_eq!(fdc.read_status(), RQM | DIO | (1 << 5) | CB, "execution, FDC to CPU");
    receive(&mut fdc, 256);
    assert_eq!(fdc.read_status() & (RQM | DIO | CB), RQM | DIO | CB, "result phase");
    receive(&mut fdc, 7);

    send(&mut fdc, &rw(WRITE_DATA, 0, 0, 1, 1, 1));
    assert_eq!(fdc.read_status(), RQM | (1 << 5) | CB, "execution, CPU to FDC");
}

#[test]
fn a_written_disc_is_marked_modified_until_saved() {
    let mut fdc = ready_fdc();
    assert!(!fdc.is_modified(FdcDrive::A));
    let (data, _) = read(&mut fdc, &rw(READ_DATA, 0, 0, 1, 1, 1), 256);
    assert_eq!(data.len(), 256);
    assert!(!fdc.is_modified(FdcDrive::A), "reading does not modify");
    send(&mut fdc, &rw(WRITE_DATA, 0, 0, 1, 1, 1));
    send(&mut fdc, &[0x77; 256]);
    receive(&mut fdc, 7);
    assert!(fdc.is_modified(FdcDrive::A));
    fdc.mark_saved(FdcDrive::A);
    assert!(!fdc.is_modified(FdcDrive::A));
}
