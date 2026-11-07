//! Runs a .sna in amsrust for a while and saves one screenshot of the result.
//!
//! `cargo run --release --example probe_sna -- <file.sna> <out_dir> [crtc] [wait_seconds]`

use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let sna = a
        .next()
        .expect("usage: <file.sna> <out_dir> [crtc] [wait_s]");
    let out = a
        .next()
        .expect("usage: <file.sna> <out_dir> [crtc] [wait_s]");
    let crtc: u8 = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let wait: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    std::fs::create_dir_all(&out).unwrap();
    let mut runner = CslRunner::new(&out);
    runner
        .run_text(
            &format!("csl_version 1.0\ncrtc_select {crtc}\nreset\nwait 1000000\n"),
            ".",
        )
        .unwrap();
    amsrust::snapshot::load_sna(runner.motherboard_mut(), &sna).expect("load sna");
    runner.set_verbose(true);
    let report = runner
        .run_text(
            &format!("csl_version 1.0\nwait {}\nscreenshot\n", wait * 1_000_000),
            ".",
        )
        .unwrap();
    for i in report.ignored {
        eprintln!("ignored: {i}");
    }
}
