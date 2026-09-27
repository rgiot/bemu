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
    parse_csl_with_rich_errors, CrtcModel as CslCrtc, CslInstruction, Drive, KeyElement,
    SpecialKey
};

use crate::crtc::CrtcType;
use crate::disc_image::{load_dsk, DiscImage};
use crate::fdc::FdcDrive;
use crate::keyboard::CpcKey;
use crate::monitor::FrameGrabber;
use crate::motherboard::{Motherboard, MotherboardBuilder};

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
    pub at_us: u64
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
    pub elapsed_us: u64
}

#[derive(Debug, Clone, Copy)]
struct KeyDelays {
    press: u64,
    after_key: u64,
    after_cr: u64
}

impl Default for KeyDelays {
    /// CSL standard defaults (in microseconds).
    fn default() -> Self {
        KeyDelays {
            press: 70_000,
            after_key: 70_000,
            after_cr: 400_000
        }
    }
}

/// A change of state of some keys, due at an emulated time.
struct KeyEvent {
    due_t_states: usize,
    keys: Vec<CpcKey>,
    pressed: bool
}

pub struct CslRunner {
    motherboard: Motherboard,
    /// Chosen by `crtc_select`, applied at the next `reset`.
    crtc: CrtcType,
    disc: [Option<DiscImage>; 2],
    key_delays: KeyDelays,
    key_events: VecDeque<KeyEvent>,
    /// When the last scheduled key event is over, in T-states of this machine.
    keys_busy_until: usize,
    screenshot_name: Option<String>,
    screenshot_dir: PathBuf,
    /// Base directory for the discs and the scripts loaded by name.
    base_dir: PathBuf,
    /// Count of screenshots per file stem, to keep repeated codes apart.
    taken: HashMap<String, usize>,
    /// Machines are rebuilt on reset; this accumulates their time.
    elapsed_before_reset_us: u64,
    /// Print each screenshot and reset as it happens.
    verbose: bool,
    report: CslReport
}

impl CslRunner {
    /// `output_dir` receives the screenshots.
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        let crtc = CrtcType::default();
        CslRunner {
            motherboard: Self::machine(crtc),
            crtc,
            disc: [None, None],
            key_delays: KeyDelays::default(),
            key_events: VecDeque::new(),
            keys_busy_until: 0,
            screenshot_name: None,
            screenshot_dir: output_dir.into(),
            base_dir: PathBuf::from("."),
            taken: HashMap::new(),
            elapsed_before_reset_us: 0,
            verbose: false,
            report: CslReport::default()
        }
    }

    /// Print progress on stderr while running.
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    fn machine(crtc: CrtcType) -> Motherboard {
        MotherboardBuilder::default()
            .set_crtc(crtc)
            .enable_interruptions()
            .build()
    }

    /// The machine being driven.
    pub fn motherboard(&self) -> &Motherboard {
        &self.motherboard
    }

    pub fn motherboard_mut(&mut self) -> &mut Motherboard {
        &mut self.motherboard
    }

    /// Emulated microseconds played since the runner was created.
    pub fn elapsed_us(&self) -> u64 {
        self.elapsed_before_reset_us + (self.motherboard.t_state_counter().value() / T_STATES_PER_UNIT) as u64
    }

    /// Play a whole script file (following its `csl_load` chain).
    pub fn run_file(&mut self, path: impl AsRef<Path>) -> Result<CslReport, String> {
        let mut next = Some(path.as_ref().to_path_buf());
        while let Some(path) = next.take() {
            let text = read_text(&path)?;
            let script = parse_csl_with_rich_errors(&text, Some(path.display().to_string()))
                .map_err(|e| format!("{}: {e:?}", path.display()))?;
            self.base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            next = self.run_instructions(script.iter())?;
        }
        self.report.elapsed_us = self.elapsed_us();
        Ok(std::mem::take(&mut self.report))
    }

    /// Play a script given as text; discs are looked up in `base_dir`.
    pub fn run_text(&mut self, text: &str, base_dir: impl Into<PathBuf>) -> Result<CslReport, String> {
        self.base_dir = base_dir.into();
        let script = parse_csl_with_rich_errors(text, None).map_err(|e| format!("{e:?}"))?;
        if let Some(next) = self.run_instructions(script.iter())? {
            return self.run_file(next);
        }
        self.report.elapsed_us = self.elapsed_us();
        Ok(std::mem::take(&mut self.report))
    }

    /// Returns the script to chain to, if any.
    fn run_instructions<'a>(
        &mut self,
        instructions: impl Iterator<Item = &'a CslInstruction>
    ) -> Result<Option<PathBuf>, String> {
        for instruction in instructions {
            if let Some(next) = self.execute(instruction)? {
                return Ok(Some(next));
            }
        }
        Ok(None)
    }

    fn execute(&mut self, instruction: &CslInstruction) -> Result<Option<PathBuf>, String> {
        use CslInstruction::*;
        match instruction {
            CslVersion(_) | Comment(_) | Empty => {},
            InstructionWithComment(inner, _) => return self.execute(inner),
            CrtcSelect(model) => self.crtc = crtc_type(*model),
            Reset(_) => self.reset(),
            Wait(us) => self.run_for(*us),
            WaitVsyncOffOn => self.wait_vsync_off_on(),
            WaitSsm0000 => self.wait_ssm_0000(),
            DiskInsert { drive, filename } => self.insert_disc(*drive, filename.as_str())?,
            DiskDir(dir) => self.base_dir = resolve(&self.base_dir, dir.as_str()),
            KeyDelay {
                press_delay,
                delay_after_key,
                delay_after_cr
            } => {
                self.key_delays = KeyDelays {
                    press: *press_delay,
                    after_key: delay_after_key.unwrap_or(*press_delay),
                    after_cr: delay_after_cr.unwrap_or(*press_delay)
                }
            },
            KeyOutput(output) => self.schedule_keys(output.elements()),
            ScreenshotName(name) => self.screenshot_name = Some(name.as_str().to_owned()),
            ScreenshotDir(dir) => self.screenshot_dir = PathBuf::from(dir.as_str().replace('\\', "/")),
            Screenshot { .. } => {
                let name = self.screenshot_name.clone();
                if let Err(error) = self.screenshot(None, name.as_deref()) {
                    self.report.ignored.push(format!("screenshot: {error}"));
                }
            },
            CslLoad(name) => return Ok(Some(self.find_script(name.as_str())?)),
            other => self.report.ignored.push(other.to_string())
        }
        Ok(None)
    }

    // ------------------------------------------------------------- machine

    fn reset(&mut self) {
        if self.verbose {
            eprintln!("[{:>9} µs] reset, CRTC {}", self.elapsed_us(), self.crtc as u8);
        }
        self.elapsed_before_reset_us = self.elapsed_us();
        self.motherboard = Self::machine(self.crtc);
        for (unit, disc) in self.disc.iter().enumerate() {
            self.motherboard.fdc_mut().insert_image(drive(unit), disc.clone());
        }
        self.key_events.clear();
        self.keys_busy_until = 0;
    }

    fn insert_disc(&mut self, drive_letter: Drive, filename: &str) -> Result<(), String> {
        // The Shaker scripts sit in one directory per module while the disc
        // is shared by all of them: look in the parent directories too.
        let path = self
            .base_dir
            .ancestors()
            .take(3)
            .find_map(|dir| find_case_insensitive(&resolve(dir, filename)))
            .unwrap_or_else(|| resolve(&self.base_dir, filename));
        let image = load_dsk(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let unit = match drive_letter {
            Drive::A => 0,
            Drive::B => 1
        };
        self.motherboard.fdc_mut().insert_image(drive(unit), Some(image.clone()));
        self.disc[unit] = Some(image);
        Ok(())
    }

    fn find_script(&self, name: &str) -> Result<PathBuf, String> {
        let path = resolve(&self.base_dir, name);
        [path.clone(), path.with_extension("csl"), path.with_extension("CSL")]
            .into_iter()
            .find_map(|candidate| find_case_insensitive(&candidate))
            .ok_or_else(|| format!("csl_load: no script {}", path.display()))
    }

    // ---------------------------------------------------------------- time

    /// Run the machine for `us` emulated microseconds.
    fn run_for(&mut self, us: u64) {
        let target = self.motherboard.t_state_counter().value() + us as usize * T_STATES_PER_UNIT;
        while self.motherboard.t_state_counter().value() < target {
            self.step();
        }
    }

    /// One instruction, then whatever it made happen: due key events and SSM
    /// requests. Returns the SSM codes it raised.
    fn step(&mut self) -> Vec<u16> {
        self.motherboard.execute_next_opcode_no_video();
        self.after_instruction()
    }

    fn after_instruction(&mut self) -> Vec<u16> {
        self.apply_due_key_events();
        let codes = self.motherboard.cpu_mut().take_ssm_codes();
        for &code in &codes {
            self.report.ssm_codes.push(code);
            match code {
                SSM_WAIT_RELEASE => {},
                // one frame later the picture is the same: the Shaker
                // programs draw once then spin
                code => {
                    let name = (code == SSM_NAMED_SCREENSHOT)
                        .then(|| self.screenshot_name.clone())
                        .flatten();
                    if let Err(error) = self.screenshot(Some(code), name.as_deref()) {
                        self.report.ignored.push(format!("screenshot {code:04X}: {error}"));
                    }
                }
            }
        }
        codes
    }

    fn wait_ssm_0000(&mut self) {
        // bounded: a script waiting for a code that never comes must end
        for _ in 0..u32::MAX {
            if self.step().contains(&SSM_WAIT_RELEASE) {
                return;
            }
        }
    }

    fn wait_vsync_off_on(&mut self) {
        while self.motherboard.crtc().vsync() {
            self.step();
        }
        while !self.motherboard.crtc().vsync() {
            self.step();
        }
    }

    // ------------------------------------------------------------ keyboard

    fn schedule_keys(&mut self, elements: &[KeyElement]) {
        let now = self.motherboard.t_state_counter().value();
        let mut at = self.keys_busy_until.max(now);
        let delays = self.key_delays;
        let units = |us: u64| us as usize * T_STATES_PER_UNIT;

        let mut no_delay_next = false;
        for element in elements {
            if matches!(element, KeyElement::Special(SpecialKey::NoDelayNextKey)) {
                no_delay_next = true;
                continue;
            }
            let Some(keys) = keys_of(element) else {
                self.report.ignored.push(format!("key {element}"));
                continue;
            };
            let after = if matches!(element, KeyElement::Special(SpecialKey::Return | SpecialKey::Enter)) {
                delays.after_cr
            }
            else {
                delays.after_key
            };
            self.key_events.push_back(KeyEvent {
                due_t_states: at,
                keys: keys.clone(),
                pressed: true
            });
            at += units(delays.press);
            self.key_events.push_back(KeyEvent {
                due_t_states: at,
                keys,
                pressed: false
            });
            if !std::mem::take(&mut no_delay_next) {
                at += units(after);
            }
        }
        self.keys_busy_until = at;
    }

    fn apply_due_key_events(&mut self) {
        while let Some(event) = self.key_events.front() {
            if event.due_t_states > self.motherboard.t_state_counter().value() {
                break;
            }
            let event = self.key_events.pop_front().unwrap();
            for key in event.keys {
                self.motherboard.keyboard_mut().set_pressed(key, event.pressed);
            }
        }
    }

    // --------------------------------------------------------- screenshots

    /// Capture the next whole frame the machine displays (keeping the
    /// program running meanwhile) and save it.
    fn screenshot(&mut self, ssm_code: Option<u16>, name: Option<&str>) -> Result<(), String> {
        let mut grabber = FrameGrabber::default();
        let mut ticks = 0;
        let mut frame = None;
        while frame.is_none() {
            if ticks > MAX_FRAME_TICKS {
                let crtc = self.motherboard.crtc();
                return Err(format!(
                    "the machine displays no complete frame (R0-R9: {:?}, PC {:04X})",
                    [crtc.r0(), crtc.r1(), crtc.r2(), crtc.r3(), crtc.r4(), crtc.r5(), crtc.r6(), crtc.r7(), crtc.r8(), crtc.r9()],
                    self.motherboard.pc()
                ));
            }
            self.motherboard.execute_next_opcode_with_video(|(rgb, sync)| {
                grabber.receive(rgb, sync);
                ticks += 1;
            });
            self.apply_due_key_events();
            frame = grabber.take_frame();
        }
        let frame = frame.unwrap();

        let stem = match (name, ssm_code) {
            (Some(name), _) => name.trim_end_matches(".png").to_owned(),
            (None, Some(code)) => format!("AMSRUST_{}_{code:04X}", self.crtc as u8),
            (None, None) => format!("AMSRUST_{}", self.crtc as u8)
        };
        let count = self.taken.entry(stem.clone()).or_insert(0);
        let file = if *count == 0 {
            format!("{stem}.png")
        }
        else {
            format!("{stem}_{count}.png")
        };
        *count += 1;

        std::fs::create_dir_all(&self.screenshot_dir).map_err(|e| e.to_string())?;
        let path = self.screenshot_dir.join(file);
        frame.save(&path).map_err(|e| e.to_string())?;
        if self.verbose {
            eprintln!("[{:>9} µs] {}", self.elapsed_us(), path.display());
        }
        self.report.screenshots.push(Screenshot {
            path,
            ssm_code,
            crtc: self.crtc,
            at_us: self.elapsed_us()
        });
        Ok(())
    }
}

// ---------------------------------------------------------------- helpers

fn drive(unit: usize) -> FdcDrive {
    if unit == 0 {
        FdcDrive::A
    }
    else {
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
        CslCrtc::Type4 => CrtcType::Crtc4
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
    }
    else {
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
fn keys_of_char(c: char) -> Option<Vec<CpcKey>> {
    use CpcKey::*;
    let letter = |c: char| -> Option<CpcKey> {
        Some(match c.to_ascii_lowercase() {
            'a' => A, 'b' => B, 'c' => C, 'd' => D, 'e' => E, 'f' => F, 'g' => G,
            'h' => H, 'i' => I, 'j' => J, 'k' => K, 'l' => L, 'm' => M, 'n' => N,
            'o' => O, 'p' => P, 'q' => Q, 'r' => R, 's' => S, 't' => T, 'u' => U,
            'v' => V, 'w' => W, 'x' => X, 'y' => Y, 'z' => Z,
            _ => return None
        })
    };
    let digit = |d: u32| -> CpcKey {
        [Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9][d as usize]
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
        _ => return None
    })
}

fn keys_of_special(key: SpecialKey) -> Option<Vec<CpcKey>> {
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
        SpecialKey::LeftBrace => return keys_of_char('{'),
        SpecialKey::RightBrace => return keys_of_char('}'),
        SpecialKey::Backslash => return keys_of_char('\\'),
        SpecialKey::Quote => return keys_of_char('\''),
        SpecialKey::NoDelayNextKey => return None
    })
}

/// The keys of one element of a `key_output` (a group `{..}` is pressed together).
fn keys_of(element: &KeyElement) -> Option<Vec<CpcKey>> {
    match element {
        KeyElement::Character(c) => keys_of_char(*c),
        KeyElement::Special(key) => keys_of_special(*key),
        KeyElement::Simultaneous(elements) => {
            let mut keys = Vec::new();
            for element in elements {
                for key in keys_of(element)? {
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
            KeyElement::Special(SpecialKey::Shift)
        ]);
        assert_eq!(keys_of(&group), Some(vec![CpcKey::Shift, CpcKey::A]));
    }

    #[test]
    fn windows_paths_are_resolved_case_insensitively() {
        let base = Path::new("/base");
        assert_eq!(resolve(base, r"sub\SHAKE.CSL"), Path::new("/base/sub/SHAKE.CSL"));
        assert_eq!(resolve(base, r"Z:\tmp\a.dsk"), Path::new("/tmp/a.dsk"));
    }

    #[test]
    fn both_um6845r_names_are_crtc_1() {
        assert_eq!(crtc_type(CslCrtc::Type1A), CrtcType::Crtc1);
        assert_eq!(crtc_type(CslCrtc::Type1B), CrtcType::Crtc1);
        assert_eq!(crtc_type(CslCrtc::Type4), CrtcType::Crtc4);
    }
}
