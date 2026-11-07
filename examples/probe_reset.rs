//! After a CSL script, run instructions until PC reaches `<stop_pc>` (default 0:
//! a reset) and print the last `<ring>` instructions before it with the
//! registers. `probe_reset <script.csl> [stop_pc_hex] [max_instructions] [ring]`
use amsrust::csl_runner::CslRunner;
use amsrust::z80::{HasValue, Reg16};
use std::collections::VecDeque;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let stop = a
        .next()
        .map(|s| u16::from_str_radix(&s, 16).unwrap())
        .unwrap_or(0);
    let max: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(200_000_000);
    let ring_len: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(60);
    let mut runner = CslRunner::new("/tmp/probe_reset_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut ring: VecDeque<String> = VecDeque::new();
    let mut window: std::collections::BTreeMap<u16, u32> = Default::default();
    let watch: Option<u16> = std::env::var("PROBE_WATCH")
        .ok()
        .and_then(|v| u16::from_str_radix(&v, 16).ok());
    let mut watched = watch.map(|w| mb.read_byte(w));
    for n in 0..max {
        let pc = mb.pc();
        if let Some(w) = watch {
            let v = mb.read_byte(w);
            if Some(v) != watched {
                println!(
                    "n={n} watch {w:04X}: {:02X} -> {v:02X}, last pc {}",
                    watched.unwrap(),
                    ring.back().cloned().unwrap_or_default()
                );
                watched = Some(v);
            }
        }
        if std::env::var("PROBE_HIST").is_ok() {
            *window.entry(pc >> 8).or_default() += 1;
            if n % 20000 == 19999 || (n > 930000 && n % 2000 == 1999) {
                let mut v: Vec<_> = window.iter().collect();
                v.sort_by(|a, b| b.1.cmp(a.1));
                let top: Vec<String> = v
                    .iter()
                    .take(5)
                    .map(|(p, c)| format!("{:02X}xx:{}", p, c))
                    .collect();
                println!("n={} top pages {}", n + 1, top.join(" "));
                window.clear();
            }
        }
        if pc == stop && n > 0 {
            println!("reached {stop:04X} after {n} instructions; last instructions:");
            for l in &ring {
                println!("{l}");
            }
            return;
        }
        if std::env::var("PROBE_JUMPS").is_ok() && n > 596000 && n < 597200 {
            let prev = ring
                .back()
                .map(|l| u16::from_str_radix(&l[..4], 16).unwrap())
                .unwrap_or(0);
            if pc.wrapping_sub(prev) > 4 {
                println!(
                    "n={n} jump {prev:04X} -> {pc:04X}  {}",
                    ring.back().cloned().unwrap_or_default()
                );
            }
        }
        if let Ok(spec) = std::env::var("PROBE_DUMP") {
            let (at, file) = spec.split_once(',').unwrap();
            if n == at.parse::<u64>().unwrap() {
                let all: Vec<u8> = (0..=0xffffu16).map(|a| mb.read_byte(a)).collect();
                std::fs::write(file, all).unwrap();
                println!(
                    "dumped at n={n} pc={pc:04X} sp={:04X}",
                    mb.cpu().reg16(Reg16::Sp).value()
                );
            }
        }
        let bytes: Vec<String> = (0..4)
            .map(|i| format!("{:02X}", mb.read_byte(pc.wrapping_add(i))))
            .collect();
        let cpu = mb.cpu();
        ring.push_back(format!(
            "{pc:04X}: {}  AF={:04X} BC={:04X} DE={:04X} HL={:04X} SP={:04X} IX={:04X}",
            bytes.join(" "),
            cpu.reg16(Reg16::Af).value(),
            cpu.reg16(Reg16::Bc).value(),
            cpu.reg16(Reg16::De).value(),
            cpu.reg16(Reg16::Hl).value(),
            cpu.reg16(Reg16::Sp).value(),
            cpu.reg16(Reg16::Ix).value()
        ));
        if ring.len() > ring_len {
            ring.pop_front();
        }
        mb.execute_next_opcode_no_video();
    }
    println!("not reached; pc={:04X}", mb.pc());
}
