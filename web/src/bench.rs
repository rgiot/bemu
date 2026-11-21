//! Emulation speed in wasm, without a page: `bench(sna, "6128plus", seconds)` plays a snapshot
//! with the monitor building the frames, as the page does, and returns the frames made.
//! `bench_cartridge(cpr, "6128plus", seconds)` does the same with a CPC+ cartridge.

use amsrust::monitor::Monitor;
use amsrust::motherboard::{CpcModel, MotherboardBuilder};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn bench(sna: &[u8], model: &str, seconds: usize) -> Result<usize, JsError> {
    let mut builder = MotherboardBuilder::default();
    builder.enable_interruptions();
    builder.set_model(model.parse::<CpcModel>().map_err(|e| JsError::new(&e))?);
    let mut motherboard = builder.build();
    amsrust::snapshot::load_sna_bytes(&mut motherboard, sna).map_err(|e| JsError::new(&e))?;
    let mut monitor = Monitor::default();
    let mut frames = 0;
    let end = seconds * 4_000_000;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_with_video(|(rgb, sync)| monitor.receive(rgb, sync));
        if let Some(frame) = monitor.take_frame() {
            frames += 1;
            monitor.recycle(frame);
        }
        if motherboard.psg().pending_samples() > 4096 {
            motherboard.take_audio_samples();
        }
    }
    Ok(frames)
}

/// A CPC+ running the cartridge `cpr` for `seconds` emulated seconds, with the audio and the
/// monitor's frames, as the page has them. Returns the frames made.
#[wasm_bindgen]
pub fn bench_cartridge(cpr: &[u8], model: &str, seconds: usize) -> Result<usize, JsError> {
    let mut builder = MotherboardBuilder::default();
    builder.enable_interruptions();
    builder.set_model(model.parse::<CpcModel>().map_err(|e| JsError::new(&e))?);
    let mut motherboard = builder.build();
    motherboard.set_audio_output(Some(amsrust::psg::AudioConfig::stereo(48_000)));
    let cartridge = amsrust::asic::Cartridge::from_bytes(cpr, "cartridge", None).map_err(|e| JsError::new(&e))?;
    motherboard.insert_cartridge(Some(cartridge));
    motherboard.restart();
    let mut monitor = Monitor::default();
    let mut frames = 0;
    let end = seconds * 4_000_000;
    while motherboard.t_state_counter().value() < end {
        motherboard.execute_next_opcode_with_video(|(rgb, sync)| monitor.receive(rgb, sync));
        if let Some(frame) = monitor.take_frame() {
            frames += 1;
            monitor.recycle(frame);
        }
        if motherboard.psg().pending_samples() > 4096 {
            motherboard.take_audio_samples();
        }
    }
    Ok(frames)
}
