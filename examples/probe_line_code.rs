//! After a CSL script: from the next VSYNC start, for the 64 us lines `<from>..<to>`, every
//! instruction start (PC) and every I/O write (port, value), dated in us.tick since the VSYNC, with
//! C0 and the CRTC's HSYNC: which code runs where on a raster line.
//! `probe_line_code <script.csl> <from_line> <to_line>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let from: u32 = a.next().expect("from line").parse().unwrap();
    let to: u32 = a.next().expect("to line").parse().unwrap();
    let mut runner = CslRunner::new("/tmp/probe_line_code_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut prev_vs = mb.crtc().vsync();
    loop {
        mb.tick();
        let vs = mb.crtc().vsync();
        if vs && !prev_vs {
            break;
        }
        prev_vs = vs;
    }
    mb.watch_io_writes(true);
    let mut tick = 0u32;
    while tick < to * 64 * 16 {
        mb.tick();
        tick += 1;
        let writes: Vec<(u16, u8)> = mb.take_io_writes().collect();
        if tick / (64 * 16) < from {
            continue;
        }
        let at = format!("{:5}.{:02} C0={:3} hs={}", tick / 16, tick % 16, mb.crtc().hcc, u8::from(mb.crtc().hsync()));
        for (port, value) in writes {
            println!("{at}   OUT {port:04X},{value:02X}");
        }
        if mb.is_aligned_with_new_instruction() {
            println!("{at} {:04X}", mb.pc());
        }
    }
}
