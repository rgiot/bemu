//! The AMSDOS catalogue of a disc image as the emulator reads it (a DSK, an HFE, `file.zip`,
//! `file.zip#entry`): `disc_cat <disc>`.

fn main() {
    let spec = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: disc_cat <disc>");
        std::process::exit(2);
    });
    let image = amsrust::disc_image::load_disc_file(&spec).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    println!(
        "{} tracks, {} side(s){}",
        image.nb_tracks(),
        image.nb_heads(),
        if image.is_bitstream() {
            ", bitstream"
        } else {
            ""
        }
    );
    for entry in image.amsdos_catalog() {
        println!(
            "{:>3}K  {}",
            (usize::from(entry.records) * 128).div_ceil(1024),
            entry.filename()
        );
    }
}
