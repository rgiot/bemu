//! After a CSL script: one frame line by line - CRTC line number since VSYNC,
//! R2/R3, HSYNC width and every GA video mode change inside the line
//! (active and next), to see a demo's per-line mode switching.
//! `probe_modes_lines <script.csl> [lines]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let lines: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(320);
    let mut runner = CslRunner::new("/tmp/probe_modes_lines_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    // wait for a VSYNC start
    let mut prev_vs = mb.crtc().vsync();
    loop {
        mb.tick();
        let vs = mb.crtc().vsync();
        if vs && !prev_vs { break; }
        prev_vs = vs;
    }
    let mut line = 0usize;
    let mut last_hcc = 255u8;
    let mut desc = String::new();
    let mut hs_w = 0u32;
    let mut last = (mb.ga().next_video_mode(), mb.ga().active_video_mode());
    let mut tick_in_line = 0u32;
    let (mut prev_hs, mut prev_gs) = (false, false);
    while line < lines {
        mb.tick();
        tick_in_line += 1;
        let c = mb.crtc();
        if c.hsync() { hs_w += 1; }
        let hs_now = c.hsync();
        if hs_now != prev_hs { desc.push_str(&format!(" [{}: CRTC_HS={}]", tick_in_line, hs_now)); prev_hs = hs_now; }
        let gs_now = mb.ga().sync().hsync;
        if gs_now != prev_gs { desc.push_str(&format!(" [{}: C-HS={}]", tick_in_line, gs_now)); prev_gs = gs_now; }
        let now = (mb.ga().next_video_mode(), mb.ga().active_video_mode());
        if now != last {
            desc.push_str(&format!(" [{}: next={:?} active={:?}]", tick_in_line, now.0, now.1));
            last = now;
        }
        let hcc = mb.crtc().hcc;
        if hcc == 0 && last_hcc != 0 {
            let c = mb.crtc();
            println!("{line:3} R0={} R2={} R3={} hs={hs_w:3} C4={:?} C9={}{}", c.r0(), c.r2(), c.r3(), c.vcc, c.vlc.value(), desc);
            desc.clear(); hs_w = 0; tick_in_line = 0; line += 1;
        }
        last_hcc = hcc;
    }
}
