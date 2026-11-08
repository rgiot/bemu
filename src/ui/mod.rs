//! The user interface drawn over the picture (egui): the menu bar made from the command
//! registry, the command palette, the questions the commands ask, and the debugger's windows.
//!
//! Nothing here changes the machine: every item, button and field queues a
//! [`Command`](crate::commands::Command) on the [`CommandBus`], which the host runs between two
//! frames. The interface reads the machine through the [`CommandTarget`] to show the check
//! marks, the greyed items and the debugger's windows.
//!
//! The same code runs in the window (`frontend/winit_gpu.rs`, egui-winit for the input) and in
//! the web page (`web/`, its input forwarded from the page): [`Gui::run`] takes egui's input and
//! gives what to draw, [`Painter`] draws it with wgpu after the picture.

mod chips;
mod files;
mod menu;
mod paint;
mod palette;
mod prompt;
mod views;

pub use paint::Painter;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::commands::{
    self, Arg, ArgKind, CommandBus, CommandError, CommandTarget, HostRequest, Report, View,
};

/// A file the host is asked to pick (a web page opens its file chooser): the command `id` then
/// runs with it.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRequest {
    pub id: &'static str,
    pub title: &'static str,
    /// A file to write (a download), else to read.
    pub save: bool,
    pub extensions: &'static [&'static str],
    pub default_name: &'static str,
}

/// A message shown for a while at the bottom of the picture.
struct Toast {
    text: String,
    error: bool,
    until: f64,
}

/// The user interface's state.
pub struct Gui {
    ctx: egui::Context,
    palette: Option<palette::Palette>,
    prompt: Option<prompt::Prompt>,
    views: BTreeSet<ViewKey>,
    view_state: views::ViewState,
    texts: Vec<(String, String)>,
    toasts: VecDeque<Toast>,
    file_requests: Vec<FileRequest>,
    menu_height: f32,
    /// The debugger's windows are windows of the system (the desktop), each with its own
    /// context, not windows drawn inside the main one (the web page).
    native_views: bool,
    view_contexts: BTreeMap<ViewKey, egui::Context>,
}

/// A debugger's window of the system: its key, title and context (see [`Gui::native_views`]).
pub struct NativeView {
    pub key: u8,
    pub title: &'static str,
    pub ctx: egui::Context,
}

fn new_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut(|style| {
        style.interaction.tooltip_delay = 0.4;
    });
    ctx
}

/// [`View`] in a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ViewKey(u8);

const ALL_VIEWS: [View; 20] = [
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
    View::Asic,
    View::Preferences,
    View::Shortcuts,
    View::About,
];

impl ViewKey {
    fn of(view: View) -> Self {
        Self(ALL_VIEWS.iter().position(|v| *v == view).unwrap_or(0) as u8)
    }
    fn view(self) -> View {
        ALL_VIEWS[usize::from(self.0)]
    }
}

impl Default for Gui {
    fn default() -> Self {
        Self::new()
    }
}

impl Gui {
    pub fn new() -> Self {
        let ctx = new_context();
        Self {
            ctx,
            palette: None,
            prompt: None,
            views: BTreeSet::new(),
            view_state: views::ViewState::default(),
            texts: Vec::new(),
            toasts: VecDeque::new(),
            file_requests: Vec::new(),
            menu_height: 0.0,
            native_views: false,
            view_contexts: BTreeMap::new(),
        }
    }

    /// Open the debugger's windows as windows of the system (a host that can open several).
    pub fn set_native_views(&mut self, native: bool) {
        self.native_views = native;
    }

    /// The debugger's windows open, when they are windows of the system: the host opens one
    /// for each (and closes the others), then runs [`Self::run_view`] with its input.
    pub fn native_views(&mut self) -> Vec<NativeView> {
        if !self.native_views {
            return Vec::new();
        }
        let open: BTreeSet<ViewKey> = self
            .views
            .iter()
            .copied()
            .filter(|key| views::is_debugger(key.view()))
            .collect();
        self.view_contexts.retain(|key, _| open.contains(key));
        open.into_iter()
            .map(|key| NativeView {
                key: key.0,
                title: views::title(key.view()),
                ctx: self
                    .view_contexts
                    .entry(key)
                    .or_insert_with(new_context)
                    .clone(),
            })
            .collect()
    }

    /// The window of the system of view `key` was closed.
    pub fn close_view(&mut self, key: u8) {
        self.views.remove(&ViewKey(key));
        self.view_contexts.remove(&ViewKey(key));
    }

    /// Lay out one frame of the debugger's window `key` (a window of the system), and the size
    /// its content wants, in points (the window takes it).
    pub fn run_view(
        &mut self,
        key: u8,
        raw: egui::RawInput,
        target: &dyn CommandTarget,
        bus: &mut CommandBus,
    ) -> Option<(egui::FullOutput, egui::Vec2)> {
        let key = ViewKey(key);
        let ctx = self.view_contexts.get(&key)?.clone();
        let state = &mut self.view_state;
        let mut wanted = egui::Vec2::ZERO;
        let output = ctx.run(raw, |ctx| {
            let panel = egui::CentralPanel::default().show(ctx, |ui| {
                // both ways: the lines keep their length (no wrapping to the window's width),
                // so the content says how wide the window is to be
                egui::ScrollArea::both()
                    .auto_shrink([true, true])
                    .show(ui, |ui| {
                        views::contents(key.view(), ui, target, bus, state);
                    })
                    .content_size
            });
            // with the panel's margins
            wanted = panel.inner
                + egui::Frame::central_panel(&ctx.style())
                    .total_margin()
                    .sum();
        });
        Some((output, wanted))
    }

    pub fn context(&self) -> &egui::Context {
        &self.ctx
    }

    /// The host's request the interface answers itself (its windows): `true` when it did.
    pub fn handle(&mut self, request: &HostRequest) -> bool {
        match request {
            HostRequest::Open(View::Palette) => {
                self.palette = Some(palette::Palette::default());
                true
            }
            HostRequest::Open(view) => {
                self.views.insert(ViewKey::of(*view));
                true
            }
            HostRequest::ShowText { title, text } => {
                self.texts.push((title.clone(), text.clone()));
                true
            }
            _ => false,
        }
    }

    /// Whether the interface answers `request` (for [`CommandTarget::can`]).
    pub fn can(request: &HostRequest) -> bool {
        matches!(request, HostRequest::Open(_) | HostRequest::ShowText { .. })
    }

    /// Show a message for a few seconds.
    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        let now = self.ctx.input(|i| i.time);
        self.toasts.push_back(Toast {
            text: text.into(),
            error,
            until: now + if error { 6.0 } else { 3.0 },
        });
        while self.toasts.len() > 4 {
            self.toasts.pop_front();
        }
    }

    /// Show what the commands reported (their messages, their errors).
    pub fn report(&mut self, reports: impl Iterator<Item = Report>) {
        for report in reports {
            match report.result {
                Ok(Some(message)) => self.notify(message, false),
                Ok(None) => {}
                Err(CommandError::Unsupported) => {
                    self.notify(format!("{}: not available here", report.id), true)
                }
                Err(error) => self.notify(format!("{}: {error}", report.id), true),
            }
        }
    }

    /// The files the host is to pick for the commands (where the interface has no file
    /// browser: a web page).
    pub fn take_file_requests(&mut self) -> Vec<FileRequest> {
        std::mem::take(&mut self.file_requests)
    }

    /// Whether the keys go to the interface (a field, the palette, a menu) rather than the CPC.
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.wants_keyboard_input()
            || self
                .view_contexts
                .values()
                .any(egui::Context::wants_keyboard_input)
            || self.palette.is_some()
            || self.prompt.is_some()
            || self.ctx.is_popup_open()
    }

    /// Whether the pointer is over the interface.
    pub fn wants_pointer(&self) -> bool {
        self.ctx.wants_pointer_input()
    }

    /// The height of the menu bar, in points (0 without it): the picture goes below.
    pub fn menu_height(&self) -> f32 {
        self.menu_height
    }

    /// Lay out one frame of the interface. Its commands are queued on `bus`.
    pub fn run(
        &mut self,
        raw: egui::RawInput,
        target: &dyn CommandTarget,
        bus: &mut CommandBus,
    ) -> egui::FullOutput {
        let ctx = self.ctx.clone();
        ctx.run(raw, |ctx| self.ui(ctx, target, bus))
    }

    fn ui(&mut self, ctx: &egui::Context, target: &dyn CommandTarget, bus: &mut CommandBus) {
        self.shortcuts(ctx, target, bus);
        // a command run from the palette asks its argument there (a file: in the browser)
        if let Some(id) = self.palette.as_ref().and_then(palette::Palette::waiting) {
            match bus.prompt() {
                Some(question) if question.id == id && palette::Palette::asks(&question.kind) => {
                    let question = bus.take_prompt().expect("a prompt");
                    if let Some(palette) = &mut self.palette {
                        palette.ask(question);
                    }
                }
                _ => self.palette = None,
            }
        }
        // a command asks its argument
        if self.prompt.is_none() {
            if let Some(question) = bus.take_prompt() {
                match (&question.kind, target.has_file_system()) {
                    (ArgKind::OpenFile(extensions), false) => {
                        self.file_requests.push(FileRequest {
                            id: question.id,
                            title: question.title,
                            save: false,
                            extensions,
                            default_name: "",
                        })
                    }
                    (ArgKind::SaveFile(extensions, name), false) => {
                        // a page downloads what it saves: the name is enough
                        bus.push(
                            commands::Command::with_arg(question.id, Arg::Text((*name).to_owned()))
                                .expect("a command of the registry"),
                        );
                        let _ = extensions;
                    }
                    (ArgKind::Folder, false) => {
                        self.notify(format!("{}: no folders here", question.title), true)
                    }
                    _ => self.prompt = Some(prompt::Prompt::new(question)),
                }
            }
        }
        let visible = target.settings().is_none_or(|s| s.menu_visible);
        self.menu_height = if visible {
            egui::TopBottomPanel::top("menu")
                .show(ctx, |ui| menu::bar(ui, target, bus))
                .response
                .rect
                .height()
        } else {
            0.0
        };
        if let Some(palette) = &mut self.palette {
            let recent: Vec<_> = bus.recent().collect();
            if !palette.show(ctx, target, bus, &recent) {
                self.palette = None;
            }
        }
        if let Some(prompt) = &mut self.prompt {
            match prompt.show(ctx, target) {
                prompt::Answer::Waiting => {}
                prompt::Answer::Cancelled => {
                    self.prompt = None;
                }
                prompt::Answer::Given(id, arg) => {
                    if let Ok(command) = commands::Command::with_arg(id, arg) {
                        bus.push(command);
                    }
                    self.prompt = None;
                }
            }
        }
        let mut closed = Vec::new();
        let embedded = if self.native_views {
            // the palette, the preferences... stay inside: they are not the debugger's
            self.views
                .iter()
                .copied()
                .filter(|key| !views::is_debugger(key.view()))
                .collect()
        } else {
            self.views.clone()
        };
        for key in embedded {
            let mut open = true;
            views::show(
                key.view(),
                &mut open,
                ctx,
                target,
                bus,
                &mut self.view_state,
            );
            if !open {
                closed.push(key);
            }
        }
        for key in closed {
            self.views.remove(&key);
        }
        self.texts.retain_mut(|(title, text)| {
            let mut open = true;
            egui::Window::new(title.as_str())
                .open(&mut open)
                .default_width(420.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(
                                egui::RichText::new(text.as_str()).monospace(),
                            ));
                        });
                });
            open
        });
        self.toasts(ctx);
    }

    /// The chords with Ctrl or Alt (the palette's Ctrl+Shift+P, Ctrl+V): the host runs the
    /// function keys itself, the window having no keyboard focus for them.
    fn shortcuts(&mut self, ctx: &egui::Context, target: &dyn CommandTarget, bus: &mut CommandBus) {
        // Ctrl+V comes as a paste (egui reads the clipboard): typed on the CPC, unless a field
        // of the interface has the keyboard
        if !ctx.wants_keyboard_input() {
            let pasted = ctx.input_mut(|i| {
                let mut pasted = None;
                i.events.retain(|event| match event {
                    egui::Event::Paste(text) => {
                        pasted = Some(text.clone());
                        false
                    }
                    _ => true,
                });
                pasted
            });
            if let Some(text) = pasted.filter(|text| !text.is_empty()) {
                if let Ok(command) = commands::Command::with_arg("autotype-text", Arg::Text(text)) {
                    bus.push(command);
                }
            }
        }
        for spec in commands::all() {
            let Some(chord) = spec.shortcut.and_then(commands::Chord::parse) else {
                continue;
            };
            if !(chord.ctrl || chord.alt) || chord.key.len() != 1 {
                continue;
            }
            let Some(key) = egui::Key::from_name(chord.key) else {
                continue;
            };
            // Ctrl, or Cmd on macOS
            let modifiers = egui::Modifiers {
                alt: chord.alt,
                ctrl: false,
                shift: chord.shift,
                command: chord.ctrl,
                mac_cmd: false,
            };
            if ctx.input_mut(|i| i.consume_key(modifiers, key)) {
                if let Some(id) = commands::shortcut_command(chord, target) {
                    bus.push_id(id);
                }
            }
        }
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.toasts.retain(|toast| toast.until > now);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -12.0])
            .interactable(false)
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let text = egui::RichText::new(&toast.text);
                        ui.label(if toast.error {
                            text.color(ui.visuals().error_fg_color)
                        } else {
                            text
                        });
                    });
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandBus, CommandTarget, HostSettings};
    use crate::debug_server::DebugServer;
    use crate::motherboard::Motherboard;

    struct Target(Motherboard, DebugServer, HostSettings);

    impl CommandTarget for Target {
        fn motherboard(&self) -> &Motherboard {
            &self.0
        }
        fn motherboard_mut(&mut self) -> &mut Motherboard {
            &mut self.0
        }
        fn debugger(&self) -> Option<&DebugServer> {
            Some(&self.1)
        }
        fn settings(&self) -> Option<&HostSettings> {
            Some(&self.2)
        }
        fn has_file_system(&self) -> bool {
            false
        }
    }

    /// A frame as the web page gives it: no window size from a backend, its own clock.
    fn frame(gui: &mut Gui, target: &Target, bus: &mut CommandBus, time: f64) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1068.0, 756.0),
            )),
            time: Some(time),
            focused: true,
            ..Default::default()
        };
        gui.run(raw, target, bus);
    }

    #[test]
    fn a_window_opened_stays_open() {
        let target = Target(
            Motherboard::default(),
            DebugServer::offline(),
            HostSettings::default(),
        );
        let mut bus = CommandBus::default();
        let mut gui = Gui::new();
        for view in [View::Disassembly, View::About, View::Z80] {
            frame(&mut gui, &target, &mut bus, 0.0);
            gui.handle(&HostRequest::Open(view));
            for n in 1..5 {
                frame(&mut gui, &target, &mut bus, f64::from(n) * 0.02);
            }
            assert_eq!(gui.views.len(), 1, "{view:?}");
            gui.views.clear();
        }
    }

    #[test]
    fn every_window_draws_on_a_cpc_and_a_cpc_plus() {
        for model in [
            crate::motherboard::CpcModel::Cpc6128,
            crate::motherboard::CpcModel::Cpc6128Plus,
        ] {
            let mut builder = crate::motherboard::MotherboardBuilder::default();
            builder.set_model(model);
            let mut motherboard = builder.build();
            for _ in 0..200_000 {
                motherboard.execute_next_opcode_no_video();
            }
            let target = Target(motherboard, DebugServer::offline(), HostSettings::default());
            let mut bus = CommandBus::default();
            let mut gui = Gui::new();
            for view in ALL_VIEWS {
                frame(&mut gui, &target, &mut bus, 0.0);
                gui.handle(&HostRequest::Open(view));
                frame(&mut gui, &target, &mut bus, 0.02);
                gui.views.clear();
            }
        }
    }
}
