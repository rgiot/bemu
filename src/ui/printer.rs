//! Debug > View > Printer: what the DMP2000 printed, in two tabs: the text (ASCII: the graphics and
//! the styles are lost) and the paper (the sheets as they look, in a scroll area as they are long).

use std::time::Duration;

use crate::commands::CommandBus;
use crate::dmp2000::Dmp2000;
use crate::motherboard::Motherboard;

/// The most often the text and the sheets are drawn again while the CPC prints (a sheet takes a
/// moment to render).
const REFRESH_SECONDS: f64 = 0.3;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Tab {
    #[default]
    Text,
    Paper,
}

/// The window's state: the tab, and what was drawn of the printout.
pub struct PrinterWindow {
    tab: Tab,
    /// The sheets as wide as the window (else at half their printed size: 120 dpi).
    fit_width: bool,
    /// The printer's bytes the text was made from, and when.
    text_seen: usize,
    text: String,
    /// The printer's bytes the sheets were drawn from, when, and the sheets (their number, their
    /// picture): only the sheet being printed is drawn again.
    sheets_seen: usize,
    sheets_at: f64,
    sheets: Vec<(usize, egui::TextureHandle)>,
}

impl Default for PrinterWindow {
    fn default() -> Self {
        Self {
            tab: Tab::Text,
            fit_width: true,
            text_seen: 0,
            text: String::new(),
            sheets_seen: 0,
            sheets_at: f64::NEG_INFINITY,
            sheets: Vec::new(),
        }
    }
}

/// The picture of an RGB sheet at half its size: its 2x2 pixels averaged (a sheet is 2280 x 2640
/// pixels, more than some graphics cards take as a texture, and far more than a window shows).
fn half_size(rgb: &[u8], width: usize, height: usize) -> ([usize; 2], Vec<u8>) {
    let (half_width, half_height) = (width / 2, height / 2);
    let mut half = Vec::with_capacity(half_width * half_height * 3);
    for y in 0..half_height {
        for x in 0..half_width {
            for channel in 0..3 {
                let at = |dx: usize, dy: usize| {
                    u16::from(rgb[((2 * y + dy) * width + 2 * x + dx) * 3 + channel])
                };
                half.push(((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4) as u8);
            }
        }
    }
    ([half_width, half_height], half)
}

pub fn show(ui: &mut egui::Ui, mb: &Motherboard, state: &mut PrinterWindow, bus: &mut CommandBus) {
    let Some(printer) = mb
        .printer_port()
        .and_then(|port| port.device_as::<Dmp2000>())
    else {
        ui.weak("No printer: plug the DMP2000 (Expansions > DMP2000).");
        return;
    };
    let received = printer.raw().len();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, Tab::Text, "Text");
        ui.selectable_value(&mut state.tab, Tab::Paper, "Paper");
        ui.separator();
        ui.weak(format!("{received} bytes"));
        if ui
            .button("Save printout…")
            .on_hover_text("The text, the bytes the CPC sent and a PNG for each sheet")
            .clicked()
        {
            bus.push_id("printer-save-printout");
        }
    });
    ui.separator();
    match state.tab {
        Tab::Text => {
            if received != state.text_seen {
                state.text = printer.text();
                state.text_seen = received;
            }
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(state.text.clone());
                }
                ui.weak(format!("{} lines", state.text.lines().count()));
            });
            egui::ScrollArea::both()
                .id_salt("printer-text")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // read-only, but selectable
                    ui.add(
                        egui::TextEdit::multiline(&mut state.text.as_str())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
        }
        Tab::Paper => {
            let now = ui.input(|input| input.time);
            if received != state.sheets_seen {
                if now - state.sheets_at >= REFRESH_SECONDS {
                    refresh_sheets(ui, printer, state);
                    state.sheets_seen = received;
                    state.sheets_at = now;
                } else {
                    // printing: draw again soon
                    ui.ctx()
                        .request_repaint_after(Duration::from_secs_f64(REFRESH_SECONDS));
                }
            }
            ui.checkbox(&mut state.fit_width, "Fit the width");
            if state.sheets.is_empty() {
                ui.weak("Nothing printed yet.");
                return;
            }
            egui::ScrollArea::both()
                .id_salt("printer-paper")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (index, texture) in &state.sheets {
                        ui.label(format!("Sheet {}", index + 1));
                        let size = texture.size_vec2();
                        let scale = if state.fit_width {
                            (ui.available_width() - 8.0).max(120.0) / size.x
                        } else {
                            1.0
                        };
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            texture.id(),
                            size * scale,
                        )));
                        ui.add_space(8.0);
                    }
                });
        }
    }
}

/// The sheets with ink drawn: the ones not drawn yet, and the last (still printed on).
fn refresh_sheets(ui: &egui::Ui, printer: &Dmp2000, state: &mut PrinterWindow) {
    let inked: Vec<usize> = printer.inked_pages().map(|(index, _)| index).collect();
    // a new machine, or a paper that went back: what is not on the paper any more goes
    state.sheets.retain(|(index, _)| inked.contains(index));
    let last = inked.last().copied();
    for (index, page) in printer.inked_pages() {
        let drawn = state.sheets.iter().position(|(drawn, _)| *drawn == index);
        if drawn.is_some() && Some(index) != last {
            continue;
        }
        let (size, rgb) = half_size(&page.to_rgb(), page.width() as usize, page.height() as usize);
        let image = egui::ColorImage::from_rgb(size, &rgb);
        match drawn {
            Some(at) => state.sheets[at].1.set(image, egui::TextureOptions::LINEAR),
            None => {
                let texture = ui.ctx().load_texture(
                    format!("printer-sheet-{index}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                state.sheets.push((index, texture));
            }
        }
    }
    state.sheets.sort_by_key(|(index, _)| *index);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motherboard::MotherboardBuilder;

    /// The window draws what the printer printed, in both tabs, and says when there is no printer.
    #[test]
    fn the_text_and_the_sheets_are_drawn() {
        let mut state = PrinterWindow::default();
        let mut bus = CommandBus::default();
        let ctx = egui::Context::default();
        let mut draw = |mb: &Motherboard, state: &mut PrinterWindow, time: f64| {
            let input = egui::RawInput { time: Some(time), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| show(ui, mb, state, &mut bus));
            });
        };

        let mut mb = MotherboardBuilder::default().build();
        draw(&mb, &mut state, 0.0);
        assert!(state.text.is_empty() && state.sheets.is_empty(), "no printer");

        let mut mb = MotherboardBuilder::default().plug_dmp2000().build();
        let printer = mb.printer_port_mut().unwrap().device_as_mut::<Dmp2000>().unwrap();
        for byte in b"HELLO PAPER\r\n" {
            printer.receive(*byte);
        }
        draw(&mb, &mut state, 1.0);
        assert!(state.text.contains("HELLO PAPER"), "{:?}", state.text);
        assert!(state.sheets.is_empty(), "the paper is drawn when its tab is open");
        state.tab = Tab::Paper;
        draw(&mb, &mut state, 2.0);
        assert_eq!(state.sheets.len(), 1);
        let size = state.sheets[0].1.size();
        assert_eq!(size, [1140, 1320], "half of the 2280 x 2640 sheet");
    }

    #[test]
    fn a_picture_is_halved_by_averaging() {
        let rgb: Vec<u8> = [0u8, 100, 200, 255].iter().flat_map(|v| [*v; 3]).collect();
        let (size, half) = half_size(&rgb, 2, 2);
        assert_eq!((size, half), ([1, 1], vec![138; 3]));
    }
}
