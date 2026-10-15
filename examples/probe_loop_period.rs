//! After a CSL script: ticks between VSYNC starts and between R0<-0 writes.
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut runner = CslRunner::new("/tmp/probe_loop_period_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    mb.crtc_mut().start_write_log();
    let (mut t, mut prev_vs, mut last_vs, mut last_r0, mut seen) = (0u64, mb.crtc().vsync(), 0u64, 0u64, 0);
    while seen < n {
        mb.tick();
        t += 1;
        let vs = mb.crtc().vsync();
        if vs && !prev_vs {
            println!("VSYNC at {t} (+{})", t - last_vs);
            last_vs = t;
            seen += 1;
        }
        prev_vs = vs;
        let mut first = true;
        for (_, _, _, r, v) in mb.crtc_mut().take_write_log() {
            if r == 0 && v == 0 && first && t - last_r0 > 100 {
                println!("  R0<-0 at {t} (+{})", t - last_r0);
                last_r0 = t;
            }
            first = false;
        }
    }
}
