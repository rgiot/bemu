//! Rhino-Raster (validation/demos/rhino): a mode 2 picture whose silhouette is made of pen switches
//! timed to the pixel after an `EI`+`HALT`. Both Gate Arrays must reproduce the reference
//! emulators' pictures: the 40007/8 AMSpiriT Lite's (which lets a tiny black pixel through before
//! a switch), the 40010 ACE-DL's. The two references differ by one pixel of palette timing.
//!
//! Slow (plays 37 s of emulated time): `cargo test --release --test rhino_raster -- --ignored`.

use amsrust::csl_runner::CslRunner;
use image::RgbImage;

/// Pixels of `ours` (a picture of 272 lines, each drawn twice) that differ from `reference` (544 or
/// 542 lines), over the best vertical shift, in the part of the picture all emulators agree on.
fn differing_pixels(reference: &RgbImage, ours: &RgbImage) -> usize {
    let at = |image: &RgbImage, x: u32, y: i64, doubled: bool| -> [i32; 3] {
        let y = if doubled { y / 2 } else { y };
        let pixel = image.get_pixel(x, y.clamp(0, image.height() as i64 - 1) as u32);
        [pixel[0] as i32, pixel[1] as i32, pixel[2] as i32]
    };
    (-4i64..=4)
        .map(|shift| {
            let mut count = 0;
            for y in 40i64..480 {
                for x in 60u32..640 {
                    let (a, b) = (at(reference, x, y, false), at(ours, x, y + shift, true));
                    if (0..3).map(|c| (a[c] - b[c]).abs()).sum::<i32>() > 60 {
                        count += 1;
                    }
                }
            }
            count
        })
        .min()
        .unwrap()
}

fn rhino(gate_array: &str) -> RgbImage {
    let script =
        std::fs::read_to_string("validation/demos/rhino/rhino.csl").expect("the Rhino CSL");
    let script = script
        .replace(
            "crtc_select 1",
            &format!("gate_array {gate_array}\ncrtc_select 0"),
        )
        .replace("'../../rhino_mdr.dsk'", "'validation/rhino_mdr.dsk'");
    let dir = std::env::temp_dir().join(format!("amsrust_rhino_{gate_array}"));
    let mut runner = CslRunner::new(&dir);
    runner
        .run_text(&script, std::env::current_dir().unwrap())
        .expect("the CSL runs");
    image::open(dir.join("rhino_01.png"))
        .expect("the screenshot")
        .to_rgb8()
}

#[test]
#[ignore = "slow: plays the demo"]
fn both_gate_arrays_match_their_reference_emulators() {
    let amspirit = image::open("validation/demos/rhino/amspirit_1.13.4_crtc1.png")
        .unwrap()
        .to_rgb8();
    let ace = image::open("validation/demos/rhino/ace_dl.png")
        .unwrap()
        .to_rgb8();
    let (ga40007, ga40010) = (rhino("40007"), rhino("40010"));

    assert!(
        differing_pixels(&amspirit, &ga40007) < 10,
        "GA 40007/8 vs AMSpiriT Lite"
    );
    assert!(differing_pixels(&ace, &ga40010) < 10, "GA 40010 vs ACE-DL");
    // and the chips really differ
    assert!(
        differing_pixels(&amspirit, &ga40010) > 20,
        "GA 40010 should not look like AMSpiriT Lite"
    );
    assert!(
        differing_pixels(&ace, &ga40007) > 20,
        "GA 40007/8 should not look like ACE-DL"
    );
}
