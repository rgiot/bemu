//! The host's side of the commands: its settings (changed by the commands, applied by the host
//! at its next frame) and its requests (what only a window or a page can do).

use std::path::PathBuf;
use std::sync::Arc;

use crate::display::DisplayMode;
use crate::input::{HostLayout, KeyboardMapping};
use crate::joystick::JoystickSource;

/// The debug server's port (AMSpiriT's).
pub const DEFAULT_DEBUG_PORT: u16 = crate::debug_server::DEFAULT_PORT;

/// The emulation speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    /// This share of the real speed (1.0: real time, 50 frames a second).
    Factor(f32),
    /// As fast as the host can, silent.
    Max,
}

impl Default for Speed {
    fn default() -> Self {
        Speed::Factor(1.0)
    }
}

impl Speed {
    pub fn is_max(self) -> bool {
        self == Speed::Max
    }

    /// Frames a second to aim at (`None`: as many as possible).
    pub fn frames_per_second(self) -> Option<f32> {
        match self {
            Speed::Factor(factor) => Some(50.0 * factor),
            Speed::Max => None,
        }
    }
}

/// The files the media came from: what Save, Update and the recent lists use.
#[derive(Clone, Debug, Default)]
pub struct MediaPaths {
    /// The disc file of drive A and B.
    pub discs: [Option<String>; 2],
    /// The discs used lately, the last first, per drive.
    pub recent_discs: [Vec<String>; 2],
    /// The disc images of the archive each drive's disc came from (their paths in it), when it
    /// came from one: the other versions or sides one may insert instead.
    pub archive_discs: [Vec<String>; 2],
    /// The snapshot last loaded or saved.
    pub snapshot: Option<PathBuf>,
    /// The tape's file.
    pub tape: Option<String>,
    /// Quick snapshots taken (`bemu_NNNN.sna`).
    pub quick_snapshots: usize,
}

/// How many recent discs are kept.
pub const RECENT_DISCS: usize = 10;

impl MediaPaths {
    /// Note `path` as drive `drive`'s disc, at the top of its recent list.
    pub fn disc_inserted(&mut self, drive: usize, path: Option<String>) {
        if let Some(path) = &path {
            let recent = &mut self.recent_discs[drive];
            recent.retain(|p| p != path);
            recent.insert(0, path.clone());
            recent.truncate(RECENT_DISCS);
        }
        self.archive_discs[drive] = path
            .as_deref()
            .map(crate::disc_image::archive_disc_entries_of_file)
            .unwrap_or_default();
        self.discs[drive] = path;
    }

    /// Drive `drive`'s disc removed.
    pub fn disc_removed(&mut self, drive: usize) {
        self.discs[drive] = None;
        self.archive_discs[drive].clear();
    }

    /// The archive entry drive `drive`'s disc is (the first disc image when none is named).
    pub fn archive_disc(&self, drive: usize) -> Option<&str> {
        let spec = self.discs[drive].as_deref()?;
        let entries = &self.archive_discs[drive];
        match crate::disc_image::split_disc_spec(spec).1 {
            Some(entry) => entries
                .iter()
                .find(|e| e.as_str() == entry)
                .or_else(|| entries.iter().find(|e| e.eq_ignore_ascii_case(entry)))
                .map(String::as_str),
            None => entries.first().map(String::as_str),
        }
    }
}

/// How long a typed key is held, the pause after it and the pause after a whole text (after a
/// RETURN), in emulated microseconds (CSL's `key_delay`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypingDelays {
    pub press_us: u64,
    pub after_key_us: u64,
    pub after_text_us: u64,
}

impl Default for TypingDelays {
    /// The CSL standard's.
    fn default() -> Self {
        Self {
            press_us: 70_000,
            after_key_us: 70_000,
            after_text_us: 400_000,
        }
    }
}

impl std::str::FromStr for TypingDelays {
    type Err = String;

    /// "PRESS [AFTER_KEY [AFTER_TEXT]]" in µs, as `key_delay`: the missing ones are the press.
    fn from_str(text: &str) -> Result<Self, String> {
        let numbers = text
            .split([' ', ','])
            .filter(|word| !word.is_empty())
            .map(|word| {
                word.parse::<u64>()
                    .map_err(|_| format!("{word}: a number of microseconds"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let press = *numbers
            .first()
            .ok_or("PRESS [AFTER_KEY [AFTER_TEXT]], in microseconds")?;
        Ok(Self {
            press_us: press,
            after_key_us: numbers.get(1).copied().unwrap_or(press),
            after_text_us: numbers.get(2).copied().unwrap_or(press),
        })
    }
}

/// What the host shows and plays, changed by the commands and applied by the host.
#[derive(Clone, Debug)]
pub struct HostSettings {
    /// Paused, when there is no debugger to say it.
    pub paused: bool,
    pub speed: Speed,
    pub display: DisplayMode,
    /// The CRT glass' curvature, 0 (flat) to 4.
    pub curvature: f32,
    /// The grey of a black ink, 0-255 (the monitor's brightness knob).
    pub black_level: u8,
    /// The picture brighter (up to 100) or darker (down to -100), 0 as it is: a gamma curve
    /// of a moderate range, see `Monitor::with_brightness`.
    pub brightness: i8,
    /// The monitor's H-hold and V-hold knobs, -100 to 100 (0: steady, see `Monitor::with_holds`).
    pub hhold: i8,
    pub vhold: i8,
    /// The CRT drawn by the GPU.
    pub crt_gpu: bool,
    /// The picture as large as the window allows (Video > Maximal size): by default it is
    /// zoomed by a whole factor, every CPC pixel the same size.
    pub free_zoom: bool,
    /// A whole zoom, the CRT drawn directly at that resolution (Video > Interface zoom > Maximal
    /// integer size); `free_zoom` comes first.
    pub native_zoom: bool,
    pub fullscreen: bool,
    /// A page too slow for real time makes the picture of some frames only (Video > Skip frames
    /// when too slow): off, every picture is made (a video recording wants that).
    pub frame_skip: bool,
    /// The CRTC / Gate Array writes drawn on the picture.
    pub overlay: bool,
    pub keyboard: KeyboardMapping,
    /// The host keyboard's layout, for the positional mapping (`None`: automatic).
    pub host_layout: Option<HostLayout>,
    /// Where joystick 1's state comes from (none / host / simulated window).
    pub joystick_1: JoystickSource,
    /// Where joystick 2's state comes from (none / host / simulated window).
    pub joystick_2: JoystickSource,
    pub stereo: bool,
    pub muted: bool,
    /// 0-100 %.
    pub volume: u8,
    /// The menu bar shown.
    pub menu_visible: bool,
    /// The interface's zoom (menus, windows), 0.5-3: the emulator's picture keeps its size.
    pub ui_zoom: f32,
    /// The modified discs written back to their files.
    pub save_discs: bool,
    /// The files reloaded when they change.
    pub watch_files: bool,
    /// The snapshot saved when the emulator exits.
    pub save_snapshot_on_exit: Option<PathBuf>,
    /// The debug server's port.
    pub debug_port: u16,
    pub media: MediaPaths,
    /// How fast the keys are typed (Tools > Auto-type > Typing speed, CSL's `key_delay`):
    /// `None`, the host's own pace.
    pub typing: Option<TypingDelays>,
    /// The snapshot version written (2 or 3: File > Snapshot > Version, CSL's
    /// `snapshot_version`).
    pub snapshot_version: u8,
    /// What the cards are plugged with, to plug them again with a change.
    pub devices: DeviceFiles,
}

/// The files and jumpers of the cards: a card is plugged again with one of them changed.
#[derive(Clone, Debug)]
pub struct DeviceFiles {
    /// The Albireo's USB key: a folder or a FAT image.
    pub albireo: Option<PathBuf>,
    /// The M4's SD card folder, its ROM (`None`: the bundled one), slot and WiFi.
    pub m4_sd: Option<PathBuf>,
    pub m4_rom: Option<String>,
    pub m4_slot: u8,
    pub m4_wifi: bool,
    /// The Nova's NVRAM file.
    pub nova: Option<PathBuf>,
    /// Where the DMP2000's printout goes (a prefix), and its EPROM.
    pub dmp2000_prefix: Option<String>,
    pub dmp2000_rom: Option<Arc<[u8]>>,
    /// Willy's jumpers and the S2P's SoundFont.
    pub opl3_base: u16,
    pub s2p_base: u16,
    pub s2p_soundfont: Option<(String, Arc<[u8]>)>,
    /// The Booster's UART link and files.
    pub booster_uart: Option<String>,
    pub booster_eeprom: Option<PathBuf>,
    pub booster_flash: Option<PathBuf>,
    pub booster_adc: Option<PathBuf>,
    /// The Dandanator's romset, and whether the CPC writes it back.
    pub dandanator: Option<(String, Arc<[u8]>)>,
    pub dandanator_writable: bool,
}

impl Default for DeviceFiles {
    fn default() -> Self {
        Self {
            albireo: None,
            m4_sd: None,
            m4_rom: None,
            m4_slot: 6,
            m4_wifi: true,
            nova: None,
            dmp2000_prefix: None,
            dmp2000_rom: None,
            opl3_base: 0xfebc,
            s2p_base: 0xfeac,
            s2p_soundfont: None,
            booster_uart: None,
            booster_eeprom: None,
            booster_flash: None,
            booster_adc: None,
            dandanator: None,
            dandanator_writable: false,
        }
    }
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            paused: false,
            speed: Speed::default(),
            display: DisplayMode::Crt(crate::display::CrtProfile::Ctm644),
            curvature: 1.0,
            black_level: 8,
            brightness: 0,
            hhold: 0,
            vhold: 0,
            ui_zoom: 1.0,
            crt_gpu: true,
            free_zoom: false,
            native_zoom: false,
            fullscreen: false,
            frame_skip: false,
            overlay: false,
            keyboard: KeyboardMapping::default(),
            host_layout: None,
            joystick_1: JoystickSource::default(),
            joystick_2: JoystickSource::default(),
            stereo: true,
            muted: false,
            volume: 100,
            menu_visible: true,
            save_discs: false,
            watch_files: false,
            save_snapshot_on_exit: None,
            debug_port: DEFAULT_DEBUG_PORT,
            media: MediaPaths::default(),
            typing: None,
            snapshot_version: 3,
            devices: DeviceFiles::default(),
        }
    }
}

/// A window of the user interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum View {
    Z80,
    Stack,
    Crtc,
    GateArray,
    Psg,
    Fdc,
    MemoryMap,
    Memory,
    Disassembly,
    History,
    Symbols,
    Beam,
    Breakpoints,
    Counters,
    Ppi,
    Tape,
    Printer,
    CslLog,
    Watch,
    Asic,
    Screen,
    Palette,
    Preferences,
    Shortcuts,
    About,
    Joystick,
    Keyboard,
}

/// What a [`View`] is: everything the menus, the interface and the hosts need to know of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewInfo {
    /// The command that opens it (`doc/menu-ui.csv`).
    pub command: &'static str,
    /// Its title ("" for the palette, which is no window).
    pub title: &'static str,
    /// One of the debugger's windows (on a desktop, a window of the system): not the
    /// preferences, the shortcuts, the about box or the palette.
    pub debugger: bool,
    /// Several can be open, each with its own settings (Memory, Disassembly).
    pub copies: bool,
    /// Sized by the user, from this size (in points), instead of to its content.
    pub resizable: Option<[f32; 2]>,
}

impl ViewInfo {
    const fn debugger(command: &'static str, title: &'static str) -> Self {
        Self {
            command,
            title,
            debugger: true,
            copies: false,
            resizable: None,
        }
    }
    const fn copies(self) -> Self {
        Self {
            copies: true,
            ..self
        }
    }
    const fn resizable(self, width: f32, height: f32) -> Self {
        Self {
            resizable: Some([width, height]),
            ..self
        }
    }
    const fn not_debugger(self) -> Self {
        Self {
            debugger: false,
            ..self
        }
    }
}

impl View {
    /// Every view, in the order of the Window menu.
    pub const ALL: [View; 27] = [
        View::Z80,
        View::Stack,
        View::Crtc,
        View::GateArray,
        View::Psg,
        View::Fdc,
        View::MemoryMap,
        View::Memory,
        View::Disassembly,
        View::History,
        View::Symbols,
        View::Beam,
        View::Breakpoints,
        View::Counters,
        View::Ppi,
        View::Tape,
        View::Printer,
        View::CslLog,
        View::Watch,
        View::Asic,
        View::Screen,
        View::Palette,
        View::Preferences,
        View::Shortcuts,
        View::About,
        View::Joystick,
        View::Keyboard,
    ];

    pub const fn info(self) -> ViewInfo {
        use ViewInfo as I;
        match self {
            View::Z80 => I::debugger("view-z80", "Z80"),
            View::Stack => I::debugger("view-stack", "Stack"),
            View::Crtc => I::debugger("view-crtc", "CRTC"),
            View::GateArray => I::debugger("view-ga", "Gate Array"),
            View::Psg => I::debugger("view-psg", "PSG"),
            View::Fdc => I::debugger("view-fdc", "FDC"),
            View::MemoryMap => I::debugger("view-memmap", "Memory map"),
            View::Memory => I::debugger("view-memory", "Memory").copies(),
            View::Disassembly => I::debugger("view-disassembly", "Disassembly").copies(),
            View::History => I::debugger("view-history", "Instruction history"),
            View::Symbols => I::debugger("view-symbols", "Symbols"),
            View::Beam => I::debugger("view-beam", "Beam"),
            View::Breakpoints => I::debugger("view-breakpoints", "Breakpoints"),
            View::Counters => I::debugger("view-counters", "Counters"),
            View::Ppi => I::debugger("view-ppi", "PPI"),
            View::Tape => I::debugger("view-tape", "Tape"),
            View::Printer => I::debugger("view-printer", "Printer").resizable(560.0, 520.0),
            View::CslLog => I::debugger("view-csl-log", "CSL log"),
            View::Watch => I::debugger("view-watch", "Watch"),
            View::Asic => I::debugger("view-asic", "ASIC (CPC+)"),
            View::Screen => I::debugger("view-screen", "Screen memory"),
            View::Palette => I::debugger("palette-open", "").not_debugger(),
            View::Preferences => I::debugger("preferences-open", "Preferences").not_debugger(),
            View::Shortcuts => I::debugger("help-shortcuts", "Keyboard shortcuts").not_debugger(),
            View::About => I::debugger("help-about", "About bemu").not_debugger(),
            View::Joystick => I::debugger("joystick-simulated-window", "Joystick"),
            View::Keyboard => I::debugger("keyboard-view", "Keyboard").resizable(820.0, 300.0),
        }
    }

    /// The view the command `id` opens.
    pub fn opened_by(id: &str) -> Option<View> {
        View::ALL.into_iter().find(|view| view.info().command == id)
    }

    /// The view fits the machine: a CPC+ has the ASIC, a CPC the Gate Array.
    pub fn fits(self, model: crate::motherboard::CpcModel) -> bool {
        match self {
            View::GateArray => !model.is_plus(),
            View::Asic => model.is_plus(),
            _ => true,
        }
    }
}

/// What the host records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Recording {
    Wav,
    Ym,
    Video,
    ScreenshotSeries,
}

/// What only the host can do.
#[derive(Clone, Debug, PartialEq)]
pub enum HostRequest {
    /// Close the emulator.
    Quit,
    /// Open a window of the user interface.
    Open(View),
    /// Open another window of a view that can have several (Memory, Disassembly).
    OpenAnother(View),
    /// Show a text (the embedded ROMs, a disc's catalogue).
    ShowText {
        title: String,
        text: String,
    },
    /// Open a page in the browser.
    OpenUrl(String),
    /// Save the last frame as a PNG (`None`: in the host's own way, a download).
    Screenshot(Option<PathBuf>),
    /// Copy the last frame to the clipboard.
    CopyScreenshot,
    /// Type the clipboard on the CPC keyboard.
    PasteClipboard,
    /// Start recording to a file (or the host's own place).
    StartRecording(Recording, Option<PathBuf>),
    StopRecording(Recording),
    /// Play a CSL script (its text, the folder its files are in).
    RunCsl {
        text: String,
        folder: Option<PathBuf>,
    },
    StopCsl,
    /// Serve the debug API (on the settings' port), or stop serving it.
    DebugServer(bool),
    /// Write a file (a disc, a snapshot, a BASIC listing): on a disk, or downloaded.
    SaveFile {
        name: String,
        data: Vec<u8>,
    },
    /// Write the printer's output now.
    SavePrintout,
}
