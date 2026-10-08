//! Per-line CRTC/GA sync statistics after a CSL script: for each CRTC line
//! (HSYNC start to HSYNC start): period, HSYNC width, GA HSYNC width.
//! `probe_lines <script.csl> <lines>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let lines: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(40);
    let mut runner = CslRunner::new("/tmp/probe_lines_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let (mut t, mut last_start, mut crtc_w, mut ga_w, mut ga_start) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut ch, mut gh) = (false, false);
    let mut n = 0;
    while n < lines {
        mb.tick();
        t += 1;
        let c = mb.crtc().hsync();
        let g = mb.ga().sync().hsync;
        if c && !ch {
            println!("line {n:3} period={:5} crtc_hs_w={:4} ga_hs_w={:4} R0={} R2={} R3={} C0={} vs={}", t - last_start, crtc_w, ga_w, mb.crtc().r0(), mb.crtc().r2(), mb.crtc().r3(), mb.crtc().hcc, mb.crtc().vsync());
            last_start = t;
            n += 1;
            crtc_w = 0;
        }
        if c { crtc_w += 1; }
        if g && !gh { ga_start = t; }
        if g { ga_w = t - ga_start + 1; }
        ch = c;
        gh = g;
    }
}
