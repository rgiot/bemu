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
#[derive(Default)]
struct Motherboard {
    cpu: Z80,
    crtc: Crtc,
    ga: GateArray,
    pal: Pal,
    rom_selector: UpperRomSelector,

    memory: Memory,
}

impl Motherboard {
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
        // Divides and distribute the 16Mhz clock
        self.ga.manage_clocks();

        /// fill bus with memory data if needed
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

        self.sync_chips_pins();

        ga_output
    }


    /// manage the memory accesses (read/write cpu ; read ga)
    pub fn handle_memory_access(&mut self) {
        unimplemented!()
    }

    /// Completly execute an opcode and returns the set of video information associated
    pub fn execute_next_opcode(&mut self) -> Vec<(RgbLevels, Sync)> {
        let mut video = Vec::new();
        assert!(self.cpu.next_state.is_new_opcode(), "We have to start by a new opcode");

        /// execute the whole stuff as soon as it is necessary
        loop {
            video.push(self.tick());
            if self.cpu.next_state.is_new_opcode() {
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
            self.cpu.get_pin(Z80Pin::Neg_WR) || self.cpu.get_pin(Z80Pin::Neg_IORQ),
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
    fn test_chips_activation() {
        unimplemented!();
    }


    #[test]
    fn test_nop_execution() {
        let mut motherboard = motherboard::default();
        let memory_config = motherboard.memory_config();
        
        // install the program
        motherboard.cpu.reg16_mut(Register16::PC).set(0x4000);
        motherboard.memory.write(0x4000, 0x00, &memory_config);

        // execute it
        let video = motherboard.execute_next_opcode();

        assert_eq!(
            motherboard.cpu.reg16(Register16::Pc).value(),
            0x40001
        );

        assert_eq!(
            video.len(),
            4
        );
    }
}
