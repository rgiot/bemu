//! The CLAUDESTRAD test discs (`tests/fixtures/claudestrad`, see its `LISEZMOI.md`): one small disc
//! per timing/hardware experiment (interrupt position, HALT, INK timing, FDC, PSG...), each with
//! the screens another emulator (CLAUDESTRAD) shows for a 6128 with CRTC 0, 1 and 2 (the 6128+
//! for the Plus tests).
//!
//! The reference PNGs are what CLAUDESTRAD draws, not what a CPC draws: the real machines'
//! pictures are the photos in `tests/fixtures/claudestrad/photos/<case>/` (AVIF, named after the
//! machine when it is known: `crtc1_sans_m4_...`, `crtc2_m4_...`), to be read by a person. So a
//! case does this:
//!
//! * it checks that the disc boots and that the program ran (the screen is not the firmware's
//!   blank BASIC screen any more);
//! * a screen of text is compared as text, read back from both pictures by `cpclib-ocr` (so the
//!   framing and zoom of each emulator's picture do not matter), and must be identical;
//! * any other screen is compared as pixels, over the best of a few small shifts. A match below
//!   [`MIN_MATCH`] fails;
//! * [`KNOWN_DIFFERENCES`] lists what bemu is known to draw otherwise (with the share of pixels
//!   it reaches, for a pixel comparison): those are reported, not failed;
//! * bemu's picture is written to `target/claudestrad/<case>_<machine>.png`, to put next to the
//!   reference and the photos.
//!
//! Slow (minutes of emulated time over ~140 runs): they are ignored by default, run them with
//! `cargo test --release --test claudestrad -- --ignored --nocapture`. Run one case with its name
//! as a filter, e.g. `... --ignored intpos`.

use amsrust::crtc::CrtcType;
use amsrust::csl_runner::CslRunner;
use amsrust::motherboard::{CpcModel, MotherboardBuilder};
use image::RgbImage;
use std::path::PathBuf;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claudestrad");

/// Share of the non-background pixels that must agree with CLAUDESTRAD's screen, unless the case is listed
/// in [`KNOWN_DIFFERENCES`].
const MIN_MATCH: f64 = 0.97;

#[derive(Clone, Copy)]
enum Machine {
    Crtc(u8),
    Plus,
}

impl Machine {
    fn label(self) -> String {
        match self {
            Machine::Crtc(n) => format!("crtc{n}"),
            Machine::Plus => "6128plus".to_owned(),
        }
    }
}

struct Case {
    /// Name of the experiment (and of the reference pictures).
    name: &'static str,
    /// Disc in `disks/`.
    disk: &'static str,
    /// The program to `RUN"`.
    program: &'static str,
    /// Keys to press (a space) after the program is up, with the emulated seconds to wait after
    /// each one. The first wait is for the program to start.
    waits: &'static [u32],
    /// Needs the PlayCity card.
    playcity: bool,
    /// The outcome varies from run to run on a real machine (random timing): the picture is not
    /// compared as strictly.
    random: bool,
}

const fn case(name: &'static str, disk: &'static str, waits: &'static [u32]) -> Case {
    Case {
        name,
        disk,
        program: name,
        waits,
        playcity: false,
        random: false,
    }
}

const CASES: &[Case] = &[
    case("INTPOS", "INTPOS.DSK", &[12]),
    case("INTPOS2", "INTPOS2.DSK", &[12]),
    case("INTPOS3", "INTPOS3.DSK", &[12]),
    case("INTREP", "INTREP.DSK", &[12]),
    case("HALTPOS", "HALTPOS.DSK", &[12]),
    case("HALTTST2", "HALTTST2.DSK", &[12, 6]),
    case("HALTTEST", "HALTTEST.DSK", &[12, 6]),
    case("HALTPEND", "HALTPEND.DSK", &[12]),
    case("INTLAT", "INTLAT.DSK", &[12]),
    case("INTLAT1", "INTLAT1.DSK", &[12]),
    case("VSINT", "VSINT.DSK", &[12]),
    case("VSINT1", "VSINT1.DSK", &[12]),
    Case {
        random: true,
        ..case("PPIRACE", "PPIRACE.DSK", &[30])
    },
    case("PALA14", "PALA14.DSK", &[12]),
    case("PALA14B", "PALA14B.DSK", &[12]),
    case("HSYNCPOS", "HSYNCPOS.DSK", &[12, 6]),
    case("INKBORD", "INKBORD.DSK", &[12, 6]),
    case("INKMODE", "INKMODE.DSK", &[12]),
    case("INKPOINT", "INKPOINT.DSK", &[12]),
    case("INKTEST", "INKTEST.DSK", &[12, 6]),
    case("CRTCTEST", "CRTCTEST.DSK", &[30]),
    case("R0ZTEST", "R0ZTEST.DSK", &[12]),
    case("VSYNCTST", "VSYNCTST.DSK", &[12]),
    case("KUKU", "KUKU.DSK", &[12]),
    case("Z80TYPE", "Z80TYPE.DSK", &[12]),
    case("CADRAGE", "CADRAGE.DSK", &[12, 6]),
    case("FDCTEST", "FDCTEST.DSK", &[30]),
    case("FDCTEST2", "FDCTEST2.DSK", &[30, 10, 10, 10, 10]),
    case("FDCTEST3", "FDCTEST3.DSK", &[30, 10, 10, 10, 10]),
    Case {
        program: "SEEKJIT",
        ..case("SEEKJIT", "SEEKJIT.DSK", &[30])
    },
    case("SEEKJIT2", "SEEKJIT.DSK", &[30]),
    case("SEEKJIT3", "SEEKJIT.DSK", &[30]),
    case("SEEKJIT4", "SEEKJIT.DSK", &[30]),
    case("HSYNCW", "MESURES.DSK", &[12, 6]),
    case("FDCRES", "MESURES.DSK", &[30, 10]),
    case("READYTST", "MESURES.DSK", &[30, 10]),
    case("SPINDOWN", "MESURES.DSK", &[40, 10]),
    case("SPINDWN2", "MESURES.DSK", &[40, 10]),
    case("ROTATION", "MESURES.DSK", &[30, 10]),
    case("AYLECT", "MESURES.DSK", &[12]),
    case("AYSEQ", "MESURES.DSK", &[12]),
    case("AYTRAME", "MESURES.DSK", &[12]),
    case("VHOLDTST", "MESURES.DSK", &[12]),
    Case {
        playcity: true,
        ..case("NMICUR", "PLAYCITY_MESURE.DSK", &[20])
    },
    Case {
        playcity: true,
        ..case("PCSON", "PCSON.DSK", &[20])
    },
    case("PLUSDMA", "PLUSDMA.DSK", &[12, 6]),
    case("PLUSDMA2", "PLUSDMA2.DSK", &[12, 6]),
    case("PLUSIM2", "PLUSIM2.DSK", &[12, 6]),
];

/// Cases (and machines) where bemu is known to differ from CLAUDESTRAD, with the share of agreeing
/// pixels it reaches today (ignored for a screen of text, which is then only reported). Keep it
/// honest: a missing entry means "identical text, or at least [`MIN_MATCH`] of the pixels".
/// (Filled in from a first run, see the commit that added this file.)
const KNOWN_DIFFERENCES: &[(&str, &str, f64)] = &[];

fn machine(machine: Machine, playcity: bool) -> MotherboardBuilder {
    let mut builder = MotherboardBuilder::default();
    match machine {
        Machine::Crtc(n) => {
            builder.set_crtc(match n {
                0 => CrtcType::Crtc0,
                1 => CrtcType::Crtc1,
                _ => CrtcType::Crtc2,
            });
        }
        Machine::Plus => {
            builder.set_model("6128plus".parse::<CpcModel>().unwrap());
            builder.set_crtc(CrtcType::Crtc3);
        }
    }
    if playcity {
        #[cfg(all(feature = "playcity", feature = "digiblaster"))]
        builder.plug_playcity();
    }
    builder
}

/// The CSL script of a case: boot, `RUN"program`, one space per extra wait, a screenshot.
fn script(case: &Case) -> String {
    let mut s = String::from("csl_version 1.1\nreset\nwait 6000000\n");
    s += &format!("disk_insert '{FIXTURES}/disks/{}'\n", case.disk);
    s += "key_delay 70000 70000 400000\n";
    s += &format!("key_output 'RUN\"{}\"\\(RET)'\n", case.program);
    for (i, seconds) in case.waits.iter().enumerate() {
        if i > 0 {
            s += "key_output ' '\n";
        }
        s += &format!("wait {}\n", *seconds as u64 * 4_000_000);
    }
    s += "screenshot_name 'shot'\nscreenshot\n";
    s
}

fn run(case: &Case, which: Machine) -> RgbImage {
    let dir = std::env::temp_dir().join(format!(
        "amsrust_claudestrad_{}_{}",
        case.name,
        which.label()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut runner = CslRunner::new(&dir);
    runner.set_machine(machine(which, case.playcity));
    runner
        .run_text(&script(case), std::env::current_dir().unwrap())
        .unwrap_or_else(|e| panic!("{}: the CSL fails: {e}", case.name));
    image::open(dir.join("shot_01.png"))
        .or_else(|_| image::open(dir.join("shot.png")))
        .unwrap_or_else(|e| panic!("{}: no screenshot: {e}", case.name))
        .to_rgb8()
}

/// Brings `image` (any size) to `width` x `height` by nearest neighbour.
fn scaled(image: &RgbImage, width: u32, height: u32) -> RgbImage {
    image::imageops::resize(image, width, height, image::imageops::FilterType::Nearest)
}

/// Share of the pixels of `reference` that `ours` reproduces (within a small colour distance),
/// after bringing `ours` to the reference's size, counted only over the area where either picture
/// has something but the background (most of a screen is empty paper, which would hide
/// everything). The two emulators frame the picture a pixel or two apart, so the best of a few
/// small shifts counts.
fn match_share(reference: &RgbImage, ours: &RgbImage) -> f64 {
    let ours = scaled(ours, reference.width(), reference.height());
    let background = |image: &RgbImage| *image.get_pixel(2, image.height() - 2);
    let (bg_ref, bg_ours) = (background(reference), background(&ours));
    let (width, height) = (reference.width() as i64, reference.height() as i64);
    let mut best = 0.0f64;
    for dy in -4i64..=4 {
        for dx in -8i64..=8 {
            let (mut total, mut same) = (0usize, 0usize);
            for y in 0..height {
                let sy = y - dy;
                if !(0..height).contains(&sy) {
                    continue;
                }
                for x in 0..width {
                    let sx = x - dx;
                    if !(0..width).contains(&sx) {
                        continue;
                    }
                    let a = reference.get_pixel(x as u32, y as u32);
                    let b = ours.get_pixel(sx as u32, sy as u32);
                    if *a == bg_ref && *b == bg_ours {
                        continue;
                    }
                    total += 1;
                    if (0..3)
                        .map(|c| (a[c] as i32 - b[c] as i32).abs())
                        .sum::<i32>()
                        <= 60
                    {
                        same += 1;
                    }
                }
            }
            best = best.max(if total == 0 {
                1.0
            } else {
                same as f64 / total as f64
            });
        }
    }
    best
}

fn floor(case: &str, machine: &str) -> f64 {
    KNOWN_DIFFERENCES
        .iter()
        .find(|(c, m, _)| *c == case && *m == machine)
        .map_or(MIN_MATCH, |(_, _, share)| *share)
}

/// The colour of the picture's corner: the border.
fn corner(image: &RgbImage) -> [u8; 3] {
    image.get_pixel(2, 2).0
}

/// The most frequent colour: the paper.
fn dominant(image: &RgbImage) -> [u8; 3] {
    let mut counts = std::collections::HashMap::new();
    for pixel in image.pixels() {
        *counts.entry(pixel.0).or_insert(0usize) += 1;
    }
    counts.into_iter().max_by_key(|(c, n)| (*n, *c)).unwrap().0
}

fn blank_basic(image: &RgbImage) -> bool {
    // a screen with at most a few text lines: almost all pixels are the paper
    let background = *image.get_pixel(2, image.height() - 2);
    let other = image.pixels().filter(|p| **p != background).count();
    other * 200 < (image.width() * image.height()) as usize
}

fn check(case: &Case, which: Machine, failures: &mut Vec<String>) {
    let label = which.label();
    let reference = PathBuf::from(format!("{FIXTURES}/refs/{}_{label}.png", case.name));
    if !reference.exists() {
        return;
    }
    let reference = image::open(&reference).unwrap().to_rgb8();
    let ours = run(case, which);
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/claudestrad");
    let _ = std::fs::create_dir_all(&out);
    let _ = ours.save(out.join(format!("{}_{label}.png", case.name)));

    if blank_basic(&ours) && !blank_basic(&reference) {
        failures.push(format!(
            "{} on {label}: nothing ran (blank screen)",
            case.name
        ));
        return;
    }
    let listed = KNOWN_DIFFERENCES
        .iter()
        .any(|(c, m, _)| *c == case.name && *m == label);

    // What the screen says (text, read back from both pictures with the CPC font: it does not
    // depend on how each emulator frames or scales its picture) is compared apart from how it is
    // drawn (the border and paper colours, then every pixel: raster effects, colours, shapes).
    let (expected, got) = (read_text(&reference), read_text(&ours));
    if expected.is_text(20) {
        let (expected, got) = (expected.text(), got.text());
        let rows = |t: &str| t.lines().count();
        let agree = if case.random {
            rows(&expected) == rows(&got)
        } else {
            expected == got
        };
        println!(
            "{:9} {:9} text {}",
            case.name,
            label,
            if agree {
                "identical"
            } else if listed {
                "differs (known)"
            } else {
                "DIFFERS"
            }
        );
        if !agree && !listed {
            failures.push(format!(
                "{} on {label}: the text differs from CLAUDESTRAD's\n--- CLAUDESTRAD\n{expected}\n--- bemu\n{got}",
                case.name
            ));
        }
    }
    for (what, a, b) in [
        ("border", corner(&reference), corner(&ours)),
        ("paper", dominant(&reference), dominant(&ours)),
    ] {
        let distance: i32 = (0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).sum();
        if distance > 60 && !listed {
            println!("{:9} {:9} {what} colour {a:?} vs {b:?}", case.name, label);
            failures.push(format!(
                "{} on {label}: the {what} colour is {b:?}, CLAUDESTRAD's is {a:?}",
                case.name
            ));
        }
    }
    let share = match_share(&reference, &ours);
    println!(
        "{:9} {:9} {:6.2}% of CLAUDESTRAD's pixels",
        case.name,
        label,
        share * 100.0
    );
    let wanted = floor(case.name, &label) - if case.random { 0.10 } else { 0.0 };
    if share + 1e-9 < wanted {
        failures.push(format!(
            "{} on {label}: {:.2}% of the pixels agree with CLAUDESTRAD, {:.2}% wanted",
            case.name,
            share * 100.0,
            wanted * 100.0
        ));
    }
}

/// The text of a screen, read with the 6128's font.
fn read_text(image: &RgbImage) -> cpclib_ocr::Screen {
    let rom = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/roms/os6128.rom")).unwrap();
    let ocr = cpclib_ocr::Ocr::new(cpclib_ocr::Font::from_lower_rom(&rom).unwrap());
    let picture = cpclib_ocr::Picture::new(
        image.width() as usize,
        image.height() as usize,
        image.as_raw().clone(),
    )
    .unwrap();
    ocr.read(&picture)
}

fn run_case(name: &str) {
    let case = CASES.iter().find(|c| c.name == name).expect("a known case");
    let machines = [
        Machine::Crtc(0),
        Machine::Crtc(1),
        Machine::Crtc(2),
        Machine::Plus,
    ];
    let mut failures = Vec::new();
    for which in machines {
        check(case, which, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every disc listed in the cases exists, and every reference belongs to a case: a fixture that
/// is not wired up is a test that does not run. Fast, runs by default.
#[test]
fn fixtures_are_all_wired() {
    for case in CASES {
        assert!(
            std::path::Path::new(&format!("{FIXTURES}/disks/{}", case.disk)).exists(),
            "{}: disc {} missing",
            case.name,
            case.disk
        );
    }
    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(format!("{FIXTURES}/refs")).unwrap() {
        let file = entry.unwrap().file_name().into_string().unwrap();
        let case = file
            .trim_end_matches(".png")
            .trim_end_matches("_crtc0")
            .trim_end_matches("_crtc1")
            .trim_end_matches("_crtc2")
            .trim_end_matches("_6128plus");
        if !CASES.iter().any(|c| c.name == case) {
            orphans.push(file);
        }
    }
    assert!(orphans.is_empty(), "references without a case: {orphans:?}");
    // the script of every case is valid CSL
    for case in CASES {
        cpclib_csl::parse_csl_with_rich_errors(&script(case), None)
            .unwrap_or_else(|_| panic!("{}: invalid CSL", case.name));
    }
}

/// The text of the references is readable (the OCR's check on real screens, fast).
#[test]
fn references_read_as_text() {
    let image = image::open(format!("{FIXTURES}/refs/INTPOS_crtc1.png"))
        .unwrap()
        .to_rgb8();
    let screen = read_text(&image);
    assert!(screen.is_text(100), "{screen:?}");
    let text = screen.text();
    assert!(
        text.starts_with("INTPOS (IM 1) - us depuis le HALT"),
        "{text}"
    );
    assert!(text.contains("NOP            3282  3282"), "{text}");
    // a graphics screen is not text
    let image = image::open(format!("{FIXTURES}/refs/INKTEST_crtc1.png"))
        .unwrap()
        .to_rgb8();
    assert!(!read_text(&image).is_text(20));
}

macro_rules! cases {
    ($($test:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            #[ignore = "slow: runs the disc on four machines"]
            fn $test() {
                run_case($name);
            }
        )*
    };
}

cases! {
    intpos => "INTPOS", intpos2 => "INTPOS2", intpos3 => "INTPOS3", intrep => "INTREP",
    halt_pos => "HALTPOS", halt_tst2 => "HALTTST2", halt_test => "HALTTEST",
    halt_pend => "HALTPEND", intlat => "INTLAT", intlat1 => "INTLAT1", vsint => "VSINT",
    vsint1 => "VSINT1", ppirace => "PPIRACE", pala14 => "PALA14", pala14b => "PALA14B",
    hsyncpos => "HSYNCPOS", inkbord => "INKBORD", inkmode => "INKMODE", inkpoint => "INKPOINT",
    inktest => "INKTEST", crtctest => "CRTCTEST", r0ztest => "R0ZTEST", vsynctst => "VSYNCTST",
    kuku => "KUKU", z80type => "Z80TYPE", cadrage => "CADRAGE", fdctest => "FDCTEST",
    fdctest2 => "FDCTEST2", fdctest3 => "FDCTEST3", seekjit => "SEEKJIT", seekjit2 => "SEEKJIT2",
    seekjit3 => "SEEKJIT3", seekjit4 => "SEEKJIT4", hsyncw => "HSYNCW", fdcres => "FDCRES",
    readytst => "READYTST", spindown => "SPINDOWN", spindwn2 => "SPINDWN2", rotation => "ROTATION",
    aylect => "AYLECT", ayseq => "AYSEQ", aytrame => "AYTRAME", vholdtst => "VHOLDTST",
    nmicur => "NMICUR", pcson => "PCSON", plusdma => "PLUSDMA", plusdma2 => "PLUSDMA2",
    plusim2 => "PLUSIM2",
}
