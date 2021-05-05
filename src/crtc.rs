use crate::chip::*;

use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;
use rand::{thread_rng, Rng};

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum CrtcPin {
    D0 = 1 << 0,
    D1 = 1 << 1,
    D2 = 1 << 2,
    D3 = 1 << 3,
    D4 = 1 << 4,
    D5 = 1 << 5,
    D6 = 1 << 6,
    D7 = 1 << 7,

    /// Register select (A8)
    RS = 1 << 8,
    /// Chip Select (14)
    CS = 1 << 9,
    /// Read Write (A9)
    RW = 1 << 10,
    /// Enable the CRTC
    EN = 1 << 11,

    /// Vertical Sync
    VS = 1 << 12,
    /// Horizontal Sync
    HS = 1 << 13,

    /// Display Enable
    DE = 1 << 14,

    /// Clock
    CCLK = 1 << 15,

    /// Reset
    RESET = 1 << 16,

    MA13 = 1 << 17,
    MA12 = 1 << 18,
    MA11 = 1 << 19,
    MA10 = 1 << 20,
    MA9 = 1 << 21,
    MA8 = 1 << 22,
    MA7 = 1 << 23,
    MA6 = 1 << 24,
    MA5 = 1 << 25,
    MA4 = 1 << 26,
    MA3 = 1 << 27,
    MA2 = 1 << 28,
    MA1 = 1 << 29,
    MA0 = 1 << 30,

    RA0 = 1 << 31,
    RA1 = 1 << 32,
    RA2 = 1 << 33,
    RA3 = 1 << 34,
    RA4 = 1 << 35,
}

impl Into<u64> for CrtcPin {
    fn into(self) -> u64 {
        self as u64
    }
}

const CRTC_PIN_DATA_BUS: [CrtcPin; 8] = [
    CrtcPin::D7,
    CrtcPin::D6,
    CrtcPin::D5,
    CrtcPin::D4,
    CrtcPin::D3,
    CrtcPin::D2,
    CrtcPin::D1,
    CrtcPin::D0,
];

const CRTC_PIN_ADDRESS_BUS: [CrtcPin; 16] = [
    CrtcPin::MA13,
    CrtcPin::MA12,
    CrtcPin::RA2,
    CrtcPin::RA1,
    CrtcPin::RA0,
    CrtcPin::MA9,
    CrtcPin::MA8,
    CrtcPin::MA7,
    CrtcPin::MA6,
    CrtcPin::MA5,
    CrtcPin::MA4,
    CrtcPin::MA3,
    CrtcPin::MA2,
    CrtcPin::MA1,
    CrtcPin::MA0,
    CrtcPin::CCLK,
];

#[derive(Default)]
/// https://www.grimware.org/doku.php/documentations/devices/crtc
/// https://neuro-sys.github.io/2019/10/01/amstrad-cpc-crtc.html
pub struct Crtc {
    pins: PinsBuffer,

    /// Address Register / Selection of the working register
    ar: u8,
    /// Status register (crtvc 1)
    sr: u8,
    /// Horizontal Total Register
    r0: u8,
    /// Horizontal Display Register
    r1: u8,
    /// Horizontal Sync Position Register
    r2: u8,
    /// Sync Width Register
    r3: u8,
    /// Verticzl Total Register
    r4: u8,
    /// Vertical Total Adjust
    r5: u8,
    /// Vertical Display Register
    r6: u8,
    /// Vertical Sync Position
    r7: u8,
    /// Interlaced mode
    r8: u8,
    /// Maximum Scan Line Register
    r9: u8,
    /// Cursor start register
    r10: u8,
    r11: u8,
    r12: u8,
    r13: u8,
    r14: u8,
    r15: u8,
    r16: u8,
    r17: u8,

    /// Horizontal Char Counter
    hcc: u8,
    /// Vertical Char Counter
    vcc: u8,
    /// Vertical Line Counter (Raster address)
    vlc: u8,

    /// Memory Adress to put on the pins
    ma: u16,
    /// Base address (corresponds to bufferised R12/R13)
    base: u16,

    crtc_type: CrtcType,

    hsync_counter: u8,
    vsync_counter: u8,
}

#[derive(PartialEq)]
pub enum CrtcType {
    Crtc0,
    Crtc1,
    Crtc2,
    Crtc3,
    Crtc4,
}

impl Default for CrtcType {
    fn default() -> Self {
        CrtcType::Crtc0
    }
}

impl CrtcType {
    /// Provides the mask to keep the bit depending on the CRTC when reading the value
    pub fn read_register_mask(&self, r: u8) -> u8 {
        match r {
            0..10 => unreachable!(),
            10 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0b00011111
                } else {
                    0b01111111
                }
            }

            11 => 0b00011111,

            12 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0b00111111
                } else {
                    0x00
                }
            }

            13 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0xff
                } else {
                    0x00
                }
            }

            14 | 16 => 0b111111,

            15 | 17 => 0xff,

            18.. => unreachable!(),
        }
    }

    /// Provides the mask to keep the bit depending on the CRTC when writting the value
    pub fn write_register_mask(&self, r: u8) -> u8 {
        match r {
            0 | 1 | 2 | 13 | 15 => 0xff,

            3 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0b10001111
                } else {
                    0b1111
                }
            }

            4 => 0b01111111,

            5 | 9 | 11 => 0b00011111,

            6 | 7 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0b01111111
                } else {
                    0b00111111
                }
            }

            8 => {
                if self.is_crtc0() || self.is_crtc2() || self.is_crtc4() {
                    0b00000011
                } else if self.is_crtc1() {
                    0xff
                } else {
                    0b11110011
                }
            }

            10 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0b00011111
                } else {
                    0b01111111
                }
            }

            12 | 14 => 0b00111111,

            16.. => {
                unimplemented!()
            }
        }
    }

    fn is_crtc0(&self) -> bool {
        *self == CrtcType::Crtc0
    }

    fn is_crtc1(&self) -> bool {
        *self == CrtcType::Crtc1
    }

    fn is_crtc2(&self) -> bool {
        *self == CrtcType::Crtc2
    }

    fn is_crtc3(&self) -> bool {
        *self == CrtcType::Crtc3
    }

    fn is_crtc4(&self) -> bool {
        *self == CrtcType::Crtc4
    }
}

impl Crtc {
    /// Execute the CRTC register selection
    pub fn select_crtc_register(&mut self) {
        let reg = self.get_data_from_pins();
        self.ar = reg & 0b11111;
    }

    /// Execute the CRTC register write
    pub fn write_crtc_register(&mut self) {
        let val = self.get_data_from_pins();
        if self.is_register_writable(self.ar) {
            self.do_write_register(self.ar, val);
        }
    }

    /// Execute the CRTC register status
    pub fn status_register(&mut self) {
        let data: u8 = if self.crtc_type.is_crtc1() {
            todo!("Implement CRTC1 behavior when reading status register");
            let u = 1; // TODO set the right value
            let v = 1; // TODO set the right value
            let l = if self.get_pin(CrtcPin::VS) { 1 } else { 0 }; // TODO set the right value
            u << 7 + l << 6 + v << 5
        } else if self.crtc_type.is_crtc3() || self.crtc_type.is_crtc4() {
            self.do_read_register(self.ar)
        } else {
            thread_rng().gen::<u8>()
        };

        self.set_data_on_pins(data);
    }

    /// Execute the CRTC register read
    pub fn read_crtc_register(&mut self) {
        if self.is_register_readable(self.ar) {
            let val = self.do_read_register(self.ar) & self.crtc_type.read_register_mask(self.ar);
            self.set_data_on_pins(val);
        }
    }

    /// REturn true if the CRTC allows the writting of the register
    pub fn is_register_writable(&self, r: u8) -> bool {
        match r {
            0..=15 => true,
            _ => false,
        }
    }

    /// REturn true if the CRTC allows the reading of the register
    pub fn is_register_readable(&self, r: u8) -> bool {
        match r {
            10..=17 => true,
            _ => false,
        }
    }

    /// panic if register is not valid
    /// Keep the bits of interest (that depend on the crtc) and store it
    pub fn do_write_register(&mut self, r: u8, v: u8) {
        let v = v & self.crtc_type.write_register_mask(r);
        let r = match r {
            0 => &mut self.r0,
            1 => &mut self.r1,
            2 => &mut self.r2,
            3 => &mut self.r3,
            4 => &mut self.r4,
            5 => &mut self.r5,
            6 => &mut self.r6,
            7 => &mut self.r7,
            8 => &mut self.r8,
            9 => &mut self.r9,
            10 => &mut self.r10,
            11 => &mut self.r11,
            12 => &mut self.r12,
            13 => &mut self.r13,
            14 => &mut self.r14,
            15 => &mut self.r15,
            _ => unreachable!(),
        };
        *r = v;
    }

    pub fn do_read_register(&self, r: u8) -> u8 {
        let v = match r {
            0 => self.r0,
            1 => self.r1,
            2 => self.r2,
            3 => self.r3,
            4 => self.r4,
            5 => self.r5,
            6 => self.r6,
            7 => self.r7,
            8 => self.r8,
            9 => self.r9,
            10 => self.r10,
            11 => self.r11,
            12 => self.r12,
            13 => self.r13,
            14 => self.r14,
            15 => self.r15,
            _ => panic!(),
        };
        let m = self.crtc_type.read_register_mask(r);
        v & m
    }

    /// A new screen begins
    /// This is call when VCC=0 and VLC<=r9 CRTC1 but only called when VCC and VLC are reseted for the others
    fn trigger_new_screen_if_needed(&mut self) {
        let trigger = if self.crtc_type.is_crtc1() {
            self.vcc == 0 && self.vlc <= self.r9
        } else {
            self.vcc == 0 && self.vlc == 0
        };

        if trigger && self.hcc == 0 {
            self.base = (self.r12 as u16) << 8 + self.r13 as u16;
        }
    }

    fn vsync_height(&self) -> u8 {
        let val = (self.r3 & 0b11110000) >> 4;
        if val == 0 {
            16 // raster line ?
        } else {
            val
        }
    }

    /// To set 0 is problematic (for non CTM I would say)
    fn hsync_width(&self) -> u8 {
        self.r3 & 0b00001111
    }

    fn update_memory_address(&mut self) {
        dbg!(&self.hcc, &self.vcc, &self.vlc);

        // decrease hsync counter if any
        if self.hsync_counter != 0 {
            self.hsync_counter -= 1;
        }

        self.hcc += 1;

        if self.hcc == self.r0 + 1 {
            // This is the end of the line, we have to trigger all the other stuffs
            self.hcc = 0;
            self.vlc += 1;

            // This is the end of a char line
            if self.vlc == self.r9 + 1 {
                self.vlc = 0;
                self.vcc += 1;

                // decrease vsync counter if any
                if self.vsync_counter != 0 {
                    self.vsync_counter -= 1;
                }

                if self.vcc == self.r4 + 1 {
                    // This is the end of the screen
                    self.vcc = 0;
                }
            }
        }

        if self.hcc >= self.r1 || self.vcc >= self.r6 {
            self.set_pin(CrtcPin::DE, false); // Activate Border
        } else {
            // TODO handle the special case of CRTC1 that is unable to leave this state at some point
            self.set_pin(CrtcPin::DE, true);
        }

        // Request an hsync if needed
        if self.hcc == self.r2 {
            self.hsync_counter = self.hsync_width();
        }

        // Request a vsync if needed
        if self.vcc == self.r7 {
            self.vsync_counter = self.vsync_height();
        }

        self.set_pin(CrtcPin::HS, self.hsync_counter != 0); // Activate Hsync if needed
        self.set_pin(CrtcPin::VS, self.vsync_counter != 0); // Activate Vsync if needed

        self.trigger_new_screen_if_needed();
        self.set_ma_ra_on_pins();
    }

    /// MA and RA are variables manipulated during the process
    /// they are directly wired to the pin; this fonction replicate them here
    fn set_ma_ra_on_pins(&mut self) {
        self.ma = self.vcc as u16 * self.r0 as u16 + self.hcc as u16 + self.base;

        self.set_value_in_pins(
            self.ma & ((1 << 15) - 1),
            &[
                CrtcPin::MA13,
                CrtcPin::MA12,
                CrtcPin::MA11,
                CrtcPin::MA10,
                CrtcPin::MA9,
                CrtcPin::MA8,
                CrtcPin::MA7,
                CrtcPin::MA6,
                CrtcPin::MA5,
                CrtcPin::MA4,
                CrtcPin::MA3,
                CrtcPin::MA2,
                CrtcPin::MA1,
                CrtcPin::MA0,
            ],
        );

        self.set_value_in_pins(
            self.vlc & 0b11111,
            &[
                CrtcPin::RA4,
                CrtcPin::RA3,
                CrtcPin::RA2,
                CrtcPin::RA1,
                CrtcPin::RA0,
            ],
        )
    }
}

impl Chip for Crtc {
    type PinType = CrtcPin;

    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &PinsBuffer {
        &self.pins
    }

    fn must_execute_tick(&self) -> bool {
        self.get_pin(CrtcPin::CCLK) && !self.get_pin(CrtcPin::CS)
    }

    /// Execute the requested command from the CPU
    fn execute_tick(&mut self) {
        match (self.get_pin(CrtcPin::RW), self.get_pin(CrtcPin::RS)) {
            (false, false) => self.select_crtc_register(),
            (false, true) => self.write_crtc_register(),
            (true, false) => self.status_register(),
            (true, true) => self.read_crtc_register(),
        };

        self.update_memory_address();
    }

    /// Execute the CRTC stuff command
    /// TODO use buffered values
    fn pre_tick(&mut self) {
        //  self.set_ma_ra_on_pins(); // For the CCLK change to ensure get_address works properly
    }

    fn reset(&mut self) {
        self.ma = 0;

        self.r0 = 63;
        self.r1 = 40;
        self.r2 = 46;
        self.r3 = 128 + 14;
        self.r4 = 38;
        self.r5 = 0;
        self.r6 = 25;
        self.r7 = 30;
        self.r8 = 0;
        self.r9 = 7;
        self.r10 = 0;
        self.r11 = 0;
        self.r12 = 48;
        self.r13 = 0;
        self.r14 = 0;
        self.r15 = 0;
        self.r16 = 0;
        self.r17 = 0;

        // Set the counters to the limits
        self.vcc = self.r4;
        self.vlc = self.r9;
        self.hcc = self.r0;
    }
}

impl DataChip for Crtc {
    fn pins_data() -> &'static [Self::PinType; 8] {
        return &CRTC_PIN_DATA_BUS;
    }
}
impl ReadDataChip for Crtc {}
impl WriteDataChip for Crtc {}

impl AddressChip for Crtc {
    fn pins_address() -> &'static [Self::PinType; 16] {
        return &CRTC_PIN_ADDRESS_BUS;
    }
}
impl ReadAddressChip for Crtc {
    /// CCLK as to be negated
    fn get_address_from_pins(&self) -> u16 {
        let addr = self.get_value_from_pins(Self::pins_address());
        addr ^ 1
    }
}
// Does not implement the writable one because adress is generated
impl Activable for Crtc {
    fn clock_pin() -> Self::PinType {
        CrtcPin::CCLK
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_address_generation() {
        let mut crtc = Crtc::default();
        crtc.reset();

        assert_eq!(crtc.r1, 40);
        assert_eq!(crtc.r4, 38);
        assert_eq!(crtc.r9, 7);

        crtc.set_pin(CrtcPin::CCLK, true);
        crtc.set_pin(CrtcPin::CS, false);
        crtc.set_pin(CrtcPin::RW, false);
        crtc.set_pin(CrtcPin::RS, false);
        crtc.set_pin(CrtcPin::EN, false);

        crtc.r12 = 0x30;
        crtc.r13 = 0x00;
        crtc.vcc = 0;
        crtc.vlc = 0;
        crtc.hcc = 0;
        crtc.base = 0x3000;
        crtc.set_ma_ra_on_pins();

        let address1 = crtc.get_address_from_pins();

        crtc.set_pin(CrtcPin::CCLK, false);
        crtc.set_ma_ra_on_pins();

        let address2 = crtc.get_address_from_pins();

        assert_eq!((0xc000, 0xc001), (address1, address2));
    }
}
