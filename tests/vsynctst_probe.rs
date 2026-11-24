//! VSYNCTST (tools/vsynctest of CLAUDESTRAD): with the border red the program takes the HSYNC away for
//! 8 lines at the VSYNC, and the monitor, which syncs on the second HSYNC, shows the picture one text
//! row (8 lines) higher than with the border blue. The user's real photos of both states (a CTM 644)
//! show the first text row gone above the picture in the red state.

use amsrust::csl_runner::CslRunner;
use image::RgbImage;

/// (border is red, first row with white text) of a screenshot.
fn state(image: &RgbImage) -> (bool, u32) {
    let (w, h) = image.dimensions();
    let red = image.get_pixel(2, 2).0[0] > 200;
    let first = (0..h)
        .find(|&y| {
            (w / 3..2 * w / 3)
                .step_by(2)
                .any(|x| image.get_pixel(x, y).0.iter().map(|&c| u32::from(c)).sum::<u32>() > 600)
        })
        .unwrap();
    (red, first)
}

#[test]
fn the_picture_rises_one_text_row_when_the_hsync_is_missing_at_the_vsync() {
    let out = std::env::temp_dir().join(format!("bemu-vsynctst-{}", std::process::id()));
    let mut runner = CslRunner::new(out.to_str().unwrap());
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claudestrad/disks");
    let mut script = String::from(
        "csl_version 1.0\ncrtc_select 1\nreset\nwait 6000000\ndisk_insert 'VSYNCTST.DSK'\n\
         key_delay 70000 70000 400000\nkey_output 'RUN\"VSYNCTST\\(RET)'\nwait 12000000\n",
    );
    for k in 0..8 {
        script += &format!("screenshot_name 'v{k:02}'\nscreenshot\nwait 700000\n");
    }
    runner.run_text(&script, dir).unwrap();
    let states: Vec<(bool, u32)> = (0..8)
        .map(|k| state(&image::open(out.join(format!("v{k:02}.png"))).unwrap().to_rgb8()))
        .collect();
    let blue = states.iter().find(|s| !s.0).expect("a blue frame").1;
    let red = states.iter().find(|s| s.0).expect("a red frame").1;
    assert_eq!(blue - red, 8, "{states:?}");
    assert!(states.iter().all(|s| s.1 == if s.0 { red } else { blue }), "{states:?}");
}
