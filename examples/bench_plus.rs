//! Emulation speed of a CPC+ running a cartridge, against a CPC, per emulated second, with the
//! audio and the video (frames built by the monitor) as a page has them:
//! `cargo run --release --example bench_plus -- [game.cpr] [seconds] [model]`
//! (model: 6128plus by default; 6128 to compare with a CPC, which takes no cartridge).
//! With `PLAY=1` a few keys are pressed from the 3rd second, as a player's; `KEYS=3:Return,5:Space`
//! taps the keys named (`CpcKey` variants) at the seconds given instead; `SHOT=file.png` saves the
//! last picture.

use amsrust::asic::Cartridge;
use amsrust::keyboard::CpcKey;
use amsrust::monitor::Monitor;
use amsrust::motherboard::{CpcModel, MotherboardBuilder};
use amsrust::psg::AudioConfig;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let file = args.next().filter(|name| name != "-");
    let seconds: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let play = std::env::var_os("PLAY").is_some();
    let model: CpcModel = args
        .next()
        .map_or(Ok(CpcModel::Cpc6128Plus), |m| m.parse())
        .expect("a model");

    let mut builder = MotherboardBuilder::default();
    builder.enable_interruptions();
    builder.set_model(model);
    let mut motherboard = builder.build();
    motherboard.set_audio_output(Some(AudioConfig::stereo(48_000)));
    if let Some(file) = &file {
        let bytes = std::fs::read(file).expect("the cartridge");
        let cartridge = Cartridge::from_bytes(&bytes, file, None).expect("a cartridge");
        if model.is_plus() {
            motherboard.insert_cartridge(Some(cartridge));
            motherboard.restart();
        }
    }
    let mut monitor = Monitor::default();
    let mut frames = 0usize;
    let mut last = None;
    // a player's keys: from the 3rd second, one of these pressed for a tenth of a second every
    // fifth of a second (a game needs a key to start, then moves)
    let keys = [CpcKey::Space, CpcKey::Return, CpcKey::CursorLeft, CpcKey::CursorRight, CpcKey::CursorDown, CpcKey::CursorUp, CpcKey::Joy0Left, CpcKey::Joy0Right, CpcKey::Joy0Fire1, CpcKey::Z, CpcKey::X];
    let mut pressed: Option<CpcKey> = None;
    // KEYS=second:Key[:tenths of a second held],...: from that second, 0.1 s by default
    let scripted: Vec<(usize, CpcKey, usize)> = std::env::var("KEYS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|item| {
            let mut fields = item.split(':');
            let (second, name) = (fields.next()?, fields.next()?);
            let held = fields.next().and_then(|tenths| tenths.parse().ok()).unwrap_or(1);
            let key = (0..200).filter_map(amsrust::keyboard::key_at).find(|key| format!("{key:?}") == name)?;
            Some((second.parse().ok()?, key, held))
        })
        .collect();
    let mut step = 0usize;
    let total = Instant::now();
    for second in 0..seconds {
        let start = Instant::now();
        let end = (second + 1) * 4_000_000;
        while motherboard.t_state_counter().value() < end {
            let tick = motherboard.t_state_counter().value();
            for (at, key, held) in &scripted {
                let from = at * 4_000_000;
                let down = (from..from + held * 400_000).contains(&tick);
                if motherboard.keyboard().is_pressed(*key) != down {
                    motherboard.keyboard_mut().set_pressed(*key, down);
                }
            }
            // 800,000 T-states: a fifth of a second
            let slot = motherboard.t_state_counter().value() / 400_000;
            if play && second >= 3 && slot != step {
                step = slot;
                if let Some(key) = pressed.take() {
                    motherboard.keyboard_mut().set_pressed(key, false);
                }
                if slot % 2 == 0 {
                    let key = keys[(slot / 2).wrapping_mul(2654435761) % keys.len()];
                    motherboard.keyboard_mut().set_pressed(key, true);
                    pressed = Some(key);
                }
            }
            motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                monitor.receive(rgb, sync);
                if let Some(frame) = monitor.take_frame() {
                    frames += 1;
                    if let Some(old) = last.replace(frame) {
                        monitor.recycle(old);
                    }
                }
            });
            if motherboard.psg().pending_samples() > 4096 {
                motherboard.take_audio_samples();
            }
        }
        let wall = start.elapsed().as_secs_f64();
        println!("second {second}: {wall:.3}s wall ({:.1}x real time)", 1.0 / wall);
    }
    let wall = total.elapsed().as_secs_f64();
    if let (Some(frame), Some(path)) = (&last, std::env::var_os("SHOT")) {
        frame.save(path).expect("the picture");
    }
    println!("{seconds} s in {wall:.2} s: {:.1}x real time, {frames} frames", seconds as f64 / wall);
}
