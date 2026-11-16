//! The CSL log window: the instructions of the script playing (or of the last one), in the
//! order they were played. A screenshot shows when its line is hovered; a click on a line
//! keeps it selected, its picture shown under the list. The log can be saved as text.

use std::collections::HashMap;

use super::window::{Cx, ViewWindow};

/// The pictures already given to this window's context, by line (per context: a native
/// window has its own).
#[derive(Clone, Default)]
struct Textures {
    log: usize,
    by_line: HashMap<usize, egui::TextureHandle>,
}

/// The window: the line selected (of the log it was selected in).
#[derive(Default)]
pub(super) struct CslLogWindow {
    selected: Option<(usize, usize)>,
}

impl ViewWindow for CslLogWindow {
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Cx) {
        let (target, bus, _) = cx.parts();
        let running = target.csl_running();
        ui.horizontal(|ui| {
            ui.label(if running {
                "A script is playing."
            } else {
                "No script is playing."
            });
            if running && ui.button("Stop").clicked() {
                bus.push_id("csl-stop");
            }
            if ui
                .add_enabled(
                    crate::commands::state("csl-log-save", target).enabled,
                    egui::Button::new("Save…"),
                )
                .on_hover_text("The log as text: time, instruction, screenshot file")
                .clicked()
            {
                bus.push_id("csl-log-save");
            }
        });
        let Some(log) = target.csl_log() else {
            ui.weak("No CSL script played yet (File > Load CSL).");
            return;
        };
        let entries = log.entries();
        let shots = entries.iter().filter(|e| e.picture.is_some()).count();
        ui.weak(format!(
            "{} instruction(s), {shots} screenshot(s). Hover a screenshot to see it, click a line to keep it.",
            entries.len()
        ));
        super::widgets::rule(ui);

        let id = egui::Id::new("csl-log-textures");
        let mut textures: Textures = ui.ctx().data_mut(|d| d.get_temp(id)).unwrap_or_default();
        if textures.log != log.id() {
            textures = Textures {
                log: log.id(),
                by_line: HashMap::new(),
            };
        }
        // a selection of another log (another script) is no more
        let selected = self
            .selected
            .filter(|(of, line)| *of == log.id() && *line < entries.len())
            .map(|(_, line)| line);
        let mut texture_of = |ui: &egui::Ui, line: usize| {
            let (_, picture) = entries[line].picture.as_ref()?;
            Some(
                textures
                    .by_line
                    .entry(line)
                    .or_insert_with(|| {
                        let size = [picture.width() as usize, picture.height() as usize];
                        ui.ctx().load_texture(
                            format!("csl-log-{line}"),
                            egui::ColorImage::from_rgb(size, picture.as_raw()),
                            egui::TextureOptions::LINEAR,
                        )
                    })
                    .clone(),
            )
        };
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        let last = entries.len().saturating_sub(1);
        let mut clicked = None;
        // as tall as its lines (up to a limit), whatever the window's height: the window
        // sizes to it (a list inside a window takes only what the window has)
        let max_height = if selected.is_some() { 240.0 } else { 420.0 };
        let wanted =
            (entries.len() as f32 * (row_height + ui.spacing().item_spacing.y)).min(max_height);
        egui::ScrollArea::vertical()
            .id_salt("csl-log")
            .max_height(max_height)
            .min_scrolled_height(wanted)
            .stick_to_bottom(selected.is_none())
            .auto_shrink([false, true])
            .show_rows(ui, row_height, entries.len(), |ui, rows| {
                for line in rows {
                    let entry = &entries[line];
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{:>10.3} s", entry.at_us as f64 / 1e6));
                        let text = egui::RichText::new(&entry.text).monospace();
                        let text = if running && line == last {
                            text.strong()
                        } else {
                            text
                        };
                        let mut response = ui.selectable_label(selected == Some(line), text);
                        if let Some((path, _)) = &entry.picture {
                            let shot = ui.add(
                                egui::Label::new(egui::RichText::new("[screenshot]").weak())
                                    .sense(egui::Sense::click()),
                            );
                            if let Some(texture) = texture_of(ui, line) {
                                let show = |ui: &mut egui::Ui| {
                                    ui.add(egui::Image::new(&texture).fit_to_original_size(1.0));
                                    ui.weak(path.display().to_string());
                                };
                                response = response.on_hover_ui(show);
                                if shot.on_hover_ui(show).clicked() {
                                    clicked = Some(line);
                                }
                            }
                        }
                        if response.clicked() {
                            clicked = Some(line);
                        }
                    });
                }
            });
        if let Some(line) = clicked {
            // a second click lets it go
            self.selected = (selected != Some(line)).then_some((log.id(), line));
        }
        // the line kept: its instruction, its time, its picture
        if let Some(line) = self
            .selected
            .map(|(_, line)| line)
            .filter(|l| *l < entries.len())
        {
            super::widgets::rule(ui);
            let entry = &entries[line];
            ui.horizontal(|ui| {
                ui.strong(format!("Line {}", line + 1));
                ui.monospace(&entry.text);
                ui.weak(format!("at {:.3} s", entry.at_us as f64 / 1e6));
            });
            match (&entry.picture, texture_of(ui, line)) {
                (Some((path, _)), Some(texture)) => {
                    ui.add(egui::Image::new(&texture).fit_to_original_size(1.0));
                    ui.weak(path.display().to_string());
                }
                _ => {
                    ui.weak("no screenshot on this line");
                }
            }
        }
        drop(entries);
        ui.ctx().data_mut(|d| d.insert_temp(id, textures));
    }
}
