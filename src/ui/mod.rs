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

mod files;
mod menu;
mod paint;
mod palette;
mod prompt;
mod views;

pub use paint::Painter;

use std::collections::{BTreeSet, VecDeque};

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
}

/// [`View`] in a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ViewKey(u8);

const ALL_VIEWS: [View; 17] = [
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
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Dark);
        ctx.style_mut(|style| {
            style.interaction.tooltip_delay = 0.4;
        });
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
        }
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
        for key in self.views.iter().copied() {
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
            let modifiers = egui::Modifiers {
                alt: chord.alt,
                ctrl: chord.ctrl,
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
