//! After a CSL script, write the 64K the Z80 sees to a file.
//! `dump_ram <script.csl> <out.bin>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let out = a.next().expect("output file");
    let mut runner = CslRunner::new("/tmp/dump_ram_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let bytes: Vec<u8> = (0..=0xffffu16).map(|a| mb.read_byte(a)).collect();
    std::fs::write(out, bytes).unwrap();
}
