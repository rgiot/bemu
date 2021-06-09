use cpclib_asm::preamble::Token;
use smallvec::SmallVec;
use smallvec::smallvec;

use crate::chip::*;
use crate::counter::*;
use crate::crtc::*;
use crate::debug::DisassembledInstruction;
use crate::ga::*;
use crate::memory::*;
use crate::pal::*;
use crate::ppi::*;
use crate::z80::*;

use std::convert::TryFrom;
use std::fmt;

/// Motherboard of the AMstrad CPC
/// Some component are emulated at the lowest level as possible (Z80/CRTC) while
/// some others are partially emulated (GA/PAL) or totally ignored (RAM)
/// http://www.cpcwiki.eu/index.php/Default_I/O_Port_Summary
pub struct Motherboard {
    cpu: Z80,
    crtc: Crtc,
    ga: GateArray,
    pal: Pal,
    ppi: Ppi,
    rom_selector: UpperRomSelector,
    memory: Memory,

    // Counter of t-states (to be removed later)
    t_state_counter: Counter,
    // Counter of nops
    nops_counter: Counter,
}

impl fmt::Debug for Motherboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Motherboard")
            .field("cpu", &self.cpu)
            .field("crtc", &self.crtc)
            .field("ga", &self.ga)
            .field("pal", &self.pal)
            .finish()
    }
}

impl Default for Motherboard {
    fn default() -> Self {
        let mut motherboard = Motherboard {
            cpu: Default::default(),
            crtc: Default::default(),
            ga: Default::default(),
            pal: Default::default(),
            ppi: Default::default(),
            rom_selector: Default::default(),
            memory: Default::default(),

            t_state_counter: Default::default(),
            nops_counter: Default::default(),
        };
        motherboard.stabilize_run();
        motherboard
    }
}

/// TODO add the other kinds of varitions
#[derive(Default, Clone, Copy)]
pub struct MotherboardBuilder {
    /// The type of CRTC in the motherboard
    crtc_type: Option<CrtcType>,
    /// If interruption are allowed
    enable_interruptions: bool, // false by default
    disable_lower_rom: bool, // false by default
    disable_upper_rom: bool, // false by defualt
}

impl MotherboardBuilder {
    pub fn set_crtc(&mut self, crtc_type: CrtcType) -> &mut Self {
        self.crtc_type = Some(crtc_type);
        self
    }

    pub fn enable_interruptions(&mut self) -> &mut Self {
        self.enable_interruptions = true;
        self
    }

    pub fn disable_lower_rom(&mut self) -> &mut Self {
        self.disable_lower_rom = true;
        self
    }

    pub fn disable_upper_rom(&mut self) -> &mut Self {
        self.disable_lower_rom = true;
        self
    }

    pub fn build(self) -> Motherboard {
        let crtc_type = self.crtc_type.unwrap_or_default();

        let mut motherboard = Motherboard {
            crtc: crtc_type.into(),
            ..Default::default()
        };
        if self.enable_interruptions {
            motherboard.cpu.iff1.set(1);
            motherboard.cpu.im = InterruptMode::Mode1;
        }
        if self.disable_lower_rom {
            motherboard.disable_lower_rom();
        }
        if self.disable_upper_rom {
            motherboard.disable_upper_rom();
        }
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

    pub fn ga(&self) -> &GateArray {
        &self.ga
    }

    pub fn ga_mut(&mut self) -> &mut GateArray {
        &mut self.ga
    }

    pub fn t_state_counter(&self) -> &Counter {
        &self.t_state_counter
    }

    pub fn nops_counter(&self) -> &Counter {
        &self.nops_counter
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
        // if self.crtc.is_activated() {dbg!(&self.cpu().next_state);}

        // fill bus with memory data if needed
        self.handle_memory_access();

        // Keep this order to avoid side effect mixing previous state and current state
        if self.crtc.is_activated() {
            self.crtc.tick(); // Virtually uses 16 cycles
        }
        if self.ppi.is_activated() {
            self.ppi.tick();
        }
        self.ga.tick();
        self.pal.tick();

        if self.cpu.is_activated() {
            self.cpu.tick(); // Virtually uses 4 cycles
            self.t_state_counter.inc();

            // By definition it is a new opcode because the previous one has been executed
            if self.t_state_counter.value() % 4 == 0 {
                self.nops_counter.inc();
            }
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
            //   eprintln!("Want at 0x{:x} with {:?}", addr, memory_cfg);
            let byte = self.memory.read(addr, &memory_cfg);
            //    eprintln!("Read 0x{:x} at 0x{:x} with {:?}", byte, addr, memory_cfg);
            self.cpu.set_data_on_pins(byte);
            self.cpu.set_pin(Z80Pin::Neg_RD, false); // TODO do the real stuff - but here I am sure to not play with the data bus ...
        } else if self.cpu_request_byte_write() && !self.ga_request_read() {
            let addr = self.cpu.get_address_from_pins();
            let data = self.cpu.get_data_from_pins();
            let memory_cfg = self.memory_config();
            //    println!("Write 0x{:x} at 0x{:x}", data, addr);
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
        assert!(
            self.cpu.next_state.is_new_opcode(),
            "We have to start by a new opcode"
        );

        let mut has_executed_first_state_counter = false;
        let mut has_finished_z80_execution = false;

        /// execute the whole stuff as soon as it is necessary
        loop {
            video.push(self.tick());

            has_executed_first_state_counter =
                has_executed_first_state_counter | !self.cpu.next_state.is_new_opcode();

            if has_executed_first_state_counter && self.cpu.is_downfront(Z80::clock_pin()) {
                has_finished_z80_execution = true;
            }

            if self.cpu.is_activated()
                && self.cpu.next_state.is_new_opcode()
                && has_executed_first_state_counter
                && has_finished_z80_execution
            {
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

        // Make some pins associattions
        let Neg_IOWR = self.cpu.get_pin(Z80Pin::Neg_WR) || self.cpu.get_pin(Z80Pin::Neg_IORQ);
        let Neg_IORD = self.cpu.get_pin(Z80Pin::Neg_RD) || self.cpu.get_pin(Z80Pin::Neg_IORQ);

        // GA to CPU
        sync_pins(
            &self.ga,
            &[GateArrayPin::NCPU, GateArrayPin::INTERRUPT],
            &mut self.cpu,
            &[Z80Pin::Neg_CLK, Z80Pin::Neg_INT],
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
            &[Z80Pin::A14, Z80Pin::A15, Z80Pin::Neg_IORQ, Z80Pin::Neg_RD],
            &mut self.ga,
            &[
                GateArrayPin::A14,
                GateArrayPin::A15,
                GateArrayPin::IORQ,
                GateArrayPin::RD,
            ],
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
        self.pal.set_pin(PalPin::IOWR, !Neg_IOWR);

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

        // CPU to ppi
        sync_pins(&self.cpu, Z80::pins_data(), &mut self.ppi, Ppi::pins_data());
        sync_pins(
            &self.cpu,
            &[Z80Pin::A11, Z80Pin::A9, Z80Pin::A8],
            &mut self.ppi,
            &[PpiPin::Neg_CS, PpiPin::A9, PpiPin::A8],
        );
        self.ppi.set_pin(PpiPin::Neg_RD, Neg_IORD);
        self.ppi.set_pin(PpiPin::Neg_WR, Neg_IOWR);
        // CRTC to ppi
        self.ppi.set_pin(PpiPin::B0, self.crtc.get_pin(CrtcPin::VS));
    }
}

// Block for disassembling related functions
impl Motherboard {
    /// TODO delegate that to another component
    pub fn disassemble_next_instruction(&self) -> DisassembledInstruction
    {
        let  mut pc= self.cpu.pc.value();
        let mut read = || {
            let byte = self.read_byte(pc);
            pc += 1;
            byte
        };
        
        let (prefix, ir) = {
            let current = read();
            let prefix = OpcodePrefix::from(current);
            if let OpcodePrefix::None = prefix {
                (prefix, current)
            } else {
               (prefix, read())
           }
        };

        let opcode_info = prefix.opcode_information(ir);
        let mut memory: SmallVec<[u8;3]> = if let OpcodePrefix::None = prefix{
            smallvec![ir]
        } else {
            smallvec![prefix as u8, ir]
        };
        
        for _i in 0..opcode_info.nb_additional_opcode_bytes  {
           memory.push(read());
        };

        let token = cpclib_asm::disass::disassemble_with_potential_argument(
            ir,
            &prefix.disassembling_table(),
            &memory[1..],
        )
        .unwrap().0;

        DisassembledInstruction{
             token, memory,
             address: self.cpu.pc.value()
        }
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

    /// Changes the rom configuration of the machine
    pub fn force_rom_config(&mut self, config: RomConfig) {
        self.ga.force_lower_rom_pagging(config.lower_rom);
        self.ga.force_upper_rom_pagging(config.upper_rom);
        self.rom_selector.force_upper_rom_number(config.rom_number);
    }

    pub fn disable_lower_rom(&mut self) {
        self.ga.force_lower_rom_pagging(Flag::Disable);
    }

    pub fn disable_upper_rom(&mut self) {
        self.ga.force_lower_rom_pagging(Flag::Disable);
    }

    /// Write the databuffer in memory according to the memory configuration
    pub fn write_buffer(&mut self, mut addr: u16, buffer: &[u8]) {
        let config = self.memory_config();
        for byte in buffer.iter() {
            self.memory.write(addr, *byte, &config);
            addr = addr.wrapping_add(1);
        }
    }

    /// Write one memory byte according to the memory configuration
    #[inline]
    pub fn write_byte(&mut self, addr: u16, byte: u8) {
        let memory_cfg = self.memory_config();
        self.memory.write(addr, byte, &memory_cfg);
    }

    pub fn write_word(&mut self, addr: u16, word: u16) {
        let memory_cfg = self.memory_config();
        let low = (word & 0xff) as u8;
        let high = ((word & 0xff00) >> 8) as u8;
        self.memory.write(addr, low, &memory_cfg);
        self.memory.write(addr.wrapping_add(1), high, &memory_cfg);
    }

    /// Read one memory byte according to the memory configuration
    #[inline]
    pub fn read_byte(&self, addr: u16) -> u8 {
        let memory_cfg = self.memory_config();
        self.memory.read(addr, &memory_cfg)
    }

    #[inline]
    pub fn read_word(&self, addr: u16) -> u16 {
        let memory_cfg = self.memory_config();

        let low = self.memory.read(addr, &memory_cfg) as u16;
        let high = self.memory.read(addr.wrapping_add(1), &memory_cfg) as u16;

        256 * high + low
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

        assert!(
            motherboard.cpu.next_state.is_new_opcode(),
            "We have to start by a new opcode"
        );
        assert!(motherboard.cpu.is_activated());

        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(
            video.len(),
            16 // 4 t-states for 4 mcycles
        );

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4001,
            "{:x} != {:x} ",
            motherboard.cpu.pc.value(),
            0x4001
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

        assert!(
            motherboard.cpu.next_state.is_new_opcode(),
            "We have to start by a new opcode"
        );
        assert!(motherboard.cpu.is_activated());

        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(
            video.len(),
            16 // 4 t-states for 4 mcycles
        );

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4001,
            "{:x} != {:x} ",
            motherboard.cpu.pc.value(),
            0x4001
        );

        assert_eq!(motherboard.cpu.af.high().value(), 0);
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

        assert!(
            motherboard.cpu.next_state.is_new_opcode(),
            "We have to start by a new opcode"
        );
        assert!(motherboard.cpu.is_activated());

        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(motherboard.cpu.hl.value(), 0xdead);

        assert_eq!(
            motherboard.cpu.pc.value(),
            0x4003,
            "{:x} != {:x} ",
            motherboard.cpu.pc.value(),
            0x4003
        );

        assert_eq!(video.len(), 4 * 3 * 4);
    }

    #[test]
    fn test_chips_activation() {
        let mut motherboard = Motherboard::default();

        // setup pins
        motherboard.cpu.set_pin(Z80Pin::Neg_WAIT, true); // delegate that to GA

        let mut count_cpu = 0;
        let mut count_crtc = 0;
        for i in 0..32 {
            println!(
                "{},{},{}",
                true,
                motherboard.cpu.is_activated(),
                motherboard.crtc.is_activated()
            );

            if motherboard.cpu.is_activated() {
                count_cpu += 1;
            }

            if motherboard.crtc.is_activated() {
                count_crtc += 1;
            }

            motherboard.ga.manage_clocks();
            motherboard.sync_chips_pins();
        }

        assert_eq!(count_cpu, 32 / 4);

        assert_eq!(count_crtc, 32 / 16);
    }
}
