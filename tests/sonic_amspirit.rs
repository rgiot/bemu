//! Sonic the Hedgehog (2025, GX4000; validation/games/sonic): the ring route of `rings.csl`, which
//! replays the joystick at the emulated times AMSpiriT Lite 1.16.0 played it, must give AMSpiriT's
//! pictures (`amspirit_1.16.0/`): the scroll, Sonic, the enemies, the rings he takes and the ones
//! he loses on the spikes, drawn by hardware sprites moved while the beam is lower (multiplexing,
//! see `examples/probe_sprite_mux.rs`).
//!
//! What still differs is a palette write landing about a microsecond apart (a few pixels of one
//! line: the sky's gradient at the left edge, a pen inside Sonic or the ring counter), so a shot
//! may differ by a few dozen pixels.
//!
//! Slow (plays 40 s of emulated time): `cargo test --release --test sonic_amspirit -- --ignored`.

use amsrust::csl_runner::CslRunner;
use image::RgbImage;

/// The palette jitter above: never more than 48 pixels in the shots kept.
const TOLERATED: usize = 64;

/// Pixels that differ at the CPC+'s 16 levels per channel. AMSpiriT's shots are 542 lines (each
/// line twice), from level 5 to 245, one pixel to the left of ours.
fn differing_pixels(amspirit: &RgbImage, ours: &RgbImage) -> usize {
    let level = |v: u8| (v as f64 / 17.0).round() as i32;
    let amspirit_level = |v: u8| ((v as f64 - 5.0) / 240.0 * 15.0).round().clamp(0.0, 15.0) as i32;
    let mut count = 0;
    for y in 0..271 {
        for x in 0..767 {
            let (a, b) = (ours.get_pixel(x + 1, y), amspirit.get_pixel(x, y * 2));
            if (0..3).any(|c| level(a[c]) != amspirit_level(b[c])) {
                count += 1;
            }
        }
    }
    count
}

#[test]
#[ignore = "slow: plays the game"]
fn the_ring_route_matches_amspirit() {
    let base = std::path::Path::new("validation/games/sonic");
    let script = std::fs::read_to_string(base.join("rings.csl")).expect("rings.csl");
    let dir = std::env::temp_dir().join("amsrust_sonic_rings");
    let mut runner = CslRunner::new(&dir);
    runner.run_text(&script, base).expect("the CSL runs");

    let mut references: Vec<_> = std::fs::read_dir(base.join("amspirit_1.16.0"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    references.sort();
    assert!(references.len() >= 10);
    let mut worst = Vec::new();
    for reference in references {
        let name = reference.file_name().unwrap();
        let ours = image::open(dir.join(name)).expect("our shot").to_rgb8();
        let amspirit = image::open(&reference).unwrap().to_rgb8();
        let differ = differing_pixels(&amspirit, &ours);
        println!("{}: {differ} pixels differ", name.to_string_lossy());
        if differ > TOLERATED {
            worst.push((name.to_string_lossy().into_owned(), differ));
        }
    }
    assert!(worst.is_empty(), "shots unlike AMSpiriT's: {worst:?}");
}
