use crate::chip::*;


use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;

use flagset::{FlagSet, flags};

/*
#[allow(non_camel_case_types)]
#[derive(Eq, PartialEq, TryFromPrimitive)]
#[derive(IntoPrimitive)]
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum PalPin {

    /// Serves to detect a memory commutation order
    D6_D7,
    /// Ram configuration
    D0,
    /// Reset at start
    RESET,
    /// Disable motherboard RAM
    RAMDIS,

    CAS,
    A15,
    A14,

    GND,
    CPU,


    /// Ram configuration
    D1,
    /// Ram configuration
    D2,

    A15_OUT,
    CAS1,
    CAS0,

    IOWR,

    A14_OUT,
    VDD
}
*/

flags! {
#[allow(non_camel_case_types)]
#[repr(u64)]
pub enum PalPin: u64 {

    /// Serves to detect a memory commutation order
    D6_D7,
    /// Ram configuration
    D0,
    /// Reset at start
    RESET,
    /// Disable motherboard RAM
    RAMDIS,

    CAS,
    A15,
    A14,

    GND,
    CPU,


    /// Ram configuration
    D1,
    /// Ram configuration
    D2,

    A15_OUT,
    CAS1,
    CAS0,

    IOWR,

    A14_OUT,
    VDD
}
}

impl Into<u64> for PalPin {
    fn into(self) -> u64 {
        self as u64
    }
}


pub struct Pal {
    pins: u64
}

impl Default for Pal {
    fn default() -> Self {
        Pal {
            pins: 0
        }
    }
}

impl Chip for Pal {
    type PinType = PalPin;

    
    fn pins_buffer_mut(&mut self) -> &mut u64 {
        &mut self.pins
    }
    fn pins_buffer(&self) -> & u64 {
        & self.pins
    }


    fn must_execute_tick(&self) -> bool {
        !self.get_pin(PalPin::A15) && self.get_pin(PalPin::A14) && self.get_pin(PalPin::IOWR)
    }

    fn execute_tick(&mut self) {
        unimplemented!();
    }


    fn reset(&mut self) {
        unimplemented!();
    }
}


#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_activation() {
        let mut pal = Pal::default();
    }
}