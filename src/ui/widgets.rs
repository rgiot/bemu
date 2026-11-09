//! Small pieces the debugger's windows share: colour swatches, the ink picker, a hexadecimal
//! field, a register's go-to buttons.

/// The colour of firmware ink `firmware` (0-26).
pub fn ink_color(firmware: u8) -> egui::Color32 {
    let levels = crate::ga::RgbLevels::from(crate::ga::Ink::from(firmware.min(26)));
    let byte = |level: f32| (level * 255.0 / 100.0).round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(byte(levels.r()), byte(levels.g()), byte(levels.b()))
}

/// A coloured cell to click, outlined when `outlined` (or hovered).
pub fn swatch(ui: &mut egui::Ui, color: egui::Color32, outlined: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 16.0), egui::Sense::click());
    ui.painter().rect_filled(rect, 2.0, color);
    let stroke = if outlined || response.hovered() {
        egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color)
    } else {
        egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color())
    };
    ui.painter()
        .rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Outside);
    response
}

/// What the ink picker gave.
pub enum Picked {
    Ink(u8),
    /// Back to what it was (the Gate Array's).
    Reset,
}

/// A swatch of ink `ink` that opens, on a click, the 27 inks on two lines (under it, over the
/// rest of the window) and "Its own" when `resettable`. `id` keeps the open picker apart from the
/// others of the window.
pub fn ink_picker(
    ui: &mut egui::Ui,
    id: egui::Id,
    ink: u8,
    outlined: bool,
    resettable: bool,
    tooltip: &str,
) -> Option<Picked> {
    let response = swatch(ui, ink_color(ink), outlined).on_hover_text(tooltip);
    let open_id = id.with("open");
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
    if response.clicked() {
        open = !open;
    }
    let mut picked = None;
    if open {
        let area = egui::Area::new(id.with("inks"))
            .order(egui::Order::Foreground)
            .fixed_pos(response.rect.left_bottom() + egui::vec2(0.0, 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
                    // 27 inks: 14 then 13
                    for row in [0..14u8, 14..27] {
                        ui.horizontal(|ui| {
                            for ink in row {
                                if swatch(ui, ink_color(ink), false)
                                    .on_hover_text(format!("ink {ink}"))
                                    .clicked()
                                {
                                    picked = Some(Picked::Ink(ink));
                                }
                            }
                        });
                    }
                    if resettable && ui.small_button("Its own").clicked() {
                        picked = Some(Picked::Reset);
                    }
                });
            });
        // a choice, or a click elsewhere, closes it
        let clicked_elsewhere =
            ui.input(|i| i.pointer.any_click()) && !area.response.hovered() && !response.clicked();
        if picked.is_some() || clicked_elsewhere {
            open = false;
        }
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    picked
}

/// A hexadecimal value of `digits` digits, typed (`&` shown before it); `true` when changed.
pub fn hex_field(ui: &mut egui::Ui, value: &mut u16, digits: usize) -> bool {
    let mut edited = u32::from(*value);
    let max = (1u32 << (4 * digits as u32)) - 1;
    let changed = ui
        .add(
            egui::DragValue::new(&mut edited)
                .range(0..=max)
                .hexadecimal(digits, false, true)
                .prefix("&"),
        )
        .changed();
    if changed {
        *value = edited as u16;
    }
    changed
}
