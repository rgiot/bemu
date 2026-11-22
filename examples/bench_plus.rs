//! Emulation speed of a CPC+ running a cartridge, against a CPC, per emulated second, with the
//! audio and the video (frames built by the monitor) as a page has them:
//! `cargo run --release --example bench_plus -- [game.cpr] [seconds] [model]`
//! (model: 6128plus by default; 6128 to compare with a CPC, which takes no cartridge).
//! With `PLAY=1` a few keys are pressed from the 3rd second, as a player's; `KEYS=3:Return,5:Space`
//! taps the keys named (`CpcKey` variants) at the seconds given instead; `SHOT=file.png` saves the
//! last picture, `SHOTS=dir[:from_second]` one every 10 frames from then on. `HASH=1` prints a
//! hash of every picture and every sound sample made: it must not change when the emulator is made
//! faster (the same run, the same hash).

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
    let hashing = std::env::var_os("HASH").is_some();
    // NOVIDEO=1: no picture (a frame nobody sees), what a page does above real time or behind it
    let no_video = std::env::var_os("NOVIDEO").is_some();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut mix = |bytes: &[u8]| {
        for byte in bytes {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut audio_hash = 0xcbf2_9ce4_8422_2325_u64;
    // a player's keys: from the 3rd second, one of these pressed for a tenth of a second every
    // fifth of a second (a game needs a key to start, then moves)
    let keys = [CpcKey::Space, CpcKey::Return, CpcKey::CursorLeft, CpcKey::CursorRight, CpcKey::CursorDown, CpcKey::CursorUp, CpcKey::Joy0Left, CpcKey::Joy0Right, CpcKey::Joy0Fire1, CpcKey::Z, CpcKey::X];
    let mut pressed: Option<CpcKey> = None;
    let shots = std::env::var("SHOTS").ok().map(|spec| {
        let (dir, from) = spec.split_once(':').unwrap_or((&spec, "0"));
        (std::path::PathBuf::from(dir), from.parse::<usize>().unwrap_or(0) * 50)
    });
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
            // a key is down while any of its taps is on (a key tapped several times has several entries)
            for (_, key, _) in &scripted {
                let down = scripted.iter().any(|(at, other, held)| {
                    other == key && (at * 4_000_000..at * 4_000_000 + held * 400_000).contains(&tick)
                });
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
            if no_video {
                motherboard.set_video(false);
                motherboard.execute_next_opcode_no_video();
            } else {
            motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                monitor.receive(rgb, sync);
                if let Some(frame) = monitor.take_frame() {
                    frames += 1;
                    if hashing {
                        mix(frame.as_raw());
                    }
                    if let Some((dir, from)) = &shots {
                        if frames >= *from && frames % 10 == 0 {
                            frame.save(dir.join(format!("f{frames:05}.png"))).expect("a picture");
                        }
                    }
                    if let Some(old) = last.replace(frame) {
                        monitor.recycle(old);
                    }
                }
            });
            }
            if motherboard.psg().pending_samples() > 4096 {
                let samples = motherboard.take_audio_samples();
                if hashing {
                    for sample in samples.iter() {
                        for byte in sample.to_le_bytes() {
                            audio_hash = (audio_hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
                        }
                    }
                }
            }
        }
        let wall = start.elapsed().as_secs_f64();
        println!("second {second}: {wall:.3}s wall ({:.1}x real time)", 1.0 / wall);
    }
    let wall = total.elapsed().as_secs_f64();
    if let (Some(frame), Some(path)) = (&last, std::env::var_os("SHOT")) {
        frame.save(path).expect("the picture");
    }
    if hashing {
        // the 64K the CPU sees as RAM, as it is at the end: what the emulation did, picture or not
        let mut memory = 0xcbf2_9ce4_8422_2325_u64;
        for address in 0..=0xffff_u32 {
            let byte = motherboard.read_bank_byte((address / 0x4000) as u16, (address & 0x3fff) as u16).unwrap_or(0);
            memory = (memory ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        println!("hash pictures {hash:016x} sound {audio_hash:016x} ram {memory:016x}");
    }
    println!("{seconds} s in {wall:.2} s: {:.1}x real time, {frames} frames", seconds as f64 / wall);
}
