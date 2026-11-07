//! After a CSL script, run until CRTC register <reg> takes `value`, then print the last
//! PCs executed. `probe_r4 <script.csl> <value> [max_instr] [reg]`
use amsrust::csl_runner::CslRunner;
use std::collections::VecDeque;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let value: u8 = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let max: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(100_000_000);
    let reg: u8 = a.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    let get = |c: &amsrust::crtc::Crtc| match reg {
        0 => c.r0(),
        1 => c.r1(),
        2 => c.r2(),
        3 => c.r3(),
        4 => c.r4(),
        5 => c.r5(),
        6 => c.r6(),
        7 => c.r7(),
        _ => c.r9(),
    };
    let mut runner = CslRunner::new("/tmp/probe_r4_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut last: VecDeque<u16> = VecDeque::new();
    let mut count = 0u64;
    let before = get(mb.crtc());
    let mut prev = before;
    while count < max {
        let pc = mb.pc();
        mb.execute_next_opcode_no_video();
        count += 1;
        last.push_back(pc);
        if last.len() > 40 {
            last.pop_front();
        }
        let now = get(mb.crtc());
        if now != prev {
            println!(
                "R{reg} {prev} -> {now} after {count} instr, t={}",
                mb.t_state_counter().value()
            );
            println!(
                "last PCs: {}",
                last.iter()
                    .map(|p| format!("{p:04X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let lastpc = *last.back().unwrap();
            let start = (lastpc & 0xfff0).wrapping_sub(0x50);
            for row in 0..14u16 {
                let base = start.wrapping_add(row * 16);
                let bytes: Vec<String> = (0..16)
                    .map(|i| format!("{:02X}", mb.read_byte(base.wrapping_add(i))))
                    .collect();
                println!("{base:04X}: {}", bytes.join(" "));
            }
            if now == value {
                break;
            }
            prev = now;
        }
    }
}
