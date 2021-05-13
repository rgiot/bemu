use crate::chip::*;
use crate::crtc::*;
use crate::ga::*;
use crate::memory::*;
use crate::pal::*;
use crate::z80::*;

/// Motherboard of the AMstrad CPC
/// Some component are emulated at the lowest level as possible (Z80/CRTC) while
/// some others are partially emulated (GA/PAL) or totally ignored (RAM)
/// http://www.cpcwiki.eu/index.php/Default_I/O_Port_Summary
pub struct Motherboard {
    cpu: Z80,
    crtc: Crtc,
    ga: GateArray,
    pal: Pal,
    rom_selector: UpperRomSelector,
    memory: Memory,
}

impl Default for Motherboard {
    fn default() -> Self {
        let mut motherboard = Motherboard { 
            cpu: Default::default(),
            crtc: Default::default(),
            ga: Default::default(),
            pal: Default::default(),
            rom_selector: Default::default(),
            memory: Default::default(),
        };
        motherboard.stabilize_run();
        motherboard
    }
}

/// TODO add the other kinds of varitions
#[derive(Default, Clone, Copy)]
pub struct MotherboardBuilder {
    crtc_type: Option<CrtcType>
}

impl MotherboardBuilder {
    pub fn set_crtc(&mut self, crtc_type: CrtcType) -> &mut Self {
        self.crtc_type = Some(crtc_type);
        self
    }

    pub fn build(self) -> Motherboard {
        let crtc_type = self.crtc_type.unwrap_or_default();

        let mut motherboard = Motherboard {
            crtc: crtc_type.into(),
            ..Default::default()
        };
        motherboard.stabilize_run();
        motherboard
    }
}

// Fields access
impl Motherboard {
    pub fn cpu(&self) -> &Z80 {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut Z80 {
        &mut self.cpu
    }
}

impl Motherboard {

    fn stabilize_run(&mut self) {
        self.reset();
        while !self.cpu.is_activated() {
            self.ga.manage_clocks();
            self.sync_chips_pins(); // just to get the front stuff
        }
        self.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA !!
    }

    /// Reset all the components of the motherboard
    pub fn reset(&mut self) {
        self.crtc.reset();
        self.ga.reset();
        self.pal.reset();
        self.cpu.reset();

        self.sync_chips_pins();
    }

    /// Execute one tick on all the components of the motherboard and returns the 
    /// corresponding gate array information
    /// TODO check if it must be generated before or after
    pub fn tick(&mut self) -> (RgbLevels, Sync) {

        dbg!(
            self.cpu.get_pin(Z80::clock_pin()),
            self.crtc.get_pin(Crtc::clock_pin())
        );

        // fill bus with memory data if needed
        self.handle_memory_access();

        // Keep this order to avoid side effect mixing previous state and current state
        if self.crtc.is_activated() {
            self.crtc.tick(); // Virtually uses 16 cycles
        }
        self.ga.tick();
        self.pal.tick();

        if self.cpu.is_activated() {
            self.cpu.tick(); // Virtually uses 4 cycles
        }

        let ga_output = (self.ga.rgb(), self.ga.sync());


        // Divides and distribute the 16Mhz clock for next round
        self.ga.manage_clocks();

        // make the synchro between the pins of the various chips
        // TODO replace that by a real bus abstraction
        self.sync_chips_pins();


        ga_output
    }


    /// manage the memory accesses (read/write cpu ; read ga)
    /// TODO implement orchestration between chips
    pub fn handle_memory_access(&mut self) {

        if self.cpu_request_byte_read() && !self.ga_request_read() {
            let addr = self.cpu.get_address_from_pins();
            let memory_cfg = self.memory_config();
            eprintln!("Want at 0x{:x} with {:?}", addr, memory_cfg);
            let byte = self.memory.read(addr, &memory_cfg);
            eprintln!("Read 0x{:x} at 0x{:x} with {:?}", byte, addr, memory_cfg) ;
            self.cpu.set_data_on_pins(byte);
            self.cpu.set_pin(Z80Pin::Neg_RD, false); // TODO do the real stuff - but here I am sure to not play with the data bus ...
        }

        else if self.cpu_request_byte_write() && !self.ga_request_read() {
            let addr = self.cpu.get_address_from_pins();
            let data = self.cpu.get_data_from_pins();
            let memory_cfg = self.memory_config();
            println!("Write 0x{:x} at 0x{:x}", data, addr) ;
            self.memory.write(addr, data, &memory_cfg);
            self.cpu.set_pin(Z80Pin::Neg_WR, false); // TODO do the real stuff - but here I am sure to not play with the data bus ...
        }
    }

    #[inline]
    fn cpu_request_byte_read(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_RD)
    }

    #[inline]
    fn cpu_request_byte_write(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_WR)
    }

    #[inline]
    /// TODO implement that aspect
    fn ga_request_read(&self) -> bool {
        false
    }

    /// Completly execute an opcode and returns the set of video information associated
    pub fn execute_next_opcode(&mut self) -> Vec<(RgbLevels, Sync)> {
        let mut video = Vec::new();
        assert!(self.cpu.next_state.is_new_opcode(), "We have to start by a new opcode");

        let mut has_executed_first_state = false;

        /// execute the whole stuff as soon as it is necessary
        loop {
            video.push(self.tick());

            has_executed_first_state = has_executed_first_state | !self.cpu.next_state.is_new_opcode();

            if self.cpu.is_activated() {
                dbg!(&self.cpu.next_state);
            }

            if self.cpu.is_activated() && self.cpu.next_state.is_new_opcode() && has_executed_first_state {
                break;
            }
        }

        video
    }

    /// Make the appropriate copy between some pins of some chips to other (we simulate the various buses here)
    fn sync_chips_pins(&mut self) {
        // Backup previous state
        self.ga.save_pins_state();
        self.pal.save_pins_state();
        self.crtc.save_pins_state();
        self.cpu.save_pins_state();

        // GA to CPU
        sync_pins(
            &self.ga,
            &[GateArrayPin::NCPU],
            &mut self.cpu,
            &[Z80Pin::Neg_CLK],
        );

        // GA to CRTC
        sync_pins(
            &self.ga,
            &[GateArrayPin::CCLK],
            &mut self.crtc,
            &[CrtcPin::CCLK],
        );

        // CPU to GA
        sync_pins(
            &self.cpu,
            Z80::pins_data(),
            &mut self.ga,
            GateArray::pins_data(),
        );
        sync_pins(
            &self.cpu,
            &[Z80Pin::A14, Z80Pin::A15],
            &mut self.ga,
            &[GateArrayPin::A14, GateArrayPin::A15],
        );

        // CPU to PAL
        sync_pins(
            &self.cpu,
            &[
                Z80Pin::A14,
                Z80Pin::A15,
                Z80Pin::D0,
                Z80Pin::D1,
                Z80Pin::D2,
                Z80Pin::D3,
                Z80Pin::D4,
                Z80Pin::D5,
            ],
            &mut self.pal,
            &[
                PalPin::A14,
                PalPin::A15,
                PalPin::D0,
                PalPin::D1,
                PalPin::D2,
                PalPin::D3,
                PalPin::D4,
                PalPin::D5,
            ],
        );
        self.pal.set_pin(
            PalPin::D6_and_D7,
            self.cpu.get_pin(Z80Pin::D6) && self.cpu.get_pin(Z80Pin::D7),
        );
        self.pal.set_pin(
            PalPin::IOWR,
            !self.cpu.get_pin(Z80Pin::Neg_WR) && !self.cpu.get_pin(Z80Pin::Neg_IORQ),
        );

        // CPU to CRTC
        self.crtc.set_pin(
            CrtcPin::EN,
            !self.cpu.get_pin(Z80Pin::Neg_IORQ)
                && (!self.cpu.get_pin(Z80Pin::Neg_RD) || !self.cpu.get_pin(Z80Pin::Neg_WR)),
        );

        sync_pins(
            &self.cpu,
            &[Z80Pin::A8, Z80Pin::A9, Z80Pin::A15],
            &mut self.crtc,
            &[CrtcPin::RS, CrtcPin::RW, CrtcPin::CS],
        );

        // CPU to rom selection
        sync_pins(
            &self.cpu,
            Z80::pins_data(),
            &mut self.rom_selector,
            UpperRomSelector::pins_data(),
        );
        sync_pins(
            &self.cpu,
            &[Z80Pin::A13, Z80Pin::Neg_IORQ, Z80Pin::Neg_WR],
            &mut self.rom_selector,
            &[
                UpperRomSelectorPin::A13,
                UpperRomSelectorPin::IORQ,
                UpperRomSelectorPin::WR,
            ],
        );
    }
}


// Block for the memory related functions
impl Motherboard {

    /// Provides the memory configuration for a proper access
    pub fn memory_config(&self) -> MemoryConfig {
        MemoryConfig(
            RomConfig {
                lower_rom: self.ga.lower_rom_pagging(),
                upper_rom: self.ga.upper_rom_pagging(),
                rom_number: self.rom_selector.upper_rom_number(),
            },
            self.pal.get_ram_configuration(),
        )
    }

    /// Write the databuffer in memory according to the memory configuration
    pub fn write_buffer(&mut self, mut addr: u16, buffer: &[u8]) {
        let config = self.memory_config();
        for byte in buffer.iter() {
            self.memory.write(addr, *byte, &config);
            addr = addr.wrapping_add(1);
        }
    }

    /// Read one memory byte according to the memory configuration
    #[inline]
    pub fn read_byte(&self, addr: u16) -> u8 {
        let memory_cfg = self.memory_config();
        self.memory.read(addr, &memory_cfg)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_pins_consistency() {
        let mut motherboard = Motherboard::default();
        motherboard.cpu.set_data_on_pins(0xfe);
        motherboard.sync_chips_pins();
        assert_eq!(motherboard.ga.get_data_from_pins(), 0xfe);
    }


    #[test]
    fn test_nop_execution() {
        let mut motherboard = Motherboard::default();
        let memory_config = motherboard.memory_config();
        
        // setup pins
        motherboard.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA !!
        // install the program
        motherboard.cpu.pc.set(0x4000);
        motherboard.memory.write(0x4000, 0x00, &memory_config);

        assert!(motherboard.cpu.next_state.is_new_opcode(), "We have to start by a new opcode");
        assert!(motherboard.cpu.is_activated());

        
        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(
              video.len(),           16 // 4 t-states for 4 mcycles
        );

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4001,
            "{:x} != {:x} ", motherboard.cpu.pc.value(), 0x4001
        );

    }


    #[test]
    fn test_xor_a_execution() {
        let mut motherboard = Motherboard::default();
        let memory_config = motherboard.memory_config();
        
        // setup pins
        motherboard.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA !!
        // install the program
        motherboard.cpu.pc.set(0x4000);
        motherboard.cpu.af.set(0xffff);
        motherboard.memory.write(0x4000, 0xaf, &memory_config);

        assert!(motherboard.cpu.next_state.is_new_opcode(), "We have to start by a new opcode");
        assert!(motherboard.cpu.is_activated());

        
        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(
              video.len(),           16 // 4 t-states for 4 mcycles
        );

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4001,
            "{:x} != {:x} ", motherboard.cpu.pc.value(), 0x4001
        );

        assert_eq!(
            motherboard.cpu.af.high().value(),
            0
        );

    }


    #[test]
    fn test_hl_dead_execution() {
        let mut motherboard = Motherboard::default();
        let memory_config = motherboard.memory_config();
        
        // setup pins
        // install the program
        motherboard.cpu.pc.set(0x4000);
        motherboard.cpu.hl.set(0xff);
        motherboard.memory.write(0x4000, 0x21, &memory_config);
        motherboard.memory.write(0x4001, 0xad, &memory_config);
        motherboard.memory.write(0x4002, 0xde, &memory_config);

        assert!(motherboard.cpu.next_state.is_new_opcode(), "We have to start by a new opcode");
        assert!(motherboard.cpu.is_activated());

        
        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(
            motherboard.cpu.hl.value(),
            0xdead
        );

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4003,
            "{:x} != {:x} ", motherboard.cpu.pc.value(), 0x4003
        );



        assert_eq!(
            video.len(), 
            4*3*4
      );

    }


    #[test]
    fn test_chips_activation() {
        let mut motherboard = Motherboard::default();
        
        // setup pins
        motherboard.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA 

        let mut count_cpu = 0;
        let mut count_crtc = 0;
        for i in 0..32 {
            println!("{},{},{}", true, motherboard.cpu.is_activated(), motherboard.crtc.is_activated());

            if motherboard.cpu.is_activated() {
                count_cpu += 1;
            }

            if motherboard.crtc.is_activated() {
                count_crtc += 1;
            }

            motherboard.ga.manage_clocks();
            motherboard.sync_chips_pins();

        }

        assert_eq!(
            count_cpu,
            32/4
        );

        assert_eq!(
            count_crtc,
            32/16
        );
    }
}
