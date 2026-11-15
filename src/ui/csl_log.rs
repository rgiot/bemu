//! The CSL log window: the instructions of the script playing (or of the last one), in the
//! order they were played; a screenshot shows when its line is hovered.

use std::collections::HashMap;

use crate::commands::{CommandBus, CommandTarget};

/// The pictures already given to this window's context, by line (per context: a native
/// window has its own).
#[derive(Clone, Default)]
struct Textures {
    log: usize,
    by_line: HashMap<usize, egui::TextureHandle>,
}

pub(super) fn csl_log(ui: &mut egui::Ui, target: &dyn CommandTarget, bus: &mut CommandBus) {
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
    });
    let Some(log) = target.csl_log() else {
        ui.weak("No CSL script played yet (File > CSL).");
        return;
    };
    let entries = log.entries();
    let shots = entries.iter().filter(|e| e.picture.is_some()).count();
    ui.weak(format!(
        "{} instruction(s), {shots} screenshot(s). Hover a screenshot to see it.",
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
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
    let last = entries.len().saturating_sub(1);
    egui::ScrollArea::vertical()
        .id_salt("csl-log")
        .max_height(420.0)
        .stick_to_bottom(true)
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
                    let response = ui.label(text);
                    if let Some((path, picture)) = &entry.picture {
                        let shot = ui.add(
                            egui::Label::new(egui::RichText::new("[screenshot]").weak())
                                .sense(egui::Sense::hover()),
                        );
                        let texture = textures
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
                            .clone();
                        let show = |ui: &mut egui::Ui| {
                            ui.add(egui::Image::new(&texture).fit_to_original_size(1.0));
                            ui.weak(path.display().to_string());
                        };
                        response.on_hover_ui(show);
                        shot.on_hover_ui(show);
                    }
                });
            }
        });
    drop(entries);
    ui.ctx().data_mut(|d| d.insert_temp(id, textures));
}
