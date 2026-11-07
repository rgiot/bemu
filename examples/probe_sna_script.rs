//! Loads a .sna and plays a CSL script on top of it (keys, waits, screenshots).
//! `probe_sna_script <file.sna> <script.csl> <out_dir> [crtc]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let usage = "usage: <file.sna> <script.csl> <out_dir> [crtc]";
    let sna = a.next().expect(usage);
    let script = a.next().expect(usage);
    let out = a.next().expect(usage);
    let crtc: u8 = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    std::fs::create_dir_all(&out).unwrap();
    let mut runner = CslRunner::new(&out);
    runner
        .run_text(
            &format!("csl_version 1.0\ncrtc_select {crtc}\nreset\nwait 1000000\n"),
            ".",
        )
        .unwrap();
    amsrust::snapshot::load_sna(runner.motherboard_mut(), &sna).expect("load sna");
    let text = std::fs::read_to_string(&script).unwrap();
    let base = std::path::Path::new(&script)
        .parent()
        .unwrap()
        .to_path_buf();
    let report = runner.run_text(&text, base).unwrap();
    for i in report.ignored {
        eprintln!("ignored: {i}");
    }
    println!("PC={:04X}", runner.motherboard_mut().pc());
}
