//! The Gate Array window: its mode, its inks, its state.

use super::*;

/// The Gate Array: its mode (changed with a command), its pens (each opens the 27 inks), its
/// registers. A CPC+ has the ASIC's instead: [`asic`].
pub fn gate_array(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    ga_mode(ui, mb, bus);
    ga_inks(ui, mb, bus);
    ga_state(ui, mb);
}

/// The chip and the screen mode, a button per mode (`ga-set-mode`, from the next line).
pub(super) fn ga_mode(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    let ga = mb.ga();
    let mode = |mode: crate::ga::VideoMode| mode as u8;
    ui.horizontal(|ui| {
        ui.label(format!(
            "{}: mode",
            match (ga.plus(), ga.gate_array_type()) {
                (Some(_), _) => "ASIC",
                (None, crate::ga::GateArrayType::Ga40010) => "40010",
                _ => "40007/40008",
            }
        ));
        let active = mode(ga.active_video_mode());
        for wanted in 0..4u8 {
            if ui
                .selectable_label(active == wanted, wanted.to_string())
                .on_hover_text(format!("Mode {wanted} from the next line"))
                .clicked()
                && active != wanted
            {
                if let Ok(command) = Command::with_arg("ga-set-mode", Arg::Text(wanted.to_string()))
                {
                    bus.push(command);
                }
            }
        }
        if mode(ga.next_video_mode()) != active {
            ui.weak(format!("(next line: mode {})", mode(ga.next_video_mode())));
        }
    });
}

/// The 16 pens and the border, each a swatch that opens the 27 inks (`ga-set-ink`).
fn ga_inks(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    use crate::ui::widgets::{ink_picker, Picked};
    let ga = mb.ga();
    let palette = ga.palette();
    let pen = ga.selected_pen();
    let values: Vec<u32> = palette.iter().map(|ink| u32::from(ink.number())).collect();
    let heat = crate::ui::heat::watch(ui, "ga-inks", 0, mb, &values);
    ui.weak("A click on a pen gives it another ink, until the program writes it again")
        .on_hover_text("The firmware sends its own inks again at each flash period: a change lasts under BASIC only while the firmware is stopped");
    egui::Grid::new("ga-inks")
        .spacing([4.0, 6.0])
        .show(ui, |ui| {
            for (index, ink) in palette.iter().enumerate() {
                ui.vertical_centered(|ui| {
                    let label = if index == 16 {
                        "B".to_owned()
                    } else {
                        index.to_string()
                    };
                    // the pen number lit when its ink changed lately
                    ui.label(crate::ui::heat::text(
                        egui::RichText::new(label).small(),
                        heat.heat(index),
                    ));
                    let tooltip = format!(
                        "pen {index}: ink {} (hardware &{:02X})",
                        ink.number(),
                        ink.hardware_number()
                    );
                    let id = egui::Id::unique(("ga-pen", index));
                    if let Some(Picked::Ink(new)) = ink_picker(
                        ui,
                        id,
                        ink.number(),
                        usize::from(pen) == index,
                        false,
                        &tooltip,
                    ) {
                        let pen = if index == 16 {
                            "B".to_owned()
                        } else {
                            index.to_string()
                        };
                        if let Ok(command) =
                            Command::with_arg("ga-set-ink", Arg::Text(format!("{pen} {new}")))
                        {
                            bus.push(command);
                        }
                    }
                });
                if index == 8 {
                    ui.end_row();
                }
            }
        });
}

/// The selected pen, RMR decoded, the upper ROM, the interrupt counter.
pub(super) fn ga_state(ui: &mut egui::Ui, mb: &Motherboard) {
    let ga = mb.ga();
    let pen = ga.selected_pen();
    let rmr = ga.rmr();
    egui::Grid::new("ga-state").striped(true).show(ui, |ui| {
        ui.label("Selected pen");
        ui.monospace(if pen == 16 {
            "border".to_owned()
        } else {
            pen.to_string()
        });
        ui.end_row();
        ui.label("RMR");
        ui.monospace(format!(
            "&{rmr:02X}: mode {}, lower ROM {}, upper ROM {}",
            rmr & 3,
            if rmr & 4 == 0 { "on" } else { "off" },
            if rmr & 8 == 0 { "on" } else { "off" },
        ));
        ui.end_row();
        ui.label("Upper ROM");
        ui.monospace(format!("{}", mb.upper_rom_number()));
        ui.end_row();
        ui.label("Interrupt counter (R52)");
        ui.monospace(format!("{} / 52", ga.r52()));
        ui.end_row();
    });
}
