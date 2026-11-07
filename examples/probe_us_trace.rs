//! After a CSL script, from the next CRTC VSYNC start, one line per microsecond (C0 change):
//! the time since the VSYNC, C0/C4/C9, MA, DE, HSYNC, VSYNC and R0/R1/R2/R4/R9/R5/R6/R7 when they change.
//! `probe_us_trace <script.csl> <microseconds>` (PROBE_EVERY_US=1: every microsecond, for R0=0 lines)
use amsrust::chip::Chip;
use amsrust::crtc::CrtcPin;
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let count: u64 = a.next().unwrap().parse().unwrap();
    let mut runner = CslRunner::new("/tmp/probe_us_trace_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut prev_vs = mb.crtc().vsync();
    loop {
        mb.tick();
        let vs = mb.crtc().vsync();
        if vs && !prev_vs {
            break;
        }
        prev_vs = vs;
    }
    let (mut tick, mut us) = (0u64, 0u64);
    let mut last_c0 = 255u8;
    let mut last_regs = [255u8; 8];
    let every_us = std::env::var("PROBE_EVERY_US").is_ok();
    while us < count {
        mb.tick();
        tick += 1;
        let c = mb.crtc();
        if (every_us && tick % 16 == 0) || (!every_us && c.hcc != last_c0) {
            last_c0 = c.hcc;
            us += 1;
            let regs = [
                c.regs_r(0),
                c.regs_r(1),
                c.regs_r(2),
                c.regs_r(4),
                c.regs_r(9),
                c.regs_r(5),
                c.regs_r(6),
                c.regs_r(7),
            ];
            let r = if regs != last_regs {
                format!(
                    " R0={} R1={} R2={} R4={} R9={} R5={} R6={} R7={}",
                    regs[0], regs[1], regs[2], regs[3], regs[4], regs[5], regs[6], regs[7]
                )
            } else {
                String::new()
            };
            last_regs = regs;
            println!(
                "{:7.2} C0={:2} C4={:2} C9={:2} MA={:04X} DE={} HS={} VS={}{r}",
                tick as f64 / 16.0,
                c.hcc,
                c.vcc.value(),
                c.vlc.value(),
                c.vma(),
                c.get_pin(CrtcPin::DE) as u8,
                c.get_pin(CrtcPin::HS) as u8,
                c.vsync() as u8
            );
        }
    }
}
