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

mod breakpoints;
mod chips;
mod code_views;
mod csl_log;
mod files;
mod heat;
mod menu;
mod paging;
mod paint;
mod palette;
mod prompt;
mod psg;
mod screen;
mod views;
mod widgets;
mod window;
pub mod workspace;

pub use paint::Painter;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::commands::{
    self, Arg, ArgKind, CommandBus, CommandError, CommandTarget, HostRequest, Report, View,
};
use crate::joystick::Joysticks;
use crate::keyboard::CpcKey;
use window::{ViewKey, Windows, COPIES};

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
    windows: Windows,
    texts: Vec<(String, String)>,
    toasts: VecDeque<Toast>,
    file_requests: Vec<FileRequest>,
    menu_height: f32,
    /// The machine was paused at the last frame: a pause (the button, F8, a breakpoint) shows
    /// the Disassembly window.
    was_paused: bool,
    /// When the pointer was last used (moved, clicked, scrolled): in fullscreen the bars hide
    /// [`BARS_HIDE_S`] after it.
    pointer_seen: f64,
    /// The status bar's height, in points (0 without it): the picture goes above.
    status_height: f32,
    /// The debugger's windows are windows of the system (the desktop), each with its own
    /// context, not windows drawn inside the main one (the web page).
    native_views: bool,
    view_contexts: BTreeMap<ViewKey, egui::Context>,
    /// The windows to bring to the front: the embedded ones at the next frame, the system's
    /// by the host ([`Self::take_focus_requests`]).
    focus: Vec<ViewKey>,
    /// Where the windows of the system are (as the host last said, or a workspace).
    geometries: BTreeMap<ViewKey, workspace::Geometry>,
}

/// A debugger's window of the system: its key, title and context (see [`Gui::native_views`]).
#[derive(Clone)]
pub struct NativeView {
    pub key: u8,
    pub title: String,
    /// Sized by the user from this size (points), else to its content.
    pub resizable: Option<[f32; 2]>,
    /// Where it was (a workspace): the host opens it there.
    pub geometry: Option<workspace::Geometry>,
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
            windows: Windows::default(),
            texts: Vec::new(),
            toasts: VecDeque::new(),
            file_requests: Vec::new(),
            menu_height: 0.0,
            pointer_seen: 0.0,
            was_paused: false,
            status_height: 0.0,
            native_views: false,
            view_contexts: BTreeMap::new(),
            focus: Vec::new(),
            geometries: BTreeMap::new(),
        }
    }

    /// Where the window of the system of view `key` is now (the host says, each frame).
    pub fn note_geometry(&mut self, key: u8, geometry: workspace::Geometry) {
        self.geometries.insert(ViewKey(key), geometry);
    }

    /// The workspace: the windows open, where they are, their settings (and those of the
    /// windows closed during the session, for when they open again).
    pub fn workspace(&self) -> workspace::Workspace {
        let mut keys: BTreeMap<ViewKey, serde_json::Value> =
            self.windows.saved().into_iter().collect();
        for key in &self.views {
            keys.entry(*key).or_default();
        }
        let windows = keys
            .into_iter()
            .filter(|(key, _)| key.view() != View::Palette)
            .map(|(key, settings)| workspace::SavedWindow {
                view: key.view().info().command.to_owned(),
                copy: key.copy_number(),
                open: self.views.contains(&key),
                geometry: self.geometries.get(&key).copied(),
                settings,
            })
            .collect();
        workspace::Workspace { windows }
    }

    /// Back to a workspace: its windows opened again where they were, as they were.
    pub fn restore(&mut self, saved: &workspace::Workspace) {
        for window in &saved.windows {
            let Some(view) = View::opened_by(&window.view) else {
                continue;
            };
            if view == View::Palette || (window.copy > 0 && !view.info().copies) {
                continue;
            }
            let key = ViewKey::copy(view, window.copy);
            if !window.settings.is_null() {
                self.windows.get(key).load(&window.settings);
            }
            if let Some(geometry) = window.geometry {
                self.geometries.insert(key, geometry);
            }
            if window.open {
                self.views.insert(key);
            }
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
            .filter(|key| key.view().info().debugger)
            .collect();
        self.view_contexts.retain(|key, _| open.contains(key));
        open.into_iter()
            .map(|key| NativeView {
                key: key.0,
                title: key.title(),
                resizable: key.view().info().resizable,
                geometry: self.geometries.get(&key).copied(),
                ctx: self
                    .view_contexts
                    .entry(key)
                    .or_insert_with(new_context)
                    .clone(),
            })
            .collect()
    }

    /// What the windows asked of the others (`Windows::links`): the Memory or
    /// Disassembly window (the first, or a new one) at the address, open and in front.
    fn follow_links(&mut self) {
        for link in std::mem::take(&mut self.windows.links) {
            let copy = if link.new {
                match self.free_copy(link.view) {
                    Some(copy) => copy,
                    None => continue,
                }
            } else {
                0
            };
            let key = ViewKey::copy(link.view, copy);
            let window = if link.new {
                self.windows.reset(key)
            } else {
                self.windows.get(key)
            };
            window.go_to(format!("&{:04X}", link.address));
            self.open_copy(link.view, copy);
        }
    }

    /// The first window number of `view` not open (`None`: [`COPIES`] are).
    fn free_copy(&self, view: View) -> Option<u8> {
        (0..COPIES).find(|&copy| !self.views.contains(&ViewKey::copy(view, copy)))
    }

    /// Window `copy` of `view` open, and in front.
    fn open_copy(&mut self, view: View, copy: u8) {
        let key = ViewKey::copy(view, copy);
        if !self.views.insert(key) {
            self.focus.push(key);
        }
    }

    /// The windows of the system to bring to the front (the Window menu, a view opened again).
    pub fn take_focus_requests(&mut self) -> Vec<u8> {
        if !self.native_views {
            return Vec::new();
        }
        let keys: Vec<u8> = self
            .focus
            .iter()
            .filter(|key| key.view().info().debugger)
            .map(|key| key.0)
            .collect();
        self.focus.retain(|key| !key.view().info().debugger);
        keys
    }

    /// The window of the system of view `key` was closed.
    pub fn close_view(&mut self, key: u8) {
        self.windows.closed(ViewKey(key));
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
        // the interface zoom is the emulator window's: the debugger's windows keep their size
        let ctx = self.view_contexts.get(&key)?.clone();
        let windows = &mut self.windows;
        let mut wanted = egui::Vec2::ZERO;
        let output = ctx.run(raw, |ctx| {
            let panel = egui::CentralPanel::default().show(ctx, |ui| {
                // both ways: the lines keep their length (no wrapping to the window's width),
                // so the content says how wide the window is to be
                egui::ScrollArea::both()
                    .auto_shrink([true, true])
                    .show(ui, |ui| {
                        // no text wraps to the window's width: the content's size is its own,
                        // whatever the window's (the window is then sized to it)
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        windows.contents(key, ui, target, bus);
                    })
                    .content_size
            });
            // with the panel's margins
            wanted = panel.inner
                + egui::Frame::central_panel(&ctx.style())
                    .total_margin()
                    .sum();
        });
        self.follow_links();
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
            // another Memory or Disassembly window, of its own
            HostRequest::OpenAnother(view) => {
                if let Some(copy) = self.free_copy(*view) {
                    self.windows.reset(ViewKey::copy(*view, copy));
                    self.open_copy(*view, copy);
                }
                true
            }
            HostRequest::Open(view) => {
                // open, or brought to the front when it is already
                let key = ViewKey::of(*view);
                if !self.views.insert(key) {
                    self.focus.push(key);
                }
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

    /// The simulated joystick window's current state (stick + fire buttons).
    pub fn simulated_joysticks(&self) -> Joysticks {
        self.windows.input.joysticks
    }

    /// The CPC keys currently held down by the virtual keyboard (mouse held on a key).
    pub fn virtual_keys_held(&self) -> Vec<CpcKey> {
        self.windows.input.keys_held.clone()
    }

    /// Whether Ctrl is being used as a mouse modifier on the virtual Shift key.
    pub fn virtual_keyboard_ctrl_click(&self) -> bool {
        self.windows.input.ctrl_click
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
        // (a field of a debugger's own window takes the keys of that window only)
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

    /// Whether the menu and status bars are shown: always in a window; in fullscreen while the
    /// pointer is used, and [`BARS_HIDE_S`] after, or while a menu, the palette or a question is
    /// open. Hidden, the pointer is too.
    fn bars_shown(&mut self, ctx: &egui::Context, target: &dyn CommandTarget) -> bool {
        let now = ctx.input(|i| i.time);
        let used = ctx.input(|i| {
            i.pointer.delta() != egui::Vec2::ZERO
                || i.pointer.any_down()
                || i.pointer.any_click()
                || i.raw_scroll_delta != egui::Vec2::ZERO
        });
        if used {
            self.pointer_seen = now;
        }
        if !target.settings().is_some_and(|s| s.fullscreen) {
            return true;
        }
        let busy = egui::Popup::is_any_open(ctx) || self.palette.is_some() || self.prompt.is_some();
        let shown = busy || now - self.pointer_seen < BARS_HIDE_S;
        if shown {
            // to hide them on time even without events
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                (BARS_HIDE_S - (now - self.pointer_seen)).max(0.05),
            ));
        } else {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        shown
    }

    /// The height of the status bar, in points (0 without it): the picture goes above.
    pub fn status_height(&self) -> f32 {
        self.status_height
    }

    /// Lay out one frame of the interface. Its commands are queued on `bus`.
    pub fn run(
        &mut self,
        raw: egui::RawInput,
        target: &dyn CommandTarget,
        bus: &mut CommandBus,
    ) -> egui::FullOutput {
        let ctx = self.ctx.clone();
        apply_zoom(&ctx, target);
        let output = ctx.run(raw, |ctx| self.ui(ctx, target, bus));
        self.follow_links();
        output
    }

    fn ui(&mut self, ctx: &egui::Context, target: &dyn CommandTarget, bus: &mut CommandBus) {
        self.shortcuts(ctx, target, bus);
        // stopped (Pause, F8, a breakpoint, a step done): the code where it stopped is what one
        // looks at, the first Disassembly window back on the PC, in front
        let paused = target.debugger().is_some_and(|d| d.paused());
        if paused && !self.was_paused {
            self.windows
                .get(ViewKey::of(View::Disassembly))
                .go_to(String::new());
            self.open_copy(View::Disassembly, 0);
        }
        self.was_paused = paused;
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
        let visible =
            target.settings().is_none_or(|s| s.menu_visible) && self.bars_shown(ctx, target);
        self.menu_height = if visible {
            let windows: Vec<(u8, String)> =
                self.views.iter().map(|key| (key.0, key.title())).collect();
            let panel = egui::TopBottomPanel::top("menu")
                .show(ctx, |ui| menu::bar(ui, target, bus, &windows));
            match panel.inner {
                Some(menu::WindowAction::Focus(key)) => self.focus.push(ViewKey(key)),
                Some(menu::WindowAction::CloseAll) => {
                    for key in std::mem::take(&mut self.views) {
                        self.windows.closed(key);
                    }
                    self.view_contexts.clear();
                }
                None => {}
            }
            panel.response.rect.height()
        } else {
            0.0
        };
        // the status bar, with the menus (shown and hidden together)
        self.status_height = if visible {
            egui::TopBottomPanel::bottom("status")
                .show(ctx, |ui| chips::status_bar(ui, target))
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
                .filter(|key| !key.view().info().debugger)
                .collect()
        } else {
            self.views.clone()
        };
        for key in embedded {
            let mut open = true;
            self.windows.show(key, &mut open, ctx, target, bus);
            if !open {
                closed.push(key);
            }
        }
        for key in closed {
            self.windows.closed(key);
            self.views.remove(&key);
        }
        // the embedded windows asked for: in front (the system's ones: the host's)
        let native = self.native_views;
        self.focus.retain(|key| {
            if native && key.view().info().debugger {
                return true;
            }
            let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new(key.title()));
            ctx.move_to_top(layer);
            false
        });
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

/// In fullscreen, the bars hide this many seconds after the pointer was last used.
const BARS_HIDE_S: f64 = 3.0;

/// The interface's zoom (Video > Interface zoom), on a context before its pass: the menus and
/// every window follow it at once. egui's own Ctrl +/- zoom is off: the setting is the one zoom.
fn apply_zoom(ctx: &egui::Context, target: &dyn CommandTarget) {
    let zoom = target.settings().map_or(1.0, |s| s.ui_zoom);
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    if (ctx.zoom_factor() - zoom).abs() > 1e-3 {
        ctx.set_zoom_factor(zoom);
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

    /// A pause shows the code where the machine stopped: the first Disassembly window, back on
    /// the PC.
    #[test]
    fn a_pause_opens_the_disassembly_on_the_pc() {
        let mut target = Target(
            Motherboard::default(),
            DebugServer::offline(),
            HostSettings::default(),
        );
        let mut bus = CommandBus::default();
        let mut gui = Gui::new();
        let key = ViewKey::of(View::Disassembly);
        gui.windows.get(key).go_to("&4000".into());
        frame(&mut gui, &target, &mut bus, 0.0);
        assert!(!gui.views.contains(&key));
        target.1.set_paused(true);
        frame(&mut gui, &target, &mut bus, 0.02);
        assert!(gui.views.contains(&key));
        assert!(!gui.views.contains(&ViewKey::of(View::Z80)));
        let window = gui.windows.state::<window::Disassembly>(key).expect("open");
        assert_eq!(window.window.address, "");
    }

    /// A workspace brings the windows back open, where they were, with their settings.
    #[test]
    fn a_workspace_brings_the_windows_back() {
        let target = Target(
            Motherboard::default(),
            DebugServer::offline(),
            HostSettings::default(),
        );
        let mut bus = CommandBus::default();
        let mut gui = Gui::new();
        gui.handle(&HostRequest::Open(View::Crtc));
        gui.handle(&HostRequest::OpenAnother(View::Memory));
        gui.handle(&HostRequest::OpenAnother(View::Memory));
        gui.windows
            .get(ViewKey::copy(View::Memory, 1))
            .go_to("&C000".into());
        let geometry = workspace::Geometry {
            position: [100, 200],
            size: Some([300.0, 400.0]),
        };
        gui.note_geometry(ViewKey::of(View::Crtc).0, geometry);
        frame(&mut gui, &target, &mut bus, 0.0);
        let saved = gui.workspace();
        // as a file would give it back
        let saved: workspace::Workspace =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();

        let mut again = Gui::new();
        again.restore(&saved);
        assert!(again.views.contains(&ViewKey::of(View::Crtc)));
        assert!(again.views.contains(&ViewKey::copy(View::Memory, 1)));
        assert_eq!(memory_address(&mut again, 1), "&C000");
        assert_eq!(
            again.geometries.get(&ViewKey::of(View::Crtc)),
            Some(&geometry)
        );
    }

    /// The address Memory window `copy` shows.
    fn memory_address(gui: &mut Gui, copy: u8) -> String {
        let key = ViewKey::copy(View::Memory, copy);
        let memory = gui.windows.state::<window::Memory>(key).expect("open");
        memory.window.address.clone()
    }

    #[test]
    fn memory_windows_open_several_each_its_own() {
        use code_views::Link;
        let target = Target(
            Motherboard::default(),
            DebugServer::offline(),
            HostSettings::default(),
        );
        let mut bus = CommandBus::default();
        let mut gui = Gui::new();
        for address in [0x4000, 0xc000] {
            gui.windows.links.push(Link {
                view: View::Memory,
                address,
                new: true,
            });
            gui.follow_links();
        }
        frame(&mut gui, &target, &mut bus, 0.0);
        let keys: Vec<ViewKey> = gui.views.iter().copied().collect();
        assert_eq!(
            keys,
            vec![
                ViewKey::copy(View::Memory, 0),
                ViewKey::copy(View::Memory, 1)
            ]
        );
        assert_eq!(keys[1].title(), "Memory 2");
        assert_eq!(memory_address(&mut gui, 0), "&4000");
        assert_eq!(memory_address(&mut gui, 1), "&C000");
        // the first window, not a new one
        gui.windows.links.push(Link {
            view: View::Memory,
            address: 0x1234,
            new: false,
        });
        gui.follow_links();
        assert_eq!(gui.views.len(), 2);
        assert_eq!(memory_address(&mut gui, 0), "&1234");
        // 8 at most
        for _ in 0..10 {
            gui.handle(&HostRequest::OpenAnother(View::Memory));
        }
        assert_eq!(gui.views.len(), usize::from(COPIES));
    }

    #[test]
    fn closing_native_joystick_window_releases_controls() {
        let mut gui = Gui::new();
        gui.handle(&HostRequest::Open(View::Joystick));
        gui.windows.input.joysticks.joy0.x = 1;
        gui.windows.input.joysticks.joy1.fire[0] = true;
        gui.close_view(ViewKey::of(View::Joystick).0);
        assert_eq!(gui.simulated_joysticks(), Joysticks::default());
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
            for view in View::ALL {
                frame(&mut gui, &target, &mut bus, 0.0);
                gui.handle(&HostRequest::Open(view));
                frame(&mut gui, &target, &mut bus, 0.02);
                gui.views.clear();
            }
        }
    }
}
