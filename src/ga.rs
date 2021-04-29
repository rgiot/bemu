use crate::chip::*;

use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;

use flagset::{FlagSet, flags};

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum GateArrayPin {
    /// Specify if the pin is for CPU or CRTC. 1MHZ signal
    CPU_ADDR = 1<<0,
    /// Request the CPU to wait
    READY = 1<<1,
    /// Validate high byte of address
    CAS = 1<<2,
    /// Activate data from cpu to ga
    TWO_FOR_FOR_EN = 1<<3,
    /// Request write in RAM
    MWE = 1<<4,
    /// ????
    CAS_ADDR = 1<<5,
    /// Adress is line address
    RAS = 1<<6,
    /// 16 Mhz signal
    XTAL = 1<<7,
    VCC2 = 1<<8,
    /// Request the CPU to interrupt
    INTERRUPT = 1<<9,
    /// Synchronization for screen
    SYNC = 1<<10,
    /// Access to ROM
    ROMEN = 1<<11,
    /// Request data send to CPU
    RAMRD = 1<<12,
    /// Horizontal synchronisation from CRTC
    HSYNC = 1<<13,
    /// Vertical synchronisation from CRTC
    VSYNC = 1<<14,
    /// The CPU request an input/output (ignored if does not match)
    IORQ = 1<<15,
    /// CPU received an interruption signal
    M1 = 1<<16,
    /// CPU wants to access the memory
    MERQ = 1<<17,
    /// CPU wants to read the memory
    RD = 1<<18,

    A15 = 1<<19,

    /// 1 Mhz for RAM refresh
    MAO_CCLK = 1<<20,

    /// 4MHZ for CPU
    PHI = 1<<21,
    VCC1 = 1<<22,

    /// Reset of the GA
    RESET = 1<<23,

    /// Red component of the video signal
    R = 1<<24,
    GND = 1<<25,
    /// Green component of the video signal
    G = 1<<26,
    VCC3 = 1<<27,
    /// Blue component of the video signal
    B = 1<<28,

    D7 = 1<<29,
    D6 = 1<<30,
    D5 = 1<<31,
    D4 = 1<<32,
    D3 = 1<<33,
    D2 = 1<<34,
    D1 = 1<<35,
    D0 = 1<<36,

    /// CRTC request a display
    DISPEN = 1<<37,
    VCC4 = 1<<38,
    A14 = 1<<39
}


impl Into<u64> for GateArrayPin {
    fn into(self) -> u64 {
        self as u64
    }
}

impl Into<u64> for &GateArrayPin {
    fn into(self) -> u64 {
        *self as u64
    }
}
const GA_PIN_DATA_BUS : [GateArrayPin;8] = [GateArrayPin::D7, GateArrayPin::D6, GateArrayPin::D5, GateArrayPin::D4, GateArrayPin::D3, GateArrayPin::D2, GateArrayPin::D1, GateArrayPin::D0];

#[derive(Default)]
pub struct GateArray {
    clock: u64,
    pins: u64
}

impl DataChip for GateArray {

    fn pins_data() -> &'static [Self::PinType; 8] {
        return &GA_PIN_DATA_BUS;
    }
}

impl GateArray {
    /// Gate array works at 16Mhz and divide it to generate signals for other chips.
    /// CPU is at 4 Mhz
    pub fn manage_clocks(&mut self) {
        self.set_pin(GateArrayPin::CPU_ADDR, self.clock % 4 != 0 );

        self.clock += 1;
    }
}


impl Chip for GateArray {
    type PinType = GateArrayPin;

    fn must_execute_tick(&self) -> bool {
        !self.get_pin(GateArrayPin::A15) && self.get_pin(GateArrayPin::A14)
    }

    fn execute_tick(&mut self) {
        unimplemented!();
    }

    
    
    fn pins_buffer_mut(&mut self) -> &mut u64 {
        &mut self.pins
    }
    fn pins_buffer(&self) -> & u64 {
        & self.pins
    }


    fn reset(&mut self) {
        self.clock = 0;
    }
}


#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn run_ga() {
        let mut ga = GateArray::default();
        ga.reset();

        ga.tick();
        ga.tick();
    }
}