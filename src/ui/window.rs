//! The windows of the interface: each open window is a [`ViewWindow`], its own state with it
//! (a Memory window its address, the symbols window its filter...), made by [`new_window`] from
//! its [`View`] (whose [`ViewInfo`](crate::commands::ViewInfo) says the rest: title, command,
//! copies, sizing).

use std::any::Any;
use std::collections::BTreeMap;

use super::code_views::{DisassemblyWindow, Link, MemoryWindow};
use super::views;
use crate::commands::{CommandBus, CommandTarget, View};
use crate::joystick::Joysticks;
use crate::keyboard::CpcKey;

/// What the windows give the machine, read by the host each frame: the simulated joysticks
/// and the virtual keyboard's keys.
#[derive(Default)]
pub struct VirtualInput {
    pub joysticks: Joysticks,
    /// CPC matrix keys held by the virtual keyboard.
    pub keys_held: Vec<CpcKey>,
    /// Ctrl is a mouse modifier while Ctrl-clicking a virtual Shift key, not CPC input.
    pub ctrl_click: bool,
}

/// What a window's content works with.
pub struct Cx<'a> {
    pub target: &'a dyn CommandTarget,
    pub bus: &'a mut CommandBus,
    /// What the window asks of the others (an address to disassemble...): the interface
    /// follows them after the frame.
    pub links: &'a mut Vec<Link>,
    pub input: &'a mut VirtualInput,
}

impl<'a> Cx<'a> {
    /// The parts most windows need, borrowed apart.
    pub fn parts(&mut self) -> (&'a dyn CommandTarget, &mut CommandBus, &mut Vec<Link>) {
        (self.target, self.bus, self.links)
    }
}

/// An open window: its state, and how it draws.
pub trait ViewWindow: Any {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx);
    /// Another window asks it to show `address` (a Memory or Disassembly window).
    fn go_to(&mut self, _address: String) {}
    /// It is closed: what it gave the machine is taken back.
    fn closed(&mut self, _input: &mut VirtualInput) {}
    /// Its own settings, for a workspace (see [`super::workspace`]).
    fn save(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    /// Its settings from a workspace.
    fn load(&mut self, _saved: &serde_json::Value) {}
}

/// A window with nothing of its own to remember.
struct Plain(fn(&mut egui::Ui, &mut Cx));

impl ViewWindow for Plain {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        (self.0)(ui, cx)
    }
}

/// A Memory window, its egui ids its own (several share a context on the web).
pub struct Memory {
    id: String,
    pub window: MemoryWindow,
}

impl ViewWindow for Memory {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        super::code_views::memory(ui, cx.target, cx.bus, &mut self.window, cx.links, &self.id);
    }
    fn go_to(&mut self, address: String) {
        self.window.go_to(address);
    }
    fn save(&self) -> serde_json::Value {
        self.window.save()
    }
    fn load(&mut self, saved: &serde_json::Value) {
        self.window.load(saved);
    }
}

pub struct Disassembly {
    id: String,
    pub window: DisassemblyWindow,
}

impl ViewWindow for Disassembly {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        super::code_views::disassembly(ui, cx.target, cx.bus, &mut self.window, cx.links, &self.id);
    }
    fn go_to(&mut self, address: String) {
        self.window.go_to(address);
    }
    fn save(&self) -> serde_json::Value {
        self.window.save()
    }
    fn load(&mut self, saved: &serde_json::Value) {
        self.window.load(saved);
    }
}

impl ViewWindow for super::screen::ScreenState {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        super::screen::show(ui, cx.target.motherboard(), self);
    }
}

/// Window `copy` of `view`, as it opens.
pub fn new_window(view: View, copy: u8) -> Box<dyn ViewWindow> {
    let plain = |draw: fn(&mut egui::Ui, &mut Cx)| -> Box<dyn ViewWindow> { Box::new(Plain(draw)) };
    match view {
        View::Z80 => Box::<views::Z80>::default(),
        View::Stack => plain(views::stack),
        View::Crtc => plain(views::crtc),
        View::GateArray => plain(views::gate_array),
        View::Psg => plain(views::psg),
        View::Fdc => plain(views::fdc),
        View::MemoryMap => plain(views::memory_map),
        View::Beam => plain(views::beam),
        View::History => plain(views::history),
        View::Shortcuts => plain(views::shortcuts),
        View::About => plain(views::about),
        View::Ppi => plain(|ui, cx| super::chips::ppi(ui, cx.target.motherboard())),
        View::Tape => plain(|ui, cx| super::chips::tape(ui, cx.target.motherboard(), cx.bus)),
        View::Asic => plain(|ui, cx| super::chips::asic(ui, cx.target.motherboard(), cx.bus)),
        View::CslLog => plain(|ui, cx| super::csl_log::csl_log(ui, cx.target, cx.bus)),
        View::Preferences => plain(|ui, cx| views::preferences(ui, cx.target, cx.bus)),
        // drawn by the interface itself
        View::Palette => plain(|_, _| {}),
        View::Screen => Box::<super::screen::ScreenState>::default(),
        View::Symbols => Box::<views::Symbols>::default(),
        View::Counters => Box::<views::Counters>::default(),
        View::Breakpoints => Box::<views::Breakpoints>::default(),
        View::Joystick => Box::<views::Joystick>::default(),
        View::Keyboard => Box::<views::Keyboard>::default(),
        View::Memory => Box::new(Memory {
            id: format!("memory-{copy}"),
            window: MemoryWindow::default(),
        }),
        View::Disassembly => Box::new(Disassembly {
            id: format!("disassembly-{copy}"),
            window: DisassemblyWindow::default(),
        }),
    }
}

/// The [`View`] in its key's low 5 bits: its place in [`View::ALL`].
const VIEW_BITS: u8 = 5;

/// The windows of one view at most (the key's 3 high bits).
pub const COPIES: u8 = 8;

/// A window: its view, and which of its windows (0: the first; a view whose
/// [`ViewInfo::copies`](crate::commands::ViewInfo::copies) opens several, [`COPIES`] at most).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ViewKey(pub u8);

impl ViewKey {
    pub fn of(view: View) -> Self {
        Self::copy(view, 0)
    }
    pub fn copy(view: View, copy: u8) -> Self {
        let index = View::ALL.iter().position(|v| *v == view).unwrap_or(0) as u8;
        Self(index | (copy.min(COPIES - 1) << VIEW_BITS))
    }
    pub fn view(self) -> View {
        View::ALL[usize::from(self.0 & ((1 << VIEW_BITS) - 1))]
    }
    pub fn copy_number(self) -> u8 {
        self.0 >> VIEW_BITS
    }
    /// Its title: the view's, numbered from the second ("Memory 2").
    pub fn title(self) -> String {
        let title = self.view().info().title;
        match self.copy_number() {
            0 => title.to_owned(),
            n => format!("{title} {}", n + 1),
        }
    }
}

/// The windows' states (kept when a window closes: it opens again as it was), what they asked
/// of the others, and what they give the machine.
#[derive(Default)]
pub struct Windows {
    windows: BTreeMap<ViewKey, Box<dyn ViewWindow>>,
    pub links: Vec<Link>,
    pub input: VirtualInput,
}

impl Windows {
    /// The window of `key`, made when it first opens.
    pub fn get(&mut self, key: ViewKey) -> &mut dyn ViewWindow {
        self.windows
            .entry(key)
            .or_insert_with(|| new_window(key.view(), key.copy_number()))
            .as_mut()
    }

    /// The settings of the windows made so far (open or closed since), for a workspace.
    pub fn saved(&self) -> Vec<(ViewKey, serde_json::Value)> {
        self.windows
            .iter()
            .map(|(key, window)| (*key, window.save()))
            .collect()
    }

    /// The window of `key` as it first opens (a new window, not the last one closed).
    pub fn reset(&mut self, key: ViewKey) -> &mut dyn ViewWindow {
        self.windows.remove(&key);
        self.get(key)
    }

    /// The window of `key` was closed.
    pub fn closed(&mut self, key: ViewKey) {
        // made if it never drew: what it gives the machine may have been set from outside
        let window = self
            .windows
            .entry(key)
            .or_insert_with(|| new_window(key.view(), key.copy_number()));
        window.closed(&mut self.input);
    }

    /// The state of the window of `key`, as its type (for the tests).
    #[cfg(test)]
    pub fn state<T: ViewWindow>(&mut self, key: ViewKey) -> Option<&T> {
        let window: &dyn Any = self.windows.get(&key)?.as_ref();
        window.downcast_ref()
    }

    /// Draw the content of the window of `key`.
    pub fn contents(
        &mut self,
        key: ViewKey,
        ui: &mut egui::Ui,
        target: &dyn CommandTarget,
        bus: &mut CommandBus,
    ) {
        let window = self
            .windows
            .entry(key)
            .or_insert_with(|| new_window(key.view(), key.copy_number()));
        let mut cx = Cx {
            target,
            bus,
            links: &mut self.links,
            input: &mut self.input,
        };
        window.ui(ui, &mut cx);
    }

    /// The window of `key` inside the main one (the web page; the preferences...): as large
    /// as its content, a long one scrolled, unless the user sizes it.
    pub fn show(
        &mut self,
        key: ViewKey,
        open: &mut bool,
        ctx: &egui::Context,
        target: &dyn CommandTarget,
        bus: &mut CommandBus,
    ) {
        let info = key.view().info();
        if info.title.is_empty() {
            return;
        }
        let title = key.title();
        let mut window = egui::Window::new(title.as_str())
            .open(open)
            .resizable(info.resizable.is_some());
        if let Some([width, height]) = info.resizable {
            window = window.default_size(egui::vec2(width, height));
        }
        window.show(ctx, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([true, true])
                .max_height(560.0)
                .max_width(760.0)
                .show(ui, |ui| self.contents(key, ui, target, bus));
        });
    }
}
