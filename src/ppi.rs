use crate::chip::*;

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum PpiPin {
    // Input
    D0 = 1<<0,
    D1 = 1<<1,
    D2 = 1<<2,
    D3 = 1<<3,
    D4 = 1<<4,
    D5 = 1<<5,
    D6 = 1<<6,
    D7 = 1<<7,

    A11 = 1<<8,
    A9 = 1<<9,
    A8 = 1<<10,
    Neg_IORD = 1<<11,
    Neg_IOWR = 1<<12,


    B0 = 1<<13, // VSYNC
    B1 = 1<<14,
    B2 = 1<<15,
    B3 = 1<<16,
    B4 = 1<<17,
    B5 = 1<<18,
    B6 = 1<<19,
    B7 = 1<<20,


    // Output
    C0 = 1<<30,
    C1 = 1<<31,
    C2 = 1<<32,
    C3 = 1<<33,
    C4 = 1<<34,
    C5 = 1<<35,
    C6 = 1<<36,
    C7 = 1<<37,

    A0 = 1<<40,
    A1 = 1<<41,
    A2 = 1<<42,
    A3 = 1<<43,
    A4 = 1<<44,
    A5 = 1<<45,
    A6 = 1<<46,
    A7 = 1<<47,
}

impl Into<u64> for PpiPin {
    fn into(self) -> u64 {
        self as u64
    }
}

impl Into<u64> for &PpiPin {
    fn into(self) -> u64 {
        *self as u64
    }
}

const PPI_PIN_DATA_BUS: [PpiPin; 8] = [
    PpiPin::D7,
    PpiPin::D6,
    PpiPin::D5,
    PpiPin::D4,
    PpiPin::D3,
    PpiPin::D2,
    PpiPin::D1,
    PpiPin::D0,
];



struct Ppi {
    pins: PinsBuffer
}

impl Ppi {
    fn is_port_a_addressed(&self) -> bool {
        !self.get_pin(PpiPin::A9) && !self.get_pin(PpiPin::A8)
    }

    fn is_port_b_addressed(&self) -> bool {
        !self.get_pin(PpiPin::A9) && self.get_pin(PpiPin::A8)
    }

    fn is_port_c_addressed(&self) -> bool {
        self.get_pin(PpiPin::A9) && !self.get_pin(PpiPin::A8)
    }

    fn is_control_addressed(&self) -> bool {
        self.get_pin(PpiPin::A9) && self.get_pin(PpiPin::A8)
    }
}

impl Chip for Ppi {
    type PinType = PpiPin;

    fn must_execute_tick(&self) -> bool {
        !self.get_pin(PpiPin::A11)
    }


    
    /// The buffer that encodes the current state of the pin (previous, current) (writeable)
    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }
    /// The buffer that encodes the current state of the pin (previous, current) (readable)
    fn pins_buffer(&self) -> &PinsBuffer {
        & self.pins
    }

    /// Reset pins and counter
    fn reset(&mut self) {
        self.pins = Default::default();
    }

}