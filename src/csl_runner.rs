//! Plays CSL scripts (*CPC Script Language*, see `CSL-STANDARD-EN.pdf`) on a
//! [`Motherboard`]: the way the Shaker test-suite drives an emulator.
//!
//! A script picks the CRTC, resets the machine, inserts a disc, types on the
//! keyboard and waits; the programs it starts request screenshots with SSM
//! codes (`ED xx ED yy`, see `SSM-STANDARD-EN.pdf`). The runner keeps the
//! emulated time itself - one CSL time unit is one microsecond, that is four
//! Z80 T-states - and feeds the keyboard matrix from a schedule of press and
//! release events, so keystrokes go through the firmware's own scan.
//!
//! Only what a test-suite needs is implemented; the instructions that don't
//! apply (tape, snapshots, ROM configuration...) are recorded in
//! [`CslReport::ignored`] rather than silently dropped.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use cpclib_csl::{
    parse_csl_with_rich_errors, CrtcModel as CslCrtc, CslInstruction, Drive, KeyElement, SpecialKey,
};

use crate::commands::{
    Arg, Command, CommandError, CommandTarget, HostRequest, HostSettings, Outcome, TypingDelays,
};
use crate::crtc::CrtcType;
use crate::disc_image::split_disc_spec;
use crate::fdc::FdcDrive;
use crate::keyboard::{CpcKey, KeyboardLayout};
use crate::monitor::Monitor;
use crate::motherboard::{Motherboard, MotherboardBuilder};
use crate::shakerland::Manifest;

/// Z80 T-states per CSL time unit (4MHz clock, microsecond unit).
const T_STATES_PER_UNIT: usize = 4;

/// Ticks (1/16 µs) after which a screenshot gives up waiting for a whole
/// frame (two are needed): several frames' worth, for a machine that never
/// synchronises.
const MAX_FRAME_TICKS: usize = 4_000_000;

/// SSM code that releases `wait_ssm0000` (SSM standard).
const SSM_WAIT_RELEASE: u16 = 0x0000;
/// SSM code asking for a screenshot named by `screenshot_name`.
const SSM_NAMED_SCREENSHOT: u16 = 0xfffe;

/// Default `wait_ssm0000`/`wait_ssm`/`wait_driveonoff` timeout, in emulated
/// microseconds: an unattended multi-hour run must not hang forever on a
/// program that, because of some remaining bug, never raises the code or
/// never cycles the drive motor. [`CslRunner::set_wait_timeout_us`] can
/// shorten it, e.g. for a test that deliberately waits for a code that
/// never comes.
const DEFAULT_WAIT_TIMEOUT_US: u64 = 90_000_000;

/// A screenshot the script produced.
#[derive(Debug, Clone)]
pub struct Screenshot {
    /// File written.
    pub path: PathBuf,
    /// The SSM code that requested it (`None` for the `screenshot` instruction).
    pub ssm_code: Option<u16>,
    /// The CRTC of the machine that took it.
    pub crtc: CrtcType,
    /// Emulated microseconds since the runner started.
    pub at_us: u64,
}

/// What happened during a script.
#[derive(Debug, Clone, Default)]
pub struct CslReport {
    pub screenshots: Vec<Screenshot>,
    /// SSM codes seen (whether or not they produced a picture).
    pub ssm_codes: Vec<u16>,
    /// Instructions the runner does not implement.
    pub ignored: Vec<String>,
    /// Emulated microseconds played.
    pub elapsed_us: u64,
    /// What the script wrote (`snapshot`) where the files are not on a disk: name and bytes.
    pub files: Vec<(String, Vec<u8>)>,
}

/// A change of state of some keys, due at an emulated time.
struct KeyEvent {
    due_t_states: usize,
    keys: Vec<CpcKey>,
    pressed: bool,
}

/// Where a script's files come from: the files around it on disc ([`FsFiles`]), or files
/// given as bytes ([`MemFiles`], what a web page has).
pub trait CslFiles {
    /// The script `name` names (`csl_load`, or the one started), from `base_dir`: its path,
    /// whose directory the files it names are looked up from, and its text.
    fn script(&mut self, base_dir: &Path, name: &str) -> Result<(PathBuf, String), String>;
    /// The disc image `disk_insert` names, from `base_dir`. A substitute (a script naming a disc
    /// it does not ship with) is noted in `notes`.
    fn disc(
        &mut self,
        base_dir: &Path,
        name: &str,
        notes: &mut Vec<String>,
    ) -> Result<Vec<u8>, String>;
    /// The snapshot `snapshot_load` names, in `dir`.
    fn snapshot(&mut self, dir: &Path, name: &str) -> Result<Vec<u8>, String>;
    /// The tape `tape_insert` names, in `dir`.
    fn tape(&mut self, dir: &Path, name: &str) -> Result<Vec<u8>, String>;
    /// The text `key_from_file` names, in `dir`.
    fn text(&mut self, dir: &Path, name: &str) -> Result<Vec<u8>, String>;
    /// Whether the files are on a disk: what the script writes (`snapshot`) goes there.
    fn on_disk(&self) -> bool {
        true
    }
}

/// Where the screenshots go.
pub trait ScreenshotSink {
    /// Keep `frame` as `file` (`<name>.png`) of the screenshot directory `dir`; returns its path.
    fn save(&mut self, dir: &Path, file: &str, frame: &image::RgbImage) -> Result<PathBuf, String>;
}

/// The files of the file system, as the CSL players look them up: case-insensitively, in the
/// script's directory and its parents.
#[derive(Default)]
pub struct FsFiles;

impl CslFiles for FsFiles {
    fn script(&mut self, base_dir: &Path, name: &str) -> Result<(PathBuf, String), String> {
        let path = resolve(base_dir, name);
        let found = [
            path.clone(),
            path.with_extension("csl"),
            path.with_extension("CSL"),
        ]
        .into_iter()
        .find_map(|candidate| find_case_insensitive(&candidate))
        .ok_or_else(|| format!("no script {}", path.display()))?;
        let text = read_text(&found)?;
        Ok((found, text))
    }

    fn disc(
        &mut self,
        base_dir: &Path,
        name: &str,
        notes: &mut Vec<String>,
    ) -> Result<Vec<u8>, String> {
        // The Shaker scripts sit in one directory per module while the disc
        // is shared by all of them: look in the parent directories too.
        let path = base_dir
            .ancestors()
            .take(3)
            .find_map(|dir| find_case_insensitive(&resolve(dir, name)))
            .or_else(|| only_disc_image_nearby(base_dir, name, notes))
            .unwrap_or_else(|| resolve(base_dir, name));
        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn snapshot(&mut self, dir: &Path, name: &str) -> Result<Vec<u8>, String> {
        let path = resolve(dir, name);
        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn tape(&mut self, dir: &Path, name: &str) -> Result<Vec<u8>, String> {
        let path = resolve(dir, name);
        let path = find_case_insensitive(&path).unwrap_or(path);
        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn text(&mut self, dir: &Path, name: &str) -> Result<Vec<u8>, String> {
        self.tape(dir, name)
    }
}

/// Files given as bytes, by name: the directories a script names them with are ignored and the
/// case does not matter.
#[derive(Default, Clone)]
pub struct MemFiles {
    files: HashMap<String, Vec<u8>>,
}

impl MemFiles {
    pub fn insert(&mut self, name: &str, bytes: Vec<u8>) {
        self.files.insert(file_key(name), bytes);
    }

    fn get(&self, name: &str) -> Option<&Vec<u8>> {
        self.files.get(&file_key(name))
    }
}

/// The file name of `name` (a CSL path, maybe with `\`), lower case.
fn file_key(name: &str) -> String {
    name.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

impl CslFiles for MemFiles {
    fn on_disk(&self) -> bool {
        false
    }

    fn script(&mut self, _base_dir: &Path, name: &str) -> Result<(PathBuf, String), String> {
        let bytes = [name.to_owned(), format!("{name}.csl")]
            .iter()
            .find_map(|candidate| self.get(candidate))
            .ok_or_else(|| format!("no script {name}"))?;
        let text = String::from_utf8_lossy(bytes);
        Ok((
            PathBuf::from(file_key(name)),
            text.trim_start_matches('\u{feff}').to_owned(),
        ))
    }

    fn disc(
        &mut self,
        _base_dir: &Path,
        name: &str,
        notes: &mut Vec<String>,
    ) -> Result<Vec<u8>, String> {
        if let Some(bytes) = self.get(name) {
            return Ok(bytes.clone());
        }
        // a disc inside an archive (zip, 7z, tar.gz) given (a production's download), by its file name
        if let Some(bytes) = self
            .files
            .values()
            .find_map(|bytes| crate::disc_image::zip_entry_by_file_name(bytes, name))
        {
            return Ok(bytes);
        }
        // as on disc: the only disc image (or archive) given stands in for one the script names wrongly
        let mut discs = self.files.iter().filter(|(file, _)| {
            [".dsk", ".hfe", ".zip", ".7z", ".gz", ".tgz"]
                .iter()
                .any(|ext| file.ends_with(ext))
        });
        match (discs.next(), discs.next()) {
            (Some((file, bytes)), None) => {
                notes.push(format!(
                    "disk_insert: {name:?} not found, using {file} instead"
                ));
                Ok(bytes.clone())
            }
            _ => Err(format!("{name}: no such disc image")),
        }
    }

    fn snapshot(&mut self, _dir: &Path, name: &str) -> Result<Vec<u8>, String> {
        self.get(name)
            .cloned()
            .ok_or_else(|| format!("{name}: no such snapshot"))
    }

    fn tape(&mut self, _dir: &Path, name: &str) -> Result<Vec<u8>, String> {
        self.get(name)
            .cloned()
            .ok_or_else(|| format!("{name}: no such tape"))
    }

    fn text(&mut self, _dir: &Path, name: &str) -> Result<Vec<u8>, String> {
        self.get(name)
            .cloned()
            .ok_or_else(|| format!("{name}: no such file"))
    }
}

/// Screenshots saved as PNG files.
#[derive(Default)]
pub struct PngFiles;

impl ScreenshotSink for PngFiles {
    fn save(&mut self, dir: &Path, file: &str, frame: &image::RgbImage) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(file);
        frame.save(&path).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

/// What [`CslRunner::advance`] stopped on.
#[derive(Debug)]
pub enum Progress {
    /// The time given is spent; the script goes on.
    Running,
    /// The monitor completed a frame ([`CslRunner::last_frame`]); the script goes on.
    Frame,
    /// The script is over.
    Done(CslReport),
}

/// The instruction being played, while it takes emulated time.
enum Wait {
    /// `wait`: until this T-state.
    Until(usize),
    /// `key_output`: until this T-state, the keys typed and the third `key_delay` waited.
    Typing(usize),
    /// `wait_vsyncoffon`, first until the VSYNC is off...
    VsyncOff,
    /// ...then until it is on.
    VsyncOn,
    /// `wait_ssm0000` / `wait_ssm`: until the code is raised, or the deadline.
    Ssm {
        code: u16,
        deadline: usize,
        instruction: &'static str,
        raised: bool,
    },
    /// `wait_driveonoff`: until the motor has gone on then off `count` times, or the deadline.
    DriveOnOff {
        count: u32,
        done: u32,
        on: bool,
        deadline: usize,
    },
}

pub struct CslRunner {
    motherboard: Motherboard,
    /// The machine the script's settings apply to (see [`Self::set_machine`]).
    base: MotherboardBuilder,
    /// Chosen by `crtc_select`, applied at the next `reset`.
    crtc: CrtcType,
    /// Chosen by `gate_array`, applied at the next `reset`.
    gate_array: crate::ga::GateArrayType,
    /// Chosen by `memory_exp`, applied at the next `reset`.
    memory_exp: cpclib_csl::MemoryExpansion,
    /// Chosen by `cpc_model` and `rom_config C` (a CPC+ cartridge), applied at the next `reset`.
    model: Option<crate::motherboard::CpcModel>,
    /// The CPC+ cartridge `rom_config C` names (its file and bytes), inserted at the next `reset`.
    cartridge: Option<(String, Vec<u8>)>,
    /// The upper ROMs `rom_config` names (slot, file), fitted at the next `reset`.
    roms: Vec<(u8, String)>,
    /// Where `rom_config` looks (`rom_dir`), else `base_dir`.
    rom_dir: Option<PathBuf>,
    /// The settings of the commands the script runs (typing delays, snapshot version).
    settings: HostSettings,
    /// The kind of each drive (3" by default), kept across resets.
    drive_kinds: [crate::fdc::DriveKind; 2],
    files: Box<dyn CslFiles>,
    screenshots: Box<dyn ScreenshotSink>,
    /// The instructions left to play, and the one playing.
    script: VecDeque<CslInstruction>,
    waiting: Option<Wait>,
    key_events: VecDeque<KeyEvent>,
    /// When the last scheduled key event is over, in T-states of this machine.
    keys_busy_until: usize,
    screenshot_name: Option<String>,
    screenshot_dir: PathBuf,
    /// The name `snapshot_name` gives the snapshots (`bemu` otherwise).
    snapshot_name: Option<String>,
    /// Base directory for the discs and the scripts loaded by name.
    base_dir: PathBuf,
    /// Where `snapshot_load` looks (`snapshot_dir`), else `base_dir`.
    snapshot_dir: Option<PathBuf>,
    /// Where `tape_insert` looks (`tape_dir`), else `base_dir`.
    tape_dir: Option<PathBuf>,
    /// Count of screenshots per file stem, to keep repeated codes apart.
    taken: HashMap<String, usize>,
    /// The beam is always followed, so that a screenshot is the last whole frame the
    /// monitor displayed when it was requested (what AMSpiriT saves).
    monitor: Monitor,
    last_frame: Option<image::RgbImage>,
    /// The monitor completed a frame since [`Self::advance`] last said so.
    new_frame: bool,
    /// Machines are rebuilt on reset; this accumulates their time.
    elapsed_before_reset_us: u64,
    /// Print each screenshot and reset as it happens.
    verbose: bool,
    /// The Shaker module ('A'-'E') currently running, for naming screenshots
    /// the way <https://shaker.logonsystem.eu/tests> does (see [`Manifest`]).
    /// `None` outside of a `set_module` call: screenshots then keep the
    /// generic `<crtc>_<code>` naming.
    module: Option<char>,
    manifest: Option<&'static Manifest>,
    /// See [`Self::set_wait_timeout_us`].
    wait_timeout_us: u64,
    report: CslReport,
    /// The SSM codes raised while the last `key_output` was typed, for the `wait_ssm` after it.
    ssm_while_typing: Vec<u16>,
}

impl CslRunner {
    /// `output_dir` receives the screenshots; the files come from the file system.
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self::with_io(Box::new(FsFiles), Box::new(PngFiles), output_dir)
    }

    /// The files come from `files`, the screenshots (of `output_dir`) go to `screenshots`.
    pub fn with_io(
        files: Box<dyn CslFiles>,
        screenshots: Box<dyn ScreenshotSink>,
        output_dir: impl Into<PathBuf>,
    ) -> Self {
        let base = MotherboardBuilder::default();
        let crtc = CrtcType::default();
        CslRunner {
            motherboard: Self::machine(
                base,
                crtc,
                crate::ga::GateArrayType::default(),
                cpclib_csl::MemoryExpansion::Kb128,
                None,
            ),
            base,
            gate_array: crate::ga::GateArrayType::default(),
            memory_exp: cpclib_csl::MemoryExpansion::Kb128,
            model: None,
            cartridge: None,
            roms: Vec::new(),
            rom_dir: None,
            settings: HostSettings {
                typing: Some(TypingDelays::default()),
                ..HostSettings::default()
            },
            crtc,
            drive_kinds: Default::default(),
            files,
            screenshots,
            script: VecDeque::new(),
            waiting: None,
            key_events: VecDeque::new(),
            keys_busy_until: 0,
            screenshot_name: None,
            screenshot_dir: output_dir.into(),
            snapshot_name: None,
            base_dir: PathBuf::from("."),
            snapshot_dir: None,
            tape_dir: None,
            taken: HashMap::new(),
            monitor: Monitor::default(),
            last_frame: None,
            new_frame: false,
            elapsed_before_reset_us: 0,
            verbose: false,
            module: None,
            manifest: None,
            wait_timeout_us: DEFAULT_WAIT_TIMEOUT_US,
            report: CslReport::default(),
            ssm_while_typing: Vec::new(),
        }
    }

    /// The machine (model, CRTC, Gate Array, memory, brand...) the script starts with and
    /// `reset` rebuilds; the script's `crtc_select`, `gate_array` and `memory_exp` change it at
    /// its next `reset`. The machine is rebuilt now.
    pub fn set_machine(&mut self, machine: MotherboardBuilder) {
        self.base = machine;
        self.crtc = machine.crtc();
        self.gate_array = machine.gate_array_type();
        self.memory_exp = cpclib_csl::MemoryExpansion::Kb128;
        let discs = [0, 1].map(|unit| self.motherboard.fdc().image(drive(unit)).cloned());
        self.motherboard = Self::machine(
            self.base,
            self.crtc,
            self.gate_array,
            self.memory_exp,
            self.model,
        );
        for (unit, disc) in discs.into_iter().enumerate() {
            self.motherboard
                .fdc_mut()
                .set_drive_kind(drive(unit), self.drive_kinds[unit]);
            self.motherboard.fdc_mut().insert_image(drive(unit), disc);
        }
    }

    /// Make drive `unit` (0 = A, 1 = B) a 3" or a 3.5" one, now and after each `reset`.
    pub fn set_drive_kind(&mut self, unit: usize, kind: crate::fdc::DriveKind) {
        self.drive_kinds[unit] = kind;
        self.motherboard.fdc_mut().set_drive_kind(drive(unit), kind);
    }

    /// Print progress on stderr while running.
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// How long `wait_ssm0000`, `wait_ssm` and `wait_driveonoff` wait
    /// (emulated microseconds) before giving up and letting the script
    /// carry on, logging an ignored note instead of blocking forever.
    /// Defaults to [`DEFAULT_WAIT_TIMEOUT_US`]; a test that deliberately
    /// waits for a code that never comes should shorten it first.
    pub fn set_wait_timeout_us(&mut self, timeout_us: u64) {
        self.wait_timeout_us = timeout_us;
    }

    /// Name screenshots the way <https://shaker.logonsystem.eu/tests> does
    /// for this Shaker module ('A'-'E'): looks up each SSM code in the
    /// bundled [`Manifest`] for `<test id>_CRTC<n>_<subtest>.png` instead of
    /// the generic `<module>_CRTC<n>_<code>.png`.
    pub fn set_module(&mut self, module: char) {
        static MANIFEST: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        self.module = Some(module);
        self.manifest = Some(MANIFEST.get_or_init(Manifest::bundled));
    }

    /// `memory`: `Kb128` keeps the memory of `base`; `model`, when given, replaces its model.
    fn machine(
        base: MotherboardBuilder,
        crtc: CrtcType,
        gate_array: crate::ga::GateArrayType,
        memory: cpclib_csl::MemoryExpansion,
        model: Option<crate::motherboard::CpcModel>,
    ) -> Motherboard {
        use cpclib_csl::MemoryExpansion::*;
        let mut builder = base;
        builder
            .set_crtc(crtc)
            .set_gate_array_type(gate_array)
            .enable_interruptions();
        if let Some(model) = model {
            builder.set_model(model);
        }
        match memory {
            Kb128 => {}
            Kb256Standard | Kb256Silicon => {
                builder.set_extra_ram_pages(3);
            }
            Kb512DkTronics => {
                builder.set_extra_ram_pages(7);
            }
            Mb4 => {
                builder.plug_large_ram_expansion(64);
            }
        }
        builder.build()
    }

    /// The machine being driven.
    pub fn motherboard(&self) -> &Motherboard {
        &self.motherboard
    }

    pub fn motherboard_mut(&mut self) -> &mut Motherboard {
        &mut self.motherboard
    }

    /// The last whole frame the monitor displayed.
    pub fn last_frame(&self) -> Option<&image::RgbImage> {
        self.last_frame.as_ref()
    }

    /// Emulated microseconds played since the runner was created.
    pub fn elapsed_us(&self) -> u64 {
        self.elapsed_before_reset_us
            + (self.motherboard.t_state_counter().value() / T_STATES_PER_UNIT) as u64
    }

    /// Play a whole script file (following its `csl_load` chain).
    pub fn run_file(&mut self, path: impl AsRef<Path>) -> Result<CslReport, String> {
        self.start_file(path)?;
        self.run_to_end()
    }

    /// Play a script given as text; discs are looked up in `base_dir`.
    pub fn run_text(
        &mut self,
        text: &str,
        base_dir: impl Into<PathBuf>,
    ) -> Result<CslReport, String> {
        self.start_text(text, base_dir)?;
        self.run_to_end()
    }

    fn run_to_end(&mut self) -> Result<CslReport, String> {
        loop {
            if let Progress::Done(report) = self.advance(usize::MAX)? {
                return Ok(report);
            }
        }
    }

    /// Published scripts that cannot work as written, corrected before they are played (each
    /// change is listed in the report's `ignored`). Shaker 2.7's module A scripts wait 14 s after
    /// the AU key ('U') before pressing SPACE, a wait written for Shaker 2.6: 2.7 added a 9th AU
    /// sub-test (about 1.6 s more), so the SPACE lands while AU still runs, the next key ('I') is
    /// spent leaving AU's result screen and the AI screens are never reached. The wait becomes
    /// 16 s, as CPCSyntaxError does.
    fn apply_errata(&mut self, text: String) -> String {
        if !text.contains("SHAKE27A") {
            return text;
        }
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let instruction = |line: &str| line.split(';').next().unwrap_or("").trim().to_owned();
        for n in 0..lines.len() {
            if instruction(&lines[n]) != "key_output 'U'" {
                continue;
            }
            let Some(w) = (n + 1..lines.len()).find(|&w| !instruction(&lines[w]).is_empty()) else {
                continue;
            };
            if instruction(&lines[w]) == "wait 14000000" {
                lines[w] = "wait 16000000".to_owned();
                self.report.ignored.push(format!(
                    "erratum: line {}, wait after key 'U' 14000000 -> 16000000 (Shaker 2.7's AU has a 9th sub-test)",
                    w + 1
                ));
            }
        }
        lines.join("\n") + "\n"
    }

    /// Start a script file, played by [`Self::advance`].
    pub fn start_file(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        let name = path.as_ref().to_string_lossy().into_owned();
        self.load_script(&name, Path::new(""))
    }

    /// Start a script given as text, played by [`Self::advance`]; its files are looked up
    /// from `base_dir`.
    pub fn start_text(&mut self, text: &str, base_dir: impl Into<PathBuf>) -> Result<(), String> {
        self.base_dir = base_dir.into();
        let text = self.apply_errata(text.to_owned());
        let script = parse_csl_with_rich_errors(&text, None).map_err(|e| e.to_string())?;
        self.script = script.iter().cloned().collect();
        self.waiting = None;
        Ok(())
    }

    /// The script `name` (from `dir`) replaces what is left of the one playing.
    fn load_script(&mut self, name: &str, dir: &Path) -> Result<(), String> {
        let (path, text) = self.files.script(dir, name)?;
        let text = self.apply_errata(text);
        let script = parse_csl_with_rich_errors(&text, Some(path.display().to_string()))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        self.script = script.iter().cloned().collect();
        self.waiting = None;
        Ok(())
    }

    /// Play the script for about `t_states` T-states of emulated time, stopping early when the
    /// monitor completes a frame or the script ends.
    pub fn advance(&mut self, t_states: usize) -> Result<Progress, String> {
        let mut spent = 0usize;
        loop {
            match &self.waiting {
                None => match self.script.pop_front() {
                    Some(instruction) => self.execute(&instruction)?,
                    None => {
                        self.report.elapsed_us = self.elapsed_us();
                        return Ok(Progress::Done(std::mem::take(&mut self.report)));
                    }
                },
                Some(_) => {
                    if self.wait_is_over() {
                        self.waiting = None;
                        continue;
                    }
                    let before = self.motherboard.t_state_counter().value();
                    let codes = self.step();
                    spent = spent.saturating_add(
                        self.motherboard
                            .t_state_counter()
                            .value()
                            .saturating_sub(before),
                    );
                    match &mut self.waiting {
                        Some(Wait::Ssm { code, raised, .. }) => *raised |= codes.contains(code),
                        Some(Wait::Typing(_)) => self.ssm_while_typing.extend(codes),
                        _ => {}
                    }
                }
            }
            if std::mem::take(&mut self.new_frame) {
                return Ok(Progress::Frame);
            }
            if spent >= t_states {
                return Ok(Progress::Running);
            }
        }
    }

    /// Whether the instruction playing is over, checked before each machine step.
    fn wait_is_over(&mut self) -> bool {
        let now = self.motherboard.t_state_counter().value();
        match self.waiting.as_mut() {
            None => true,
            Some(Wait::Until(target) | Wait::Typing(target)) => now >= *target,
            Some(Wait::VsyncOff) => {
                if !self.motherboard.crtc().vsync() {
                    self.waiting = Some(Wait::VsyncOn);
                }
                false
            }
            Some(Wait::VsyncOn) => self.motherboard.crtc().vsync(),
            Some(Wait::Ssm {
                code,
                deadline,
                instruction,
                raised,
            }) => {
                if *raised {
                    return true;
                }
                if now < *deadline {
                    return false;
                }
                let note = format!(
                    "{instruction}: gave up waiting for SSM code {code:04X} after {} µs (PC {:04X})",
                    self.wait_timeout_us,
                    self.motherboard.pc()
                );
                self.note(note);
                true
            }
            Some(Wait::DriveOnOff {
                count,
                done,
                on,
                deadline,
            }) => {
                if now >= *deadline {
                    let note = format!(
                        "wait_driveonoff: gave up after {} µs, motor never cycled {count} time(s)",
                        self.wait_timeout_us
                    );
                    self.note(note);
                    return true;
                }
                let motor = self.motherboard.fdc().motor();
                if !*on && motor {
                    *on = true;
                } else if *on && !motor {
                    *on = false;
                    *done += 1;
                    if *done >= (*count).max(1) {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// A note of the report, printed too when verbose.
    fn note(&mut self, note: String) {
        if self.verbose {
            eprintln!("[{:>9} µs] {note}", self.elapsed_us());
        }
        self.report.ignored.push(note);
    }

    /// Play one instruction: the script's own state (the waits, the folders, the machine
    /// chosen for the next `reset`) here, everything that acts on the machine or its media as
    /// the commands of the menus, run on this runner (see [`Self::commands_of`]).
    fn execute(&mut self, instruction: &CslInstruction) -> Result<(), String> {
        use CslInstruction::*;
        match instruction {
            CslVersion(_) | Comment(_) | Empty => {}
            InstructionWithComment(inner, _) => return self.execute(inner),
            CrtcSelect(model) => self.crtc = crtc_type(*model),
            GateArray(model) => {
                self.gate_array = match model {
                    cpclib_csl::GateArrayModel::Model40010 => crate::ga::GateArrayType::Ga40010,
                    cpclib_csl::GateArrayModel::Model40007
                    | cpclib_csl::GateArrayModel::Model40008 => crate::ga::GateArrayType::Ga40007_8,
                }
            }
            MemoryExp(expansion) => self.memory_exp = *expansion,
            CpcModel(model) => {
                use crate::motherboard::CpcModel as Model;
                self.model = Some(match model {
                    cpclib_csl::CpcModel::Cpc464 => Model::Cpc464,
                    cpclib_csl::CpcModel::Cpc664 => Model::Cpc664,
                    cpclib_csl::CpcModel::Cpc6128 => Model::Cpc6128,
                    cpclib_csl::CpcModel::Cpc6128Plus => Model::Cpc6128Plus,
                    cpclib_csl::CpcModel::Cpc464Plus => Model::Cpc464Plus,
                    cpclib_csl::CpcModel::GX4000 => Model::Gx4000,
                });
            }
            RomDir(dir) => self.rom_dir = Some(resolve(&self.base_dir, dir.as_str())),
            RomConfig(config) => {
                let path = resolve(
                    self.rom_dir.as_ref().unwrap_or(&self.base_dir),
                    config.filename.as_str(),
                );
                match config.rom_type {
                    // a CPC+ cartridge, for the next reset
                    cpclib_csl::RomType::Cartridge => {
                        let bytes = std::fs::read(&path)
                            .map_err(|e| format!("rom_config {}: {e}", path.display()))?;
                        // checked now, as the script names it
                        crate::asic::Cartridge::from_bytes(
                            &bytes,
                            &path.display().to_string(),
                            None,
                        )?;
                        self.cartridge = Some((path.display().to_string(), bytes));
                    }
                    // an upper ROM, for the next reset
                    cpclib_csl::RomType::Upper => {
                        self.roms.retain(|(slot, _)| *slot != config.num);
                        self.roms.push((config.num, path.display().to_string()));
                    }
                    cpclib_csl::RomType::Lower | cpclib_csl::RomType::Multiface2 => {
                        self.note(format!("{instruction}: not supported"))
                    }
                }
            }
            Wait(us) => {
                let target =
                    self.motherboard.t_state_counter().value() + *us as usize * T_STATES_PER_UNIT;
                self.waiting = Some(self::Wait::Until(target));
            }
            WaitVsyncOffOn => self.waiting = Some(self::Wait::VsyncOff),
            WaitSsm0000 => self.wait_for_ssm_code(SSM_WAIT_RELEASE, "wait_ssm0000"),
            // `wait_ssm <code>` (CSL v1.5): like `wait_ssm0000`, but for any code - including one a
            // `screenshot`-triggering value like `0xFFFE` would also raise, since this only waits
            // for it, it does not suppress the screenshot that code still asks for (see
            // [`Self::after_instruction`]).
            WaitSsm(code) => self.wait_for_ssm_code(*code, "wait_ssm"),
            // `wait_driveonoff [<count>]`: waits for the FDC's own motor line to go on then off
            // again, `count` times (default 1).
            WaitDriveOnOff(count) => {
                let deadline = self.motherboard.t_state_counter().value()
                    + self.wait_timeout_us as usize * T_STATES_PER_UNIT * (*count).max(1) as usize;
                self.waiting = Some(self::Wait::DriveOnOff {
                    count: *count,
                    done: 0,
                    on: false,
                    deadline,
                });
            }
            DiskDir(dir) => self.base_dir = resolve(&self.base_dir, dir.as_str()),
            ScreenshotName(name) => self.screenshot_name = Some(name.as_str().to_owned()),
            ScreenshotDir(dir) => {
                self.screenshot_dir = PathBuf::from(dir.as_str().replace('\\', "/"))
            }
            SnapshotName(name) => self.snapshot_name = Some(name.as_str().to_owned()),
            SnapshotDir(dir) => self.snapshot_dir = Some(resolve(&self.base_dir, dir.as_str())),
            TapeDir(dir) => self.tape_dir = Some(resolve(&self.base_dir, dir.as_str())),
            CslLoad(name) => {
                let dir = self.base_dir.clone();
                self.load_script(name.as_str(), &dir)
                    .map_err(|e| format!("csl_load: {e}"))?;
            }
            Reset(_)
            | DiskInsert { .. }
            | TapeInsert(_)
            | TapePlay
            | TapeStop
            | TapeRewind
            | SnapshotLoad(_)
            | Snapshot { .. }
            | SnapshotVersion(_)
            | Screenshot { .. }
            | KeyDelay { .. }
            | KeyOutput(_)
            | KeyFromFile(_)
            | KeyboardWrite(_) => {
                // the media the script names are needed: a script that cannot find them stops
                let needed = matches!(
                    instruction,
                    DiskInsert { .. } | TapeInsert(_) | SnapshotLoad(_) | KeyFromFile(_)
                );
                for command in self.commands_of(instruction)? {
                    let id = command.id;
                    match command.execute(self) {
                        Ok(Outcome::Done(Some(message))) if self.verbose => {
                            eprintln!("[{:>9} µs] {id}: {message}", self.elapsed_us())
                        }
                        Ok(Outcome::Done(_)) => {}
                        Ok(Outcome::NeedsArg(_)) => {
                            self.note(format!("{instruction}: {id} needs an argument"))
                        }
                        Err(error) if needed => return Err(format!("{instruction}: {error}")),
                        Err(error) => self.note(format!("{instruction}: {error}")),
                    }
                }
                if matches!(instruction, SnapshotLoad(_)) {
                    // the snapshot's keyboard, not the keys typed before
                    self.key_events.clear();
                    self.keys_busy_until = 0;
                }
            }
        }
        Ok(())
    }

    /// The commands an instruction acting on the machine or its media is played with: the
    /// ones of the menus, the files it names given as bytes.
    fn commands_of(&mut self, instruction: &CslInstruction) -> Result<Vec<Command>, String> {
        use CslInstruction::*;
        let command = |id: &str, arg: Option<Arg>| {
            match arg {
                Some(arg) => Command::with_arg(id, arg),
                None => Command::new(id),
            }
            .map_err(|e| e.to_string())
        };
        Ok(match instruction {
            Reset(_) => return self.reset_commands(),
            DiskInsert { drive, filename } => {
                // `games.zip#side b.dsk`: the archive is the file to find
                let (path, _) = split_disc_spec(filename.as_str());
                let data = self
                    .files
                    .disc(&self.base_dir, path, &mut self.report.ignored)?;
                let id = match drive {
                    Drive::A => "drive-a-select-disc",
                    Drive::B => "drive-b-select-disc",
                };
                vec![command(
                    id,
                    Some(Arg::Bytes {
                        name: filename.as_str().to_owned(),
                        data,
                    }),
                )?]
            }
            TapeInsert(name) => {
                let dir = self
                    .tape_dir
                    .clone()
                    .unwrap_or_else(|| self.base_dir.clone());
                let data = self.files.tape(&dir, name.as_str())?;
                // inserted, not playing: tape_play presses PLAY
                vec![
                    command(
                        "tape-insert",
                        Some(Arg::Bytes {
                            name: name.as_str().to_owned(),
                            data,
                        }),
                    )?,
                    command("tape-stop", None)?,
                ]
            }
            TapePlay => vec![command("tape-play", None)?],
            TapeStop => vec![command("tape-stop", None)?],
            TapeRewind => vec![command("tape-rewind", None)?],
            SnapshotLoad(name) => {
                let dir = self
                    .snapshot_dir
                    .clone()
                    .unwrap_or_else(|| self.base_dir.clone());
                let data = self.files.snapshot(&dir, name.as_str())?;
                vec![command(
                    "snapshot-load",
                    Some(Arg::Bytes {
                        name: name.as_str().to_owned(),
                        data,
                    }),
                )?]
            }
            Snapshot { .. } => {
                let stem = self.snapshot_name.clone().unwrap_or_else(|| "bemu".into());
                let file = self.numbered(stem.trim_end_matches(".sna"), "sna");
                let dir = self
                    .snapshot_dir
                    .clone()
                    .unwrap_or_else(|| self.base_dir.clone());
                let arg = if self.files.on_disk() {
                    Arg::Path(dir.join(file))
                } else {
                    Arg::Text(file)
                };
                vec![command("snapshot-save", Some(arg))?]
            }
            SnapshotVersion(version) => vec![command(
                match version {
                    cpclib_csl::SnapshotVersion::V2 => "snapshot-version-2",
                    // the V1 one is not written any more: the closest
                    cpclib_csl::SnapshotVersion::V1 | cpclib_csl::SnapshotVersion::V3 => {
                        "snapshot-version-3"
                    }
                },
                None,
            )?],
            // named by the runner (see the request)
            Screenshot { .. } => vec![command("screenshot-save", Some(Arg::Path(PathBuf::new())))?],
            KeyDelay {
                press_delay,
                delay_after_key,
                delay_after_cr,
            } => vec![command(
                "autotype-delays",
                Some(Arg::Text(format!(
                    "{press_delay} {} {}",
                    delay_after_key.unwrap_or(*press_delay),
                    delay_after_cr.unwrap_or(*press_delay)
                ))),
            )?],
            KeyOutput(output) => vec![command(
                "autotype-text",
                Some(Arg::Text(csl_text(output.elements()))),
            )?],
            KeyFromFile(name) => {
                let data = self.files.text(&self.base_dir, name.as_str())?;
                vec![command(
                    "autotype-file",
                    Some(Arg::Bytes {
                        name: name.as_str().to_owned(),
                        data,
                    }),
                )?]
            }
            KeyboardWrite(rows) => vec![command(
                "keyboard-write",
                Some(Arg::Text(
                    rows.iter()
                        .map(|row| format!("{row:02X}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                )),
            )?],
            _ => Vec::new(),
        })
    }

    /// `reset`: the machine the script chose (`cpc_model`, `crtc_select`, `gate_array`,
    /// `memory_exp`, `rom_config`) set with the commands of the Machine menu, then switched off
    /// and on (Machine > Power cycle).
    fn reset_commands(&mut self) -> Result<Vec<Command>, String> {
        use cpclib_csl::MemoryExpansion::*;
        let mut ids: Vec<String> = Vec::new();
        let mb = &self.motherboard;
        let model = self.model.unwrap_or(self.base.model());
        if model != mb.model() {
            ids.push(format!("machine-platform-{}", platform_id(model)));
        }
        // a CPC+'s CRTC is in its ASIC
        if !model.is_plus() {
            ids.push(format!("machine-crtc-{}", self.crtc as u8));
        }
        ids.push(
            match self.gate_array {
                crate::ga::GateArrayType::Ga40010 => "machine-ga-40010",
                _ => "machine-ga-40007",
            }
            .to_owned(),
        );
        ids.push(match self.memory_exp {
            Kb128 => match self.base.ram_kb() {
                Some(kb) => format!("machine-ram-{kb}"),
                None => "machine-ram-default".to_owned(),
            },
            Kb256Standard | Kb256Silicon => "machine-ram-256".to_owned(),
            Kb512DkTronics => "machine-ram-512".to_owned(),
            Mb4 => "machine-ram-4160".to_owned(),
        });
        let mut commands = ids
            .iter()
            .map(|id| Command::new(id).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some((name, data)) = self.cartridge.clone().filter(|_| model.is_plus()) {
            commands.push(
                Command::with_arg("cartridge-insert", Arg::Bytes { name, data })
                    .map_err(|e| e.to_string())?,
            );
        }
        for (slot, file) in &self.roms {
            commands.push(
                Command::with_arg("rom-load", Arg::Text(format!("{slot}={file}")))
                    .map_err(|e| e.to_string())?,
            );
        }
        commands.push(Command::new("machine-reset-hard").map_err(|e| e.to_string())?);
        Ok(commands)
    }

    /// `stem.ext`, or `stem_N.ext` the Nth time the name is used.
    fn numbered(&mut self, stem: &str, extension: &str) -> String {
        let count = self.taken.entry(format!("{stem}.{extension}")).or_insert(0);
        let file = if *count == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}_{count}.{extension}")
        };
        *count += 1;
        file
    }

    // ------------------------------------------------------------- machine

    /// Machine > Power cycle: the machine built again as it is configured now (model, CRTC,
    /// Gate Array, RAM, cartridge, upper ROMs), from the base the runner was given; the discs,
    /// the tape and the sound recorded so far go in the new one.
    fn rebuild(&mut self) {
        if self.verbose {
            eprintln!(
                "[{:>9} µs] reset, CRTC {}",
                self.elapsed_us(),
                self.crtc as u8
            );
        }
        self.elapsed_before_reset_us = self.elapsed_us();
        let live = &self.motherboard;
        let mut builder = self.base;
        builder
            .set_crtc(live.crtc().crtc_type())
            .set_gate_array_type(live.ga().gate_array_type())
            .enable_interruptions();
        if live.model() != self.base.model() {
            builder.set_model(live.model());
        }
        let mut motherboard = builder.build();
        let previous = &mut self.motherboard;
        if motherboard.total_ram_kb() != previous.total_ram_kb() {
            let _ = motherboard.set_total_ram_kb(Some(previous.total_ram_kb()));
        }
        for slot in 0..=255 {
            if motherboard.upper_rom(slot) != previous.upper_rom(slot) {
                let rom = previous.upper_rom(slot).map(<[u8]>::to_vec);
                let _ = motherboard.set_upper_rom(slot, rom.as_deref());
            }
        }
        if let Some(cartridge) = previous
            .cartridge()
            .filter(|_| motherboard.model().is_plus())
        {
            motherboard.insert_cartridge(Some(cartridge.clone()));
            motherboard.restart();
        }
        // The sound recorded so far (and its settings) outlives the machine.
        motherboard
            .psg_mut()
            .take_audio_output_from(previous.psg_mut());
        motherboard.take_cassette_and_watches_from(previous);
        for unit in 0..2 {
            let fdc = motherboard.fdc_mut();
            fdc.set_drive_kind(drive(unit), self.drive_kinds[unit]);
            fdc.insert_image(drive(unit), previous.fdc().image(drive(unit)).cloned());
        }
        self.motherboard = motherboard;
        self.key_events.clear();
        self.keys_busy_until = 0;
        self.ssm_while_typing.clear();
    }

    // ---------------------------------------------------------------- time

    /// One instruction, then whatever it made happen: due key events and SSM
    /// requests. Returns the SSM codes it raised.
    fn step(&mut self) -> Vec<u16> {
        let monitor = &mut self.monitor;
        self.motherboard
            .execute_next_opcode_with_video(|(rgb, sync)| monitor.receive(rgb, sync));
        if let Some(frame) = self.monitor.take_frame() {
            self.last_frame = Some(frame);
            self.new_frame = true;
        }
        self.after_instruction()
    }

    fn after_instruction(&mut self) -> Vec<u16> {
        self.apply_due_key_events();
        let codes = self.motherboard.cpu_mut().take_ssm_codes();
        for &code in &codes {
            self.report.ssm_codes.push(code);
            match code {
                SSM_WAIT_RELEASE => {}
                // one frame later the picture is the same: the Shaker
                // programs draw once then spin
                code => {
                    let name = (code == SSM_NAMED_SCREENSHOT)
                        .then(|| self.screenshot_name.clone())
                        .flatten();
                    if let Err(error) = self.screenshot(Some(code), name.as_deref()) {
                        self.report
                            .ignored
                            .push(format!("screenshot {code:04X}: {error}"));
                    }
                }
            }
        }
        codes
    }

    /// Bounded: a script waiting for a code that never comes must not hang an unattended run
    /// forever.
    ///
    /// A code the program raised while the previous `key_output` was still being typed counts:
    /// the script's `key_output ' '` then `wait_ssm0000` waits for the program's answer to the key,
    /// which comes before `key_output` is over (it also waits the third `key_delay`, 400 ms in
    /// the Shaker scripts). Without it every such wait ran into its timeout (Shaker module E).
    fn wait_for_ssm_code(&mut self, code: u16, instruction: &'static str) {
        if std::mem::take(&mut self.ssm_while_typing).contains(&code) {
            return;
        }
        let deadline = self.motherboard.t_state_counter().value()
            + self.wait_timeout_us as usize * T_STATES_PER_UNIT;
        self.waiting = Some(Wait::Ssm {
            code,
            deadline,
            instruction,
            raised: false,
        });
    }

    // ------------------------------------------------------------ keyboard

    /// `key_output`: types the keys one after the other and only then lets the script go on: each key
    /// is held `press` and followed by `after_key`, and the third `key_delay` value is waited once the
    /// whole output is typed (the standard says "after a CR"). The Shaker scripts are timed that way
    /// (CPCSyntaxError plays them so too): their waits after a key are often shorter than the test
    /// they wait for (`wait 2200000 ; 2315008` for AI's second screen) and only add up with the
    /// typing time, otherwise the next key lands while the test still runs and the rest of the
    /// module is out of step.
    fn schedule_keys(&mut self, elements: &[KeyElement]) {
        let now = self.motherboard.t_state_counter().value();
        let mut at = self.keys_busy_until.max(now);
        let delays = self.settings.typing.unwrap_or_default();
        let units = |us: u64| us as usize * T_STATES_PER_UNIT;

        let mut no_delay_next = false;
        for element in elements {
            if matches!(element, KeyElement::Special(SpecialKey::NoDelayNextKey)) {
                no_delay_next = true;
                continue;
            }
            let Some(keys) = keys_of(element, self.motherboard.keyboard_layout()) else {
                self.report.ignored.push(format!("key {element}"));
                continue;
            };
            self.key_events.push_back(KeyEvent {
                due_t_states: at,
                keys: keys.clone(),
                pressed: true,
            });
            at += units(delays.press_us);
            self.key_events.push_back(KeyEvent {
                due_t_states: at,
                keys,
                pressed: false,
            });
            if !std::mem::take(&mut no_delay_next) {
                at += units(delays.after_key_us);
            }
        }
        at += units(delays.after_text_us);
        self.keys_busy_until = at;
        // the script goes on once the keys are typed
        self.ssm_while_typing.clear();
        self.waiting = Some(Wait::Typing(at));
    }

    fn apply_due_key_events(&mut self) {
        while let Some(event) = self.key_events.front() {
            if event.due_t_states > self.motherboard.t_state_counter().value() {
                break;
            }
            let event = self.key_events.pop_front().unwrap();
            for key in event.keys {
                self.motherboard
                    .keyboard_mut()
                    .set_pressed(key, event.pressed);
            }
        }
    }

    // --------------------------------------------------------- screenshots

    /// Save the last whole frame the monitor displayed (the next one when there is none yet).
    fn screenshot(&mut self, ssm_code: Option<u16>, name: Option<&str>) -> Result<(), String> {
        let mut ticks = 0;
        while self.last_frame.is_none() {
            if ticks > MAX_FRAME_TICKS {
                let crtc = self.motherboard.crtc();
                return Err(format!(
                    "the machine displays no complete frame (R0-R9: {:?}, PC {:04X})",
                    [
                        crtc.r0(),
                        crtc.r1(),
                        crtc.r2(),
                        crtc.r3(),
                        crtc.r4(),
                        crtc.r5(),
                        crtc.r6(),
                        crtc.r7(),
                        crtc.r8(),
                        crtc.r9()
                    ],
                    self.motherboard.pc()
                ));
            }
            let monitor = &mut self.monitor;
            self.motherboard
                .execute_next_opcode_with_video(|(rgb, sync)| {
                    monitor.receive(rgb, sync);
                    ticks += 1;
                });
            self.apply_due_key_events();
            self.last_frame = self.monitor.take_frame();
            self.new_frame |= self.last_frame.is_some();
        }
        let frame = self.last_frame.clone().unwrap();

        let stem = match (name, ssm_code) {
            (Some(name), _) => name.trim_end_matches(".png").to_owned(),
            (None, Some(code)) => match (self.module, self.manifest) {
                (Some(module), Some(manifest)) => manifest.file_stem(module, self.crtc as u8, code),
                _ => format!("AMSRUST_{}_{code:04X}", self.crtc as u8),
            },
            (None, None) => format!("AMSRUST_{}", self.crtc as u8),
        };
        let count = self.taken.entry(stem.clone()).or_insert(0);
        let file = if *count == 0 {
            format!("{stem}.png")
        } else {
            format!("{stem}_{count}.png")
        };
        *count += 1;

        let path = self.screenshots.save(&self.screenshot_dir, &file, &frame)?;
        if self.verbose {
            eprintln!("[{:>9} µs] {}", self.elapsed_us(), path.display());
        }
        self.report.screenshots.push(Screenshot {
            path,
            ssm_code,
            crtc: self.crtc,
            at_us: self.elapsed_us(),
        });
        Ok(())
    }
}

/// The runner is what the commands of the script act on: its machine, its settings (typing
/// delays, snapshot version); the screenshots go to its sink, the files it writes to its disk or
/// its report.
impl CommandTarget for CslRunner {
    fn motherboard(&self) -> &Motherboard {
        &self.motherboard
    }
    fn motherboard_mut(&mut self) -> &mut Motherboard {
        &mut self.motherboard
    }
    fn settings(&self) -> Option<&HostSettings> {
        Some(&self.settings)
    }
    fn settings_mut(&mut self) -> Option<&mut HostSettings> {
        Some(&mut self.settings)
    }
    fn has_file_system(&self) -> bool {
        self.files.on_disk()
    }
    fn csl_running(&self) -> bool {
        true
    }
    fn can(&self, request: &HostRequest) -> bool {
        matches!(
            request,
            HostRequest::Screenshot(_) | HostRequest::SaveFile { .. }
        )
    }
    fn request(&mut self, request: HostRequest) -> Result<(), CommandError> {
        match request {
            // the name `screenshot_name` gave, the runner's own otherwise
            HostRequest::Screenshot(path) => {
                let name = path
                    .filter(|path| !path.as_os_str().is_empty())
                    .map(|path| path.display().to_string())
                    .or_else(|| self.screenshot_name.clone());
                if let Err(error) = self.screenshot(None, name.as_deref()) {
                    self.note(format!("screenshot: {error}"));
                }
                Ok(())
            }
            HostRequest::SaveFile { name, data } => {
                self.report.files.push((name, data));
                Ok(())
            }
            _ => Err(CommandError::Unsupported),
        }
    }

    /// `key_output` typed on the runner's clock, the script waiting for the keys.
    fn type_text(&mut self, text: &str) -> Result<(), CommandError> {
        let output = cpclib_csl::KeyOutput::try_from(text).map_err(CommandError::Invalid)?;
        self.schedule_keys(output.elements());
        Ok(())
    }

    fn power_cycle(&mut self) {
        self.rebuild();
    }
}

// ---------------------------------------------------------------- helpers

/// `elements` as the text of an Auto-type command: escaped as CSL writes them, quoted (what
/// [`cpclib_csl::KeyOutput::try_from`] reads back as they are).
fn csl_text(elements: &[KeyElement]) -> String {
    fn push(text: &mut String, element: &KeyElement) {
        match element {
            KeyElement::Character(c @ ('\'' | '{' | '}' | '\\')) => {
                text.push_str(&format!("\\({c})"))
            }
            KeyElement::Simultaneous(elements) => {
                text.push('{');
                for element in elements {
                    push(text, element);
                }
                text.push('}');
            }
            element => text.push_str(&element.to_string()),
        }
    }
    let mut text = String::from("'");
    for element in elements {
        push(&mut text, element);
    }
    text.push('\'');
    text
}

/// The id of `model` in the Machine > Platform commands.
fn platform_id(model: crate::motherboard::CpcModel) -> &'static str {
    use crate::motherboard::CpcModel::*;
    match model {
        Cpc464 => "cpc464",
        Cpc664 => "cpc664",
        Cpc6128 => "cpc6128",
        Cpc464Plus => "cpc464plus",
        Cpc6128Plus => "cpc6128plus",
        Gx4000 => "gx4000",
    }
}

/// Published Shaker scripts have shipped with a `disk_insert` naming a
/// disc from a previous release that is no longer in the package (e.g.
/// `SHAKER-A-0.CSL` still says `shaker26.dsk`, present only as
/// `shaker27.dsk`) - real players reportedly get away with this because
/// whatever disc the emulator already had mounted stays in the drive
/// when the named one fails to load. Since there is nothing "already
/// mounted" here, the fallback instead looks for the one disc image
/// that actually sits near the script and uses that.
fn only_disc_image_nearby(
    base_dir: &Path,
    wanted: &str,
    notes: &mut Vec<String>,
) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in base_dir.ancestors().take(3) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_disc = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dsk"));
            if is_disc && !found.contains(&path) {
                found.push(path);
            }
        }
        if !found.is_empty() {
            break; // a closer directory's discs take priority
        }
    }
    match found.as_slice() {
        [only] => {
            notes.push(format!(
                "disk_insert: {wanted:?} not found, using {} instead",
                only.display()
            ));
            Some(only.clone())
        }
        _ => None, // none, or too ambiguous to guess
    }
}

fn drive(unit: usize) -> FdcDrive {
    if unit == 0 {
        FdcDrive::A
    } else {
        FdcDrive::B
    }
}

/// CSL type names to ours (1A/1B are both the UM6845R).
fn crtc_type(model: CslCrtc) -> CrtcType {
    match model {
        CslCrtc::Type0 => CrtcType::Crtc0,
        CslCrtc::Type1 | CslCrtc::Type1A | CslCrtc::Type1B => CrtcType::Crtc1,
        CslCrtc::Type2 => CrtcType::Crtc2,
        CslCrtc::Type3 => CrtcType::Crtc3,
        CslCrtc::Type4 => CrtcType::Crtc4,
    }
}

/// CSL files come from Windows: backslashes, arbitrary case, sometimes a
/// dangling BOM.
fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.trim_start_matches('\u{feff}').to_owned())
}

fn resolve(base: &Path, name: &str) -> PathBuf {
    let name = name.replace('\\', "/");
    let name = name.strip_prefix("Z:").unwrap_or(&name);
    let path = Path::new(name);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// `path` if it exists, else the entry of its directory that differs only by case.
fn find_case_insensitive(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let wanted = path.file_name()?.to_string_lossy().to_lowercase();
    std::fs::read_dir(path.parent()?)
        .ok()?
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().to_lowercase() == wanted)
        .map(|entry| entry.path())
}

/// The keys to hold down together to produce a character on a UK keyboard.
///
/// Public so `main.rs`'s `--window` interactive mode can reuse it for
/// typed-character input, not just CSL scripts - see `update_keyboard`'s
/// doc comment there for why that's needed (host keyboard layouts other
/// than US/UK QWERTY can't be represented as a fixed physical-key table).
pub fn keys_of_char(c: char) -> Option<Vec<CpcKey>> {
    use CpcKey::*;
    let letter = |c: char| -> Option<CpcKey> {
        Some(match c.to_ascii_lowercase() {
            'a' => A,
            'b' => B,
            'c' => C,
            'd' => D,
            'e' => E,
            'f' => F,
            'g' => G,
            'h' => H,
            'i' => I,
            'j' => J,
            'k' => K,
            'l' => L,
            'm' => M,
            'n' => N,
            'o' => O,
            'p' => P,
            'q' => Q,
            'r' => R,
            's' => S,
            't' => T,
            'u' => U,
            'v' => V,
            'w' => W,
            'x' => X,
            'y' => Y,
            'z' => Z,
            _ => return None,
        })
    };
    let digit = |d: u32| -> CpcKey {
        [
            Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
        ][d as usize]
    };

    if c.is_ascii_lowercase() {
        return letter(c).map(|k| vec![k]);
    }
    if c.is_ascii_uppercase() {
        return letter(c).map(|k| vec![Shift, k]);
    }
    if let Some(d) = c.to_digit(10) {
        return Some(vec![digit(d)]);
    }
    Some(match c {
        ' ' => vec![Space],
        '!' => vec![Shift, Digit1],
        '"' => vec![Shift, Digit2],
        '#' => vec![Shift, Digit3],
        '$' => vec![Shift, Digit4],
        '%' => vec![Shift, Digit5],
        '&' => vec![Shift, Digit6],
        '\'' => vec![Shift, Digit7],
        '(' => vec![Shift, Digit8],
        ')' => vec![Shift, Digit9],
        '_' => vec![Shift, Digit0],
        '-' => vec![Minus],
        '=' => vec![Shift, Minus],
        '^' => vec![Caret],
        '@' => vec![At],
        '|' => vec![Shift, At],
        '[' => vec![OpenBracket],
        '{' => vec![Shift, OpenBracket],
        ']' => vec![CloseBracket],
        '}' => vec![Shift, CloseBracket],
        ';' => vec![Semicolon],
        '+' => vec![Shift, Semicolon],
        ':' => vec![Colon],
        '*' => vec![Shift, Colon],
        '/' => vec![Slash],
        '?' => vec![Shift, Slash],
        '.' => vec![Dot],
        '>' => vec![Shift, Dot],
        ',' => vec![Comma],
        '<' => vec![Shift, Comma],
        '\\' => vec![Backslash],
        '`' => vec![Shift, Backslash],
        '\n' => vec![Return],
        _ => return None,
    })
}

fn keys_of_special(key: SpecialKey, layout: KeyboardLayout) -> Option<Vec<CpcKey>> {
    use CpcKey::*;
    Some(match key {
        SpecialKey::Esc => vec![Esc],
        SpecialKey::Tab => vec![Tab],
        SpecialKey::CapsLock => vec![CapsLock],
        SpecialKey::Shift => vec![Shift],
        SpecialKey::Ctrl => vec![Control],
        SpecialKey::Copy => vec![Copy],
        SpecialKey::Clr => vec![Clr],
        SpecialKey::Del => vec![Del],
        SpecialKey::Return => vec![Return],
        SpecialKey::Enter => vec![KeypadEnter],
        SpecialKey::ArrowLeft => vec![CursorLeft],
        SpecialKey::ArrowRight => vec![CursorRight],
        SpecialKey::ArrowUp => vec![CursorUp],
        SpecialKey::ArrowDown => vec![CursorDown],
        SpecialKey::F0 => vec![Keypad0],
        SpecialKey::F1 => vec![Keypad1],
        SpecialKey::F2 => vec![Keypad2],
        SpecialKey::F3 => vec![Keypad3],
        SpecialKey::F4 => vec![Keypad4],
        SpecialKey::F5 => vec![Keypad5],
        SpecialKey::F6 => vec![Keypad6],
        SpecialKey::F7 => vec![Keypad7],
        SpecialKey::F8 => vec![Keypad8],
        SpecialKey::F9 => vec![Keypad9],
        SpecialKey::LeftBrace => return layout.keys_of_char('{'),
        SpecialKey::RightBrace => return layout.keys_of_char('}'),
        SpecialKey::Backslash => return layout.keys_of_char('\\'),
        SpecialKey::Quote => return layout.keys_of_char('\''),
        SpecialKey::NoDelayNextKey => return None,
    })
}

/// The keys of a text in CSL's `key_output` syntax (`\\(RET)`, `\\(ESC)`, `\\(CPY)`…, a newline
/// is RETURN), one entry per key stroke, as a `key_output` types them. `--type` uses it.
pub fn keys_of_text(text: &str, layout: KeyboardLayout) -> Result<Vec<Vec<CpcKey>>, String> {
    // `{\(CTR)h}`: the keys of a group are pressed together, as in a CSL script (the escapes
    // `\({)` and `\(})` are the characters themselves)
    let bracket = |text: &str, wanted: char| {
        text.char_indices()
            .find(|&(at, c)| c == wanted && !text[..at].ends_with("\\("))
            .map(|(at, _)| at)
    };
    let mut strokes = Vec::new();
    let mut rest = text;
    // only a group with a special key in it (`{\(CTR)h}`) is one: `{h}` stays the three characters
    // it always was
    let group = |text: &str| {
        let open = bracket(text, '{')?;
        let close = open + 1 + bracket(&text[open + 1..], '}')?;
        text[open..close].contains("\\(").then_some((open, close))
    };
    while let Some((open, close)) = group(rest) {
        let after = &rest[open + 1..];
        let close = close - open - 1;
        strokes.extend(keys_of_plain_text(&rest[..open], layout)?);
        let mut group: Vec<CpcKey> = Vec::new();
        for key in keys_of_plain_text(&after[..close], layout)?
            .into_iter()
            .flatten()
        {
            if !group.contains(&key) {
                group.push(key);
            }
        }
        strokes.push(group);
        rest = &after[close + 1..];
    }
    strokes.extend(keys_of_plain_text(rest, layout)?);
    Ok(strokes)
}

fn keys_of_plain_text(text: &str, layout: KeyboardLayout) -> Result<Vec<Vec<CpcKey>>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let output = cpclib_csl::KeyOutput::try_from(text)?;
    Ok(output
        .elements()
        .iter()
        .filter(|element| !matches!(element, KeyElement::Special(SpecialKey::NoDelayNextKey)))
        .filter_map(|element| {
            let keys = keys_of(element, layout);
            if keys.is_none() {
                eprintln!("cannot type {element}, skipped");
            }
            keys
        })
        .collect())
}

/// The keys of one element of a `key_output` (a group `{..}` is pressed together).
fn keys_of(element: &KeyElement, layout: KeyboardLayout) -> Option<Vec<CpcKey>> {
    match element {
        KeyElement::Character(c) => layout.keys_of_char(*c),
        KeyElement::Special(key) => keys_of_special(*key, layout),
        KeyElement::Simultaneous(elements) => {
            let mut keys = Vec::new();
            for element in elements {
                for key in keys_of(element, layout)? {
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
            Some(keys)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_text_is_typed_with_the_csl_codes() {
        use CpcKey::*;
        let keys = keys_of_text(r"|rz\(RET)\(ESC)", KeyboardLayout::default()).unwrap();
        assert_eq!(
            keys,
            vec![vec![Shift, At], vec![R], vec![Z], vec![Return], vec![Esc]]
        );
    }

    #[test]
    fn a_key_output_goes_through_the_autotype_text_unchanged() {
        for script in [
            r#"key_output 'RUN"DISC"\(RET)'"#,
            r"key_output '\(')\({)\(})\(\)a{\(SHI)\(CTR)x}\(KOF)b'",
        ] {
            let instructions = parse_csl_with_rich_errors(script, None).unwrap();
            let Some(CslInstruction::KeyOutput(output)) =
                instructions.instructions().first().cloned()
            else {
                panic!("{script}: {instructions:?}");
            };
            let text = csl_text(output.elements());
            let back = cpclib_csl::KeyOutput::try_from(text.as_str()).unwrap();
            assert_eq!(back, output, "{script} -> {text}");
        }
    }

    #[test]
    fn a_group_in_braces_is_one_stroke() {
        use CpcKey::*;
        let keys = keys_of_text(r"a{\(CTR)h}\(RET)", KeyboardLayout::default()).unwrap();
        assert_eq!(keys, vec![vec![A], vec![Control, H], vec![Return]]);
        // braces around plain characters are only characters
        let keys = keys_of_text("{h}", KeyboardLayout::default()).unwrap();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn characters_map_to_the_keys_of_a_uk_keyboard() {
        assert_eq!(keys_of_char('r'), Some(vec![CpcKey::R]));
        assert_eq!(keys_of_char('R'), Some(vec![CpcKey::Shift, CpcKey::R]));
        assert_eq!(keys_of_char('"'), Some(vec![CpcKey::Shift, CpcKey::Digit2]));
        assert_eq!(keys_of_char('7'), Some(vec![CpcKey::Digit7]));
        assert_eq!(keys_of_char('é'), None);
    }

    #[test]
    fn simultaneous_keys_are_pressed_together_once() {
        let group = KeyElement::Simultaneous(vec![
            KeyElement::Character('A'),
            KeyElement::Special(SpecialKey::Shift),
        ]);
        assert_eq!(
            keys_of(&group, KeyboardLayout::Uk),
            Some(vec![CpcKey::Shift, CpcKey::A])
        );
    }

    #[test]
    fn windows_paths_are_resolved_case_insensitively() {
        let base = Path::new("/base");
        assert_eq!(
            resolve(base, r"sub\SHAKE.CSL"),
            Path::new("/base/sub/SHAKE.CSL")
        );
        assert_eq!(resolve(base, r"Z:\tmp\a.dsk"), Path::new("/tmp/a.dsk"));
    }

    /// `snapshot_load` (looked up in `snapshot_dir`) gives the machine the snapshot's state.
    #[test]
    fn snapshot_load_restores_the_machine() {
        use cpclib::sna::{Snapshot, SnapshotFlag, SnapshotVersion};
        let dir = std::env::temp_dir().join(format!("amsrust_csl_sna_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("snaps")).unwrap();
        let mut snapshot = Snapshot::default();
        snapshot.set_value(SnapshotFlag::Z80_PC, 0x4000).unwrap();
        snapshot
            .set_value(SnapshotFlag::CRTC_REG(Some(1)), 32)
            .unwrap();
        snapshot
            .save(
                dir.join("snaps").join("T.SNA").to_str().unwrap(),
                SnapshotVersion::V2,
            )
            .unwrap();

        let mut runner = CslRunner::new(dir.join("out"));
        let report = runner.run_text(
            "csl_version 1.0\nsnapshot_dir 'snaps'\nsnapshot_load 'T.SNA'\n",
            &dir,
        );
        let _ = std::fs::remove_dir_all(&dir);
        report.unwrap();
        assert_eq!(runner.motherboard.crtc().regs_r(1), 32);
        assert_ne!(
            runner.motherboard.pc(),
            0,
            "the snapshot's PC (then a few frames run)"
        );
    }

    /// Screenshots kept in memory, shared with the test.
    #[derive(Clone, Default)]
    struct Collected(std::rc::Rc<std::cell::RefCell<Vec<(String, image::RgbImage)>>>);

    impl ScreenshotSink for Collected {
        fn save(
            &mut self,
            dir: &Path,
            file: &str,
            frame: &image::RgbImage,
        ) -> Result<PathBuf, String> {
            self.0.borrow_mut().push((file.to_owned(), frame.clone()));
            Ok(dir.join(file))
        }
    }

    /// Played a little at a time, a script does exactly what it does in one go: the same
    /// emulated time, codes and screenshots, and the frames come out on the way.
    #[test]
    fn a_script_advanced_step_by_step_plays_as_in_one_go() {
        let script =
            "csl_version 1.0\ncrtc_select 1\nreset\nwait 1500000\nkey_output 'print 1\\(RET)'\n\
                      wait 600000\nwait_vsyncoffon\nscreenshot\nwait 20000\nscreenshot\n";
        let whole = Collected::default();
        let mut runner = CslRunner::with_io(
            Box::new(MemFiles::default()),
            Box::new(whole.clone()),
            "out",
        );
        let expected = runner.run_text(script, ".").unwrap();

        let steps = Collected::default();
        let mut runner = CslRunner::with_io(
            Box::new(MemFiles::default()),
            Box::new(steps.clone()),
            "out",
        );
        runner.start_text(script, ".").unwrap();
        let mut frames = 0;
        let report = loop {
            match runner.advance(5_000).unwrap() {
                Progress::Running => {}
                Progress::Frame => frames += 1,
                Progress::Done(report) => break report,
            }
        };
        assert_eq!(report.elapsed_us, expected.elapsed_us);
        assert_eq!(report.ssm_codes, expected.ssm_codes);
        assert_eq!(report.ignored, expected.ignored);
        assert_eq!(*steps.0.borrow(), *whole.0.borrow(), "the same screenshots");
        assert_eq!(steps.0.borrow().len(), 2);
        // a frame every 19.968 ms (the monitor locks after a frame or two)
        let expected_frames = (report.elapsed_us / 19_968) as i64;
        assert!(
            (frames - expected_frames).abs() <= 3,
            "{frames} frames in {} µs",
            report.elapsed_us
        );
    }

    /// Files given as bytes are found by their name whatever the directory and case a script
    /// names them with; the only disc image given stands in for a disc not given.
    #[test]
    fn files_given_as_bytes_are_found_by_name() {
        let mut files = MemFiles::default();
        files.insert("Shaker27.DSK", vec![1]);
        files.insert("next.csl", b"\xef\xbb\xbfcsl_version 1.0".to_vec());
        let mut notes = Vec::new();
        let base = Path::new("x");
        assert_eq!(
            files.disc(base, r"Z:\discs\SHAKER27.dsk", &mut notes),
            Ok(vec![1])
        );
        assert!(notes.is_empty());
        assert_eq!(files.disc(base, "shaker26.dsk", &mut notes), Ok(vec![1]));
        assert_eq!(notes.len(), 1, "the substitution is noted");
        assert_eq!(
            files.script(base, "NEXT").map(|(_, text)| text),
            Ok("csl_version 1.0".to_owned())
        );
        assert!(files.snapshot(base, "a.sna").is_err());
        files.insert("other.dsk", vec![2]);
        assert!(
            files.disc(base, "shaker26.dsk", &mut notes).is_err(),
            "two discs: no guess"
        );
    }

    /// A disc a script names is found inside a zip archive given, by its file name; a lone archive
    /// stands in for a disc not found.
    #[test]
    fn discs_are_found_in_given_archives() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("dist/Demo.DSK", options).unwrap();
        zip.write_all(&[7]).unwrap();
        let archive = zip.finish().unwrap().into_inner();
        let mut files = MemFiles::default();
        files.insert("demo.zip", archive.clone());
        let (mut notes, base) = (Vec::new(), Path::new("x"));
        assert_eq!(files.disc(base, "DEMO.dsk", &mut notes), Ok(vec![7]));
        assert!(notes.is_empty());
        assert_eq!(
            files.disc(base, "other.dsk", &mut notes),
            Ok(archive),
            "the lone archive stands in"
        );
        assert_eq!(notes.len(), 1);
    }

    /// Shaker 2.7 module A: the wait after the AU key is lengthened, nothing else changes.
    #[test]
    fn shaker_27_au_wait_erratum() {
        let mut runner = CslRunner::new(std::env::temp_dir());
        let script = "key_output 'RUN\"SHAKE27A\"\\(RET)'\nkey_output 'U'\n; comment\nwait 14000000\t; 11803953+1007210\nkey_output 'Y'\nwait 14000000\n";
        let fixed = runner.apply_errata(script.to_owned());
        assert_eq!(fixed.lines().nth(3), Some("wait 16000000"));
        assert_eq!(
            fixed.lines().nth(5),
            Some("wait 14000000"),
            "only the wait after 'U'"
        );
        assert_eq!(runner.report.ignored.len(), 1);
        let other = "key_output 'U'\nwait 14000000\n";
        assert_eq!(
            runner.apply_errata(other.to_owned()),
            other,
            "not a Shaker 2.7 module A script"
        );
    }

    #[test]
    fn both_um6845r_names_are_crtc_1() {
        assert_eq!(crtc_type(CslCrtc::Type1A), CrtcType::Crtc1);
        assert_eq!(crtc_type(CslCrtc::Type1B), CrtcType::Crtc1);
        assert_eq!(crtc_type(CslCrtc::Type4), CrtcType::Crtc4);
    }
}
