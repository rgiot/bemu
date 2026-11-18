//! Debug > View > Printer: what the DMP2000 printed, in two tabs: the text (ASCII: the graphics and
//! the styles are lost; drawn with the CPC's own font, or in the interface's monospace one to select
//! it) and the paper (the sheets as they look, in a scroll area as they are long).

use std::hash::{Hash, Hasher};
use std::time::Duration;

use crate::commands::CommandBus;
use crate::dmp2000::Dmp2000;
use crate::motherboard::Motherboard;

/// The most often the text and the sheets are drawn again while the CPC prints (a sheet takes a
/// moment to render).
const REFRESH_SECONDS: f64 = 0.3;

/// The text's lines drawn in one picture (a picture is at most 8 K high on some graphics cards).
const CHUNK_LINES: usize = 128;
/// The most columns drawn (the CPC's font is 8 pixels wide).
const MAX_COLUMNS: usize = 255;
/// A pixel of the CPC's font, in points.
const FONT_SCALE: f32 = 2.0;

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
    /// The text in the CPC's font (a picture, so not selectable: Copy gives it), else in the
    /// interface's monospace font.
    cpc_font: bool,
    /// The text's pictures: `CHUNK_LINES` lines each, with a hash of the lines they show (only the
    /// ones that changed are drawn again).
    chunks: Vec<(u64, egui::TextureHandle)>,
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
            cpc_font: true,
            chunks: Vec::new(),
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
                ui.checkbox(&mut state.cpc_font, "CPC font")
                    .on_hover_text("The firmware's characters; unchecked, a font to select the text in");
                ui.weak(format!("{} lines", state.text.lines().count()));
            });
            if state.cpc_font {
                update_chunks(ui.ctx(), &state.text, &mut state.chunks);
            }
            egui::ScrollArea::both()
                .id_salt("printer-text")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if state.cpc_font {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let ink = ui.visuals().text_color();
                        for (_, texture) in &state.chunks {
                            ui.add(
                                egui::Image::new(egui::load::SizedTexture::new(
                                    texture.id(),
                                    texture.size_vec2() * FONT_SCALE,
                                ))
                                .tint(ink),
                            );
                        }
                    } else {
                        // read-only, but selectable
                        ui.add(
                            egui::TextEdit::multiline(&mut state.text.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    }
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

/// The glyph of `character` in the CPC's firmware font (the lower ROM's &3800: 8 rows, bit 7 the
/// leftmost pixel); a character the font has not is a question mark.
fn glyph(character: char) -> [u8; 8] {
    let code = u8::try_from(character).unwrap_or(b'?');
    let at = 0x3800 + usize::from(code) * 8;
    let mut rows = [0; 8];
    rows.copy_from_slice(&crate::roms::OS6128[at..at + 8]);
    rows
}

/// `lines` in the CPC's font: white on transparent, one character a glyph (tinted when drawn).
fn text_image(lines: &[&str]) -> egui::ColorImage {
    let columns = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(1, MAX_COLUMNS);
    let width = columns * 8;
    let mut pixels = vec![egui::Color32::TRANSPARENT; width * lines.len().max(1) * 8];
    for (row, line) in lines.iter().enumerate() {
        for (column, character) in line.chars().take(columns).enumerate() {
            for (y, bits) in glyph(character).into_iter().enumerate() {
                for x in 0..8 {
                    if bits & (0x80 >> x) != 0 {
                        pixels[(row * 8 + y) * width + column * 8 + x] = egui::Color32::WHITE;
                    }
                }
            }
        }
    }
    egui::ColorImage::new([width, lines.len().max(1) * 8], pixels)
}

/// The pictures of `text`'s lines, `CHUNK_LINES` each: only those whose lines changed are drawn
/// again.
fn update_chunks(ctx: &egui::Context, text: &str, chunks: &mut Vec<(u64, egui::TextureHandle)>) {
    let lines: Vec<&str> = text.lines().collect();
    let groups: Vec<&[&str]> = lines.chunks(CHUNK_LINES).collect();
    chunks.truncate(groups.len());
    for (index, group) in groups.into_iter().enumerate() {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        group.hash(&mut hasher);
        let hash = hasher.finish();
        match chunks.get_mut(index) {
            Some((drawn, _)) if *drawn == hash => {}
            Some((drawn, texture)) => {
                texture.set(text_image(group), egui::TextureOptions::NEAREST);
                *drawn = hash;
            }
            None => chunks.push((
                hash,
                ctx.load_texture(format!("printer-text-{index}"), text_image(group), egui::TextureOptions::NEAREST),
            )),
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
        assert_eq!(state.chunks.len(), 1, "the text in the CPC's font");
        assert_eq!(state.chunks[0].1.size()[1], 8, "a line, 8 pixels high");
        assert!(state.sheets.is_empty(), "the paper is drawn when its tab is open");
        state.tab = Tab::Paper;
        draw(&mb, &mut state, 2.0);
        assert_eq!(state.sheets.len(), 1);
        let size = state.sheets[0].1.size();
        assert_eq!(size, [1140, 1320], "half of the 2280 x 2640 sheet");
    }

    /// A character is the firmware's glyph, pixel for pixel; an unknown one a question mark.
    #[test]
    fn the_text_is_drawn_with_the_cpc_font() {
        let image = text_image(&["A\u{20ac}"]);
        assert_eq!(image.size, [16, 8]);
        let rows = |column: usize, glyph: [u8; 8]| {
            (0..8)
                .map(|y| {
                    (0..8).fold(0u8, |bits, x| {
                        bits << 1 | u8::from(image.pixels[y * 16 + column * 8 + x] == egui::Color32::WHITE)
                    }) == glyph[y]
                })
                .all(|same| same)
        };
        assert!(rows(0, glyph('A')) && glyph('A') != [0; 8]);
        assert!(rows(1, glyph('?')));
    }

    #[test]
    fn a_picture_is_halved_by_averaging() {
        let rgb: Vec<u8> = [0u8, 100, 200, 255].iter().flat_map(|v| [*v; 3]).collect();
        let (size, half) = half_size(&rgb, 2, 2);
        assert_eq!((size, half), ([1, 1], vec![138; 3]));
    }
}
