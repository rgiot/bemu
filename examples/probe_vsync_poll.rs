//! After a CSL script: for `<frames>` frames, the time (us.tick) of each CRTC VSYNC start and of
//! the first instruction at `<pc>` after it (the code following a VSYNC poll), and their distance:
//! the phase a program's VSYNC synchronisation gives it against the CRTC.
//! `probe_vsync_poll <script.csl> <pc_hex> [<frames>]`
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let pc = u16::from_str_radix(&a.next().expect("pc"), 16).unwrap();
    let frames: usize = a.next().map_or(8, |v| v.parse().unwrap());
    let mut runner = CslRunner::new("/tmp/probe_vsync_poll_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut tick = 0u64;
    let mut prev_vs = mb.crtc().vsync();
    let mut vs_at: Option<u64> = None;
    let mut last_vs: Option<u64> = None;
    let mut seen = 0;
    while seen < frames {
        mb.tick();
        tick += 1;
        let vs = mb.crtc().vsync();
        if vs && !prev_vs {
            if let Some(last) = last_vs {
                print!(
                    "VSYNC +{:6}.{:02}  ",
                    (tick - last) / 16,
                    (tick - last) % 16
                );
            }
            last_vs = Some(tick);
            vs_at = Some(tick);
        }
        prev_vs = vs;
        if mb.is_aligned_with_new_instruction() && mb.pc() == pc {
            if let Some(at) = vs_at.take() {
                println!(
                    "code {:3}.{:02} us after the VSYNC (C0={})",
                    (tick - at) / 16,
                    (tick - at) % 16,
                    mb.crtc().hcc
                );
                seen += 1;
            }
        }
    }
}
