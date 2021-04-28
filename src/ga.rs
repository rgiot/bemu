use crate::chip::*;

use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;

use flagset::{FlagSet, flags};

flags! {
    #[allow(non_camel_case_types)]
    #[repr(u64)]
pub enum GateArrayPin : u64 {
    /// Specify if the pin is for CPU or CRTC. 1MHZ signal
    CPU_ADDR,
    /// Request the CPU to wait
    READY,
    /// Validate high byte of address
    CAS,
    /// Activate data from cpu to ga
    TWO_FOR_FOR_EN,
    /// Request write in RAM
    MWE,
    /// ????
    CAS_ADDR,
    /// Adress is line address
    RAS,
    /// 16 Mhz signal
    XTAL,
    VCC2,
    /// Request the CPU to interrupt
    INTERRUPT,
    /// Synchronization for screen
    SYNC,
    /// Access to ROM
    ROMEN,
    /// Request data send to CPU
    RAMRD,
    /// Horizontal synchronisation from CRTC
    HSYNC,
    /// Vertical synchronisation from CRTC
    VSYNC,
    /// The CPU request an input/output (ignored if does not match)
    IORQ,
    /// CPU received an interruption signal
    M1,
    /// CPU wants to access the memory
    MERQ,
    /// CPU wants to read the memory
    RD,

    A15,

    /// 1 Mhz for RAM refresh
    MAO_CCLK,

    /// 4MHZ for CPU
    PHI,
    VCC1,

    /// Reset of the GA
    RESET,

    /// Red component of the video signal
    R,
    GND,
    /// Green component of the video signal
    G,
    VCC3,
    /// Blue component of the video signal
    B,

    D7,
    D6,
    D5,
    D4,
    D3,
    D2,
    D1,
    D0,

    /// CRTC request a display
    DISPEN,
    VCC4,
    A14
}
}

impl Into<u64> for GateArrayPin {
    fn into(self) -> u64 {
        self as u64
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