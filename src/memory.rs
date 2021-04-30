/// A Bank contains 16kb
#[derive(Clone)]
pub struct Bank([u8; 0x4000]);
impl Default for Bank {
    fn default() -> Self {
        Self([0; 0x4000])
    }
}

/// A Page contains 4 Banks
#[derive(Default, Clone)]
pub struct Page([Bank; 4]);

#[derive(Clone)]
pub struct Rom([u8; 0x4000]);
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

pub struct Memory {
    /// Main 64kb of the machine
    main_memory: Page,
    /// Additional pages (1 page for a standard CPC)
    additional_memory: Vec<Page>,

    /// Roms
    roms: Vec<Option<Rom>>,
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

/// Number of the page
#[derive(PartialEq, Copy, Clone, Debug)]
pub enum PageNumber {
    /// Main memory linked to CRTC
    Base,
    /// From 0 to 3 in an extra page
    Page(u8),
}

impl From<u8> for PageNumber {
    fn from(u: u8) -> Self {
        assert!(u <= 3);
        PageNumber::Page(u)
    }
}

/// Number of the bank
#[derive(PartialEq, Copy, Clone, Debug)]
pub enum BankNumber {
    Bank0,
    Bank1,
    Bank2,
    Bank3,
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

#[derive(PartialEq, Copy, Clone, Debug)]
pub struct BankConfig(pub PageNumber, pub BankNumber);

// [0x0000 - 0x3fff]
// [0x4000 - 0x7fff]
// [0x8000 - 0xbfff]
//[0xc000 - 0xffff]
#[derive(PartialEq, Copy, Clone, Debug)]
pub struct RamConfig(pub [BankConfig; 4]);
