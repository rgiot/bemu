//! `probe_hsync_lines <script.csl|file.sna> <frames_to_print>`: plays the script, then prints for the
//! next frames, per CRTC line, the CRTC HSYNC pulses (C0 start..end, in microseconds from the line
//! start) as `C4/C9 R2 R3 [start-end ...]`, only for lines with more than one pulse or odd ones.
use amsrust::chip::Chip;
use amsrust::crtc::CrtcPin;
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let script = a.next().expect("script");
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut runner = CslRunner::new("/tmp/probe_hsync_shots");
    if script.ends_with(".sna") {
        runner.run_text("csl_version 1.0\ncrtc_select 0\nreset\nwait 1000000\n", ".").unwrap();
        amsrust::snapshot::load_sna(runner.motherboard_mut(), &script).expect("load sna");
        runner.run_text("csl_version 1.0\nwait 100000\n", ".").unwrap();
    } else {
        runner.run_text(&std::fs::read_to_string(&script).unwrap(), ".").unwrap();
    }
    let mb = runner.motherboard_mut();
    let mut vs_rises = 0;
    let mut prev_vs = mb.crtc().vsync();
    let mut last_ends = mb.crtc().line_ends;
    let mut pulses: Vec<(u32, u32)> = vec![];
    let mut cur_start: Option<u32> = None;
    let mut prev_hs = false;
    let mut prev_mode = format!("{:?}", mb.ga().active_video_mode());
    let mut mode_changes: Vec<String> = vec![];
    let mut rgbs: Vec<String> = vec![];
    let dump = std::env::var("RGB").is_ok();
    let mut t_in_line = 0u32;
    while vs_rises <= n {
        mb.tick();
        let c = mb.crtc();
        let vs = c.vsync();
        if vs && !prev_vs { vs_rises += 1; }
        prev_vs = vs;
        let hs = if std::env::var("GA").is_ok() { mb.ga().hsync_ga() } else if std::env::var("DE").is_ok() { !c.get_pin(CrtcPin::DE) } else { c.hsync() };
        if hs && !prev_hs { cur_start = Some(t_in_line); }
        if !hs && prev_hs { if let Some(s) = cur_start.take() { pulses.push((s, t_in_line)); } }
        prev_hs = hs;
        let m = format!("{:?}", mb.ga().active_video_mode());
        if m != prev_mode { mode_changes.push(format!("{}.{}:{}", t_in_line / 16, t_in_line % 16, m)); prev_mode = m; }
        if dump && t_in_line % 2 == 0 && t_in_line >= 0 && t_in_line < 64 * 16 {
            let r = mb.ga().rgb();
            rgbs.push(format!("{}:{:.0}/{:.0}/{:.0}", t_in_line / 2, r.r(), r.g(), r.b()));
        }
        t_in_line += 1;
        if c.line_ends != last_ends {
            last_ends = c.line_ends;
            if vs_rises >= 1 && !pulses.is_empty() {
                if dump { let pal: Vec<String> = mb.ga().palette().iter().take(4).map(|i| format!("{:?}", i)).collect(); print!("pal={:?} mode={:?} ", pal, mb.ga().active_video_mode()); let mut runs: Vec<String> = vec![]; let mut last = String::new(); for r in &rgbs { let (t, v) = r.split_once(':').unwrap(); if v != last { runs.push(format!("{}={}", t, v)); last = v.to_string(); } } println!("{}/{} {}", c.vcc.value(), c.c9(), runs.join(" ")); } else { println!("{}/{} R2={} R3={} [{}] {}", c.vcc.value(), c.c9(), c.regs_r(2), c.regs_r(3), mode_changes.join(" "),
                    pulses.iter().map(|(s, e)| format!("{}.{}-{}.{}", s / 16, s % 16, e / 16, e % 16)).collect::<Vec<_>>().join(" ")); }
            }
            pulses.clear();
            mode_changes.clear();
            rgbs.clear();
            t_in_line = 0;
            cur_start = None;
        }
    }
}
