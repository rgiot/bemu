use crate::chip::*;
use crate::memory::Flag;
use delegate::delegate;


#[allow(non_camel_case_types)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u64)]
pub enum GateArrayPin {
    /// Specify if the pin is for CPU or CRTC. 1MHZ signal
    CPU_ADDR = 1 << 0,
    /// Request the CPU to wait
    READY = 1 << 1,
    /// Validate high byte of address
    CAS = 1 << 2,
    /// Activate data from cpu to ga
    TWO_FOR_FOR_EN = 1 << 3,
    /// Request write in RAM
    MWE = 1 << 4,
    /// ????
    CAS_ADDR = 1 << 5,
    /// Adress is line address
    RAS = 1 << 6,
    /// 16 Mhz signal
    XTAL = 1 << 7,
    VCC2 = 1 << 8,
    /// Request the CPU to interrupt
    INTERRUPT = 1 << 9,
    /// Synchronization for screen
    SYNC = 1 << 10,
    /// Access to ROM
    ROMEN = 1 << 11,
    /// Request data send to CPU
    RAMRD = 1 << 12,
    /// Horizontal synchronisation from CRTC
    HSYNC = 1 << 13,
    /// Vertical synchronisation from CRTC
    VSYNC = 1 << 14,
    /// The CPU request an input/output (ignored if does not match)
    IORQ = 1 << 15,
    /// CPU received an interruption signal
    M1 = 1 << 16,
    /// CPU wants to access the memory
    MERQ = 1 << 17,
    /// CPU wants to read the memory
    RD = 1 << 18,

    A15 = 1 << 19,

    /// 1 Mhz for RAM refresh
    MAO_CCLK = 1 << 20,

    /// 4MHZ for CPU
    PHI = 1 << 21,
    VCC1 = 1 << 22,

    /// Reset of the GA
    RESET = 1 << 23,

    /// Red component of the video signal
    R = 1 << 24,
    GND = 1 << 25,
    /// Green component of the video signal
    G = 1 << 26,
    VCC3 = 1 << 27,
    /// Blue component of the video signal
    B = 1 << 28,

    D7 = 1 << 29,
    D6 = 1 << 30,
    D5 = 1 << 31,
    D4 = 1 << 32,
    D3 = 1 << 33,
    D2 = 1 << 34,
    D1 = 1 << 35,
    D0 = 1 << 36,

    /// CRTC request a display
    DISPEN = 1 << 37,
    VCC4 = 1 << 38,
    A14 = 1 << 39,
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
const GA_PIN_DATA_BUS: [GateArrayPin; 8] = [
    GateArrayPin::D7,
    GateArrayPin::D6,
    GateArrayPin::D5,
    GateArrayPin::D4,
    GateArrayPin::D3,
    GateArrayPin::D2,
    GateArrayPin::D1,
    GateArrayPin::D0,
];

/// Content of the PENR register
struct PENR(u8);
#[derive(Debug, Copy, Clone)]
pub enum Pen {
    Pen(u8),
    Border
}
impl Default for Pen {
    fn default() -> Self {
        Pen::Pen(0)
    }
}
impl Into<u8> for &Pen {
    fn into(self) -> u8 {
        match self {
            Pen::Pen(nb) => *nb,
            Pen::Border => 17
        }
    }
}

/// Content of the INKR register
struct INKR(u8);
#[derive(Debug, Copy, Clone)]
pub struct Ink(u8);

// https://www.grimware.org/doku.php/documentations/devices/gatearray
const RGB_PERCENTAGE_LEVELS_PER_INK: [(f32,f32,f32); 32] = [
    (0.0000, 	0.9615, 	0.4808),
    (0.0000, 	0.9615, 	41.8269),
    (4.8077, 	0.9615, 	95.6731),
    (42.3077, 	0.9615, 	0.4808),
    (41.3462, 	0.9615, 	40.8654),
    (42.3077, 	0.9615, 	94.7115),
    (95.1923, 	1.9231, 	2.4038),
    (94.2308, 	0.9615, 	40.8654),
    (95.1923, 	0.9615, 	95.6731),
    (0.9615, 	47.1154, 	0.4808),
    (0.0000, 	47.1154, 	40.8654),
    (4.8077, 	48.0769, 	95.6731),
    (43.2692, 	48.0769, 	0.4808),
    (43.2692, 	49.0385, 	41.8269),
    (43.2692, 	48.0769, 	96.6346),
    (95.1923, 	49.0385, 	5.2885),
    (95.1923, 	49.0385, 	41.8269),
    (98.0769, 	50.0000, 	97.5962),
    (0.9615, 	94.2308, 	0.4808),
    (0.0000, 	95.1923, 	41.8269),
    (5.7692, 	95.1923, 	94.7115),
    (44.2308, 	96.1538, 	1.4423),
    (44.2308, 	95.1923, 	41.8269),
    (44.2308, 	95.1923, 	95.6731),
    (95.1923, 	95.1923, 	5.2885),
    (95.1923, 	95.1923, 	42.7885),
    (100.000, 	95.1923, 	97.5962),
    (43.2692, 	48.0769, 	42.7885),
    (95.1923, 	0.9615, 	40.8654),
    (95.1923, 	95.1923, 	41.8269),
    (0.0000, 	0.9615, 	40.8654),
    (0.9615, 	95.1923, 	41.8269),
];

/// Content of the RMR register
struct RMR(u8);

#[derive(Clone,Copy,Debug,PartialEq)]
pub enum VideoMode {
    Mode0,
    Mode1,
    Mode2, // Pay attention to its 1 pixel shift
    Mode3
}

impl Default for VideoMode {
    fn default() -> Self {
        VideoMode::Mode0
    }
}

impl From<u8> for VideoMode {
    fn from(u: u8) -> Self {
        match u {
            0 => VideoMode::Mode0,
            1 => VideoMode::Mode1,
            2 => VideoMode::Mode2,
            3 => VideoMode::Mode3,
            _ => unreachable!()
        }
    }
}



trait IsAddressedBy {
    /// Check if the followgin code is addressed to the register
    fn is_addressed(u: u8) -> bool;
}

impl IsAddressedBy for PENR {
    fn is_addressed(u: u8) -> bool {
        (u & (0b11100000) == 0b000_00000) ||
        (u & (0b11100000) == 0b001_00000) 
    }
}

impl IsAddressedBy for INKR {
    fn is_addressed(u: u8) -> bool {
        (u & (0b111_00000) == 0b010_00000) ||
        (u & (0b111_00000) == 0b011_00000) 
    }
}

impl IsAddressedBy for RMR {
    fn is_addressed(u: u8) -> bool {
        (u & (0b111_00000) == 0b100_00000) ||
        (u & (0b111_00000) == 0b101_00000) 
    }
}

impl PENR {
    /// Get the encoded pen number
    fn pen(&self) -> Pen {
        if self.is_border() {
            Pen::Border
        }
        else {
            Pen::Pen(self.0 & 0b000_11111)
        }
    }

    fn is_border(&self) -> bool {
        self.0 & 0b000_10000 != 0
    }
}


impl INKR {
    fn ink(&self) -> Ink {
        Ink(self.0 & 0b000_11111)
    }
}


impl RMR {
    fn interrupt_counter(&self) -> bool {
        self.0 & 0b000_10000 != 0
    }

    fn upper_rom_pagging(&self) -> Flag {
        if self.0 & 0b000_01000 == 0 {
            Flag::Enable
        }
        else {
            Flag::Disable
        }
    }


    fn lower_rom_pagging(&self) -> Flag {
        if self.0 & 0b000_00100 == 0 {
            Flag::Enable
        }
        else {
            Flag::Disable
        }
    }

    fn video_mode(&self) -> VideoMode  {
        (self.0 & 0b000_00011).into()
    }
}

/// This is this strcuture that has the responsibility to produce the colors
pub enum PixelAutomata {
    AvailablePixel(u8, u8, VideoMode, [Ink;17]),
    NeedToReadMemory
}

impl Default for PixelAutomata {
    fn default() -> Self {
        PixelAutomata::NeedToReadMemory
    }
}

impl PixelAutomata {
    pub fn new(b: u8, m: VideoMode, p: [Ink;17]) -> Self {
        PixelAutomata::AvailablePixel(0, b, m, p)
    }

    pub fn is_done(&self) -> bool {
        match self {
            PixelAutomata::NeedToReadMemory => true,
            _ => false
        }
    }

    fn next_state(&mut self) {
        *self = match self {
            PixelAutomata::AvailablePixel(s, b, m, p) => {
                if *s == 7 {
                    PixelAutomata::NeedToReadMemory
                }
                else {
                    PixelAutomata::AvailablePixel(*s+1, *b, *m, *p)
                }
            },
            PixelAutomata::NeedToReadMemory => PixelAutomata::NeedToReadMemory,
        }
    }

    /// XXX Can only be called in a valid state;
    /// TODO manage the border
    pub fn generate_pixel(&mut self, hsync: bool, border: bool) -> (f32, f32, f32) {
        if hsync {
           return (0.0, 0.0, 0.0);
        }

        let pen = if border {
            Pen::Border
        }
        else {
            match self.mode() {
                Some(VideoMode::Mode0) => self.get_mode0_pen(),
                Some(VideoMode::Mode1) => self.get_mode1_pen(),
                Some(VideoMode::Mode2) => self.get_mode2_pen(),
                Some(VideoMode::Mode3) => self.get_mode3_pen(),
                None => unreachable!()
            }
        };
        
        let ink = self.palette().unwrap()[u8::from(pen) as usize];
        self.next_state();
        Self::rgb(ink)
    }

    pub fn get_mode0_pen(&mut self) -> Pen {
        match (self.step(), self.byte(), self.palette()) {
            (Some(step), Some(byte), Some(pal)) => {
                let pens = cpclib::pixels::mode0::byte_to_pens(byte);
                let step = step / 4; // The same pixel is generated 4 times
                match step {
                    0 => pens.0.into(),
                    1 => pens.1.into(,
                    _ => unreachable!()
                }
            },
            _ => unreachable!();
        }
    }
    pub fn get_mode1_pen(&mut self) -> Pen  {
        match (self.step(), self.byte(), self.palette()) {
            (Some(step), Some(byte), Some(pal)) => {
                let pens = cpclib::pixels::mode0::byte_to_pens(byte);
                let step = step / 2; // The same pixel is generated 2 times
                match step {
                    0 => pens.0.into(),
                    1 => pens.1.into(),
                    2 => pens.2.into(),
                    3 => pens.3.into(),
                    _ => unreachable!()
                }
            },
            _ => unreachable!();
        }
    }

    pub fn get_mode2_pen(&mut self) -> Pen {
        match (self.step(), self.byte(), self.palette()) {
            (Some(bit), Some(byte), Some(pal)) => {
                if byte & (1<<(7-bit)) != 0 {
                    Pen::Pen(0)
                }
                else {
                    Pen::Pen(0)
                }
            },
            _ => unreachable!()
        }
    }
    pub fn get_mode3_pen(&mut self)  -> Pen {
        let mode0_pen = self.get_mode0_pen();
<<<<<<< HEAD
        Pen::Pen(mode0_pen & 0b11)
=======
        unimplemented!()
>>>>>>> d66776e8bd10f0816dd51bfec418cc1d9bf58e17
    }

    pub fn step(&self) -> Option<u8> {
        match self {
            PixelAutomata::AvailablePixel(s, _, _, _) => Some(*s),
            PixelAutomata::NeedToReadMemory => None
        }
    }

    pub fn mode(&self) -> Option<VideoMode> {
        match self {
            PixelAutomata::AvailablePixel(_, _, m,  _) => Some(*m),
            PixelAutomata::NeedToReadMemory => None
        }
    }

    pub fn byte(&self) -> Option<u8> {
        match self {
            PixelAutomata::AvailablePixel(_, b, _, _) => Some(*b),
            PixelAutomata::NeedToReadMemory => None
        }
    }

    pub fn palette(&self) -> Option<&[Ink; 17]> {
        match self {
            PixelAutomata::AvailablePixel(_, _, _, p) => Some(p),
            PixelAutomata::NeedToReadMemory => None
        }
    }

    fn rgb(ink: Ink) -> (f32, f32, f32) {
        RGB_PERCENTAGE_LEVELS_PER_INK[ink.0 as usize]
    }

}


/// GateArray emulation of the CPC.
/// Version 40010 is targeted; it suffers of a shift in pixel rendering for mode 2
pub struct GateArray {
    clock: u64,
    pins: u64,

    rmr: RMR,
    penr: PENR,
    inkr: INKR,

    palette: [Ink;17],

    /// The video mode used to generate the pixels
    current_mode: VideoMode,
    /// The automata that generate the 8 pixel per byte
    pixel_automata: PixelAutomata,
    /// Lastly generated pixel
    rgb: (f32,f32, f32)
}

impl Default for GateArray {
    fn default()-> Self {
    GateArray {
        clock: 0,
        pins: 0,

        penr: PENR(0b000_00000),
        inkr: INKR(0b010_00000),
        rmr: RMR(0b111_00000),

        palette: [
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
            Ink(0),
        ],

        current_mode: VideoMode::Mode0,
        pixel_automata: PixelAutomata::default(),
        rgb: RGB_PERCENTAGE_LEVELS_PER_INK[0]
    }
}
}

impl DataChip for GateArray {
    fn pins_data() -> &'static [Self::PinType; 8] {
        return &GA_PIN_DATA_BUS;
    }
}

impl ReadDataChip for GateArray {}
impl WriteDataChip for GateArray {}

impl GateArray {
    /// Gate array works at 16Mhz and divide it to generate signals for other chips.
    /// CPU is at 4 Mhz
    pub fn manage_clocks(&mut self) {
        self.set_pin(GateArrayPin::CPU_ADDR, self.clock % 4 != 0);

        self.clock += 1;
    }

    /// get the active video mode
    pub fn active_video_mode(&self) -> VideoMode {
        self.current_mode
    }

    /// Get the video mode for the next line
    pub fn next_video_mode(&self) -> VideoMode {
        self.rmr.video_mode()
    }

    pub fn rgb(&self) -> (f32, f32, f32) {
        self.rgb
    }

    /// Set the video memory byte that serves the the next 8 RGB signal generation
    fn set_memory_byte(&mut self, b: u8) {
        assert!(self.pixel_automata.is_done());
        self.pixel_automata = PixelAutomata::AvailablePixel(
            0,
            b,
            self.active_video_mode(),
            self.palette.clone()
        );
    }

    /// Properly setup the pins for the video signal
    /// However, the R G B pins are not ON/OFF but make the intensity.
    /// For this reason, we do not use the pin mechanism as for the other components.
    /// Instead RGB values are stored in 3 floats.
    /// There are 1024 mode 2 pixels per line and 312 lines per frame.
    /// Here, we do not care of where we are; it is up to the monitor to properly place these pixels at the right place thanks to other synchronization messages
    /// We assume that the automata is in a valid state
    fn generate_pixel(&mut self) {
        assert!(!self.pixel_automata.is_done());

        if self.get_pin(GateArrayPin::HSYNC) {
            self.rgb = RGB_PERCENTAGE_LEVELS_PER_INK[0];
        }
        else {
            self.rgb = self.pixel_automata.generate_pixel(GateArrayPin::HSYNC);
        }

    }

    delegate! {
        // Delegate all methods EXCEPT video mode as it is bufferized
        to self.rmr {
            pub fn upper_rom_pagging(&self) -> Flag;
            pub fn lower_rom_pagging(&self) -> Flag;
            pub fn interrupt_counter(&self) -> bool;
        }

        to self.inkr {
            pub fn ink(&self) -> Ink;
        }

        to self.penr {
            pub fn pen(&self) -> Pen ;
            pub fn is_border(&self) -> bool ;
        }
    }

}

impl Chip for GateArray {
    type PinType = GateArrayPin;

    fn must_execute_tick(&self) -> bool {
        !self.get_pin(GateArrayPin::A15) && self.get_pin(GateArrayPin::A14)
    }

    /// Change the screen mode when hsync is set
    fn pre_tick(&mut self) {

        if self.get_pin(GateArrayPin::HSYNC) {
            self.current_mode = self.next_video_mode();
        }

        self.generate_pixel()
    }

    /// React to the port addressing of the CPU
    fn execute_tick(&mut self) {
        let command = self.get_data_from_pins();

        if PENR::is_addressed(command) {
            self.penr = PENR(command);
        }
        else if INKR::is_addressed(command) {
            // TODO pay attention to delaÿ the change of the color palette
            self.inkr = INKR(command);
        }
        else if RMR::is_addressed(command){
            self.rmr = RMR(command);
        }
    }

    fn pins_buffer_mut(&mut self) -> &mut u64 {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &u64 {
        &self.pins
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
        ga.set_memory_byte(0x0);
        ga.reset();

        ga.tick();
        ga.tick();
    }


    #[test]
    fn test_video_mode_consistency() {
        let mut ga = GateArray::default();
        ga.set_memory_byte(0x0);
        assert_eq!(
            ga.active_video_mode(),
            VideoMode::Mode0
        );

        ga.set_data_on_pins(0x8d); // Mode 1
        ga.set_pin(GateArrayPin::A15, false);
        ga.set_pin(GateArrayPin::A14, true);
        assert!(ga.must_execute_tick());
        ga.tick();

        assert_eq!(
            ga.active_video_mode(),
            VideoMode::Mode0
        );
        assert_eq!(
            ga.next_video_mode(),
            VideoMode::Mode1
        );
        assert_eq!(
            ga.lower_rom_pagging(),
            Flag::Disable
        );
        assert_eq!(
            ga.upper_rom_pagging(),
            Flag::Disable
        );
        ga.set_data_on_pins(0x8e); // Mode 2
        ga.set_pin(GateArrayPin::A15, false);
        ga.set_pin(GateArrayPin::A14, true);
        ga.set_pin(GateArrayPin::HSYNC, true);
        ga.tick();


        assert_eq!(
            ga.active_video_mode(),
            VideoMode::Mode1
        );
        assert_eq!(
            ga.next_video_mode(),
            VideoMode::Mode2
        );
        assert_eq!(
            ga.lower_rom_pagging(),
            Flag::Disable
        );
        assert_eq!(
            ga.upper_rom_pagging(),
            Flag::Disable
        );
    }

    #[test]
    fn test_mode2_pixels_generation() {
        let mut ga = GateArray::default();
        ga.current_mode = VideoMode::Mode2;
        ga.palette[1] = Ink(1);
        ga.set_memory_byte(0b01010101);


        ga.tick();
        let rgb = dbg!(ga.rgb());
        assert!((rgb.0 - 0.0).abs() < 0.0001);
        assert!((rgb.1 - 0.9615).abs() < 0.0001);
        assert!((rgb.2 - 0.4808).abs() < 0.0001);


        ga.tick();
        let rgb = dbg!(ga.rgb());
        assert!((rgb.0 - 0.0).abs() < 0.0001);
        assert!((rgb.1 - 0.9615).abs() < 0.0001);
        assert!((rgb.2 - 41.8269).abs() < 0.0001);

        ga.tick();
        ga.tick();
        ga.tick();
        ga.tick();
        ga.tick();
        ga.tick();

        assert!(ga.pixel_automata.is_done());
    }
}
