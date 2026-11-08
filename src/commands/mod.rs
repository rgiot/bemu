//! The emulator's commands: everything the menus, the command palette, the shortcuts and the
//! debug server can ask the emulator to do, in one place.
//!
//! The Command pattern, the Rust way:
//!
//! * [`CommandSpec`] - the registry, generated from `doc/menu-ui.csv` (the menu specification)
//!   by `build.rs`: an id (`machine-crtc-1`), its place in the menus, its kind (action, toggle,
//!   radio, input...), its shortcut and what it does.
//! * [`Command`] - the command object: an id and its argument. It is what a menu item, the
//!   palette, a key or a debug server route creates, queues on the [`CommandBus`] (the invoker)
//!   and later [executes](Command::execute) on a [`CommandTarget`] (the receiver).
//! * [`CommandTarget`] - what a command acts on: the machine, the debugger, and the host's side
//!   (the window or the web page: the display, the sound, files...). The desktop window, the web
//!   page and the debug server each implement it; a host that cannot do something (the debug
//!   server cannot show a window) answers [`CommandError::Unsupported`], and the command shows
//!   disabled there.
//!
//! The machine changes made by commands keep the machine running: a CRTC of another type takes
//! over the registers and counters of the previous one, the RAM keeps its content when it grows,
//! the cards are plugged and unplugged with the power on.

mod config;
mod execute;
mod host;
pub mod palette;

pub use execute::{state, MACHINE_RAM_SIZES};
pub use host::{
    DeviceFiles, HostRequest, HostSettings, MediaPaths, Recording, Speed, TypingDelays, View,
    DEFAULT_DEBUG_PORT,
};

use std::collections::VecDeque;
use std::path::PathBuf;

use crate::debug_server::DebugServer;
use crate::motherboard::Motherboard;

/// How a command shows in a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Does something at once.
    Action,
    /// A checkable item: on or off.
    Toggle,
    /// One of a group of exclusive choices (the items of the same submenu).
    Radio,
    /// Needs an argument (a file, an address, a value): asked when not given.
    Input,
    /// Starts something that runs until its stop command (a recording).
    StartStop,
    /// A submenu filled at run time (recent discs).
    Submenu,
}

/// A command of the registry (one row of `doc/menu-ui.csv`).
#[derive(Debug)]
pub struct CommandSpec {
    /// The command's name, `area-verb`, as the palette matches it.
    pub id: &'static str,
    /// Its place in the menus, the last element being the item's label.
    pub path: &'static [&'static str],
    pub kind: Kind,
    /// The command that stops what this one started.
    pub stop_id: Option<&'static str>,
    /// Its key chord, as written in the menu (`F5`, `Shift+F5`, `Ctrl+Shift+P`).
    pub shortcut: Option<&'static str>,
    /// What it is for.
    pub aim: &'static str,
    /// When it is available, what it asks.
    pub note: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/commands_table.rs"));

impl CommandSpec {
    /// The item's label: the last element of its menu path.
    pub fn label(&self) -> &'static str {
        self.path.last().copied().unwrap_or(self.id)
    }

    /// The menus leading to the item.
    pub fn menus(&self) -> &'static [&'static str] {
        &self.path[..self.path.len().saturating_sub(1)]
    }

    /// The radio group: the radio items of the same submenu are exclusive.
    pub fn group(&self) -> &'static [&'static str] {
        self.menus()
    }

    /// The item shows in a menu: radio items of a submenu, others as they come.
    pub fn in_menu(&self) -> bool {
        // the palette opens from its shortcut, and from its own menu item
        true
    }
}

/// Every command, in menu order.
pub fn all() -> &'static [CommandSpec] {
    COMMANDS
}

/// The command named `id`, or the start command `id` stops.
pub fn spec(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .iter()
        .find(|spec| spec.id == id || spec.stop_id == Some(id))
}

/// A command's argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    /// Text: what to type, an address or symbol, a value.
    Text(String),
    /// A number (a value, a slot, a port).
    Number(i64),
    /// A file or a folder of the host.
    Path(PathBuf),
    /// A file's content, where there is no file system (a web page): its name and bytes.
    Bytes { name: String, data: Vec<u8> },
}

impl Arg {
    /// The argument as text (a number as decimal, a path as given).
    pub fn text(&self) -> String {
        match self {
            Arg::Text(text) => text.clone(),
            Arg::Number(n) => n.to_string(),
            Arg::Path(path) => path.display().to_string(),
            Arg::Bytes { name, .. } => name.clone(),
        }
    }

    /// The argument as a number (decimal, `&`/`#`/`$`/`0x` hexadecimal).
    pub fn number(&self) -> Result<i64, CommandError> {
        match self {
            Arg::Number(n) => Ok(*n),
            other => {
                let text = other.text();
                let t = text.trim();
                let hex = t
                    .strip_prefix('&')
                    .or_else(|| t.strip_prefix('#'))
                    .or_else(|| t.strip_prefix('$'))
                    .or_else(|| t.strip_prefix("0x"))
                    .or_else(|| t.strip_prefix("0X"));
                match hex {
                    Some(hex) => i64::from_str_radix(hex, 16),
                    None => t.parse(),
                }
                .map_err(|_| CommandError::Invalid(format!("{t}: not a number")))
            }
        }
    }

    /// The file's name and content.
    pub fn bytes(&self) -> Result<(String, Vec<u8>), CommandError> {
        match self {
            Arg::Bytes { name, data } => Ok((name.clone(), data.clone())),
            Arg::Path(path) => std::fs::read(path)
                .map(|data| (path.display().to_string(), data))
                .map_err(|e| CommandError::Failed(format!("{}: {e}", path.display()))),
            Arg::Text(text) => Arg::Path(PathBuf::from(text)).bytes(),
            Arg::Number(_) => Err(CommandError::Invalid("a file is expected".into())),
        }
    }

    /// A path of the host (not a file given as bytes).
    pub fn path(&self) -> Result<PathBuf, CommandError> {
        match self {
            Arg::Path(path) => Ok(path.clone()),
            Arg::Text(text) => Ok(PathBuf::from(text)),
            _ => Err(CommandError::Unsupported),
        }
    }
}

/// What a command asks when it was given no argument.
#[derive(Clone, Debug, PartialEq)]
pub enum ArgKind {
    /// A file to read, with the extensions it may have.
    OpenFile(&'static [&'static str]),
    /// A file to write, with its extensions and a default name.
    SaveFile(&'static [&'static str], &'static str),
    /// A folder of the host.
    Folder,
    /// A line of text.
    Text,
    /// A CPU address, or a symbol.
    Address,
    /// A number between two bounds, with a default.
    Number { min: i64, max: i64, default: i64 },
    /// A decimal number between two bounds, with a default (the curvature: 1.5).
    Decimal { min: f64, max: f64, default: f64 },
    /// One of a list of values (`(value, label)`).
    Choice(&'static [(&'static str, &'static str)]),
}

/// The question asked for an argument.
#[derive(Clone, Debug, PartialEq)]
pub struct ArgPrompt {
    pub id: &'static str,
    pub title: &'static str,
    pub kind: ArgKind,
}

/// Why a command did not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    /// No command of that name.
    Unknown(String),
    /// This host cannot do it (the debug server has no window, a page no folders...).
    Unsupported,
    /// The emulator cannot do it yet.
    NotImplemented,
    /// Not now (no disc to remove, not paused...).
    Unavailable(String),
    /// The argument is wrong.
    Invalid(String),
    /// It failed (a file that cannot be read...).
    Failed(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Unknown(id) => write!(f, "{id}: no such command"),
            CommandError::Unsupported => write!(f, "not available here"),
            CommandError::NotImplemented => write!(f, "not implemented yet"),
            CommandError::Unavailable(why)
            | CommandError::Invalid(why)
            | CommandError::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for CommandError {}

/// What a command did.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Done; with a message to show.
    Done(Option<String>),
    /// It needs an argument: ask it, then run the command again with it.
    NeedsArg(ArgPrompt),
}

pub type CommandResult = Result<Outcome, CommandError>;

/// How a command shows in the menus now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandState {
    /// It can run now.
    pub enabled: bool,
    /// Its check mark (a toggle on, the radio item selected, a recording running).
    pub checked: bool,
}

impl CommandState {
    pub const DISABLED: Self = Self {
        enabled: false,
        checked: false,
    };

    pub fn enabled(enabled: bool) -> Self {
        Self {
            enabled,
            checked: false,
        }
    }

    pub fn checked(checked: bool) -> Self {
        Self {
            enabled: true,
            checked,
        }
    }
}

/// The receiver of the commands: what they act on.
///
/// Only the machine is required; the rest is the host's (a window, a web page, the debug
/// server), each default saying the host has none.
pub trait CommandTarget {
    fn motherboard(&self) -> &Motherboard;
    fn motherboard_mut(&mut self) -> &mut Motherboard;

    /// The debugger: breakpoints, stepping, history, counters.
    fn debugger(&self) -> Option<&DebugServer> {
        None
    }
    fn debugger_mut(&mut self) -> Option<&mut DebugServer> {
        None
    }
    /// Both at once, for the commands that need the two.
    fn debugger_and_motherboard(&mut self) -> Option<(&mut DebugServer, &mut Motherboard)> {
        None
    }

    /// The host's settings: display, sound, speed... (`None`: a host without them).
    fn settings(&self) -> Option<&HostSettings> {
        None
    }
    fn settings_mut(&mut self) -> Option<&mut HostSettings> {
        None
    }

    /// Ask the host for what only it can do (a window, a file dialog, a recording...).
    fn request(&mut self, request: HostRequest) -> Result<(), CommandError> {
        let _ = request;
        Err(CommandError::Unsupported)
    }
    /// Whether the host can do `request`'s kind of thing (for the menus' greying).
    fn can(&self, request: &HostRequest) -> bool {
        let _ = request;
        false
    }
    /// Whether `recording` is running.
    fn is_recording(&self, recording: Recording) -> bool {
        let _ = recording;
        false
    }
    /// The host reads and writes files on a disk (desktop), not only bytes (web page).
    fn has_file_system(&self) -> bool {
        cfg!(not(target_arch = "wasm32"))
    }
    /// The CSL script running, if any.
    fn csl_running(&self) -> bool {
        false
    }

    /// Type `text` on the CPC's keyboard (Tools > Auto-type, CSL's `key_output`; `\(RET)`,
    /// `\(WAIT n)`... as CSL writes them), at the pace of the settings' typing delays. By default
    /// the debugger types it; a CSL player types it on its own clock.
    fn type_text(&mut self, text: &str) -> Result<(), CommandError> {
        let layout = self.motherboard().keyboard_layout();
        let keys = crate::csl_runner::keys_of_text(text, layout).map_err(CommandError::Invalid)?;
        let typing = self.settings().and_then(|s| s.typing);
        let debugger = self.debugger_mut().ok_or(CommandError::Unsupported)?;
        if let Some(typing) = typing {
            debugger.set_key_delays(typing.press_us, typing.after_key_us);
        }
        debugger.type_keys(keys);
        Ok(())
    }

    /// Switch the machine off and on (Machine > Power cycle, CSL's `reset`): RAM cleared. A
    /// CSL player builds the machine again from its configuration.
    fn power_cycle(&mut self) {
        self.motherboard_mut().power_cycle();
    }
}

/// A command object: what to run, with its argument.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub id: &'static str,
    pub arg: Option<Arg>,
}

impl Command {
    /// The command named `id` (a start or stop command of the registry).
    pub fn new(id: &str) -> Result<Self, CommandError> {
        let spec = spec(id).ok_or_else(|| CommandError::Unknown(id.to_owned()))?;
        // keep the static name
        let id = if spec.id == id {
            spec.id
        } else {
            spec.stop_id.unwrap()
        };
        Ok(Self { id, arg: None })
    }

    pub fn with_arg(id: &str, arg: Arg) -> Result<Self, CommandError> {
        Ok(Self {
            arg: Some(arg),
            ..Self::new(id)?
        })
    }

    pub fn spec(&self) -> &'static CommandSpec {
        spec(self.id).expect("a command of the registry")
    }

    /// Run the command on `target`.
    pub fn execute(self, target: &mut dyn CommandTarget) -> CommandResult {
        execute::execute(self, target)
    }
}

/// What a command reported, for the host to show (a status line, a toast, the console).
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub id: &'static str,
    pub result: Result<Option<String>, CommandError>,
}

/// The invoker: the commands asked by the menus, the palette and the keys, run by the
/// emulation loop between two frames (never while the menus are drawn), the prompts for the
/// arguments they need, and the recent ones (for the palette).
#[derive(Default)]
pub struct CommandBus {
    queue: VecDeque<Command>,
    prompt: Option<ArgPrompt>,
    recent: VecDeque<&'static str>,
    reports: VecDeque<Report>,
}

/// How many commands the palette remembers.
const RECENT: usize = 16;

impl CommandBus {
    pub fn push(&mut self, command: Command) {
        self.queue.push_back(command);
    }

    /// Queue the command named `id` (an unknown name is reported).
    pub fn push_id(&mut self, id: &str) {
        match Command::new(id) {
            Ok(command) => self.push(command),
            Err(error) => self.reports.push_back(Report {
                id: "",
                result: Err(error),
            }),
        }
    }

    /// Run the queued commands. A command that needs an argument leaves its prompt for the
    /// host to ask.
    pub fn run(&mut self, target: &mut dyn CommandTarget) {
        while let Some(command) = self.queue.pop_front() {
            let id = command.id;
            let result = command.execute(target);
            self.recent.retain(|recent| *recent != id);
            self.recent.push_front(id);
            self.recent.truncate(RECENT);
            match result {
                Ok(Outcome::NeedsArg(prompt)) => self.prompt = Some(prompt),
                Ok(Outcome::Done(message)) => self.reports.push_back(Report {
                    id,
                    result: Ok(message),
                }),
                Err(error) => self.reports.push_back(Report {
                    id,
                    result: Err(error),
                }),
            }
        }
    }

    /// The argument to ask, if a command needs one.
    pub fn prompt(&self) -> Option<&ArgPrompt> {
        self.prompt.as_ref()
    }

    pub fn take_prompt(&mut self) -> Option<ArgPrompt> {
        self.prompt.take()
    }

    /// The prompt answered: the command is queued again with its argument.
    pub fn answer(&mut self, arg: Arg) {
        if let Some(prompt) = self.prompt.take() {
            if let Ok(command) = Command::with_arg(prompt.id, arg) {
                self.push(command);
            }
        }
    }

    /// The prompt cancelled.
    pub fn cancel_prompt(&mut self) {
        self.prompt = None;
    }

    /// The commands run lately, the last first.
    pub fn recent(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.recent.iter().copied()
    }

    /// What the commands reported since the last call.
    pub fn take_reports(&mut self) -> impl Iterator<Item = Report> + '_ {
        self.reports.drain(..)
    }

    pub fn is_idle(&self) -> bool {
        self.queue.is_empty()
    }
}

/// A key chord of a shortcut: modifiers and a key name (`F1`-`F12`, `A`-`Z`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord<'a> {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: &'a str,
}

impl<'a> Chord<'a> {
    pub fn parse(text: &'a str) -> Option<Self> {
        let mut chord = Chord {
            ctrl: false,
            shift: false,
            alt: false,
            key: "",
        };
        for part in text.split('+') {
            match part {
                "Ctrl" => chord.ctrl = true,
                "Shift" => chord.shift = true,
                "Alt" => chord.alt = true,
                key => chord.key = key,
            }
        }
        (!chord.key.is_empty()).then_some(chord)
    }
}

/// The command a key chord runs now: of the commands sharing the chord (F8 runs and pauses,
/// F10 switches between the normal and the full speed), the first one enabled and not already
/// checked.
pub fn shortcut_command(chord: Chord, target: &dyn CommandTarget) -> Option<&'static str> {
    let mut candidates = COMMANDS
        .iter()
        .filter(|spec| spec.shortcut.and_then(Chord::parse) == Some(chord))
        .peekable();
    candidates.peek()?;
    let candidates: Vec<_> = candidates.collect();
    candidates
        .iter()
        .find(|spec| {
            let state = state(spec.id, target);
            state.enabled && !state.checked
        })
        .or_else(|| {
            candidates
                .iter()
                .find(|spec| state(spec.id, target).enabled)
        })
        .map(|spec| spec.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_is_the_specification() {
        let csv = include_str!("../../doc/menu-ui.csv");
        // every command of the specification is registered, once
        let rows = csv
            .lines()
            .filter(|line| line.contains(",drive-a-"))
            .count();
        assert!(rows >= 9, "{rows}");
        assert!(COMMANDS.len() > 200, "{}", COMMANDS.len());
        let mut ids: Vec<_> = COMMANDS.iter().map(|spec| spec.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), COMMANDS.len());
        let crtc1 = spec("machine-crtc-1").unwrap();
        assert_eq!(crtc1.path, ["Machine", "CRTC", "CRTC 1"]);
        assert_eq!(crtc1.kind, Kind::Radio);
        assert_eq!(
            spec("drive-b-flip-disc").unwrap().shortcut,
            Some("Shift+F5")
        );
        assert_eq!(spec("exit").unwrap().shortcut, Some("F4"));
        // a note is not a shortcut
        assert_eq!(spec("video-rendering-raw").unwrap().shortcut, None);
        assert_eq!(spec("wav-stop").unwrap().id, "wav-start");
    }

    #[test]
    fn every_command_has_a_behaviour() {
        for spec in COMMANDS {
            assert!(
                execute::is_known(spec.id),
                "{}: no behaviour (see src/commands/execute.rs)",
                spec.id
            );
        }
    }

    #[test]
    fn numbers_in_every_base() {
        assert_eq!(Arg::Text("&BE00".into()).number(), Ok(0xbe00));
        assert_eq!(Arg::Text("#38".into()).number(), Ok(0x38));
        assert_eq!(Arg::Text("0x10".into()).number(), Ok(16));
        assert_eq!(Arg::Text("42".into()).number(), Ok(42));
        assert!(Arg::Text("x".into()).number().is_err());
    }
}
