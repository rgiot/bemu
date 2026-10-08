//! After a CSL script, histogram of PCs over N instructions and a hex dump
//! of a memory range. `probe_pc <script.csl> <instructions> [dump_from_hex dump_len]`
use amsrust::csl_runner::CslRunner;
use std::collections::BTreeMap;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(200000);
    let from = a.next().map(|s| u16::from_str_radix(&s, 16).unwrap());
    let len: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(256);
    let mut runner = CslRunner::new("/tmp/probe_pc_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut hist: BTreeMap<u16, usize> = BTreeMap::new();
    for _ in 0..n {
        mb.execute_next_opcode_no_video();
        *hist.entry(mb.pc()).or_default() += 1;
    }
    let mut v: Vec<_> = hist.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (pc, c) in v.iter().take(40) { println!("{pc:04X} {c}"); }
    if let Some(from) = from {
        for row in 0..(len / 16) {
            let base = from.wrapping_add((row * 16) as u16);
            let bytes: Vec<String> = (0..16).map(|i| format!("{:02X}", mb.read_byte(base.wrapping_add(i)))).collect();
            println!("{base:04X}: {}", bytes.join(" "));
        }
    }
}
