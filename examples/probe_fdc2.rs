//! Run a CSL script with FDC command logging on, then print the commands
//! issued so far. `probe_fdc2 <script.csl> [more_seconds]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let secs: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut runner = CslRunner::new("/tmp/probe_fdc2_shots");
    runner.motherboard_mut().fdc_mut().start_command_log();
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut last = mb.fdc().command_log().len();
    println!("after script: commands={last}");
    for sec in 0..secs {
        for _ in 0..250_000 {
            mb.execute_next_opcode_no_video();
        }
        let n = mb.fdc().command_log().len();
        println!(
            "+{}x0.25s: commands={} (+{}) motor={} PC={:04X}",
            sec,
            n,
            n - last,
            mb.fdc().motor(),
            mb.pc()
        );
        last = n;
    }
    let log = mb.fdc().command_log();
    println!("--- commands (first 12, last 12) of {}", log.len());
    for r in log.iter().take(12) {
        println!("{:02X?} -> {:02X?}", r.command, r.result);
    }
    println!("...");
    for r in log.iter().rev().take(12).rev() {
        println!("{:02X?} -> {:02X?}", r.command, r.result);
    }
}
