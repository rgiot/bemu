//! After a CSL script, for the next `<ms>` ms: each time the gate array pulls /INT of the Z80 low, print the
//! tick (1/16 us) of the microsecond it falls in, the PC and the Z80 state, and the delay in ticks until the
//! interrupt response reaches PC 0x38/0x39: `probe_int_phase <script.csl> <ms>`
use amsrust::chip::Chip;
use amsrust::csl_runner::CslRunner;
use amsrust::z80::Z80Pin;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let ms: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    let mut runner = CslRunner::new("/tmp/probe_int_phase_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut t = 0u64;
    let mut us = mb.fdc().time_us();
    let mut phase = 0u64;
    let mut prev_int = mb.cpu().get_pin(Z80Pin::Neg_INT);
    let mut asserted: Option<(u64, u64, u16, u64)> = None;
    let mut last_pc = mb.pc();
    let mut pc_since = 0u64;
    while t < ms * 16000 {
        mb.tick();
        t += 1;
        let now = mb.fdc().time_us();
        if now != us { us = now; phase = 0; } else { phase += 1; }
        if mb.pc() != last_pc { last_pc = mb.pc(); pc_since = t; }
        let int = mb.cpu().get_pin(Z80Pin::Neg_INT);
        if prev_int && !int {
            asserted = Some((t, phase, mb.pc(), (t - pc_since) / 16));
        }
        prev_int = int;
        let pc = mb.pc();
        if (pc == 0x38 || pc == 0x39) && asserted.is_some() {
            let (t0, ph, pc0, idle) = asserted.take().unwrap();
            println!("INT low at phase {ph:2} (pc {pc0:04X}, pc unchanged for {idle} us) -> {:3} ticks until PC={pc:04X}", t - t0);
        }
    }
}
