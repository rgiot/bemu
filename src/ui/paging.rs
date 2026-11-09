//! The paging form of the Memory and Disassembly windows, after ACE's: follow the CPU's paging,
//! or look through another (a RAM configuration and page, the lower ROM, an upper ROM); the map
//! of the 4 blocks of 16K and the registers that page them.

use crate::motherboard::Motherboard;

/// The RAM configurations (the MMR's bits 0-2): the bank of each 16K block, 4-7 being the
/// extra 64K page chosen.
const CONFIGS: [[u8; 4]; 8] = [
    [0, 1, 2, 3],
    [0, 1, 2, 7],
    [4, 5, 6, 7],
    [0, 3, 2, 7],
    [0, 4, 2, 3],
    [0, 5, 2, 3],
    [0, 6, 2, 3],
    [0, 7, 2, 3],
];

/// A window's paging: the emulator's, or its own.
#[derive(Clone)]
pub struct Paging {
    /// Follow the emulator's paging (the CPU's view).
    pub follow: bool,
    ram_config: u8,
    page: u8,
    lower_rom: bool,
    upper_rom: bool,
    upper_number: u8,
}

impl Default for Paging {
    fn default() -> Self {
        Self {
            follow: true,
            ram_config: 0,
            page: 0,
            lower_rom: true,
            upper_rom: false,
            upper_number: 0,
        }
    }
}

/// Where a block of 16K reads from.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    LowerRom,
    UpperRom(u8),
    Ram(u16),
}

impl Paging {
    /// Take the emulator's paging as the window's own.
    pub fn synchronise(&mut self, mb: &Motherboard) {
        let mmr = mb.mmr();
        let rmr = mb.ga().rmr();
        self.ram_config = mmr & 7;
        self.page = (mmr >> 3) & 7;
        self.lower_rom = rmr & 4 == 0;
        self.upper_rom = rmr & 8 == 0;
        self.upper_number = mb.upper_rom_number();
    }

    /// The physical 16K RAM bank of `bank` (0-7 in a configuration) in this page.
    fn physical(&self, bank: u8) -> u16 {
        match bank {
            0..=3 => u16::from(bank),
            _ => 4 + u16::from(self.page) * 4 + u16::from(bank - 4),
        }
    }

    fn source(&self, mb: &Motherboard, block: u16) -> Source {
        if self.follow {
            return match mb.physical_address(block * 0x4000) {
                crate::memory::PhysicalAddress::LowerRom { .. } => Source::LowerRom,
                crate::memory::PhysicalAddress::UpperRom { rom, .. } => Source::UpperRom(rom),
                crate::memory::PhysicalAddress::Ram { bank, .. } => Source::Ram(bank),
            };
        }
        match block {
            0 if self.lower_rom => Source::LowerRom,
            3 if self.upper_rom => Source::UpperRom(self.upper_number),
            _ => Source::Ram(
                self.physical(CONFIGS[usize::from(self.ram_config)][usize::from(block)]),
            ),
        }
    }

    /// The byte at `address` through this paging.
    pub fn read(&self, mb: &Motherboard, address: u16) -> u8 {
        if self.follow {
            return mb.read_byte(address);
        }
        let offset = address & 0x3fff;
        match self.source(mb, address >> 14) {
            Source::LowerRom => mb.read_lower_rom_byte(offset),
            // an empty slot reads as BASIC's (slot 0), as on the CPC
            Source::UpperRom(slot) => mb
                .upper_rom(slot)
                .or_else(|| mb.upper_rom(0))
                .and_then(|rom| rom.get(usize::from(offset)).copied())
                .unwrap_or(0xff),
            Source::Ram(bank) => mb.read_bank_byte(bank, offset).unwrap_or(0xff),
        }
    }
}

/// The form: follow or not, the paging chosen, its map and the registers.
pub fn show(ui: &mut egui::Ui, id: &str, mb: &Motherboard, paging: &mut Paging) {
    const ROM: egui::Color32 = egui::Color32::from_rgb(200, 70, 50);
    const RAM: egui::Color32 = egui::Color32::from_rgb(60, 150, 70);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
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
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt((id, "config"))
                                .selected_text(format!("C{}", paging.ram_config))
                                .show_ui(ui, |ui| {
                                    for (config, banks) in CONFIGS.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut paging.ram_config,
                                            config as u8,
                                            format!("C{config}: banks {banks:?}"),
                                        );
                                    }
                                });
                            let pages = (mb.physical_bank_count().saturating_sub(4) / 4).max(1);
                            egui::ComboBox::from_id_salt((id, "page"))
                                .selected_text(format!("page {}", paging.page))
                                .show_ui(ui, |ui| {
                                    for page in 0..pages.min(64) as u8 {
                                        ui.selectable_value(
                                            &mut paging.page,
                                            page,
                                            format!(
                                                "page {page} (+{}K)",
                                                64 + 64 * u32::from(page)
                                            ),
                                        );
                                    }
                                });
                        });
                        ui.end_row();
                        ui.label("Lower ROM");
                        ui.checkbox(&mut paging.lower_rom, "in &0000-&3FFF");
                        ui.end_row();
                        ui.label("Upper ROM");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut paging.upper_rom, "in &C000-&FFFF, slot");
                            ui.add(egui::DragValue::new(&mut paging.upper_number).range(0..=255));
                        });
                        ui.end_row();
                    });
            });
        });
        // (a separator here would stretch to the whole scroll area's height)
        ui.add_space(16.0);
        ui.vertical(|ui| {
            // the map: what each block reads (ROM red, RAM green) and, when they differ, where
            // writes go (a second cell); a block that reads and writes the same RAM has one cell
            // as wide as both
            const CELL: f32 = 72.0;
            const GAP: f32 = 4.0;
            let cell = |ui: &mut egui::Ui, text: String, fill: egui::Color32, width: f32| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::hover());
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
                        Source::Ram(bank) => cell(ui, format!("RAM{bank}"), RAM, 2.0 * CELL + GAP),
                        _ => {
                            let text = match source {
                                Source::UpperRom(slot) => format!("ROM {slot}"),
                                _ => "Firmware".to_owned(),
                            };
                            cell(ui, text, ROM, CELL);
                            // under a ROM, writes go to RAM
                            let bank = if paging.follow {
                                match mb.physical_address_for_write(block * 0x4000) {
                                    crate::memory::PhysicalAddress::Ram { bank, .. } => bank,
                                    _ => 0,
                                }
                            } else {
                                paging.physical(
                                    CONFIGS[usize::from(paging.ram_config)][usize::from(block)],
                                )
                            };
                            cell(ui, format!("RAM{bank}"), RAM, CELL);
                        }
                    }
                });
            }
            ui.weak(format!(
                "MMR &{:02X} · RMR &{:02X} · upper ROM &{:02X}",
                mb.mmr(),
                mb.ga().rmr(),
                mb.upper_rom_number()
            ));
        });
    });
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn a_paging_of_its_own_reads_the_bank_it_names() {
        let mut mb = crate::motherboard::Motherboard::default();
        // the extra bank 4 (C4 puts it at &4000) and the base bank 1
        mb.write_bank_byte(4, 0x10, 0xaa);
        mb.write_bank_byte(1, 0x10, 0x55);
        let mut paging = Paging {
            follow: false,
            ..Paging::default()
        };
        assert_eq!(paging.read(&mb, 0x4010), 0x55);
        paging.ram_config = 4;
        assert_eq!(paging.read(&mb, 0x4010), 0xaa);
        // the lower ROM over RAM bank 0, then RAM when it is off
        mb.write_bank_byte(0, 0, 0x42);
        assert_eq!(paging.read(&mb, 0), mb.read_lower_rom_byte(0));
        paging.lower_rom = false;
        assert_eq!(paging.read(&mb, 0), 0x42);
    }
}
