//! After a CSL script: per line, the vertical counters at the line start and
//! the writes of R4/R7/R9 (with the C0 they land on).
//! `probe_lines2 <script.csl> <skip_lines> <lines>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let skip: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let lines: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let mut runner = CslRunner::new("/tmp/probe_lines2_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut regs = [0u8; 16];
    let mut last_hcc = 255u8;
    let mut n = 0usize;
    let mut line = String::new();
    while n < skip + lines {
        mb.tick();
        let c = mb.crtc();
        let hcc = c.hcc;
        if hcc != last_hcc && hcc == 0 {
            if n >= skip && !line.is_empty() { println!("{line}"); }
            n += 1;
            line = format!("C4={:?} C9={} R4={} R9={} LL={} VS={} |", c.vcc, c.vlc.value(), c.r4(), c.r9(), c.last_line(), c.vsync());
        }
        last_hcc = hcc;
        let now = [c.r0(), c.r1(), c.r2(), c.r3(), c.r4(), c.r5(), c.r6(), c.r7(), c.r8(), c.r9(), 0, 0, 0, 0, 0, 0];
        for r in [4usize, 5, 6, 7, 9] {
            if now[r] != regs[r] { line.push_str(&format!(" R{r}={}@{}", now[r], hcc)); }
        }
        regs = now;
    }
}
