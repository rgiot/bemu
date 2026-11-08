//! [`Emulator`]: the machine, its picture, keys, files, sound and CSL scripts, for `bemu.js`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use amsrust::csl_runner::{CslReport, CslRunner, MemFiles, Progress, ScreenshotSink};
use amsrust::disc_image::{load_disc_bytes, DiscImage};
use amsrust::display::{CrtProfile, DisplayMode};
use amsrust::fdc::{DriveKind, FdcDrive};
use amsrust::input::{HostInput, HostKey, HostKeyboard};
use amsrust::monitor::Monitor;
use amsrust::motherboard::Motherboard;
use amsrust::psg::AudioConfig;
use image::RgbImage;
use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

use crate::config::Config;
use crate::gfx::{Canvas, Gfx};
use crate::keys;
use crate::ui::WebUi;

/// T-states after which a frame is given up (a machine that never synchronises): two frames.
const MAX_FRAME_T_STATES: usize = 2 * FRAME_T_STATES;
/// T-states of a CPC frame: 312 lines of 64 µs at 4 MHz.
const FRAME_T_STATES: usize = 312 * 64 * 4;
/// T-states a CSL script plays between two looks at the frame it may have completed.
const CSL_SLICE_T_STATES: usize = 20_000;

type Callback = Rc<RefCell<Option<Function>>>;

/// The configured machine, its drives 3.5" ones: they read both sides of any disc image.
fn build_machine(config: &Config) -> Result<Motherboard, JsError> {
    let mut motherboard = config.machine().map_err(|e| JsError::new(&e))?.build();
    for drive in [FdcDrive::A, FdcDrive::B] {
        motherboard.fdc_mut().set_drive_kind(drive, DriveKind::ThreeAndHalfInch);
    }
    Ok(motherboard)
}

#[wasm_bindgen]
pub struct Emulator {
    config: Config,
    display: DisplayMode,
    /// See `Config::curvature`.
    curvature: f32,
    gfx: Gfx,
    /// The machine, when no script plays it.
    motherboard: Motherboard,
    monitor: Monitor,
    frame: Option<RgbImage>,
    /// The discs the page inserted, for a new machine.
    discs: [Option<DiscImage>; 2],
    /// A CSL script playing, with its own machine.
    csl: Option<CslRunner>,
    /// Files a CSL script names.
    files: MemFiles,
    on_screenshot: Callback,
    on_csl_done: Callback,
    audio_rate: Option<u32>,
    keyboard: HostKeyboard,
    /// Keys held, by physical key (`KeyboardEvent.code`), so that a release matches its press
    /// whatever the modifiers made of the key meanwhile.
    held: HashMap<String, HostKey>,
    /// The same keys by their place (the positional keyboard's).
    placed: HashMap<String, HostKey>,
    typed: Vec<char>,
    /// Frames completed, see [`Self::frame_count`].
    frames: u64,
    /// The menus, the palette and the debugger (`config.menu`, WebGPU / WebGL only).
    ui: Option<WebUi>
}

#[wasm_bindgen]
impl Emulator {
    /// An emulator drawing in `canvas`, configured by `config` (see `config.rs`); on WebGPU when
    /// `webgpu`, else on Canvas 2D (raw picture only). Fails when WebGPU does, leaving the canvas
    /// unusable for Canvas 2D.
    pub async fn create(canvas: JsValue, config: JsValue, webgpu: bool) -> Result<Emulator, JsError> {
        console_error_panic_hook::set_once();
        let config = Config::from_js(config).map_err(|e| JsError::new(&e))?;
        let display = config.display().map_err(|e| JsError::new(&e))?;
        let curvature = config.curvature().map_err(|e| JsError::new(&e))?;
        let keyboard = config.keyboard().map_err(|e| JsError::new(&e))?;
        let motherboard = build_machine(&config)?;
        let canvas = Canvas::from_js(canvas).map_err(|e| JsError::new(&e))?;
        let gfx = Gfx::new(canvas, webgpu).await.map_err(|e| JsError::new(&e))?;
        let ui = (config.menu != Some(false) && gfx.backend() == "webgpu").then(|| {
            WebUi::new(amsrust::commands::HostSettings {
                display,
                curvature,
                black_level: config.black_level(),
                keyboard,
                ..Default::default()
            })
        });
        Ok(Emulator {
            monitor: Monitor::default().with_black_level(config.black_level()),
            config,
            display,
            curvature,
            gfx,
            motherboard,
            frame: None,
            discs: [None, None],
            csl: None,
            files: MemFiles::default(),
            on_screenshot: Rc::default(),
            on_csl_done: Rc::default(),
            audio_rate: None,
            keyboard: HostKeyboard::new(keyboard),
            held: HashMap::new(),
            placed: HashMap::new(),
            typed: Vec::new(),
            frames: 0,
            ui
        })
    }

    /// "webgpu" or "canvas2d".
    pub fn backend(&self) -> String {
        self.gfx.backend().to_owned()
    }

    /// Physical size needed to display the CTM mask without downscaling, or zero without GPU.
    pub fn ctm_min_width(&self) -> u32 {
        if self.gfx.backend() == "webgpu" { CrtProfile::Ctm644.params().output_size().0 as u32 } else { 0 }
    }

    pub fn ctm_min_height(&self) -> u32 {
        if self.gfx.backend() == "webgpu" { CrtProfile::Ctm644.params().output_size().1 as u32 } else { 0 }
    }

    /// Minimum physical canvas size for the CTM phosphor mask; zero for other modes.
    pub fn ctm_width(&self) -> u32 {
        if self.display == DisplayMode::Crt(CrtProfile::Ctm644) {
            self.ctm_min_width()
        } else {
            0
        }
    }

    pub fn ctm_height(&self) -> u32 {
        if self.ctm_width() != 0 { self.ctm_min_height() } else { 0 }
    }

    /// A new machine as `config` says (a script playing stops); the discs stay in.
    pub fn configure(&mut self, config: JsValue) -> Result<(), JsError> {
        let config = Config::from_js(config).map_err(|e| JsError::new(&e))?;
        let display = config.display().map_err(|e| JsError::new(&e))?;
        let curvature = config.curvature().map_err(|e| JsError::new(&e))?;
        let keyboard = config.keyboard().map_err(|e| JsError::new(&e))?;
        let motherboard = build_machine(&config)?;
        self.keyboard.set_mapping(keyboard);
        self.config = config;
        self.display = display;
        self.curvature = curvature;
        self.csl = None;
        self.replace_machine(motherboard);
        Ok(())
    }

    /// Switch the machine off and on.
    pub fn reset(&mut self) -> Result<(), JsError> {
        self.csl = None;
        let motherboard = build_machine(&self.config)?;
        self.replace_machine(motherboard);
        Ok(())
    }

    /// "raw", "ctm", "trinitron", "gt65" or "atari" (Canvas 2D shows them raw).
    pub fn set_display(&mut self, name: &str) -> Result<(), JsError> {
        self.display = name.parse().map_err(|e: String| JsError::new(&e))?;
        self.config.display = Some(name.to_owned());
        if let Some(ui) = &mut self.ui {
            ui.settings.display = self.display;
        }
        Ok(())
    }

    /// How curved a CRT's glass is, times its profile's bulge: 0 (flat) to 4 (WebGPU only).
    pub fn set_curvature(&mut self, curvature: f32) -> Result<(), JsError> {
        let config = Config { curvature: Some(curvature), ..self.config.clone() };
        self.curvature = config.curvature().map_err(|e| JsError::new(&e))?;
        self.config = config;
        if let Some(ui) = &mut self.ui {
            ui.settings.curvature = self.curvature;
        }
        Ok(())
    }

    /// The canvas's drawing buffer was resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.gfx.resize(width, height);
    }

    /// Insert a disc in drive 0 (A) or 1 (B): a DSK, or a zip, 7z or tar.gz archive (its first DSK, or the
    /// one named `entry`).
    pub fn insert_disc(&mut self, drive: u8, bytes: &[u8], entry: Option<String>) -> Result<(), JsError> {
        let image = load_disc_bytes(bytes, "disc", entry.as_deref()).map_err(|e| JsError::new(&e))?;
        let unit = usize::from(drive.min(1));
        self.machine().fdc_mut().insert_image(fdc_drive(unit), Some(image.clone()));
        self.discs[unit] = Some(image);
        Ok(())
    }

    /// Changes the disc of a running machine at once: the program sees the new disc on its next access.
    pub fn swap_disc(&mut self, drive: u8, bytes: &[u8], entry: Option<String>) -> Result<(), JsError> {
        let image = load_disc_bytes(bytes, "disc", entry.as_deref()).map_err(|e| JsError::new(&e))?;
        let unit = usize::from(drive.min(1));
        self.machine().fdc_mut().insert_image(fdc_drive(unit), Some(image.clone()));
        self.discs[unit] = Some(image);
        Ok(())
    }

    pub fn eject(&mut self, drive: u8) {
        let unit = usize::from(drive.min(1));
        self.machine().fdc_mut().insert_image(fdc_drive(unit), None);
        self.discs[unit] = None;
    }

    /// A CDT (or TZX) tape in the cassette deck, rewound, PLAY down: it plays when the CPC starts
    /// the motor. Returns what was skipped of it.
    pub fn insert_tape(&mut self, bytes: &[u8]) -> Result<Vec<String>, JsError> {
        let tape = amsrust::tape::Tape::from_cdt(bytes).map_err(|e| JsError::new(&e))?;
        let notes = tape.notes.clone();
        self.machine().insert_tape(Some(tape));
        Ok(notes)
    }

    pub fn eject_tape(&mut self) {
        self.machine().insert_tape(None);
    }

    /// The deck's keys: "play", "stop" or "rewind".
    pub fn tape_control(&mut self, key: &str) -> Result<(), JsError> {
        let deck = self.machine().tape_deck_mut().ok_or_else(|| JsError::new("no tape inserted"))?;
        match key {
            "play" => deck.set_playing(true),
            "stop" => deck.set_playing(false),
            "rewind" => deck.rewind(),
            other => return Err(JsError::new(&format!("{other}: play, stop or rewind"))),
        }
        Ok(())
    }

    /// Where the tape is, in seconds from its start, and whether PLAY is down; none without a tape.
    pub fn tape_position(&self) -> Option<Vec<f64>> {
        let machine = self.csl.as_ref().map_or(&self.motherboard, CslRunner::motherboard);
        machine
            .tape_deck()
            .map(|deck| vec![deck.position_seconds(), deck.tape().duration_seconds(), f64::from(u8::from(deck.playing()))])
    }

    /// Load an SNA snapshot in the machine.
    pub fn load_snapshot(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        amsrust::snapshot::load_sna_bytes(self.machine(), bytes)
            .map(|_| ())
            .map_err(|e| JsError::new(&e))
    }

    /// A file a CSL script names (`disk_insert`, `snapshot_load`, `csl_load`), by its name.
    pub fn add_file(&mut self, name: &str, bytes: Vec<u8>) {
        self.files.insert(name, bytes);
    }

    /// `callback(name, pngBytes)` for each screenshot of a CSL script.
    pub fn on_screenshot(&mut self, callback: Option<Function>) {
        *self.on_screenshot.borrow_mut() = callback;
    }

    /// `callback(report)` when a CSL script ends: `{ screenshots, ssmCodes, ignored, elapsedUs }`,
    /// or `{ error }`.
    pub fn on_csl_done(&mut self, callback: Option<Function>) {
        *self.on_csl_done.borrow_mut() = callback;
    }

    /// Play the CSL script `text` live, on a machine of its own (of the configuration, the
    /// script's `crtc_select`... applied at its `reset`). The keyboard is the script's until
    /// it ends; the machine then stays as the script left it.
    pub fn run_csl(&mut self, text: &str) -> Result<(), JsError> {
        let mut runner = CslRunner::with_io(
            Box::new(self.files.clone()),
            Box::new(JsScreenshots(self.on_screenshot.clone())),
            "screenshots"
        );
        runner.set_machine(self.config.machine().map_err(|e| JsError::new(&e))?);
        runner.set_drive_kind(0, DriveKind::ThreeAndHalfInch);
        runner.set_drive_kind(1, DriveKind::ThreeAndHalfInch);
        runner.motherboard_mut().psg_mut().take_audio_output_from(self.motherboard.psg_mut());
        runner.start_text(text, ".").map_err(|e| JsError::new(&e))?;
        self.csl = Some(runner);
        Ok(())
    }

    /// Stop the script playing; its machine goes on.
    pub fn stop_csl(&mut self) {
        if let Some(mut runner) = self.csl.take() {
            self.adopt(&mut runner);
        }
    }

    pub fn csl_running(&self) -> bool {
        self.csl.is_some()
    }

    /// A key pressed: `code` and `key` of the `KeyboardEvent`, `ctrl_or_meta` its modifiers.
    pub fn key_down(&mut self, code: &str, key: &str, ctrl_or_meta: bool) {
        self.key_event(code, key, true, ctrl_or_meta, false, false, false);
    }

    /// A key pressed or released, with its modifiers: the menus, the palette and their fields
    /// take it when they have the focus (or for a shortcut, Ctrl+Shift+P), the CPC otherwise.
    /// Whether the interface took it.
    #[allow(clippy::too_many_arguments)]
    pub fn key_event(&mut self, code: &str, key: &str, pressed: bool, ctrl: bool, shift: bool, alt: bool, meta: bool) -> bool {
        if let Some(ui) = &mut self.ui {
            if ui.key(code, key, pressed, ctrl, shift, alt, meta) {
                return true;
            }
        }
        if pressed {
            self.cpc_key_down(code, key, ctrl || meta);
        } else {
            self.key_up(code);
        }
        false
    }

    /// The pointer over the canvas, at (`x`, `y`) CSS pixels: "move", "down", "up" or "leave",
    /// `button` as `PointerEvent.button`. Whether the interface is under it.
    pub fn ui_pointer(&mut self, kind: &str, x: f32, y: f32, button: u8) -> bool {
        self.ui.as_mut().is_some_and(|ui| ui.pointer(kind, x, y, button))
    }

    /// The wheel over the canvas, in CSS pixels.
    pub fn ui_wheel(&mut self, dx: f32, dy: f32) {
        if let Some(ui) = &mut self.ui {
            ui.wheel(dx, dy);
        }
    }

    /// The canvas's device pixels per CSS pixel (`devicePixelRatio`).
    pub fn ui_scale(&mut self, pixels_per_point: f32) {
        if let Some(ui) = &mut self.ui {
            ui.set_pixels_per_point(pixels_per_point);
        }
    }

    /// Whether the page has the menus.
    pub fn has_ui(&self) -> bool {
        self.ui.is_some()
    }

    /// What the menus ask the page to do since the last call (see `ui.rs`, `PageRequest`).
    pub fn take_ui_requests(&mut self) -> Result<JsValue, JsError> {
        let requests = self.ui.as_mut().map(|ui| std::mem::take(&mut ui.page)).unwrap_or_default();
        serde_wasm_bindgen::to_value(&requests).map_err(|e| JsError::new(&e.to_string()))
    }

    /// The file the page picked for the command `id` (a `pickFile` request).
    pub fn answer_file(&mut self, id: &str, name: String, bytes: Vec<u8>) {
        if let Some(ui) = &mut self.ui {
            ui.answer_file(id, name, bytes);
        }
    }

    fn cpc_key_down(&mut self, code: &str, key: &str, ctrl_or_meta: bool) {
        if let Some(host_key) = keys::host_key(code, key) {
            self.held.insert(code.to_owned(), host_key);
        }
        if let Some(placed) = keys::placed_key(code) {
            self.placed.insert(code.to_owned(), placed);
        }
        if let Some(c) = keys::typed_char(key, ctrl_or_meta) {
            self.typed.push(c);
        }
    }

    pub fn key_up(&mut self, code: &str) {
        self.held.remove(code);
        self.placed.remove(code);
    }

    /// Every key up (the page lost the focus).
    pub fn release_keys(&mut self) {
        self.held.clear();
        self.placed.clear();
    }

    /// "symbolic" (a key types its character) or "positional" (a key presses the CPC key at
    /// its place, for games).
    pub fn set_keyboard(&mut self, mapping: &str) -> Result<(), JsError> {
        let config = Config { keyboard: Some(mapping.to_owned()), ..self.config.clone() };
        self.keyboard.set_mapping(config.keyboard().map_err(|e| JsError::new(&e))?);
        self.config = config;
        Ok(())
    }

    /// Run the machine to its next frame, and show it.
    pub fn run_frame(&mut self) -> Result<(), JsError> {
        self.advance_frame()?;
        self.redraw()
    }

    /// Run the machine to its next frame without showing it ([`Self::redraw`] shows the last
    /// one): several frames a refresh, at more than real time, only the last needs drawing.
    pub fn advance_frame(&mut self) -> Result<(), JsError> {
        if self.ui.as_ref().is_some_and(WebUi::paused) {
            return Ok(());
        }
        let step = self.csl.as_mut().map(runner_frame);
        match step {
            Some(step) => self.csl_frame(step)?,
            None => self.free_frame()
        }
        Ok(())
    }

    /// Run the machine for a frame's time without any picture: a frame nobody will see (above real
    /// time, all but the last frame of a refresh), about a quarter faster. A CSL script keeps its
    /// picture (its screenshots need it).
    pub fn advance_frame_unseen(&mut self) -> Result<(), JsError> {
        if self.csl.is_some() || self.ui.as_ref().is_some_and(|ui| ui.debugger.is_active()) {
            return self.advance_frame();
        }
        self.apply_keyboard();
        self.motherboard.set_video(false);
        let end = self.motherboard.t_state_counter().value() + FRAME_T_STATES;
        while self.motherboard.t_state_counter().value() < end {
            self.motherboard.execute_next_opcode_no_video();
        }
        self.motherboard.set_video(true);
        self.frames += 1;
        Ok(())
    }

    /// Frames the machine produced since it was created (for the page's speed figures).
    pub fn frame_count(&self) -> f64 {
        self.frames as f64
    }

    /// Type `text` on the keyboard, a character after the other as the host's typed characters
    /// are (`\n` is RETURN). Not while a CSL script plays: the keyboard is the script's.
    pub fn type_text(&mut self, text: &str) {
        self.typed.extend(text.chars().filter(|&c| c != '\r'));
    }

    /// The last frame, as a PNG file of the monitor's picture (768x272, raw); none before the
    /// first frame.
    pub fn screenshot(&self) -> Result<Option<Vec<u8>>, JsError> {
        let Some(frame) = &self.frame else {
            return Ok(None);
        };
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(frame.clone())
            .write_to(&mut png, image::ImageOutputFormat::Png)
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Some(png))
    }

    /// Whether the disc drive motor is running, i.e. the drive is being accessed.
    pub fn drive_active(&self) -> bool {
        let machine = self.csl.as_ref().map_or(&self.motherboard, CslRunner::motherboard);
        machine.fdc().motor()
    }

    /// How many disc transfers the machine has begun (wrapping): the drive light flashes when it changes.
    pub fn drive_transfers(&self) -> u32 {
        let machine = self.csl.as_ref().map_or(&self.motherboard, CslRunner::motherboard);
        machine.fdc().executions()
    }

    /// The DSK image in drive 0 (A) or 1 (B), as the machine has written it; none if empty.
    pub fn disc_image(&self, drive: u8) -> Option<Vec<u8>> {
        let machine = self.csl.as_ref().map_or(&self.motherboard, CslRunner::motherboard);
        machine.fdc().image(fdc_drive(usize::from(drive.min(1)))).map(DiscImage::to_bytes)
    }

    /// The machine's state as a version 3 SNA file (the script's machine while one plays).
    pub fn save_snapshot(&self) -> Result<Vec<u8>, JsError> {
        let machine = self.csl.as_ref().map_or(&self.motherboard, CslRunner::motherboard);
        amsrust::snapshot::save_sna_bytes(machine).map_err(|e| JsError::new(&e))
    }

    /// Show the last frame again.
    pub fn redraw(&mut self) -> Result<(), JsError> {
        let ui = match self.ui.take() {
            Some(mut ui) => {
                let size = self.gfx.size();
                let csl = self.csl.is_some();
                let output = ui.frame(size, self.machine(), csl);
                // the menus changed the display, the keyboard
                let settings = &ui.settings;
                self.display = settings.display;
                self.curvature = settings.curvature;
                self.keyboard.set_mapping(settings.keyboard);
                Some((ui, output))
            },
            None => None
        };
        let result = match (&self.frame, ui) {
            (Some(frame), Some((ui, output))) => {
                let shown = self.gfx.show_with_ui(frame, self.display, self.curvature, Some((&ui.gui, output)));
                self.ui = Some(ui);
                shown
            },
            (None, Some((ui, _))) => {
                self.ui = Some(ui);
                Ok(())
            },
            (Some(frame), None) => self.gfx.show(frame, self.display, self.curvature),
            (None, None) => Ok(())
        };
        result.map_err(|e| JsError::new(&e))
    }

    /// Produce sound at `sample_rate` (the `AudioContext`'s), stereo: see [`Self::audio_samples`].
    pub fn enable_audio(&mut self, sample_rate: u32) {
        self.audio_rate = Some(sample_rate);
        self.machine().set_audio_output(Some(AudioConfig::stereo(sample_rate)));
    }

    /// The sound since the previous call: stereo samples, interleaved.
    pub fn audio_samples(&mut self) -> Vec<f32> {
        self.machine().take_audio_samples()
    }
}

/// What a step of the script gave.
enum Step {
    /// A frame completed: `CslRunner::last_frame`.
    Frame,
    Done(Result<CslReport, String>),
    Nothing
}

/// Play the script to its next frame (a frame's time at most).
fn runner_frame(runner: &mut CslRunner) -> Step {
    let mut played = 0;
    while played < MAX_FRAME_T_STATES {
        match runner.advance(CSL_SLICE_T_STATES) {
            Ok(Progress::Frame) => return Step::Frame,
            Ok(Progress::Running) => played += CSL_SLICE_T_STATES,
            Ok(Progress::Done(report)) => return Step::Done(Ok(report)),
            Err(e) => return Step::Done(Err(e))
        }
    }
    Step::Nothing
}

impl Emulator {
    /// The machine running: the script's, or the page's.
    fn machine(&mut self) -> &mut Motherboard {
        match self.csl.as_mut() {
            Some(runner) => runner.motherboard_mut(),
            None => &mut self.motherboard
        }
    }

    /// `motherboard` replaces the machine: the discs go in, the sound goes on.
    fn replace_machine(&mut self, mut motherboard: Motherboard) {
        motherboard.psg_mut().take_audio_output_from(self.motherboard.psg_mut());
        if let (Some(rate), None) = (self.audio_rate, motherboard.psg().audio_config()) {
            motherboard.set_audio_output(Some(AudioConfig::stereo(rate)));
        }
        for (unit, disc) in self.discs.iter().enumerate() {
            motherboard.fdc_mut().insert_image(fdc_drive(unit), disc.clone());
        }
        // the deck keeps its tape, where it is
        motherboard.take_tape_deck_from(&mut self.motherboard);
        self.motherboard = motherboard;
        self.monitor = Monitor::default().with_black_level(self.config.black_level());
    }

    /// The machine of a script that ends goes on as the page's.
    fn adopt(&mut self, runner: &mut CslRunner) {
        let motherboard = std::mem::take(runner.motherboard_mut());
        self.motherboard = motherboard;
        self.monitor = Monitor::default().with_black_level(self.config.black_level());
    }

    fn csl_frame(&mut self, step: Step) -> Result<(), JsError> {
        match step {
            Step::Frame => {
                self.frames += 1;
                // the script's frame, copied in the one already allocated
                if let Some(source) = self.csl.as_ref().and_then(CslRunner::last_frame) {
                    match &mut self.frame {
                        Some(frame) if frame.dimensions() == source.dimensions() => frame.copy_from_slice(source),
                        frame => *frame = Some(source.clone())
                    }
                }
            },
            Step::Nothing => {},
            Step::Done(result) => {
                let mut runner = self.csl.take().expect("a script plays");
                self.adopt(&mut runner);
                let report = match &result {
                    Ok(report) => report_object(report),
                    Err(e) => {
                        let object = Object::new();
                        let _ = Reflect::set(&object, &"error".into(), &e.into());
                        object.into()
                    }
                };
                if let Some(callback) = self.on_csl_done.borrow().as_ref() {
                    callback.call1(&JsValue::NULL, &report).map_err(|e| JsError::new(&format!("{e:?}")))?;
                }
            }
        }
        Ok(())
    }

    /// The page's machine, with the host keyboard, to its next frame.
    fn free_frame(&mut self) {
        self.apply_keyboard();
        let start = self.motherboard.t_state_counter().value();
        loop {
            let monitor = &mut self.monitor;
            self.motherboard.execute_next_opcode_with_video(|(rgb, sync)| monitor.receive(rgb, sync));
            if let Some(ui) = self.ui.as_mut().filter(|ui| ui.debugger.is_active()) {
                ui.debugger.after_instruction(&mut self.motherboard);
                if ui.debugger.paused() {
                    return;
                }
            }
            if let Some(frame) = self.monitor.take_frame() {
                self.frames += 1;
                // the previous frame is drawn in next
                if let Some(previous) = self.frame.replace(frame) {
                    self.monitor.recycle(previous);
                }
                return;
            }
            if self.motherboard.t_state_counter().value().wrapping_sub(start) > MAX_FRAME_T_STATES {
                return;
            }
        }
    }
}

impl Emulator {
    /// The host keyboard to the page's machine, once a frame.
    fn apply_keyboard(&mut self) {
        let input = HostInput {
            down: self.held.values().copied().collect(),
            typed: std::mem::take(&mut self.typed),
            placed: self.placed.values().copied().collect()
        };
        self.keyboard.apply(&mut self.motherboard, &input);
    }
}

fn fdc_drive(unit: usize) -> FdcDrive {
    if unit == 0 {
        FdcDrive::A
    }
    else {
        FdcDrive::B
    }
}

/// `{ screenshots: [names], ssmCodes, ignored, elapsedUs }`.
fn report_object(report: &CslReport) -> JsValue {
    let object = Object::new();
    let names: Array = report
        .screenshots
        .iter()
        .map(|shot| JsValue::from(shot.path.file_name().unwrap_or_default().to_string_lossy().into_owned()))
        .collect();
    let codes: Array = report.ssm_codes.iter().map(|&code| JsValue::from(code)).collect();
    let ignored: Array = report.ignored.iter().map(|note| JsValue::from(note.as_str())).collect();
    let _ = Reflect::set(&object, &"screenshots".into(), &names);
    let _ = Reflect::set(&object, &"ssmCodes".into(), &codes);
    let _ = Reflect::set(&object, &"ignored".into(), &ignored);
    let _ = Reflect::set(&object, &"elapsedUs".into(), &JsValue::from(report.elapsed_us as f64));
    object.into()
}

/// The screenshots of a script, to the page's callback as PNG files.
struct JsScreenshots(Callback);

impl ScreenshotSink for JsScreenshots {
    fn save(&mut self, dir: &Path, file: &str, frame: &RgbImage) -> Result<PathBuf, String> {
        if let Some(callback) = self.0.borrow().as_ref() {
            let mut png = Vec::new();
            image::DynamicImage::ImageRgb8(frame.clone())
                .write_to(&mut png, image::ImageOutputFormat::Png)
                .map_err(|e| e.to_string())?;
            callback
                .call2(&JsValue::NULL, &JsValue::from(file), &Uint8Array::from(png.as_slice()))
                .map_err(|e| format!("screenshot callback: {e:?}"))?;
        }
        Ok(dir.join(file))
    }
}
