//! The Tape window: the cassette deck and the recording.

use super::*;

/// The cassette deck and the recorder.
pub fn tape(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    let motor = mb.ppi().get_data_c_from_pins() & 0x10 != 0;
    match mb.tape_deck() {
        Some(deck) => {
            let duration = deck.tape().duration_seconds();
            let position = deck.position_seconds();
            ui.add(
                egui::ProgressBar::new((position / duration.max(1e-6)) as f32)
                    .desired_width(320.0)
                    .text(format!("{position:.1} s / {duration:.1} s")),
            );
            egui::Grid::new("tape").striped(true).show(ui, |ui| {
                ui.label("PLAY");
                ui.monospace(if deck.at_end() {
                    "at the end"
                } else if deck.playing() {
                    "down"
                } else {
                    "up"
                });
                ui.end_row();
                ui.label("Motor");
                ui.monospace(if motor { "on" } else { "off" });
                ui.end_row();
                ui.label("Head level");
                ui.monospace(u8::from(deck.level()).to_string());
                ui.end_row();
            });
            ui.horizontal(|ui| {
                for (label, id) in [
                    ("Play", "tape-play"),
                    ("Stop", "tape-stop"),
                    ("Rewind", "tape-rewind"),
                ] {
                    if ui.button(label).clicked() {
                        bus.push_id(id);
                    }
                }
            });
        }
        None => {
            ui.weak(format!(
                "no tape (motor {})",
                if motor { "on" } else { "off" }
            ));
        }
    }
    crate::ui::widgets::rule(ui);
    match mb.tape_recorder() {
        Some(recorder) => {
            ui.label(format!("Recording: {:.1} s of tape", recorder.seconds()));
            if ui.button("Stop and save…").clicked() {
                bus.push_id("tape-record-stop");
            }
        }
        None => {
            if ui.button("Record what the CPC saves").clicked() {
                bus.push_id("tape-record");
            }
        }
    }
}
