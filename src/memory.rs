use crate::chip::*;


/// A Bank contains 16kb
#[derive(Clone)]
pub struct Bank([u8; 0x4000]);

/// A Page contains 4 Banks
#[derive(Default, Clone)]
pub struct Page([Bank; 4]);

#[derive(Clone)]
pub struct Rom([u8; 0x4000]);



pub struct Memory {
    /// Main 64kb of the machine
    main_memory: Page,
    /// Additional pages (1 page for a standard CPC)
    additional_memory: Vec<Page>,

    /// Roms
    roms: Vec<Option<Rom>>,
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
    pins: u64
}

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

impl Default for Rom {
    fn default() -> Self {
        Self([0; 0x4000])
    }
}

impl Rom {
    fn basic_1_1() -> Self {
        unimplemented!();
    }

    fn amsdos() -> Self {
        unimplemented!();
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

    fn pins_buffer_mut(&mut self) -> &mut u64 {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &u64 {
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


impl Default for Memory {
    fn default() -> Self {
        let mut roms = Vec::new();
        roms.push(Some(Rom::basic_1_1())); // 0
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
            roms,
        }
    }
}
