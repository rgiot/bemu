//! After a CSL script, save the monitor's whole raster (sync and borders included) of the next
//! `<n>` frames: `probe_full_frames <script.csl> <n> <out prefix>`
use amsrust::csl_runner::CslRunner;
use amsrust::monitor::Monitor;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let n: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    let prefix = a.next().unwrap_or_else(|| "/tmp/full".into());
    let mut runner = CslRunner::new("/tmp/probe_full_frames_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut monitor = Monitor::default();
    monitor.keep_full_frames();
    let mut saved = 0;
    while saved < n {
        mb.execute_next_opcode_with_video(|(rgb, sync)| monitor.receive(rgb, sync));
        if monitor.take_frame().is_some() {
            let c = mb.crtc();
            println!("frame done at C4={} C9={} C0={} vs={}", c.vcc.value(), c.vlc.value(), c.hcc, c.vsync());
            if let Some(f) = monitor.full_frame() {
                f.save(format!("{prefix}{saved}.png")).unwrap();
                saved += 1;
            }
        }
    }
}
