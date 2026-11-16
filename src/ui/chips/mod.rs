//! The chips' windows: what the registers mean, decoded (as WinAPE, ACE and MAME show them),
//! over the raw values `DebugServer::inspect` serves the API. A file a window (and the status
//! bar's); the helpers they share here.

use crate::commands::{Arg, Command, CommandBus};
use crate::ga::RgbLevels;
use crate::motherboard::Motherboard;

/// A colour of the CPC's levels (0-100 each).
pub fn rgb_color(levels: RgbLevels) -> egui::Color32 {
    color(levels)
}

fn color(levels: RgbLevels) -> egui::Color32 {
    let byte = |level: f32| (level * 255.0 / 100.0).round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(byte(levels.r()), byte(levels.g()), byte(levels.b()))
}

/// A coloured cell, outlined when `selected`.
fn swatch(ui: &mut egui::Ui, fill: egui::Color32, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 16.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, fill);
    let stroke = if selected {
        egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color)
    } else {
        egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color())
    };
    painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Outside);
    response
}

fn yes(on: bool) -> &'static str {
    if on {
        "yes"
    } else {
        "no"
    }
}

/// The names of the bits of `value` that are set (`names[0]` is bit 7).
fn bits(value: u8, names: [&str; 8]) -> String {
    let set: Vec<&str> = names
        .iter()
        .enumerate()
        .filter(|(i, name)| !name.is_empty() && value & (0x80 >> i) != 0)
        .map(|(_, name)| *name)
        .collect();
    if set.is_empty() {
        "-".to_owned()
    } else {
        set.join(" ")
    }
}

fn run(bus: &mut CommandBus, id: &str, arg: Arg) {
    if let Ok(command) = Command::with_arg(id, arg) {
        bus.push(command);
    }
}

/// A LED: lit (green, or red for a recording) or off.
fn led(ui: &mut egui::Ui, lit: bool, color: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    let fill = if lit {
        color
    } else {
        ui.visuals().widgets.noninteractive.bg_fill
    };
    ui.painter().circle_filled(rect.center(), 4.0, fill);
    ui.painter().circle_stroke(
        rect.center(),
        4.0,
        egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color()),
    );
    response
}

mod asic;
mod code;
mod crtc;
mod disc_view;
mod fdc;
mod ga;
mod memory_map;
mod ppi;
mod sector_rows;
mod status_bar;
mod tape;
mod z80;

pub use asic::*;
pub use code::*;
pub use crtc::*;
pub use disc_view::DiscView;
pub use fdc::*;
pub use ga::*;
pub use memory_map::*;
pub use ppi::*;
pub use status_bar::*;
pub use tape::*;
pub use z80::*;
