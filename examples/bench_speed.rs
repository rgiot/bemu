//! Emulation speed of a booting CPC (firmware), per emulated second, with
//! or without PSG audio output and video (frames built by the monitor, as
//! the window does):
//! `cargo run --release --example bench_speed -- [seconds] [audio] [video]`

use amsrust::monitor::Monitor;
use amsrust::motherboard::MotherboardBuilder;
use amsrust::psg::AudioConfig;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let seconds: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(4);
    let options: Vec<String> = args.collect();
    let audio = options.iter().any(|o| o == "audio").then(|| AudioConfig::stereo(48_000));
    let video = options.iter().any(|o| o == "video");
    let mut monitor = Monitor::default();
    let mut frames = 0usize;

    let start = Instant::now();
    let mut motherboard = MotherboardBuilder::default().enable_interruptions().build();
    motherboard.set_audio_output(audio);
    println!("build: {:.2}s", start.elapsed().as_secs_f64());
    for second in 0..seconds {
        let start = Instant::now();
        let end = (second + 1) * 4_000_000;
        while motherboard.t_state_counter().value() < end {
            if video {
                motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                    monitor.receive(rgb, sync);
                    frames += monitor.take_frame().is_some() as usize;
                });
            }
            else {
                motherboard.execute_next_opcode_no_video();
            }
            if motherboard.psg().pending_samples() > 4096 {
                motherboard.take_audio_samples();
            }
        }
        let wall = start.elapsed().as_secs_f64();
        println!("second {second}: {wall:.2}s wall ({:.2}x real time)", 1.0 / wall);
    }
    if video {
        println!("{frames} frames");
    }
}
