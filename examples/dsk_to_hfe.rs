//! Convert a disc image to HFE with the HxC floppy library (as HxC's tools would write it):
//! `dsk_to_hfe <in.dsk> <out.hfe>`. For trying the emulator's HFE support on known discs.

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: dsk_to_hfe <in.dsk> <out.hfe>");
        std::process::exit(2);
    };
    let img = hxcfe::Hxcfe::get().load(&input).unwrap_or_else(|e| panic!("{input}: {e}"));
    let hfe = img.save_to_buffer(hxcfe::ImageFormat::HxcHfe).unwrap_or_else(|e| panic!("HFE: {e}"));
    std::fs::write(&output, hfe).unwrap_or_else(|e| panic!("{output}: {e}"));
}
