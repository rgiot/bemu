use crate::chip::*;

/// A Bank contains 16kb
#[derive(Clone)]
pub struct Bank([u8; 0x4000]);

/// A Page contains 4 Banks
#[derive(Default, Clone)]
pub struct Page([Bank; 4]);

#[derive(Clone)]
pub struct Rom(Bank);

pub trait Readable {
    fn read(&self, addr: u16) -> u8;
}

pub trait Writeable {
    fn fill(&mut self, v: u8);
    fn write(&mut self, addr: u16, byte: u8);
}

/// What a card plugged into the expansion port can provide to `Memory`.
///
/// Real expansion cards vary hugely in what they actually carry - a pure
/// RAM board provides none of this beyond RAM banking; something like the
/// M4 (<https://github.com/M4Duke/m4hardware>) bundles RAM, its own ROM
/// (<https://github.com/M4Duke/m4rom>, <https://github.com/M4Duke/m4hackmenu>),
/// persistent storage and more on one card. `Memory` never owns any of this
/// storage - real hardware puts the RAM/ROM chips *on the card*, not inside
/// the CPC - it only ever *borrows* a [`Page`]/[`Bank`] through this trait
/// to service one read or write. Every method has a default "I don't
/// provide this" implementation, so a card only overrides the capabilities
/// it actually has (see [`crate::expansion::LargeRamExpansion`], which only
/// overrides the RAM-banking methods).
///
/// Deliberately *not* covering here: anything that isn't a byte
/// [`Memory`] itself addresses. Persistent storage (an SD/CF card behind
/// its own command protocol) or a "hacker menu" (effectively a second,
/// separate computer riding the bus) are peripherals in their own right,
/// modelled the same way `Fdc`/other chips already are - their own
/// pin-level `Chip` and/or their own command-and-result state machine, not
/// a method on this trait. Implementing that for a specific real card (M4
/// or otherwise) needs that card's own protocol researched first, the same
/// way the RAM banking protocol here was confirmed against its actual spec
/// rather than guessed at - not done in this codebase yet.
pub trait Expansion {
    /// How many 64kb RAM banks this card provides (0 if it doesn't provide
    /// RAM banking at all - the default).
    fn ram_bank_count(&self) -> usize {
        0
    }

    /// Never called unless `ram_bank_count()` is nonzero for the same
    /// card - the default panics rather than silently reading zeroes, so a
    /// card that only forgets to override this fails loudly.
    fn ram_page(&self, bank: u8) -> &Page {
        let _ = bank;
        panic!("this expansion doesn't provide RAM banking (ram_bank_count() should have been 0)")
    }

    fn ram_page_mut(&mut self, bank: u8) -> &mut Page {
        let _ = bank;
        panic!("this expansion doesn't provide RAM banking (ram_bank_count() should have been 0)")
    }

    /// If this card provides upper ROM slot `number` (its own firmware,
    /// e.g. a hacker menu), the image itself - `None` (the default) falls
    /// through to whatever's normally in that slot.
    fn upper_rom(&self, number: u8) -> Option<&Bank> {
        let _ = number;
        None
    }
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
    /// One of the CPC's own on-board extra pages (`Memory::additional_memory`,
    /// max 7 = the standard CPC6128-style RAM select's own 512K limit).
    Page(u8),
    /// One of the banks a card plugged into the expansion port provides
    /// (`Expansion::ram_bank_count`/`ram_page`, max 64 = 4MB) - real
    /// hardware puts this RAM on the card itself, so `Memory` never owns
    /// it, only addresses it.
    Expansion(u8),
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

impl BankConfig {
    /// The 16K bank index used by debuggers (AMSpiriT's `Bnn`): 0-3 for the
    /// base 64K, then 4 per extra 64K page - on-board (`Page`) or on an
    /// expansion card (`Expansion`, whose low bits are the same page).
    pub fn physical_index(&self) -> u16 {
        let bank = self.1.index() as u16;
        match self.0 {
            PageNumber::Base => bank,
            PageNumber::Page(p) | PageNumber::Expansion(p) => 4 + 4 * p as u16 + bank
        }
    }

    /// The bank with that physical index: `Page` when the extra RAM is on
    /// board, `Expansion` when a large RAM card provides it.
    pub fn of_physical_index(index: u16, on_expansion_card: bool) -> Self {
        let bank = [BankNumber::Bank0, BankNumber::Bank1, BankNumber::Bank2, BankNumber::Bank3][index as usize % 4];
        if index < 4 {
            return Self(PageNumber::Base, bank);
        }
        let page = ((index - 4) / 4) as u8;
        Self(if on_expansion_card { PageNumber::Expansion(page) } else { PageNumber::Page(page) }, bank)
    }
}

/// Where a byte physically is - what a CPU address designates under a given
/// mapping. For debuggers: a breakpoint on a physical location fires
/// whatever address the code is reached through, and only when that RAM
/// bank or ROM is mapped.
#[derive(PartialEq, Eq, Hash, Copy, Clone, Debug)]
pub enum PhysicalAddress {
    /// A 16K RAM bank (see [`BankConfig::physical_index`]) and an offset in it.
    Ram { bank: u16, offset: u16 },
    LowerRom { offset: u16 },
    UpperRom { rom: u8, offset: u16 }
}

// [0x0000 - 0x3fff]
// [0x4000 - 0x7fff]
// [0x8000 - 0xbfff]
//[0xc000 - 0xffff]
#[derive(PartialEq, Copy, Clone, Debug)]
pub struct RamConfig(pub [BankConfig; 4]);

impl RamConfig {
    /// Combines a large RAM expansion's extra high bits (see
    /// `crate::expansion::LargeRamExpansion`) into whichever quadrant the
    /// gate array/PAL's own `PageNumber::Page` selects, turning its 3-bit
    /// page number (0-7, this config's own D3-D5 bits) into a
    /// `PageNumber::Expansion` holding the full 6-bit bank number
    /// (`(high_bits << 3) | page`, up to 4MB) the expansion protocol
    /// defines - the expansion's own storage, not `Memory`'s. A no-op
    /// wherever this config already selects `PageNumber::Base` (those
    /// quadrants are the CPC's own base 64K, which no RAM-select write -
    /// extended or not - ever touches).
    pub fn with_expansion_high_bits(self, high_bits: u8) -> RamConfig {
        RamConfig(self.0.map(|BankConfig(page, bank)| {
            let page = match page {
                PageNumber::Base => PageNumber::Base,
                PageNumber::Page(p) => PageNumber::Expansion((high_bits << 3) | p),
                PageNumber::Expansion(p) => PageNumber::Expansion(p),
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
#[derive(Default)]
pub enum Flag {
    Enable,
    #[default]
    Disable,
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

impl From<UpperRomSelectorPin> for u64 {
    fn from(val: UpperRomSelectorPin) -> Self {
        val as u64
    }
}

impl From<&UpperRomSelectorPin> for u64 {
    fn from(val: &UpperRomSelectorPin) -> Self {
        *val as u64
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
    /// Power-on content of a RAM bank: 8 bytes of #00 then 8 bytes of #FF,
    /// over and over (what AMSpiriT fills every bank with; programs such
    /// as Logon System's Dsc4 read RAM they never wrote, and depend on it).
    fn default() -> Self {
        let mut bank = [0u8; 0x4000];
        bank.iter_mut().enumerate().for_each(|(offset, byte)| {
            if offset & 8 != 0 {
                *byte = 0xff;
            }
        });
        Self(bank)
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
        Self(Bank(*crate::roms::BASIC6128_AZERTY))
    }

    fn basic_6128() -> Self {
        Self(Bank(*crate::roms::BASIC6128))
    }

    fn amsdos() -> Self {
        Self(Bank(*crate::roms::AMSDOS))
    }

    fn os6128_azerty() -> Self {
        Self(Bank(*crate::roms::OS6128_AZERTY))
    }

    fn os6128() -> Self {
        Self(Bank(*crate::roms::OS6128))
    }

    fn os464() -> Self {
        Self(Bank(*crate::roms::OS464))
    }

    fn basic464() -> Self {
        Self(Bank(*crate::roms::BASIC464))
    }

    fn os664() -> Self {
        Self(Bank(*crate::roms::OS664))
    }

    fn basic664() -> Self {
        Self(Bank(*crate::roms::BASIC664))
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
        &URS_PIN_DATA_BUS
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
        let mut roms = vec![Some(Rom::basic_6128())]; // 0
        roms.extend([None, None, None, None, None, None]);
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
    pub fn bank(&self, id: BankNumber) -> &Bank {
        &self.0[id.index()]
    }

    pub fn bank_mut(&mut self, id: BankNumber) -> &mut Bank {
        &mut self.0[id.index()]
    }
}

impl Memory {
    /// Fit the ROMs of `model`: its OS (lower ROM) and its BASIC (upper
    /// ROM 0), French ones for a 6128 with the French layout. AMSDOS comes with the disc interface
    /// ([`Self::set_amsdos`]).
    pub fn set_firmware(&mut self, model: crate::motherboard::CpcModel, layout: crate::keyboard::KeyboardLayout) {
        use crate::keyboard::KeyboardLayout;
        use crate::motherboard::CpcModel;
        let (os, basic) = match (model, layout) {
            (CpcModel::Cpc464, _) => (Rom::os464(), Rom::basic464()),
            (CpcModel::Cpc664, _) => (Rom::os664(), Rom::basic664()),
            (CpcModel::Cpc6128, KeyboardLayout::Uk) => (Rom::os6128(), Rom::basic_6128()),
            (CpcModel::Cpc6128, KeyboardLayout::French) => (Rom::os6128_azerty(), Rom::basic_6128_azerty())
        };
        self.lower_rom = os;
        self.upper_roms[0] = Some(basic);
    }

    /// Fit `data` as upper ROM `slot` (a 16K ROM; an AMSDOS header before it is skipped, a
    /// shorter one is padded with &FF), or nothing there with `None`.
    pub fn set_upper_rom(&mut self, slot: u8, data: Option<&[u8]>) -> Result<(), String> {
        let rom = match data {
            None => None,
            Some(data) => {
                let data = match data.len() {
                    0x4080 => &data[0x80..],
                    len if len > 0x4000 => return Err(format!("a ROM is 16K at most, not {len} bytes")),
                    _ => data
                };
                let mut bank = [0xffu8; 0x4000];
                bank[..data.len()].copy_from_slice(data);
                Some(Rom(Bank(bank)))
            }
        };
        let slot = usize::from(slot);
        if self.upper_roms.len() <= slot {
            self.upper_roms.resize_with(slot + 1, || None);
        }
        self.upper_roms[slot] = rom;
        Ok(())
    }

    /// Fit AMSDOS as upper ROM 7 (a disc interface), or nothing there.
    pub fn set_amsdos(&mut self, fitted: bool) {
        self.upper_roms[7] = fitted.then(Rom::amsdos);
    }

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

    /// Fill extra page `page` (64K: banks 0-3 in order) with `data`; growing
    /// the memory if the page is not fitted yet.
    pub fn load_extra_page(&mut self, page: usize, data: &[u8]) {
        if page >= self.additional_memory.len() {
            self.additional_memory.resize_with(page + 1, Page::default);
        }
        for (i, chunk) in data.chunks(0x4000).take(4).enumerate() {
            for (offset, byte) in chunk.iter().enumerate() {
                self.additional_memory[page].0[i].0[offset] = *byte;
            }
        }
    }

    /// The 64K of RAM of `page`: the main memory for `None`, else that extra page.
    pub fn page_dump(&self, page: Option<usize>) -> Vec<u8> {
        let page = match page {
            None => &self.main_memory,
            Some(page) => &self.additional_memory[page]
        };
        page.0.iter().flat_map(|bank| bank.0.iter().copied()).collect()
    }

    /// How many extra 64kb pages are currently fitted (see
    /// [`Memory::set_extra_pages`]).
    pub fn extra_pages(&self) -> usize {
        self.additional_memory.len()
    }

    /// Read a byte according to the provided memory configuration.
    /// If the address is in an activate rom space; this rom is read instead of the ram
    /// If the adress is within the ram space, then the value depends on the ram configuration
    /// TODO manage strange configuration with missing roms
    ///
    /// `expansion` is whatever card is plugged into the expansion port, if
    /// any (see [`Expansion`]).
    pub fn read(&self, addr: u16, conf: &MemoryConfig, expansion: Option<&dyn Expansion>) -> u8 {
        let byte = if conf.is_in_enabled_lower_rom(addr) {
            self.lower_rom_bank().read(MemoryConfig::relative_address_at(addr))
        } else if conf.is_in_enabled_upper_rom(addr) {
            self.upper_rom_bank(conf.0.rom_number, expansion).read(MemoryConfig::relative_address_at(addr))
        } else {
            self.ram_bank(conf.bank_at(addr), expansion).read(MemoryConfig::relative_address_at(addr))
        };
        byte
    }

    /// Write a byte according to the memory configuration.
    /// write is done on RAM even if ROMs are activated
    ///
    /// `expansion` is whatever card is plugged into the expansion port, if
    /// any (see [`Expansion`]) - only consulted when `conf` selects
    /// `PageNumber::Expansion`, which only happens when one actually is.
    pub fn write<'a>(&'a mut self, addr: u16, val: u8, conf: &MemoryConfig, expansion: Option<&'a mut dyn Expansion>) {
        let bank = self.ram_bank_mut(conf.bank_at(addr), expansion);
        bank.write(MemoryConfig::relative_address_at(addr), val);
    }

    /// TODO implement properly the behavior when a rom indice is wrong
    fn upper_rom_bank<'a>(&'a self, number: u8, expansion: Option<&'a dyn Expansion>) -> &'a Bank {
        if let Some(rom) = expansion.and_then(|e| e.upper_rom(number)) {
            return rom;
        }

        let index = number as usize;
        if index >= self.upper_roms.len() {
            self.upper_rom_bank(0, expansion)
        } else {
            match &self.upper_roms[index] {
                Some(r) => &r.0,
                None => {
                    assert!(number != 0);
                    self.upper_rom_bank(0, expansion)
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
    /// indexing out of bounds. `PageNumber::Expansion` reaches into
    /// whatever's plugged into the expansion port the same way - see
    /// [`Expansion`] - and is never produced unless one actually is (see
    /// `RamConfig::with_expansion_high_bits`), but wraps defensively too
    /// rather than panicking if `expansion` somehow provides fewer banks
    /// than the bank number asks for.
    fn ram_bank<'a>(&'a self, conf: &BankConfig, expansion: Option<&'a dyn Expansion>) -> &'a Bank {
        match conf.0 {
            PageNumber::Base => self.main_memory.bank(conf.1),
            PageNumber::Page(p) => self.additional_memory[p as usize % self.additional_memory.len()].bank(conf.1),
            PageNumber::Expansion(bank) => {
                let expansion = expansion.expect("PageNumber::Expansion produced but no expansion is plugged in");
                let count = expansion.ram_bank_count().max(1) as u8;
                expansion.ram_page(bank % count).bank(conf.1)
            }
        }
    }

    /// Get the memory for the given bank. See [`Memory::ram_bank`] for why
    /// the page index wraps instead of being asserted in range.
    fn ram_bank_mut<'a>(&'a mut self, conf: &BankConfig, expansion: Option<&'a mut dyn Expansion>) -> &'a mut Bank {
        match conf.0 {
            PageNumber::Base => self.main_memory.bank_mut(conf.1),
            PageNumber::Page(p) => {
                let len = self.additional_memory.len();
                self.additional_memory[p as usize % len].bank_mut(conf.1)
            }
            PageNumber::Expansion(bank) => {
                let expansion = expansion.expect("PageNumber::Expansion produced but no expansion is plugged in");
                let count = expansion.ram_bank_count().max(1) as u8;
                expansion.ram_page_mut(bank % count).bank_mut(conf.1)
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn memory_access() {
        let mut mem = Memory::default();
        let _conf = MemoryConfig::default();

        mem.main_memory.0[0].fill(0);
        mem.main_memory.0[1].fill(1);
        mem.main_memory.0[2].fill(2);
        mem.main_memory.0[3].fill(3);

        mem.additional_memory[0].0[0].fill(4);
        mem.additional_memory[0].0[1].fill(5);
        mem.additional_memory[0].0[2].fill(6);
        mem.additional_memory[0].0[3].fill(7);

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank0), None)
                .read(0),
            0
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank1), None)
                .read(0),
            1
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank2), None)
                .read(0),
            2
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank3), None)
                .read(0),
            3
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank0), None)
                .read(0),
            4
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank1), None)
                .read(0),
            5
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank2), None)
                .read(0),
            6
        );

        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank3), None)
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
                mem.ram_bank(&BankConfig(PageNumber::Page(page), BankNumber::Bank0), None).read(0),
                0x42,
                "page {page} should alias back onto the only page actually fitted"
            );
        }

        // Same for the write path, and for a freshly-shrunk configuration
        // (exercises `set_extra_pages` itself, not just the Default impl).
        mem.set_extra_pages(1);
        mem.ram_bank_mut(&BankConfig(PageNumber::Page(5), BankNumber::Bank1), None).write(0, 0x99);
        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(0), BankNumber::Bank1), None).read(0),
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
            mem.ram_bank_mut(&BankConfig(PageNumber::Page(page), BankNumber::Bank0), None)
                .write(0, 0x10 + page);
        }
        for page in 0u8..7 {
            assert_eq!(
                mem.ram_bank(&BankConfig(PageNumber::Page(page), BankNumber::Bank0), None).read(0),
                0x10 + page,
                "page {page} should hold its own value, independent of the other fitted pages"
            );
        }

        // Shrinking back down shouldn't panic either, and re-exercises the
        // aliasing path on a configuration that used to be bigger.
        mem.set_extra_pages(1);
        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Page(3), BankNumber::Bank0), None).read(0),
            0x10,
            "after shrinking, every page number should alias back onto page 0"
        );
    }

    /// Not every expansion card has RAM at all - e.g. a pure ROM board, or
    /// something like the M4 (<https://github.com/M4Duke/m4hardware>)'s own
    /// firmware ROM (<https://github.com/M4Duke/m4rom>,
    /// <https://github.com/M4Duke/m4hackmenu>) sitting in an upper ROM slot
    /// alongside its (separately modelled, not this trait's concern) RAM,
    /// storage and menu capabilities. `Expansion`'s default
    /// `ram_bank_count() == 0` means such a card simply never overrides the
    /// RAM methods at all - proven here with a minimal card that *only*
    /// implements `upper_rom`, and confirming it plugging in doesn't
    /// disturb `PageNumber::Page`/`Base` RAM addressing (which never
    /// touches `Expansion` in the first place - only
    /// `RamConfig::with_expansion_high_bits`, itself only ever called for a
    /// RAM-providing card, would).
    struct RomOnlyExpansion {
        rom: Bank
    }

    impl Expansion for RomOnlyExpansion {
        fn upper_rom(&self, number: u8) -> Option<&Bank> {
            (number == 7).then_some(&self.rom)
        }
    }

    #[test]
    fn a_rom_only_expansion_provides_no_ram_but_does_provide_its_rom() {
        let mut rom = Bank::default();
        rom.fill(0xAA);
        let card = RomOnlyExpansion { rom };

        assert_eq!(card.ram_bank_count(), 0, "the default: this card has no RAM at all");
        assert!(card.upper_rom(0).is_none(), "slot 0 (BASIC) isn't this card's to provide");
        assert_eq!(card.upper_rom(7).unwrap().read(0), 0xAA, "slot 7 is this card's own firmware");

        let mem = Memory::default();
        assert_eq!(
            mem.upper_rom_bank(7, Some(&card)).read(0),
            0xAA,
            "Memory should reach the card's ROM for the slot it claims"
        );
        assert_eq!(
            mem.upper_rom_bank(0, Some(&card)).read(0),
            mem.upper_rom_bank(0, None).read(0),
            "and fall through to its own BASIC ROM for a slot the card doesn't claim"
        );

        // Plain RAM addressing (Base/Page, no expansion high-bits combine)
        // is entirely unaffected by this ROM-only card being present.
        assert_eq!(
            mem.ram_bank(&BankConfig(PageNumber::Base, BankNumber::Bank0), None).read(0),
            0,
            "a ROM-only card must not disturb ordinary base RAM addressing"
        );
    }
}
