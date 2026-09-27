use crate::chip::*;
use crate::memory::*;
use delegate::delegate;

use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;

/// The real CPC pal does not have access to D3-D5.
/// Extra memory selection is done elsewhere and I while not code the extra logic elswhere. Thus, there are more pins there
#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum PalPin {
    /// Reset at start
    RESET = 1 << 3,
    /// Disable motherboard RAM
    RAMDIS = 1 << 4,

    CAS = 1 << 5,
    A15 = 1 << 6,
    A14 = 1 << 7,

    GND = 1 << 8,
    CPU = 1 << 9,

    A15_OUT = 1 << 12,
    CAS1 = 1 << 13,
    CAS0 = 1 << 14,

    IOWR = 1 << 15,

    A14_OUT = 1 << 16,
    VDD = 1 << 17,

    D0 = 1 << 32,
    D1 = 1 << 33,
    D2 = 1 << 34,
    D3 = 1 << 35,
    D4 = 1 << 36,
    D5 = 1 << 37,
    D6_and_D7 = 1 << 38,
}

impl Into<u64> for PalPin {
    fn into(self) -> u64 {
        self as u64
    }
}

const PAL_PIN_DATA_BUS: [PalPin; 8] = [
    PalPin::D6_and_D7,
    PalPin::D6_and_D7,
    PalPin::D5,
    PalPin::D4,
    PalPin::D3,
    PalPin::D2,
    PalPin::D1,
    PalPin::D0,
];

/// MMR register that contains the RAM memory configuration
#[derive(Debug)]
pub struct MMR(u8);

impl MMR {
    pub fn set(&mut self, b: u8) {
        assert_eq!(0b11000000 & b, 0b11000000);
        self.0 = b
    }

    pub fn get(&self) -> u8 {
        self.0
    }
}

#[derive(Debug, PartialEq)]
pub struct MemoryMapping(u8);
#[derive(Debug, PartialEq)]
pub struct PageMapping(u8);

impl Into<PageNumber> for PageMapping {
    fn into(self) -> PageNumber {
        self.0.into()
    }
}

impl Into<BankNumber> for MemoryMapping {
    fn into(self) -> BankNumber {
        self.0.into()
    }
}

impl MMR {
    pub fn memory_mapping(&self) -> MemoryMapping {
        MemoryMapping(self.0 & 0b11)
    }

    pub fn use_supplementary_pages(&self) -> bool {
        (self.0 & 1 << 2) != 0
    }

    pub fn page_mapping(&self) -> PageMapping {
        PageMapping((self.0 & 0b111000) >> 3)
    }

    /// Provides the RAM configuration given the MMR value
    pub fn get_ram_configuration(&self) -> RamConfig {
        if self.use_supplementary_pages() {
            RamConfig([
                BankConfig(PageNumber::Base, BankNumber::Bank0),
                BankConfig(self.page_mapping().into(), self.memory_mapping().into()),
                BankConfig(PageNumber::Base, BankNumber::Bank2),
                BankConfig(PageNumber::Base, BankNumber::Bank3),
            ])
        } else {
            match self.memory_mapping() {
                MemoryMapping(0) => RamConfig([
                    BankConfig(PageNumber::Base, BankNumber::Bank0),
                    BankConfig(PageNumber::Base, BankNumber::Bank1),
                    BankConfig(PageNumber::Base, BankNumber::Bank2),
                    BankConfig(PageNumber::Base, BankNumber::Bank3),
                ]),

                MemoryMapping(1) => RamConfig([
                    BankConfig(PageNumber::Base, BankNumber::Bank0),
                    BankConfig(PageNumber::Base, BankNumber::Bank1),
                    BankConfig(PageNumber::Base, BankNumber::Bank2),
                    BankConfig(self.page_mapping().into(), BankNumber::Bank3),
                ]),

                MemoryMapping(2) => RamConfig([
                    BankConfig(self.page_mapping().into(), BankNumber::Bank0),
                    BankConfig(self.page_mapping().into(), BankNumber::Bank1),
                    BankConfig(self.page_mapping().into(), BankNumber::Bank2),
                    BankConfig(self.page_mapping().into(), BankNumber::Bank3),
                ]),

                MemoryMapping(3) => RamConfig([
                    BankConfig(PageNumber::Base, BankNumber::Bank0),
                    BankConfig(PageNumber::Base, BankNumber::Bank3),
                    BankConfig(PageNumber::Base, BankNumber::Bank2),
                    BankConfig(self.page_mapping().into(), BankNumber::Bank3),
                ]),

                _ => unreachable!(),
            }
        }
    }
}

/// Pal manages the memory selection.
/// On a real CPC it does not have access to bit 3-5 of the data.
/// Here, we take the liberty to do it
#[derive(Debug)]
pub struct Pal {
    pins: PinsBuffer,
    mmr: MMR,
}

impl DataChip for Pal {
    fn pins_data() -> &'static [Self::PinType; 8] {
        return &PAL_PIN_DATA_BUS;
    }
}

impl ReadDataChip for Pal {}

impl Default for Pal {
    fn default() -> Self {
        Pal {
            pins: (0, 0).into(),
            mmr: MMR(0xc0),
        }
    }
}

impl Chip for Pal {
    type PinType = PalPin;

    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &PinsBuffer {
        &self.pins
    }

    fn must_execute_tick(&self) -> bool {
        !self.get_pin(PalPin::A15) && self.get_pin(PalPin::A14) && self.get_pin(PalPin::IOWR)
    }

    /// Retreive the MMR value and store it
    fn execute_tick(&mut self) {
        let a = self.get_data_from_pins();
        if (a & (1 << 7)) != 0 && (a & (1 << 6) != 0) {
            self.mmr.set(a);
        }
    }

    fn reset(&mut self) {
        self.mmr = MMR(0xc0);
    }
}

impl Pal {
    delegate! {
        // Delegate all methods EXCEPT video mode as it is bufferized
        to self.mmr {
            pub fn memory_mapping(&self) -> MemoryMapping;
            pub fn use_supplementary_pages(&self) -> bool;
            pub fn page_mapping(&self) -> PageMapping;
            pub fn get_ram_configuration(&self) -> RamConfig;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_activation() {
        let mut pal = Pal::default();
    }

    #[test]
    fn test_memory_decoding() {
        let mut pal = Pal::default();

        pal.mmr.set(0xc0);
        assert_eq!(pal.mmr.memory_mapping(), MemoryMapping(0));
        assert_eq!(pal.mmr.use_supplementary_pages(), false);
        assert_eq!(pal.mmr.page_mapping(), PageMapping(0));
        assert_eq!(
            pal.mmr.get_ram_configuration(),
            RamConfig([
                BankConfig(PageNumber::Base, BankNumber::Bank0),
                BankConfig(PageNumber::Base, BankNumber::Bank1),
                BankConfig(PageNumber::Base, BankNumber::Bank2),
                BankConfig(PageNumber::Base, BankNumber::Bank3),
            ])
        );

        pal.mmr.set(0xc7);
        assert_eq!(pal.mmr.memory_mapping(), MemoryMapping(3));
        assert_eq!(pal.mmr.use_supplementary_pages(), true);
        assert_eq!(pal.mmr.page_mapping(), PageMapping(0));
        assert_eq!(
            pal.mmr.get_ram_configuration(),
            RamConfig([
                BankConfig(PageNumber::Base, BankNumber::Bank0),
                BankConfig(0.into(), 3.into()),
                BankConfig(PageNumber::Base, BankNumber::Bank2),
                BankConfig(PageNumber::Base, BankNumber::Bank3),
            ])
        );
    }
}
