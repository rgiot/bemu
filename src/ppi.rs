use num_enum::UnsafeFromPrimitive;

use crate::chip::*;

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum PpiPin {
    // Input
    D0 = 1 << 0,
    D1 = 1 << 1,
    D2 = 1 << 2,
    D3 = 1 << 3,
    D4 = 1 << 4,
    D5 = 1 << 5,
    D6 = 1 << 6,
    D7 = 1 << 7,

    Neg_CS = 1 << 8,
    InA1 = 1 << 9,
    InA0 = 1 << 10,
    Neg_RD = 1 << 11,
    Neg_WR = 1 << 12,

    B0 = 1 << 13, // VSYNC
    B1 = 1 << 14,
    B2 = 1 << 15,
    B3 = 1 << 16,
    B4 = 1 << 17,
    B5 = 1 << 18,
    B6 = 1 << 19,
    B7 = 1 << 20,

    // Output
    C0 = 1 << 30,
    C1 = 1 << 31,
    C2 = 1 << 32,
    C3 = 1 << 33,
    C4 = 1 << 34,
    C5 = 1 << 35,
    C6 = 1 << 36,
    C7 = 1 << 37,

    A0 = 1 << 40,
    A1 = 1 << 41,
    A2 = 1 << 42,
    A3 = 1 << 43,
    A4 = 1 << 44,
    A5 = 1 << 45,
    A6 = 1 << 46,
    A7 = 1 << 47,
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

const PPI_PIN_DATA_A_BUS: [PpiPin; 8] = [
    PpiPin::A7,
    PpiPin::A6,
    PpiPin::A5,
    PpiPin::A4,
    PpiPin::A3,
    PpiPin::A2,
    PpiPin::A1,
    PpiPin::A0,
];

const PPI_PIN_DATA_B_BUS: [PpiPin; 8] = [
    PpiPin::B7,
    PpiPin::B6,
    PpiPin::B5,
    PpiPin::B4,
    PpiPin::B3,
    PpiPin::B2,
    PpiPin::B1,
    PpiPin::B0, // vsync
];

const PPI_BRAND_PIN: [PpiPin; 3] = [PpiPin::B3, PpiPin::B2, PpiPin::B1];

const PPI_PIN_DATA_C_BUS: [PpiPin; 8] = [
    PpiPin::C7,
    PpiPin::C6,
    PpiPin::C5,
    PpiPin::C4,
    PpiPin::C3,
    PpiPin::C2,
    PpiPin::C1,
    PpiPin::C0,
];

#[derive(PartialEq, Clone, Copy)]
pub enum PpiMode {
    Mode0,
    Mode1,
    Mode2,
}

impl Default for PpiMode {
    fn default() -> Self {
        PpiMode::Mode0
    }
}

#[derive(PartialEq)]
pub enum PpiDirection {
    Input,
    Output,
}

impl Default for PpiDirection {
    fn default() -> Self {
        PpiDirection::Output
    }
}
/// The brand displayed by firmware
#[repr(u8)]
#[derive(Copy, Clone)]
pub enum Brand {
    Lsp = 0,
    Triumph = 1,
    Saisho = 2,
    Solavox = 3,
    Awa = 4,
    Schneider = 5,
    Orion = 6,
    Amstrad = 7,
}

impl Default for Brand {
    fn default() -> Self {
        Brand::Amstrad
    }
}
#[derive(Copy, Clone)]
#[repr(u8)]
pub enum ScreenRefresh {
    FiftyHz = 1,
    SixtyHz = 0,
}

impl Default for ScreenRefresh {
    fn default() -> Self {
        ScreenRefresh::FiftyHz
    }
}

pub struct Ppi {
    pins: PinsBuffer,
    brand: Brand,
    refresh: ScreenRefresh,

    mode_a: PpiMode,
    mode_b: PpiMode,
    mode_c_lower: PpiMode,
    mode_c_upper: PpiMode,

    direction_a: PpiDirection,
    direction_b: PpiDirection,
    direction_c_lower: PpiDirection,
    direction_c_upper: PpiDirection,

    /// A mode-set control word clears the output latches of a real 8255.
    /// The PPI inside the CPC+'s ASIC does not (it keeps port C, which the
    /// CRTC detection programs use to recognise such a machine).
    clears_ports_on_mode_set: bool,
}

impl Default for Ppi {
    fn default() -> Self {
        let mut ppi = Self {
            pins: Default::default(),
            brand: Brand::default(),
            refresh: Default::default(),

            mode_a: Default::default(),
            mode_b: Default::default(),
            mode_c_lower: Default::default(),
            mode_c_upper: Default::default(),

            direction_a: Default::default(),
            direction_b: PpiDirection::Input, // no idea if it is the best choice, but it allows to directly test vsync
            direction_c_lower: Default::default(),
            direction_c_upper: Default::default(),
            clears_ports_on_mode_set: true,
        };
        ppi.reset();
        ppi
    }
}

impl Ppi {
    pub fn is_port_a_addressed(&self) -> bool {
        !self.get_pin(PpiPin::InA1) && !self.get_pin(PpiPin::InA0)
    }

    pub fn is_port_b_addressed(&self) -> bool {
        !self.get_pin(PpiPin::InA1) && self.get_pin(PpiPin::InA0)
    }

    pub fn is_port_c_addressed(&self) -> bool {
        self.get_pin(PpiPin::InA1) && !self.get_pin(PpiPin::InA0)
    }

    pub fn is_control_addressed(&self) -> bool {
        self.get_pin(PpiPin::InA1) && self.get_pin(PpiPin::InA0)
    }

    /// Whether a mode-set control word clears the ports (a real 8255) or
    /// leaves them alone (the PPI clone in the CPC+'s ASIC).
    pub fn set_clears_ports_on_mode_set(&mut self, clears: bool) {
        self.clears_ports_on_mode_set = clears;
    }

    /// Set which manufacturer's name the firmware reads back from port B
    /// bits 1-3 (and therefore which one appears in the boot banner).
    pub fn set_brand(&mut self, brand: Brand) {
        self.brand = brand;
        self.drive_hardwired_port_b_bits();
    }

    /// (Re-)assert the port B bits that are hardwired to external signals
    /// rather than driven by the CPU: B1-B3 (manufacturer jumpers) and B4
    /// (screen refresh rate strap). Must be called after anything that
    /// clears port B's pins, since those pins are never CPU outputs here.
    fn drive_hardwired_port_b_bits(&mut self) {
        self.set_value_in_pins(self.brand as u8, &PPI_BRAND_PIN);
        self.set_pin(PpiPin::B4, self.refresh as u8 != 0);
    }

    /// Drive port A's pins from an external source (the PSG, whose data bus
    /// *is* port A on real hardware - see `psg.rs`) rather than from the
    /// CPU. `Motherboard` calls this right after a PSG read bus cycle so
    /// the byte is already sitting on port A by the time the CPU's `IN
    /// A,(C)` reaches `execute_tick()`.
    pub fn set_port_a_input(&mut self, value: u8) {
        self.set_data_a_on_pins(value);
    }
}

impl Ppi {
    fn set_data_a_on_pins(&mut self, value: u8) {
        self.set_value_in_pins(value, &PPI_PIN_DATA_A_BUS)
    }
    pub fn get_data_a_from_pins(& self) -> u8 {
        self.get_8bit_value_from_pins(&PPI_PIN_DATA_A_BUS)
    }

    fn set_data_b_on_pins(&mut self, value: u8) {
        self.set_value_in_pins(value, &PPI_PIN_DATA_B_BUS)
    }
    pub fn get_data_b_from_pins(& self) -> u8 {
        self.get_8bit_value_from_pins(&PPI_PIN_DATA_B_BUS)
    }

    fn set_data_c_on_pins(&mut self, value: u8) {
        self.set_value_in_pins(value, &PPI_PIN_DATA_C_BUS)
    }
    pub fn get_data_c_from_pins(& self) -> u8 {
        self.get_8bit_value_from_pins(&PPI_PIN_DATA_C_BUS)
    }
}

impl Ppi {
    fn set_control_configuration(&mut self, byte: u8) {
        self.direction_c_lower = if byte & 1 << 0 != 0 {
            PpiDirection::Input
        } else {
            PpiDirection::Output
        };

        self.direction_b = if byte & 1 << 1 != 0 {
            PpiDirection::Input
        } else {
            PpiDirection::Output
        };

        let mode = if byte & 1 << 2 != 0 {
            PpiMode::Mode1
        } else {
            PpiMode::Mode0
        };
        self.mode_b = mode;
        self.mode_c_lower = mode;

        self.direction_c_upper = if byte & 1 << 3 != 0 {
            PpiDirection::Input
        } else {
            PpiDirection::Output
        };

        self.direction_a = if byte & 1 << 4 != 0 {
            PpiDirection::Input
        } else {
            PpiDirection::Output
        };

        let mode = match (byte & 1 << 6 != 0, byte & 1 << 5 != 0) {
            (false, false) => PpiMode::Mode0,
            (false, true) => PpiMode::Mode1,
            (true, false) => PpiMode::Mode2,
            (true, true) => PpiMode::Mode0,
        };
        self.mode_a = mode;
        self.mode_c_upper = mode;
    }
}

impl Chip for Ppi {
    type PinType = PpiPin;

    fn must_execute_tick(&self) -> bool {
        !self.get_pin(PpiPin::Neg_CS) &
        (!self.get_pin(PpiPin::Neg_RD) | !self.get_pin(PpiPin::Neg_WR))
    }

    fn execute_tick(&mut self) {

      ///  dbg!(self.is_port_a_addressed() , self.is_port_b_addressed() , self.is_port_c_addressed());
        if self.is_port_a_addressed() && self.mode_a == PpiMode::Mode0 {
            if !self.get_pin(PpiPin::Neg_WR) {
                // Writing to an input-configured port still updates the
                // internal output latch on a real 8255 (it just isn't
                // driven onto the external pins) - see the read-side
                // comment just below for why this doesn't need
                // `direction_a` either.
                let byte = self.get_data_from_pins();
                self.set_data_a_on_pins(byte);
            } else if !self.get_pin(PpiPin::Neg_RD) {
                // Reading an input-configured port returns the live input
                // pins; reading an output-configured one returns the
                // output latch (last written value) - on the real 8255,
                // both are valid, so this doesn't need `direction_a` at all.
                let byte = self.get_data_a_from_pins();
                self.set_data_on_pins(byte);
            } else {
                unimplemented!()
            }
        } else if self.is_port_b_addressed() && self.mode_b == PpiMode::Mode0 {
            if !self.get_pin(PpiPin::Neg_WR) && self.direction_b == PpiDirection::Output {
                eprintln!("Warning vsync is supposely changed by ppi, but not properly handled by emulator");
                self.set_pin(PpiPin::B0, self.get_pin(PpiPin::D0));
            } else if !self.get_pin(PpiPin::Neg_WR) {
                // Unlike port A/C, port B's pins (B0=VSYNC, B1-B3=brand,
                // B4=refresh rate) are hardwired to real external signals
                // in this emulator, not just a CPU-driven latch - so a
                // write while it's configured Input must be a genuine
                // no-op here (real hardware would latch it internally, but
                // that latch is never what's read back for an
                // input-configured port, so there's nothing to do).
                // Silently discarding the byte is correct; overwriting
                // B0-B7 the way port A/C's write does would corrupt VSYNC/
                // brand/refresh with whatever the CPU happened to write.
            } else if !self.get_pin(PpiPin::Neg_RD) {
                // See the port A comment: valid whether port B is
                // configured Input (live pins, e.g. VSYNC on B0) or Output
                // (the output latch).
                let byte = self.get_data_b_from_pins();
                self.set_data_on_pins(byte);

                assert_eq!(byte, self.get_data_from_pins());
            } else {
                unimplemented!()
            }
        } else if self.is_port_c_addressed()
            && self.mode_c_lower == PpiMode::Mode0
            && self.mode_c_upper == PpiMode::Mode0
        {
            if !self.get_pin(PpiPin::Neg_WR) {
                // Same reasoning as port A: the write goes to the output
                // latch regardless of direction_c_lower/direction_c_upper.
                let byte = self.get_data_from_pins();
                self.set_data_c_on_pins(byte);
            } else if !self.get_pin(PpiPin::Neg_RD) {
                // See the port A comment: valid regardless of
                // direction_c_lower/direction_c_upper - e.g. the Madram/
                // AmsLive CRTC-type detection deliberately reads Port C
                // right after configuring it as Output, to check whether
                // the mode-set write cleared its output latch to 0 (real
                // PPI) or not (CPC+'s buggy PPI clone).
                let byte = self.get_data_c_from_pins();
                self.set_data_on_pins(byte);
            } else {
                unimplemented!()
            }
        } else if self.is_control_addressed() {
            let byte = self.get_data_from_pins();
            self.set_control_configuration(byte);

            if self.clears_ports_on_mode_set {
                self.set_data_a_on_pins(0);
                // Real 8255 hardware clears port B's output latch to 0 on a
                // mode-set write, but in this emulator B0/B1-B3/B4 aren't CPU
                // output latches - they're hardwired to VSYNC, the manufacturer
                // jumpers and the refresh-rate strap. Zeroing them here would
                // wipe out the brand the firmware is about to read back.
                self.drive_hardwired_port_b_bits();
                self.set_data_c_on_pins(0);
            }
        } else {
            let byte = self.get_data_from_pins();

            let bit = byte & 1 << 0 != 0;
            let pin = [
                PpiPin::C0,
                PpiPin::C1,
                PpiPin::C2,
                PpiPin::C3,
                PpiPin::C4,
                PpiPin::C5,
                PpiPin::C6,
                PpiPin::C7,
            ][((byte & 0b1110) >> 1) as usize];
            self.set_pin(pin, bit)
        }
    }

    /// The buffer that encodes the current state of the pin (previous, current) (writeable)
    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }
    /// The buffer that encodes the current state of the pin (previous, current) (readable)
    fn pins_buffer(&self) -> &PinsBuffer {
        &self.pins
    }

    /// Reset pins and counter
    fn reset(&mut self) {
        self.pins = Default::default();

        self.drive_hardwired_port_b_bits();
    }
}

impl DataChip for Ppi {
    /// Returns the ordered list of pins that contains the data
    fn pins_data() -> &'static [Self::PinType; 8] {
        &PPI_PIN_DATA_BUS
    }
}

impl ReadDataChip for Ppi {}
impl WriteDataChip for Ppi {}

impl Activable for Ppi {
    fn clock_pin() -> Self::PinType {
        PpiPin::Neg_CS
    }

    fn activated_at_up_front() -> bool {
        false
    }
}
