//! Cost of the PSG with and without audio output: `bench_audio [emulated seconds]`
use amsrust::psg::{AudioConfig, Psg};
use std::time::Instant;

fn run(audio: bool, seconds: u64) -> f64 {
    let mut psg = Psg::default();
    if audio {
        psg.set_audio_output(Some(AudioConfig::stereo(48_000)));
    }
    for (reg, value) in [(7u8, 0b111000u8), (0, 125), (2, 90), (4, 60), (8, 15), (9, 12), (10, 10)] {
        psg.set_register(reg, value);
    }
    let start = Instant::now();
    let mut sink = 0usize;
    for _ in 0..seconds {
        for _ in 0..1_000_000 {
            psg.clock();
        }
        sink += psg.take_samples().len();
    }
    std::hint::black_box(sink);
    start.elapsed().as_secs_f64() / seconds as f64
}

fn main() {
    let seconds = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let without = run(false, seconds);
    let with = run(true, seconds);
    println!("per emulated second: psg alone {:.2} ms, with audio {:.2} ms (audio {:.2} ms = {:.1}% of a 20ms frame budget x50)", without * 1e3, with * 1e3, (with - without) * 1e3, (with - without) * 1e3 / 1000.0 * 100.0);
}
