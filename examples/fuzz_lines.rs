//! Per CRTC line summary of a snapshot's run, to diff against CPCSyntaxError
//! (validation/fuzz). `fuzz_lines <file.sna> [microseconds] [crtc]` prints, from
//! the first VSYNC start, one line per CRTC line: `C4 C9 V len`.
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let sna = a.next().expect("usage: <file.sna> [us] [crtc]");
    let us: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(80_000);
    let crtc: u8 = a.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut runner = CslRunner::new("/tmp/fuzz_lines_shots");
    runner
        .run_text(
            &format!("csl_version 1.0\ncrtc_select {crtc}\nreset\nwait 1000000\n"),
            ".",
        )
        .unwrap();
    amsrust::snapshot::load_sna(runner.motherboard_mut(), &sna).expect("load sna");
    let mb = runner.motherboard_mut();
    mb.crtc_mut().start_write_log();
    let mut last_ends = mb.crtc().line_ends;
    let mut started = false;
    let mut prev_vs = mb.crtc().vsync();
    let mut line_start = 0usize;
    let mut cur: Option<(u8, u8, bool)> = None;
    for t in 0..us * 16 {
        mb.tick();
        let c = mb.crtc();
        let vs = c.vsync();
        if !started && vs && !prev_vs {
            started = true;
            line_start = t;
            cur = Some((c.vcc.value(), c.c9(), vs));
            last_ends = c.line_ends;
        }
        prev_vs = vs;
        if started {
            if c.line_ends != last_ends {
                if let Some((c4, c9, v)) = cur {
                    println!("{c4} {c9} {} {}", u8::from(v), (t - line_start) / 16);
                }
                line_start = t;
                cur = Some((c.vcc.value(), c.c9(), vs));
            }
            last_ends = c.line_ends;
        }
    }
    // the register writes (C0 C4 C9 reg value), on stderr, to compare with CPCSE_TRACE_REG
    for (c0, c4, c9, r, v) in mb.crtc_mut().take_write_log() {
        eprintln!("WRITE R{r}<-{v} C0={c0} C4={c4} C9={c9}");
    }
}
