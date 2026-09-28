use crate::chip::*;

/// A Bank contains 16kb
#[derive(Clone)]
pub struct Bank([u8; 0x4000]);

/// A Page contains 4 Banks
#[derive(Default, Clone)]
pub struct Page([Bank; 4]);

#[derive(Clone)]
pub struct Rom(Bank);

trait Readable {
    fn read(&self, addr: u16) -> u8;
}

trait Writeable {
    fn fill(&mut self, v: u8);
    fn write(&mut self, addr: u16, byte: u8);
}

pub struct Memory {
    /// Main 64kb of the machine
    main_memory: Page,
    /// Additional pages (1 page for a standard CPC)
    additional_memory: Vec<Page>,

    /// Roms
    upper_roms: Vec<Option<Rom>>,
    lower_rom: Rom,
}

/// Number of the page
#[derive(PartialEq, Copy, Clone, Debug)]
pub enum PageNumber {
    /// Main memory linked to CRTC
    Base,
    /// From 0 to 3 in an extra page
    Page(u8),
}

/// Number of the bank
#[derive(PartialEq, Copy, Clone, Debug)]
pub enum BankNumber {
    Bank0,
    Bank1,
    Bank2,
    Bank3,
}

impl BankNumber {
    pub fn index(&self) -> usize {
        match self {
            BankNumber::Bank0 => 0,
            BankNumber::Bank1 => 1,
            BankNumber::Bank2 => 2,
            BankNumber::Bank3 => 3,
        }
    }
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct BankConfig(pub PageNumber, pub BankNumber);

// [0x0000 - 0x3fff]
// [0x4000 - 0x7fff]
// [0x8000 - 0xbfff]
//[0xc000 - 0xffff]
#[derive(PartialEq, Copy, Clone, Debug)]
pub struct RamConfig(pub [BankConfig; 4]);

impl RamConfig {
    /// Combines a large RAM expansion's extra high bits (see
    /// `crate::expansion::LargeRamExpansion`) into whichever quadrant the
    /// gate array/PAL's own `PageNumber::Page` selects, widening its 3-bit
    /// page number (0-7, this config's own D3-D5 bits) into the full 6-bit
    /// bank number (`(high_bits << 3) | page`, up to 4MB) the expansion
    /// protocol defines. A no-op wherever this config already selects
    /// `PageNumber::Base` (those quadrants are the CPC's own base 64K,
    /// which no RAM-select write - extended or not - ever touches).
    pub fn with_expansion_high_bits(self, high_bits: u8) -> RamConfig {
        RamConfig(self.0.map(|BankConfig(page, bank)| {
            let page = match page {
                PageNumber::Base => PageNumber::Base,
                PageNumber::Page(p) => PageNumber::Page((high_bits << 3) | p),
            };
            BankConfig(page, bank)
        }))
    }
}

/// I have no idea of how is coded the ROM selection stuff within the CPC.
/// So I create this chip only for this purpose
#[derive(Default)]
pub struct UpperRomSelector {
    rom_number: u8,
    pins: PinsBuffer,
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum Flag {
    Enable,
    Disable,
}
impl Default for Flag {
    fn default() -> Self {
        Flag::Disable
    }
}

impl Flag {
    pub fn is_enabled(&self) -> bool {
        match self {
            Flag::Enable => true,
            Flag::Disable => false,
        }
    }
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct RomConfig {
    pub lower_rom: Flag,
    pub upper_rom: Flag,
    pub rom_number: u8,
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct MemoryConfig(pub RomConfig, pub RamConfig);

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum UpperRomSelectorPin {
    D7 = 1 << 7,
    D6 = 1 << 6,
    D5 = 1 << 5,
    D4 = 1 << 4,
    D3 = 1 << 3,
    D2 = 1 << 2,
    D1 = 1 << 1,
    D0 = 1 << 0,

    A13 = 1 << 8,
    IORQ = 1 << 9,
    WR = 1 << 10,
}

impl Into<u64> for UpperRomSelectorPin {
    fn into(self) -> u64 {
        self as u64
    }
}

impl Into<u64> for &UpperRomSelectorPin {
    fn into(self) -> u64 {
        *self as u64
    }
}

const URS_PIN_DATA_BUS: [UpperRomSelectorPin; 8] = [
    UpperRomSelectorPin::D7,
    UpperRomSelectorPin::D6,
    UpperRomSelectorPin::D5,
    UpperRomSelectorPin::D4,
    UpperRomSelectorPin::D3,
    UpperRomSelectorPin::D2,
    UpperRomSelectorPin::D1,
    UpperRomSelectorPin::D0,
];

impl Default for Bank {
    fn default() -> Self {
        Self([0; 0x4000])
    }
}

impl Readable for Bank {
    fn read(&self, addr: u16) -> u8 {
        self.0[addr as usize]
    }
}

impl Writeable for Bank {
    fn fill(&mut self, v: u8) {
        self.0.iter_mut().for_each(|m| *m = v);
    }

    fn write(&mut self, addr: u16, val: u8) {
        self.0[addr as usize] = val;
    }
}

impl Default for Rom {
    fn default() -> Self {
        Self(Bank([0; 0x4000]))
    }
}

impl Readable for Rom {
    fn read(&self, addr: u16) -> u8 {
        self.0.read(addr)
    }
}

impl Rom {
    fn basic_6128_azerty() -> Self {
        Self(Bank(*include_bytes!("../roms/basic6128_azerty.rom")))
    }

    fn basic_6128() -> Self {
        Self(Bank(*include_bytes!("../roms/basic6128.rom")))
    }

    fn amsdos() -> Self {
        Self(Bank(*include_bytes!("../roms/amsdos.rom")))
    }

    fn os6128_azerty() -> Self {
        Self(Bank(*include_bytes!("../roms/os6128_azerty.rom")))
    }

    fn os6128() -> Self {
        Self(Bank(*include_bytes!("../roms/os6128.rom")))
    }
}

impl From<u8> for PageNumber {
    fn from(u: u8) -> Self {
        // `u` is the gate array's raw 3-bit page-select field (0-7, see
        // ACCC/memory expansion scheme) - it legitimately spans the full
        // range on real hardware even on a stock machine that only has RAM
        // for page 0 fitted (see `Memory::ram_bank`, which aliases any page
        // beyond what's actually installed rather than asserting here).
        PageNumber::Page(u)
    }
}

impl From<u8> for BankNumber {
    fn from(u: u8) -> Self {
        assert!(u <= 3);
        [
            BankNumber::Bank0,
            BankNumber::Bank1,
            BankNumber::Bank2,
            BankNumber::Bank3,
        ][u as usize]
    }
}

impl DataChip for UpperRomSelector {
    fn pins_data() -> &'static [Self::PinType; 8] {
        return &URS_PIN_DATA_BUS;
    }
}

impl ReadDataChip for UpperRomSelector {}

impl Chip for UpperRomSelector {
    type PinType = UpperRomSelectorPin;

    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &PinsBuffer {
        &self.pins
    }

    fn must_execute_tick(&self) -> bool {
        // `IORQ`/`WR` here hold Neg_IORQ/Neg_WR verbatim (see the
        // sync_pins call in motherboard.rs - it copies the CPU's pins
        // unchanged, it doesn't invert them), i.e. true means the line is
        // *inactive*, matching the active-low convention every other chip
        // in this codebase negates before testing (e.g. Ppi::must_execute_tick
        // checks `!get_pin(Neg_RD)`). This condition was requiring both
        // lines inactive - the opposite of "a write is happening" - so
        // `execute_tick()` fired on essentially every tick that *wasn't* an
        // upper-ROM-select write, latching whatever garbage happened to be
        // on the data bus (usually an opcode fetch byte) as the selected
        // ROM number, and the one real select write got overwritten again
        // within nanoseconds by the next unrelated bus cycle.
        // The upper ROM select port is &DFxx: it is decoded by A13 being
        // *low* (A13 is copied here verbatim, so high = 1). Testing for it
        // high instead matched the gate array's &7Fxx port and latched its
        // command bytes (e.g. &85) as ROM numbers, so no real ROM select -
        // AMSDOS in slot 7 included - ever took effect.
        !self.get_pin(UpperRomSelectorPin::A13)
            && !self.get_pin(UpperRomSelectorPin::IORQ)
            && !self.get_pin(UpperRomSelectorPin::WR)
    }

    fn execute_tick(&mut self) {
        self.rom_number = self.get_data_from_pins();
    }

    fn reset(&mut self) {
        self.rom_number = 0;
    }
}

impl UpperRomSelector {
    pub fn upper_rom_number(&self) -> u8 {
        self.rom_number
    }

    pub fn force_upper_rom_number(&mut self, nb: u8) {
        self.rom_number = nb;
    }
}

impl Default for Memory {
    fn default() -> Self {
        let mut roms = Vec::new();
        roms.push(Some(Rom::basic_6128())); // 0
        roms.push(None);
        roms.push(None);
        roms.push(None);
        roms.push(None);
        roms.push(None);
        roms.push(None);
        roms.push(Some(Rom::amsdos())); // 7

        Memory {
            main_memory: Page::default(),
            additional_memory: vec![Page::default(); 1],
            upper_roms: roms,
            lower_rom: Rom::os6128(),
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> MemoryConfig {
        MemoryConfig(
            RomConfig {
                lower_rom: Flag::Disable,
                upper_rom: Flag::Disable,
                rom_number: 7,
            },
            RamConfig([
                BankConfig(PageNumber::Base, BankNumber::Bank0),
                BankConfig(PageNumber::Base, BankNumber::Bank1),
                BankConfig(PageNumber::Base, BankNumber::Bank2),
                BankConfig(PageNumber::Base, BankNumber::Bank3),
            ]),
        )
    }
}

impl MemoryConfig {
    /// Check if we have to read in the lower rom
    pub fn is_in_enabled_lower_rom(&self, addr: u16) -> bool {
        self.0.lower_rom.is_enabled() &&
       /* addr >= 0x0000 &&*/
        addr < 0x4000
    }

    /// Check if we have to read in the upper rom
    pub fn is_in_enabled_upper_rom(&self, addr: u16) -> bool {
        self.0.upper_rom.is_enabled() && addr >= 0xc000
    }

    /// Return the RAM Bank configuration for the given address
    pub fn bank_at(&self, addr: u16) -> &BankConfig {
        match addr {
            0x0000..0x4000 => &self.1 .0[0],
            0x4000..0x8000 => &self.1 .0[1],
            0x8000..0xc000 => &self.1 .0[2],
            0xc000.. => &self.1 .0[3],
        }
    }

    /// Get the relative address in the corresponding bnak
    pub fn relative_address_at(addr: u16) -> u16 {
        addr % 0x4000
    }
}

impl Page {
    /// Get the appropriate bank
    fn bank(&self, id: BankNumber) -> &Bank {
        &self.0[id.index()]
    }

    fn bank_mut(&mut self, id: BankNumber) -> &mut Bank {
        &mut self.0[id.index()]
    }
}

impl Memory {
    /// Change how many extra 64kb pages are actually fitted, beyond the
    /// base 64kb every CPC has (1 extra page = a stock 128K CPC 6128; the
    /// gate array's page-select field is 3 bits wide, so real hardware
    /// supports up to 7 extra pages = 512K total).
    ///
    /// Existing pages keep their content when growing; pages dropped when
    /// shrinking are discarded. `pages` is clamped to at least 1 so
    /// `PageNumber::Page` always has somewhere to alias to (see
    /// [`Memory::ram_bank`]).
    pub fn set_extra_pages(&mut self, pages: usize) {
        self.additional_memory.resize_with(pages.max(1), Page::default);
    }

    /// How many extra 64kb pages are currently fitted (see
    /// [`Memory::set_extra_pages`]).
    pub fn extra_pages(&self) -> usize {
        self.additional_memory.len()
    }

    /// Read a byte according to the provided memory configuration
    /// If the address is in an activate rom space; this rom is read instead of the ram
    /// If the adress is within the ram space, then the value depends on the ram configuration
    /// TODO manage strange configuration with missing roms
    pub fn read(&self, addr: u16, conf: &MemoryConfig) -> u8 {
        /// Retreive the right bank
        let memory = self.bank(addr, conf);

        // and read it
        memory.read(MemoryConfig::relative_address_at(addr))
    }

    /// Write a byte according to the memory configuration
    /// write is done on RAM even if ROMs are activated
    pub fn write(&mut self, addr: u16, val: u8, conf: &MemoryConfig) {
        /// Retreive the right bank
        let memory = self.bank(addr, conf);

        // and read it
        let bank = self.ram_bank_mut(conf.bank_at(addr));
        bank.write(MemoryConfig::relative_address_at(addr), val);
    }

    /// Get the appropriate bank to read for the given configuration
    fn bank(&self, addr: u16, conf: &MemoryConfig) -> &Bank {
        if conf.is_in_enabled_lower_rom(addr) {
            self.lower_rom_bank()
        } else if conf.is_in_enabled_upper_rom(addr) {
            self.upper_rom_bank(conf.0.rom_number)
        } else {
            self.ram_bank(conf.bank_at(addr))
        }
    }

    /// TODO implement properly the behavior when a rom indice is wrong
    fn upper_rom_bank(&self, number: u8) -> &Bank {
        let number = number as usize;

        if number >= self.upper_roms.len() {
            self.upper_rom_bank(0)
        } else {
            match &self.upper_roms[number] {
                Some(r) => &r.0,
                None => {
                    assert!(number != 0);
                    self.upper_rom_bank(0)
                }
            }
        }
    }

    fn lower_rom_bank(&self) -> &Bank {
        &self.lower_rom.0
    }

    /// Get the memory for the given bank.
    ///
    /// `PageNumber::Page` carries the raw 3-bit page-select field of the
    /// gate array's RAM configuration byte (0-7, ACCC/memory expansion
    /// scheme), but a stock machine only has as many extra 64kb pages as
    /// `additional_memory` actually holds (1 for a real 128K CPC). Real
    /// hardware doesn't wire a comparator for the pages it doesn't have
    /// fitted, so selecting one of those aliases back onto the installed
    /// pages rather than addressing anything new - it does *not* trap or
    /// panic. Modulo-wrapping here reproduces that aliasing instead of
    /// indexing out of bounds.
    fn ram_bank(&self, conf: &BankConfig) -> &Bank {
        let page = match conf.0 {
            PageNumber::Base => &self.main_memory,
            PageNumber::Page(p) => &self.additional_memory[p as usize % self.additional_memory.len()],
        };

        page.bank(conf.1)
    }

    /// Get the memory for the given bank. See [`Memory::ram_bank`] for why
    /// the page index wraps instead of being asserted in range.
    fn ram_bank_mut(&mut self, conf: &BankConfig) -> &mut Bank {
        let len = self.additional_memory.len();
        let page = match conf.0 {
            PageNumber::Base => &mut self.main_memory,
            PageNumber::Page(p) => &mut self.additional_memory[p as usize % len],
        };

        page.bank_mut(conf.1)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn memory_access() {
        let mut mem = Memory::default();
        let conf = MemoryConfig::default();

        mem.main_memory.0[0].fill(0);
        mem.main_memory.0[1].fill(1);
        mem.main_memory.0[2].fill(2);
        mem.main_memory.0[3].fill(3);

        mem.additional_memory[0].0[0].fill(4);
        mem.additional_memory[0].0[1].fill(5);
        mem.additional_memory[0].0[2].fill(6);
        mem.additional_memory[0].0[3].fill(7);

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank0))
                .read(0),
            0
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank1))
                .read(0),
            1
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank2))
                .read(0),
            2
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank3))
                .read(0),
            3
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank0))
                .read(0),
            4
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank1))
                .read(0),
            5
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank2))
                .read(0),
            6
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank3))
                .read(0),
            7
        );
    }

    /// A stock 128K CPC (the default: 1 extra 64kb page) only wires a
    /// comparator for that one page - the gate array's page-select field is
    /// 3 bits wide (0-7) regardless, and real hardware doesn't trap or
    /// crash when firmware/software (e.g. a memory-sizing routine run at
    /// every cold boot, or a hardware diagnostic tool probing how much RAM
    /// is really fitted) selects one of the pages it doesn't have: it
    /// aliases back onto the page(s) that *are* fitted. This is a
    /// regression test for exactly that panic (`assert!(u <= 3)` in
    /// `PageNumber::from`, `src/memory.rs:210` before the fix), found by
    /// running the real `AmstradDiag.dsk` hardware diagnostics tool (see
    /// `tests/amstrad_diagnostics.rs`) - not a hypothetical case.
    #[test]
    fn a_page_beyond_what_is_fitted_aliases_instead_of_panicking() {
        let mut mem = Memory::default();
        assert_eq!(mem.extra_pages(), 1, "a stock CPC has exactly 1 extra page by default");

        mem.additional_memory[0].0[0].fill(0x42);

        for page in 0u8..=7 {
            assert_eq!(
                mem.ram_bank(&BankConfig(PageNumber::Page(page), BankNumber::Bank0)).read(0),
                0x42,
                "page {page} should alias back onto the only page actually fitted"
            );
        }

        // Same for the write path, and for a freshly-shrunk configuration
        // (exercises `set_extra_pages` itself, not just the Default impl).
        mem.set_extra_pages(1);
        mem.ram_bank_mut(&BankConfig(PageNumber::Page(5), BankNumber::Bank1)).write(0, 0x99);
        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank1)).read(0),
            0x99,
            "writing through an unfitted page's alias should land on the real page"
        );
    }

    /// A machine actually configured with more than the default one extra
    /// page (e.g. a full 512K expansion, `set_extra_ram_pages(7)` on
    /// `Motherboard`/`MotherboardBuilder`) must address each extra page
    /// independently - this is the "genuine expanded memory" counterpart to
    /// `a_page_beyond_what_is_fitted_aliases_instead_of_panicking`'s
    /// "gracefully alias when unexpanded" case.
    #[test]
    fn extra_pages_are_independent_when_actually_fitted() {
        let mut mem = Memory::default();
        mem.set_extra_pages(7);
        assert_eq!(mem.extra_pages(), 7);

        for page in 0u8..7 {
            mem.ram_bank_mut(&BankConfig(PageNumber::Page(page), BankNumber::Bank0))
                .write(0, 0x10 + page);
        }
        for page in 0u8..7 {
            assert_eq!(
                mem.ram_bank(&BankConfig(PageNumber::Page(page), BankNumber::Bank0)).read(0),
                0x10 + page,
                "page {page} should hold its own value, independent of the other fitted pages"
            );
        }

        // Shrinking back down shouldn't panic either, and re-exercises the
        // aliasing path on a configuration that used to be bigger.
        mem.set_extra_pages(1);
        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(3), BankNumber::Bank0)).read(0),
            0x10,
            "after shrinking, every page number should alias back onto page 0"
        );
    }
}
