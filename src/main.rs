//! `amsrust`: boots a machine with the given CRTC/disc(s)/snapshot, for
//! manually testing the emulator against real-world software.
//!
//! ```text
//! amsrust [--crtc 0-4] [--dsk path] [--dsk-b path] [--sna path]
//!         [--screenshot dir [--seconds N] [--every N]]
//! ```
//!
//! With no `--screenshot`/`--window`, drops into the existing interactive
//! monitor (`amsrust::debug::Monitor`) once everything is loaded: `h` at its
//! `>>` prompt lists its commands (`run`/`next`/`step`/`continue`,
//! breakpoints, `info z`/`info crtc`/`info ga`, `disassemble`...) - a real
//! debugger, for stepping through what a piece of software actually does.
//!
//! With `--screenshot <dir>`, runs headless instead: plays for `--seconds`
//! (default 30) real-time seconds of emulated time, saving a screenshot to
//! `<dir>` every `--every` seconds (default 2) - useful to eyeball what a
//! disc/snapshot does without a terminal to type commands into.
//!
//! With `--window`, opens a live window instead, redrawn once per emulated
//! video frame (the CPC's own ~50Hz) with the keyboard driving the CPC's
//! keyboard matrix directly - for actually playing a game/demo rather than
//! debugging it. No menus, no mouse/joystick support yet: close the window
//! to quit.
//!
//! `--sna` restores CPU registers, the base 64K of RAM, the CRTC's
//! registers, the gate array's ink/mode/ROM-paging state and the PSG's
//! registers (see `amsrust::snapshot`); apply it after `--dsk` if the
//! software expects a particular disc already in the drive.

use amsrust::crtc::CrtcType;
use amsrust::debug::{Monitor, MonitorContext};
use amsrust::disc_image::load_dsk;
use amsrust::fdc::FdcDrive;
use amsrust::keyboard::CpcKey;
use amsrust::monitor::FrameGrabber;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use minifb::{Key, Window, WindowOptions};

struct Args {
    crtc: CrtcType,
    dsk_a: Option<String>,
    dsk_b: Option<String>,
    sna: Option<String>,
    screenshot_dir: Option<String>,
    seconds: u64,
    every: u64,
    window: bool
}

fn parse_args() -> Args {
    let mut args = Args {
        crtc: CrtcType::default(),
        dsk_a: None,
        dsk_b: None,
        sna: None,
        screenshot_dir: None,
        seconds: 30,
        every: 2,
        window: false
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut next = |name: &str| it.next().unwrap_or_else(|| panic!("{name} needs a value"));
        match arg.as_str() {
            "--crtc" => {
                let value = next("--crtc");
                args.crtc = value
                    .parse::<u8>()
                    .ok()
                    .and_then(|n| CrtcType::try_from(n).ok())
                    .unwrap_or_else(|| panic!("--crtc must be 0-4, got {value:?}"));
            },
            "--dsk" => args.dsk_a = Some(next("--dsk")),
            "--dsk-b" => args.dsk_b = Some(next("--dsk-b")),
            "--sna" => args.sna = Some(next("--sna")),
            "--screenshot" => args.screenshot_dir = Some(next("--screenshot")),
            "--seconds" => args.seconds = next("--seconds").parse().expect("--seconds must be a number"),
            "--every" => args.every = next("--every").parse().expect("--every must be a number"),
            "--window" => args.window = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            },
            other => {
                eprintln!("unknown argument {other:?}\n");
                print_help();
                std::process::exit(2);
            }
        }
    }
    args
}

fn print_help() {
    eprintln!(concat!(
        "amsrust [--crtc 0-4] [--dsk path] [--dsk-b path] [--sna path]\n",
        "        [--screenshot dir [--seconds N] [--every N]] [--window]\n",
        "\n",
        "With no --screenshot/--window, drops into the interactive monitor once\n",
        "everything is loaded ('h' at its prompt lists its commands). With\n",
        "--screenshot, runs headless for --seconds (default 30), saving a\n",
        "screenshot every --every seconds (default 2). With --window, opens a\n",
        "live, playable window redrawn at the CPC's own ~50Hz, keyboard included\n",
        "- close the window to quit."
    ));
}

fn insert_disc(path: &str, drive: FdcDrive, motherboard: &mut amsrust::motherboard::Motherboard) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let image = load_dsk(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    motherboard.fdc_mut().insert_image(drive, Some(image));
}

fn main() {
    let args = parse_args();

    let mut motherboard = MotherboardBuilder::default().set_crtc(args.crtc).enable_interruptions().build();

    if let Some(path) = &args.dsk_a {
        insert_disc(path, FdcDrive::A, &mut motherboard);
    }
    if let Some(path) = &args.dsk_b {
        insert_disc(path, FdcDrive::B, &mut motherboard);
    }
    if let Some(path) = &args.sna {
        amsrust::snapshot::load_sna(&mut motherboard, path).unwrap_or_else(|e| panic!("{e}"));
    }

    match args.screenshot_dir {
        Some(dir) => run_headless(&mut motherboard, &dir, args.seconds, args.every),
        None if args.window => run_window(&mut motherboard),
        None => {
            let mut handle = |_motherboard: &mut amsrust::motherboard::Motherboard| false;
            let mut ctx = MonitorContext::new(Box::new(&mut handle));
            Monitor::cli_monitoring(&mut motherboard, &mut ctx);
        }
    }
}

/// T-states per second at the CPC's 4MHz clock.
const T_STATES_PER_SECOND: usize = 4_000_000;

fn run_headless(motherboard: &mut amsrust::motherboard::Motherboard, dir: &str, seconds: u64, every: u64) {
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{dir}: {e}"));

    let target_t_states = seconds as usize * T_STATES_PER_SECOND;
    let every_t_states = (every.max(1) as usize * T_STATES_PER_SECOND).max(1);
    // The first screenshot is taken as soon as a frame is available (0
    // isn't reached by running first), the rest paced `every` seconds
    // apart from there.
    let mut next_shot_at = 0;
    let mut shot = 0usize;

    while motherboard.t_state_counter().value() < target_t_states {
        while motherboard.t_state_counter().value() < next_shot_at.min(target_t_states) {
            // between screenshots, run without collecting video (cheaper)
            motherboard.execute_next_opcode_no_video();
        }
        if motherboard.t_state_counter().value() >= target_t_states {
            break;
        }

        let mut grabber = FrameGrabber::default();
        let mut frame = None;
        // Capture the next whole frame once due, rather than every tick:
        // matches how the CSL runner and play_demo grab a screenshot.
        while frame.is_none() && motherboard.t_state_counter().value() < target_t_states {
            motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                grabber.receive(rgb, sync);
                // take_frame() consumes it: a later tick within this same
                // multi-tick opcode would otherwise immediately overwrite
                // a just-captured frame with the None the second take gives.
                if frame.is_none() {
                    frame = grabber.take_frame();
                }
            });
        }
        let Some(frame) = frame
        else {
            break; // ran out of time mid-frame
        };

        let path = format!("{dir}/frame_{shot:04}.png");
        frame.save(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!("[{:>6}s] {path}", motherboard.t_state_counter().value() / T_STATES_PER_SECOND);
        shot += 1;
        next_shot_at = motherboard.t_state_counter().value() + every_t_states;
    }
    eprintln!("{shot} screenshot(s) over {seconds}s emulated, saved to {dir}/");
}

/// A generous upper bound on a captured frame's size (see
/// `amsrust::monitor`'s own, private `GRABBER_WIDTH`/`MONITOR_HEIGHT`,
/// which this can't reference across the bin/lib crate boundary): the
/// window is created at this fixed size once, and every captured frame -
/// whatever its actual dimensions, which can vary a little between modes -
/// is blitted into a buffer of exactly this size, so `update_with_buffer`
/// always gets what the window was created with.
const WINDOW_WIDTH: usize = 52 * 8 * 2; // 832
/// Each scanline is drawn twice, matching the vertical doubling
/// `amsrust::monitor`'s own ideal-monitor `Texture` already does, for a
/// roughly correct 4:3 aspect instead of a squashed one.
const WINDOW_HEIGHT: usize = 312 * 2;

fn run_window(motherboard: &mut Motherboard) {
    let mut window = Window::new("amsrust", WINDOW_WIDTH, WINDOW_HEIGHT, WindowOptions {
        resize: true,
        ..WindowOptions::default()
    })
    .unwrap_or_else(|e| panic!("failed to open a window: {e}"));
    // Cap redraws (and so input polling and emulated speed) at the CPC's
    // own real refresh rate rather than running full tilt.
    window.set_target_fps(50);

    let mut buffer = vec![0u32; WINDOW_WIDTH * WINDOW_HEIGHT];

    while window.is_open() {
        update_keyboard(&window, motherboard);

        let mut grabber = FrameGrabber::default();
        let mut frame = None;
        // One whole frame per redraw, same as run_headless()/csl_runner's
        // own screenshot(): locked to the emulator's real VSYNC, not to a
        // fixed t-state count.
        while frame.is_none() {
            motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                grabber.receive(rgb, sync);
                if frame.is_none() {
                    frame = grabber.take_frame();
                }
            });
        }
        let Some(frame) = frame
        else {
            break;
        };

        blit_doubled(&frame, &mut buffer);
        window
            .update_with_buffer(&buffer, WINDOW_WIDTH, WINDOW_HEIGHT)
            .unwrap_or_else(|e| panic!("failed to redraw the window: {e}"));
    }
}

/// Copy `frame` into `buffer` (a `WINDOW_WIDTH`x`WINDOW_HEIGHT` canvas,
/// minifb's expected 0RGB per pixel), doubling each scanline vertically and
/// letterboxing in black whatever `frame` doesn't cover - it can be a little
/// smaller than the canvas (see `WINDOW_WIDTH`/`WINDOW_HEIGHT`'s own doc).
fn blit_doubled(frame: &image::RgbImage, buffer: &mut [u32]) {
    buffer.fill(0);
    let width = (frame.width() as usize).min(WINDOW_WIDTH);
    let height = (frame.height() as usize).min(WINDOW_HEIGHT / 2);
    for y in 0..height {
        for x in 0..width {
            let pixel = frame.get_pixel(x as u32, y as u32);
            let packed = (pixel[0] as u32) << 16 | (pixel[1] as u32) << 8 | pixel[2] as u32;
            buffer[(y * 2) * WINDOW_WIDTH + x] = packed;
            buffer[(y * 2 + 1) * WINDOW_WIDTH + x] = packed;
        }
    }
}

/// Set every mapped [`CpcKey`]'s pressed state from what's currently held
/// down on the host keyboard - called once per redraw, so this is polled at
/// the CPC's own ~50Hz rather than per-event.
///
/// Several host keys map to the same [`CpcKey`] (both Shift keys, both Ctrl
/// keys, both Alt keys), so a naive "call `set_pressed` once per table
/// entry, in order" gets this wrong: whichever entry for a given `CpcKey`
/// happens to be *last* in the table unconditionally decides its final
/// state, silently overriding an earlier entry that *was* held. E.g. with
/// `(LeftShift, Shift)` before `(RightShift, Shift)`, holding only the left
/// Shift key got immediately cancelled by the RightShift entry (not held)
/// processed right after it - Shift would never actually reach the CPC
/// unless the right-hand key specifically was used, breaking every
/// Shift-combo character (`"` included: Shift+2 on this table) for anyone
/// who naturally reaches for the left one. Each `CpcKey` must instead be
/// OR-combined across every host key mapped to it.
fn update_keyboard(window: &Window, motherboard: &mut Motherboard) {
    let matrix = motherboard.keyboard_mut();
    for key in HOST_KEY_MAP {
        matrix.set_pressed(key.1, any_host_key_pressed(HOST_KEY_MAP, key.1, |k| window.is_key_down(k)));
    }
}

/// Whether `target` should be considered pressed: any host key `map`s to it
/// that `is_down` reports held. Pulled out of `update_keyboard` so the
/// OR-combining logic itself (see that function's doc comment) is
/// unit-testable without a real `Window` - there's no display in CI/this
/// sandbox to interactively verify a keyboard fix against.
fn any_host_key_pressed(map: &[(Key, CpcKey)], target: CpcKey, is_down: impl Fn(Key) -> bool) -> bool {
    map.iter().filter(|(_, cpc)| *cpc == target).any(|(host, _)| is_down(*host))
}

#[cfg(test)]
mod keyboard_test {
    use super::*;

    /// The exact bug this whole refactor exists to prevent regressing:
    /// holding only ONE of two host keys mapped to the same `CpcKey` must
    /// still register as pressed, not get cancelled by the other, unheld,
    /// one - see `update_keyboard`'s doc comment for the real symptom this
    /// caused (Shift-combo characters, `"` included, not typeable unless
    /// the right-hand modifier specifically was used).
    #[test]
    fn holding_just_one_mapped_host_key_is_not_cancelled_by_the_other_unheld_one() {
        let map: &[(Key, CpcKey)] = &[(Key::LeftShift, CpcKey::Shift), (Key::RightShift, CpcKey::Shift)];
        assert!(any_host_key_pressed(map, CpcKey::Shift, |k| k == Key::LeftShift), "left alone should register");
        assert!(any_host_key_pressed(map, CpcKey::Shift, |k| k == Key::RightShift), "right alone should register");
        assert!(any_host_key_pressed(map, CpcKey::Shift, |_| true), "both together should register");
        assert!(!any_host_key_pressed(map, CpcKey::Shift, |_| false), "neither held should not register");
    }

    /// Both Alt keys act as the CPC's own COPY key.
    #[test]
    fn alt_is_mapped_to_copy() {
        assert!(HOST_KEY_MAP.contains(&(Key::LeftAlt, CpcKey::Copy)));
        assert!(HOST_KEY_MAP.contains(&(Key::RightAlt, CpcKey::Copy)));
    }

    /// `"` is Shift+2 on a real CPC keyboard (see `keys_of_char('"')` in
    /// `csl_runner.rs`, already covered on the CSL/scripted-input side) -
    /// this is the interactive-input counterpart: the host keys that
    /// should combine to reach it are both mapped and distinct.
    #[test]
    fn double_quote_is_reachable_via_shift_plus_2() {
        assert!(HOST_KEY_MAP.iter().any(|(h, c)| *h == Key::Key2 && *c == CpcKey::Digit2));
        assert!(HOST_KEY_MAP.iter().any(|(h, c)| (*h == Key::LeftShift || *h == Key::RightShift) && *c == CpcKey::Shift));
    }
}

/// Host key -> CPC key, per `amsrust::keyboard`'s own contract (see its doc
/// comment): this table is the only thing that would need to change if the
/// windowing/input library changes. A first pass covering the obvious keys;
/// a real CPC keyboard's punctuation doesn't line up 1:1 with a modern one,
/// so a few of these (`Colon`, `At`, `Caret`) are best-effort placements
/// rather than a faithful physical-layout match.
const HOST_KEY_MAP: &[(Key, CpcKey)] = {
    use CpcKey::*;
    &[
        (Key::A, A), (Key::B, B), (Key::C, C), (Key::D, D), (Key::E, E),
        (Key::F, F), (Key::G, G), (Key::H, H), (Key::I, I), (Key::J, J),
        (Key::K, K), (Key::L, L), (Key::M, M), (Key::N, N), (Key::O, O),
        (Key::P, P), (Key::Q, Q), (Key::R, R), (Key::S, S), (Key::T, T),
        (Key::U, U), (Key::V, V), (Key::W, W), (Key::X, X), (Key::Y, Y),
        (Key::Z, Z),
        (Key::Key0, Digit0), (Key::Key1, Digit1), (Key::Key2, Digit2),
        (Key::Key3, Digit3), (Key::Key4, Digit4), (Key::Key5, Digit5),
        (Key::Key6, Digit6), (Key::Key7, Digit7), (Key::Key8, Digit8),
        (Key::Key9, Digit9),
        (Key::Space, Space),
        (Key::Enter, Return),
        (Key::LeftShift, Shift), (Key::RightShift, Shift),
        (Key::LeftCtrl, Control), (Key::RightCtrl, Control),
        (Key::LeftAlt, Copy), (Key::RightAlt, Copy),
        (Key::CapsLock, CapsLock),
        (Key::Tab, Tab),
        (Key::Escape, Esc),
        (Key::Backspace, Del),
        (Key::Delete, Clr),
        (Key::Up, CursorUp), (Key::Down, CursorDown),
        (Key::Left, CursorLeft), (Key::Right, CursorRight),
        (Key::Minus, Minus),
        (Key::LeftBracket, OpenBracket), (Key::RightBracket, CloseBracket),
        (Key::Backslash, Backslash),
        (Key::Semicolon, Semicolon),
        (Key::Apostrophe, Colon),
        (Key::Comma, Comma),
        (Key::Period, Dot),
        (Key::Slash, Slash),
        (Key::Backquote, Caret),
        (Key::Equal, At),
        (Key::NumPad0, Keypad0), (Key::NumPad1, Keypad1),
        (Key::NumPad2, Keypad2), (Key::NumPad3, Keypad3),
        (Key::NumPad4, Keypad4), (Key::NumPad5, Keypad5),
        (Key::NumPad6, Keypad6), (Key::NumPad7, Keypad7),
        (Key::NumPad8, Keypad8), (Key::NumPad9, Keypad9),
        (Key::NumPadDot, KeypadDot),
        (Key::NumPadEnter, KeypadEnter),
    ]
};
