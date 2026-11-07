//! `probe_fdc_log <script.csl> [max]`: plays a script and prints the FDC commands (and results) the
//! program sent, with the microsecond they ended at, to compare with CPCSE_TRACE_FDC.
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let script = a.next().expect("script.csl");
    let max: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(60);
    let mut runner = CslRunner::new("/tmp/probe_fdc_shots");
    if std::env::var_os("DRIVE35").is_some() {
        runner.set_drive_kind(0, "3.5".parse().unwrap());
        runner.set_drive_kind(1, "3.5".parse().unwrap());
    }
    runner.motherboard_mut().fdc_mut().start_command_log();
    let base = std::path::Path::new(&script)
        .parent()
        .unwrap()
        .to_path_buf();
    runner
        .run_text(&std::fs::read_to_string(&script).unwrap(), base)
        .unwrap();
    for (i, r) in runner
        .motherboard_mut()
        .fdc()
        .command_log()
        .iter()
        .take(max)
        .enumerate()
    {
        let hex = |v: &[u8]| {
            v.iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!(
            "{i:3} {:>9} {:>9} {} -> {}",
            r.start_us,
            r.end_us,
            hex(&r.command),
            hex(&r.result)
        );
    }
}
