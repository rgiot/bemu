//! After a CSL script, one frame line by line: the counters at the start of
//! the line, R1/R4/R9, how many characters have DE on, and the first address.
//! `probe_rows <script.csl> [lines]`
use amsrust::chip::Chip;
use amsrust::crtc::CrtcPin;
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let lines: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(320);
    let mut runner = CslRunner::new("/tmp/probe_rows_shots");
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
    let mut de = 0u32;
    let mut first_ma = 0u16;
    let mut head = String::new();
    let mut seen_chars = 0u32;
    while n < lines {
        mb.tick();
        let c = mb.crtc();
        if c.hcc != last_hcc {
            if c.hcc == 0 {
                if !head.is_empty() { println!("{head} de={de:3} ma0={first_ma:04X} chars={seen_chars}"); n += 1; }
                head = format!("line {n:3} C4={:3} C9={:2} R0={:2} R1={:2} R4={:3} R9={:2} R12={:02X} R13={:02X} vs={} r52={}", c.vcc.value(), c.vlc.value(), c.r0(), c.r1(), c.r4(), c.r9(), c.r12(), c.r13(), u8::from(c.vsync()), mb.ga().r52());
                de = 0; seen_chars = 0; first_ma = c.vma();
            }
            seen_chars += 1;
            if c.get_pin(CrtcPin::DE) { de += 1; }
            last_hcc = c.hcc;
        }
    }
}
