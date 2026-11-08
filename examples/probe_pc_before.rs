//! After a CSL script: the PCs executed (once each, in order) in the `<count>` instructions before
//! the `<n>`th time the PC reaches `<pc>`, with the time in us.tick since the script's end.
//! `probe_pc_before <script.csl> <pc_hex> <n> <count>`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let pc = u16::from_str_radix(&a.next().expect("pc"), 16).unwrap();
    let n: usize = a.next().unwrap().parse().unwrap();
    let count: usize = a.next().unwrap().parse().unwrap();
    let mut runner = CslRunner::new("/tmp/probe_pc_before_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut history: std::collections::VecDeque<(u64, u16)> = Default::default();
    let mut hits = 0;
    let start = mb.t_state_counter().value() as u64;
    loop {
        let now = mb.pc();
        let t = mb.t_state_counter().value() as u64 - start;
        if now == pc {
            hits += 1;
            if hits == n {
                for (t, p) in &history {
                    println!("t={:8} (T) PC={:04X}", t, p);
                }
                println!("t={:8} (T) PC={:04X}  <- hit", t, now);
                return;
            }
        }
        history.push_back((t, now));
        if history.len() > count {
            history.pop_front();
        }
        mb.execute_next_opcode_no_video();
    }
}
