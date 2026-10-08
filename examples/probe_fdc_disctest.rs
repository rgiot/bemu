//! Reads the interesting sectors of DISCTEST's error test disc the way that
//! program does (SEEK, READ ID until the IDs repeat, READ DATA with EOT=R) and
//! prints what the FDC answers, to compare with `docs/notes/cpc-fdc-notes.md` of
//! <https://github.com/WacKEDmaN/Z80ASM>.
//!
//! `cargo run --release --example probe_fdc_disctest`

use amsrust::disc_image::load_dsk;
use amsrust::fdc::{Fdc, FdcDrive};

const RQM: u8 = 1 << 7;

fn send(fdc: &mut Fdc, bytes: &[u8]) {
    for &b in bytes {
        assert!(fdc.read_status() & RQM != 0, "refused {b:#04x}");
        fdc.write_data(b);
    }
}
fn recv(fdc: &mut Fdc, n: usize) -> Vec<u8> {
    (0..n).map(|_| fdc.read_data()).collect()
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
/// Returns (data bytes received, result bytes)
fn read_sector(fdc: &mut Fdc, c: u8, h: u8, r: u8, n: u8) -> (usize, Vec<u8>) {
    send(fdc, &[0x46, h << 2, c, h, r, n, r, 0x2a, 0xff]);
    // the execution phase: bytes while RQM+DIO+EXM; else straight to the result
    let mut data = 0;
    loop {
        let st = fdc.read_status();
        let exm = st & (1 << 5) != 0;
        if !exm {
            break;
        }
        let _ = fdc.read_data();
        data += 1;
        if data > 70_000 {
            break;
        }
    }
    (data, recv(fdc, 7))
}

fn main() {
    let bytes = std::fs::read("tests/fixtures/disctest_data_errors.dsk").expect("fixture");
    let image = load_dsk(&bytes).expect("dsk");
    let mut fdc = Fdc::default();
    fdc.insert_image(FdcDrive::A, Some(image));
    fdc.set_motor(true);
    for _ in 0..4 {
        send(&mut fdc, &[0x08]);
        let r = recv(&mut fdc, 1);
        if r[0] == 0x80 {
            break;
        }
        let _ = recv(&mut fdc, 1);
    }
    let cases: &[(&str, u8, &[(u8, u8, u8, u8)])] = &[
        ("data CRC error (T20 C3)", 20, &[(20, 0, 0xc3, 2)]),
        ("ID CRC error (T20 C6)", 20, &[(20, 0, 0xc6, 2)]),
        ("deleted data (T21 C5)", 21, &[(21, 0, 0xc5, 2)]),
        ("not found ND (T23 C2)", 23, &[(23, 0, 0xc2, 2)]),
        ("MA+MD (T23 C8)", 23, &[(23, 0, 0xc8, 2)]),
        ("18x256 (T24 R1,R18)", 24, &[(24, 0, 1, 1), (24, 0, 18, 1)]),
        ("8K sector N=6 CRC (T25)", 25, &[(25, 0, 0xc1, 6)]),
        ("wrong cylinder C=99 (T26 R41)", 26, &[(26, 0, 0x41, 2), (26, 0, 0xff, 2)]),
    ];
    println!("READ ID on unformatted track 22:");
    seek(&mut fdc, 22);
    println!("  -> {:02x?}", read_id(&mut fdc));
    for (name, track, sectors) in cases {
        seek(&mut fdc, *track);
        println!("{name}:");
        let ids: Vec<_> = (0..4).map(|_| read_id(&mut fdc)).collect();
        println!("  READ ID x4 -> {:02x?}", ids);
        for &(c, h, r, n) in *sectors {
            let (data, res) = read_sector(&mut fdc, c, h, r, n);
            println!("  READ DATA C={c} R={r:02x} N={n}: {data} data bytes, ST0 {:02x} ST1 {:02x} ST2 {:02x} CHRN {:02x?}", res[0], res[1], res[2], &res[3..]);
        }
    }
}
