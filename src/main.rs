//! `amsrust`: boots a machine with the given CRTC/disc(s)/snapshot, for
//! manually testing the emulator against real-world software.
//!
//! ```text
//! amsrust [--crtc 0-4] [--dsk path] [--dsk-b path] [--sna path]
//!         [--screenshot dir [--seconds N] [--every N]] [--window [--mute]]
//!         [--record file [--seconds N]] [--type text [--type-at N]]
//!         [--debug-server [--debug-port N]]
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
//! Sound: built with `--features audio`, `--window` plays the PSG on the
//! default audio output device (see `host_audio.rs`), and the device's
//! clock then sets the emulation speed; `--mute` turns it off. Without the
//! feature the window is silent.
//!
//! With `--record <file>` (e.g. `capture.mp4`), records a video, picture
//! and sound, through `ffmpeg` (which must be installed; the extension picks
//! the format, see `recorder.rs`). With `--window`, records what is played
//! until the window is closed; otherwise runs headless for `--seconds`
//! (default 30) of emulated time. Ctrl+C (or SIGTERM) ends a recording
//! early, the file being finished properly.
//!
//! `--type <text>` types `text` on the CPC keyboard once `--type-at`
//! seconds (default 3) of emulated time have passed ("\n" is RETURN), e.g.
//! `--dsk demo.dsk --type 'run"demo\n' --record demo.mp4`. Not in the
//! interactive monitor.
//!
//! Breakpoints stored in the snapshot by an assembler or another emulator
//! (REMU, ACE "BRKC", WinAPE "WABP" and "BRKS" chunks, see `amsrust::snapshot`) are
//! set in the interactive monitor.
//!
//! `--debug-server` starts the HTTP debug server compatible with the
//! AMSpiriT-Lite web API on `127.0.0.1:6128` (`--debug-port` to change it),
//! see `amsrust::debug_server`: with `--window` the window is driven by it,
//! otherwise the machine runs headless in real time (Ctrl+C to quit).
//!
//! `--sna` restores CPU registers, the base 64K of RAM, the CRTC's
//! registers, the gate array's ink/mode/ROM-paging state and the PSG's
//! registers (see `amsrust::snapshot`); apply it after `--dsk` if the
//! software expects a particular disc already in the drive.

use amsrust::crtc::CrtcType;
use amsrust::csl_runner::keys_of_char;
use amsrust::debug::{Monitor, MonitorContext};
use amsrust::disc_image::load_dsk;
use amsrust::fdc::FdcDrive;
use amsrust::keyboard::CpcKey;
use amsrust::monitor::Monitor as CrtMonitor;
use amsrust::motherboard::{CpcModel, Motherboard, MotherboardBuilder};
use amsrust::debug_server::{Beam, DebugServer};
use amsrust::psg::AudioConfig;
use minifb::{InputCallback, Key, Window, WindowOptions};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

mod host_audio;
mod recorder;

struct Args {
    crtc: CrtcType,
    model: CpcModel,
    /// Total RAM in KB (64 = no extra page), if not the model's.
    ram_kb: Option<usize>,
    /// Disc interface (a DDI-1 on a 464), if not the model's.
    disc_interface: Option<bool>,
    dsk_a: Option<String>,
    dsk_b: Option<String>,
    sna: Option<String>,
    screenshot_dir: Option<String>,
    seconds: u64,
    every: u64,
    window: bool,
    mute: bool,
    record: Option<String>,
    debug_server: bool,
    debug_port: u16,
    text: Option<String>,
    type_at: f64
}

fn parse_args() -> Args {
    let mut args = Args {
        crtc: CrtcType::default(),
        model: CpcModel::default(),
        ram_kb: None,
        disc_interface: None,
        dsk_a: None,
        dsk_b: None,
        sna: None,
        screenshot_dir: None,
        seconds: 30,
        every: 2,
        window: false,
        mute: false,
        record: None,
        debug_server: false,
        debug_port: amsrust::debug_server::DEFAULT_PORT,
        text: None,
        type_at: 3.0
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
            "--model" => {
                args.model = next("--model").parse().unwrap_or_else(|e: String| panic!("--model: {e}"))
            },
            "--ram" => {
                let kb: usize = next("--ram").parse().expect("--ram must be a size in KB");
                assert!(kb >= 64 && kb % 64 == 0 && kb <= 576, "--ram must be 64, 128, ... 576 (KB)");
                args.ram_kb = Some(kb);
            },
            "--ddi" => args.disc_interface = Some(true),
            "--no-disc" => args.disc_interface = Some(false),
            "--dsk" => args.dsk_a = Some(next("--dsk")),
            "--dsk-b" => args.dsk_b = Some(next("--dsk-b")),
            "--sna" => args.sna = Some(next("--sna")),
            "--screenshot" => args.screenshot_dir = Some(next("--screenshot")),
            "--seconds" => args.seconds = next("--seconds").parse().expect("--seconds must be a number"),
            "--every" => args.every = next("--every").parse().expect("--every must be a number"),
            "--window" => args.window = true,
            "--mute" => args.mute = true,
            "--record" => args.record = Some(next("--record")),
            "--debug-server" => args.debug_server = true,
            "--debug-port" => {
                args.debug_server = true;
                args.debug_port = next("--debug-port").parse().expect("--debug-port must be a port number")
            },
            "--type" => args.text = Some(next("--type").replace("\\n", "\n")),
            "--type-at" => args.type_at = next("--type-at").parse().expect("--type-at must be a number"),
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
        "amsrust [--model 464|664|6128] [--ram KB] [--ddi|--no-disc] [--crtc 0-4]\n",
        "        [--dsk path] [--dsk-b path] [--sna path]\n",
        "        [--screenshot dir [--seconds N] [--every N]] [--window [--mute]]\n",
        "        [--record file [--seconds N]] [--type text [--type-at N]]\n",
        "        [--debug-server [--debug-port N]]\n",
        "\n",
        "--model picks the ROMs (default 6128). The 464 and 664 have 64K and the\n",
        "6128 128K: --ram 128 (up to 576) adds the extension banked as a 6128's.\n",
        "The 664 and 6128 have a disc interface; --ddi adds a DDI-1 (AMSDOS) to\n",
        "a 464, --no-disc removes it.\n",
        "With no --screenshot/--window, drops into the interactive monitor once\n",
        "everything is loaded ('h' at its prompt lists its commands). With\n",
        "--screenshot, runs headless for --seconds (default 30), saving a\n",
        "screenshot every --every seconds (default 2). With --window, opens a\n",
        "live, playable window redrawn at the CPC's own ~50Hz, keyboard included\n",
        "- close the window to quit. Built with --features audio, the window\n",
        "plays the sound too (--mute to silence it). --record capture.mp4 records\n",
        "a video with sound through ffmpeg: what the window shows until it is\n",
        "closed, or without --window --seconds (default 30) run headless.\n",
        "--type 'run\"demo\\n' types on the CPC keyboard after --type-at seconds\n",
        "(default 3). Breakpoints of a snapshot's REMU/BRKC/BRKS/WABP chunks are set\n",
        "in the interactive monitor.\n",
        "--debug-server serves the AMSpiriT-Lite compatible debug API on\n",
        "127.0.0.1:6128 (GET /api/doc lists the endpoints), with --window or\n",
        "headless."
    ));
}

fn insert_disc(path: &str, drive: FdcDrive, motherboard: &mut amsrust::motherboard::Motherboard) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let image = load_dsk(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
    motherboard.fdc_mut().insert_image(drive, Some(image));
}

/// Set by Ctrl+C / SIGTERM: the window and headless recording loops stop at
/// the next frame, so a recording is still finished properly.
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn stop_requested() -> bool {
    STOP.load(std::sync::atomic::Ordering::Relaxed)
}

fn main() {
    let args = parse_args();
    if args.record.is_some() {
        // Only when recording: elsewhere (the interactive monitor) Ctrl+C
        // keeps its usual meaning.
        ctrlc::set_handler(|| STOP.store(true, std::sync::atomic::Ordering::Relaxed))
            .expect("cannot install the Ctrl+C handler");
    }

    let mut builder = MotherboardBuilder::default();
    builder.set_crtc(args.crtc).set_model(args.model).enable_interruptions();
    if let Some(kb) = args.ram_kb {
        builder.set_extra_ram_pages(kb / 64 - 1);
    }
    if let Some(fitted) = args.disc_interface {
        builder.set_disc_interface(fitted);
    }
    let mut motherboard = builder.build();
    if (args.dsk_a.is_some() || args.dsk_b.is_some()) && !motherboard.has_disc_interface() {
        eprintln!("--dsk: this {} has no disc interface (--ddi adds one)", args.model.name());
    }

    if let Some(path) = &args.dsk_a {
        insert_disc(path, FdcDrive::A, &mut motherboard);
    }
    if let Some(path) = &args.dsk_b {
        insert_disc(path, FdcDrive::B, &mut motherboard);
    }
    let mut breakpoints = amsrust::snapshot::SnapshotBreakpoints::default();
    if let Some(path) = &args.sna {
        amsrust::snapshot::load_sna(&mut motherboard, path).unwrap_or_else(|e| panic!("{e}"));
        if motherboard.model() != args.model {
            eprintln!("{path}: a CPC {} snapshot, run as such", motherboard.model().name());
        }
        breakpoints = amsrust::snapshot::load_breakpoints(path).unwrap_or_else(|e| panic!("{e}"));
        for what in &breakpoints.ignored {
            eprintln!("snapshot breakpoint not supported: {what}");
        }
    }
    let monitor_mode =
        args.screenshot_dir.is_none() && !args.window && args.record.is_none() && !args.debug_server;
    if !breakpoints.is_empty() && !monitor_mode && !args.debug_server {
        eprintln!(
            "{} snapshot breakpoint(s), used by the interactive monitor and the debug server only",
            breakpoints.len()
        );
    }
    let mut typist = args.text.as_deref().map(|text| Typist::new(text, args.type_at));
    if typist.is_some() && monitor_mode {
        eprintln!("--type is ignored in the interactive monitor");
    }

    match args.screenshot_dir {
        Some(dir) => run_headless(&mut motherboard, &dir, args.seconds, args.every, &mut typist),
        None if args.window => {
            let mut server = start_debug_server(&args, &breakpoints, &mut motherboard);
            run_window(&mut motherboard, !args.mute, args.record.as_deref(), &mut typist, server.as_mut())
        },
        None if args.debug_server => {
            let mut server = start_debug_server(&args, &breakpoints, &mut motherboard).unwrap();
            run_debug_server(&mut motherboard, &mut server, &mut typist)
        },
        None if args.record.is_some() => {
            run_recording(&mut motherboard, args.record.as_deref().unwrap(), args.seconds, &mut typist)
        },
        None => {
            let mut handle = |_motherboard: &mut amsrust::motherboard::Motherboard| false;
            let mut ctx = MonitorContext::new(Box::new(&mut handle));
            for breakpoint in &breakpoints.exec {
                eprintln!("breakpoint at {:#06x} (from the snapshot's {} chunk)", breakpoint.address, breakpoint.chunk);
                ctx.add_breakpoint(amsrust::debug::BreakPoint::Address(breakpoint.address));
            }
            if !breakpoints.advanced.is_empty() {
                eprintln!(
                    "{} advanced (memory/IO/watch) breakpoint(s) need --debug-server",
                    breakpoints.advanced.len()
                );
            }
            Monitor::cli_monitoring(&mut motherboard, &mut ctx);
        }
    }
}

/// T-states per second at the CPC's 4MHz clock.
const T_STATES_PER_SECOND: usize = 4_000_000;

/// `--type`: the text typed on the CPC keyboard, one character after the
/// other, each held for 2 frames then released for 2 frames.
struct Typist {
    keys: VecDeque<Vec<CpcKey>>,
    /// T-state at which the next press or release is due.
    next_at: usize,
    pressed: Option<Vec<CpcKey>>
}

/// 40ms: 2 frames, long enough for the firmware's keyboard scan.
const KEY_T_STATES: usize = T_STATES_PER_SECOND / 25;

impl Typist {
    fn new(text: &str, at_seconds: f64) -> Self {
        let keys = text
            .chars()
            .filter_map(|c| {
                let keys = keys_of_char(c);
                if keys.is_none() {
                    eprintln!("--type: cannot type {c:?}, skipped");
                }
                keys
            })
            .collect();
        Self { keys, next_at: (at_seconds * T_STATES_PER_SECOND as f64) as usize, pressed: None }
    }

    /// Press or release the keys that are due. Cheap: call it as often as
    /// wanted (every instruction or every frame).
    fn update(&mut self, motherboard: &mut Motherboard) {
        let now = motherboard.t_state_counter().value();
        if now < self.next_at {
            // Held keys stay pressed even if something else (the window's
            // host keyboard) released them meanwhile.
            for &key in self.pressed.iter().flatten() {
                motherboard.keyboard_mut().set_pressed(key, true);
            }
            return;
        }
        let (keys, pressed) = match self.pressed.take() {
            Some(keys) => (keys, false),
            None => match self.keys.pop_front() {
                Some(keys) => (keys, true),
                None => return
            }
        };
        for &key in &keys {
            motherboard.keyboard_mut().set_pressed(key, pressed);
        }
        if pressed {
            self.pressed = Some(keys);
        }
        self.next_at = now + KEY_T_STATES;
    }
}

fn type_due_keys(typist: &mut Option<Typist>, motherboard: &mut Motherboard) {
    if let Some(typist) = typist {
        typist.update(motherboard);
    }
}

fn run_headless(
    motherboard: &mut amsrust::motherboard::Motherboard,
    dir: &str,
    seconds: u64,
    every: u64,
    typist: &mut Option<Typist>
) {
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
            type_due_keys(typist, motherboard);
        }
        if motherboard.t_state_counter().value() >= target_t_states {
            break;
        }

        let mut monitor = CrtMonitor::default();
        let mut frame = None;
        // Capture the next whole frame once due, rather than every tick:
        // matches how the CSL runner and play_demo grab a screenshot.
        while frame.is_none() && motherboard.t_state_counter().value() < target_t_states {
            type_due_keys(typist, motherboard);
            motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                monitor.receive(rgb, sync);
                // take_frame() consumes it: a later tick within this same
                // multi-tick opcode would otherwise immediately overwrite
                // a just-captured frame with the None the second take gives.
                if frame.is_none() {
                    frame = monitor.take_frame();
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

/// The format the PSG is recorded in when no audio device dictates it.
const RECORD_AUDIO: fn() -> AudioConfig = || AudioConfig::stereo(48_000);

fn start_recorder(path: &str, audio: Option<AudioConfig>) -> recorder::Recorder {
    recorder::Recorder::start(std::path::Path::new(path), WINDOW_WIDTH, WINDOW_HEIGHT / 2, audio)
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1)
        })
}

fn finish_recorder(recorder: recorder::Recorder, path: &str) {
    match recorder.finish() {
        Ok(frames) => eprintln!("{path}: {frames} frames ({:.1}s)", frames as f64 * 312.0 / 15625.0),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    }
}

/// `--record` without `--window`: run `seconds` of emulated time headless,
/// recording every frame and the sound.
fn run_recording(motherboard: &mut Motherboard, path: &str, seconds: u64, typist: &mut Option<Typist>) {
    motherboard.set_audio_output(Some(RECORD_AUDIO()));
    let mut recorder = start_recorder(path, Some(RECORD_AUDIO()));
    let target_t_states = seconds as usize * T_STATES_PER_SECOND;
    let mut monitor = CrtMonitor::default();
    while motherboard.t_state_counter().value() < target_t_states && !stop_requested() {
        type_due_keys(typist, motherboard);
        let mut frame = None;
        motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
            monitor.receive(rgb, sync);
            if frame.is_none() {
                frame = monitor.take_frame();
            }
        });
        if let Some(frame) = frame {
            let recorded = recorder
                .samples(&motherboard.take_audio_samples())
                .and_then(|()| recorder.frame(&frame));
            if let Err(e) = recorded {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    // The sound of the last, incomplete frame is not recorded: the video
    // ends with the last whole frame.
    finish_recorder(recorder, path);
}

/// A generous upper bound on a captured frame's size (see
/// `amsrust::monitor`'s own, private `GRABBER_WIDTH`/`MONITOR_HEIGHT`,
/// which this can't reference across the bin/lib crate boundary): the
/// window is created at this fixed size once, and every captured frame -
/// whatever its actual dimensions, which can vary a little between modes -
/// is blitted into a buffer of exactly this size, so `update_with_buffer`
/// always gets what the window was created with.
const WINDOW_WIDTH: usize = 48 * 8 * 2; // 768
/// Each scanline is drawn twice, matching the vertical doubling
/// `amsrust::monitor`'s own ideal-monitor `Texture` already does, for a
/// roughly correct 4:3 aspect instead of a squashed one.
const WINDOW_HEIGHT: usize = 272 * 2;

/// Start the debug server if asked, with the snapshot's breakpoints set.
fn start_debug_server(
    args: &Args,
    breakpoints: &amsrust::snapshot::SnapshotBreakpoints,
    motherboard: &mut Motherboard
) -> Option<DebugServer> {
    if !args.debug_server {
        return None;
    }
    match DebugServer::start(args.debug_port) {
        Ok(mut server) => {
            eprintln!("debug server: http://127.0.0.1:{}/api/doc", args.debug_port);
            for breakpoint in &breakpoints.exec {
                server.add_exec_breakpoint(breakpoint.address, motherboard);
            }
            for breakpoint in &breakpoints.advanced {
                server.add_advanced_breakpoint(breakpoint.clone(), motherboard);
            }
            Some(server)
        },
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    }
}

/// Run until the next whole frame, or until the debug server pauses the
/// CPU (breakpoint, step) - then `None`, the monitor keeping the part of
/// the frame already drawn.
fn run_frame(
    motherboard: &mut Motherboard,
    monitor: &mut CrtMonitor,
    mut server: Option<&mut DebugServer>
) -> Option<image::RgbImage> {
    let mut frame = None;
    while frame.is_none() {
        motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
            monitor.receive(rgb, sync);
            if frame.is_none() {
                frame = monitor.take_frame();
            }
        });
        if let Some(server) = server.as_deref_mut() {
            let (x, line) = monitor.beam();
            let (crop_x, crop_y) = monitor.crop_origin();
            server.set_beam(Beam { x, line, crop_x, crop_y });
            server.after_instruction(motherboard);
            if let Some(pc) = server.take_stop() {
                eprintln!("debug server: breakpoint at {pc:#06x}");
            }
            if server.paused() {
                break;
            }
        }
    }
    if let (Some(frame), Some(server)) = (&frame, server) {
        server.frame_completed(motherboard);
        server.set_frame(frame);
    }
    frame
}

/// `--debug-server` without a window: run in real time, driven by the
/// debug API (Ctrl+C to quit).
fn run_debug_server(motherboard: &mut Motherboard, server: &mut DebugServer, typist: &mut Option<Typist>) {
    ctrlc::set_handler(|| STOP.store(true, std::sync::atomic::Ordering::Relaxed)).ok();
    let mut monitor = CrtMonitor::default();
    let frame_duration = std::time::Duration::from_micros(19_968);
    let mut deadline = std::time::Instant::now();
    while !stop_requested() && !server.quit_requested() {
        server.poll(motherboard);
        if server.paused() {
            std::thread::sleep(std::time::Duration::from_millis(2));
            deadline = std::time::Instant::now();
            continue;
        }
        type_due_keys(typist, motherboard);
        if run_frame(motherboard, &mut monitor, Some(server)).is_some() {
            // The CPC's own pace.
            deadline += frame_duration;
            let now = std::time::Instant::now();
            if deadline > now {
                std::thread::sleep(deadline - now);
            }
            else {
                deadline = now;
            }
        }
    }
}

fn run_window(
    motherboard: &mut Motherboard,
    sound: bool,
    record: Option<&str>,
    typist: &mut Option<Typist>,
    mut server: Option<&mut DebugServer>
) {
    let mut window = Window::new("amsrust", WINDOW_WIDTH, WINDOW_HEIGHT, WindowOptions {
        resize: true,
        ..WindowOptions::default()
    })
    .unwrap_or_else(|e| panic!("failed to open a window: {e}"));
    // Cap redraws (and so input polling and emulated speed) at the CPC's
    // own real refresh rate rather than running full tilt.
    window.set_target_fps(50);

    let audio = open_audio(sound, motherboard);
    if audio.is_some() {
        // The audio device's clock paces the emulation instead.
        window.set_target_fps(0);
    }
    // The recording takes the sound in the device's format if it plays it.
    let mut recorder = record.map(|path| {
        let config = motherboard.psg().audio_config().unwrap_or_else(|| {
            motherboard.set_audio_output(Some(RECORD_AUDIO()));
            RECORD_AUDIO()
        });
        start_recorder(path, Some(config))
    });

    let typed_chars = TypedChars::default();
    window.set_input_callback(Box::new(typed_chars.clone()));
    let mut typed_combo = TypedCombo::default();

    let mut buffer = vec![0u32; WINDOW_WIDTH * WINDOW_HEIGHT];

    // One whole frame per redraw, locked to the emulator's real VSYNC, not
    // to a fixed t-state count.
    let mut monitor = CrtMonitor::default();
    while window.is_open() && !stop_requested() {
        update_keyboard(&window, motherboard, typed_combo.advance(&typed_chars));
        // After the host keyboard, which would release the typed keys.
        type_due_keys(typist, motherboard);

        if let Some(server) = server.as_deref_mut() {
            server.poll(motherboard);
            if server.quit_requested() {
                break;
            }
            if server.paused() {
                // Stopped by the debugger: keep the window alive.
                window
                    .update_with_buffer(&buffer, WINDOW_WIDTH, WINDOW_HEIGHT)
                    .unwrap_or_else(|e| panic!("failed to redraw the window: {e}"));
                std::thread::sleep(std::time::Duration::from_millis(15));
                continue;
            }
        }
        let Some(frame) = run_frame(motherboard, &mut monitor, server.as_deref_mut())
        else {
            continue; // paused mid-frame
        };

        let samples = motherboard.take_audio_samples();
        if let Some(recorder) = &mut recorder {
            let recorded = recorder.samples(&samples).and_then(|()| recorder.frame(&frame));
            if let Err(e) = recorded {
                eprintln!("recording stopped: {e}");
                break;
            }
        }
        play_frame_audio(audio.as_ref(), &samples, &window);

        blit_doubled(&frame, &mut buffer);
        window
            .update_with_buffer(&buffer, WINDOW_WIDTH, WINDOW_HEIGHT)
            .unwrap_or_else(|e| panic!("failed to redraw the window: {e}"));
    }

    if let (Some(recorder), Some(path)) = (recorder, record) {
        finish_recorder(recorder, path);
    }

    #[cfg(feature = "audio")]
    if let Some(audio) = &audio {
        let missing = audio.missing_frames();
        if missing > 0 {
            eprintln!(
                "sound: {:.1}s of gaps in total (the emulation ran slower than real time)",
                missing as f64 / audio.config().sample_rate as f64
            );
        }
    }
}

#[cfg(feature = "audio")]
type AudioOut = host_audio::AudioOut;
/// Stands for the audio device when built without the `audio` feature.
#[cfg(not(feature = "audio"))]
type AudioOut = ();

/// Open the default audio output and make the PSG produce samples for it.
/// `None` (silent window) when `sound` is off, the `audio` feature is not
/// built in or no device can be opened.
fn open_audio(sound: bool, motherboard: &mut Motherboard) -> Option<AudioOut> {
    if !sound {
        return None;
    }
    #[cfg(feature = "audio")]
    {
        match host_audio::AudioOut::open() {
            Ok(audio) => {
                let config = audio.config();
                eprintln!(
                    "sound: {}Hz {}",
                    config.sample_rate,
                    if config.stereo { "stereo" } else { "mono" }
                );
                motherboard.set_audio_output(Some(config));
                Some(audio)
            },
            Err(error) => {
                eprintln!("sound disabled: {error}");
                None
            }
        }
    }
    #[cfg(not(feature = "audio"))]
    {
        let _ = motherboard;
        None
    }
}

/// Hand the frame's samples to the device, then wait while enough sound is
/// already queued: this keeps the emulation in step with the device.
#[allow(unused_variables)]
fn play_frame_audio(audio: Option<&AudioOut>, samples: &[f32], window: &Window) {
    #[cfg(feature = "audio")]
    if let Some(audio) = audio {
        audio.push(samples);
        while audio.buffered() > host_audio::TARGET_LATENCY && window.is_open() {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
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

/// Queue of Unicode code points typed through minifb's character callback
/// (`InputCallback::add_char`), fed by the OS's real, layout-and-modifier-
/// aware text input resolution (`Xutf8LookupString` on X11) - a `Clone`
/// handle sharing the same underlying queue, since minifb needs to own a
/// boxed callback while `run_window`'s loop also needs to drain it.
#[derive(Default, Clone)]
struct TypedChars(Rc<RefCell<VecDeque<u32>>>);

impl InputCallback for TypedChars {
    fn add_char(&mut self, uni_char: u32) {
        self.0.borrow_mut().push_back(uni_char);
    }
}

impl TypedChars {
    /// The next queued character, if any (silently skipping any code point
    /// that isn't valid UTF-32, which shouldn't happen in practice).
    fn pop(&self) -> Option<char> {
        loop {
            let code = self.0.borrow_mut().pop_front()?;
            if let Some(c) = char::from_u32(code) {
                return Some(c);
            }
        }
    }
}

/// Holds a [`keys_of_char`]-resolved key combo "pressed" for a few frames
/// whenever a character arrives through minifb's host-layout-aware char
/// callback, on top of `HOST_KEY_MAP`'s direct physical-key polling.
///
/// Needed because minifb's `Key` enum resolves a physical key's *base*
/// (unshifted) character under the host OS's active layout, but always at
/// a fixed shift-level-0 lookup (see minifb 0.28's X11 backend,
/// `src/os/posix/x11.rs`: `XLookupKeysym(&mut ev.key, 0)`) - so it can
/// only ever report keys whose *unshifted* glyph happens to be in its own
/// small, fixed, US/UK-centric keysym table (confirmed by reading that
/// table: no accented letters, no `XK_quotedbl`, nothing beyond a
/// handful of plain ASCII punctuation keysyms). On a French AZERTY host,
/// the digit row's unshifted glyphs are `& é " ' ( - è _ ç à` - digits
/// only exist at the *shifted* level there - so minifb reports nothing
/// at all for those keys, regardless of Shift: not just `"`, but every
/// digit too, while a French layout is active. "Nothing happens" when
/// pressing the key labelled `"` is exactly this, not a timing or focus
/// issue.
///
/// minifb's character callback is backed by `Xutf8LookupString` - the
/// same locale/layout/modifier-aware API any ordinary text field uses -
/// so typed characters are routed through it and `keys_of_char` (the
/// same table CSL scripts already use to turn text into CPC key combos)
/// instead, and the resulting combo is forced on for a few frames: long
/// enough for the CPC firmware's own keyboard scan to notice, short
/// enough to not stall held-key gameplay using the same keys once the
/// physical polling above takes back over.
struct TypedCombo {
    keys: Vec<CpcKey>,
    frames_left: u8,
}

impl Default for TypedCombo {
    fn default() -> Self {
        Self { keys: Vec::new(), frames_left: 0 }
    }
}

impl TypedCombo {
    /// How many frames (at the CPC's own ~50Hz redraw rate) to hold a
    /// typed character's key combo down for.
    const HOLD_FRAMES: u8 = 4;

    /// The combo to force on this frame: continues holding whatever's
    /// already in progress, else starts holding the next queued
    /// character's combo (skipping any character `keys_of_char` doesn't
    /// know how to type), else nothing.
    fn advance(&mut self, typed: &TypedChars) -> &[CpcKey] {
        if self.frames_left == 0 {
            self.keys.clear();
            while let Some(c) = typed.pop() {
                if let Some(keys) = keys_of_char(c) {
                    self.keys = keys;
                    self.frames_left = Self::HOLD_FRAMES;
                    break;
                }
            }
        }
        if self.frames_left > 0 {
            self.frames_left -= 1;
        }
        &self.keys
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
///
/// `typed_combo` additionally forces on whatever [`CpcKey`]s `advance()`
/// says to this frame, on top of the physical polling above - see
/// `TypedCombo`'s doc comment for why `HOST_KEY_MAP`'s physical,
/// `minifb::Key`-based mapping alone can't reach every character on every
/// host keyboard layout.
fn update_keyboard(window: &Window, motherboard: &mut Motherboard, typed_combo: &[CpcKey]) {
    let matrix = motherboard.keyboard_mut();
    for key in HOST_KEY_MAP {
        matrix.set_pressed(key.1, any_host_key_pressed(HOST_KEY_MAP, key.1, |k| window.is_key_down(k)));
    }
    for key in typed_combo {
        matrix.set_pressed(*key, true);
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

    /// The actual bug report: on a French AZERTY host, `minifb::Key`-based
    /// polling alone can never produce `"` (see `TypedCombo`'s doc comment
    /// for why, verified against minifb 0.28's own X11 source) - but the
    /// OS's real character resolution still correctly reports `"` was
    /// typed, regardless of host layout. Simulates that: feed `"` straight
    /// through the char callback, with no `minifb::Key` involved at all,
    /// and check it resolves to the CPC's own Shift+Digit2 and stays
    /// asserted for more than one frame (so the firmware's scan has time
    /// to notice it).
    #[test]
    fn typed_double_quote_resolves_to_shift_plus_digit2_and_holds_for_several_frames() {
        let typed = TypedChars::default();
        typed.0.borrow_mut().push_back('"' as u32);
        let mut combo = TypedCombo::default();

        let first = combo.advance(&typed).to_vec();
        assert_eq!(first, vec![CpcKey::Shift, CpcKey::Digit2]);

        // Still held on a later frame, with nothing new typed meanwhile.
        let second = combo.advance(&typed).to_vec();
        assert_eq!(second, vec![CpcKey::Shift, CpcKey::Digit2]);

        // Eventually releases rather than holding forever.
        let mut released = false;
        for _ in 0..TypedCombo::HOLD_FRAMES {
            if combo.advance(&typed).is_empty() {
                released = true;
                break;
            }
        }
        assert!(released, "a typed combo must eventually release, not stick forever");
    }

    /// A character `keys_of_char` doesn't know how to type (e.g. a control
    /// character minifb's char callback is documented to never send, but
    /// defensively handled anyway) is just skipped, not mistaken for an
    /// empty-but-held combo.
    #[test]
    fn untypeable_character_is_skipped_without_holding_anything() {
        let typed = TypedChars::default();
        typed.0.borrow_mut().push_back(0x01); // unmapped control code
        let mut combo = TypedCombo::default();
        assert!(combo.advance(&typed).is_empty());
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
