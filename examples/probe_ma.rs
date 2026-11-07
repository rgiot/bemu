//! After a CSL script, from the next VSYNC start print per CRTC character
//! (C0 change) the C4/C9/C0, MA address and DE for lines `<from>..<to>`.
//! `probe_ma <script.csl> <from_line> <to_line>`
use amsrust::chip::Chip;
use amsrust::crtc::CrtcPin;
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let from: usize = a.next().unwrap().parse().unwrap();
    let to: usize = a.next().unwrap().parse().unwrap();
    let mut runner = CslRunner::new("/tmp/probe_ma_shots");
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
    let mut n = 0usize;
    let mut last = (255u8, 0u16);
    loop {
        mb.tick();
        let c = mb.crtc();
        let key = (c.hcc, c.vma());
        if key.0 != last.0 {
            if key.0 == 0 {
                n += 1;
            }
            if n >= from && n < to {
                println!(
                    "line {n:3} C4={:3} C9={} C0={:2} R1={} MA={:04X} DE={}",
                    c.vcc.value(),
                    c.vlc.value(),
                    c.hcc,
                    c.r1(),
                    c.vma(),
                    c.get_pin(CrtcPin::DE)
                );
            }
            if n >= to {
                break;
            }
        }
        last = key;
    }
}
