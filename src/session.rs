//! `--window`: the live window, its menus and command palette (with the `gui` feature, on the
//! winit frontend), driven by the commands of `amsrust::commands`.
//!
//! [`Core`] is what the commands act on (their receiver): the machine, the debugger (always
//! there, its HTTP server started by `--debug-server` or Debug > Debug server), the settings
//! the window applies between two frames, and the requests only the window can do (a
//! recording, a screenshot, a window of the interface). The menus, the palette and the
//! function keys all queue commands on the [`CommandBus`], run before the next frame.

use std::collections::HashSet;
use std::path::PathBuf;

use amsrust::commands::{
    self, Chord, CommandBus, CommandError, CommandTarget, HostRequest, HostSettings, Recording,
    Speed,
};
use amsrust::debug_server::{Beam, DebugServer};
use amsrust::display::DisplayMode;
use amsrust::input::{HostInput, HostKey, HostKeyboard, KeyboardMapping};
use amsrust::monitor::Monitor as CrtMonitor;
use amsrust::motherboard::Motherboard;
use amsrust::overlay::Overlay;

use crate::cli::{Args, FrontendKind};
use crate::frontend::{self, Frontend, MinifbFrontend};
use crate::{
    export_samples, finish_printer_export, finish_wav, finish_ym, open_audio, play_frame_audio,
    recording_wav, recording_ym, start_wav, start_ym, stop_requested, type_due_keys,
    viewing_monitor, AudioOut, Reloader, Typist, FRAME_SHOWN_EVERY, FRAME_T_STATES, RECORD_AUDIO,
};

/// What the commands act on.
struct Core<'m> {
    motherboard: &'m mut Motherboard,
    debugger: DebugServer,
    settings: HostSettings,
    requests: Vec<HostRequest>,
    recording: HashSet<Recording>,
    /// The window has the interface (menus, windows, clipboard).
    gui: bool,
}

impl CommandTarget for Core<'_> {
    fn motherboard(&self) -> &Motherboard {
        self.motherboard
    }
    fn motherboard_mut(&mut self) -> &mut Motherboard {
        self.motherboard
    }
    fn debugger(&self) -> Option<&DebugServer> {
        Some(&self.debugger)
    }
    fn debugger_mut(&mut self) -> Option<&mut DebugServer> {
        Some(&mut self.debugger)
    }
    fn debugger_and_motherboard(&mut self) -> Option<(&mut DebugServer, &mut Motherboard)> {
        Some((&mut self.debugger, self.motherboard))
    }
    fn settings(&self) -> Option<&HostSettings> {
        Some(&self.settings)
    }
    fn settings_mut(&mut self) -> Option<&mut HostSettings> {
        Some(&mut self.settings)
    }
    fn request(&mut self, request: HostRequest) -> Result<(), CommandError> {
        if !self.can(&request) {
            return Err(CommandError::Unsupported);
        }
        self.requests.push(request);
        Ok(())
    }
    fn can(&self, request: &HostRequest) -> bool {
        match request {
            HostRequest::Quit
            | HostRequest::Screenshot(_)
            | HostRequest::StopRecording(_)
            | HostRequest::DebugServer(_)
            | HostRequest::SaveFile { .. }
            | HostRequest::SavePrintout => true,
            HostRequest::StartRecording(recording, _) => *recording != Recording::Tape,
            HostRequest::Open(_)
            | HostRequest::ShowText { .. }
            | HostRequest::OpenUrl(_)
            | HostRequest::CopyScreenshot
            | HostRequest::PasteClipboard => self.gui,
            // a CSL script drives a machine of its own (`amsrust::csl_runner`)
            HostRequest::RunCsl { .. } | HostRequest::StopCsl => false,
        }
    }
    fn is_recording(&self, recording: Recording) -> bool {
        self.recording.contains(&recording)
    }
}

/// Screenshot > series: a PNG every `every` frames in `folder`.
struct Series {
    folder: PathBuf,
    every: u64,
    seen: u64,
    shot: usize,
}

impl Series {
    /// "FOLDER [every N|Nf]": N seconds apart, or N frames.
    fn parse(text: &str) -> Self {
        let mut words = text.split_whitespace();
        let folder = PathBuf::from(words.next().unwrap_or("."));
        let mut every = 100;
        while let Some(word) = words.next() {
            if word == "every" {
                if let Some(n) = words.next() {
                    every = match n.strip_suffix('f') {
                        Some(frames) => frames.parse().unwrap_or(1),
                        None => (n.parse::<f64>().unwrap_or(2.0) * 50.0) as u64,
                    };
                }
            }
        }
        Self {
            folder,
            every: every.max(1),
            seen: 0,
            shot: 0,
        }
    }
}

/// The window's own state, next to the commands' [`Core`].
struct Window {
    window: Box<dyn Frontend>,
    direct: bool,
    gpu: bool,
    filter: Box<dyn amsrust::display::DisplayFilter>,
    buffer: Vec<u32>,
    audio: Option<AudioOut>,
    recorder: Option<(crate::recorder::Recorder, PathBuf)>,
    series: Option<Series>,
    host_keyboard: HostKeyboard,
    overlay: Option<Overlay>,
    monitor: CrtMonitor,
    /// The settings applied, to apply what the commands changed.
    applied: HostSettings,
    /// The last whole frame, for the screenshots.
    last_frame: Option<image::RgbImage>,
    /// The function keys held, for their shortcuts on a press.
    keys_down: HashSet<HostKey>,
    paced_fps: usize,
    quit: bool,
    #[cfg(feature = "gui")]
    gui: Option<amsrust::ui::Gui>,
}

const FUNCTION_KEYS: [(HostKey, &str); 7] = [
    (HostKey::F5, "F5"),
    (HostKey::F6, "F6"),
    (HostKey::F7, "F7"),
    (HostKey::F8, "F8"),
    (HostKey::F9, "F9"),
    (HostKey::F10, "F10"),
    (HostKey::F12, "F12"),
];

impl Window {
    fn notify(&mut self, text: String, error: bool) {
        #[cfg(feature = "gui")]
        if let Some(gui) = &mut self.gui {
            gui.notify(text.clone(), error);
        }
        let _ = error;
        eprintln!("{text}");
    }

    /// The function keys pressed since the last frame: their commands.
    fn shortcuts(&mut self, input: &HostInput, core: &Core, bus: &mut CommandBus) {
        let shift =
            input.down.contains(&HostKey::LeftShift) || input.down.contains(&HostKey::RightShift);
        for (key, name) in FUNCTION_KEYS {
            let down = input.down.contains(&key);
            if down && !self.keys_down.contains(&key) {
                let chord = Chord {
                    ctrl: false,
                    shift,
                    alt: false,
                    key: name,
                };
                let found = commands::shortcut_command(chord, core).or_else(|| {
                    commands::shortcut_command(
                        Chord {
                            shift: false,
                            ..chord
                        },
                        core,
                    )
                });
                if let Some(id) = found {
                    bus.push_id(id);
                }
            }
            if down {
                self.keys_down.insert(key);
            } else {
                self.keys_down.remove(&key);
            }
        }
    }

    /// Do what only the window can do.
    fn request(&mut self, request: HostRequest, core: &mut Core, bus: &mut CommandBus) {
        #[cfg(feature = "gui")]
        if let Some(gui) = &mut self.gui {
            if gui.handle(&request) {
                return;
            }
        }
        match request {
            HostRequest::Quit => self.quit = true,
            HostRequest::Screenshot(path) => {
                let Some(frame) = &self.last_frame else {
                    return self.notify("no picture yet".into(), true);
                };
                let path = path.unwrap_or_else(|| PathBuf::from("bemu.png"));
                let picture = core.settings.display.filter().render_to_image(frame);
                match picture.save(&path) {
                    Ok(()) => self.notify(format!("{}: screenshot saved", path.display()), false),
                    Err(e) => self.notify(format!("{}: {e}", path.display()), true),
                }
            }
            #[cfg(feature = "gui")]
            HostRequest::CopyScreenshot => {
                if let (Some(gui), Some(frame)) = (&self.gui, &self.last_frame) {
                    let picture = core.settings.display.filter().render_to_image(frame);
                    let size = [picture.width() as usize, picture.height() as usize];
                    gui.context()
                        .copy_image(egui::ColorImage::from_rgb(size, picture.as_raw()));
                    self.notify("screenshot copied".into(), false);
                }
            }
            #[cfg(feature = "gui")]
            HostRequest::OpenUrl(url) => {
                if let Some(gui) = &self.gui {
                    gui.context().open_url(egui::OpenUrl::new_tab(url));
                }
            }
            HostRequest::PasteClipboard => match self.window.clipboard_text() {
                Some(text) => {
                    if let Ok(command) =
                        commands::Command::with_arg("autotype-text", commands::Arg::Text(text))
                    {
                        bus.push(command);
                    }
                }
                None => self.notify("nothing to paste".into(), true),
            },
            HostRequest::StartRecording(recording, path) => {
                self.start_recording(recording, path, core)
            }
            HostRequest::StopRecording(recording) => self.stop_recording(recording, core),
            HostRequest::DebugServer(on) => {
                if on {
                    let port = core.settings.debug_port;
                    match core.debugger.listen(port) {
                        Ok(()) => self.notify(
                            format!("debug server: http://127.0.0.1:{port}/api/doc"),
                            false,
                        ),
                        Err(e) => self.notify(e, true),
                    }
                } else {
                    core.debugger.stop_listening();
                    self.notify("debug server stopped".into(), false);
                }
            }
            HostRequest::SaveFile { name, data } => match std::fs::write(&name, data) {
                Ok(()) => self.notify(format!("{name} written"), false),
                Err(e) => self.notify(format!("{name}: {e}"), true),
            },
            HostRequest::SavePrintout => {
                let prefix = core
                    .settings
                    .devices
                    .dmp2000_prefix
                    .clone()
                    .unwrap_or_else(|| "printout".into());
                finish_printer_export(core.motherboard, &prefix);
            }
            other => self.notify(format!("{other:?}: not available here"), true),
        }
    }

    fn start_recording(&mut self, recording: Recording, path: Option<PathBuf>, core: &mut Core) {
        let path = path.unwrap_or_else(|| PathBuf::from("bemu"));
        let text = path.display().to_string();
        match recording {
            Recording::Wav => start_wav(&text, core.motherboard),
            Recording::Ym => start_ym(&text, core.motherboard),
            Recording::Video => {
                let config = core.motherboard.psg().audio_config().unwrap_or_else(|| {
                    core.motherboard.set_audio_output(Some(RECORD_AUDIO()));
                    RECORD_AUDIO()
                });
                // a recording has one size: the filter of the display now, on the CPU
                self.filter = core
                    .settings
                    .display
                    .filter_on(false, core.settings.curvature);
                let (width, height) = self.filter.output_size();
                self.buffer = vec![0; width * height];
                match crate::recorder::Recorder::start(
                    &path,
                    width,
                    height,
                    Some(config),
                    core.motherboard.asic().is_some(),
                ) {
                    Ok(recorder) => self.recorder = Some((recorder, path)),
                    Err(e) => return self.notify(e, true),
                }
            }
            Recording::ScreenshotSeries => {
                let series = Series::parse(&text);
                if let Err(e) = std::fs::create_dir_all(&series.folder) {
                    return self.notify(format!("{}: {e}", series.folder.display()), true);
                }
                self.series = Some(series);
            }
            Recording::Tape => return,
        }
        core.recording.insert(recording);
        self.notify(format!("recording {text}"), false);
    }

    fn stop_recording(&mut self, recording: Recording, core: &mut Core) {
        match recording {
            Recording::Wav => finish_wav(core.motherboard),
            Recording::Ym => finish_ym(core.motherboard),
            Recording::Video => {
                if let Some((recorder, path)) = self.recorder.take() {
                    match recorder.finish() {
                        Ok(frames) => {
                            self.notify(format!("{}: {frames} frames", path.display()), false)
                        }
                        Err(e) => self.notify(e, true),
                    }
                }
            }
            Recording::ScreenshotSeries => {
                if let Some(series) = self.series.take() {
                    self.notify(
                        format!(
                            "{} screenshot(s) in {}",
                            series.shot,
                            series.folder.display()
                        ),
                        false,
                    );
                }
            }
            Recording::Tape => {}
        }
        core.recording.remove(&recording);
    }

    /// Apply the settings the commands changed.
    fn apply(&mut self, core: &mut Core, reloader: &mut Option<Reloader>, args: &Args) {
        let wanted = core.settings.clone();
        let applied = self.applied.clone();
        if wanted.display != applied.display
            || wanted.curvature != applied.curvature
            || wanted.crt_gpu != applied.crt_gpu
        {
            if self.recorder.is_none() {
                self.gpu = wanted.crt_gpu && !self.direct;
                self.filter = wanted.display.filter_on(self.gpu, wanted.curvature);
                let (width, height) = self.filter.output_size();
                self.buffer = vec![0; width * height];
                self.window.set_curvature(wanted.curvature);
            } else {
                // a video has one size
                core.settings.display = applied.display;
                self.notify("not while recording a video".into(), true);
            }
        }
        if wanted.speed != applied.speed || wanted.muted != applied.muted {
            self.window.set_target_fps(self.target_fps(&wanted));
        }
        if wanted.overlay != applied.overlay {
            self.overlay = wanted.overlay.then(Overlay::default);
            core.motherboard.watch_io_writes(wanted.overlay);
        }
        if wanted.keyboard != applied.keyboard {
            self.host_keyboard.set_mapping(wanted.keyboard);
            if wanted.keyboard == KeyboardMapping::Positional && !self.window.knows_key_places() {
                self.notify(
                    "keyboard: this window names the keys after the host's layout: positional on a QWERTY host only".into(),
                    false,
                );
            }
        }
        if wanted.black_level != applied.black_level && core.debugger.listening_port().is_none() {
            self.monitor = viewing_monitor(wanted.black_level);
        }
        if wanted.fullscreen != applied.fullscreen {
            self.window.set_fullscreen(wanted.fullscreen);
        }
        if wanted.watch_files && reloader.is_none() {
            *reloader = Some(Reloader::new(args));
        } else if !wanted.watch_files {
            *reloader = None;
        }
        self.applied = core.settings.clone();
        // F11 and the window's own control switch fullscreen too
        let fullscreen = self.window.fullscreen();
        core.settings.fullscreen = fullscreen;
        self.applied.fullscreen = fullscreen;
    }

    /// The frames a second the window paces (0: the sound device's clock does it, or full
    /// speed).
    fn target_fps(&self, settings: &HostSettings) -> usize {
        match settings.speed {
            Speed::Max => 0,
            Speed::Factor(factor) if factor == 1.0 && !settings.muted => self.paced_fps,
            Speed::Factor(factor) => (50.0 * factor).round().max(1.0) as usize,
        }
    }

    /// Hand the frame's sound to the device: at the real speed only, at the volume chosen.
    fn play(&self, settings: &HostSettings, samples: &[f32]) {
        if settings.muted || settings.speed != Speed::Factor(1.0) {
            return;
        }
        let stereo = self.audio_stereo();
        let gain = f32::from(settings.volume) / 100.0;
        if gain == 1.0 && (settings.stereo || !stereo) {
            play_frame_audio(self.audio.as_ref(), samples, self.window.as_ref());
            return;
        }
        let mut samples = samples.to_vec();
        if stereo && !settings.stereo {
            for pair in samples.chunks_mut(2) {
                let mono = pair.iter().sum::<f32>() / pair.len() as f32;
                pair.fill(mono);
            }
        }
        for sample in &mut samples {
            *sample *= gain;
        }
        play_frame_audio(self.audio.as_ref(), &samples, self.window.as_ref());
    }

    fn audio_stereo(&self) -> bool {
        #[cfg(feature = "audio")]
        return self
            .audio
            .as_ref()
            .is_some_and(|audio| audio.config().stereo);
        #[cfg(not(feature = "audio"))]
        false
    }

    /// The interface's frame: what it asks queued on `bus`, what it draws handed to the window.
    /// Whether it keeps the keyboard.
    fn interface(&mut self, core: &Core, bus: &mut CommandBus) -> bool {
        #[cfg(feature = "gui")]
        if let Some(gui) = &mut self.gui {
            if let Some(raw) = self.window.ui_input() {
                let output = gui.run(raw, core, bus);
                self.window.set_ui_output(output, gui.menu_height());
            }
            return gui.wants_keyboard();
        }
        let _ = (core, bus);
        false
    }

    fn show_again(&mut self) {
        let shown = if self.direct {
            self.window.redraw()
        } else {
            self.window.present(&self.buffer, self.filter.output_size())
        };
        shown.unwrap_or_else(|e| panic!("{e}"));
    }
}

/// The settings the command line starts the window with.
fn settings_of(args: &Args, sound: bool) -> HostSettings {
    let mut settings = HostSettings {
        speed: if args.turbo {
            Speed::Max
        } else {
            Speed::default()
        },
        display: args.display,
        curvature: args.curvature,
        black_level: args.black_level,
        crt_gpu: args.gpu,
        overlay: args.overlay,
        keyboard: args.keyboard,
        host_layout: args.host_keyboard_layout,
        muted: !sound,
        menu_visible: !args.no_menu,
        save_discs: args.save_discs,
        watch_files: args.watch,
        save_snapshot_on_exit: args.save_sna.as_ref().map(PathBuf::from),
        debug_port: args.debug_port(),
        ..HostSettings::default()
    };
    settings.media.disc_inserted(0, args.dsk_a.clone());
    settings.media.disc_inserted(1, args.dsk_b.clone());
    settings.media.snapshot = args.sna.as_ref().map(PathBuf::from);
    settings.media.tape = args.cdt.clone();
    let devices = &mut settings.devices;
    devices.m4_sd = args.m4.as_ref().map(PathBuf::from);
    devices.m4_rom = args.m4_rom.clone();
    devices.m4_slot = args.m4_slot.unwrap_or(devices.m4_slot);
    devices.m4_wifi = !args.m4_no_wifi;
    devices.albireo = args.albireo.as_ref().map(PathBuf::from);
    devices.nova = args.nova.as_ref().map(PathBuf::from);
    devices.dmp2000_prefix = args.dmp2000.clone();
    settings
}

/// The live window, until it is closed (or Ctrl+C, File > Exit, `POST /api/quit`).
pub fn run_window(
    motherboard: &mut Motherboard,
    args: &Args,
    typist: &mut Option<Typist>,
    reloader: &mut Option<Reloader>,
    server: Option<DebugServer>,
) {
    let sound = !args.mute;
    let record = args.record.as_deref();
    let mode: DisplayMode = args.display;
    let (width, height) = mode.output_size();
    #[cfg(feature = "gui")]
    let gui = (args.frontend == FrontendKind::Winit).then(amsrust::ui::Gui::new);
    let window: Box<dyn Frontend> = match args.frontend {
        FrontendKind::Minifb => Box::new(
            MinifbFrontend::open("amsrust", width, height, args.host_keyboard_layout)
                .unwrap_or_else(|e| panic!("{e}")),
        ),
        #[cfg(feature = "gpu-window")]
        FrontendKind::Winit => Box::new(
            frontend::WinitGpuFrontend::open(
                (width, height),
                args.curvature,
                gui.as_ref().map(|gui| gui.context().clone()),
            )
            .unwrap_or_else(|e| panic!("{e}")),
        ),
        #[cfg(not(feature = "gpu-window"))]
        FrontendKind::Winit => panic!("--frontend winit needs a build with --features gpu-window"),
    };
    let direct = window.direct();
    let gpu = args.gpu && !direct;
    let audio = open_audio(sound, motherboard);
    let has_server = server.is_some();
    let mut core = Core {
        motherboard,
        debugger: server.unwrap_or_else(DebugServer::offline),
        settings: settings_of(args, sound),
        requests: Vec::new(),
        recording: HashSet::new(),
        gui: false,
    };
    #[cfg(feature = "gui")]
    {
        core.gui = gui.is_some();
    }
    // The debug server's pictures stay exact: no lifted black.
    let mut monitor = if has_server {
        CrtMonitor::default()
    } else {
        viewing_monitor(args.black_level)
    };
    if has_server {
        monitor.keep_full_frames();
    }
    let filter = mode.filter_on(gpu, args.curvature);
    let (fw, fh) = filter.output_size();
    let mut window = Window {
        window,
        direct,
        gpu,
        filter,
        buffer: vec![0; fw * fh],
        // the audio device's clock paces the emulation, if there is one
        paced_fps: if audio.is_some() { 0 } else { 50 },
        audio,
        recorder: None,
        series: None,
        host_keyboard: HostKeyboard::new(args.keyboard),
        overlay: args.overlay.then(Overlay::default),
        monitor,
        applied: core.settings.clone(),
        last_frame: None,
        keys_down: HashSet::new(),
        quit: false,
        #[cfg(feature = "gui")]
        gui,
    };
    core.motherboard.watch_io_writes(args.overlay);
    let fps = window.target_fps(&core.settings);
    window.window.set_target_fps(fps);
    if args.turbo {
        eprintln!("speed: full (F10 for real time)");
    }
    if let Some(path) = record {
        window.start_recording(Recording::Video, Some(PathBuf::from(path)), &mut core);
    }
    if recording_wav() {
        core.recording.insert(Recording::Wav);
    }
    if recording_ym() {
        core.recording.insert(Recording::Ym);
    }
    let mut bus = CommandBus::default();
    // at full speed, the frames shown (at most 50 a second)
    let mut last_present = std::time::Instant::now();

    while window.window.is_open() && !stop_requested() && !window.quit {
        let mut input = window.window.poll_input();
        let keyboard_taken = window.interface(&core, &mut bus);
        if keyboard_taken {
            input = HostInput::default();
        }
        window.shortcuts(&input, &core, &mut bus);
        bus.run(&mut core);
        for request in std::mem::take(&mut core.requests) {
            window.request(request, &mut core, &mut bus);
        }
        let reports: Vec<_> = bus.take_reports().collect();
        #[cfg(feature = "gui")]
        let reports = match &mut window.gui {
            Some(gui) => {
                gui.report(reports.into_iter());
                Vec::new()
            }
            None => reports,
        };
        for report in reports {
            match report.result {
                Ok(Some(message)) => eprintln!("{message}"),
                Ok(None) => {}
                Err(e) => eprintln!("{}: {e}", report.id),
            }
        }
        window.apply(&mut core, reloader, args);

        if let Some(reloader) = reloader {
            reloader.poll(core.motherboard, typist);
        }
        window.host_keyboard.apply(core.motherboard, &input);
        // After the host keyboard, which would release the typed keys.
        type_due_keys(typist, core.motherboard);

        core.debugger
            .poll_with(core.motherboard, Some(&window.monitor));
        if core.debugger.quit_requested() {
            break;
        }
        if core.debugger.paused() {
            // stopped (F8, a breakpoint, the debugger): the window and its menus stay alive
            window.show_again();
            std::thread::sleep(std::time::Duration::from_millis(15));
            continue;
        }
        // At full speed the frames not shown run without any picture (a quarter faster): one
        // frame's time, then the next loop. A recording and the debug server see every frame.
        let turbo = core.settings.speed.is_max();
        let unseen = turbo
            && window.recorder.is_none()
            && window.series.is_none()
            && core.debugger.listening_port().is_none()
            && !core.debugger.has_breakpoints()
            && last_present.elapsed() < FRAME_SHOWN_EVERY;
        core.motherboard.set_video(!unseen);
        if unseen {
            let end = core.motherboard.t_state_counter().value() + FRAME_T_STATES;
            while core.motherboard.t_state_counter().value() < end {
                core.motherboard.execute_next_opcode_no_video();
            }
            core.motherboard.take_audio_samples();
            core.motherboard.take_io_writes().for_each(drop);
            continue;
        }
        let Some(mut frame) = run_frame(
            core.motherboard,
            &mut window.monitor,
            &mut core.debugger,
            window.overlay.as_mut(),
        ) else {
            continue; // paused mid-frame
        };
        if let Some(overlay) = &mut window.overlay {
            overlay.draw_and_clear(&mut frame);
        }
        match &mut window.last_frame {
            Some(last) if last.dimensions() == frame.dimensions() => last.copy_from_slice(&frame),
            last => *last = Some(frame.clone()),
        }
        if !window.direct || window.recorder.is_some() {
            window.filter.render(&frame, &mut window.buffer);
        }
        let samples = core.motherboard.take_audio_samples();
        export_samples(core.motherboard, &samples);
        if let Some((recorder, _)) = &mut window.recorder {
            let recorded = recorder
                .samples(&samples)
                .and_then(|()| recorder.frame(&window.buffer));
            if let Err(e) = recorded {
                window.notify(format!("recording stopped: {e}"), true);
                window.stop_recording(Recording::Video, &mut core);
            }
        }
        if let Some(series) = &mut window.series {
            if series.seen.is_multiple_of(series.every) {
                let path = series.folder.join(format!("bemu_{:04}.png", series.shot));
                series.shot += 1;
                let picture = core.settings.display.filter().render_to_image(&frame);
                if let Err(e) = picture.save(&path) {
                    eprintln!("{}: {e}", path.display());
                }
            }
            series.seen += 1;
        }
        if turbo {
            // the sound of a fast-forward is dropped, and only some frames are shown
            if last_present.elapsed() < FRAME_SHOWN_EVERY {
                window.monitor.recycle(frame);
                continue;
            }
            last_present = std::time::Instant::now();
        } else {
            window.play(&core.settings, &samples);
        }
        let shown = if window.direct {
            window.window.present_frame(&frame, core.settings.display)
        } else {
            window
                .window
                .present(&window.buffer, window.filter.output_size())
        };
        shown.unwrap_or_else(|e| panic!("{e}"));
        // the next frame is drawn in this one
        window.monitor.recycle(frame);
    }

    for recording in core.recording.clone() {
        window.stop_recording(recording, &mut core);
    }
    // File > Snapshot > Save when the emulator exits, chosen in the menus (the command line's
    // one is saved by main)
    if let Some(path) = &core.settings.save_snapshot_on_exit {
        if args.save_sna.as_ref().map(PathBuf::from).as_ref() != Some(path) {
            match amsrust::snapshot::save_sna(core.motherboard, &path.display().to_string()) {
                Ok(()) => eprintln!("snapshot saved: {}", path.display()),
                Err(e) => eprintln!("cannot save the snapshot: {e}"),
            }
        }
    }
    #[cfg(feature = "audio")]
    if let Some(audio) = &window.audio {
        let missing = audio.missing_frames();
        if missing > 0 {
            eprintln!(
                "sound: {:.1}s of gaps in total (the emulation ran slower than real time)",
                missing as f64 / audio.config().sample_rate as f64
            );
        }
    }
}

/// Run until the next whole frame, or until the debugger pauses the CPU (breakpoint, step) -
/// then `None`, the monitor keeping the part of the frame already drawn.
fn run_frame(
    motherboard: &mut Motherboard,
    monitor: &mut CrtMonitor,
    debugger: &mut DebugServer,
    mut overlay: Option<&mut Overlay>,
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
            crate::note_io_writes(overlay, motherboard, monitor);
        }
        if !debugger.is_active() {
            continue;
        }
        let (x, line) = monitor.beam();
        let (crop_x, crop_y) = monitor.crop_origin();
        debugger.set_beam(Beam {
            x,
            line,
            crop_x,
            crop_y,
        });
        debugger.after_instruction(motherboard);
        if let Some(pc) = debugger.take_stop() {
            eprintln!("debugger: stopped at {pc:#06x}");
        }
        if debugger.paused() {
            break;
        }
    }
    if let Some(frame) = &frame {
        debugger.frame_completed(motherboard);
        debugger.set_frame(frame);
    }
    frame
}
