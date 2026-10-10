//! After a CSL script, print every CRTC register write (C0, C4, C9, reg=value)
//! of the next `<frames>` VSYNC-to-VSYNC frames, collapsing identical frames.
//! `probe_frame_writes <script.csl> [<frames>]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let frames: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut runner = CslRunner::new("/tmp/probe_frame_writes_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut prev_vs = mb.crtc().vsync();
    let mut seen = 0;
    mb.crtc_mut().start_write_log();
    let mut line = 0usize;
    let mut last_hcc = 255u8;
    while seen <= frames {
        mb.tick();
        let vs = mb.crtc().vsync();
        if vs && !prev_vs {
            seen += 1;
            line = 0;
            println!("--- VSYNC {seen}");
        }
        prev_vs = vs;
        let hcc = mb.crtc().hcc;
        if hcc == 0 && last_hcc != 0 {
            line += 1;
        }
        last_hcc = hcc;
        for (c0, c4, c9, r, v) in mb.crtc_mut().take_write_log() {
            println!("line {line:3} C0={c0:2} C4={c4:2} C9={c9} R{r}={v}");
        }
    }
}
