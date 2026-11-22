//! [`Emulator`]: the machine, its picture, keys, files, sound and CSL scripts, for `bemu.js`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use amsrust::display::PictureSize;
use amsrust::csl_runner::{CslReport, CslRunner, MemFiles, ScreenshotSink};
use amsrust::frame::{ScriptStep, FRAME_T_STATES};
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

type Callback = Rc<RefCell<Option<Function>>>;

/// The configured machine, its drives 3.5" ones: they read both sides of any disc image.
/// The card `T` of `old`'s expansion port, if there is one, onto `new`'s.
#[cfg(any(feature = "dandanator", feature = "nova", feature = "albireo", feature = "m4"))]
fn carry_card<T: amsrust::expansion_port::ExpansionDevice>(old: &mut Motherboard, new: &mut Motherboard) {
    if let Some(card) = old.unplug_expansion::<T>() {
        new.plug_expansion(card);
    }
}

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
    /// The M4's SD card, in memory: the page's files, whether the board is plugged or not.
    #[cfg(feature = "m4")]
    m4_card: amsrust::m4::MemoryCard,
    /// The CPC+ cartridge the page inserted, for a new machine (kept while the model is a CPC+).
    cartridge: Option<amsrust::asic::Cartridge>,
    /// The upper ROMs the page inserted (slot, image), for a new machine.
    roms: Vec<(u8, Vec<u8>)>,
    /// A CSL script playing, with its own machine.
    csl: Option<CslRunner>,
    /// What the script playing, or the last one, played (the CSL log window).
    csl_log: Option<amsrust::csl_runner::CslLog>,
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
    /// Avoid repeating the same browser gamepad access error on every frame.
    gamepad_error_reported: bool,
    /// Frames completed, see [`Self::frame_count`].
    frames: u64,
    /// The menus, the palette and the debugger (`config.menu`, WebGPU / WebGL only).
    ui: Option<WebUi>,
    /// The debugger without the menus (none with them: theirs is the one): what [`Self::api`]
    /// asks, a debugger in the page's host (cpclib-dap) driving it.
    debugger: Option<amsrust::debug_server::DebugServer>,
    /// The CRTC / Gate Array writes drawn on the picture (Debug > Overlay).
    overlay: Option<amsrust::overlay::Overlay>,
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
                brightness: config.brightness.unwrap_or(0),
                hhold: config.h_hold.unwrap_or(0),
                vhold: config.v_hold.unwrap_or(0),
                keyboard,
                ..Default::default()
            })
        });
        Ok(Emulator {
            monitor: config.monitor(),
            config,
            display,
            curvature,
            gfx,
            motherboard,
            frame: None,
            discs: [None, None],
            #[cfg(feature = "m4")]
            m4_card: amsrust::m4::MemoryCard::new(),
            cartridge: None,
            roms: Vec::new(),
            csl: None,
            csl_log: None,
            files: MemFiles::default(),
            on_screenshot: Rc::default(),
            on_csl_done: Rc::default(),
            audio_rate: None,
            keyboard: HostKeyboard::new(keyboard),
            held: HashMap::new(),
            placed: HashMap::new(),
            typed: Vec::new(),
            gamepad_error_reported: false,
            frames: 0,
            debugger: ui.is_none().then(amsrust::debug_server::DebugServer::offline),
            ui,
            overlay: None,
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
        if let Some(ui) = &mut self.ui {
            ui.settings.keyboard = keyboard;
            ui.settings.display = display;
            ui.settings.curvature = curvature;
            ui.settings.black_level = self.config.black_level();
            ui.settings.brightness = self.config.brightness.unwrap_or(0);
            ui.settings.hhold = self.config.h_hold.unwrap_or(0);
            ui.settings.vhold = self.config.v_hold.unwrap_or(0);
        }
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

    /// "raw", "ctm", "trinitron", "gt65", "gt64", "mm12" or "atari" (Canvas 2D shows them raw).
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

    /// A CPC+ cartridge (a CPR file, or a zip holding one) in the machine, which restarts to boot it.
    pub fn insert_cartridge(&mut self, bytes: &[u8], entry: Option<String>) -> Result<(), JsError> {
        if !self.machine().model().is_plus() {
            return Err(JsError::new("a cartridge goes in a CPC+ (model 464plus, 6128plus or gx4000)"));
        }
        // checked before anything changes: a bad file leaves the machine as it was
        let cartridge = amsrust::asic::Cartridge::from_bytes(bytes, "cartridge", entry.as_deref())
            .map_err(|e| JsError::new(&e))?;
        self.cartridge = Some(cartridge.clone());
        let machine = self.machine();
        machine.insert_cartridge(Some(cartridge));
        machine.restart();
        Ok(())
    }

    /// The cartridge out of the machine, which restarts (on the system cartridge's BASIC).
    pub fn eject_cartridge(&mut self) {
        self.cartridge = None;
        let machine = self.machine();
        machine.insert_cartridge(None);
        machine.restart();
    }

    /// An upper ROM (a 16 KB image: Protext, Maxam, UniDOS...) in `slot`, 0 to 255. The machine is not
    /// reset: the firmware finds the ROM at the next reset. It stays in through `reset()` and `configure()`.
    pub fn insert_rom(&mut self, slot: u8, bytes: &[u8]) -> Result<(), JsError> {
        self.machine().set_upper_rom(slot, Some(bytes)).map_err(|e| JsError::new(&e))?;
        self.roms.retain(|(taken, _)| *taken != slot);
        self.roms.push((slot, bytes.to_vec()));
        Ok(())
    }

    /// Empty the upper ROM `slot`.
    pub fn remove_rom(&mut self, slot: u8) -> Result<(), JsError> {
        self.machine().set_upper_rom(slot, None).map_err(|e| JsError::new(&e))?;
        self.roms.retain(|(taken, _)| *taken != slot);
        Ok(())
    }

    /// A Dandanator Mini (a 512 KB romset, uploaded: what the CPC programs stays in memory, see
    /// `dandanator_flash`) on the expansion port, in place of the CPC's ROMs: the machine restarts to boot it.
    pub fn insert_dandanator(&mut self, romset: &[u8]) -> Result<(), JsError> {
        #[cfg(feature = "dandanator")]
        {
            use amsrust::dandanator::Dandanator;
            let machine = self.machine();
            machine.unplug_expansion::<Dandanator>();
            machine.plug_expansion(Box::new(Dandanator::new(romset)));
            machine.restart();
            Ok(())
        }
        #[cfg(not(feature = "dandanator"))]
        {
            let _ = romset;
            Err(JsError::new("this build has no Dandanator"))
        }
    }

    /// The Dandanator unplugged; the machine restarts.
    pub fn remove_dandanator(&mut self) {
        #[cfg(feature = "dandanator")]
        {
            let machine = self.machine();
            machine.unplug_expansion::<amsrust::dandanator::Dandanator>();
            machine.restart();
        }
    }

    /// The Dandanator's flash (512 KB), with what the CPC programmed in it; none without one.
    pub fn dandanator_flash(&self) -> Option<Vec<u8>> {
        #[cfg(feature = "dandanator")]
        {
            let machine = self.csl.as_ref().map_or(&self.motherboard, CslRunner::motherboard);
            machine.expansion_port()?.device::<amsrust::dandanator::Dandanator>().map(|card| card.flash().to_vec())
        }
        #[cfg(not(feature = "dandanator"))]
        None
    }

    /// A Nova (an ethernet card with 128 KB of NVRAM, kept in memory) on the expansion port.
    pub fn plug_nova(&mut self) -> Result<(), JsError> {
        #[cfg(feature = "nova")]
        {
            use amsrust::nova::Nova;
            let card = Nova::new(None).map_err(|e| JsError::new(&e))?;
            let machine = self.machine();
            machine.unplug_expansion::<Nova>();
            machine.plug_expansion(Box::new(card));
            Ok(())
        }
        #[cfg(not(feature = "nova"))]
        Err(JsError::new("this build has no Nova"))
    }

    /// The Nova unplugged.
    pub fn unplug_nova(&mut self) {
        #[cfg(feature = "nova")]
        self.machine().unplug_expansion::<amsrust::nova::Nova>();
    }

    /// An Albireo (a CH376 board) on the expansion port, its USB key the FAT image `image`: what the CPC
    /// writes stays in memory.
    pub fn insert_albireo(&mut self, image: Vec<u8>) -> Result<(), JsError> {
        #[cfg(feature = "albireo")]
        {
            use amsrust::ch376::Albireo;
            let machine = self.machine();
            machine.unplug_expansion::<Albireo>();
            machine.plug_expansion(Box::new(Albireo::with_image(image)));
            Ok(())
        }
        #[cfg(not(feature = "albireo"))]
        {
            let _ = image;
            Err(JsError::new("this build has no Albireo"))
        }
    }

    /// The Albireo unplugged.
    pub fn remove_albireo(&mut self) {
        #[cfg(feature = "albireo")]
        self.machine().unplug_expansion::<amsrust::ch376::Albireo>();
    }

    /// An M4 board (Duke's SD/WiFi board; its ROM v2.0.8 in upper ROM slot 6) on the expansion port, its SD
    /// card the files the page keeps in memory (`m4_put_file`...): the CPC reads them with `|DIR`, `|CD`,
    /// `LOAD`, `RUN"` and the M4 tools, and what it saves is there for the page to take. The machine
    /// restarts, so that the firmware finds the ROM. The WiFi, HTTP and the board's web page are not
    /// emulated.
    pub fn plug_m4(&mut self) -> Result<(), JsError> {
        #[cfg(feature = "m4")]
        {
            use amsrust::m4::M4Board;
            let card = M4Board::with_bundled_rom_in_memory(self.m4_card.clone()).map_err(|e| JsError::new(&e))?;
            let machine = self.machine();
            machine.unplug_expansion::<M4Board>();
            machine.plug_expansion(Box::new(card));
            machine.restart();
            Ok(())
        }
        #[cfg(not(feature = "m4"))]
        Err(JsError::new("this build has no M4"))
    }

    /// The M4 unplugged; the machine restarts. Its files stay for the next `plug_m4`.
    pub fn unplug_m4(&mut self) {
        #[cfg(feature = "m4")]
        {
            let machine = self.machine();
            machine.unplug_expansion::<amsrust::m4::M4Board>();
            machine.restart();
        }
    }

    /// `path` ("games/demo.bin") of the M4's card written with `bytes`, its folders made.
    pub fn m4_put_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), JsError> {
        #[cfg(feature = "m4")]
        return self.m4_card.write_file(path, bytes).map_err(|e| JsError::new(&e));
        #[cfg(not(feature = "m4"))]
        {
            let _ = (path, bytes);
            Err(JsError::new("this build has no M4"))
        }
    }

    /// The file `path` of the M4's card; none if there is none.
    pub fn m4_get_file(&self, path: &str) -> Option<Vec<u8>> {
        #[cfg(feature = "m4")]
        return self.m4_card.read_file(path);
        #[cfg(not(feature = "m4"))]
        {
            let _ = path;
            None
        }
    }

    /// The folder `path` of the M4's card: `[{ name, folder, size }]`, folders first; none if it is not a folder.
    pub fn m4_list(&self, path: &str) -> Option<Array> {
        #[cfg(feature = "m4")]
        return self.m4_card.list(path).map(|entries| {
            entries
                .into_iter()
                .map(|(name, folder, size)| {
                    let entry = Object::new();
                    let _ = Reflect::set(&entry, &"name".into(), &name.into());
                    let _ = Reflect::set(&entry, &"folder".into(), &folder.into());
                    let _ = Reflect::set(&entry, &"size".into(), &(size as f64).into());
                    JsValue::from(entry)
                })
                .collect()
        });
        #[cfg(not(feature = "m4"))]
        {
            let _ = path;
            None
        }
    }

    /// A file, or a folder and what is in it, taken from the M4's card; false if there was none.
    pub fn m4_remove(&mut self, path: &str) -> bool {
        #[cfg(feature = "m4")]
        return self.m4_card.remove(path);
        #[cfg(not(feature = "m4"))]
        {
            let _ = path;
            false
        }
    }

    /// A folder (and the ones above it) made on the M4's card.
    pub fn m4_make_dir(&mut self, path: &str) -> Result<(), JsError> {
        #[cfg(feature = "m4")]
        return self.m4_card.make_dir(path).map_err(|e| JsError::new(&e));
        #[cfg(not(feature = "m4"))]
        {
            let _ = path;
            Err(JsError::new("this build has no M4"))
        }
    }

    /// Record what the CPC writes to the cassette port (`SAVE` on the CPC) until `stop_tape_recording`.
    pub fn start_tape_recording(&mut self) {
        self.machine().start_tape_recording();
    }

    /// The recording as a CDT file; none when nothing was being recorded.
    pub fn stop_tape_recording(&mut self) -> Option<Vec<u8>> {
        self.machine().stop_tape_recording()
    }

    /// What a DMP2000 printer (`dmp2000: true`) printed, as files named after `prefix`:
    /// `[{ name, data }]` with `prefix.txt` (the text), `prefix.prn` (the bytes the CPC sent) and
    /// `prefix_001.png`... (the sheets with ink). None without a printer.
    pub fn printout(&self, prefix: &str) -> Option<Array> {
        let machine = self.csl.as_ref().map_or(&self.motherboard, CslRunner::motherboard);
        let printer = machine.printer_port()?.device_as::<amsrust::dmp2000::Dmp2000>()?;
        Some(
            printer
                .files(prefix)
                .into_iter()
                .map(|(name, data)| {
                    let file = Object::new();
                    let _ = Reflect::set(&file, &"name".into(), &name.into());
                    let _ = Reflect::set(&file, &"data".into(), &Uint8Array::from(data.as_slice()));
                    JsValue::from(file)
                })
                .collect(),
        )
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
        amsrust::frame::start_script(&mut runner, &mut self.motherboard);
        runner.start_text(text, ".").map_err(|e| JsError::new(&e))?;
        self.csl_log = Some(runner.log());
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
                // a key released while the menus have the keyboard is released on the CPC too
                // (pressed before they took it: Ctrl and Shift of Ctrl+Shift+P)
                if !pressed {
                    self.key_up(code);
                }
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

    /// The page's speed (times real time, `Infinity` at full speed), for the menus.
    pub fn ui_speed(&mut self, speed: f64) {
        if let Some(ui) = &mut self.ui {
            ui.page_speed(if speed.is_finite() {
                amsrust::commands::Speed::Factor(speed as f32)
            } else {
                amsrust::commands::Speed::Max
            });
        }
    }

    /// Whether the page is fullscreen, for the menus.
    pub fn ui_fullscreen(&mut self, on: bool) {
        if let Some(ui) = &mut self.ui {
            ui.page_fullscreen(on);
        }
    }

    /// Whether the page skips frames when too slow (its `frameSkip`), for the menus.
    pub fn ui_frame_skip(&mut self, on: bool) {
        if let Some(ui) = &mut self.ui {
            ui.page_frame_skip(on);
        }
    }

    /// The page's volume (0-1) and mute, for the menus.
    pub fn ui_volume(&mut self, volume: f64, muted: bool) {
        if let Some(ui) = &mut self.ui {
            ui.page_volume(volume, muted);
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
        let mapping = config.keyboard().map_err(|e| JsError::new(&e))?;
        self.keyboard.set_mapping(mapping);
        self.config = config;
        if let Some(ui) = &mut self.ui {
            ui.settings.keyboard = mapping;
        }
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
        // the API's commands applied, its pause told (paused, no instruction runs to do it)
        if let Some(debugger) = &mut self.debugger {
            debugger.poll_with(&mut self.motherboard, Some(&self.monitor));
        }
        if self.ui.as_ref().is_some_and(WebUi::paused) || self.debugger.as_ref().is_some_and(|d| d.paused()) {
            return Ok(());
        }
        let step = self.csl.as_mut().map(amsrust::frame::play_frame);
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
        if self.csl.is_some()
            || self.ui.as_ref().is_some_and(|ui| ui.debugger.is_active())
            || self.debugger.as_ref().is_some_and(|d| d.is_active())
        {
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

    /// The debug API, in the page (it has no HTTP server): `method` and `path` (`GET`,
    /// `/api/ping`), the query as in a URL (`a=1&b=%262`, decoded here) and the body, answered
    /// as the desktop's server answers them, by the same code (`DebugServer::handle_with`):
    /// `{ status, contentType, body, headers }`, the body a `Uint8Array`, the headers
    /// `[name, value]` pairs. The debugger of the menus when they are shown, else the page's own.
    pub fn api(&mut self, method: &str, path: &str, query: &str, body: Vec<u8>) -> JsValue {
        let request = amsrust::debug_server::ApiRequest {
            method: method.to_ascii_uppercase(),
            path: path.to_owned(),
            query: amsrust::debug_server::query_pairs(query),
            body,
        };
        let debugger = api_debugger(&mut self.ui, &mut self.debugger);
        let response = debugger.handle_with(&request, &mut self.motherboard, Some(&self.monitor));
        let object = Object::new();
        let _ = Reflect::set(&object, &"status".into(), &JsValue::from(response.status));
        let _ = Reflect::set(&object, &"contentType".into(), &JsValue::from(response.content_type));
        let _ = Reflect::set(&object, &"body".into(), &Uint8Array::from(response.body.as_slice()));
        let headers = Array::new();
        for (name, value) in &response.headers {
            headers.push(&Array::of2(&name.into(), &value.into()));
        }
        let _ = Reflect::set(&object, &"headers".into(), &headers);
        object.into()
    }

    /// The debug events since the previous call, as `/api/events` streams them (`event:
    /// NAME\ndata: JSON\n\n` each). The first call subscribes the page: from then on the
    /// debugger sees every instruction, as a served one does.
    pub fn debug_events(&mut self) -> Array {
        api_debugger(&mut self.ui, &mut self.debugger).local_events().into_iter().map(JsValue::from).collect()
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
                let log = self.csl_log.clone();
                let output = ui.frame(size, self.machine(), csl, log);
                // the menus changed the display, the keyboard (a new mapping releases the keys:
                // only on a change)
                let settings = &ui.settings;
                self.display = settings.display;
                self.curvature = settings.curvature;
                if self.keyboard.mapping() != settings.keyboard {
                    self.keyboard.set_mapping(settings.keyboard);
                }
                if settings.black_level != self.config.black_level()
                    || settings.brightness != self.config.brightness.unwrap_or(0)
                    || (Some(settings.hhold), Some(settings.vhold)) != (self.config.h_hold, self.config.v_hold)
                {
                    self.config.black_level = Some(settings.black_level);
                    self.config.brightness = Some(settings.brightness);
                    self.config.h_hold = Some(settings.hhold);
                    self.config.v_hold = Some(settings.vhold);
                    self.monitor = self.config.monitor();
                }
                if settings.overlay != self.overlay.is_some() {
                    self.overlay = settings.overlay.then(amsrust::overlay::Overlay::default);
                    self.motherboard.watch_io_writes(settings.overlay);
                }
                Some((ui, output))
            },
            None => None
        };
        // paused (the debugger stopped it): the frame being drawn up to the beam, the last one
        // after it, the beam's lines
        let paused = ui
            .as_ref()
            .filter(|(ui, _)| ui.paused() && self.csl.is_none())
            .map(|_| self.monitor.paused_picture(self.frame.as_ref()));
        let shown_frame = paused.as_ref().or(self.frame.as_ref());
        let result = match (shown_frame, ui) {
            (Some(frame), Some((mut ui, output))) => {
                self.gfx.set_picture_size(PictureSize::of(ui.settings.free_zoom, ui.settings.native_zoom));
                let shown = self.gfx.show_with_ui(frame, self.display, self.curvature, Some((&ui.gui, output)));
                let shown = shown.map(|undrawn| {
                    if let Some(output) = undrawn {
                        ui.not_drawn(output);
                    }
                });
                self.ui = Some(ui);
                shown
            },
            (None, Some((mut ui, output))) => {
                ui.not_drawn(output);
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
        let mut samples = self.machine().take_audio_samples();
        // Audio > Mono: both channels the mix of the two
        if self.ui.as_ref().is_some_and(|ui| !ui.settings.stereo) {
            amsrust::audio_mix::fold_to_mono(&mut samples, 2);
        }
        // Audio > Record WAV: what the page plays
        if let Some((recorded, ..)) = self.ui.as_mut().and_then(|ui| ui.wav.as_mut()) {
            recorded.extend_from_slice(&samples);
        }
        samples
    }
}


impl Emulator {
    /// The machine running: the script's, or the page's.
    fn machine(&mut self) -> &mut Motherboard {
        match self.csl.as_mut() {
            Some(runner) => runner.motherboard_mut(),
            None => &mut self.motherboard
        }
    }

    /// `motherboard` replaces the machine: the discs, the cartridge and the ROMs go in, the sound goes on.
    fn replace_machine(&mut self, mut motherboard: Motherboard) {
        motherboard.psg_mut().take_audio_output_from(self.motherboard.psg_mut());
        if let (Some(rate), None) = (self.audio_rate, motherboard.psg().audio_config()) {
            motherboard.set_audio_output(Some(AudioConfig::stereo(rate)));
        }
        for (unit, disc) in self.discs.iter().enumerate() {
            motherboard.fdc_mut().insert_image(fdc_drive(unit), disc.clone());
        }
        // the cards on the expansion port stay plugged, with what they hold (the Nova's NVRAM, the
        // Albireo's key, the Dandanator's flash)
        #[cfg(feature = "dandanator")]
        carry_card::<amsrust::dandanator::Dandanator>(&mut self.motherboard, &mut motherboard);
        #[cfg(feature = "nova")]
        carry_card::<amsrust::nova::Nova>(&mut self.motherboard, &mut motherboard);
        #[cfg(feature = "albireo")]
        carry_card::<amsrust::ch376::Albireo>(&mut self.motherboard, &mut motherboard);
        #[cfg(feature = "m4")]
        carry_card::<amsrust::m4::M4Board>(&mut self.motherboard, &mut motherboard);
        // the cartridge and the upper ROMs stay in (a CPC has no cartridge port: the cartridge waits)
        if motherboard.model().is_plus() {
            if let Some(cartridge) = &self.cartridge {
                motherboard.insert_cartridge(Some(cartridge.clone()));
            }
        }
        for (slot, rom) in &self.roms {
            // a slot a model refuses (the disc interface's) is not fatal for the machine
            let _ = motherboard.set_upper_rom(*slot, Some(rom));
        }
        // the deck keeps its tape, where it is
        motherboard.take_cassette_and_watches_from(&mut self.motherboard);
        self.motherboard = motherboard;
        self.monitor = self.config.monitor();
    }

    /// The machine of a script that ends goes on as the page's.
    fn adopt(&mut self, runner: &mut CslRunner) {
        self.motherboard = amsrust::frame::end_script(runner);
        self.monitor = self.config.monitor();
    }

    fn csl_frame(&mut self, step: ScriptStep) -> Result<(), JsError> {
        match step {
            ScriptStep::Frame => {
                self.frames += 1;
                // the script's frame, copied in the one already allocated
                if let Some(source) = self.csl.as_ref().and_then(CslRunner::last_frame) {
                    match &mut self.frame {
                        Some(frame) if frame.dimensions() == source.dimensions() => frame.copy_from_slice(source),
                        frame => *frame = Some(source.clone())
                    }
                }
            },
            ScriptStep::Running => {},
            ScriptStep::Done(result) => {
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

    /// The page's machine, with the host keyboard, to its next frame (`amsrust::frame::run_frame`:
    /// the debugger and the overlay as the window has them).
    fn free_frame(&mut self) {
        self.apply_keyboard();
        let frame = amsrust::frame::run_frame(
            &mut self.motherboard,
            &mut self.monitor,
            self.ui.as_mut().map(|ui| &mut ui.debugger).or(self.debugger.as_mut()),
            self.overlay.as_mut(),
            MAX_FRAME_T_STATES
        );
        if let Some(mut frame) = frame {
            if let Some(overlay) = &mut self.overlay {
                overlay.draw_and_clear(&mut frame);
            }
            self.frames += 1;
            // the previous frame is drawn in next
            if let Some(previous) = self.frame.replace(frame) {
                self.monitor.recycle(previous);
            }
        }
    }
}

impl Emulator {
    /// The host keyboard to the page's machine, once a frame.
    fn apply_keyboard(&mut self) {
        let mut input = HostInput {
            down: self.held.values().copied().collect(),
            typed: std::mem::take(&mut self.typed),
            placed: self.placed.values().copied().collect(),
            joysticks: amsrust::joystick::Joysticks::default(),
        };
        self.apply_gamepads(&mut input);
        // the debugger types (Auto-type, the palette): the host keyboard would release its key
        if self.ui.as_ref().is_some_and(|ui| ui.debugger.is_typing()) {
            return;
        }
        self.keyboard.apply(&mut self.motherboard, &input);
    }

    /// The browser's gamepads (`navigator.getGamepads()`) to `input`'s joysticks, once a frame.
    /// Gamepad 0 → joystick 0 (line 9), gamepad 1 → joystick 1 (line 6).
    fn apply_gamepads(&mut self, input: &mut HostInput) {
        use wasm_bindgen::JsCast;
        use web_sys::{Gamepad, GamepadButton};
        let Some(window) = web_sys::window() else { return };
        let navigator = window.navigator();
        let gamepads = match navigator.get_gamepads() {
            Ok(gamepads) => {
                self.gamepad_error_reported = false;
                gamepads
            }
            Err(error) => {
                if !self.gamepad_error_reported {
                    web_sys::console::warn_2(&"bemu: unable to read gamepads".into(), &error);
                    self.gamepad_error_reported = true;
                }
                return;
            }
        };
        for slot in 0..2u32 {
            let joy = match slot {
                0 => &mut input.joysticks.joy0,
                _ => &mut input.joysticks.joy1,
            };
            let value = gamepads.get(slot);
            if value.is_null() || value.is_undefined() {
                continue;
            }
            let Ok(gamepad) = value.dyn_into::<Gamepad>() else {
                web_sys::console::warn_1(&"bemu: invalid gamepad entry".into());
                continue;
            };
            if !gamepad.connected() {
                continue;
            }
            // left stick: axes 0 = X, 1 = Y (digital: one axis at a time)
            let axes = gamepad.axes();
            let axis = |index| {
                if index < axes.length() { axes.get(index).as_f64() } else { Some(0.0) }
            };
            if let (Some(raw_x), Some(raw_y)) = (axis(0), axis(1)) {
                let dead = 0.5;
                let dx = if raw_x.abs() > dead { raw_x.signum() as i8 } else { 0 };
                let dy = if raw_y.abs() > dead { raw_y.signum() as i8 } else { 0 };
                if dx != 0 && dy != 0 {
                    if raw_x.abs() >= raw_y.abs() {
                        joy.x = dx;
                        joy.y = 0;
                    } else {
                        joy.x = 0;
                        joy.y = dy;
                    }
                } else {
                    joy.x = dx;
                    joy.y = dy;
                }
            } else {
                web_sys::console::warn_1(&"bemu: invalid gamepad axes".into());
            }
            // buttons 0/1/2 → fire1/fire2/fire3 (South/East/West on typical gamepads)
            let buttons = gamepad.buttons();
            for index in 0..buttons.length().min(3) {
                match buttons.get(index).dyn_into::<GamepadButton>() {
                    Ok(button) => joy.fire[index as usize] = button.pressed(),
                    Err(_) => web_sys::console::warn_1(&"bemu: invalid gamepad button".into()),
                }
            }
        }
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

/// The debugger [`Emulator::api`] asks: the menus' when they are shown, else the page's own.
fn api_debugger<'a>(
    ui: &'a mut Option<WebUi>,
    own: &'a mut Option<amsrust::debug_server::DebugServer>,
) -> &'a mut amsrust::debug_server::DebugServer {
    match (ui, own) {
        (Some(ui), _) => &mut ui.debugger,
        (None, Some(debugger)) => debugger,
        (None, None) => unreachable!("a page has the menus' debugger or its own"),
    }
}
