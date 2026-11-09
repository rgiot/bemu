//! The paging form of the Memory and Disassembly windows, after ACE's: follow the CPU's paging,
//! or look through another (a RAM configuration and page, the lower ROM, an upper ROM); the map
//! of the 4 blocks of 16K and the registers that page them.

use crate::motherboard::Motherboard;

/// The RAM configurations (the value's bits 0-2): the bank of each 16K block, 0-3 the main
/// 64K's (MB0-MB3), 4-7 the blocks of the expansion's 64K page chosen (bits 3-5:
/// P<page>B0-B3). The value written to &7Fxx is &C0 + 8 × page + configuration: &C0-&C7 for
/// page 0, &C8-&CF for page 1... &F8-&FF for page 7.
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

/// A 16K RAM bank by its page and block: the main 64K is page M (`MB0`-`MB3`), an expansion's
/// 64K pages are P0-P7... (the value's bits 3-5: `P0B0`-`P0B3`, `P1B0`...): physical bank 4 is
/// `P0B0`, 9 is `P1B1`.
pub fn bank_name(bank: u16) -> String {
    match bank {
        0..=3 => format!("MB{bank}"),
        _ => format!("P{}B{}", (bank - 4) / 4, (bank - 4) % 4),
    }
}

/// A window's paging: the emulator's, or its own.
#[derive(Clone)]
pub struct Paging {
    /// Follow the emulator's paging (the CPU's view).
    pub follow: bool,
    /// Show what a write reaches (the RAM under the ROMs), not what a read gives (WinAPE's and
    /// MacCPC's Read / Write).
    pub writes: bool,
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
            writes: false,
            ram_config: 0,
            page: 0,
            lower_rom: true,
            upper_rom: false,
            upper_number: 0,
        }
    }
}

/// Where a block of 16K reads from.
#[derive(Clone, Copy, PartialEq, Debug)]
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

    /// The RAM bank a write to `block` reaches.
    fn write_bank(&self, mb: &Motherboard, block: u16) -> u16 {
        if self.follow {
            match mb.physical_address_for_write(block * 0x4000) {
                crate::memory::PhysicalAddress::Ram { bank, .. } => bank,
                _ => 0,
            }
        } else {
            self.physical(CONFIGS[usize::from(self.ram_config)][usize::from(block)])
        }
    }

    /// The PAL's value for this configuration and page (`&C0`-`&FF`), `None` beyond the 8 pages
    /// the MMR names.
    fn mmr(&self) -> Option<u8> {
        (self.page < 8).then(|| 0xc0 | (self.page << 3) | self.ram_config)
    }

    /// "&C3 · 0-3-2-7": the configuration and the banks it puts in the 4 blocks.
    fn ram_text(&self) -> String {
        let banks = CONFIGS[usize::from(self.ram_config)]
            .iter()
            .map(|&bank| bank_name(self.physical(bank)))
            .collect::<Vec<_>>()
            .join("-");
        match self.mmr() {
            Some(mmr) => format!("&{mmr:02X} · {banks}"),
            None => format!("page {} C{} · {banks}", self.page, self.ram_config),
        }
    }

    /// The byte at `address` through this paging: what a read gives, or what a write reaches.
    pub fn read(&self, mb: &Motherboard, address: u16) -> u8 {
        if self.writes {
            return mb
                .read_bank_byte(self.write_bank(mb, address >> 14), address & 0x3fff)
                .unwrap_or(0xff);
        }
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
                ui.separator();
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
                        Source::Ram(bank) => cell(ui, bank_name(bank), RAM, 2.0 * CELL + GAP),
                        _ => {
                            let text = match source {
                                Source::UpperRom(slot) => format!("ROM {slot}"),
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
            if let Some(mmr) = paging.mmr() {
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
        });
    });
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::z80::HasValue;

    /// The map the window draws: (read, write bank) per block.
    fn map(paging: &Paging, mb: &Motherboard) -> Vec<(Source, u16)> {
        (0..4u16)
            .map(|block| {
                let write = if paging.follow {
                    match mb.physical_address_for_write(block * 0x4000) {
                        crate::memory::PhysicalAddress::Ram { bank, .. } => bank,
                        _ => unreachable!(),
                    }
                } else {
                    paging.physical(CONFIGS[usize::from(paging.ram_config)][usize::from(block)])
                };
                (paging.source(mb, block), write)
            })
            .collect()
    }

    /// The CPC pages its RAM itself: a program at &8000 (in every bank) writes
    /// `mmr` to the PAL, then loops.
    fn page_with_out(mb: &mut Motherboard, mmr: u8) {
        let source = format!(" org &8000\n di\n ld bc,&7f{mmr:02x}\n out (c),c\n jp $\n");
        let code = cpclib_asm::assemble(&source).unwrap_or_else(|e| panic!("{e}"));
        // in every bank: C2 pages bank 6 at &8000, so the loop must be there after the OUT too
        for bank in 0..mb.physical_bank_count() {
            for (offset, byte) in code.iter().enumerate() {
                mb.write_bank_byte(bank, offset as u16, *byte);
            }
        }
        mb.tick_until_next_instruction();
        mb.set_pc(0x8000);
        // an interrupt may come before the DI: the firmware takes it and comes back
        let pc = |mb: &Motherboard| mb.cpu().reg16(crate::z80::Reg16::Pc).value();
        for _ in 0..100_000 {
            if pc(mb) == 0x8006 {
                return;
            }
            mb.execute_next_opcode_no_video();
        }
        panic!("the OUT program did not reach its loop: PC &{:04X}", pc(mb));
    }

    #[test]
    fn every_ram_configuration_maps_as_the_emulator_pages() {
        let mut mb = crate::motherboard::Motherboard::default();
        mb.set_total_ram_kb(Some(576)).unwrap();
        // to BASIC's prompt: the firmware's interrupt handler is in place
        while mb.t_state_counter().value() < 4_000_000 {
            mb.execute_next_opcode_no_video();
        }
        // the banks of each configuration with page 0 (&C0-&C7: 4-7 are P0B0-P0B3); with page 3
        // (&D8-&DF), P3B0-P3B3 (physical 16-19) in their place
        let expected: [[u16; 4]; 8] = [
            [0, 1, 2, 3],
            [0, 1, 2, 7],
            [4, 5, 6, 7],
            [0, 3, 2, 7],
            [0, 4, 2, 3],
            [0, 5, 2, 3],
            [0, 6, 2, 3],
            [0, 7, 2, 3],
        ];
        for page in [0u8, 3] {
            for (config, banks) in expected.iter().enumerate() {
                let mmr = 0xc0 | (page << 3) | config as u8;
                page_with_out(&mut mb, mmr);
                assert_eq!(mb.mmr() & 0x3f, mmr & 0x3f);
                // a byte of its own in every bank, to tell them apart
                for bank in 0..mb.physical_bank_count() {
                    mb.write_bank_byte(bank, 0x123, bank as u8);
                }
                let shift = |bank: u16| {
                    if bank < 4 {
                        bank
                    } else {
                        bank + 4 * u16::from(page)
                    }
                };
                let mut follow = Paging::default();
                let mut own = Paging {
                    follow: false,
                    ..Paging::default()
                };
                own.synchronise(&mb);
                follow.follow = true;
                // ROMs off: every block reads its RAM
                own.lower_rom = false;
                own.upper_rom = false;
                for block in 0..4u16 {
                    let bank = shift(banks[usize::from(block)]);
                    let address = block * 0x4000 + 0x123;
                    assert_eq!(own.read(&mb, address), bank as u8, "&{mmr:02X}");
                    assert_eq!(
                        map(&own, &mb)[usize::from(block)],
                        (Source::Ram(bank), bank)
                    );
                }
                // the emulator's paging (the lower ROM on after the reset): the same map
                let followed = map(&follow, &mb);
                own.lower_rom = mb.ga().rmr() & 4 == 0;
                own.upper_rom = mb.ga().rmr() & 8 == 0;
                assert_eq!(followed, map(&own, &mb), "&{mmr:02X}");
                // under the lower ROM, writes go to the configuration's first bank
                assert_eq!(followed[0].1, shift(banks[0]), "&{mmr:02X}");
                for block in 1..3 {
                    let bank = shift(banks[block]);
                    assert_eq!(followed[block], (Source::Ram(bank), bank), "&{mmr:02X}");
                }
            }
        }
    }

    #[test]
    fn the_ram_is_named_by_the_value_given_to_the_gate_array() {
        let paging = |page, ram_config| Paging {
            page,
            ram_config,
            ..Paging::default()
        };
        assert_eq!(paging(0, 0).ram_text(), "&C0 · MB0-MB1-MB2-MB3");
        assert_eq!(paging(0, 3).ram_text(), "&C3 · MB0-MB3-MB2-P0B3");
        assert_eq!(paging(1, 2).ram_text(), "&CA · P1B0-P1B1-P1B2-P1B3");
        assert_eq!(paging(3, 3).ram_text(), "&DB · MB0-MB3-MB2-P3B3");
        assert_eq!(paging(7, 7).ram_text(), "&FF · MB0-P7B3-MB2-MB3");
    }

    #[test]
    fn a_write_view_shows_the_ram_under_the_rom() {
        let mut mb = crate::motherboard::Motherboard::default();
        mb.write_bank_byte(0, 0, 0x42);
        let mut paging = Paging::default();
        assert_eq!(paging.read(&mb, 0), mb.read_lower_rom_byte(0));
        paging.writes = true;
        assert_eq!(paging.read(&mb, 0), 0x42);
    }

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
