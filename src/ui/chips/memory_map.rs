//! The memory map window: what each 16K page shows, the ROMs and the cartridge.

use super::*;

/// The memory map: the CPU's paging (the paging form's map), every 16K block of RAM and where
/// it is paged now, the ROMs in their slots (named from their header).
pub fn memory_map(ui: &mut egui::Ui, mb: &Motherboard) {
    use crate::memory::PhysicalAddress;
    crate::ui::paging::map(ui, mb, &crate::ui::paging::Paging::default());
    if let Some(banks) = mb.large_ram_expansion_bank_count() {
        ui.label(format!("4M card: {banks} pages of 64K"));
    }
    ui.add_space(6.0);
    // each block of RAM: where the CPU reads or writes it now
    let mut read_at: Vec<Vec<u16>> = vec![Vec::new(); usize::from(mb.physical_bank_count())];
    let mut written_at: Vec<Vec<u16>> = read_at.clone();
    for block in 0..4u16 {
        let base = block * 0x4000;
        if let PhysicalAddress::Ram { bank, .. } = mb.physical_address(base) {
            if let Some(at) = read_at.get_mut(usize::from(bank)) {
                at.push(base);
            }
        }
        if let PhysicalAddress::Ram { bank, .. } = mb.physical_address_for_write(base) {
            if let Some(at) = written_at.get_mut(usize::from(bank)) {
                at.push(base);
            }
        }
    }
    ui.strong(format!(
        "RAM: {}K",
        16 * u32::from(mb.physical_bank_count())
    ));
    egui::Grid::new("memmap-banks")
        .spacing([4.0, 4.0])
        .show(ui, |ui| {
            for (bank, (read, written)) in read_at.iter().zip(&written_at).enumerate() {
                let bank = bank as u16;
                let paged = !read.is_empty() || !written.is_empty();
                let fill = if !read.is_empty() {
                    egui::Color32::from_rgb(60, 150, 70)
                } else if !written.is_empty() {
                    egui::Color32::from_rgb(40, 90, 50)
                } else {
                    ui.visuals().extreme_bg_color
                };
                let at = |addresses: &[u16]| {
                    addresses
                        .iter()
                        .map(|a| format!("&{a:04X}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                let tooltip = if !read.is_empty() {
                    format!("read and written at {}", at(read))
                } else if !written.is_empty() {
                    format!("written at {} (a ROM is read there)", at(written))
                } else {
                    "not paged".to_owned()
                };
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(64.0, 30.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, fill);
                ui.painter().text(
                    rect.center_top() + egui::vec2(0.0, 2.0),
                    egui::Align2::CENTER_TOP,
                    crate::ui::paging::bank_name(bank),
                    egui::FontId::proportional(12.0),
                    if paged {
                        egui::Color32::WHITE
                    } else {
                        ui.visuals().weak_text_color()
                    },
                );
                if paged {
                    ui.painter().text(
                        rect.center_bottom() - egui::vec2(0.0, 2.0),
                        egui::Align2::CENTER_BOTTOM,
                        at(if read.is_empty() { written } else { read }),
                        egui::FontId::monospace(10.0),
                        egui::Color32::WHITE,
                    );
                }
                response
                    .on_hover_text(format!("{}: {tooltip}", crate::ui::paging::bank_name(bank)));
                // a 64K page a row: the main 64K, then each expansion page
                if bank % 4 == 3 {
                    ui.end_row();
                }
            }
        });
    ui.add_space(6.0);
    // a CPC+: its cartridge (the system one, or one inserted), each page by its ROM name
    if let Some(cartridge) = mb.any_cartridge() {
        let system = mb.cartridge().is_none();
        ui.strong(format!(
            "Cartridge: {}, {} pages of 16K",
            if system { "the system one" } else { "inserted" },
            cartridge.page_count()
        ));
        let lower = mb.ga().plus().map(|plus| plus.rmr2() & 7);
        let upper = mb.cartridge_page_of_upper_rom(mb.upper_rom_number());
        // the pages 8 a row, the lower and upper ROMs' lit; a page's name on hover
        egui::Grid::new("memmap-cartridge")
            .spacing([4.0, 4.0])
            .show(ui, |ui| {
                for page in 0..cartridge.page_count().min(32) as u8 {
                    let role = match (lower == Some(page), upper == Some(page)) {
                        (true, true) => "lower and upper ROM",
                        (true, false) => "lower ROM",
                        (false, true) => "upper ROM",
                        _ => "",
                    };
                    let fill = if role.is_empty() {
                        ui.visuals().extreme_bg_color
                    } else {
                        egui::Color32::from_rgb(200, 70, 50)
                    };
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(34.0, 20.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, fill);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        page.to_string(),
                        egui::FontId::monospace(12.0),
                        if role.is_empty() {
                            ui.visuals().weak_text_color()
                        } else {
                            egui::Color32::WHITE
                        },
                    );
                    let bytes: Vec<u8> = (0..0x400u16)
                        .filter_map(|offset| mb.read_cartridge_byte(page, offset))
                        .collect();
                    let name = rom_name(&bytes)
                        .unwrap_or_else(|| crate::ui::paging::cartridge_page_name(mb, page));
                    response.on_hover_text(if role.is_empty() {
                        format!("page {page}: {name}")
                    } else {
                        format!("page {page}: {name} ({role})")
                    });
                    if page % 8 == 7 {
                        ui.end_row();
                    }
                }
            });
    }
    ui.strong("ROMs");
    egui::Grid::new("memmap-roms").striped(true).show(ui, |ui| {
        let selected = mb.upper_rom_number();
        for slot in 0..=255u8 {
            let Some(rom) = mb.upper_rom(slot) else {
                continue;
            };
            ui.monospace(format!("{slot:3}"));
            ui.monospace(rom_name(rom).unwrap_or_else(|| "(no name)".to_owned()));
            if slot == selected {
                ui.weak("selected");
            }
            ui.end_row();
        }
    });
}

/// A ROM's name: the first name of its command table (its header: type, mark, version,
/// revision, then the table's address; each name ends with bit 7 set on its last letter).
pub fn rom_name(rom: &[u8]) -> Option<String> {
    let table = usize::from(u16::from_le_bytes([*rom.get(4)?, *rom.get(5)?]));
    let offset = table.checked_sub(0xc000)?;
    let mut name = String::new();
    for &byte in rom.get(offset..)?.iter().take(16) {
        let c = byte & 0x7f;
        if !(0x20..0x7f).contains(&c) {
            return None;
        }
        name.push(c as char);
        if byte & 0x80 != 0 {
            return Some(format!(
                "{} {}.{}.{}",
                name.trim(),
                rom.get(1)?,
                rom.get(2)?,
                rom.get(3)?
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rom_is_named_from_its_header() {
        // a background ROM 1.2.3, its table at &C010, its first name "MYROM"
        let mut rom = vec![0u8; 0x40];
        rom[..6].copy_from_slice(&[1, 1, 2, 3, 0x10, 0xc0]);
        rom[0x10..0x15].copy_from_slice(b"MYRO\xcd");
        assert_eq!(rom_name(&rom).as_deref(), Some("MYROM 1.2.3"));
        // no table: no name
        assert_eq!(rom_name(&[0; 8]), None);
    }
}
