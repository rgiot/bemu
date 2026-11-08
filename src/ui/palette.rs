//! The command palette (Tools > Command palette, Ctrl+Shift+P), as VS Code's: type a few
//! letters of a command's name, id or description, the arrows choose, Enter runs it. Each row
//! shows the command's name and shortcut, then its id and what it does. A command with a value
//! to choose (CRTC, speed, rendering...) or an argument to give (an address, a number, a text)
//! asks it in a second step, in the palette.

use crate::commands::palette::{search, search_values, Choice, Entry};
use crate::commands::{
    state, Arg, ArgKind, ArgPrompt, Command, CommandBus, CommandSpec, CommandState, CommandTarget,
    Kind,
};

/// Where the palette is.
enum Step {
    /// Searching the commands.
    Search,
    /// Choosing the value of a family (CRTC 0 to 4...).
    Value(&'static Choice),
    /// A command ran: whether it asks something.
    Waiting(&'static str),
    /// Giving the argument a command asks.
    Argument(ArgPrompt),
}

pub struct Palette {
    step: Step,
    query: String,
    selected: usize,
    focus: bool,
    /// Frames shown: a click from before it opened does not close it.
    frames: u32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            step: Step::Search,
            query: String::new(),
            selected: 0,
            focus: true,
            frames: 0,
        }
    }
}

/// How many rows are listed.
const SHOWN: usize = 12;
const WIDTH: f32 = 600.0;

/// One row: the name (and shortcut) above, the id and what it does below.
fn row(
    ui: &mut egui::Ui,
    selected: bool,
    state: CommandState,
    name: &str,
    shortcut: Option<&str>,
    id: &str,
    aim: &str,
) -> egui::Response {
    let visuals = ui.visuals().clone();
    let strong = if state.enabled {
        visuals.strong_text_color()
    } else {
        visuals.weak_text_color()
    };
    let weak = visuals.weak_text_color();
    let mut job = egui::text::LayoutJob::default();
    let body = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::FontId::proportional(body.size * 0.9);
    let mono = egui::FontId::monospace(body.size * 0.85);
    let mark = if state.checked { "✔ " } else { "" };
    job.append(
        &format!("{mark}{name}"),
        0.0,
        egui::TextFormat::simple(body.clone(), strong),
    );
    if let Some(shortcut) = shortcut {
        job.append(
            shortcut,
            12.0,
            egui::TextFormat::simple(small.clone(), weak),
        );
    }
    job.append("\n", 0.0, egui::TextFormat::simple(small.clone(), weak));
    job.append(
        id,
        0.0,
        egui::TextFormat::simple(mono, visuals.hyperlink_color),
    );
    let mut aim = aim.to_owned();
    if aim.chars().count() > 90 {
        aim = aim.chars().take(88).collect::<String>() + "…";
    }
    job.append(&aim, 10.0, egui::TextFormat::simple(small, weak));
    job.wrap.max_width = WIDTH - 24.0;
    let response = ui.add_enabled(
        state.enabled,
        egui::Button::selectable(selected, job).min_size(egui::vec2(WIDTH - 16.0, 0.0)),
    );
    if selected {
        response.scroll_to_me(None);
    }
    response
}

/// The command a palette entry of `spec` runs: a running recording's stop.
fn run_id(spec: &'static CommandSpec, state: CommandState) -> &'static str {
    match (spec.kind, spec.stop_id) {
        (Kind::StartStop, Some(stop)) if state.checked => stop,
        _ => spec.id,
    }
}

/// The keys of the frame.
struct Keys {
    up: bool,
    down: bool,
    enter: bool,
    escape: bool,
    backspace: bool,
}

impl Palette {
    /// The step, for the field's id (a new step, a new field).
    fn step_key(&self) -> u8 {
        match self.step {
            Step::Search => 0,
            Step::Value(_) => 1,
            Step::Waiting(_) => 2,
            Step::Argument(_) => 3,
        }
    }

    /// The command it ran and waits on, to ask its argument here.
    pub fn waiting(&self) -> Option<&'static str> {
        match self.step {
            Step::Waiting(id) => Some(id),
            _ => None,
        }
    }

    /// Ask `prompt` here (a text, an address, a number, a choice).
    pub fn ask(&mut self, prompt: ArgPrompt) {
        self.query = match prompt.kind {
            ArgKind::Number { default, .. } => default.to_string(),
            _ => String::new(),
        };
        self.step = Step::Argument(prompt);
        self.selected = 0;
        self.focus = true;
    }

    /// Whether the palette asks `kind` itself (files go to the file browser).
    pub fn asks(kind: &ArgKind) -> bool {
        !matches!(
            kind,
            ArgKind::OpenFile(_) | ArgKind::SaveFile(..) | ArgKind::Folder
        )
    }

    fn enter_step(&mut self, step: Step) {
        self.step = step;
        self.query.clear();
        self.selected = 0;
        self.focus = true;
    }

    /// Show the palette: `false` once it is closed.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        target: &dyn CommandTarget,
        bus: &mut CommandBus,
        recent: &[&str],
    ) -> bool {
        if let Step::Waiting(_) = self.step {
            // the command ran without asking anything: done
            return false;
        }
        let empty = self.query.is_empty();
        let keys = ctx.input_mut(|i| Keys {
            up: i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            down: i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            enter: i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            escape: i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            backspace: empty && i.key_pressed(egui::Key::Backspace),
        });
        if keys.escape {
            return false;
        }
        // Backspace on an empty field: back to the commands
        if keys.backspace && !matches!(self.step, Step::Search) {
            self.enter_step(Step::Search);
        }
        let mut open = true;
        egui::Window::new("Command palette")
            .id(egui::Id::new("command-palette"))
            .title_bar(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 36.0])
            .default_width(WIDTH)
            .resizable(false)
            .show(ctx, |ui| {
                open = self.contents(ui, &keys, target, bus, recent);
            });
        self.frames += 1;
        if self.frames > 2 && ctx.input(|i| i.pointer.any_click()) && !ctx.is_pointer_over_area() {
            return false;
        }
        open
    }

    fn field(&mut self, ui: &mut egui::Ui, hint: &str) {
        let id = egui::Id::new(("command-palette-field", self.step_key()));
        // what is typed before the field has the focus (the frame it opens) is its
        if !ui.memory(|m| m.has_focus(id)) {
            let typed: String = ui.input_mut(|i| {
                let mut typed = String::new();
                i.events.retain(|event| match event {
                    egui::Event::Text(text) => {
                        typed.push_str(text);
                        false
                    }
                    _ => true,
                });
                typed
            });
            if !typed.is_empty() {
                self.query.push_str(&typed);
                self.selected = 0;
            }
        }
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .id(id)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
        if self.focus {
            field.request_focus();
            // the cursor after what was already typed
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let end = egui::text::CCursor::new(self.query.chars().count());
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            }
            self.focus = false;
        }
        if field.changed() {
            self.selected = 0;
        }
    }

    fn select(&mut self, keys: &Keys, count: usize) {
        if keys.down && self.selected + 1 < count {
            self.selected += 1;
        }
        if keys.up {
            self.selected = self.selected.saturating_sub(1);
        }
        self.selected = self.selected.min(count.saturating_sub(1));
    }

    /// The palette's contents: `false` when it is done.
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        keys: &Keys,
        target: &dyn CommandTarget,
        bus: &mut CommandBus,
        recent: &[&str],
    ) -> bool {
        match &self.step {
            Step::Search => {
                self.field(ui, "Type a command's name, id or what it does");
                // the query as typed this frame (Enter may come with the last letters)
                let found = search(&self.query, recent);
                self.select(keys, found.len().min(SHOWN));
                ui.separator();
                if found.is_empty() {
                    ui.weak("No command matches");
                }
                let mut chosen = None;
                egui::ScrollArea::vertical()
                    .max_height(520.0)
                    .show(ui, |ui| {
                        for (index, entry) in found.iter().take(SHOWN).enumerate() {
                            let (state, name) = match entry {
                                Entry::Command(spec) => {
                                    let mut name = entry.label().to_owned();
                                    if spec.kind == Kind::Input && !name.ends_with('…') {
                                        name.push('…');
                                    }
                                    (state(spec.id, target), name)
                                }
                                Entry::Choice(choice) => {
                                    let states: Vec<_> =
                                        choice.items.iter().map(|i| state(i.id, target)).collect();
                                    let current = choice
                                        .items
                                        .iter()
                                        .zip(&states)
                                        .find(|(_, s)| s.checked)
                                        .map(|(item, _)| item.label());
                                    let name = match current {
                                        Some(current) => format!("{}…   = {current}", choice.label),
                                        None => format!("{}…", choice.label),
                                    };
                                    (
                                        CommandState::enabled(states.iter().any(|s| s.enabled)),
                                        name,
                                    )
                                }
                            };
                            let response = row(
                                ui,
                                index == self.selected,
                                state,
                                &name,
                                entry.shortcut(),
                                entry.id(),
                                entry.aim(),
                            );
                            if response.clicked()
                                || (keys.enter && index == self.selected && state.enabled)
                            {
                                chosen = Some(*entry);
                            }
                        }
                    });
                match chosen {
                    Some(Entry::Choice(choice)) => self.enter_step(Step::Value(choice)),
                    Some(Entry::Command(spec)) => {
                        let id = run_id(spec, state(spec.id, target));
                        bus.push_id(id);
                        // it may ask an argument: asked here, the next frame
                        self.step = Step::Waiting(spec.id);
                    }
                    None => {}
                }
                true
            }
            Step::Value(choice) => {
                let choice = *choice;
                ui.horizontal(|ui| {
                    ui.strong(&choice.label);
                    ui.weak(egui::RichText::new(&choice.id).monospace());
                });
                self.field(ui, "Choose a value (Backspace: back to the commands)");
                let values = search_values(choice, &self.query);
                self.select(keys, values.len());
                ui.separator();
                let mut chosen = None;
                for (index, spec) in values.iter().enumerate() {
                    let state = state(spec.id, target);
                    let response = row(
                        ui,
                        index == self.selected,
                        state,
                        spec.label(),
                        spec.shortcut,
                        spec.id,
                        spec.aim,
                    );
                    if response.clicked() || (keys.enter && index == self.selected && state.enabled)
                    {
                        chosen = Some(spec.id);
                    }
                }
                match chosen {
                    Some(id) => {
                        bus.push_id(id);
                        false
                    }
                    None => true,
                }
            }
            Step::Argument(prompt) => {
                let prompt = prompt.clone();
                ui.horizontal(|ui| {
                    ui.strong(prompt.title);
                    ui.weak(egui::RichText::new(prompt.id).monospace());
                });
                if let ArgKind::Choice(choices) = prompt.kind {
                    self.field(ui, "Choose (Backspace: back to the commands)");
                    let values: Vec<_> = choices
                        .iter()
                        .filter(|(value, label)| {
                            let q = self.query.to_lowercase();
                            value.to_lowercase().contains(&q) || label.to_lowercase().contains(&q)
                        })
                        .collect();
                    self.select(keys, values.len());
                    ui.separator();
                    for (index, (value, label)) in values.iter().enumerate() {
                        let response =
                            ui.add(egui::Button::selectable(index == self.selected, *label));
                        if response.clicked() || (keys.enter && index == self.selected) {
                            if let Ok(command) =
                                Command::with_arg(prompt.id, Arg::Text((*value).to_owned()))
                            {
                                bus.push(command);
                            }
                            return false;
                        }
                    }
                    return true;
                }
                let hint = match prompt.kind {
                    ArgKind::Address => "&BE00, #38, 1000, or a symbol".to_owned(),
                    ArgKind::Number { min, max, .. } => format!("a number from {min} to {max}"),
                    _ => String::new(),
                };
                self.field(ui, &hint);
                if let (ArgKind::Address, Some(debugger)) = (&prompt.kind, target.debugger()) {
                    if let Some(address) = debugger.symbols().resolve(self.query.trim()) {
                        ui.weak(format!("= &{address:04X}"));
                    }
                }
                ui.weak("Enter runs it, Escape cancels");
                if keys.enter {
                    let arg = match prompt.kind {
                        ArgKind::Number { .. } => match Arg::Text(self.query.clone()).number() {
                            Ok(n) => Arg::Number(n),
                            Err(_) => Arg::Text(self.query.clone()),
                        },
                        _ => Arg::Text(self.query.clone()),
                    };
                    if let Ok(command) = Command::with_arg(prompt.id, arg) {
                        bus.push(command);
                    }
                    return false;
                }
                true
            }
            Step::Waiting(_) => false,
        }
    }
}
