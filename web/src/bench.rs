//! Emulation speed in wasm, without a page: `bench(sna, "6128plus", seconds)` plays a snapshot
//! with the monitor building the frames, as the page does, and returns the frames made.

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
