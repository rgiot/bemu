//! Onescreen Colonies #0 to #3 (Vanity; validation/demos/oc0..oc3) against AMSpiriT Lite 1.16.0,
//! CRTC 1 and Gate Array 40007 as AMSpiriT emulates them. Each demo is played from its script and
//! 800 frames in a row are kept from 20 s after the RUN; every shot AMSpiriT took
//! (`amspirit_1.16.0/`) must have an identical frame among them (the two emulators do not start
//! the demo at the same moment: the CSL typing differs, the demo then runs the same).
//!
//! #1 is not identical yet: its screen splitting (two HSYNC positions a line, R2=21 and R2=50)
//! makes AMSpiriT's monitor pull the picture, and on two short gaps of the second HSYNC it ends
//! one pixel further each time, which bemu does not reproduce: a bound is checked instead.
//!
//! Slow (plays 4 x 36 s): `cargo test --release --test onescreen_colonies -- --ignored`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use amsrust::csl_runner::{CslRunner, FsFiles, ScreenshotSink};
use image::RgbImage;

const FRAMES: usize = 800;
const HEIGHT: u32 = 271;
const WIDTH: u32 = 768;

/// Pixels of `ours` (272 lines) unlike `amspirit` (542 lines, each line twice): the two emulators'
/// 27 colours are a few units apart.
fn differing(ours: &RgbImage, amspirit: &RgbImage) -> usize {
    let mut count = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let (a, b) = (ours.get_pixel(x, y), amspirit.get_pixel(x, y * 2));
            let d: i32 = (0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).sum();
            if d > 24 {
                count += 1;
            }
        }
    }
    count
}

/// Keeps, for each reference, the fewest differing pixels of a frame seen so far.
struct BestMatch {
    references: Vec<(String, RgbImage)>,
    best: Rc<RefCell<Vec<usize>>>,
}

impl ScreenshotSink for BestMatch {
    fn save(&mut self, dir: &Path, file: &str, frame: &RgbImage) -> Result<PathBuf, String> {
        let mut best = self.best.borrow_mut();
        for (i, (_, reference)) in self.references.iter().enumerate() {
            if best[i] > 0 {
                best[i] = best[i].min(differing(frame, reference));
            }
        }
        Ok(dir.join(file))
    }
}

/// The best match of each AMSpiriT shot of `demo` among the frames bemu plays.
fn play(demo: &str) -> Vec<(String, usize)> {
    let dir = Path::new("validation/demos").join(demo);
    let script = std::fs::read_to_string(dir.join(format!("{demo}.csl")))
        .or_else(|_| std::fs::read_to_string(dir.join(format!("{demo}_crtc1.csl"))))
        .expect("the demo's script");
    // the script up to the RUN, then a frame every 20 ms
    let mut text: String = script
        .lines()
        .take_while(|line| !line.starts_with("wait 5000000"))
        .map(|line| format!("{line}\n"))
        .collect();
    text.push_str("wait 20000000\n");
    for i in 0..FRAMES {
        text.push_str(&format!("screenshot_name 'f{i:03}'\nscreenshot\nwait 20000\n"));
    }
    let mut references: Vec<(String, RgbImage)> = std::fs::read_dir(dir.join("amspirit_1.16.0"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, image::open(&path).unwrap().to_rgb8())
        })
        .collect();
    references.sort_by(|a, b| a.0.cmp(&b.0));
    let best = Rc::new(RefCell::new(vec![usize::MAX; references.len()]));
    let names: Vec<String> = references.iter().map(|(name, _)| name.clone()).collect();
    let mut runner = CslRunner::with_io(
        Box::new(FsFiles::default()),
        Box::new(BestMatch {
            references,
            best: best.clone(),
        }),
        std::env::temp_dir().join(format!("amsrust_{demo}")),
    );
    runner.run_text(&text, &dir).expect("the CSL runs");
    let best = best.borrow();
    names.into_iter().zip(best.iter().copied()).collect()
}

fn check(demo: &str, tolerated: usize) {
    let results = play(demo);
    println!("{demo}: {results:?}");
    assert!(!results.is_empty());
    for (name, differ) in results {
        assert!(
            differ <= tolerated,
            "{demo} {name}: no frame of bemu within {tolerated} pixels ({differ})"
        );
    }
}

#[test]
#[ignore = "slow: plays the demo"]
fn onescreen_colonies_0_matches_amspirit() {
    check("oc0", 0);
}

#[test]
#[ignore = "slow: plays the demo"]
fn onescreen_colonies_1_is_near_amspirit() {
    // the first two shots: the logo alone; the last two: the picture drawn over the screen
    check("oc1", 12_000);
}

#[test]
#[ignore = "slow: plays the demo"]
fn onescreen_colonies_2_matches_amspirit() {
    check("oc2", 0);
}

#[test]
#[ignore = "slow: plays the demo"]
fn onescreen_colonies_3_matches_amspirit() {
    check("oc3", 0);
}
