use cpclib_asm::preamble::Mnemonic;
use cpclib_asm::preamble::Token;
use smallvec::*;

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
        self.disable_upper_rom = true;
        self
    }

    pub fn build(self) -> Motherboard {
        let crtc_type = self.crtc_type.unwrap_or_default();

        let mut motherboard = Motherboard {
            crtc: crtc_type.into(),
            ..Default::default()
        };

        if self.disable_lower_rom {
            motherboard.disable_lower_rom();
        }
        if self.disable_upper_rom {
            motherboard.disable_upper_rom();
        }
        motherboard.stabilize_run();

        if self.enable_interruptions {
            motherboard.cpu.iff1 = true ;
            motherboard.cpu.iff2 = true ;
            motherboard.cpu.im = InterruptMode::Mode1;
        }
        
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

    pub fn crtc(&self) -> &Crtc {
        &self.crtc
    }

    pub fn crtc_mut(&mut self) -> &mut Crtc {
        &mut self.crtc
    }

    pub fn ga(&self) -> &GateArray {
        &self.ga
    }

    pub fn ga_mut(&mut self) -> &mut GateArray {
        &mut self.ga
    }

    pub fn ppi(&self) -> &Ppi {
        &self.ppi
    }

    pub fn ppi_mut(&mut self) -> &mut Ppi {
        &mut self.ppi
    }

    pub fn t_state_counter(&self) -> &Counter {
        &self.t_state_counter
    }

    pub fn nops_counter(&self) -> &Counter {
        &self.nops_counter
    }

    pub fn nops_counter_mut(&mut self) -> &mut Counter {
        &mut self.nops_counter
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

        for _ in 0..2 {
            self.sync_chips_pins();
        }
    }

    /// Execute one tick on all the components of the motherboard and returns the
    /// corresponding gate array information
    /// TODO check if it must be generated before or after
    pub fn tick(&mut self) -> (RgbLevels, Sync) {
        // if self.crtc.is_activated() {dbg!(&self.cpu().next_state);}

        self.handle_write_access();

        // Keep this order to avoid side effect mixing previous state and current state
        if self.crtc.is_activated() {
        //    println!("*");
            self.crtc.tick(); // Virtually uses 16 cycles
        } else {
       //     println!(".");
        }
        if self.ppi.is_activated() {
            self.ppi.tick();
        }
        self.ga.tick();
        self.pal.tick();

        self.handle_read_access();

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

    // Manage the operations where CPU requests a write
    pub fn handle_write_access(&mut self) {
        let addr = self.cpu.get_address_from_pins();
        let byte = self.cpu.get_data_from_pins();

        if self.ga_request_read() {
            unimplemented!();       
        }  else if self.cpu_request_byte_write() {
        //    eprintln!("0x{:x}=0x{:x}", addr, byte);
            self.write_byte(addr, byte);
            self.cpu.set_pin(Z80Pin::Neg_WR, true); // TODO do the real stuff - but here I am sure to not play with the data bus ...
        } else if self.cpu_request_byte_output() {
            // there is nothing to do there as the pin syncing method already did everything
        }
    }

    /// Manage the operations where CPU requests a read
    pub fn handle_read_access(&mut self) {
        let addr = self.cpu.get_address_from_pins();

        if self.ga_request_read() {
            unimplemented!();
        
        } else  if self.cpu_request_byte_read() {
            let byte = self.read_byte(addr);
            self.cpu.set_data_on_pins(byte);
            self.cpu.set_pin(Z80Pin::Neg_RD, true); // TODO do the real stuff - but here I am sure to not play with the data bus ...
        }else if self.cpu_request_byte_input() {
            let byte = self.input_byte();
            self.cpu.set_data_on_pins(byte);
            self.cpu.set_pin(Z80Pin::Neg_RD, true);
        }
    }

    #[inline]
    fn cpu_request_byte_read(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_RD) && !self.cpu.get_pin(Z80Pin::Neg_MREQ)
    }

    #[inline]
    fn cpu_request_byte_write(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_WR)&& !self.cpu.get_pin(Z80Pin::Neg_MREQ)
    }

    #[inline]
    fn cpu_request_byte_input(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_RD) && !self.cpu.get_pin(Z80Pin::Neg_IORQ)
    }

    #[inline]
    fn cpu_request_byte_output(&self) -> bool {
        !self.cpu.get_pin(Z80Pin::Neg_WR)&& !self.cpu.get_pin(Z80Pin::Neg_IORQ)
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

    pub fn execute_ticks(&mut self, ticks: usize) -> Vec<(RgbLevels, Sync)> {
        let mut video = Vec::new();

        loop {
            video.append(&mut self.execute_next_opcode());
            if video.len() >= ticks {
                break;
            }
        }
        video
    }

    /// Make the appropriate copy between some pins of some chips to other (we simulate the various buses here)
    /// TODO improve behavior by handling BUSACK BUSREQ
    /// XXX as z80 timings are WRONG at them moment, it may change the signals too fast. As a workaround copy are made conditional for some chips
    fn sync_chips_pins(&mut self) {
        // Backup previous state
        self.ga.save_pins_state();
        self.pal.save_pins_state();
        self.crtc.save_pins_state();
        self.cpu.save_pins_state();

        // Make some pins associattions
        let Neg_IOWR = self.cpu.get_pin(Z80Pin::Neg_WR) || self.cpu.get_pin(Z80Pin::Neg_IORQ);
        let Neg_IORD = self.cpu.get_pin(Z80Pin::Neg_RD) || self.cpu.get_pin(Z80Pin::Neg_IORQ);

        if !Neg_IORD {
            assert!(!self.cpu.get_pin(Z80Pin::Neg_IORQ));
        }

        let chip_access = !Neg_IOWR | !Neg_IORD;

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
            &[
                Z80Pin::A14, 
                Z80Pin::A15, 
                Z80Pin::Neg_IORQ, 
                Z80Pin::Neg_RD,
                Z80Pin::Neg_M1,
                Z80Pin::Neg_MREQ, ],
            &mut self.ga,
            &[
                GateArrayPin::A14,
                GateArrayPin::A15,
                GateArrayPin::Neg_IORQ,
                GateArrayPin::Neg_RD,
                GateArrayPin::Neg_M1,
                GateArrayPin::Neg_MREQ
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
            (Neg_IORD & Neg_IOWR),
        );

        sync_pins(
            &self.cpu,
            &[Z80Pin::A8, Z80Pin::A9, Z80Pin::A14],
            &mut self.crtc,
            &[CrtcPin::RS, CrtcPin::RW, CrtcPin::CS],
        );
        sync_pins(
            &self.cpu,
            Z80::pins_data(),
            &mut self.crtc,
            Crtc::pins_data(),
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
        if !Neg_IOWR {
            sync_pins(&self.cpu, Z80::pins_data(), &mut self.ppi, Ppi::pins_data());
        }
        sync_pins(
            &self.cpu,
            &[Z80Pin::A11, Z80Pin::A9, Z80Pin::A8],
            &mut self.ppi,
            &[PpiPin::Neg_CS, PpiPin::InA1, PpiPin::InA0],
        );
        self.ppi.set_pin(PpiPin::Neg_RD, Neg_IORD);
        self.ppi.set_pin(PpiPin::Neg_WR, Neg_IOWR);
        // CRTC to ppi
        self.ppi.set_pin(PpiPin::B0, self.crtc.get_pin(CrtcPin::VS));

        // Sync CRTC to gate array
        sync_pins(
            &self.crtc,
            &[CrtcPin::VS, CrtcPin::HS, CrtcPin::DE],
            &mut self.ga,
            &[
                GateArrayPin::VSYNC,
                GateArrayPin::HSYNC,
                GateArrayPin::DISPEN,
            ],
        );
    }
}

// Block for disassembling related functions
impl Motherboard {
    /// Return the opcode that will be executed
    pub fn disassemble_next_instruction(&self) -> DisassembledInstruction {
        self.disassemble_instruction_at(self.cpu.pc.value())
    }

    /// Disassemble a bunch of instruction given an address
    pub fn disassemble_instructions_from(&self, mut pc: u16, count: usize) -> Vec<DisassembledInstruction> {
        let mut res = Vec::new();
        for _ in 0..count {
            let inst = self.disassemble_instruction_at(pc);
            pc = pc.wrapping_add(inst.len() as u16);
            res.push(inst);
        }
        res
    }

    /// Disassemble the instruction at the given address
    pub fn disassemble_instruction_at(&self, mut pc: u16) -> DisassembledInstruction {
        let address = pc;

        let mut read = || {
            let byte = self.read_byte(pc);
            pc = pc.wrapping_add(1);
            byte
        };

        let mut current_prefix = OpcodePrefix::None;
        let mut instruction_bytes : SmallVec <[u8;4]> = Default::default();
        let mut ir = 0;
        loop {
            ir = read();
            instruction_bytes.push(ir);
            match current_prefix {
                OpcodePrefix::None => {
                    current_prefix = match ir {
                        0xdd => OpcodePrefix::Dd,
                        0xfd => OpcodePrefix::Fd,
                        0xcb => OpcodePrefix::Cb,
                        0xed => OpcodePrefix::Ed,
                        _ => OpcodePrefix::None
                    };
                    
                    if current_prefix == OpcodePrefix::None {
                        break;
                    }
                },

                OpcodePrefix::Cb | OpcodePrefix::Ed => {
                    break;
                }

                OpcodePrefix::Dd => {
                    if ir == 0xcb {
                        current_prefix = OpcodePrefix::DdCb
                    } else {
                        break;
                    }
                },
                
                OpcodePrefix::Fd => {
                    if ir == 0xcb {
                        current_prefix = OpcodePrefix::FdCb
                    } else {
                        break;
                    }
                },

                OpcodePrefix::DdCb |  OpcodePrefix::FdCb => {
                   break;
                }
            }
        }

        /// Read the extra byte
        let nb_bytes = if matches!(current_prefix, OpcodePrefix::DdCb |  OpcodePrefix::FdCb) {1} else {current_prefix.opcode_information(ir).nb_additional_opcode_bytes};
        for _i in 0..nb_bytes {
            instruction_bytes.push(read());
        }

        let token = cpclib_asm::disass::disassemble(&instruction_bytes)
        .get(0)
        .cloned()
        .unwrap_or(Token::OpCode(Mnemonic::Nop, None, None, None));
        

        DisassembledInstruction {
            token,
            memory: instruction_bytes,
            address,
        }
    }

    /// Execute the right amount of ticks to be sure next CPU tick will be in the start of an instruction
    pub fn tick_until_next_instruction(&mut self) {
        while !self.is_aligned_with_new_instruction() {
            self.tick();
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
        self.ga.force_upper_rom_pagging(Flag::Disable);
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

impl Motherboard {
    /// Return the byte read from a chip
    /// TODO handle the case where there is a read conflict over several chips
    fn input_byte(&self) -> u8 {
        if self.ppi().must_execute_tick() {
            self.ppi().get_data_from_pins()
        } else if self.ga().must_execute_tick() {
            self.ga().get_data_from_pins()
        } else if self.crtc().must_execute_tick() {
            self.crtc().get_data_from_pins()
        } else {
            0xff
        }
    }
}

impl Motherboard {
    delegate::delegate! {
        to self.cpu {
            pub fn pc(&self) -> u16;
            pub fn sp(&self) -> u16;
            pub fn af(&self) -> u16;
            pub fn bc(&self) -> u16;
            pub fn de(&self) -> u16;
            pub fn hl(&self) -> u16;
            pub fn ix(&self) -> u16;
            pub fn iy(&self) -> u16;


    pub fn a(&self) -> u8;
    pub fn b(&self) -> u8;
    pub fn d(&self) -> u8;
    pub fn h(&self) -> u8;
    pub fn ixh(&self) -> u8;
    pub fn iyh(&self) -> u8;
    pub fn c(&self) -> u8;
    pub fn e(&self) -> u8;
    pub fn l(&self) -> u8;
    pub fn x(&self) -> u8;
    pub fn y(&self) -> u8;

            pub fn set_pc(&mut self, value: u16);
            pub fn set_sp(&mut self, value: u16);
            pub fn set_af(&mut self, value: u16);
            pub fn set_bc(&mut self, value: u16);
            pub fn set_de(&mut self, value: u16);
            pub fn set_hl(&mut self, value: u16);
            pub fn set_ix(&mut self, value: u16);
            pub fn set_iy(&mut self, value: u16);

            pub fn is_aligned_with_new_instruction(&self) -> bool;
        }
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
    fn test_rom_config() {
        let mut motherboard = Motherboard::default();

        let memory_config = motherboard.memory_config();
        assert_eq!(memory_config.0.lower_rom, Flag::Enable);
        assert_eq!(memory_config.0.upper_rom, Flag::Enable);

        motherboard.disable_lower_rom();
        let memory_config = motherboard.memory_config();
        assert_eq!(memory_config.0.lower_rom, Flag::Disable);
        assert_eq!(memory_config.0.upper_rom, Flag::Enable);

        motherboard.disable_upper_rom();
        let memory_config = motherboard.memory_config();
        assert_eq!(memory_config.0.lower_rom, Flag::Disable);
        assert_eq!(memory_config.0.upper_rom, Flag::Disable);
    }

    #[test]
    fn test_motherboard_builder() {
        let motherboard = MotherboardBuilder::default()
            .disable_lower_rom()
            .disable_upper_rom()
            .build();
        let memory_config = motherboard.memory_config();
        assert_eq!(memory_config.0.lower_rom, Flag::Disable);
        assert_eq!(memory_config.0.upper_rom, Flag::Disable);
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



    #[test]
    fn motherboard_execute_step_nop() {
        let mut motherboard = MotherboardBuilder::default()
                                .disable_lower_rom()
                                .disable_upper_rom()
                                .build();

        // full of nop, no need to inject
        motherboard.set_pc(0x1000);
        motherboard.execute_next_opcode(); // ensure state is correct (it seems emu ini is wrong)

        motherboard.nops_counter_mut().reset();
        motherboard.set_pc(0x1000);
        let res = motherboard.execute_next_opcode();
        assert_eq!(
            motherboard.pc(),
            0x1001
        );
        assert_eq!(
            res.len(),
            16
        );
        assert_eq!(
            motherboard.nops_counter().value(),
            1
        );
    }

    #[test]
    fn motherboard_execute_step_halt_no_inter() {
        let mut motherboard = MotherboardBuilder::default()
                                .disable_lower_rom()
                                .disable_upper_rom()
                                .build();

         motherboard.nops_counter_mut().reset();
        motherboard.set_pc(0x1000);
        motherboard.cpu_mut().force_interruption_received(true);


        motherboard.write_byte(0x1000, 0x76);
        let res = motherboard.execute_next_opcode();
        assert_eq!(
            motherboard.pc(),
            0x1000
        );
        assert_eq!(
            res.len(),
            16
        );
        assert_eq!(
            motherboard.nops_counter().value(),
            1
        );
    }

    #[test]
    fn motherboard_execute_step_halt() {
        let mut motherboard = MotherboardBuilder::default()
                                .disable_lower_rom()
                                .disable_upper_rom()
                                .enable_interruptions()
                                .build();


        motherboard.nops_counter_mut().reset();
        motherboard.set_pc(0x1000);
        motherboard.cpu_mut().force_interruption_received(true);
        motherboard.cpu_mut().iff1 = true;
        motherboard.cpu_mut().iff2 = true;

        motherboard.write_byte(0x1000, 0x76);
        let res = motherboard.execute_next_opcode();
        dbg!(&motherboard.cpu.next_state);
        assert_eq!(
            motherboard.pc(),
            0x38
        );
        assert_eq!(
            motherboard.nops_counter().value(),
            1 + 3
        );
    }

}
