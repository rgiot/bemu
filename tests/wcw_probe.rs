//! What a Cool Weekend (Benediction, validation/demos/wcw): the second part waves the whole picture
//! with the HSYNC. Plays the demo up to the waving screens and saves frames, to look at how the
//! monitor follows a moving HSYNC (a jump in the picture's edge is a loss of the line lock).
//!
//! Slow (a minute of emulated time): `WCW_OUT=/some/dir cargo test --release --test wcw_probe --
//! --ignored --nocapture`; the frames are `<dir>/wcw_NN.png`.

use amsrust::csl_runner::CslRunner;

#[test]
#[ignore = "slow: plays the demo"]
fn save_the_waving_frames() {
    let out = std::env::var("WCW_OUT").unwrap_or_else(|_| "/tmp/wcw_probe".to_owned());
    let mut lines: Vec<String> = [
        "csl_version 1.0",
        "crtc_select 0",
        "reset",
        "wait 6000000",
        "disk_insert 'wcw.dsk'",
        "key_delay 70000 70000 400000",
        "key_output 'RUN\"-WACWE!\"\\(RET)'",
        "wait 60000000",
        "key_output ' '",
        "wait 6000000",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();
    for k in 1..=8 {
        lines.push(format!("screenshot_name 'wcw_{k:02}'"));
        lines.push("screenshot".to_owned());
        lines.push("wait 900000".to_owned());
    }
    let script = lines.join("\n") + "\n";
    let mut runner = CslRunner::new(&out);
    runner
        .run_text(&script, concat!(env!("CARGO_MANIFEST_DIR"), "/validation/demos/wcw"))
        .expect("the CSL plays");
}
