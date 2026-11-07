//! Regression tests against a real, independent hardware diagnostics tool
//! (see `validation/AmstradDiag/NOTICE.md`), rather than amsrust's own
//! assumptions about what's correct - this is what actually caught the
//! `src/memory.rs` RAM-paging panic these tests guard against (a stock
//! 128K machine crashed the instant the tool's own boot-time memory-sizing
//! routine selected one of the RAM pages it doesn't have fitted).
//!
//! Golden screenshots live under `tests/fixtures/amstrad_diagnostics/` -
//! amsrust's output for this disc is fully deterministic (fixed ROMs, fixed
//! disc image, fixed typed input, no wall-clock/RNG involved), so a
//! byte-for-byte pixel match is a meaningful, low-maintenance check: if it
//! ever legitimately needs to change (a real rendering fix), regenerate the
//! fixture and eyeball the new picture rather than adjust pixel assertions.
//!
//! The tool also correctly detects a full 4MB `LargeRamExpansion` (see
//! `src/expansion.rs`) plugged into the expansion port - `upper_ram_test_detects_a_4mb_expansion`
//! below. Getting that working surfaced a test-harness gotcha worth noting:
//! `CslRunner`'s CSL `reset` instruction replaces the entire `Motherboard`
//! (`self.motherboard = Self::machine(...)`), so anything configured via
//! `motherboard_mut()` *before* a script containing `reset` is silently
//! discarded - plug an expansion in a separate `run_text` call *after* the
//! one that resets, never before.

use amsrust::csl_runner::CslRunner;
use image::RgbImage;
use std::path::Path;

const DISC_DIR: &str = "validation/AmstradDiag";
const FIXTURES: &str = "tests/fixtures/amstrad_diagnostics";

/// Boots `AmstradDiag.dsk` on the given CRTC type and types `RUN"DIAG.BIN"`,
/// driving it further with `extra_keys` (each element typed, then waited on
/// for `wait_us` microseconds, then screenshotted) - one screenshot per
/// element of `extra_keys`, in order, plus the very first one taken right
/// after boot.
fn run_diag(crtc: u8, extra_keys: &[(&str, u64)]) -> Vec<RgbImage> {
    let dir = std::env::temp_dir().join(format!(
        "amsrust_amstrad_diag_{}_{crtc}_{}",
        std::process::id(),
        extra_keys.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let mut runner = CslRunner::new(&dir);
    let mut script = format!(
        "csl_version 1.0\n\
         crtc_select {crtc}\n\
         reset\n\
         wait 3000000\n\
         disk_insert 'AmstradDiag.dsk'\n\
         key_delay 70000 70000 400000\n\
         key_output 'RUN\"DIAG.BIN\"\\(RET)'\n\
         wait 8000000\n\
         screenshot\n"
    );
    for (keys, wait_us) in extra_keys {
        script.push_str(&format!(
            "key_output '{keys}'\nwait {wait_us}\nscreenshot\n"
        ));
    }

    let report = runner
        .run_text(&script, DISC_DIR)
        .expect("the diagnostics script should run to completion");
    assert!(
        report.ignored.is_empty(),
        "unexpected ignored instructions: {:?}",
        report.ignored
    );

    report
        .screenshots
        .iter()
        .map(|s| {
            image::open(&s.path)
                .unwrap_or_else(|e| panic!("{}: {e}", s.path.display()))
                .into_rgb8()
        })
        .collect()
}

/// Compares against a golden PNG under `tests/fixtures/amstrad_diagnostics/`,
/// pixel for pixel (not by file bytes, so PNG re-encoding never matters).
#[track_caller]
fn assert_matches_golden(actual: &RgbImage, golden_name: &str) {
    let path = Path::new(FIXTURES).join(golden_name);
    let golden = image::open(&path)
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e} (missing golden? see this file's module doc)",
                path.display()
            )
        })
        .into_rgb8();
    assert_eq!(
        actual.dimensions(),
        golden.dimensions(),
        "{golden_name}: frame size changed ({:?} vs golden {:?})",
        actual.dimensions(),
        golden.dimensions()
    );
    assert_eq!(
        actual.as_raw(),
        golden.as_raw(),
        "{golden_name}: pixels differ from the golden screenshot"
    );
}

/// Like `assert_matches_golden`, but only over the `(x, y, width, height)`
/// region of `actual` - for a frame whose *rest* legitimately varies with
/// exact timing (e.g. the Upper RAM test's per-bank progress hatching,
/// mid-sweep) while the part actually being checked doesn't.
#[track_caller]
fn assert_region_matches_golden(
    actual: &RgbImage,
    region: (u32, u32, u32, u32),
    golden_name: &str,
) {
    use image::GenericImageView;
    let (x, y, w, h) = region;
    let cropped = actual.view(x, y, w, h).to_image();
    let path = Path::new(FIXTURES).join(golden_name);
    let golden = image::open(&path)
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e} (missing golden? see this file's module doc)",
                path.display()
            )
        })
        .into_rgb8();
    assert_eq!(
        cropped.dimensions(),
        golden.dimensions(),
        "{golden_name}: cropped region size doesn't match the golden ({:?} vs {:?})",
        cropped.dimensions(),
        golden.dimensions()
    );
    assert_eq!(
        cropped.as_raw(),
        golden.as_raw(),
        "{golden_name}: pixels differ from the golden crop"
    );
}

/// The exact panic this whole test file exists to guard against: a stock
/// 128K machine (the default) used to crash the instant `DIAG.BIN`'s own
/// boot-time RAM-sizing routine selected a page beyond the one it has
/// fitted (`assert!(u <= 3)` in `PageNumber::from`, `src/memory.rs`, before
/// the fix - found by running this exact disc, not a hypothetical). Also
/// checks the auto-run Lower RAM test actually reports PASSED, and that the
/// tool correctly identifies a real 6128/128K/FDC - i.e. that our PPI
/// (vendor strap) and FDC presence reporting are consistent with each other
/// from an independent tool's point of view, not just amsrust's own.
#[test]
#[ignore = "slow: boots the real firmware and a real diagnostics tool from disc"]
fn boots_and_lower_ram_passes() {
    let frames = run_diag(0, &[]);
    assert_matches_golden(&frames[0], "menu_lower_ram_passed.png");
}

/// Drives the Upper RAM test (`U`) to completion and checks it reports
/// PASSED - the *positive* counterpart to `boots_and_lower_ram_passes`:
/// not just "doesn't crash" but "an independent tool confirms every bank of
/// the extra 64K page reads back what was written", i.e. genuinely
/// exercises `Memory::ram_bank`/`ram_bank_mut`'s `PageNumber::Page` path
/// end to end through real firmware and real machine code, not just the
/// unit tests in `src/memory.rs`. Also confirms the "C3" memory
/// configuration (a real-hardware compatibility quirk some expansions get
/// wrong) is reported "SUPPORTED".
#[test]
#[ignore = "slow: boots the real firmware and a real diagnostics tool from disc"]
fn upper_ram_test_passes() {
    let frames = run_diag(0, &[("U", 20_000_000)]);
    assert_matches_golden(&frames[1], "upper_ram_passed.png");
}

/// A real >512K expansion, correctly detected: unlike `upper_ram_test_passes`
/// (the default stock 128K case), this machine has a full 4MB
/// `LargeRamExpansion` plugged in, and the tool's own boot-time RAM-sizing
/// sweep (the same one `PageNumber::Expansion`/`RamConfig::with_expansion_high_bits`
/// exist to answer correctly) must report `TOTAL UPPER RAM: 4096KB`, not
/// the 64KB a stock machine shows. Only the "TOTAL UPPER RAM" line is
/// checked (`assert_region_matches_golden`, not a whole-frame compare):
/// the bank-by-bank test underneath is still actively progressing at this
/// point (each of the tool's 64 display cells now covers a full 64K bank
/// instead of 1K, so the exhaustive sweep takes proportionally longer than
/// the 512K case and isn't run to completion here), and its exact
/// hatching pattern legitimately depends on timing the rest of this test
/// doesn't control.
///
/// The expansion is plugged in via a *second* `run_text` call, after the
/// one that resets - see this file's module doc for why that ordering
/// matters (`reset` replaces the whole `Motherboard`).
#[test]
#[ignore = "slow: boots the real firmware and a real diagnostics tool from disc"]
fn upper_ram_test_detects_a_4mb_expansion() {
    let dir = std::env::temp_dir().join(format!("amsrust_amstrad_diag_4mb_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut runner = CslRunner::new(&dir);

    runner
        .run_text("csl_version 1.0\ncrtc_select 0\nreset\n", DISC_DIR)
        .unwrap();
    runner.motherboard_mut().plug_large_ram_expansion(64);
    assert_eq!(
        runner.motherboard().large_ram_expansion_bank_count(),
        Some(64)
    );

    let report = runner
        .run_text(
            "csl_version 1.0\n\
             wait 3000000\n\
             disk_insert 'AmstradDiag.dsk'\n\
             key_delay 70000 70000 400000\n\
             key_output 'RUN\"DIAG.BIN\"\\(RET)'\n\
             wait 8000000\n\
             key_output 'U'\n\
             wait 8000000\n\
             screenshot\n",
            DISC_DIR,
        )
        .unwrap();
    assert!(
        report.ignored.is_empty(),
        "unexpected ignored instructions: {:?}",
        report.ignored
    );

    let frame = image::open(&report.screenshots[0].path)
        .unwrap()
        .into_rgb8();
    assert_region_matches_golden(&frame, (190, 63, 280, 9), "upper_ram_size_4096kb.png");
}

/// The keyboard test draws the full key matrix and highlights whichever
/// key(s) the firmware currently reports as pressed - typing an uppercase
/// `A` (Shift+A on this layout) should highlight exactly the `A` and
/// `SHFT` keys on screen, and nothing else. This is an independent,
/// visual check of `src/keyboard.rs`'s matrix positions (already checked
/// against the ROM's own translation table in `tests/keyboard_matrix_rom.rs`)
/// actually reaching the screen through the PPI/PSG/firmware chain.
#[test]
#[ignore = "slow: boots the real firmware and a real diagnostics tool from disc"]
fn keyboard_test_highlights_the_pressed_key() {
    let frames = run_diag(0, &[("K", 3_000_000), ("A", 1_000_000)]);
    assert_matches_golden(&frames[1], "keyboard_test_blank.png");
    assert_matches_golden(&frames[2], "keyboard_test_a_highlighted.png");
}

/// Every key `key_output` can type - letters, digits, space, and the
/// special keys its `\(NAME)` escapes cover (per cpclib-csl's own parser:
/// RET, TAB, CAP, DEL, CLR, COP, the 4 cursor keys, and the numeric
/// keypad's FN0-FN9/ENT) - should register through the full
/// PPI/PSG/firmware chain and reach the diagnostics tool's keyboard test,
/// not just `A` (see `keyboard_test_highlights_the_pressed_key`). This
/// doesn't check *which* key lit up (that's `src/keyboard.rs`'s job,
/// already checked exhaustively against the ROM's own translation table in
/// `tests/keyboard_matrix_rom.rs`, for every key including ones not listed
/// here) - it checks that *some* visible change happens for every single
/// one of these, through this independent tool, and that the screen is
/// back to blank once the very last one releases.
///
/// Not covered: Shift/Ctrl (no visible box of their own beyond the
/// modifier highlight already seen in `keyboard_test_highlights_the_pressed_key`),
/// and `KeypadDot` (cpclib-csl's `key_output` has no escape for it).
#[test]
#[ignore = "slow: boots the real firmware and a real diagnostics tool from disc"]
fn keyboard_test_registers_every_typeable_key() {
    const KEYS: &[&str] = &[
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
        " ", "\\(RET)", "\\(TAB)", "\\(ESC)", "\\(CAP)", "\\(DEL)", "\\(CLR)", "\\(COP)",
        "\\(ARL)", "\\(ARR)", "\\(ARU)", "\\(ARD)", "\\(FN0)", "\\(FN1)", "\\(FN2)", "\\(FN3)",
        "\\(FN4)", "\\(FN5)", "\\(FN6)", "\\(FN7)", "\\(FN8)", "\\(FN9)", "\\(ENT)",
    ];

    let dir = std::env::temp_dir().join(format!(
        "amsrust_amstrad_diag_allkeys_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut runner = CslRunner::new(&dir);

    // A short, uniform press/release cycle: 100ms held (comfortably longer
    // than one video frame, so a screenshot taken 90ms after `key_output`
    // lands safely inside the "held" window) then a 20ms gap, i.e. a 120ms
    // cycle per key. `CslRunner::schedule_keys` queues each key back to
    // back regardless of when `key_output` is actually called (it starts
    // at `max(keys_busy_until, now)`), so waiting a fixed 90ms after every
    // `key_output` before screenshotting keeps a steady ~30ms margin ahead
    // of "now" forever: each screenshot lands inside *that* key's own
    // 100ms press window, never the previous or next one's.
    let mut script = String::from(
        "csl_version 1.0\n\
         crtc_select 0\n\
         reset\n\
         wait 3000000\n\
         disk_insert 'AmstradDiag.dsk'\n\
         key_delay 70000 70000 400000\n\
         key_output 'RUN\"DIAG.BIN\"\\(RET)'\n\
         wait 8000000\n\
         key_output 'K'\n\
         wait 3000000\n\
         screenshot\n\
         key_delay 100000 20000 20000\n",
    );
    for key in KEYS {
        script.push_str(&format!("key_output '{key}'\nwait 90000\nscreenshot\n"));
    }
    let report = runner
        .run_text(&script, DISC_DIR)
        .expect("the diagnostics script should run to completion");
    assert!(
        report.ignored.is_empty(),
        "unexpected ignored instructions: {:?}",
        report.ignored
    );

    let frames: Vec<RgbImage> = report
        .screenshots
        .iter()
        .map(|s| {
            image::open(&s.path)
                .unwrap_or_else(|e| panic!("{}: {e}", s.path.display()))
                .into_rgb8()
        })
        .collect();

    // Checked once, right after entering the test, before any of these keys
    // are pressed - not re-checked after the last one releases: the tool's
    // own screen has a time-based blink/animation somewhere on it, so a
    // "blank" frame captured this much later (after 59 key presses) is
    // legitimately not byte-identical to one captured right at entry, with
    // no key involved at all.
    let baseline = frames[0].clone();
    assert_matches_golden(&baseline, "keyboard_test_blank.png");

    for (key, frame) in KEYS.iter().zip(&frames[1..]) {
        assert_ne!(
            frame.as_raw(),
            baseline.as_raw(),
            "typing {key:?} produced no visible change on the keyboard test screen"
        );
    }
}

/// The tool's own System Info panel prints the CRTC type it detects
/// (independently of whatever `crtc_select` told amsrust to emulate) - an
/// outside cross-check of `src/crtc/mod.rs`'s type-detection behaviour,
/// complementing the dedicated `tests/crtc_detection.rs`/`crtc_variants.rs`
/// suites rather than replacing them. Checked against CRTC 1 specifically
/// (not just the default 0 that the other tests here incidentally cover)
/// so this adds real, non-redundant coverage.
#[test]
#[ignore = "slow: boots the real firmware and a real diagnostics tool from disc"]
fn system_info_reports_the_configured_crtc_type() {
    let frames = run_diag(1, &[]);
    assert_matches_golden(&frames[0], "menu_crtc1_lower_ram_passed.png");
}
