//! The command palette (Tools > Command palette, Ctrl+Shift+P): type a few letters of any
//! command, the arrows choose, Enter runs it.

use crate::commands::{palette::search, state, CommandBus, CommandTarget, Kind};

#[derive(Default)]
pub struct Palette {
    query: String,
    selected: usize,
    focused: bool,
    /// Frames shown: a click from before it opened does not close it.
    frames: u32,
}

/// How many matches are listed.
const SHOWN: usize = 14;

impl Palette {
    /// Show the palette: `false` once it is closed.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        target: &dyn CommandTarget,
        bus: &mut CommandBus,
        recent: &[&str],
    ) -> bool {
        let mut open = true;
        let (up, down, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if escape {
            return false;
        }
        let mut run = None;
        egui::Window::new("Command palette")
            .id(egui::Id::new("command-palette"))
            .title_bar(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 40.0])
            .fixed_size([520.0, 0.0])
            .show(ctx, |ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Run a command: type part of its name")
                        .desired_width(f32::INFINITY),
                );
                if !self.focused {
                    field.request_focus();
                    self.focused = true;
                }
                if field.changed() {
                    self.selected = 0;
                }
                // the query as typed this frame (Enter may come with the last letters)
                let found = search(&self.query, recent);
                let shown = found.len().min(SHOWN);
                if down && self.selected + 1 < shown {
                    self.selected += 1;
                }
                if up {
                    self.selected = self.selected.saturating_sub(1);
                }
                self.selected = self.selected.min(shown.saturating_sub(1));
                ui.separator();
                if found.is_empty() {
                    ui.weak("No command matches");
                }
                for (index, spec) in found.iter().take(SHOWN).enumerate() {
                    let state = state(spec.id, target);
                    let mut text = egui::text::LayoutJob::default();
                    let style = ui.style();
                    let strong = egui::TextFormat {
                        color: if state.enabled {
                            style.visuals.strong_text_color()
                        } else {
                            style.visuals.weak_text_color()
                        },
                        ..Default::default()
                    };
                    let weak = egui::TextFormat {
                        color: style.visuals.weak_text_color(),
                        ..Default::default()
                    };
                    let mark = match (spec.kind, state.checked) {
                        (Kind::Toggle | Kind::Radio | Kind::StartStop, true) => "✔ ",
                        _ => "",
                    };
                    text.append(&format!("{mark}{}", spec.label()), 0.0, strong);
                    text.append(
                        &format!("   {}", spec.menus().join(" › ")),
                        0.0,
                        weak.clone(),
                    );
                    if let Some(shortcut) = spec.shortcut {
                        text.append(&format!("   {shortcut}"), 0.0, weak);
                    }
                    let response = ui
                        .add_enabled(
                            state.enabled,
                            egui::Button::selectable(index == self.selected, text),
                        )
                        .on_hover_text(spec.aim)
                        .on_disabled_hover_text(format!("Not available now: {}", spec.note));
                    if index == self.selected {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() || (enter && index == self.selected && state.enabled) {
                        let id = match (spec.kind, spec.stop_id) {
                            (Kind::StartStop, Some(stop)) if state.checked => stop,
                            _ => spec.id,
                        };
                        run = Some(id);
                    }
                }
            });
        self.frames += 1;
        if self.frames > 2 && ctx.input(|i| i.pointer.any_click()) && !ctx.is_pointer_over_area() {
            open = false;
        }
        if let Some(id) = run {
            bus.push_id(id);
            return false;
        }
        open
    }
}
