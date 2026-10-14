//! `amsrust`: boots a machine with the given CRTC/disc(s)/snapshot, for
//! manually testing the emulator against real-world software.
//!
//! ```text
//! amsrust [--crtc 0-4] [--dsk path] [--dsk-b path] [--sna path]
//!         [--screenshot dir [--seconds N] [--every N]] [--window [--mute]]
//!         [--record file [--seconds N]] [--type text [--type-at N]] [--display raw]
//!         [--debug-server [--debug-port N] [--debug-paused]]
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

use amsrust::csl_runner::keys_of_char;
use amsrust::display::DisplayFilter;
use amsrust::debug::{Monitor, MonitorContext};
use amsrust::disc_image::load_dsk;
use amsrust::fdc::FdcDrive;
use amsrust::input::HostKeyboard;
use amsrust::keyboard::CpcKey;
use amsrust::monitor::Monitor as CrtMonitor;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::debug_server::{Beam, DebugServer};
use amsrust::psg::AudioConfig;
use frontend::{Frontend, MinifbFrontend};
use std::collections::VecDeque;

mod cli;
mod frontend;
mod host_audio;
mod recorder;

use cli::{parse_args, Args};

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
    let mut filter = args.display.filter();
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
        Some(dir) => run_headless(&mut motherboard, &dir, args.seconds, args.every, &mut typist, args.display, filter.as_mut()),
        None if args.window => {
            let mut server = start_debug_server(&args, &breakpoints, &mut motherboard);
            run_window(&mut motherboard, !args.mute, args.record.as_deref(), &mut typist, server.as_mut(), filter.as_mut())
        },
        None if args.debug_server => {
            let mut server = start_debug_server(&args, &breakpoints, &mut motherboard).unwrap();
            run_debug_server(&mut motherboard, &mut server, &mut typist)
        },
        None if args.record.is_some() => {
            run_recording(&mut motherboard, args.record.as_deref().unwrap(), args.seconds, &mut typist, filter.as_mut())
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
    typist: &mut Option<Typist>,
    mode: amsrust::display::DisplayMode,
    filter: &mut dyn DisplayFilter
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
        // The raw mode keeps the monitor's own frame, as the test suites do.
        let saved = if mode.is_raw() { frame.save(&path) } else { filter.render_to_image(&frame).save(&path) };
        saved.unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!("[{:>6}s] {path}", motherboard.t_state_counter().value() / T_STATES_PER_SECOND);
        shot += 1;
        next_shot_at = motherboard.t_state_counter().value() + every_t_states;
    }
    eprintln!("{shot} screenshot(s) over {seconds}s emulated, saved to {dir}/");
}

/// The format the PSG is recorded in when no audio device dictates it.
const RECORD_AUDIO: fn() -> AudioConfig = || AudioConfig::stereo(48_000);

fn start_recorder(path: &str, audio: Option<AudioConfig>, size: (usize, usize)) -> recorder::Recorder {
    recorder::Recorder::start(std::path::Path::new(path), size.0, size.1, audio)
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
fn run_recording(
    motherboard: &mut Motherboard,
    path: &str,
    seconds: u64,
    typist: &mut Option<Typist>,
    filter: &mut dyn DisplayFilter
) {
    motherboard.set_audio_output(Some(RECORD_AUDIO()));
    let mut recorder = start_recorder(path, Some(RECORD_AUDIO()), filter.output_size());
    let (width, height) = filter.output_size();
    let mut buffer = vec![0u32; width * height];
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
            filter.render(&frame, &mut buffer);
            let recorded = recorder
                .samples(&motherboard.take_audio_samples())
                .and_then(|()| recorder.frame(&buffer));
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

/// Start the debug server if asked, with the snapshot's breakpoints set.
fn start_debug_server(
    args: &Args,
    breakpoints: &amsrust::snapshot::SnapshotBreakpoints,
    motherboard: &mut Motherboard
) -> Option<DebugServer> {
    if !args.debug_server {
        return None;
    }
    if args.sna.is_none() {
        // A powered-on machine: boot the firmware to BASIC's prompt before
        // serving, so a debugger that connects at once (and loads a BASIC
        // program, reads BASIC's state) finds BASIC up rather than a boot in
        // progress. Run flat out, about half a second.
        while motherboard.t_state_counter().value() < amsrust::debug_server::BASIC_BOOT_T_STATES {
            motherboard.execute_next_opcode_no_video();
        }
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
            if args.debug_paused {
                // Nothing has run yet: the debugger arms its breakpoints,
                // then resumes (POST /api/config {"paused":false}).
                server.pause();
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
    monitor.keep_full_frames();
    let frame_duration = std::time::Duration::from_micros(19_968);
    let mut deadline = std::time::Instant::now();
    while !stop_requested() && !server.quit_requested() {
        server.poll_with(motherboard, Some(&monitor));
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
    mut server: Option<&mut DebugServer>,
    filter: &mut dyn DisplayFilter
) {
    let (width, height) = filter.output_size();
    let mut window = MinifbFrontend::open("amsrust", width, height).unwrap_or_else(|e| panic!("{e}"));
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
        start_recorder(path, Some(config), (width, height))
    });

    let mut host_keyboard = HostKeyboard::default();
    let mut buffer = vec![0u32; width * height];

    // One whole frame per redraw, locked to the emulator's real VSYNC, not
    // to a fixed t-state count.
    let mut monitor = CrtMonitor::default();
    if server.is_some() {
        monitor.keep_full_frames();
    }
    while window.is_open() && !stop_requested() {
        host_keyboard.apply(motherboard, &window.poll_input());
        // After the host keyboard, which would release the typed keys.
        type_due_keys(typist, motherboard);

        if let Some(server) = server.as_deref_mut() {
            server.poll_with(motherboard, Some(&monitor));
            if server.quit_requested() {
                break;
            }
            if server.paused() {
                // Stopped by the debugger: keep the window alive.
                window.present(&buffer).unwrap_or_else(|e| panic!("{e}"));
                std::thread::sleep(std::time::Duration::from_millis(15));
                continue;
            }
        }
        let Some(frame) = run_frame(motherboard, &mut monitor, server.as_deref_mut())
        else {
            continue; // paused mid-frame
        };

        filter.render(&frame, &mut buffer);
        let samples = motherboard.take_audio_samples();
        if let Some(recorder) = &mut recorder {
            let recorded = recorder.samples(&samples).and_then(|()| recorder.frame(&buffer));
            if let Err(e) = recorded {
                eprintln!("recording stopped: {e}");
                break;
            }
        }
        play_frame_audio(audio.as_ref(), &samples, &window);

        window.present(&buffer).unwrap_or_else(|e| panic!("{e}"));
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
fn play_frame_audio(audio: Option<&AudioOut>, samples: &[f32], window: &dyn Frontend) {
    #[cfg(feature = "audio")]
    if let Some(audio) = audio {
        audio.push(samples);
        while audio.buffered() > host_audio::TARGET_LATENCY && window.is_open() {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}
