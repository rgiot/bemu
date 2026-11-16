//! A paging of the CPU's 64K, after ACE's: the CPU's own, or another chosen (a RAM configuration
//! and page, the lower ROM, an upper ROM; on a CPC+ the cartridge page of the lower ROM and where
//! it is, the ASIC's register page), and what it reads and writes. The Memory and Disassembly
//! windows look through one (`crate::ui`'s form), the debug API too (`/api/ram?paging=`).

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
#[derive(Clone, Debug, PartialEq)]
pub struct Paging {
    /// Follow the emulator's paging (the CPU's view).
    pub follow: bool,
    /// Show what a write reaches (the RAM under the ROMs), not what a read gives (WinAPE's and
    /// MacCPC's Read / Write).
    pub writes: bool,
    pub ram_config: u8,
    pub page: u8,
    pub lower_rom: bool,
    pub upper_rom: bool,
    pub upper_number: u8,
    /// A CPC+ (RMR2): the cartridge page of the lower ROM, the block it is in (0-2: &0000,
    /// &4000, &8000), the ASIC's register page at &4000.
    pub lower_page: u8,
    pub lower_block: u8,
    pub registers: bool,
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
            lower_page: 0,
            lower_block: 0,
            registers: false,
        }
    }
}

/// Where a block of 16K reads from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    /// A CPC's lower ROM (the firmware).
    LowerRom,
    /// A ROM of its own in an upper ROM slot.
    UpperRom(u8),
    /// A page of a CPC+'s cartridge (the lower ROM, or an upper ROM number that selects it).
    Cartridge(u8),
    /// A CPC+'s ASIC registers (&4000-&7FFF, RMR2): RAM where no register answers.
    Registers,
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
        if let Some(plus) = mb.ga().plus() {
            let rmr2 = plus.rmr2();
            self.lower_page = rmr2 & 7;
            self.lower_block = match (rmr2 >> 3) & 3 {
                3 => 0,
                block => block,
            };
            self.registers = (rmr2 >> 3) & 3 == 3;
        }
    }

    /// The physical 16K RAM bank of `bank` (0-7 in a configuration) in this page.
    pub fn physical(&self, bank: u8) -> u16 {
        match bank {
            0..=3 => u16::from(bank),
            _ => 4 + u16::from(self.page) * 4 + u16::from(bank - 4),
        }
    }

    pub fn source(&self, mb: &Motherboard, block: u16) -> Source {
        let cartridge = mb.any_cartridge().is_some();
        // an upper ROM number: a ROM of its own, or (a CPC+) the cartridge page it selects
        let upper = |number: u8| match mb.cartridge_page_of_upper_rom(number) {
            Some(page) => Source::Cartridge(page),
            None => Source::UpperRom(number),
        };
        if self.follow {
            if block == 1 && mb.register_page_mapped() {
                return Source::Registers;
            }
            let lower_page = mb.ga().plus().map_or(0, |plus| plus.rmr2() & 7);
            return match mb.physical_address(block * 0x4000) {
                crate::memory::PhysicalAddress::LowerRom { .. } if cartridge => {
                    Source::Cartridge(lower_page)
                }
                crate::memory::PhysicalAddress::LowerRom { .. } => Source::LowerRom,
                crate::memory::PhysicalAddress::UpperRom { rom, .. } => upper(rom),
                crate::memory::PhysicalAddress::Ram { bank, .. } => Source::Ram(bank),
            };
        }
        if cartridge && block == 1 && self.registers {
            return Source::Registers;
        }
        let lower_block = if cartridge { self.lower_block } else { 0 };
        match block {
            _ if self.lower_rom && block == u16::from(lower_block) => {
                if cartridge {
                    Source::Cartridge(self.lower_page)
                } else {
                    Source::LowerRom
                }
            }
            3 if self.upper_rom => upper(self.upper_number),
            _ => Source::Ram(
                self.physical(CONFIGS[usize::from(self.ram_config)][usize::from(block)]),
            ),
        }
    }

    /// The RAM bank a write to `block` reaches.
    pub fn write_bank(&self, mb: &Motherboard, block: u16) -> u16 {
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
    // lazily: `page << 3` overflows for the pages past 31 a large expansion has
    #[allow(clippy::unnecessary_lazy_evaluations)]
    pub fn mmr(&self) -> Option<u8> {
        (self.page < 8).then(|| 0xc0 | (self.page << 3) | self.ram_config)
    }

    /// "&C3 · 0-3-2-7": the configuration and the banks it puts in the 4 blocks.
    pub fn ram_text(&self) -> String {
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
        let ram = |mb: &Motherboard| {
            mb.read_bank_byte(self.write_bank(mb, address >> 14), offset)
                .unwrap_or(0xff)
        };
        match self.source(mb, address >> 14) {
            Source::Cartridge(page) => mb.read_cartridge_byte(page, offset).unwrap_or(0xff),
            // a register where one answers, the RAM elsewhere
            Source::Registers => mb.read_register_page(address).unwrap_or_else(|| ram(mb)),
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

impl Paging {
    /// The paging the API names (`bemu_api::Paging`), each part it leaves out as the machine
    /// has it now. Nothing chosen, it is the CPU's own, followed (the CPC+'s register page
    /// included).
    pub fn from_api(chosen: &bemu_api::Paging, mb: &Motherboard) -> Self {
        let mut paging = Paging {
            follow: false,
            ..Paging::default()
        };
        paging.synchronise(mb);
        paging.writes = chosen.writes;
        if (bemu_api::Paging {
            writes: false,
            ..*chosen
        })
        .is_cpu()
        {
            paging.follow = true;
            return paging;
        }
        let set = |field: &mut u8, value: Option<u8>| {
            if let Some(value) = value {
                *field = value;
            }
        };
        set(&mut paging.ram_config, chosen.ram_config);
        set(&mut paging.page, chosen.page);
        set(&mut paging.upper_number, chosen.upper_number);
        set(&mut paging.lower_page, chosen.lower_page);
        set(&mut paging.lower_block, chosen.lower_block);
        if let Some(lower) = chosen.lower_rom {
            paging.lower_rom = lower;
        }
        if let Some(upper) = chosen.upper_rom {
            paging.upper_rom = upper;
        }
        if let Some(registers) = chosen.registers {
            paging.registers = registers;
        }
        paging
    }

    /// The RAM bank and offset a write to `address` reaches through this paging.
    pub fn write_location(&self, mb: &Motherboard, address: u16) -> (u16, u16) {
        (self.write_bank(mb, address >> 14), address & 0x3fff)
    }

    /// What 16K block `block` (0-3) reads, as the API names a space.
    pub fn space(&self, mb: &Motherboard, block: u16) -> bemu_api::Space {
        match self.source(mb, block) {
            Source::LowerRom => bemu_api::Space::Lower,
            Source::UpperRom(slot) => bemu_api::Space::Upper(slot),
            Source::Cartridge(page) => bemu_api::Space::Cartridge(page),
            Source::Registers => bemu_api::Space::Asic,
            Source::Ram(bank) => bemu_api::Space::Ram(bank),
        }
    }
}

/// One byte of `space` at `offset`, whatever the CPU's paging; `None` for a space there is
/// not (a RAM bank past the last, a cartridge page or the ASIC on a CPC). A RAM bank's offset
/// runs on into the next banks; the others wrap at 16K.
pub fn read_space(mb: &Motherboard, space: bemu_api::Space, offset: u32) -> Option<u8> {
    use bemu_api::Space;
    let in_16k = (offset & 0x3fff) as u16;
    match space {
        Space::Ram(bank) => {
            let bank = u32::from(bank) + offset / 0x4000;
            mb.read_bank_byte(u16::try_from(bank).ok()?, in_16k)
        }
        // a CPC+: the cartridge page RMR2 selects
        Space::Lower => match mb.any_cartridge() {
            Some(_) => {
                let page = mb.ga().plus().map_or(0, |plus| plus.rmr2() & 7);
                mb.read_cartridge_byte(page, in_16k)
            }
            None => Some(mb.read_lower_rom_byte(in_16k)),
        },
        Space::Upper(slot) => match mb.cartridge_page_of_upper_rom(slot) {
            Some(page) => mb.read_cartridge_byte(page, in_16k),
            // an empty slot reads as BASIC's (slot 0), as on the CPC
            None => mb
                .upper_rom(slot)
                .or_else(|| mb.upper_rom(0))
                .and_then(|rom| rom.get(usize::from(in_16k)).copied()),
        },
        Space::Cartridge(page) => {
            let pages = mb.any_cartridge()?.page_count();
            if usize::from(page) >= pages {
                return None;
            }
            mb.read_cartridge_byte(page, in_16k)
        }
        // a register where one answers, the RAM elsewhere, as the CPU reads the page
        Space::Asic => {
            mb.asic()?;
            Some(
                mb.read_register_page(0x4000 | in_16k)
                    .unwrap_or_else(|| mb.read_bank_byte(1, in_16k).unwrap_or(0xff)),
            )
        }
    }
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
    fn a_cpc_plus_pages_its_cartridge_and_registers() {
        let mut mb = crate::motherboard::MotherboardBuilder::default()
            .set_model(crate::motherboard::CpcModel::Cpc6128Plus)
            .build();
        // the lower ROM is the cartridge's page 0 after the reset
        let follow = Paging::default();
        assert_eq!(follow.source(&mb, 0), Source::Cartridge(0));
        assert_eq!(
            follow.read(&mb, 0x10),
            mb.read_cartridge_byte(0, 0x10).unwrap()
        );
        // a paging of its own: page 1 (BASIC) at &0000, then the registers at &4000
        let mut own = Paging {
            follow: false,
            lower_page: 1,
            ..Paging::default()
        };
        assert_eq!(own.source(&mb, 0), Source::Cartridge(1));
        assert_eq!(
            own.read(&mb, 0x20),
            mb.read_cartridge_byte(1, 0x20).unwrap()
        );
        own.registers = true;
        assert_eq!(own.source(&mb, 1), Source::Registers);
        // upper ROM 128+n: the cartridge's page n
        own.upper_rom = true;
        own.upper_number = 0x83;
        assert_eq!(own.source(&mb, 3), Source::Cartridge(3));
        mb.write_bank_byte(1, 0x123, 0x5a);
        own.registers = false;
        assert_eq!(own.read(&mb, 0x4123), 0x5a);
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

