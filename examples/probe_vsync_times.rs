//! After a CSL script, the times (us) of the CRTC's VSYNC starts, the Gate Array's C-VSYNC starts and
//! the monitor's frame starts for the next `<frames>` frames.
//! `probe_vsync_times <script.csl> [<frames>]`
use amsrust::csl_runner::CslRunner;
use amsrust::monitor::Monitor;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let frames: usize = a.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let mut runner = CslRunner::new("/tmp/probe_vsync_times_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut monitor = Monitor::default();
    let (mut tick, mut vs_prev, mut cvs_prev) = (0u64, false, false);
    let (mut last_vs, mut last_cvs, mut last_frame) = (0u64, 0u64, 0u64);
    let mut seen = 0;
    while seen < frames {
        let (rgb, sync) = mb.tick();
        monitor.receive(rgb, sync);
        tick += 1;
        let vs = mb.crtc().vsync();
        if vs && !vs_prev {
            let regs: Vec<u8> = (0..10).map(|r| mb.crtc().regs_r(r)).collect();
            println!("{:>10.1} us CRTC VSYNC   (+{:.1})  C4={:?} C9={} R0-R9={:?}", tick as f64 / 16.0, (tick - last_vs) as f64 / 16.0, mb.crtc().vcc, mb.crtc().vlc.value(), regs);
            last_vs = tick;
        }
        if sync.vsync && !cvs_prev {
            println!("{:>10.1} us C-VSYNC      (+{:.1})", tick as f64 / 16.0, (tick - last_cvs) as f64 / 16.0);
            last_cvs = tick;
        }
        vs_prev = vs;
        cvs_prev = sync.vsync;
        if monitor.take_frame().is_some() {
            println!("{:>10.1} us monitor frame (+{:.1})", tick as f64 / 16.0, (tick - last_frame) as f64 / 16.0);
            last_frame = tick;
            seen += 1;
        }
    }
}
