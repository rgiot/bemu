//! Frames without a picture, as a page makes them when it cannot keep real time: `hidden` frames
//! run without picture, then one is shown (`run_frame`), over and over, for `seconds` of emulated time.
//! `cargo run --release --example bench_frameskip -- game.cpr [seconds] [hidden] [out.png]`
//! Prints the wall time and saves the last picture shown (to see a picture torn by the skipping).

use amsrust::asic::Cartridge;
use amsrust::frame::{run_frame, FRAME_T_STATES};
use amsrust::monitor::Monitor;
use amsrust::motherboard::{CpcModel, MotherboardBuilder};
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let file = args.next().expect("a cartridge");
    let seconds: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let hidden: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let out = args.next();

    let mut builder = MotherboardBuilder::default();
    builder.enable_interruptions();
    builder.set_model(CpcModel::Cpc6128Plus);
    let mut motherboard = builder.build();
    motherboard.set_audio_output(Some(amsrust::psg::AudioConfig::stereo(48_000)));
    let bytes = std::fs::read(&file).expect("the cartridge");
    motherboard.insert_cartridge(Some(Cartridge::from_bytes(&bytes, &file, None).expect("a cartridge")));
    motherboard.restart();

    let mut monitor = Monitor::default();
    let (mut shown, mut skipped) = (0, 0);
    let mut last = None;
    let start = Instant::now();
    let end = seconds * 4_000_000;
    let mut run = 0;
    while motherboard.t_state_counter().value() < end {
        if run < hidden {
            run += 1;
            skipped += 1;
            motherboard.set_video(false);
            let until = motherboard.t_state_counter().value() + FRAME_T_STATES;
            while motherboard.t_state_counter().value() < until {
                motherboard.execute_next_opcode_no_video();
            }
            motherboard.set_video(true);
        } else {
            run = 0;
            if let Some(frame) = run_frame(&mut motherboard, &mut monitor, None, None, 2 * FRAME_T_STATES) {
                shown += 1;
                last = Some(frame);
            }
        }
        if motherboard.psg().pending_samples() > 4096 {
            motherboard.take_audio_samples();
        }
    }
    let wall = start.elapsed().as_secs_f64();
    println!(
        "{seconds} s emulated in {wall:.2} s: {:.2}x real time, {shown} pictures, {skipped} frames without",
        seconds as f64 / wall
    );
    if let (Some(frame), Some(path)) = (last, out) {
        frame.save(path).expect("the picture");
    }
}
