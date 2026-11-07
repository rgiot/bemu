//! After a CSL script: one frame from a VSYNC start, every change of the Gate Array's pending and
//! active video mode and every edge of the CRTC HSYNC and of the GA's HSYNC, with the time since
//! the VSYNC (us.tick), the 64 us line it falls in and C0.
//! `probe_mode_events <script.csl> [<from_line> <to_line>]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let from: u32 = a.next().map_or(0, |v| v.parse().unwrap());
    let to: u32 = a.next().map_or(320, |v| v.parse().unwrap());
    let mut runner = CslRunner::new("/tmp/probe_mode_events_shots");
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
    let mut tick = 0u32;
    let (mut next, mut active) = (mb.ga().next_video_mode(), mb.ga().active_video_mode());
    let (mut hs, mut hs_ga) = (mb.crtc().hsync(), mb.ga().hsync_ga());
    while tick < 312 * 64 * 16 {
        mb.tick();
        tick += 1;
        let line = tick / (64 * 16);
        if line < from || line >= to {
            continue;
        }
        let at = format!(
            "{:5}.{:02} L{:3} C0={:2}",
            tick / 16,
            tick % 16,
            line,
            mb.crtc().hcc
        );
        if mb.ga().next_video_mode() != next {
            next = mb.ga().next_video_mode();
            println!("{at} next   {next:?}");
        }
        if mb.ga().active_video_mode() != active {
            active = mb.ga().active_video_mode();
            println!("{at} active {active:?}");
        }
        if mb.crtc().hsync() != hs {
            hs = mb.crtc().hsync();
            println!("{at} HSYNC  {}", if hs { "on" } else { "off" });
        }
        if mb.ga().hsync_ga() != hs_ga {
            hs_ga = mb.ga().hsync_ga();
            println!("{at} HS-GA  {}", if hs_ga { "on" } else { "off" });
        }
    }
}
