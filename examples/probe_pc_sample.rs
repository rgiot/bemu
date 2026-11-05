//! `probe_pc_sample <script.csl> [ticks]`: plays a script, then lists the PCs the Z80 goes through
//! (with counts) and dumps the memory around the most visited one.
use amsrust::csl_runner::CslRunner;
use std::collections::BTreeMap;

fn main() {
    let mut a = std::env::args().skip(1);
    let script = a.next().expect("script.csl");
    let ticks: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1_600_000);
    let mut runner = CslRunner::new("/tmp/probe_pc_shots");
    if std::env::var_os("DRIVE35").is_some() {
        runner.set_drive_kind(0, "3.5".parse().unwrap());
        runner.set_drive_kind(1, "3.5".parse().unwrap());
    }
    let base = std::path::Path::new(&script).parent().unwrap().to_path_buf();
    runner.run_text(&std::fs::read_to_string(&script).unwrap(), base).unwrap();
    let mb = runner.motherboard_mut();
    println!("start at {} us, keyboard lines {:02X?}", mb.fdc().time_us(), mb.keyboard().lines());
    let mut count: BTreeMap<u16, u32> = BTreeMap::new();
    for _ in 0..ticks {
        mb.tick();
        *count.entry(mb.pc()).or_default() += 1;
    }
    let mut v: Vec<_> = count.into_iter().collect();
    v.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    for (pc, c) in v.iter().take(12) {
        println!("{pc:04X} {c}");
    }
    let (pc, _) = v[0];
    for extra in [0x4BE0u16, 0x4C20, 0x4C60, 0x4CA0, 0x4D30, 0x4D70, 0x4DB0] { let b: Vec<String> = (0..64u16).map(|i| format!("{:02X}", mb.read_byte(extra.wrapping_add(i)))).collect(); println!("{extra:04X}: {}", b.join(" ")); }
    let start = pc.wrapping_sub(16);
    let bytes: Vec<String> = (0..48u16).map(|i| format!("{:02X}", mb.read_byte(start.wrapping_add(i)))).collect();
    println!("{start:04X}: {}", bytes.join(" "));
}
