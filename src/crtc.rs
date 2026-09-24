use crate::bits::*;
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

#[derive(Debug)]
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

    ///  	The Horizontal Character Counter (an internal CRTC register). Keeps track of the current horizontal character number. When this value equals R1 of the CRTC (horizontal displayed) display output is disabled for the rest of the current line. When this value equals R0 of the CRTC (horizontal total), display output is enabled and this value is reset to zero. When this value equals R2 of the CRTC (horizontal sync position) a new scan line may be started depending on the current HSYNC position of the monitor.
    pub hcc: u8,
    /// The Vertical Character Counter (an internal CRTC register). Keeps track of the current vertical character number. When this value equals R6 of the CRTC (vertical displayed), display output is disabled. When this value equals R4 of the CRTC (vertical total), display output is enabled and the display base address is reset. When this value equals R7 of the CRTC (vertical sync position) a Vertical Sync occurs, the display will start drawing from the top if the current VDUR value is within a threshold range.
    pub vcc: Bit8Max7,
    ///  	The Vertical Line Counter (an internal CRTC register). Keeps track of the current raster line within the character. When this equals R9 (maximum scan line) of the CRTC the VCC will be incremented next line.
    pub vlc: Bit8Max5,
    ///  	The Vertical Sync counter (an internal emulator register). When a Vertical Sync occurs this register is set to zero. When a frame flyback is active this will be displayed in light blue and count the lines in the frame flyback period.
    pub vsc: u8,
    /// The Horizontal Display Counter shows the current horizontal position on the monitor being drawn.
    pub hdc: u8,
    /// The Vertical Total Adjust Counter is shown in light blue when the end of the current CRTC frame is reached (after VCC = R4) and CRTC Register 5 has a value other than zero. This counts up to the value in CRTC register 5 before the next frame is started.
    pub vtac: u8,
    /// True while the CRTC is between "VCC just reached R4" and "VTAC just reached R5":
    /// VCC/VLC are held and only VTAC advances, one extra raster line at a
    /// time, until the frame's total scan-line count has been topped up to
    /// exactly R5 lines past the nominal (R4+1)*(R9+1) - the mechanism a
    /// real 6845 uses to hit an exact line count (e.g. 312 for PAL) when
    /// (R4+1)*(R9+1) alone doesn't divide evenly.
    in_vertical_adjust: bool,
    /// The Horizontal Sync Counter counts the number of cycles during a HSYNC. This is shown in light blue when the CRTC Horizontal Sync period is active.
    pub hsc: u8,

    ///  The MA is reloaded with the value from R12 and R13 when VCC=0 and VLC=0 (that's when a new CRTC screen begin). However, CRTC Type 1 keep updating the MA on every new scanline while VCC=0 (and VLC=<R9)
    pub ma: u16,

    crtc_type: CrtcType,
}

impl Default for Crtc {
    fn default() -> Self {
        let mut crtc = Crtc {
            pins: Default::default(),
            ar: Default::default(),
            sr: Default::default(),
            r0: Default::default(),
            r1: Default::default(),
            r2: Default::default(),
            r3: Default::default(),
            r4: Default::default(),
            r5: Default::default(),
            r6: Default::default(),
            r7: Default::default(),
            r8: Default::default(),
            r9: Default::default(),
            r10: Default::default(),
            r11: Default::default(),
            r12: Default::default(),
            r13: Default::default(),
            r14: Default::default(),
            r15: Default::default(),
            r16: Default::default(),
            r17: Default::default(),
            hcc: Default::default(),
            vcc: Default::default(),
            vlc: Default::default(),
            vsc: 0, // meaningless when no vsync is present
            hsc: 0, // meaningless when no hsync is present
            hdc: 0,
            vtac: 0,
            in_vertical_adjust: false,
            ma: Default::default(),
            crtc_type: Default::default(),
        };
        crtc.reset();
        crtc
    }
}
#[derive(PartialEq, Debug, Clone, Copy)]
#[repr(u8)]
pub enum CrtcType {
    Crtc0 = 0,
    Crtc1 = 1,
    Crtc2 = 2,
    Crtc3 = 3,
    Crtc4 = 4,
}

impl From<&CrtcType> for Crtc {
    fn from(from: &CrtcType) -> Self {
        let mut crtc = Crtc {
            crtc_type: *from,
            ..Default::default()
        };
        crtc.reset();
        crtc
    }
}

impl From<CrtcType> for Crtc {
    fn from(from: CrtcType) -> Self {
        let mut crtc = Crtc {
            crtc_type: from,
            ..Default::default()
        };
        crtc.reset();
        crtc
    }
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
    pub fn r0(&self) -> u8 {
        self.r0
    }
    pub fn r1(&self) -> u8 {
        self.r1
    }
    pub fn r2(&self) -> u8 {
        self.r2
    }
    pub fn r3(&self) -> u8 {
        self.r3
    }
    pub fn r4(&self) -> u8 {
        self.r4
    }
    pub fn r5(&self) -> u8 {
        self.r5
    }
    pub fn r6(&self) -> u8 {
        self.r6
    }
    pub fn r7(&self) -> u8 {
        self.r7
    }
    pub fn r8(&self) -> u8 {
        self.r8
    }
    pub fn r9(&self) -> u8 {
        self.r9
    }
    pub fn r10(&self) -> u8 {
        self.r10
    }
    pub fn r11(&self) -> u8 {
        self.r11
    }
    pub fn r12(&self) -> u8 {
        self.r12
    }
    pub fn r13(&self) -> u8 {
        self.r13
    }
    pub fn r14(&self) -> u8 {
        self.r14
    }
    pub fn r15(&self) -> u8 {
        self.r15
    }
    pub fn r16(&self) -> u8 {
        self.r16
    }
    pub fn r17(&self) -> u8 {
        self.r17
    }
    pub fn ar(&self) -> u8 {
        self.ar
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
            _ => 0xff, // request to a wrong register,
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
            self.ma = ((self.r12 as u16) << 8).wrapping_add(self.r13 as u16);
        }
    }
    /// VSync in lines
    fn vsync_height(&self) -> u8 {
        let val = (self.r3 & 0b11110000) >> 4;
        if val == 0 || self.crtc_type.is_crtc2() || self.crtc_type.is_crtc1() {
            16
        } else {
            val
        }
    }

    /// To set 0 is problematic (for non CTM I would say)
    /// Hsync with in chars
    fn hsync_width(&self) -> u8 {
        let width = self.r3 & 0b00001111;
        if width != 0 {
            width
        } else if self.crtc_type.is_crtc2() {
            16 // widht of 16 chars for crtc 2
        } else {
            0 // no hsync for other crtcs
        }
    }

    /// Return the delay to display the border.
    /// it depends on r8 for CRTC 0 3 & 4,
    fn reg8_delay_border(&self) -> u8 {
        match self.crtc_type {
            CrtcType::Crtc0 | CrtcType::Crtc3 | CrtcType::Crtc4 => {
                match self.r8 & 0b110000 {
                    0b000000 => 0,
                    0b010000 => 1,                  // 1 char delay
                    0b100000 => 2,                  // 2 chars delay
                    0b110000 => self.r1 - self.hcc, // ugly trick to force border
                    _ => unreachable!(),
                }
            }
            _ => 0,
        }
    }

    /// Run all the CRTC machinery that compute the next CRTC address
    fn update_memory_address(&mut self) {
        if self.hsync() {
            self.hsc += 1;
            if self.hsc == self.hsync_width() {
                self.off(CrtcPin::HS);
            }
        }

        self.hcc = self.hcc.wrapping_add(1); // XXX check if there is a bit limit in the counter
        self.hdc += 1;

        if self.hcc == self.r0 + 1 {
            // This is the end of the line, we have to trigger all the other stuffs
            self.hcc = 0;

            // handle the end of vsync if any - independent of vertical
            // total adjust, since VSYNC width (R3) and the adjust period
            // (R5) are unrelated and can overlap.
            if self.vsync() {
                self.vsc += 1;
                if self.vsc == self.vsync_height() {
                    self.off(CrtcPin::VS);
                }
            }

            if self.in_vertical_adjust {
                // VCC/VLC are held here; only the adjust counter advances,
                // one raster line at a time, until it has topped up the
                // frame by R5 extra lines.
                self.vtac += 1;
                if self.vtac >= self.r5 {
                    self.in_vertical_adjust = false;
                    self.vtac = 0;
                    self.vcc.reset();
                    self.vlc.reset();
                }
            } else {
                self.vlc += 1;

                // This is the end of a char line
                if self.vlc == self.r9 + 1 {
                    self.vlc.reset();
                    self.vcc += 1;

                    if self.vcc == self.r4 + 1 {
                        if self.r5 == 0 {
                            // This is the end of the screen
                            self.vcc.reset();
                        } else {
                            // Enter vertical total adjust instead of
                            // immediately starting the next frame.
                            self.in_vertical_adjust = true;
                            self.vtac = 0;
                        }
                    }
                }
            }
        }

        if self.hcc.wrapping_add(self.reg8_delay_border()) >= self.r1 || self.vcc >= self.r6 {
            self.set_pin(CrtcPin::DE, false); // Activate Border
        } else {
            // TODO handle the special case of CRTC1 that is unable to leave this state at some point
            self.set_pin(CrtcPin::DE, true);
        }

        // Request an hsync if needed
        if self.hcc == self.r2 {
            self.hsc = 0;
            self.on(CrtcPin::HS);
            self.hdc = 0; // XXX I am pretty sure I am not reseting it at the right place
        }

        // Request a vsync if needed
        if self.vcc == self.r7 && !self.get_pin(CrtcPin::VS){
            self.vsc = 0;
            self.on(CrtcPin::VS); // Activate Vsync
        }

        self.trigger_new_screen_if_needed();
        self.set_address_on_pins(); // set the address in the pins
    }

    /// MA and RA are variables manipulated during the process
    /// they are directly wired to the pin; this fonction replicate them here
    /// should not be called outside of Crtc. Public only for test reasons
    pub fn set_address_on_pins(&mut self) {
        let address = (self.vcc.value() as u16)
            .wrapping_mul(self.r1 as u16)
            .wrapping_add(self.hcc as u16)
            .wrapping_add(self.ma);

        self.set_value_in_pins(
            address & ((1 << 15) - 1),
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
            self.vlc.value(),
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

impl Crtc {
    /// Check if the CRTC as generated a VSYNC
    pub fn vsync(&self) -> bool {
        self.get_pin(CrtcPin::VS)
    }

    /// Check if the CRTC as generated a HSYNC
    pub fn hsync(&self) -> bool {
        self.get_pin(CrtcPin::HS)
    }

    ///  	The Video display address. This is derived from the MA register in the CRTC, the horizontal character count HCC and the current row address (RA) register in the CRTC. This is effectively the CPC memory address of the data displayed on the monitor.
    pub fn vma(&self) -> u16 {
        self.get_address_from_pins()
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
        !self.get_pin(CrtcPin::CS)  & !self.get_pin(CrtcPin::EN)
    }

    /// Execute the requested command from the CPU
    fn execute_tick(&mut self) {

        match (self.get_pin(CrtcPin::RW), self.get_pin(CrtcPin::RS)) {
            (false, false) => self.select_crtc_register(),
            (false, true) => self.write_crtc_register(),
            (true, false) => self.status_register(),
            (true, true) => self.read_crtc_register(),
        };
    }

    /// Execute the normal CRTC address behavior
    fn pre_tick(&mut self) {
        self.update_memory_address();
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
        self.vcc = self.r4.into();
        self.vlc = self.r9.into();
        self.hcc = self.r0;
        self.vtac = 0;
        self.in_vertical_adjust = false;

        self.set_pin(CrtcPin::DE, true);
        self.set_pin(CrtcPin::HS, false);
        self.set_pin(CrtcPin::VS, false);
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

    fn activated_at_up_front() -> bool {
        false
    }
    fn clock_pin() -> Self::PinType {
        CrtcPin::CCLK
    }
}

#[cfg(test)]
mod test {
    use super::*;

 //   #[test]
 //   fn test_chip_selection() {
 //       unimplemented!()

    #[test]
    fn test_address_generation_0xc000() {
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
        crtc.vcc.reset();
        crtc.vlc.reset();
        crtc.hcc = 0;
        crtc.ma = 0x3000;
        crtc.set_address_on_pins();

        let address1 = crtc.get_address_from_pins();

        crtc.set_pin(CrtcPin::CCLK, false);
        crtc.set_address_on_pins();

        let address2 = crtc.get_address_from_pins();

        assert_eq!((0xc000, 0xc001), (address1, address2));
    }

    #[test]
    fn test_address_generation_0x0000() {
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

        crtc.r12 = 0x00;
        crtc.r13 = 0x00;
        crtc.vcc.reset();
        crtc.vlc.reset();
        crtc.hcc = 0;
        crtc.ma = 0x0000;
        crtc.set_address_on_pins();

        let address1 = crtc.get_address_from_pins();

        crtc.set_pin(CrtcPin::CCLK, false);
        crtc.set_address_on_pins();

        let address2 = crtc.get_address_from_pins();

        assert_eq!((0x0000, 0x0001), (address1, address2));
    }

    /// Test the inner working of crtc for a standard screen
    #[test]
    fn test_standard_crtc() {
        let mut crtc = Crtc::default();

        for char_line in 0..=crtc.r4 {
            for scan_line in 0..=crtc.r9 {
                for char in 0..=crtc.r0 {
                    crtc.save_pins_state();
                    crtc.set_pin(CrtcPin::CCLK, true);

                    assert!(!crtc.is_activated());

                    crtc.save_pins_state();
                    crtc.set_pin(CrtcPin::CCLK, false);
                    assert!(crtc.is_activated());
                    assert!(crtc.must_execute_tick());

                    // execute the CRTC address computation
                    crtc.tick();

                    //  dbg!(char_line, scan_line, char, &crtc);

                    // Check that main internal counter are correct
                    assert!(crtc.vcc <= crtc.r4);
                    assert!(crtc.vlc.value() <= crtc.r9);
                    assert!(crtc.hcc <= crtc.r0);

                    assert!(crtc.vcc == char_line);
                    assert!(crtc.vlc.value() == scan_line);
                    assert!(crtc.hcc == char);

                    // Test for border
                    if crtc.hcc >= crtc.r1 || crtc.vcc >= crtc.r6 {
                        assert!(!crtc.get_pin(CrtcPin::DE));
                    } else {
                        assert!(crtc.get_pin(CrtcPin::DE));
                    }

                    // Test for hsync
                    if crtc.hcc >= crtc.r2 && crtc.hcc < crtc.r2 + crtc.hsync_width() {
                        assert!(crtc.get_pin(CrtcPin::HS));
                    } else {
                        assert!(!crtc.get_pin(CrtcPin::HS));
                    }

                    // VSYNC duration is expressed in raster lines, not in character lines.
                    let raster_line = (crtc.vcc.value() as u16) * (crtc.r9 as u16 + 1)
                        + crtc.vlc.value() as u16;
                    let vsync_start = (crtc.r7 as u16) * (crtc.r9 as u16 + 1);
                    let vsync_end = vsync_start + crtc.vsync_height() as u16;
                    if raster_line >= vsync_start && raster_line < vsync_end {
                        assert!(crtc.get_pin(CrtcPin::VS));
                    } else {
                        assert!(!crtc.get_pin(CrtcPin::VS));
                    }

                    // test for address
                    if crtc.get_pin(CrtcPin::DE) {
                        let expected = 0xc000
                            + 0x800 * (scan_line as u16)
                            + 80 * (char_line as u16)
                            + 2 * (char as u16)
                            + 1;
                        let collected = crtc.get_address_from_pins();

                        assert_eq!(expected, collected, "0x{:x}!=0x{:x}", expected, collected);
                    }
                }
            }
        }
    }

    #[test]
    fn test_hsync() {
        let mut crtc = Crtc::default();

        crtc.update_memory_address();

        // ensure hsync is not present
        while !crtc.hsync() | crtc.vsync() {
            crtc.update_memory_address();
        }

        for i in 0..2 {
            let mut hsync_duration = 0;
            while crtc.hsync() {
                hsync_duration += 1;
                crtc.update_memory_address();
            }
            assert_eq!(hsync_duration, crtc.hsync_width());

            let mut hsync_wait = 0;
            while !crtc.hsync() {
                hsync_wait += 1;
                crtc.update_memory_address();
            }
            assert_eq!(hsync_duration + hsync_wait, 64);
        }
    }

    #[test]
    fn test_vsync() {
        let mut crtc = Crtc::default();

        crtc.update_memory_address();

        // ensure hsync is not present
        while !crtc.vsync() {
            crtc.update_memory_address();
        }

        for i in 0..2 {
            let mut vsync_duration = 0;
            while crtc.vsync() {
                vsync_duration += 1;
                crtc.update_memory_address();
            }

            let mut vsync_wait = 0;
            while !crtc.vsync() {
                vsync_wait += 1;
                crtc.update_memory_address();
            }
            assert_eq!(vsync_duration + vsync_wait, 19968);
        }
    }
}
