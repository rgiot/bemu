//! Between two PCs, record (opcode key -> durations in microseconds) of every
//! instruction executed. `probe_opcode_timing <script.csl> <from_pc> <to_pc> [to_hits]`
use amsrust::csl_runner::CslRunner;
use std::collections::BTreeMap;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let from = u16::from_str_radix(&a.next().expect("from"), 16).unwrap();
    let to = u16::from_str_radix(&a.next().expect("to"), 16).unwrap();
    let to_hits: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    let mut runner = CslRunner::new("/tmp/probe_opcode_timing_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut guard = 0u64;
    while mb.pc() != from {
        mb.execute_next_opcode_no_video();
        guard += 1;
        assert!(guard < 80_000_000);
    }
    let mut stats: BTreeMap<String, BTreeMap<u64, usize>> = BTreeMap::new();
    let mut hits = 0;
    let mut n = 0u64;
    while hits < to_hits && n < 3_000_000 {
        let pc = mb.pc();
        let b: Vec<u8> = (0..4).map(|i| mb.read_byte(pc.wrapping_add(i))).collect();
        let key = match b[0] {
            0xcb | 0xed => format!("{:02X} {:02X}", b[0], b[1]),
            0xdd | 0xfd if b[1] == 0xcb => format!("{:02X} CB d {:02X}", b[0], b[3]),
            0xdd | 0xfd => format!("{:02X} {:02X}", b[0], b[1]),
            _ => format!("{:02X}", b[0]),
        };
        let t0 = mb.t_state_counter().value();
        mb.execute_next_opcode_no_video();
        let dt = (mb.t_state_counter().value() - t0) as u64;
        *stats.entry(key).or_default().entry(dt).or_default() += 1;
        n += 1;
        if mb.pc() == to {
            hits += 1;
        }
    }
    for (k, v) in &stats {
        let d: Vec<String> = v.iter().map(|(dt, c)| format!("{}us x{}", dt, c)).collect();
        println!("{k}: {}", d.join(", "));
    }
}
