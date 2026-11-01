//! After a CSL script: around each /INT fall, print every tick: phase in the us, Z80 CLK/WAIT/INT pins and state.
use amsrust::chip::Chip;
use amsrust::csl_runner::CslRunner;
use amsrust::z80::Z80Pin;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let events: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut runner = CslRunner::new("/tmp/probe_z80_ticks_shots");
    runner.run_text("csl_version 1.0\ncrtc_select 1\nreset\nwait 1000000\n", ".").unwrap();
    amsrust::snapshot::load_sna(runner.motherboard_mut(), &path).expect("load sna");
    runner.run_text("csl_version 1.0\nwait 300000\n", ".").unwrap();
    let mb = runner.motherboard_mut();
    let mut us = mb.fdc().time_us();
    let mut phase = 0u64;
    let mut prev_int = mb.cpu().get_pin(Z80Pin::Neg_INT);
    let mut left = 0u32;
    let mut ring: std::collections::VecDeque<String> = Default::default();
    let mut seen = 0;
    loop {
        mb.tick();
        let now = mb.fdc().time_us();
        if now != us { us = now; phase = 0; } else { phase += 1; }
        let c = mb.cpu();
        let int = c.get_pin(Z80Pin::Neg_INT);
        let line = format!("us={us} ph={phase:2} CLK={} WAIT={} INT={} pc={:04X} st={:?}", u8::from(c.get_pin(Z80Pin::Neg_CLK)), u8::from(c.get_pin(Z80Pin::Neg_WAIT)), u8::from(int), mb.pc(), c.next_state());
        ring.push_back(line); if ring.len() > 24 { ring.pop_front(); }
        if prev_int && !int { left = 30; seen += 1; for l in &ring { println!("{l}"); } println!("---- INT falls"); }
        prev_int = int;
        if left > 0 {
            if false {
                println!("us={us} ph={phase:2} CLK={} WAIT={} INT={} pc={:04X} st={:?}", u8::from(c.get_pin(Z80Pin::Neg_CLK)), u8::from(c.get_pin(Z80Pin::Neg_WAIT)), u8::from(int), mb.pc(), c.next_state());
            }
            left -= 1;
            println!("{}", ring.back().unwrap());
            if left == 0 && seen >= events { break; }
        }
    }
}
