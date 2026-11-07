//! Logs every FDC command issued during a CSL script.
//! `probe_fdc_all <script.csl>` (prints the commands and a count)
use amsrust::csl_runner::CslRunner;

fn main() {
    let path = std::env::args().nth(1).expect("script");
    let mut runner = CslRunner::new("/tmp/probe_fdc_all_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    // the script up to its first key is the boot (reset, disc): log from there on
    let split = text.find("key_output").unwrap_or(text.len());
    let (head, tail) = text.split_at(split);
    runner.run_text(head, base.clone()).expect("csl head");
    runner.motherboard_mut().fdc_mut().start_command_log();
    runner
        .run_text(&format!("csl_version 1.0\n{tail}"), base)
        .expect("csl tail");
    let mb = runner.motherboard_mut();
    for (i, rec) in mb.fdc().command_log().iter().enumerate() {
        println!("{i:4} cmd {:02x?} -> {:02x?}", rec.command, rec.result);
    }
    println!("{} commands", mb.fdc().command_log().len());
}
