//! The paging form of the Memory and Disassembly windows, the paging itself being
//! [`crate::paging`]'s: follow the CPU's paging, or look through another; the map of the 4
//! blocks of 16K and the registers that page them.

pub use crate::paging::{bank_name, Paging};
use crate::motherboard::Motherboard;
use crate::paging::Source;

/// The form: follow or not, the paging chosen, its map and the registers.
pub fn show(ui: &mut egui::Ui, id: &str, mb: &Motherboard, paging: &mut Paging) {
    // following, the form shows the emulator's paging (greyed), ready to be changed from there
    if paging.follow {
        paging.synchronise(mb);
    }
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut paging.writes, false, "Read")
                    .on_hover_text("What the CPU reads: the ROMs where they are paged");
                ui.selectable_value(&mut paging.writes, true, "Write")
                    .on_hover_text("What a write reaches: the RAM, under the ROMs too");
                super::widgets::rule(ui);
                ui.checkbox(&mut paging.follow, "Follow the emulator's paging");
                if ui
                    .button("Resynchronise")
                    .on_hover_text("Take the emulator's paging as this window's")
                    .clicked()
                {
                    paging.synchronise(mb);
                }
            });
            ui.add_enabled_ui(!paging.follow, |ui| {
                egui::Grid::new((id, "paging"))
                    .spacing([6.0, 3.0])
                    .show(ui, |ui| {
                        ui.label("RAM");
                        // one field, as WinAPE's and Caprice Forever's: the PAL's value and its banks
                        let pages =
                            (mb.physical_bank_count().saturating_sub(4) / 4).clamp(1, 64) as u8;
                        egui::ComboBox::from_id_salt((id, "ram"))
                            .selected_text(paging.ram_text())
                            .height(360.0)
                            .show_ui(ui, |ui| {
                                for page in 0..pages {
                                    for config in 0..8 {
                                        let choice = Paging {
                                            page,
                                            ram_config: config,
                                            ..paging.clone()
                                        };
                                        let chosen =
                                            paging.page == page && paging.ram_config == config;
                                        if ui.selectable_label(chosen, choice.ram_text()).clicked()
                                        {
                                            paging.page = page;
                                            paging.ram_config = config;
                                        }
                                    }
                                }
                            });
                        ui.end_row();
                        ui.label("Lower ROM");
                        if mb.any_cartridge().is_some() {
                            // a CPC+ (RMR2): a cartridge page, at &0000, &4000 or &8000
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut paging.lower_rom, "cartridge page");
                                ui.add(egui::DragValue::new(&mut paging.lower_page).range(0..=7));
                                egui::ComboBox::from_id_salt((id, "lower-block"))
                                    .selected_text(format!("at &{:04X}", u16::from(paging.lower_block) * 0x4000))
                                    .show_ui(ui, |ui| {
                                        for block in 0..3u8 {
                                            ui.selectable_value(
                                                &mut paging.lower_block,
                                                block,
                                                format!("at &{:04X}", u16::from(block) * 0x4000),
                                            );
                                        }
                                    });
                            });
                            ui.end_row();
                            ui.label("ASIC");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut paging.registers, "registers at &4000-&7FFF");
                                let unlocked = mb.ga().plus().is_some_and(|plus| plus.unlocked);
                                if unlocked {
                                    ui.weak("(unlocked)");
                                } else {
                                    ui.weak("(locked)").on_hover_text(
                                        "Locked, the CPU cannot change RMR2: the registers stay \
                                         where the program left them (paged in before it \
                                         locked again, they are still there). This window can \
                                         show them either way.",
                                    );
                                }
                            });
                        } else {
                            ui.checkbox(&mut paging.lower_rom, "in &0000-&3FFF");
                        }
                        ui.end_row();
                        ui.label("Upper ROM");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut paging.upper_rom, "in &C000-&FFFF, slot");
                            ui.add(egui::DragValue::new(&mut paging.upper_number).range(0..=255))
                                .on_hover_text(if mb.any_cartridge().is_some() {
                                    "128-159: the cartridge's pages 0-31; another number without a ROM of its own: BASIC (page 1)"
                                } else {
                                    "An empty slot reads as BASIC (slot 0)"
                                });
                        });
                        ui.end_row();
                    });
            });
        });
        // (a separator here would stretch to the whole scroll area's height)
        ui.add_space(16.0);
        ui.vertical(|ui| map(ui, mb, paging));
    });
}

/// A cartridge page by its number: `Cart 3`, and for the system cartridge what it holds
/// (`Cart 1 BASIC`).
pub fn cartridge_page_name(mb: &Motherboard, page: u8) -> String {
    // the system cartridge: the firmware, BASIC, Burnin' Rubber's pages, AMSDOS
    let system = mb.cartridge().is_none();
    match (system, page) {
        (true, 0) => "Cart 0 OS".to_owned(),
        (true, 1) => "Cart 1 BASIC".to_owned(),
        (true, 3) => "Cart 3 DOS".to_owned(),
        _ => format!("Cart {page}"),
    }
}

/// The map of a paging: what each 16K block reads (ROM red, RAM green) and, under a ROM, the RAM
/// a write reaches; the RAM's value and the registers under it.
pub fn map(ui: &mut egui::Ui, mb: &Motherboard, paging: &Paging) {
    const ROM: egui::Color32 = egui::Color32::from_rgb(200, 70, 50);
    const RAM: egui::Color32 = egui::Color32::from_rgb(60, 150, 70);
    const REGISTERS: egui::Color32 = egui::Color32::from_rgb(130, 80, 190);
    // the map: what each block reads (ROM red, RAM green) and, when they differ, where
    // writes go (a second cell); a block that reads and writes the same RAM has one cell
    // as wide as both
    const CELL: f32 = 96.0;
    const GAP: f32 = 4.0;
    let cell = |ui: &mut egui::Ui, text: String, fill: egui::Color32, width: f32| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, fill);
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );
    };
    ui.horizontal(|ui| {
        ui.add_space(52.0);
        ui.weak("read");
        ui.add_space(CELL - 22.0);
        ui.weak("write");
    });
    for block in 0..4u16 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            ui.monospace(format!("&{:04X}", block * 0x4000));
            let source = paging.source(mb, block);
            match source {
                Source::Ram(bank) => cell(ui, bank_name(bank), RAM, 2.0 * CELL + GAP),
                // reads and writes the registers (the RAM where none answers)
                Source::Registers => cell(ui, "ASIC regs".to_owned(), REGISTERS, 2.0 * CELL + GAP),
                _ => {
                    let text = match source {
                        Source::UpperRom(slot) => format!("ROM {slot}"),
                        Source::Cartridge(page) => cartridge_page_name(mb, page),
                        _ => "Firmware".to_owned(),
                    };
                    cell(ui, text, ROM, CELL);
                    // under a ROM, writes go to RAM
                    let bank = paging.write_bank(mb, block);
                    cell(ui, bank_name(bank), RAM, CELL);
                }
            }
        });
    }
    // the RAM's value as the Gate Array is given it (&7Fxx: &C0 + 8 × page + config)
    // following, the emulator's value (the window's own fields may be older)
    let shown = if paging.follow {
        let mut shown = paging.clone();
        shown.synchronise(mb);
        shown
    } else {
        paging.clone()
    };
    if let Some(mmr) = shown.mmr() {
        ui.label(format!("RAM &{mmr:02X}"))
            .on_hover_text("The value written to &7Fxx: &C0 + 8 × page + configuration");
    }
    ui.weak(format!(
        "RMR &{:02X} · upper ROM &{:02X}",
        mb.ga().rmr(),
        mb.upper_rom_number()
    ));
    // the CPC+'s secondary mapping: the lower ROM's cartridge page and where it is, or
    // the ASIC's registers at &4000
    if let Some(plus) = mb.ga().plus() {
        let rmr2 = plus.rmr2;
        let place = match rmr2 >> 3 & 3 {
            0 => "lower ROM at &0000".to_owned(),
            1 => "lower ROM at &4000".to_owned(),
            2 => "lower ROM at &8000".to_owned(),
            _ => "ASIC registers at &4000".to_owned(),
        };
        ui.weak(format!(
            "RMR2 &{rmr2:02X} · cartridge page {} · {place}",
            rmr2 & 7
        ));
    }
}
