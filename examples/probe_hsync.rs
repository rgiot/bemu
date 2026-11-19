//! `probe_hsync <script.csl> [pulses]`: plays the script, then lists the C-HSYNC pulses the Gate Array
//! sends to the monitor (start tick, width in 1/16 usec, gap since the previous pulse's start), to see
//! which sync pulses a CRTC type really makes of a test (Shaker's R2/R3 updates during the HSYNC).
use amsrust::csl_runner::CslRunner;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("script");
    let count: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(40);
    let mut runner = CslRunner::new("/tmp/probe_hsync_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let (mut tick, mut start, mut last_start, mut shown) = (0u64, None::<u64>, 0u64, 0usize);
    let mut was = false;
    while shown < count {
        for (_, sync) in mb.execute_next_opcode() {
            let on = sync.hsync != sync.vsync;
            if on && !was {
                start = Some(tick);
            }
            if !on && was {
                if let Some(s) = start.take() {
                    println!(
                        "pulse at {:7} (line phase {:4}) width {:4} ticks ({:.2} us), {:5} after the previous start",
                        s,
                        s % 1024,
                        tick - s,
                        (tick - s) as f64 / 16.0,
                        s - last_start
                    );
                    last_start = s;
                    shown += 1;
                }
            }
            was = on;
            tick += 1;
        }
    }
}
