//! `probe_hsync_lines <script.csl> <frames_to_print>`: plays the script, then prints for the
//! next frames, per CRTC line, the CRTC HSYNC pulses (C0 start..end, in microseconds from the line
//! start) as `C4/C9 R2 R3 [start-end ...]`, only for lines with more than one pulse or odd ones.
use amsrust::chip::Chip;
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let script = a.next().expect("script");
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut runner = CslRunner::new("/tmp/probe_hsync_shots");
    runner.run_text(&std::fs::read_to_string(&script).unwrap(), ".").unwrap();
    let mb = runner.motherboard_mut();
    let mut vs_rises = 0;
    let mut prev_vs = mb.crtc().vsync();
    let mut last_ends = mb.crtc().line_ends;
    let mut pulses: Vec<(u32, u32)> = vec![];
    let mut cur_start: Option<u32> = None;
    let mut prev_hs = false;
    let mut t_in_line = 0u32;
    while vs_rises <= n {
        mb.tick();
        let c = mb.crtc();
        let vs = c.vsync();
        if vs && !prev_vs { vs_rises += 1; }
        prev_vs = vs;
        let hs = c.hsync();
        if hs && !prev_hs { cur_start = Some(t_in_line); }
        if !hs && prev_hs { if let Some(s) = cur_start.take() { pulses.push((s, t_in_line)); } }
        prev_hs = hs;
        t_in_line += 1;
        if c.line_ends != last_ends {
            last_ends = c.line_ends;
            if vs_rises >= 1 && !pulses.is_empty() {
                println!("{}/{} R2={} R3={} {}", c.vcc.value(), c.c9(), c.regs_r(2), c.regs_r(3),
                    pulses.iter().map(|(s, e)| format!("{}.{}-{}.{}", s / 16, s % 16, e / 16, e % 16)).collect::<Vec<_>>().join(" "));
            }
            pulses.clear();
            t_in_line = 0;
            cur_start = None;
        }
    }
}
