//! The ASIC window (CPC+): the Gate Array's, its palette, registers, DMA and sprites.

use super::*;

/// The ASIC of a CPC+ (the Gate Array's part, then its own): the mode and the Gate Array's
/// registers, the 32 palette entries (12 bits: pens, border, sprite colours), the raster
/// interrupt, the split, the soft scroll, RMR2, the DMA channels, the hardware sprites.
pub fn asic(ui: &mut egui::Ui, mb: &Motherboard, bus: &mut CommandBus) {
    let (Some(asic), Some(video)) = (mb.asic(), mb.ga().plus()) else {
        ui.weak("a CPC+ has one (Machine > Platform)");
        return;
    };
    ga_mode(ui, mb, bus);
    // the palette: pens 0-15, the border, the sprite colours 1-15
    ui.strong("Palette (12 bits: &GRB)");
    let pen = usize::from(mb.ga().selected_pen());
    let values: Vec<u32> = (0..32).map(|i| u32::from(video.palette_entry(i))).collect();
    let heat = crate::ui::heat::watch(ui, "asic-palette", 0, mb, &values);
    egui::Grid::new("asic-palette")
        .spacing([4.0, 6.0])
        .show(ui, |ui| {
            for index in 0..32 {
                let entry = video.palette_entry(index);
                ui.vertical_centered(|ui| {
                    let label = match index {
                        16 => "B".to_owned(),
                        17.. => format!("S{}", index - 16),
                        _ => index.to_string(),
                    };
                    // lit when its colour changed lately
                    ui.label(crate::ui::heat::text(
                        egui::RichText::new(label).small(),
                        heat.heat(index),
                    ));
                    swatch(ui, color(video.rgb(index)), index == pen).on_hover_text(format!(
                        "{}: &{entry:03X} (green {}, red {}, blue {})",
                        match index {
                            16 => "border".to_owned(),
                            17.. => format!("sprite colour {}", index - 16),
                            _ => format!("pen {index}"),
                        },
                        (entry >> 8) & 15,
                        (entry >> 4) & 15,
                        entry & 15
                    ));
                });
                // the pens, then the border and the sprite colours
                if index == 15 {
                    ui.end_row();
                }
            }
        });
    ga_state(ui, mb);
    crate::ui::widgets::rule(ui);
    let rmr2 = video.rmr2();
    let sscr = video.sscr();
    egui::Grid::new("asic").striped(true).show(ui, |ui| {
        ui.label("Registers");
        ui.monospace(if asic.unlocked() {
            "unlocked"
        } else {
            "locked"
        });
        ui.end_row();
        ui.label("Raster interrupt (PRI)");
        ui.monospace(match asic.pri() {
            0 => "off (the Gate Array's interrupts)".to_owned(),
            line => format!("line {line}"),
        });
        ui.end_row();
        ui.label("Split (SPLT, SSA)");
        ui.monospace(match asic.split() {
            Some((line, address)) => format!("after line {line}, from &{address:04X}"),
            None => "no".to_owned(),
        });
        ui.end_row();
        ui.label("Soft scroll (SSCR)");
        ui.monospace(format!(
            "&{sscr:02X}: {} pixels right, {} lines down{}",
            sscr & 15,
            (sscr >> 4) & 7,
            if sscr & 0x80 != 0 {
                ", border over the first byte"
            } else {
                ""
            }
        ));
        ui.end_row();
        ui.label("RMR2");
        ui.monospace(format!(
            "&{rmr2:02X}: cartridge page {} {}",
            rmr2 & 7,
            match (rmr2 >> 3) & 3 {
                0 => "at &0000",
                1 => "at &4000",
                2 => "at &8000",
                _ => "at &0000, the registers at &4000",
            }
        ));
        ui.end_row();
        ui.label("Interrupt vector (IVR)");
        ui.monospace(format!(
            "&{:02X}{}",
            asic.ivr() & 0xf8,
            if asic.ivr() & 1 != 0 {
                ", DMA interrupts cleared by the acknowledge"
            } else {
                ""
            }
        ));
        ui.end_row();
    });
    crate::ui::widgets::rule(ui);
    ui.strong("DMA (sound)");
    egui::Grid::new("asic-dma").striped(true).show(ui, |ui| {
        for heading in [
            "",
            "On",
            "Address",
            "Prescaler",
            "Pause",
            "Repeat",
            "Interrupt",
        ] {
            ui.weak(heading);
        }
        ui.end_row();
        for (n, channel) in asic.dma_channels().iter().enumerate() {
            ui.monospace(n.to_string());
            ui.monospace(yes(channel.enabled));
            ui.monospace(format!("&{:04X}", channel.address));
            ui.monospace(channel.prescaler.to_string());
            ui.monospace(channel.pause.to_string());
            ui.monospace(channel.loop_count.to_string());
            ui.monospace(yes(channel.interrupt));
            ui.end_row();
        }
    });
    crate::ui::widgets::rule(ui);
    ui.strong("Hardware sprites");
    asic_sprites(ui, video);
}

/// The 16 hardware sprites: each drawn in its colours (a transparent pixel checked), its
/// position and magnification; a hidden one (magnification 0) greyed.
fn asic_sprites(ui: &mut egui::Ui, video: &crate::asic::AsicVideo) {
    const PIXEL: f32 = 3.0;
    let zoom = |z: u8| match z {
        0 => "-".to_owned(),
        z => format!("x{}", 1 << (z - 1)),
    };
    egui::Grid::new("asic-sprites")
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            for n in 0..16 {
                let sprite = video.sprite(n);
                ui.vertical(|ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(16.0 * PIXEL, 16.0 * PIXEL),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter_at(rect);
                    for (i, &pen) in sprite.pixels.iter().enumerate() {
                        let (x, y) = ((i % 16) as f32, (i / 16) as f32);
                        let cell = egui::Rect::from_min_size(
                            rect.min + egui::vec2(x * PIXEL, y * PIXEL),
                            egui::vec2(PIXEL, PIXEL),
                        );
                        let fill = if pen == 0 {
                            // transparent: a checker
                            if (i % 16 + i / 16) % 2 == 0 {
                                egui::Color32::from_gray(40)
                            } else {
                                egui::Color32::from_gray(60)
                            }
                        } else {
                            color(video.rgb(16 + usize::from(pen & 15)))
                        };
                        painter.rect_filled(
                            cell,
                            0.0,
                            if sprite.shown() {
                                fill
                            } else {
                                fill.gamma_multiply(0.35)
                            },
                        );
                    }
                    response.on_hover_text(format!(
                        "sprite {n}: x {}, y {}, magnification {} x {}{}",
                        sprite.x,
                        sprite.y,
                        zoom(sprite.zoom_x),
                        zoom(sprite.zoom_y),
                        if sprite.shown() { "" } else { " (hidden)" }
                    ));
                    ui.small(format!(
                        "{n}: {},{} {}",
                        sprite.x,
                        sprite.y,
                        if sprite.shown() {
                            format!("{}{}", zoom(sprite.zoom_x), zoom(sprite.zoom_y))
                        } else {
                            "off".to_owned()
                        }
                    ));
                });
                if n % 8 == 7 {
                    ui.end_row();
                }
            }
        });
}
