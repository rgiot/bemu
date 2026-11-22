//! The menu bar: the command registry's menu paths, as nested menus, in the order of the
//! specification.

use crate::commands::{self, state, Arg, Command, CommandBus, CommandSpec, CommandTarget, Kind};

/// The menu bar.
/// What the Window menu asks.
pub enum WindowAction {
    Focus(u8),
    CloseAll,
}

/// The menu bar: the registry's menus, then Window (the windows open, `windows`: their key and
/// title).
pub fn bar(
    ui: &mut egui::Ui,
    target: &dyn CommandTarget,
    bus: &mut CommandBus,
    windows: &[(u8, String)],
) -> Option<WindowAction> {
    let mut action = None;
    egui::MenuBar::new().ui(ui, |ui| {
        let mut tops = level(ui, &[], target, bus);
        let window = ui.menu_button("Window", |ui| {
            if windows.is_empty() {
                ui.weak("No window open");
            }
            for (key, title) in windows {
                if ui
                    .add(egui::Button::new((indent(ui), title.as_str())))
                    .on_hover_text("Bring it to the front (shown again if minimised)")
                    .clicked()
                {
                    action = Some(WindowAction::Focus(*key));
                    ui.close();
                }
            }
            ui.separator();
            if ui
                .add_enabled(
                    !windows.is_empty(),
                    egui::Button::new((indent(ui), "Close all windows")),
                )
                .clicked()
            {
                action = Some(WindowAction::CloseAll);
                ui.close();
            }
        });
        tops.push(window.response);
        switch_on_hover(ui.ctx(), &tops);
    });
    action
}

/// One menu of the bar open, the pointer on another's title opens that one (as menu bars do):
/// egui's opens only on a click.
fn switch_on_hover(ctx: &egui::Context, tops: &[egui::Response]) {
    let popup = |response: &egui::Response| egui::Popup::default_response_id(response);
    let any_open = tops
        .iter()
        .any(|top| egui::Popup::is_id_open(ctx, popup(top)));
    if let Some(hovered) = tops.iter().find(|top| top.hovered()) {
        if any_open && !egui::Popup::is_id_open(ctx, popup(hovered)) {
            egui::Popup::open_id(ctx, popup(hovered));
            ctx.request_repaint();
        }
    }
}

/// The items and submenus under `prefix`, as they first come in the registry, a line between
/// their groups (the specification's `sep` rows, and after a radio group). Returns the
/// buttons of its submenus.
fn level(
    ui: &mut egui::Ui,
    prefix: &[&str],
    target: &dyn CommandTarget,
    bus: &mut CommandBus,
) -> Vec<egui::Response> {
    let mut buttons = Vec::new();
    let depth = prefix.len();
    let mut submenus: Vec<&str> = Vec::new();
    let mut previous_group: Option<&[&str]> = None;
    // no line at the top, nor two in a row
    let mut after_line = true;
    let line = |ui: &mut egui::Ui, after_line: &mut bool| {
        if !*after_line {
            ui.separator();
            *after_line = true;
        }
    };
    for spec in commands::all() {
        if spec.path.len() <= depth || spec.path[..depth] != *prefix {
            continue;
        }
        if commands::SEPARATORS.contains(&(spec.id, depth)) {
            line(ui, &mut after_line);
        }
        if spec.path.len() == depth + 1 {
            // a line between a radio group and what follows
            if previous_group.is_some_and(|g| g != spec.group()) {
                line(ui, &mut after_line);
            }
            previous_group = (spec.kind == Kind::Radio).then(|| spec.group());
            item(ui, spec, target, bus);
            after_line = false;
        } else {
            let name = spec.path[depth];
            if submenus.contains(&name) {
                continue;
            }
            if previous_group.is_some() {
                line(ui, &mut after_line);
            }
            submenus.push(name);
            previous_group = None;
            let menu = ui.menu_button((indent(ui), name), |ui| {
                level(ui, &spec.path[..=depth], target, bus);
            });
            buttons.push(menu.response);
            after_line = false;
        }
    }
    buttons
}

/// A blank as wide as a check box less the button's padding: a plain item's text starts where a
/// check box's or a radio button's does.
fn indent(ui: &egui::Ui) -> egui::Atom<'static> {
    let spacing = ui.spacing();
    let width = (spacing.icon_width - spacing.button_padding.x).max(0.0);
    egui::Atom::custom(egui::IdSalt::new("menu-indent"), egui::vec2(width, 0.0))
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
    let why_not = format!("Not available now\n\n{}", spec.note);
    if spec.kind == Kind::Submenu {
        // the recent discs of a drive
        let drive = usize::from(spec.id.starts_with("drive-b"));
        let recent = target
            .settings()
            .map(|s| s.media.recent_discs[drive].clone())
            .unwrap_or_default();
        ui.add_enabled_ui(!recent.is_empty(), |ui| {
            ui.menu_button((indent(ui), label), |ui| {
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
    let mut button = egui::Button::new((indent(ui), label));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of a bar of two menus, the pointer's events given: the menus' buttons.
    fn bar_frame(ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::Response> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            events,
            ..Default::default()
        };
        let mut tops = Vec::new();
        let _ = ctx.run_ui(raw, |root| {
            egui::Panel::top("bar").show(root, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    for name in ["File", "Machine"] {
                        tops.push(ui.menu_button(name, |ui| ui.label(name)).response);
                    }
                    switch_on_hover(ui.ctx(), &tops);
                });
            });
        });
        tops
    }

    #[test]
    fn a_menu_open_the_pointer_on_another_opens_it() {
        let ctx = egui::Context::default();
        let tops = bar_frame(&ctx, Vec::new());
        let (file, machine) = (tops[0].rect.center(), tops[1].rect.center());
        let popup = |top: &egui::Response| egui::Popup::default_response_id(top);
        let (file_menu, machine_menu) = (popup(&tops[0]), popup(&tops[1]));

        // the pointer alone opens nothing
        bar_frame(&ctx, vec![egui::Event::PointerMoved(machine)]);
        bar_frame(&ctx, Vec::new());
        assert!(!egui::Popup::is_id_open(&ctx, machine_menu));

        let press = |pressed| egui::Event::PointerButton {
            pos: file,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        bar_frame(&ctx, vec![egui::Event::PointerMoved(file), press(true)]);
        bar_frame(&ctx, vec![press(false)]);
        assert!(egui::Popup::is_id_open(&ctx, file_menu));

        bar_frame(&ctx, vec![egui::Event::PointerMoved(machine)]);
        bar_frame(&ctx, Vec::new());
        assert!(egui::Popup::is_id_open(&ctx, machine_menu));
        assert!(!egui::Popup::is_id_open(&ctx, file_menu));
    }
}
