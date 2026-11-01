//! `amsrust`: boots a machine with the given CRTC/disc(s)/snapshot, for
//! manually testing the emulator against real-world software.
//!
//! ```text
//! amsrust [--crtc 0-4] [--dsk path] [--dsk-b path] [--sna path]
//!         [--screenshot dir [--seconds N] [--every N|Nf] [--from S]] [--window [--mute]]
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


use amsrust::display::{DisplayFilter, DisplayMode};
use amsrust::input::HostKey;
use amsrust::debug::{Monitor, MonitorContext};
use amsrust::disc_image::load_dsk;
use amsrust::fdc::FdcDrive;
use amsrust::input::HostKeyboard;
use amsrust::keyboard::CpcKey;
use amsrust::monitor::Monitor as CrtMonitor;
use amsrust::motherboard::{Motherboard, MotherboardBuilder};
use amsrust::overlay::Overlay;
use amsrust::debug_server::{Beam, DebugServer};
use amsrust::psg::AudioConfig;
use cli::FrontendKind;
use frontend::{Frontend, MinifbFrontend};
use std::collections::VecDeque;

mod cli;
mod frontend;
mod host_audio;
mod recorder;
mod watch;

use cli::{parse_args, Args};

/// The monitor for a picture to look at: its black ink above the blanking.
fn viewing_monitor(black_level: u8) -> CrtMonitor {
    CrtMonitor::default().with_black_level(black_level)
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

/// A flag of an extension this build leaves out.
#[allow(dead_code)]
fn without_feature(flag: &str, feature: &str) {
    eprintln!("{flag}: this bemu is built without the \"{feature}\" feature");
    std::process::exit(2);
}

fn main() {
    let args = parse_args();
    let mut filter = args.display.filter_on(args.gpu, args.curvature);
    if args.record.is_some() {
        // Only when recording: elsewhere (the interactive monitor) Ctrl+C
        // keeps its usual meaning.
        ctrlc::set_handler(|| STOP.store(true, std::sync::atomic::Ordering::Relaxed))
            .expect("cannot install the Ctrl+C handler");
    }

    let mut builder = MotherboardBuilder::default();
    if let Some(ga_type) = args.gate_array {
        builder.set_gate_array_type(ga_type);
    }
    builder.set_crtc(args.crtc).set_model(args.model).enable_interruptions();
    if let Some(kb) = args.ram_kb {
        builder.set_extra_ram_pages(kb / 64 - 1);
    }
    if let Some(fitted) = args.disc_interface() {
        builder.set_disc_interface(fitted);
    }
    if args.playcity {
        #[cfg(feature = "playcity")]
        builder.plug_playcity();
        #[cfg(not(feature = "playcity"))]
        without_feature("--playcity", "playcity");
    }
    if args.digiblaster {
        #[cfg(feature = "digiblaster")]
        builder.plug_digiblaster();
        #[cfg(not(feature = "digiblaster"))]
        without_feature("--digiblaster", "digiblaster");
    }
    if args.french {
        builder.set_keyboard_layout(amsrust::keyboard::KeyboardLayout::French);
    }
    let mut motherboard = builder.build();
    #[cfg(not(feature = "m4"))]
    if args.m4.is_some() {
        without_feature("--m4", "m4");
    }
    #[cfg(feature = "m4")]
    if let Some(sd) = &args.m4 {
        let board = match &args.m4_rom {
            Some(path) => {
                let rom = amsrust::roms::load_rom(path).unwrap_or_else(|e| panic!("--m4-rom {e}"));
                amsrust::m4::M4Board::new(&rom, sd)
            },
            None => amsrust::m4::M4Board::with_bundled_rom(sd)
        };
        let board = board.map(|board| board.in_slot(args.m4_slot.unwrap_or(amsrust::m4::M4_ROM_SLOT)));
        #[cfg(feature = "m4-wifi")]
        let board = board.map(|board| board.with_wifi(!args.m4_no_wifi));
        motherboard.plug_expansion(Box::new(board.unwrap_or_else(|e| panic!("--m4: {e}"))));
        eprintln!("{sd}: the M4 board's SD card");
    }
    #[cfg(not(feature = "albireo"))]
    if args.albireo.is_some() {
        without_feature("--albireo", "albireo");
    }
    #[cfg(feature = "albireo")]
    if let Some(image) = &args.albireo {
        if args.m4.is_some() {
            panic!("--albireo and --m4: both decode &FExx, plug one");
        }
        let board = amsrust::ch376::Albireo::open(std::path::Path::new(image)).unwrap_or_else(|e| panic!("--albireo: {e}"));
        motherboard.plug_expansion(Box::new(board));
        eprintln!("{image}: the Albireo's USB key");
    }
    #[cfg(not(feature = "nova"))]
    if args.nova.is_some() {
        without_feature("--nova", "nova");
    }
    #[cfg(feature = "nova")]
    if let Some(file) = &args.nova {
        let nova = amsrust::nova::Nova::new(Some(file.into())).unwrap_or_else(|e| panic!("--nova: {e}"));
        motherboard.plug_expansion(Box::new(nova));
        eprintln!("{file}: the Nova's NVRAM");
    }
    #[cfg(not(feature = "booster"))]
    if args.booster().is_some() {
        without_feature("--booster", "booster");
    }
    #[cfg(feature = "booster")]
    if let Some(board) = args.booster() {
        let model = if board == "miniboster" {
            amsrust::booster::Model::MiniBooster
        }
        else {
            amsrust::booster::Model::BoosterPlus
        };
        let mut booster = amsrust::booster::Booster::new(model);
        if let Some(uart) = &args.booster_uart {
            booster = booster.with_link(match uart.split_once(':') {
                None if uart == "loopback" => Box::<amsrust::booster::Loopback>::default(),
                Some(("listen", address)) => {
                    Box::new(amsrust::booster::TcpLink::listen(address).unwrap_or_else(|e| panic!("--booster-uart {uart}: {e}")))
                },
                Some(("connect", address)) => {
                    Box::new(amsrust::booster::TcpLink::connect(address).unwrap_or_else(|e| panic!("--booster-uart {uart}: {e}")))
                },
                _ => panic!("--booster-uart: loopback, listen:ADDRESS or connect:ADDRESS, not {uart:?}")
            });
        }
        if let Some(file) = &args.booster_eeprom {
            booster = booster.with_eeprom_file(file.into()).unwrap_or_else(|e| panic!("--booster-eeprom: {e}"));
        }
        if let Some(file) = &args.booster_flash {
            booster = booster.with_flash_file(file.into()).unwrap_or_else(|e| panic!("--booster-flash: {e}"));
        }
        if let Some(file) = &args.booster_adc {
            booster = booster.with_adc(amsrust::booster::AdcSource::open(std::path::Path::new(file)).unwrap_or_else(|e| panic!("--booster-adc: {e}")));
        }
        motherboard.plug_expansion(Box::new(booster));
        eprintln!("the CPC Booster ({model:?}) is plugged");
    }
    #[cfg(not(feature = "dandanator"))]
    if args.dandanator.is_some() {
        without_feature("--dandanator", "dandanator");
    }
    #[cfg(feature = "dandanator")]
    if let Some(romset) = &args.dandanator {
        let card = amsrust::dandanator::Dandanator::open(romset, args.dandanator_writable).unwrap_or_else(|e| panic!("--dandanator {e}"));
        motherboard.plug_expansion(Box::new(card));
        eprintln!("the Dandanator: {romset}{}", if args.dandanator_writable { " (writable)" } else { "" });
    }
    #[cfg(not(feature = "willy"))]
    if args.willy_opl3().is_some() || args.willy_s2p.is_some() {
        without_feature("--willy-opl3/--willy-s2p", "willy");
    }
    #[cfg(feature = "willy")]
    {
        use amsrust::willy::{parse_base, Opl3Lpt, S2p, S2P_BASE};
        if let Some(base) = args.willy_opl3() {
            let base = parse_base(&base).unwrap_or_else(|e| panic!("--willy-opl3-at {e}"));
            motherboard.plug_expansion(Box::new(Opl3Lpt::new(base)));
            eprintln!("Willy with an OPL3LPT at &{base:04X}");
        }
        if let Some(soundfont) = &args.willy_s2p {
            let base = args.willy_s2p_at.as_deref().map_or(Ok(S2P_BASE), parse_base).unwrap_or_else(|e| panic!("--willy-s2p-at {e}"));
            let card = S2p::open(base, std::path::Path::new(soundfont)).unwrap_or_else(|e| panic!("--willy-s2p {e}"));
            motherboard.plug_expansion(Box::new(card));
            eprintln!("Willy with an S2P at &{base:04X} ({soundfont})");
        }
    }
    for (slot, path) in &args.roms {
        let data = amsrust::roms::load_rom(path).unwrap_or_else(|e| panic!("--rom {e}"));
        motherboard.set_upper_rom(*slot, Some(&data)).unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!("{path}: upper ROM {slot}");
    }
    if (args.dsk_a.is_some() || args.dsk_b.is_some()) && !motherboard.has_disc_interface() {
        eprintln!("--dsk: this {} has no disc interface (--ddi adds one)", args.model.name());
    }

    if let Some(path) = &args.dsk_a {
        insert_disc(path, FdcDrive::A, &mut motherboard);
    }
    if let Some(path) = &args.dsk_b {
        insert_disc(path, FdcDrive::B, &mut motherboard);
    }
    if let Some(path) = &args.cdt {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let tape = amsrust::tape::Tape::from_cdt(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        for note in &tape.notes {
            eprintln!("{path}: {note}");
        }
        eprintln!("{path}: {:.0} s of tape", tape.duration_seconds());
        motherboard.insert_tape(Some(tape));
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
    for path in &args.symbols {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let (symbols, ignored) = amsrust::symbols::SymbolTable::parse_file(&text);
        for line in ignored.iter().take(5) {
            eprintln!("{path}: not a symbol: {line}");
        }
        eprintln!("{path}: {} symbol(s)", symbols.len());
        breakpoints.symbols.extend(&symbols);
    }
    let monitor_mode =
        args.screenshot_dir.is_none() && !args.window && args.record.is_none() && !args.debug_server();
    if !breakpoints.is_empty() && !monitor_mode && !args.debug_server() {
        eprintln!(
            "{} snapshot breakpoint(s), used by the interactive monitor and the debug server only",
            breakpoints.len()
        );
    }
    start_sound_export(&args, &mut motherboard);
    let mut typist = match &args.basic {
        Some(file) => {
            let source = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("--basic {file}: {e}"));
            Some(Typist::with_basic(args.text.as_deref().unwrap_or(""), source, args.type_at, motherboard.keyboard_layout()))
        },
        None => args.text.as_deref().map(|text| Typist::new(text, args.type_at, motherboard.keyboard_layout()))
    };
    if typist.is_some() && monitor_mode {
        eprintln!("--type is ignored in the interactive monitor");
    }
    let mut reloader = args.watch.then(|| Reloader::new(&args));
    if reloader.is_some() && !(args.window || args.debug_server()) {
        eprintln!("--watch needs --window or --debug-server");
    }

    match args.screenshot_dir {
        Some(dir) => run_headless(&mut motherboard, &dir, args.seconds, args.every, args.from, &mut typist, args.display, filter.as_mut(), args.black_level, args.overlay),
        None if args.window => {
            let mut server = start_debug_server(&args, &breakpoints, &mut motherboard);
            run_window(&mut motherboard, !args.mute, args.record.as_deref(), &mut typist, &mut reloader, server.as_mut(), args.display, args.gpu, args.curvature, args.black_level, args.frontend, args.turbo, args.overlay)
        },
        None if args.debug_server() => {
            let mut server = start_debug_server(&args, &breakpoints, &mut motherboard).unwrap();
            run_debug_server(&mut motherboard, &mut server, &mut typist, &mut reloader)
        },
        None if args.record.is_some() => {
            run_recording(&mut motherboard, args.record.as_deref().unwrap(), args.seconds, &mut typist, filter.as_mut(), args.black_level)
        },
        None => {
            let mut handle = |_motherboard: &mut amsrust::motherboard::Motherboard| false;
            let mut ctx = MonitorContext::new(Box::new(&mut handle));
            ctx.add_symbols(&breakpoints.symbols);
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
    finish_sound_export(&mut motherboard);
    if let Some(path) = &args.save_sna {
        match amsrust::snapshot::save_sna(&motherboard, path) {
            Ok(()) => {
            let full = std::fs::canonicalize(&path).map_or(path.clone(), |p| p.display().to_string());
            eprintln!("snapshot saved: {full}")
        },
            Err(e) => eprintln!("cannot save the snapshot: {e}")
        }
    }
}

/// `--record-ym` / `--record-wav`: the sound exported, see `amsrust::sound_export`.
struct SoundExport {
    ym: Option<String>,
    wav_path: Option<String>,
    /// Opened with the first samples, at the rate they come at.
    wav: Option<amsrust::sound_export::WavWriter<std::io::BufWriter<std::fs::File>>>
}

thread_local! {
    static SOUND_EXPORT: std::cell::RefCell<Option<SoundExport>> = const { std::cell::RefCell::new(None) };
}
/// Whether the WAV export runs: checked first, as the loops call [`poll_sound_export`] often.
static WAV_EXPORT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn start_sound_export(args: &Args, motherboard: &mut Motherboard) {
    if args.record_ym.is_none() && args.record_wav.is_none() {
        return;
    }
    if args.record_ym.is_some() {
        motherboard.psg_mut().log_frames(true);
        for (_, chip) in motherboard.expansion_sound_chips() {
            chip.log_frames(true);
        }
    }
    if args.record_wav.is_some() {
        if motherboard.psg().audio_config().is_none() {
            motherboard.set_audio_output(Some(RECORD_AUDIO()));
        }
        WAV_EXPORT.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    SOUND_EXPORT.with(|export| {
        *export.borrow_mut() = Some(SoundExport { ym: args.record_ym.clone(), wav_path: args.record_wav.clone(), wav: None })
    });
}

/// Samples the main loop took: into the WAV file.
fn export_samples(motherboard: &Motherboard, samples: &[f32]) {
    if !WAV_EXPORT.load(std::sync::atomic::Ordering::Relaxed) || samples.is_empty() {
        return;
    }
    SOUND_EXPORT.with(|export| {
        let mut export = export.borrow_mut();
        let Some(export) = export.as_mut() else { return };
        if export.wav.is_none() {
            let (Some(path), Some(config)) = (&export.wav_path, motherboard.psg().audio_config()) else { return };
            let opened = std::fs::File::create(path).and_then(|file| {
                amsrust::sound_export::WavWriter::new(
                    std::io::BufWriter::new(file),
                    config.sample_rate,
                    config.nb_channels() as u16
                )
            });
            match opened {
                Ok(wav) => export.wav = Some(wav),
                Err(e) => {
                    eprintln!("{path}: {e}");
                    WAV_EXPORT.store(false, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
            }
        }
        if let Some(Err(e)) = export.wav.as_mut().map(|wav| wav.samples(samples)) {
            eprintln!("WAV export stopped: {e}");
            WAV_EXPORT.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

/// For the loops that take no samples themselves (headless, debug server): a frame's samples
/// at a time.
fn poll_sound_export(motherboard: &mut Motherboard) {
    if WAV_EXPORT.load(std::sync::atomic::Ordering::Relaxed)
        && motherboard.psg().pending_samples() >= 2048
    {
        let samples = motherboard.take_audio_samples();
        export_samples(motherboard, &samples);
    }
}

fn finish_sound_export(motherboard: &mut Motherboard) {
    let samples = motherboard.take_audio_samples();
    export_samples(motherboard, &samples);
    WAV_EXPORT.store(false, std::sync::atomic::Ordering::Relaxed);
    let Some(export) = SOUND_EXPORT.with(|export| export.borrow_mut().take()) else { return };
    if let (Some(wav), Some(path)) = (export.wav, &export.wav_path) {
        match wav.finish() {
            Ok(_) => eprintln!("sound saved: {path}"),
            Err(e) => eprintln!("{path}: {e}")
        }
    }
    if let Some(path) = &export.ym {
        let frames = motherboard.psg_mut().take_logged_frames();
        let song = std::path::Path::new(path).file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        match std::fs::write(path, amsrust::sound_export::ym6(&frames, 50, &song)) {
            Ok(()) => eprintln!("{path}: {} frames of AY registers", frames.len()),
            Err(e) => eprintln!("{path}: {e}")
        }
        // the cards' AYs (a PlayCity's: file.right.ym and file.left.ym)
        for (side, chip) in motherboard.expansion_sound_chips() {
            let side_path = std::path::Path::new(path).with_extension(format!("{side}.ym"));
            let frames = chip.take_logged_frames();
            match std::fs::write(&side_path, amsrust::sound_export::ym6(&frames, 50, &format!("{song} {side}"))) {
                Ok(()) => eprintln!("{}: the {side} AY of a card", side_path.display()),
                Err(e) => eprintln!("{}: {e}", side_path.display())
            }
        }
    }
}

/// T-states per second at the CPC's 4MHz clock.
const T_STATES_PER_SECOND: usize = 4_000_000;

/// `--type`: the text typed on the CPC keyboard, one character after the
/// other, each held for 2 frames then released for 2 frames.
struct Typist {
    /// `--basic`'s listing, put in memory (then RUN typed) when the typing starts.
    basic: Option<String>,
    keys: VecDeque<Step>,
    /// T-state at which the next press or release is due.
    next_at: usize,
    pressed: Option<Vec<CpcKey>>
}

/// 40ms: 2 frames, long enough for the firmware's keyboard scan.
const KEY_T_STATES: usize = T_STATES_PER_SECOND / 25;

/// What `--type` does next: press keys together, or wait (`\(WAIT n)`, n seconds).
enum Step {
    Keys(Vec<CpcKey>),
    Pause(usize)
}

/// `text`'s keys, with its `\(WAIT n)` pauses (n seconds: a program's prompt to wait for).
fn steps_of_text(text: &str, layout: amsrust::keyboard::KeyboardLayout) -> Result<Vec<Step>, String> {
    const WAIT: &str = "\\(WAIT ";
    let mut steps = Vec::new();
    let mut rest = text;
    loop {
        let (keys, after) = match rest.find(WAIT) {
            Some(start) => (&rest[..start], Some(&rest[start + WAIT.len()..])),
            None => (rest, None)
        };
        steps.extend(amsrust::csl_runner::keys_of_text(keys, layout)?.into_iter().map(Step::Keys));
        let Some(after) = after else {
            return Ok(steps);
        };
        let end = after.find(')').ok_or_else(|| format!("{WAIT}... without its )"))?;
        let seconds: f64 = after[..end].trim().parse().map_err(|_| format!("{WAIT}{}): not a number of seconds", &after[..end]))?;
        steps.push(Step::Pause((seconds * T_STATES_PER_SECOND as f64) as usize));
        rest = &after[end + 1..];
    }
}

impl Typist {
    fn new(text: &str, at_seconds: f64, layout: amsrust::keyboard::KeyboardLayout) -> Self {
        let keys = steps_of_text(text, layout)
            .unwrap_or_else(|e| {
                eprintln!("--type: {e}");
                Vec::new()
            })
            .into();
        Self { basic: None, keys, next_at: (at_seconds * T_STATES_PER_SECOND as f64) as usize, pressed: None }
    }

    /// `--basic`: the listing goes in memory at `at_seconds`, then RUN and `text` are typed.
    fn with_basic(text: &str, basic: String, at_seconds: f64, layout: amsrust::keyboard::KeyboardLayout) -> Self {
        let mut typist = Self::new(&format!("RUN\n{text}"), at_seconds, layout);
        typist.basic = Some(basic);
        typist
    }

    /// Press or release the keys that are due. Cheap: call it as often as
    /// wanted (every instruction or every frame).
    fn update(&mut self, motherboard: &mut Motherboard) {
        let now = motherboard.t_state_counter().value();
        if now >= self.next_at {
            if let Some(source) = self.basic.take() {
                // as LOAD would (BASIC is up by now: --type-at); what the tokenizer can't read
                // is typed instead (slower: the firmware tokenizes it)
                if let Err(error) = amsrust::basic_program::load(motherboard, &source) {
                    eprintln!("--basic: {error}\n--basic: typing the listing instead");
                    let layout = motherboard.keyboard_layout();
                    let typed = source.lines().map(|line| format!("{line}\n")).collect::<String>();
                    match amsrust::csl_runner::keys_of_text(&typed, layout) {
                        Ok(keys) => {
                            for key in keys.into_iter().rev() {
                                self.keys.push_front(Step::Keys(key));
                            }
                        },
                        Err(error) => {
                            eprintln!("--basic: {error}");
                            self.keys.clear();
                        }
                    }
                }
            }
        }
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
                Some(Step::Keys(keys)) => (keys, true),
                Some(Step::Pause(t_states)) => {
                    self.next_at = now + t_states;
                    return;
                },
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

/// `--watch`: the machine takes the files again when they change.
struct Reloader {
    watcher: watch::Watcher,
    /// The snapshot, which the machine restarts from when a disc changes too.
    sna: Option<String>,
    text: Option<String>,
    type_at: f64
}

impl Reloader {
    fn new(args: &Args) -> Self {
        let files = [
            (args.sna.clone(), watch::Kind::Sna),
            (args.dsk_a.clone(), watch::Kind::Dsk(FdcDrive::A)),
            (args.dsk_b.clone(), watch::Kind::Dsk(FdcDrive::B)),
            (args.cdt.clone(), watch::Kind::Cdt)
        ];
        let watcher = watch::Watcher::new(files.into_iter().filter_map(|(path, kind)| path.map(|path| (path, kind))));
        if watcher.is_empty() {
            eprintln!("--watch: nothing to watch (--sna, --dsk, --dsk-b, --cdt)");
        }
        else {
            eprintln!("watching {}", watcher.paths().collect::<Vec<_>>().join(", "));
        }
        Self { watcher, sna: args.sna.clone(), text: args.text.clone(), type_at: args.type_at }
    }

    /// Take the files that changed:
    /// - a disc goes in the drive again, a tape in the deck (rewound);
    /// - then the snapshot, if there is one, is loaded again (when it changed, or the disc did:
    ///   the machine starts again from it);
    /// - without a snapshot, a new disc or tape switches the machine off and on, and `--type` is
    ///   typed again.
    fn poll(&mut self, motherboard: &mut Motherboard, typist: &mut Option<Typist>) {
        let changed = self.watcher.changed();
        if changed.is_empty() {
            return;
        }
        let mut restart = false;
        for (path, kind) in &changed {
            match kind {
                watch::Kind::Dsk(drive) => match std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| load_dsk(&bytes)) {
                    Ok(image) => {
                        motherboard.fdc_mut().insert_image(*drive, Some(image));
                        eprintln!("{path}: changed, inserted again");
                        restart = true;
                    },
                    Err(e) => eprintln!("{path}: changed, but {e}")
                },
                watch::Kind::Cdt => {
                    let tape = std::fs::read(path)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| amsrust::tape::Tape::from_cdt(&bytes));
                    match tape {
                        Ok(tape) => {
                            motherboard.insert_tape(Some(tape));
                            eprintln!("{path}: changed, inserted again");
                            restart = true;
                        },
                        Err(e) => eprintln!("{path}: changed, but {e}")
                    }
                },
                watch::Kind::Sna => restart = true
            }
        }
        if !restart {
            return;
        }
        match &self.sna {
            Some(path) => match amsrust::snapshot::load_sna(motherboard, path) {
                Ok(()) => eprintln!("{path}: loaded again"),
                Err(e) => eprintln!("{e}")
            },
            None => {
                motherboard.power_cycle();
                let now = motherboard.t_state_counter().value() as f64 / T_STATES_PER_SECOND as f64;
                let layout = motherboard.keyboard_layout();
                *typist = self.text.as_deref().map(|text| Typist::new(text, now + self.type_at, layout));
                eprintln!("machine switched off and on");
            }
        }
    }
}

fn type_due_keys(typist: &mut Option<Typist>, motherboard: &mut Motherboard) {
    if let Some(typist) = typist {
        typist.update(motherboard);
    }
}

/// The machine's state in `bemu_<n>.sna`, the first `n` free in the current directory.
fn save_snapshot(motherboard: &Motherboard) {
    let path = (0..)
        .map(|n| format!("bemu_{n:04}.sna"))
        .find(|path| !std::path::Path::new(path).exists())
        .expect("a free name");
    match amsrust::snapshot::save_sna(motherboard, &path) {
        Ok(()) => eprintln!("snapshot saved: {path}"),
        Err(e) => eprintln!("cannot save the snapshot: {e}")
    }
}

fn run_headless(
    motherboard: &mut amsrust::motherboard::Motherboard,
    dir: &str,
    seconds: u64,
    every: cli::Every,
    from: f64,
    typist: &mut Option<Typist>,
    mode: amsrust::display::DisplayMode,
    filter: &mut dyn DisplayFilter,
    black_level: u8,
    show_overlay: bool
) {
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{dir}: {e}"));
    let mut overlay = show_overlay.then(Overlay::default);

    let target_t_states = seconds as usize * T_STATES_PER_SECOND;
    // The first screenshot is taken as soon as a frame is available from
    // `from`, the rest paced `every` apart from there.
    let mut next_shot_at = (from * T_STATES_PER_SECOND as f64) as usize;
    let mut shot = 0usize;
    let save = |frame: &image::RgbImage, shot: usize, filter: &mut dyn DisplayFilter, at: usize| {
        let path = format!("{dir}/frame_{shot:04}.png");
        // The raw mode keeps the monitor's own frame, as the test suites do.
        let saved = if mode.is_raw() { frame.save(&path) } else { filter.render_to_image(frame).save(&path) };
        saved.unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!("[{:>9.3}s] {path}", at as f64 / T_STATES_PER_SECOND as f64);
    };

    let every_t_states = match every {
        cli::Every::Seconds(seconds) => ((seconds * T_STATES_PER_SECOND as f64) as usize).max(1),
        cli::Every::Frames(frames) => {
            // One monitor follows the beam all along: every frame is whole and in order.
            while motherboard.t_state_counter().value() < next_shot_at.min(target_t_states) {
                motherboard.execute_next_opcode_no_video();
                type_due_keys(typist, motherboard);
                poll_sound_export(motherboard);
            }
            let mut monitor = viewing_monitor(black_level);
            motherboard.watch_io_writes(show_overlay);
            let mut seen = 0u64;
            while motherboard.t_state_counter().value() < target_t_states {
                type_due_keys(typist, motherboard);
                poll_sound_export(motherboard);
                let mut frame = None;
                motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                    monitor.receive(rgb, sync);
                    if frame.is_none() {
                        frame = monitor.take_frame();
                    }
                });
                if let Some(overlay) = &mut overlay {
                    note_io_writes(overlay, motherboard, &monitor);
                }
                if let Some(mut frame) = frame {
                    if let Some(overlay) = &mut overlay {
                        overlay.draw_and_clear(&mut frame);
                    }
                    if seen % frames == 0 {
                        save(&frame, shot, filter, motherboard.t_state_counter().value());
                        shot += 1;
                    }
                    seen += 1;
                    monitor.recycle(frame);
                }
            }
            eprintln!("{shot} screenshot(s) over {seconds}s emulated, saved to {dir}/");
            return;
        }
    };

    while motherboard.t_state_counter().value() < target_t_states {
        while motherboard.t_state_counter().value() < next_shot_at.min(target_t_states) {
            // between screenshots, run without collecting video (cheaper)
            motherboard.execute_next_opcode_no_video();
            type_due_keys(typist, motherboard);
            poll_sound_export(motherboard);
        }
        if motherboard.t_state_counter().value() >= target_t_states {
            break;
        }

        let mut monitor = viewing_monitor(black_level);
        let mut frame = None;
        // the writes of the frame captured only
        motherboard.watch_io_writes(show_overlay);
        // Capture the next whole frame once due, rather than every tick:
        // matches how the CSL runner and play_demo grab a screenshot.
        while frame.is_none() && motherboard.t_state_counter().value() < target_t_states {
            type_due_keys(typist, motherboard);
            poll_sound_export(motherboard);
            motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                monitor.receive(rgb, sync);
                // take_frame() consumes it: a later tick within this same
                // multi-tick opcode would otherwise immediately overwrite
                // a just-captured frame with the None the second take gives.
                if frame.is_none() {
                    frame = monitor.take_frame();
                }
            });
            if let Some(overlay) = &mut overlay {
                note_io_writes(overlay, motherboard, &monitor);
            }
        }
        motherboard.watch_io_writes(false);
        let Some(mut frame) = frame
        else {
            break; // ran out of time mid-frame
        };
        if let Some(overlay) = &mut overlay {
            overlay.draw_and_clear(&mut frame);
        }

        save(&frame, shot, filter, motherboard.t_state_counter().value());
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
    filter: &mut dyn DisplayFilter,
    black_level: u8
) {
    motherboard.set_audio_output(Some(RECORD_AUDIO()));
    let mut recorder = start_recorder(path, Some(RECORD_AUDIO()), filter.output_size());
    let (width, height) = filter.output_size();
    let mut buffer = vec![0u32; width * height];
    let target_t_states = seconds as usize * T_STATES_PER_SECOND;
    let mut monitor = viewing_monitor(black_level);
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
            let samples = motherboard.take_audio_samples();
            export_samples(motherboard, &samples);
            let recorded = recorder.samples(&samples).and_then(|()| recorder.frame(&buffer));
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
    if !args.debug_server() {
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
    match DebugServer::start(args.debug_port()) {
        Ok(mut server) => {
            eprintln!("debug server: http://127.0.0.1:{}/api/doc", args.debug_port());
            server.add_symbols(&breakpoints.symbols);
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
    mut server: Option<&mut DebugServer>,
    mut overlay: Option<&mut Overlay>
) -> Option<image::RgbImage> {
    let mut frame = None;
    while frame.is_none() {
        motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
            monitor.receive(rgb, sync);
            if frame.is_none() {
                frame = monitor.take_frame();
            }
        });
        if let Some(overlay) = overlay.as_deref_mut() {
            note_io_writes(overlay, motherboard, monitor);
        }
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

/// The I/O writes of the last instruction on the overlay, where the beam is.
fn note_io_writes(overlay: &mut Overlay, motherboard: &mut Motherboard, monitor: &CrtMonitor) {
    let (beam, origin) = (monitor.beam(), monitor.crop_origin());
    for (port, value) in motherboard.take_io_writes() {
        overlay.note(port, value, beam, origin);
    }
}

/// `--debug-server` without a window: run in real time, driven by the
/// debug API (Ctrl+C to quit).
fn run_debug_server(
    motherboard: &mut Motherboard,
    server: &mut DebugServer,
    typist: &mut Option<Typist>,
    reloader: &mut Option<Reloader>
) {
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
        if let Some(reloader) = reloader {
            reloader.poll(motherboard, typist);
        }
        type_due_keys(typist, motherboard);
        poll_sound_export(motherboard);
        if run_frame(motherboard, &mut monitor, Some(server), None).is_some() {
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

/// T-states of a CPC frame (312 lines of 64 µs at 4 MHz).
const FRAME_T_STATES: usize = 312 * 64 * 4;
/// At full speed, a frame is shown this often (50 a second).
const FRAME_SHOWN_EVERY: std::time::Duration = std::time::Duration::from_millis(20);

fn run_window(
    motherboard: &mut Motherboard,
    sound: bool,
    record: Option<&str>,
    typist: &mut Option<Typist>,
    reloader: &mut Option<Reloader>,
    mut server: Option<&mut DebugServer>,
    mut mode: DisplayMode,
    gpu: bool,
    curvature: f32,
    black_level: u8,
    frontend: FrontendKind,
    mut turbo: bool,
    show_overlay: bool
) {
    let (width, height) = mode.output_size();
    let mut window: Box<dyn Frontend> = match frontend {
        FrontendKind::Minifb => {
            Box::new(MinifbFrontend::open("amsrust", width, height).unwrap_or_else(|e| panic!("{e}")))
        },
        #[cfg(feature = "gpu-window")]
        FrontendKind::Winit => {
            Box::new(frontend::WinitGpuFrontend::open((width, height), curvature).unwrap_or_else(|e| panic!("{e}")))
        },
        #[cfg(not(feature = "gpu-window"))]
        FrontendKind::Winit => panic!("--frontend winit needs a build with --features gpu-window")
    };
    // A frontend that draws the frames itself needs a picture from the filter
    // only to record it...
    let direct = window.direct();
    // ...and then on the CPU: such a frontend draws on the GPU by itself.
    let gpu = gpu && !direct;
    let mut filter = mode.filter_on(gpu, curvature);
    // Cap redraws (and so input polling and emulated speed) at the CPC's
    // own real refresh rate rather than running full tilt.
    let audio = open_audio(sound, motherboard);
    // The audio device's clock paces the emulation instead, if there is one; at full speed
    // nothing does.
    let paced_fps = if audio.is_some() { 0 } else { 50 };
    window.set_target_fps(if turbo { 0 } else { paced_fps });
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
    let mut f9_was_down = false;
    let mut f10_was_down = false;
    let mut f7_was_down = false;
    let mut f8_was_down = false;
    // F8's pause, without a debug server (with one, F8 pauses through it)
    let mut paused = false;
    let mut f6_was_down = false;
    let mut overlay = show_overlay.then(Overlay::default);
    motherboard.watch_io_writes(show_overlay);
    // at full speed, the frames shown (at most 50 a second: presenting each would hold the
    // emulation back to the display's rate)
    let mut last_present = std::time::Instant::now();
    if turbo {
        eprintln!("speed: full (F10 for real time)");
    }

    // One whole frame per redraw, locked to the emulator's real VSYNC, not
    // to a fixed t-state count.
    // The debug server's pictures stay exact: no lifted black.
    let mut monitor = if server.is_some() { CrtMonitor::default() } else { viewing_monitor(black_level) };
    if server.is_some() {
        monitor.keep_full_frames();
    }
    while window.is_open() && !stop_requested() {
        let input = window.poll_input();
        // F9 cycles the display (not while recording: a video has one size).
        let f9_down = input.down.contains(&HostKey::F9);
        if f9_down && !f9_was_down && recorder.is_none() {
            mode = mode.next();
            filter = mode.filter_on(gpu, curvature);
            let (width, height) = filter.output_size();
            buffer = vec![0u32; width * height];
            eprintln!("display: {mode:?}");
        }
        f9_was_down = f9_down;
        // F10 toggles full speed
        let f10_down = input.down.contains(&HostKey::F10);
        if f10_down && !f10_was_down {
            turbo = !turbo;
            window.set_target_fps(if turbo { 0 } else { paced_fps });
            eprintln!("speed: {}", if turbo { "full" } else { "real time" });
        }
        f10_was_down = f10_down;
        if let Some(reloader) = reloader {
            reloader.poll(motherboard, typist);
        }
        // F7 saves a snapshot, in the current directory
        let f7_down = input.down.contains(&HostKey::F7);
        if f7_down && !f7_was_down {
            save_snapshot(motherboard);
        }
        f7_was_down = f7_down;
        // F8 pauses and resumes (F7 then saves the paused state)
        let f8_down = input.down.contains(&HostKey::F8);
        if f8_down && !f8_was_down {
            let now = match server.as_deref_mut() {
                Some(server) => {
                    let now = !server.paused();
                    server.set_paused(now);
                    now
                },
                None => {
                    paused = !paused;
                    paused
                }
            };
            eprintln!("{}", if now { "paused (F8 resumes, F7 saves a snapshot)" } else { "running" });
        }
        f8_was_down = f8_down;
        // F6 shows or hides the timing overlay
        let f6_down = input.down.contains(&HostKey::F6);
        if f6_down && !f6_was_down {
            overlay = if overlay.is_some() { None } else { Some(Overlay::default()) };
            motherboard.watch_io_writes(overlay.is_some());
            eprintln!("timing overlay: {}", if overlay.is_some() { "on" } else { "off" });
        }
        f6_was_down = f6_down;
        host_keyboard.apply(motherboard, &input);
        // After the host keyboard, which would release the typed keys.
        type_due_keys(typist, motherboard);

        if let Some(server) = server.as_deref_mut() {
            server.poll_with(motherboard, Some(&monitor));
            if server.quit_requested() {
                break;
            }
            if server.paused() {
                // Stopped by the debugger: keep the window alive.
                if direct {
                    window.redraw().unwrap_or_else(|e| panic!("{e}"));
                }
                else {
                    window.present(&buffer, filter.output_size()).unwrap_or_else(|e| panic!("{e}"));
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
                continue;
            }
        }
        if paused {
            if direct {
                window.redraw().unwrap_or_else(|e| panic!("{e}"));
            }
            else {
                window.present(&buffer, filter.output_size()).unwrap_or_else(|e| panic!("{e}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
            continue;
        }
        // At full speed the frames not shown run without any picture (a quarter faster): one
        // frame's time, then the next loop. A recording and the debug server see every frame.
        let unseen = turbo && recorder.is_none() && server.is_none() && last_present.elapsed() < FRAME_SHOWN_EVERY;
        motherboard.set_video(!unseen);
        if unseen {
            let end = motherboard.t_state_counter().value() + FRAME_T_STATES;
            while motherboard.t_state_counter().value() < end {
                motherboard.execute_next_opcode_no_video();
            }
            motherboard.take_audio_samples();
            motherboard.take_io_writes().for_each(drop);
            continue;
        }
        let Some(mut frame) = run_frame(motherboard, &mut monitor, server.as_deref_mut(), overlay.as_mut())
        else {
            continue; // paused mid-frame
        };
        if let Some(overlay) = &mut overlay {
            overlay.draw_and_clear(&mut frame);
        }

        if !direct || recorder.is_some() {
            filter.render(&frame, &mut buffer);
        }
        let samples = motherboard.take_audio_samples();
        export_samples(motherboard, &samples);
        if let Some(recorder) = &mut recorder {
            let recorded = recorder.samples(&samples).and_then(|()| recorder.frame(&buffer));
            if let Err(e) = recorded {
                eprintln!("recording stopped: {e}");
                break;
            }
        }
        if turbo {
            // the sound of a fast-forward is dropped, and only some frames are shown
            if last_present.elapsed() < FRAME_SHOWN_EVERY {
                monitor.recycle(frame);
                continue;
            }
            last_present = std::time::Instant::now();
        }
        else {
            play_frame_audio(audio.as_ref(), &samples, window.as_ref());
        }

        if direct {
            window.present_frame(&frame, mode).unwrap_or_else(|e| panic!("{e}"));
        }
        else {
            window.present(&buffer, filter.output_size()).unwrap_or_else(|e| panic!("{e}"));
        }
        // the next frame is drawn in this one
        monitor.recycle(frame);
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

#[cfg(test)]
mod tests {
    use amsrust::keyboard::KeyboardLayout;

    use super::{steps_of_text, Step, T_STATES_PER_SECOND};

    #[test]
    fn typed_text_waits_where_asked() {
        let steps = steps_of_text("ab\\(WAIT 1.5)c\\(RET)", KeyboardLayout::Uk).unwrap();
        let shape: Vec<Option<usize>> = steps
            .iter()
            .map(|step| match step {
                Step::Keys(_) => None,
                Step::Pause(t_states) => Some(*t_states)
            })
            .collect();
        assert_eq!(shape, [None, None, Some(T_STATES_PER_SECOND * 3 / 2), None, None]);
        assert!(steps_of_text("\\(WAIT x)a", KeyboardLayout::Uk).is_err());
        assert!(steps_of_text("\\(WAIT 2", KeyboardLayout::Uk).is_err());
    }
}
