//! Lockstep frames with another emulator: after a CSL script (typically a `snapshot_load`), the
//! last whole frame the monitor displayed at each of the next `<hits>` times the PC reaches `<pc>`
//! (code that runs once per frame), saved as `<prefix>_NN.png`.
//! `probe_frame_at_pc <script.csl> <pc_hex> <hits> <prefix>`
use amsrust::csl_runner::CslRunner;
use amsrust::monitor::Monitor;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("script");
    let pc = u16::from_str_radix(&a.next().expect("pc"), 16).unwrap();
    let hits: usize = a.next().expect("hits").parse().unwrap();
    let prefix = a.next().expect("prefix");
    let mut runner = CslRunner::new("/tmp/probe_frame_at_pc_shots");
    let text = std::fs::read_to_string(&path).unwrap();
    let base = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    runner.run_text(&text, &base).expect("csl");
    let mb = runner.motherboard_mut();
    let mut monitor = Monitor::default();
    let mut last = None;
    let mut n = 0;
    let mut previous_pc = mb.pc();
    while n < hits {
        mb.execute_next_opcode_with_video(|(rgb, sync)| monitor.receive(rgb, sync));
        if let Some(frame) = monitor.take_frame() {
            last = Some(frame);
        }
        let now = mb.pc();
        if now == pc && previous_pc != pc {
            if let Some(frame) = &last {
                frame.save(format!("{prefix}_{n:02}.png")).unwrap();
            }
            n += 1;
        }
        previous_pc = now;
    }
}
