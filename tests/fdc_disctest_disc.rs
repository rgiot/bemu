//! The FDC against the error test disc of DISCTEST
//! (<https://github.com/WacKEDmaN/Z80ASM>, `tools/mkdsk.py tests`, copied to
//! `tests/fixtures/disctest_data_errors.dsk`) read the way that program does:
//! SEEK, READ ID, READ DATA of one sector (EOT=R). Expected answers are the
//! ones in its `docs/notes/cpc-fdc-notes.md` (uPD765, checked against MAME):
//! a CRC error in an ID ends the command before any data, a CRC error in the
//! data arrives after all of it, an unformatted track gives MA|ND, ...

use amsrust::disc_image::load_dsk;
use amsrust::fdc::{Fdc, FdcDrive};

const RQM: u8 = 1 << 7;
const EXM: u8 = 1 << 5;

fn send(fdc: &mut Fdc, bytes: &[u8]) {
    for &b in bytes {
        assert!(fdc.read_status() & RQM != 0, "refused {b:#04x}");
        fdc.write_data(b);
    }
}

fn recv(fdc: &mut Fdc, n: usize) -> Vec<u8> {
    (0..n).map(|_| fdc.read_data()).collect()
}

fn disc() -> Fdc {
    let bytes = std::fs::read("tests/fixtures/disctest_data_errors.dsk").expect("fixture");
    let mut fdc = Fdc::default();
    fdc.insert_image(FdcDrive::A, Some(load_dsk(&bytes).expect("dsk")));
    fdc.set_motor(true);
    for _ in 0..4 {
        send(&mut fdc, &[0x08]);
        if recv(&mut fdc, 1)[0] == 0x80 {
            break;
        }
        let _ = recv(&mut fdc, 1);
    }
    fdc
}

fn seek(fdc: &mut Fdc, track: u8) {
    send(fdc, &[0x0f, 0x00, track]);
    send(fdc, &[0x08]);
    let _ = recv(fdc, 2);
}

fn read_id(fdc: &mut Fdc) -> Vec<u8> {
    send(fdc, &[0x4a, 0x00]);
    recv(fdc, 7)
}

/// (data bytes received, [ST0, ST1, ST2, C, H, R, N])
fn read_sector(fdc: &mut Fdc, c: u8, r: u8, n: u8) -> (usize, Vec<u8>) {
    send(fdc, &[0x46, 0x00, c, 0, r, n, r, 0x2a, 0xff]);
    let mut data = 0;
    while fdc.read_status() & EXM != 0 {
        let _ = fdc.read_data();
        data += 1;
        assert!(data < 70_000, "execution phase never ends");
    }
    (data, recv(fdc, 7))
}

#[test]
fn an_unformatted_track_gives_missing_address_mark_and_no_data() {
    let mut fdc = disc();
    seek(&mut fdc, 22);
    let id = read_id(&mut fdc);
    assert_eq!((id[0] & 0xc0, id[1], id[2]), (0x40, 0x05, 0x00), "{id:02x?}");
}

#[test]
fn a_data_crc_error_arrives_after_all_the_data() {
    let mut fdc = disc();
    seek(&mut fdc, 20);
    let (data, res) = read_sector(&mut fdc, 20, 0xc3, 2);
    assert_eq!(data, 512);
    assert_eq!((res[0] & 0xc0, res[1], res[2]), (0x40, 0x20, 0x20), "{res:02x?}");
}

#[test]
fn an_id_crc_error_ends_the_command_before_any_data() {
    let mut fdc = disc();
    seek(&mut fdc, 20);
    let (data, res) = read_sector(&mut fdc, 20, 0xc6, 2);
    assert_eq!(data, 0);
    assert_eq!(res[0] & 0xc0, 0x40);
    assert_ne!(res[1] & 0x20, 0, "DE must be set: {res:02x?}");
    assert_eq!(res[2] & 0x20, 0, "DD is for the data field only");
}

#[test]
fn read_id_reports_an_id_with_a_bad_crc_with_ma_de_nd() {
    let mut fdc = disc();
    seek(&mut fdc, 20);
    let ids: Vec<_> = (0..9).map(|_| read_id(&mut fdc)).collect();
    let bad = ids.iter().find(|id| id[5] == 0xc6).expect("the bad ID passes under the head");
    assert_eq!((bad[0] & 0xc0, bad[1]), (0x40, 0x25), "{bad:02x?}");
    let good = ids.iter().find(|id| id[5] == 0xc1).unwrap();
    assert_eq!(good[1], 0);
}

#[test]
fn a_deleted_data_mark_is_read_and_flagged() {
    let mut fdc = disc();
    seek(&mut fdc, 21);
    let (data, res) = read_sector(&mut fdc, 21, 0xc5, 2);
    assert_eq!(data, 512);
    assert_eq!(res[2] & 0x40, 0x40, "control mark: {res:02x?}");
}

#[test]
fn a_sector_not_found_or_without_data_mark_transfers_nothing() {
    let mut fdc = disc();
    seek(&mut fdc, 23);
    let (data, res) = read_sector(&mut fdc, 23, 0xc2, 2);
    assert_eq!((data, res[0] & 0xc0, res[1] & 0x04), (0, 0x40, 0x04), "{res:02x?}");
    let (data, res) = read_sector(&mut fdc, 23, 0xc8, 2);
    assert_eq!((data, res[0] & 0xc0, res[1] & 0x01, res[2] & 0x01), (0, 0x40, 0x01, 0x01), "{res:02x?}");
}

#[test]
fn eighteen_small_sectors_and_an_8k_sector_read_in_full() {
    let mut fdc = disc();
    seek(&mut fdc, 24);
    for r in [1u8, 9, 18] {
        let (data, res) = read_sector(&mut fdc, 24, r, 1);
        assert_eq!((data, res[1], res[2]), (256, 0x80, 0), "R={r}");
    }
    seek(&mut fdc, 25);
    let (data, res) = read_sector(&mut fdc, 25, 0xc1, 6);
    assert_eq!(data, 8192);
    assert_eq!((res[1], res[2]), (0x20, 0x20), "its CRC is recorded as bad");
}

#[test]
fn ids_with_a_wrong_cylinder_are_not_found_and_flag_wc() {
    let mut fdc = disc();
    seek(&mut fdc, 26);
    let (data, res) = read_sector(&mut fdc, 26, 0x41, 2);
    assert_eq!((data, res[1] & 0x04, res[2] & 0x10), (0, 0x04, 0x10), "{res:02x?}");
}
