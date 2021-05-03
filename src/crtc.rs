use crate::chip::*;

use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;

#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum CrtcPin {
    D0,
    D1,
    D2,
    D3,
    D4,
    D5,
    D6,
    D7,

    /// Register select (A8)
    RS,
    /// Chip Select (14)
    CS,
    /// Read Write (A9)
    RW,
    /// Enable the CRTC
    EN,

    /// Vertical Sync
    VS,
    /// Horizontal Sync
    HS,

    /// Display Enable
    DE,

    /// Clock
    CLK,

    /// Reset
    RESET,

    MA13,
    MA12,
    MA11,
    MA10,
    MA9,
    MA8,
    MA7,
    MA6,
    MA5,
    MA4,
    MA3,
    MA2,
    MA1,
    MA0,

    RA0,
    RA1,
    RA2,
    RA3,
    RA4
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
    CrtcPin::RA2,
    CrtcPin::MA13,
    CrtcPin::MA12,
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
    CrtcPin::CLK
];

#[derive(Default)]
/// https://www.grimware.org/doku.php/documentations/devices/crtc
/// https://neuro-sys.github.io/2019/10/01/amstrad-cpc-crtc.html
pub struct Crtc {
    pins: u64,

    /// Address Register / Selection of the working register
    ar: u8,
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
    /// Vertical Line Counter
    vlc: u8,
    /// Memory Adress
    ma: u16

    selected_register: u8,
    crtc_type: CrtcType,

    hsync_counter: u8,
    vsync_counter: u8

}

#[derive(PartialEq)]
pub enum CrtcType {
    Crtc0,
    Crtc1,
    Crtc2,
    Crtc3
}

impl CrtcType {
    /// Provides the mask to keep the bit depending on the CRTC when reading the value
    pub fn read_register_mask(&self, r: u8) ->u8 {
        match r {
            0..10 => unreachable!(),
            10 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0b00011111
                }
                else {
                    0b01111111
                }
            },

            11 => {
                0b00011111
            },

            12 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0b00111111
                }
                else {
                    0x00
                }
            },

            13 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0xff
                }
                else {
                    0x00
                }
            },

            14 | 16  => {
                0xb111111
            },

            
            15 | 17 => {
                0xff
            },

            18.. => unreachable!()

        }
    }

    /// Provides the mask to keep the bit depending on the CRTC when writting the value
    pub fn write_register_mask(&self, r: u8) ->u8 {
        match r {
            0 | 1 | 2 | 13 | 15=> {
                0xff
            },

            3 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4()  {
                    0b10001111
                }
                else {
                    0b1111
                }
            },

            4 => {
                0b01111111
            },

            5 | 9 | 11 => {
                0b00011111
            },

            6 | 7 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4()  {
                    0b01111111
                }
                else  {
                    0b00111111
                }
            },

            8 => {
                if self.is_crtc0() || self.is_crtc2() || self.is_crtc4() {
                    0b00000011
                }
                else if self.is_crtc1() {
                    0xff
                }
                else {
                    0b11110011
                }
            },

            10 => {
                if self.is_crtc0() || self.is_crtc3() || self.is_crtc4() {
                    0b00011111
                }
                else {
                    0b01111111
                }
            },

            12 | 14 => {
                0b00111111
            },

            16 .. => {
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
        let reg = self.read_data_from_pins();
        self.selected_register = reg & 0b11111;
    }

    /// Execute the CRTC register write
    pub fn write_crtc_register(&mut self) {
        let val = self.read_data_from_pins();
        if self.is_register_writable(self.selected_register) {
            self.do_write_register(self.selected_register, val);
        }
    }

    /// Execute the CRTC register status
    pub fn status_register(&mut self) {
        let data: u8 = if self.crtc_type.is_crtc1() {
            todo!("Implement CRTC1 behavior when reading status register");
            let u = 1; // TODO set the right value
            let v = 1; // TODO set the right value
            let l = self.get_pin(CrtcPin::VSYNC); // TODO set the right value
            u<<7 + l<<6 + v<<5
        } 
        else if self.crtc_type.is_crtc3() || self.crtc_type.is_crtc4() {
            self.do_read_register(self.selected_register)
        }
        else {
            rand::thread_rng().gen()
        }

        self.set_data_on_pins(data);
    }

    /// Execute the CRTC register read
    pub fn read_crtc_register(&mut self) {
        if self.is_register_readable(self.selected_register) {
            let val = self.do_read_register(self.selected_register) & self.crtc_type.read_register_mask(self.selected_register);
            self.write_data_on_pins(val);
        }
    }


    /// REturn true if the CRTC allows the writting of the register
    pub fn is_register_writable(&self, r: u8) -> bool {
        match r {
            0..=15 => true,
            _ => false
        }
    }

    /// REturn true if the CRTC allows the reading of the register
    pub fn is_register_readable(&self, r: u8) -> bool {
        match r {
            10..=17 => true,
            _ => false
        }
    }

    /// panic if register is not valid
    /// Keep the bits of interest (that depend on the crtc) and store it
    pub fn do_write_register(&mut self, r:u8, v:u8) {
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
            _ => unreachable!()
        };
        *r = v;
    }

    pub fn do_read_register(&self, r: u8) -> u8 {
        let r = match r {
            0 => &self.r0,
            1 => &self.r1,
            2 => &self.r2,
            3 => &self.r3,
            4 => &self.r4,
            5 => &self.r5,
            6 => &self.r6,
            7 => &self.r7,
            8 => &self.r8,
            9 => &self.r9,
            10 => &self.r10,
            11 => &self.r11,
            12 => &self.r12,
            13 => &self.r13,
            14 => &self.r14,
            15 => &self.r15,
            _ => unreachable!()
        };
    }


    /// A new screen begins
    /// This is call when VCC=0 and VLC<=r9 CRTC1 but only called when VCC and VLC are reseted for the others
    fn trigger_new_screen_if_needed(&mut self) {
        let trigger = if self.crtc_type.is_crtc1() {
            self.vcc == 0 && self.vlc <= self.r9
        }
        else {
            self.vcc == 0 && self.vlc == 0
        };

        if trigger && self.hcc == 0 {
            self.ma = self.r12 << 8 + self.r13;
        }
    }

    fn vsync_height(&self) -> u8 {
        let val = (self.r3 & 0b11110000) >> 4;
        if val == 0 {
            16 // raster line ?
        }
        else {
            val
        }
    }

    fn hsync_width(&self) -> u8 {
        /// To set 0 is problematic
        let val = self.r3 & 0b00001111;
    }


    fn update_memory_address(&mut self) {
        self.hcc += 1;
        self.ma += 1;

        // decrease hsync counter if any
        if self.hsync_counter != 0 {
            self.hsync_counter -= 1;
        }


        if self.hcc == self.r0 {
            // This is the end of the line, we have to trigger all the other stuffs
            self.hcc = 0; 
            self.vlc += 1;

            // This is the end of a char line
            if self.vlc == self.r9 {
                self.vlc = 0;
                self.vcc += 1;

                // decrease vsync counter if any
                if self.vsync_counter != 0 {
                    self.vsync_counter -= 1;
                }

                if self.vcc == self.r4 {
                    // This is the end of the screen
                    self.vcc == 0;
                }
            }
        }

        if self.hcc >= self.r1 || self.vcc >= self.r6 {
            self.set_pin(CrtcPin::DE, false); // Activate Border
        }
        else {
            // TODO handle the special case of CRTC1 that is unable to leave this state at some point
            self.set_pin(CrtcPin::DE, true);
        }

        // Request an hsync if needed
        if self.hcc == self.r2 {
            self.hsync_counter = self.hsync_width();
        }

        // Request a vsync if needed
        if self.vcc == self.r7 {
            self.vsync_counter == self.vsync_height();
        }

  

        self.set_pin(CrtcPin::HS, self.hsync_counter != 0); // Activate Hsync if needed
        self.set_pin(CrtcPin::VS, self.vsync_counter != 0); // Activate Vsync if needed

        self.trigger_new_screen_if_needed();
        self.set_ma_ra_on_pins();
        self.set_vlc_on_pins();
    }
 
}

impl Chip for Crtc {
    type PinType = CrtcPin;

    fn pins_buffer_mut(&mut self) -> &mut u64 {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &u64 {
        &self.pins
    }

    fn must_execute_tick(&self) -> bool {
        self.get_pin(CrtcPin::CLK) && ! self.get_pin(A14)
    }

    /// Execute the requested command from the CPU
    fn execute_tick(&mut self) {
        match (self.get_pin(CrtcPin::A9), self.get_pin(CrtcPin::A8) {
            (false, false) => self.select_crtc_register(),
            (false, true) => self.write_crtc_register(),
            (true, false) => self.status_register(),
            (true, true) => self.read_crtc_register()
        };

        self.update_memory_address();
    }

    /// Execute the CRTC stuff command
    /// TODO use buffered values
    fn pre_tick(&mut self) {
        // Screen address depends on the clock state
        self.set_ma_ra_on_pins();
    }

    fn reset(&mut self) {
        self.ma = 0;

        self.r0 = 63;
        self.r1 = 40;
        self.r2 = 46;
        self.r3 = 128+14;
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
        self.vcc = self.r9 - 1;
        self.vlc = self.r4 - 1;
        self.hcc = self.r1 - 1;

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
impl ReadAddressChip for Crtc {}
// Does not implement the writable one because adress is generated
impl Activable for Crtc {}


#[cfg(test)]
mod test {
    user super::*;

    #[test]
    fn test_address_generation() {
        let mut crtc = Crtc::default();

        crtc.r12 = 0x30;
        crtc.r13 = 0x00;
        crtc.pre_tick();
        crtc.execute_tick();
        let address1 = self.get_address_from_pins();

        crtc.pre_tick();
        crtc.execute_tick();
        let address2 = self.get_address_from_pins();


        assert_eq!(
            (0xc000, 0xc001),
            (address1, address2)
        )

    }
}