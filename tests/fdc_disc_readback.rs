//! Every sector of every real disc image we ship for validation, read back
//! through the FDC's own command interface (Seek + Read Data, one sector at
//! a time like AMSDOS and the demo loaders do) must return exactly the
//! bytes stored in the image, with a normal ST0. Catches an FDC that
//! corrupts, truncates or mis-addresses real-world data, as opposed to the
//! synthetic discs of `tests/fdc_commands.rs`.

use amsrust::disc_image::{load_dsk, DiscImage, Sector};
use amsrust::fdc::{Fdc, FdcDrive};

const RQM: u8 = 1 << 7;

const DISCS: &[&str] = &[
    "validation/Benediction/HBL.DSK",
    "validation/Benediction/bnd4_package.dsk",
    "validation/AmstradDiag/AmstradDiag.dsk",
    "validation/Shaker_CSL/shaker27.dsk",
    "validation/RHINO.DSK",
    "validation/TESTCRTC.DSK"
];

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

fn seek(fdc: &mut Fdc, track: u8) {
    send(fdc, &[0x0f, 0x00, track]);
    send(fdc, &[0x08]);
    let st0 = receive(fdc, 2);
    assert_eq!(st0[1], track, "seek ended on the wrong cylinder");
}

fn readable(sector: &Sector, track: &[Sector]) -> bool {
    let unique = track
        .iter()
        .filter(|o| (o.c, o.h, o.r, o.n) == (sector.c, sector.h, sector.r, sector.n))
        .count()
        == 1;
    unique && sector.copies() == 1 && sector.data.len() == sector.nominal_len() && sector.nominal_len() > 0
}

fn check_disc(path: &str, image: DiscImage) -> (usize, usize) {
    let (mut checked, mut skipped) = (0, 0);
    let mut fdc = Fdc::default();
    fdc.insert_image(FdcDrive::A, Some(image.clone()));
    fdc.set_motor(true);
    for _ in 0..8 {
        fdc.write_data(0x08);
        if fdc.read_data() == 0x80 {
            break;
        }
        fdc.read_data();
    }

    for head in 0..image.nb_heads() {
        for cyl in 0..image.nb_tracks() {
            let Some(track) = image.track(head, cyl)
            else {
                continue;
            };
            seek(&mut fdc, cyl);
            for sector in &track.sectors {
                if !readable(sector, &track.sectors) {
                    skipped += 1;
                    continue;
                }
                let unit_head = head << 2;
                send(
                    &mut fdc,
                    &[0x46, unit_head, sector.c, sector.h, sector.r, sector.n, sector.r, 0x2a, 0xff]
                );
                let data = receive(&mut fdc, sector.nominal_len());
                let result = receive(&mut fdc, 7);
                let ctx = format!("{path} head {head} track {cyl} R={:#04x}", sector.r);
                assert_eq!((result[0] & 0xc0, result[1]), (0x40, 0x80), "{ctx}: not a clean End of Cylinder {result:02x?}");
                assert_eq!(data, sector.data, "{ctx}: data differs from the image");
                checked += 1;
            }
        }
    }
    (checked, skipped)
}

#[test]
fn every_readable_sector_of_the_validation_discs_reads_back_verbatim() {
    let mut total = 0;
    for path in DISCS {
        let Ok(bytes) = std::fs::read(path)
        else {
            eprintln!("{path}: not present, skipped");
            continue;
        };
        let image = load_dsk(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        let (checked, skipped) = check_disc(path, image);
        eprintln!("{path}: {checked} sectors verified, {skipped} skipped (weak/duplicate/odd size)");
        total += checked;
    }
    assert!(total > 0, "no disc found to check");
}
