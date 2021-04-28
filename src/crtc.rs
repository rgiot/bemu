use crate::chip::*;

use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;

use flagset::{FlagSet, flags};


flags! {
#[allow(non_camel_case_types)]
#[repr(u64)]
pub enum CrtcPin : u64 {
D0,
D1,
D2,
D3,
D4,
D5,
D6,
D7

}
}

impl Into<u64> for CrtcPin {
    fn into(self) -> u64 {
        self as u64
    }
}

const CRTC_PIN_DATA_BUS : [CrtcPin;8] = [CrtcPin::D7, CrtcPin::D6, CrtcPin::D5, CrtcPin::D4, CrtcPin::D3, CrtcPin::D2, CrtcPin::D1, CrtcPin::D0];


#[derive(Default)]
pub struct Crtc {
    pins: u64
}

impl Chip for Crtc {
    type PinType = CrtcPin;

    fn pins_buffer_mut(&mut self) -> &mut u64 {
        &mut self.pins
    }
    fn pins_buffer(&self) -> & u64 {
        & self.pins
    }


    fn must_execute_tick(&self) -> bool {
        unimplemented!();
    }
    fn execute_tick(&mut self) {
        unimplemented!();
    }

    

    fn reset(&mut self) {
        unimplemented!();
    }
}

impl DataChip for Crtc {
    fn pins_data() -> &'static [Self::PinType; 8] {
        return &CRTC_PIN_DATA_BUS;
    }
}
impl ReadDataChip for Crtc {}