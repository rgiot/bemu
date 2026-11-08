//! The menu bar: the command registry's menu paths, as nested menus, in the order of the
//! specification.

use crate::commands::{self, state, Arg, Command, CommandBus, CommandSpec, CommandTarget, Kind};

/// The menu bar.
pub fn bar(ui: &mut egui::Ui, target: &dyn CommandTarget, bus: &mut CommandBus) {
    egui::MenuBar::new().ui(ui, |ui| {
        level(ui, &[], target, bus);
    });
}

/// The items and submenus under `prefix`, as they first come in the registry.
fn level(ui: &mut egui::Ui, prefix: &[&str], target: &dyn CommandTarget, bus: &mut CommandBus) {
    let depth = prefix.len();
    let mut submenus: Vec<&str> = Vec::new();
    let mut previous_group: Option<&[&str]> = None;
    for spec in commands::all() {
        if spec.path.len() <= depth || spec.path[..depth] != *prefix {
            continue;
        }
        if spec.path.len() == depth + 1 {
            // a line between a radio group and what follows
            if previous_group.is_some_and(|g| g != spec.group()) && spec.kind != Kind::Radio {
                ui.separator();
            }
            previous_group = (spec.kind == Kind::Radio).then(|| spec.group());
            item(ui, spec, target, bus);
        } else {
            let name = spec.path[depth];
            if submenus.contains(&name) {
                continue;
            }
            submenus.push(name);
            previous_group = None;
            ui.menu_button(name, |ui| {
                level(ui, &spec.path[..=depth], target, bus);
            });
        }
    }
}

fn hover(spec: &CommandSpec) -> String {
    let mut text = spec.aim.to_owned();
    if !spec.note.is_empty() {
        text.push_str("\n\n");
        text.push_str(spec.note);
    }
    text.push_str(&format!("\n\n{}", spec.id));
    text
}

fn item(
    ui: &mut egui::Ui,
    spec: &'static CommandSpec,
    target: &dyn CommandTarget,
    bus: &mut CommandBus,
) {
    let state = state(spec.id, target);
    let label = match spec.kind {
        Kind::Input => format!("{}…", spec.label().trim_end_matches('…')),
        _ => spec.label().to_owned(),
    };
    let why_not = if spec.note.contains("NOT IMPLEMENTED") {
        "Not implemented yet".to_owned()
    } else {
        format!("Not available now\n\n{}", spec.note)
    };
    if spec.kind == Kind::Submenu {
        // the recent discs of a drive
        let drive = usize::from(spec.id.starts_with("drive-b"));
        let recent = target
            .settings()
            .map(|s| s.media.recent_discs[drive].clone())
            .unwrap_or_default();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button(label, |ui| {
                for path in recent {
                    if ui.button(&path).clicked() {
                        if let Ok(command) = Command::with_arg(spec.id, Arg::Text(path)) {
                            bus.push(command);
                        }
                        ui.close();
                    }
                }
            });
        });
        return;
    }
    let mut button = egui::Button::new(label);
    if let Some(shortcut) = spec.shortcut {
        button = button.shortcut_text(shortcut);
    }
    let response = match spec.kind {
        Kind::Toggle | Kind::StartStop => {
            let mut checked = state.checked;
            ui.add_enabled(
                state.enabled,
                egui::Checkbox::new(&mut checked, spec.label()),
            )
        }
        Kind::Radio => ui.add_enabled(
            state.enabled,
            egui::RadioButton::new(state.checked, spec.label()),
        ),
        _ => ui.add_enabled(state.enabled, button),
    };
    let clicked = response.clicked();
    response
        .on_hover_text(hover(spec))
        .on_disabled_hover_text(why_not);
    if clicked {
        let id = match (spec.kind, spec.stop_id) {
            (Kind::StartStop, Some(stop)) if state.checked => stop,
            _ => spec.id,
        };
        bus.push_id(id);
        ui.close();
    }
}
