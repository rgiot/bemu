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


/// I have no idea of how is coded the ROM selection stuff within the CPC.
/// So I create this chip only for this purpose
#[derive(Default)]
pub struct UpperRomSelector {
    rom_number: u8,
    pins: PinsBuffer
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum Flag {
    Enable,
    Disable
}
impl Default for Flag {
    fn default()-> Self {
        Flag::Disable
    }
}

impl Flag {
    fn is_enabled(&self) -> bool {
        match self {
            Flag::Enable => true,
            Flag::Disable => false
        }
    }
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct RomConfig{
    pub lower_rom: Flag,
    pub upper_rom: Flag,
    pub rom_number: u8
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct MemoryConfig (
    pub RomConfig,
    pub RamConfig
);


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
    WR = 1 << 10

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
        self.0.iter_mut()
            .for_each(|m| *m = v);
    }

    fn write(&mut self, addr: u16, val: u8){
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

    fn amsdos() -> Self {
        Self(Bank(*include_bytes!("../roms/amsdos.rom")))
    }

    fn os6128_azerty() -> Self {
        Self(Bank(*include_bytes!("../roms/os6128_azerty.rom")))
    }
}



impl From<u8> for PageNumber {
    fn from(u: u8) -> Self {
        assert!(u <= 3);
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
        self.get_pin(UpperRomSelectorPin::A13) &&
        self.get_pin(UpperRomSelectorPin::IORQ) &&
        self.get_pin(UpperRomSelectorPin::WR)
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
}

impl Default for Memory {
    fn default() -> Self {
        let mut roms = Vec::new();
        roms.push(Some(Rom::basic_6128_azerty())); // 0
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
            lower_rom: Rom::os6128_azerty()
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> MemoryConfig {
        MemoryConfig(
            RomConfig{
                lower_rom: Flag::Disable,
                upper_rom: Flag::Disable,
                rom_number: 7
            },
            RamConfig ([
                BankConfig(PageNumber::Base, BankNumber::Bank0),
                BankConfig(PageNumber::Base, BankNumber::Bank1),
                BankConfig(PageNumber::Base, BankNumber::Bank2),
                BankConfig(PageNumber::Base, BankNumber::Bank3),
            ])
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
        self.0.upper_rom.is_enabled() &&
        addr >= 0xc000
    }

    /// Return the Bank configuration for the given address
    pub fn bank_at(&self, addr: u16) -> &BankConfig {

        match addr {
            0x0000 .. 0x4000 => &self.1.0[0],
            0x4000 .. 0x8000 => &self.1.0[1],
            0x8000 .. 0xc000 => &self.1.0[2],
            0xc000 ..  => &self.1.0[3],
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


    /// Get the appropriate bank to read for the given configuration
    fn bank(&self, addr: u16, conf: &MemoryConfig) -> & Bank {
        if conf.is_in_enabled_lower_rom(addr) {
            self.lower_rom_bank()
        }
        else if conf.is_in_enabled_upper_rom(addr) {
            self.upper_rom_bank(conf.0.rom_number)
        }
        else {
            self.ram_bank(conf.bank_at(addr))
        }
    }

    /// TODO implement properly the behavior when a rom indice is wrong
    fn upper_rom_bank(&self, number: u8) -> &Bank {
        let number = number as usize;

        if number >= self.upper_roms.len() {
            self.upper_rom_bank(0)
        }
        else {
            match & self.upper_roms[number] {
                Some(r) => &r.0,
                None => {
                    assert!(number!=0);
                    self.upper_rom_bank(0)
                }
            }
        }
    }

    fn lower_rom_bank(&self) -> & Bank {
        &self.lower_rom.0
    }


    /// Get the memory for the given bank
    fn ram_bank(&self, conf: &BankConfig) -> &Bank {
        let page = match conf.0 {
            PageNumber::Base => &self.main_memory,
            PageNumber::Page(p) => &self.additional_memory[p as usize]
        };

        page.bank(conf.1)
    }

        /// Get the memory for the given bank
        fn ram_bank_mut(&mut self, conf: &BankConfig) -> &mut Bank {
            let page = match conf.0 {
                PageNumber::Base => &mut self.main_memory,
                PageNumber::Page(p) => &mut self.additional_memory[p as usize]
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
}