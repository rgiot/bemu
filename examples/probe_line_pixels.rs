//! After a CSL script: for CRTC line N after a VSYNC start, print per GA tick
//! the RGB, DISPEN and HSYNC state inside given tick ranges.
//! `probe_line_pixels <script.csl> <line> <from_tick> <to_tick> [<from2> <to2>]`
use amsrust::chip::Chip;
use amsrust::csl_runner::CslRunner;
use amsrust::ga::GateArrayPin;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let line: usize = a.next().unwrap().parse().unwrap();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    while let Some(f) = a.next() {
        let t = a.next().expect("pair");
        ranges.push((f.parse().unwrap(), t.parse().unwrap()));
    }
    let mut runner = CslRunner::new("/tmp/probe_line_pixels_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut prev_vs = mb.crtc().vsync();
    loop {
        mb.tick();
        let vs = mb.crtc().vsync();
        if vs && !prev_vs { break; }
        prev_vs = vs;
    }
    let mut n = 0usize;
    let mut last_hcc = 255u8;
    let mut tick = 0u32;
    loop {
        let (rgb, sync) = mb.tick();
        tick += 1;
        let hcc = mb.crtc().hcc;
        if n == line && ranges.iter().any(|(f, t)| tick >= *f && tick < *t) {
            let dispen = mb.ga().get_pin(GateArrayPin::DISPEN);
            println!("tick {tick:4} C0={hcc:2} rgb=({:.0},{:.0},{:.0}) DISPEN={} chs={} hs_ga={}", rgb.r(), rgb.g(), rgb.b(), dispen, sync.hsync, mb.ga().hsync_ga());
        }
        if hcc == 0 && last_hcc != 0 {
            n += 1; tick = 0;
            if n > line { break; }
        }
        last_hcc = hcc;
    }
}
