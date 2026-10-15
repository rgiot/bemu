//! After a CSL script, print for the next `<ms>` milliseconds the time (µs) of each CRTC VSYNC start
//! and each interrupt acceptance (PC reaching 0x0038/0x0039): `probe_int_vs_vsync <script.csl> <ms>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let ms: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    let mut runner = CslRunner::new("/tmp/probe_int_vs_vsync_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    mb.crtc_mut().start_write_log();
    let mut t = 0u64; // 1/16 µs
    let mut prev_vs = mb.crtc().vsync();
    let mut prev_pc = mb.pc();
    let mut last_vs = 0u64;
    while t < ms * 16000 {
        mb.tick();
        t += 1;
        let vs = mb.crtc().vsync();
        if vs && !prev_vs {
            println!("{:9.1} VSYNC (C0={})", t as f64 / 16.0, mb.crtc().hcc);
            last_vs = t;
        }
        prev_vs = vs;
        let pc = mb.pc();
        if pc != prev_pc && (pc == 0x38 || pc == 0x39) {
            println!("{:9.1} INT   (+{:.1} after vsync)", t as f64 / 16.0, (t - last_vs) as f64 / 16.0);
        }
        prev_pc = pc;
        let _ = mb.crtc_mut().take_write_log();
    }
}
