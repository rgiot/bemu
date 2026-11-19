//! Prints the text `cpclib-ocr` reads in each picture given: `cargo run --release --example ocr_dump -- pic.png...`
use cpclib_ocr::{Font, Ocr, Picture};

fn main() {
    let rom = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/roms/os6128.rom")).unwrap();
    let ocr = Ocr::new(Font::from_lower_rom(&rom).unwrap());
    for path in std::env::args().skip(1) {
        let image = image::open(&path).unwrap().to_rgb8();
        let picture = Picture::new(
            image.width() as usize,
            image.height() as usize,
            image.into_raw(),
        )
        .unwrap();
        let screen = ocr.read(&picture);
        println!(
            "== {path} (pixel {:?}, {} known, {} unknown)\n{}",
            screen.pixel,
            screen.known,
            screen.unknown,
            screen.text()
        );
    }
}
