//! After a CSL script, log changes of R4/R7/R9 with the CRTC vertical
//! counters, plus every VSYNC edge. `probe_vregs <script.csl> <ticks> [max_lines]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let ticks: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let max: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(150);
    let mut runner = CslRunner::new("/tmp/probe_vregs_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut last = (0, 0, 0, false);
    let mut shown = 0;
    for t in 0..ticks {
        mb.tick();
        let c = mb.crtc();
        let r9 = if std::env::var("WITH_R9").is_ok() { c.r9() } else { 0 };
        let now = (c.r4(), c.r7(), r9, c.vsync());
        if now != last && shown < max {
            println!("t={t:8} R4={} R7={} R9={} VS={} R5={} R6={} | C4={:?} C9={} C0={}", now.0, now.1, now.2, now.3, c.r5(), c.r6(), c.vcc, c.vlc.value(), c.hcc);
            shown += 1;
        }
        last = now;
    }
}
