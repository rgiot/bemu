//! Runs a CSL script (booting a demo up to a point), then logs every FDC
//! command issued during the next `<ms>` of emulated time after typing space.
//! `probe_fdc_space <script.csl> [ms]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let ms: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(4000);
    let mut runner = CslRunner::new("/tmp/probe_fdc_space_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, base.clone()).expect("csl");
    runner.motherboard_mut().fdc_mut().start_command_log();
    runner
        .run_text(
            &format!("csl_version 1.0\nkey_output ' '\nwait {}\n", ms * 1000),
            base,
        )
        .expect("space");
    let mb = runner.motherboard_mut();
    for (i, rec) in mb.fdc().command_log().iter().enumerate() {
        println!("{i:4} cmd {:02x?} -> {:02x?}", rec.command, rec.result);
    }
    println!("PC={:04X}", mb.pc());
}
