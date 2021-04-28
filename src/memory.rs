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
    roms: Vec<Option<Rom>>
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
            roms
        }
    }
}