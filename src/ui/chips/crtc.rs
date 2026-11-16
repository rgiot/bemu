//! The CRTC window: its registers, its counters and the beam.

use super::*;

/// The CRTC: the counters' flags and what the registers make (timings, screen address).
pub fn crtc(ui: &mut egui::Ui, mb: &Motherboard) {
    let crtc = mb.crtc();
    let r = crtc.registers();
    let counters = crtc.counters();
    let line_us = u32::from(r[0]) + 1;
    let char_lines = u32::from(r[9] & 0x1f) + 1;
    let frame_lines = (u32::from(r[4] & 0x7f) + 1) * char_lines + u32::from(r[5] & 0x1f);
    let frame_us = frame_lines * line_us;
    let vsync = match r[3] >> 4 {
        0 => 16,
        lines => lines,
    };
    let ma = u16::from(r[12] & 0x3f) << 8 | u16::from(r[13]);
    let page = (ma >> 12) & 3;
    let size_32k = (ma >> 10) & 3 == 3;
    let start = 0x4000 * page + ((ma & 0x3ff) << 1);
    egui::Grid::new("crtc-decoded").striped(true).show(ui, |ui| {
        ui.label("Flags");
        ui.monospace(format!(
            "HSYNC {} (HSC {}), VSYNC {} (VSC {}), adjust {}",
            yes(counters.hsync),
            counters.hsc,
            yes(counters.vsync),
            counters.vsc,
            yes(counters.in_vertical_adjust)
        ));
        ui.end_row();
        ui.label("Line");
        ui.monospace(format!(
            "{line_us} µs, {} characters shown, HSYNC at {} for {} µs",
            r[1],
            r[2],
            r[3] & 0x0f
        ));
        ui.end_row();
        ui.label("Frame");
        ui.monospace(format!(
            "{frame_lines} lines ({:.2} Hz), {} character rows of {char_lines} lines shown, VSYNC at row {} for {vsync} lines",
            1_000_000.0 / f64::from(frame_us.max(1)),
            r[6],
            r[7]
        ));
        ui.end_row();
        ui.label("Screen");
        ui.monospace(format!(
            "&{start:04X} ({}K, page &{:04X}, offset &{:03X})",
            if size_32k { 32 } else { 16 },
            0x4000 * page,
            (ma & 0x3ff) << 1
        ));
        ui.end_row();
        ui.label("Cursor (R14-R15)");
        ui.monospace(format!("&{:02X}{:02X}", r[14], r[15]));
        ui.end_row();
        ui.label("Light pen (R16-R17)");
        ui.monospace(format!("&{:02X}{:02X}", r[16], r[17]));
        ui.end_row();
    });
}

/// The CRTC's registers as boxes (cpclib's VS Code CRTC view): the name over the value, the
/// selected register outlined, and the registers of a known-bad setting (cpclib-dap's
/// `validate_crtc`) bordered red (the picture is lost) or yellow (not what is likely meant),
/// the reasons below.
pub fn crtc_registers(ui: &mut egui::Ui, mb: &Motherboard) {
    use cpclib_dap::inspect::CrtcSeverity;
    let crtc = mb.crtc();
    let registers = crtc.registers();
    let selected = usize::from(crtc.selected_register());
    let warnings = cpclib_dap::inspect::validate_crtc(&registers);
    const ERROR: egui::Color32 = egui::Color32::from_rgb(220, 60, 60);
    const WARNING: egui::Color32 = egui::Color32::from_rgb(220, 180, 40);
    egui::Grid::new("crtc-boxes")
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for (r, value) in registers.iter().enumerate() {
                let name = format!("R{r}");
                let mine: Vec<_> = warnings
                    .iter()
                    .filter(|w| w.registers.contains(&name.as_str()))
                    .collect();
                let border = if mine.iter().any(|w| w.severity == CrtcSeverity::Error) {
                    Some(ERROR)
                } else if mine.is_empty() {
                    None
                } else {
                    Some(WARNING)
                };
                let stroke = match border {
                    Some(color) => egui::Stroke::new(2.0_f32, color),
                    None if r == selected => {
                        egui::Stroke::new(1.5_f32, ui.visuals().selection.stroke.color)
                    }
                    None => egui::Stroke::new(
                        1.0_f32,
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                    ),
                };
                let fill = match border {
                    Some(color) => color.gamma_multiply(0.18),
                    None => ui.visuals().extreme_bg_color,
                };
                let response = egui::Frame::new()
                    .stroke(stroke)
                    .fill(fill)
                    .corner_radius(3.0)
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.set_min_width(40.0);
                        ui.vertical_centered(|ui| {
                            ui.small(name.as_str());
                            ui.monospace(format!("&{value:02X}"));
                        });
                    })
                    .response;
                let mut tooltip = format!("{name} = {value} (&{value:02X})");
                if r == selected {
                    tooltip.push_str(", the register selected");
                }
                for warning in &mine {
                    tooltip.push_str(&format!("\n{}", warning.message));
                }
                response.on_hover_text(tooltip);
                if r % 6 == 5 {
                    ui.end_row();
                }
            }
        });
    for warning in &warnings {
        let color = match warning.severity {
            CrtcSeverity::Error => ERROR,
            CrtcSeverity::Warning => WARNING,
        };
        ui.colored_label(
            color,
            format!("{}: {}", warning.registers.join(", "), warning.message),
        );
    }
}

/// The memory around each register that points somewhere (cpclib's VS Code `-mv all,follow`):
/// two rows of 16 bytes, the byte pointed at outlined, the symbol of the address named.
pub fn follow_registers(
    ui: &mut egui::Ui,
    mb: &Motherboard,
    describe: impl Fn(u16) -> Option<String>,
    read: impl Fn(u16) -> u8,
) {
    let registers = [
        ("PC", mb.pc()),
        ("SP", mb.sp()),
        ("HL", mb.hl()),
        ("DE", mb.de()),
        ("BC", mb.bc()),
        ("IX", mb.ix()),
        ("IY", mb.iy()),
    ];
    let pointed = ui.visuals().selection.stroke.color;
    for (name, value) in registers {
        ui.horizontal(|ui| {
            ui.strong(name);
            ui.monospace(format!("&{value:04X}"));
            if let Some(symbol) = describe(value) {
                ui.weak(symbol);
            }
        });
        let start = value & 0xfff0;
        egui::Grid::new(("follow", name))
            .spacing([6.0, 0.0])
            .min_col_width(0.0)
            .show(ui, |ui| {
                for row in 0..2u16 {
                    let at = start.wrapping_add(row * 16);
                    ui.monospace(format!("{at:04X}"));
                    for column in 0..16u16 {
                        let address = at.wrapping_add(column);
                        let byte = read(address);
                        let mut text = egui::RichText::new(format!("{byte:02X}")).monospace();
                        if address == value {
                            text = text.color(pointed).strong().underline();
                        }
                        ui.label(text);
                    }
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
    }
}

/// The beam in the frame as the CRTC counts it: the frame's lines and microseconds, the
/// displayed area (R1 × R6 characters), the HSYNC column and the VSYNC band, the beam's
/// crosshair (the paused picture's magenta), and where it is in numbers.
pub fn beam_map(ui: &mut egui::Ui, mb: &Motherboard) {
    const X: f32 = 4.0; // pixels per µs
    const Y: f32 = 1.0; // pixels per line
    let crtc = mb.crtc();
    let r = crtc.registers();
    let c = crtc.counters();
    let char_lines = u32::from(r[9] & 0x1f) + 1;
    let width = u32::from(r[0]) + 1;
    let lines = ((u32::from(r[4] & 0x7f) + 1) * char_lines + u32::from(r[5] & 0x1f)).max(1);
    let line = if c.in_vertical_adjust {
        (u32::from(r[4] & 0x7f) + 1) * char_lines + u32::from(c.vtac)
    } else {
        u32::from(c.vcc) * char_lines + u32::from(c.vlc)
    };
    let x = u32::from(c.hcc);
    ui.monospace(format!(
        "line {line} / {lines} (row C4 {}, raster C9 {}{}), µs {x} / {width} (C0)",
        c.vcc,
        c.vlc,
        if c.in_vertical_adjust {
            format!(", adjust {}", c.vtac)
        } else {
            String::new()
        }
    ));
    ui.monospace(format!(
        "{} NOPs since the frame started, {} to its end",
        line * width + x,
        (lines * width).saturating_sub(line * width + x)
    ));
    // the drawing: the whole frame at most 128 µs × 512 lines
    let shown_w = width.min(128) as f32 * X;
    let shown_h = lines.min(512) as f32 * Y;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(shown_w, shown_h), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let at = |us: u32, line: u32| {
        rect.left_top() + egui::vec2(us.min(128) as f32 * X, line.min(512) as f32 * Y)
    };
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(30));
    // the displayed characters
    let shown = egui::Rect::from_min_max(
        at(0, 0),
        at(
            u32::from(r[1]).min(width),
            (u32::from(r[6] & 0x7f) * char_lines).min(lines),
        ),
    );
    painter.rect_filled(shown, 0.0, egui::Color32::from_rgb(30, 50, 110));
    // HSYNC: from R2, R3's low nibble µs (0 none)
    let hsync_w = u32::from(r[3] & 0x0f);
    if hsync_w > 0 {
        painter.rect_filled(
            egui::Rect::from_min_max(at(u32::from(r[2]), 0), at(u32::from(r[2]) + hsync_w, lines)),
            0.0,
            egui::Color32::from_rgb(90, 70, 20),
        );
    }
    // VSYNC: from row R7, R3's high nibble lines (0: 16)
    let vsync_lines = match u32::from(r[3] >> 4) {
        0 => 16,
        n => n,
    };
    let vsync_top = u32::from(r[7] & 0x7f) * char_lines;
    painter.rect_filled(
        egui::Rect::from_min_max(at(0, vsync_top), at(width, vsync_top + vsync_lines)),
        0.0,
        egui::Color32::from_rgb(110, 40, 40),
    );
    // the beam
    let magenta = egui::Color32::from_rgba_unmultiplied(255, 0, 255, 204);
    let p = at(x, line) + egui::vec2(X / 2.0, 0.0);
    painter.line_segment(
        [egui::pos2(p.x, rect.top()), egui::pos2(p.x, rect.bottom())],
        egui::Stroke::new(1.0_f32, magenta),
    );
    painter.line_segment(
        [egui::pos2(rect.left(), p.y), egui::pos2(rect.right(), p.y)],
        egui::Stroke::new(1.0_f32, magenta),
    );
    painter.rect_filled(
        egui::Rect::from_center_size(p, egui::vec2(6.0, 5.0)),
        0.0,
        magenta,
    );
    // a point of the frame under the pointer
    if let Some(pointer) = response.hover_pos() {
        let us = ((pointer.x - rect.left()) / X) as u32;
        let at_line = ((pointer.y - rect.top()) / Y) as u32;
        response.on_hover_text(format!(
            "line {at_line} (row {}, raster {}), µs {us}: NOP {} of the frame",
            at_line / char_lines,
            at_line % char_lines,
            at_line * width + us
        ));
    }
    ui.horizontal(|ui| {
        for (colour, name) in [
            (egui::Color32::from_rgb(30, 50, 110), "displayed (R1 × R6)"),
            (egui::Color32::from_rgb(90, 70, 20), "HSYNC (R2, R3)"),
            (egui::Color32::from_rgb(110, 40, 40), "VSYNC (R7, R3)"),
        ] {
            let (swatch, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(swatch, 2.0, colour);
            ui.weak(name);
        }
    });
}
