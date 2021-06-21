use crate::bits::*;
use crate::chip::*;
use crate::memory::Flag;
use array_init::*;
use delegate::delegate;
use lazy_static::*;
use tuple_map::*;

// https://www.cpc-power.com/cpcarchives/index.php?page=articles&num=47
// https://www.grimware.org/doku.php/documentations/devices/gatearray

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

    /*
    /// Synchronization for screen
    SYNC = 1 << 10,
    */
    /// Access to ROM
    ROMEN = 1 << 11,
    /// Request data send to CPU
    RAMRD = 1 << 12,
    /// Horizontal synchronisation from CRTC
    HSYNC = 1 << 13,
    /// Vertical synchronisation from CRTC
    VSYNC = 1 << 14,
    /// The CPU request an input/output (ignored if does not match)
    Neg_IORQ = 1 << 15,
    /// CPU received an interruption signal
    M1 = 1 << 16,
    /// CPU wants to access the memory
    MERQ = 1 << 17,
    /// CPU wants to read the memory
    Neg_RD = 1 << 18,

    A15 = 1 << 19,

    /// 1 Mhz for RAM refresh
    CCLK = 1 << 20,

    /// 4MHZ for CPU
    PHI = 1 << 21,
    VCC1 = 1 << 22,

    /// Reset of the GA
    RESET = 1 << 23,

    GND = 1 << 25,
    VCC3 = 1 << 27,

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

    NCPU = 1 << 40,
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
#[derive(Debug)]
pub struct PENR(u8);
#[derive(Debug, Copy, Clone)]
pub enum Pen {
    Pen(u8),
    Border,
}
impl Default for Pen {
    fn default() -> Self {
        Pen::Pen(0)
    }
}

impl From<u8> for Pen {
    fn from(b: u8) -> Pen {
        match b {
            0..16 => Pen::Pen(b),
            16 => Pen::Border,
            _ => unreachable!(),
        }
    }
}

impl From<cpclib::ga::Pen> for Pen {
    fn from(p: cpclib::ga::Pen) -> Pen {
        p.number().into()
    }
}

impl Into<u8> for &Pen {
    fn into(self) -> u8 {
        self.value()
    }
}

impl Into<u8> for Pen {
    fn into(self) -> u8 {
        self.value()
    }
}

impl Pen {
    pub fn value(&self) -> u8 {
        match self {
            Pen::Pen(nb) => *nb,
            Pen::Border => 16,
        }
    }
}

lazy_static! {
    /// LUT to retreive the pens in a given byte in mode 0
    static ref BYTE_TO_MODE0_PENS: [(Pen, Pen); 256] =
        array_init(|byte|{
            let pens = cpclib::pixels::mode0::byte_to_pens(byte as u8);
            pens.map(|p| Pen::from(p))
        });

    /// LUT to retreive the pens in a given byte in mode 1
    static ref BYTE_TO_MODE1_PENS: [(Pen, Pen, Pen, Pen); 256] =
    array_init(|byte|{
        let pens = cpclib::pixels::mode1::byte_to_pens(byte as u8);
        pens.map(|p| Pen::from(p))
    });

    /// LUT to retreive the pens in a given byte in mode 2
    static ref BYTE_TO_MODE2_PENS: [(Pen, Pen, Pen, Pen, Pen, Pen, Pen, Pen); 256] =
    array_init(|byte|{
        let pens = cpclib::pixels::mode2::byte_to_pens(byte as u8);
        pens.map(|p| Pen::from(p))
    });
}

/// Content of the INKR register
#[derive(Debug)]
pub struct INKR(u8);
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Ink(u8);

impl From<u8> for Ink {
    fn from(b: u8) -> Ink {
        assert!(b < 27);
        Ink(b)
    }
}

impl From<cpclib::ga::Ink> for Ink {
    fn from(i: cpclib::ga::Ink) -> Ink {
        Ink(i.number())
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RgbLevels((f32, f32, f32));
impl RgbLevels {
    pub fn r(&self) -> f32 {
        self.0 .0
    }
    pub fn g(&self) -> f32 {
        self.0 .1
    }
    pub fn b(&self) -> f32 {
        self.0 .2
    }
}

impl From<(f32, f32, f32)> for RgbLevels {
    fn from(rgb: (f32, f32, f32)) -> Self {
        RgbLevels(rgb)
    }
}

impl From<Ink> for RgbLevels {
    fn from(ink: Ink) -> Self {
        ink.to_rgb_levels()
    }
}

// https://www.grimware.org/doku.php/documentations/devices/gatearray
const RGB_PERCENTAGE_LEVELS_PER_INK: [RgbLevels; 32] = [
    RgbLevels((0.0000, 0.9615, 0.4808)),
    RgbLevels((0.0000, 0.9615, 41.8269)),
    RgbLevels((4.8077, 0.9615, 95.6731)),
    RgbLevels((42.3077, 0.9615, 0.4808)),
    RgbLevels((41.3462, 0.9615, 40.8654)),
    RgbLevels((42.3077, 0.9615, 94.7115)),
    RgbLevels((95.1923, 1.9231, 2.4038)),
    RgbLevels((94.2308, 0.9615, 40.8654)),
    RgbLevels((95.1923, 0.9615, 95.6731)),
    RgbLevels((0.9615, 47.1154, 0.4808)),
    RgbLevels((0.0000, 47.1154, 40.8654)),
    RgbLevels((4.8077, 48.0769, 95.6731)),
    RgbLevels((43.2692, 48.0769, 0.4808)),
    RgbLevels((43.2692, 49.0385, 41.8269)),
    RgbLevels((43.2692, 48.0769, 96.6346)),
    RgbLevels((95.1923, 49.0385, 5.2885)),
    RgbLevels((95.1923, 49.0385, 41.8269)),
    RgbLevels((98.0769, 50.0000, 97.5962)),
    RgbLevels((0.9615, 94.2308, 0.4808)),
    RgbLevels((0.0000, 95.1923, 41.8269)),
    RgbLevels((5.7692, 95.1923, 94.7115)),
    RgbLevels((44.2308, 96.1538, 1.4423)),
    RgbLevels((44.2308, 95.1923, 41.8269)),
    RgbLevels((44.2308, 95.1923, 95.6731)),
    RgbLevels((95.1923, 95.1923, 5.2885)),
    RgbLevels((95.1923, 95.1923, 42.7885)),
    RgbLevels((100.000, 95.1923, 97.5962)),
    RgbLevels((43.2692, 48.0769, 42.7885)),
    RgbLevels((95.1923, 0.9615, 40.8654)),
    RgbLevels((95.1923, 95.1923, 41.8269)),
    RgbLevels((0.0000, 0.9615, 40.8654)),
    RgbLevels((0.9615, 95.1923, 41.8269)),
];

/// Specific trait to allow ink to provide their RGB voltage level
pub trait ToRGBLevels {
    /// Returne the RGB levels for the given ink
    fn to_rgb_levels(&self) -> RgbLevels;
}

impl ToRGBLevels for Ink {
    fn to_rgb_levels(&self) -> RgbLevels {
        RGB_PERCENTAGE_LEVELS_PER_INK[self.0 as usize]
    }
}

/// Content of the RMR register
#[derive(Debug)]
pub struct RMR(u8);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VideoMode {
    Mode0,
    Mode1,
    Mode2, // Pay attention to its 1 pixel shift
    Mode3,
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
            _ => unreachable!(),
        }
    }
}

trait IsAddressedBy {
    /// Check if the followgin code is addressed to the register
    fn is_addressed(u: u8) -> bool;
}

impl IsAddressedBy for PENR {
    fn is_addressed(u: u8) -> bool {
        (u & (0b11100000) == 0b000_00000) || (u & (0b11100000) == 0b001_00000)
    }
}

impl IsAddressedBy for INKR {
    fn is_addressed(u: u8) -> bool {
        (u & (0b111_00000) == 0b010_00000) || (u & (0b111_00000) == 0b011_00000)
    }
}

impl IsAddressedBy for RMR {
    fn is_addressed(u: u8) -> bool {
        (u & (0b111_00000) == 0b100_00000) || (u & (0b111_00000) == 0b101_00000)
    }
}

impl PENR {
    /// Get the encoded pen number
    fn pen(&self) -> Pen {
        if self.is_border() {
            Pen::Border
        } else {
            Pen::Pen(self.0 & 0b000_11111)
        }
    }

    fn is_border(&self) -> bool {
        self.0 & 0b000_10000 != 0
    }
}

impl INKR {
    fn ink(&self) -> Ink {
        let hard = self.0 & 0b000_11111;
        cpclib::ga::Ink::from_hardware_color_number(hard).into()
    }
}

impl RMR {
    fn interrupt_counter(&self) -> bool {
        self.0 & 0b000_10000 != 0
    }

    fn upper_rom_pagging(&self) -> Flag {
        if self.0 & 0b000_01000 == 0 {
            Flag::Enable
        } else {
            Flag::Disable
        }
    }

    fn force_upper_rom_pagging(&mut self, flag: Flag) {
        let val = self.0 & 0b111_10111;
        self.0 = if flag.is_enabled() {
            val
        } else {
            val | 0b000_01000
        };
    }

    fn lower_rom_pagging(&self) -> Flag {
        if self.0 & 0b000_00100 == 0 {
            Flag::Enable
        } else {
            Flag::Disable
        }
    }

    fn force_lower_rom_pagging(&mut self, flag: Flag) {
        let val = self.0 & 0b111_11011;
        self.0 = if flag.is_enabled() {
            val
        } else {
            val | 0b000_00100
        };
    }

    fn video_mode(&self) -> VideoMode {
        (self.0 & 0b000_00011).into()
    }

    fn set_video_mode(&mut self, mode: VideoMode) {
        let b = mode as u8;
        self.0 = (self.0 & 0b111_11100) | b;
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum PixelOrder {
    Pixel1,
    Pixel2,
    Pixel3,
    Pixel4,
    Pixel5,
    Pixel6,
    Pixel7,
    Pixel8,
}

impl PixelOrder {
    fn next(self) -> Self {
        match self {
            PixelOrder::Pixel1 => PixelOrder::Pixel2,
            PixelOrder::Pixel2 => PixelOrder::Pixel3,
            PixelOrder::Pixel3 => PixelOrder::Pixel4,
            PixelOrder::Pixel4 => PixelOrder::Pixel5,
            PixelOrder::Pixel5 => PixelOrder::Pixel6,
            PixelOrder::Pixel6 => PixelOrder::Pixel7,
            PixelOrder::Pixel7 => PixelOrder::Pixel8,
            PixelOrder::Pixel8 => PixelOrder::Pixel1,
        }
    }

    fn is_last(&self) -> bool {
        match self {
            PixelOrder::Pixel8 => true,
            _ => false,
        }
    }
}

/// This is this strcuture that has the responsibility to produce the colors
#[derive(Debug)]
pub enum PixelAutomata {
    AvailablePixel(PixelOrder, u8, VideoMode, [Ink; 17]),
    NeedToReadMemory,
}

impl Default for PixelAutomata {
    fn default() -> Self {
        PixelAutomata::NeedToReadMemory
    }
}

impl PixelAutomata {
    /// The new automaton depends on the byte to display, its video mode and the palette
    pub fn new(b: u8, m: VideoMode, p: [Ink; 17]) -> Self {
        PixelAutomata::AvailablePixel(PixelOrder::Pixel1, b, m, p)
    }

    /// Automaton is over when pixel 8 is produced
    pub fn is_done(&self) -> bool {
        match self {
            PixelAutomata::NeedToReadMemory => true,
            _ => false,
        }
    }

    /// Select the next step (no computation is involved there)
    fn next_state(&mut self) {
        *self = match self {
            PixelAutomata::AvailablePixel(s, b, m, p) => {
                if s.is_last() {
                    PixelAutomata::NeedToReadMemory
                } else {
                    PixelAutomata::AvailablePixel(s.next(), *b, *m, *p)
                }
            }
            PixelAutomata::NeedToReadMemory => PixelAutomata::NeedToReadMemory,
        }
    }

    /// XXX Can only be called in a valid state;
    pub fn generate_pixel(&mut self, border: bool) -> RgbLevels {
        let pen = if border {
            Pen::Border
        } else {
            match self.mode() {
                Some(VideoMode::Mode0) => self.get_mode0_pen(),
                Some(VideoMode::Mode1) => self.get_mode1_pen(),
                Some(VideoMode::Mode2) => self.get_mode2_pen(),
                Some(VideoMode::Mode3) => self.get_mode3_pen(),
                None => unreachable!(),
            }
        };

        let ink = self.palette().unwrap()[pen.value() as usize];
        self.next_state();
        ink.into()
    }

    pub fn get_mode0_pen(&mut self) -> Pen {
        match (self.step(), self.byte(), self.palette()) {
            (Some(step), Some(byte), Some(pal)) => {
                let pens = BYTE_TO_MODE0_PENS[byte as usize];
                match step {
                    PixelOrder::Pixel1
                    | PixelOrder::Pixel2
                    | PixelOrder::Pixel3
                    | PixelOrder::Pixel4 => pens.0,
                    PixelOrder::Pixel5
                    | PixelOrder::Pixel6
                    | PixelOrder::Pixel7
                    | PixelOrder::Pixel8 => pens.1,
                }
            }
            _ => unreachable!(),
        }
    }
    pub fn get_mode1_pen(&mut self) -> Pen {
        match (self.step(), self.byte(), self.palette()) {
            (Some(step), Some(byte), Some(pal)) => {
                let pens = BYTE_TO_MODE1_PENS[byte as usize];
                match step {
                    PixelOrder::Pixel1 | PixelOrder::Pixel2 => pens.0,
                    PixelOrder::Pixel3 | PixelOrder::Pixel4 => pens.1,
                    PixelOrder::Pixel5 | PixelOrder::Pixel6 => pens.2,
                    PixelOrder::Pixel7 | PixelOrder::Pixel8 => pens.3,
                }
            }
            _ => unreachable!(),
        }
    }

    pub fn get_mode2_pen(&mut self) -> Pen {
        match (self.step(), self.byte(), self.palette()) {
            (Some(step), Some(byte), Some(pal)) => {
                let pens = BYTE_TO_MODE2_PENS[byte as usize];
                match step {
                    PixelOrder::Pixel1 => pens.0,
                    PixelOrder::Pixel2 => pens.1,
                    PixelOrder::Pixel3 => pens.2,
                    PixelOrder::Pixel4 => pens.3,
                    PixelOrder::Pixel5 => pens.4,
                    PixelOrder::Pixel6 => pens.5,
                    PixelOrder::Pixel7 => pens.6,
                    PixelOrder::Pixel8 => pens.7,
                }
            }
            _ => unreachable!(),
        }
    }
    pub fn get_mode3_pen(&mut self) -> Pen {
        let mode0_pen = self.get_mode0_pen();
        (mode0_pen.value() & 0b11).into()
    }

    pub fn step(&self) -> Option<&PixelOrder> {
        match self {
            PixelAutomata::AvailablePixel(s, _, _, _) => Some(s),
            PixelAutomata::NeedToReadMemory => None,
        }
    }

    pub fn mode(&self) -> Option<VideoMode> {
        match self {
            PixelAutomata::AvailablePixel(_, _, m, _) => Some(*m),
            PixelAutomata::NeedToReadMemory => None,
        }
    }

    pub fn byte(&self) -> Option<u8> {
        match self {
            PixelAutomata::AvailablePixel(_, b, _, _) => Some(*b),
            PixelAutomata::NeedToReadMemory => None,
        }
    }

    /// Return the palette needed to display the byte
    pub fn palette(&self) -> Option<&[Ink; 17]> {
        match self {
            PixelAutomata::AvailablePixel(_, _, _, p) => Some(p),
            PixelAutomata::NeedToReadMemory => None,
        }
    }
}

/// Synchronization information comes from a specific pin SYNC. I guess it contains (how?) both vsync and hsync information. Anyway, CTM has no access at all to VSYNC and HSYNC of CRTC
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Sync {
    pub hsync: bool,
    pub vsync: bool,
}

/// Handle the vsync information to generate interrupt
/// two lines after vsync
#[derive(Debug)]
pub enum VSyncCounter {
    NoVsync,
    VSync(usize),
}

impl Default for VSyncCounter {
    fn default() -> Self {
        VSyncCounter::NoVsync
    }
}

/// Handle the interruption generation
#[derive(Debug)]
pub enum InterruptionSignal {
    NoSignal,
    Signal(usize),
}

/// GateArray emulation of the CPC.
/// Version 40010 is targeted; it suffers of a shift in pixel rendering for mode 2
/// TODO ensure the following sentence is applied
/// The effect of an INKR command appears almost immediatly always with the same timings, but the change is not in sync with the character display of the CRTC so the new color appears somehow in the middle of a character (the CRTC display unit).
#[derive(Debug)]
pub struct GateArray {
    clock: u64,
    pins: PinsBuffer,

    vsync_counter: VSyncCounter,
    inter_sign: InterruptionSignal,

    rmr: RMR,
    penr: PENR,
    inkr: INKR,

    palette: [Ink; 17],

    /// The video mode used to generate the pixels
    current_mode: VideoMode,
    /// The automata that generate the 8 pixel per byte
    pixel_automata: PixelAutomata,

    /// Lastly generated pixel (to be retreived for monitor)
    rgb: RgbLevels,
    /// Synchronization information for the monitor
    sync: Sync,

    /// hsync counter
    r52: Bit8Max6,
}

impl Default for GateArray {
    fn default() -> Self {
        let mut ga = GateArray {
            clock: 0,
            pins: (0, 0).into(),

            vsync_counter: VSyncCounter::NoVsync,
            inter_sign: InterruptionSignal::NoSignal,

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
            rgb: RGB_PERCENTAGE_LEVELS_PER_INK[0],
            sync: Sync {
                vsync: false,
                hsync: false,
            },

            r52: Default::default(),
        };
        ga.divide_clock();
        ga
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
    pub fn palette(&self) -> &[Ink; 17] {
        &self.palette
    }
    /// Gate array works at 16Mhz and divide it to generate signals for other chips.
    /// CPU is at 4 Mhz
    pub fn manage_clocks(&mut self) {
        self.clock = self.clock.wrapping_add(1);
        self.divide_clock();
    }

    pub fn divide_clock(&mut self) {
        self.set_pin(GateArrayPin::NCPU, ((self.clock + 0) % 4) >= 2); // this delay is needed to let time to CRTC to get cpu infos from pins
        self.set_pin(GateArrayPin::CCLK, !(((self.clock + 1) % 16) >= 8));
    }

    /// get the active video mode
    pub fn active_video_mode(&self) -> VideoMode {
        self.current_mode
    }

    /// Force the active video mode (test purposes)
    pub fn force_active_video_mode(&mut self, mode: VideoMode) {
        self.current_mode = mode;
    }

    /// Get the video mode for the next line
    pub fn next_video_mode(&self) -> VideoMode {
        self.rmr.video_mode()
    }

    /// Return the RGB level
    pub fn rgb(&self) -> RgbLevels {
        self.rgb
    }

    /// Return the sync info generated by GA
    pub fn sync(&self) -> Sync {
        self.sync
    }

    pub fn need_memory_byte(&self) -> bool {
        self.pixel_automata.is_done()
    }

    /// Set the video memory byte that serves the the next 8 RGB signal generation
    pub fn set_memory_byte(&mut self, b: u8) {
        // assert!(self.pixel_automata.is_done()); may be false in tests
        self.pixel_automata = PixelAutomata::AvailablePixel(
            PixelOrder::Pixel1,
            b,
            self.active_video_mode(),
            self.palette.clone(),
        );
    }

    pub fn set_ink(&mut self, pen: Pen, ink: Ink) {
        self.palette[pen.value() as usize] = ink;
    }

    pub fn set_palette(&mut self, palette: [Ink; 17]) {
        self.palette = palette;
    }

    pub fn set_video_mode(&mut self, mode: VideoMode) {
        self.rmr.set_video_mode(mode);
    }

    /// Properly setup the pins for the video signal
    /// However, the R G B pins are not ON/OFF but make the intensity.
    /// For this reason, we do not use the pin mechanism as for the other components.
    /// Instead RGB values are stored in 3 floats.
    /// There are 1024 mode 2 pixels per line and 312 lines per frame.
    /// Here, we do not care of where we are; it is up to the monitor to properly place these pixels at the right place thanks to other synchronization messages
    /// We assume that the automata is in a valid state
    fn generate_pixel(&mut self) {
        if self.pixel_automata.is_done() {
            //     dbg!("Here some pixel are expected, but we do not know what as to be produced...");
            self.rgb = (0., 0., 0.).into();
        } else {
            if self.get_pin(GateArrayPin::HSYNC) || self.get_pin(GateArrayPin::VSYNC) {
                self.rgb = (0., 0., 0.).into();
            } else {
                self.rgb = self
                    .pixel_automata
                    .generate_pixel(!self.get_pin(GateArrayPin::DISPEN));
            }
        }
    }

    /// Generate the synchronization data for the CTM
    /// TODO understand/find how it is really done
    fn generate_sync(&mut self) {
        self.sync = Sync {
            vsync: self.get_pin(GateArrayPin::VSYNC),
            hsync: self.get_pin(GateArrayPin::HSYNC),
        }
    }

    /// When the hsync ends, the GA update video mode and manage the interruption counter.
    /// it also manage a counter for the interuptions and generate one when it reaches 52
    /// TODO check who disable the interrupt (GA or z80?)
    fn manage_hsync_end(&mut self) {
        self.current_mode = self.next_video_mode();
        self.r52.inc();
        if self.r52 == 52 {
            self.r52.reset();
            self.request_interrupt();
        } else {
            match self.vsync_counter {
                VSyncCounter::VSync(ref mut count) => {
                    *count += 1;
                    if *count == 2 {
                        // 2 lines after vsync counter is reseted.
                        if self.r52.bit(5) {
                            self.r52.reset();
                        } else {
                            // and an interruption could occur
                            self.r52.reset();
                            self.request_interrupt();
                        }
                    }
                }
                _ => {
                    // nothing to do when there is no vsync
                }
            }
        }
    }

    fn request_interrupt(&mut self) {
        self.inter_sign = InterruptionSignal::Signal(0);
        self.set_pin(GateArrayPin::INTERRUPT, false); //activate when low
    }
    delegate! {
        // Delegate all methods EXCEPT video mode as it is bufferized
        to self.rmr {
            pub fn upper_rom_pagging(&self) -> Flag;
            pub fn lower_rom_pagging(&self) -> Flag;

            pub fn force_upper_rom_pagging(&mut self, flag: Flag);
            pub fn force_lower_rom_pagging(&mut self, flag: Flag);

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

/// Accessors
impl GateArray {
    pub fn current_mode(&self) -> VideoMode {
        self.current_mode
    }

    pub fn r52(&self) -> u8 {
        *self.r52.deref()
    }
}
impl Chip for GateArray {
    type PinType = GateArrayPin;

    fn must_execute_tick(&self) -> bool {
        !self.get_pin(GateArrayPin::A15)
            && self.get_pin(GateArrayPin::A14)
            && !self.get_pin(GateArrayPin::Neg_IORQ)
    }

    /// Change the screen mode when hsync is set
    fn pre_tick(&mut self) {
        if self.is_upfront(GateArrayPin::VSYNC) {
            //vsync occured
            self.vsync_counter = VSyncCounter::VSync(0);
        }
        if self.is_downfront(GateArrayPin::HSYNC) {
            // hsync occured
            self.manage_hsync_end();
        }
        if let InterruptionSignal::Signal(ref mut count) = self.inter_sign {
            // cut the signal after 1.4 micro seconds
            *count += 1;
            if *count == (1.4 * 32f64 / 2f64) as usize {
                self.inter_sign = InterruptionSignal::NoSignal;
                self.set_pin(GateArrayPin::INTERRUPT, true);
            }
        }

        self.generate_pixel();
        self.generate_sync();
    }

    /// React to the port addressing of the CPU
    fn execute_tick(&mut self) {
        if self.get_pin(GateArrayPin::Neg_RD) {
            // out expected
            let command = dbg!(self.get_data_from_pins());

            if PENR::is_addressed(command) {
                self.penr = PENR(command);
            } else if INKR::is_addressed(command) {
                self.inkr = INKR(command);
                self.set_ink(self.pen(), self.ink());
            } else if RMR::is_addressed(command) {
                self.rmr = RMR(command);
                if self.interrupt_counter() {
                    self.r52.reset();
                }
            }
        } else {
            // read expected
            unimplemented!("Read on GA not implemented");
        }
    }

    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.pins
    }
    fn pins_buffer(&self) -> &PinsBuffer {
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
    fn test_clock() {
        let mut ga = GateArray::default();

        for i in 0..16 {
            println!(
                "{} {} {}",
                ga.clock,
                ga.get_pin(GateArrayPin::CCLK),
                ga.get_pin(GateArrayPin::NCPU)
            );

            if i == 0 {
                assert_eq!(ga.get_pin(GateArrayPin::CCLK), false);
                assert_eq!(ga.need_memory_byte(), true);
            }

            if i < 8 {
                assert_eq!(ga.get_pin(GateArrayPin::CCLK), false);
            }
            if i == 8 {
                assert_eq!(ga.get_pin(GateArrayPin::CCLK), true);
            }

            ga.manage_clocks();
        }
    }

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
        assert_eq!(ga.active_video_mode(), VideoMode::Mode0);

        ga.set_data_on_pins(0x8d); // Mode 1
        ga.set_pin(GateArrayPin::A15, false);
        ga.set_pin(GateArrayPin::A14, true);
        assert!(ga.must_execute_tick());
        ga.tick();

        assert_eq!(ga.active_video_mode(), VideoMode::Mode0);
        assert_eq!(ga.next_video_mode(), VideoMode::Mode1);
        assert_eq!(ga.lower_rom_pagging(), Flag::Disable);
        assert_eq!(ga.upper_rom_pagging(), Flag::Disable);
        ga.set_data_on_pins(0x8e); // Mode 2
        ga.set_pin(GateArrayPin::A15, false);
        ga.set_pin(GateArrayPin::A14, true);
        ga.set_pin(GateArrayPin::HSYNC, true);
        ga.save_pins_state();
        ga.set_pin(GateArrayPin::HSYNC, false);
        ga.tick();

        assert_eq!(ga.active_video_mode(), VideoMode::Mode1);
        assert_eq!(ga.next_video_mode(), VideoMode::Mode2);
        assert_eq!(ga.lower_rom_pagging(), Flag::Disable);
        assert_eq!(ga.upper_rom_pagging(), Flag::Disable);
    }

    #[test]
    fn test_mode2_pixels_generation() {
        let mut ga = GateArray::default();
        ga.set_pin(GateArrayPin::DISPEN, true);
        ga.current_mode = VideoMode::Mode2;
        ga.force_active_video_mode(VideoMode::Mode2);
        ga.palette[1] = Ink(1);
        let byte = 0b01010101;

        ga.set_memory_byte(byte);
        let pens = dbg!(BYTE_TO_MODE2_PENS[byte as usize]);

        assert_eq!(ga.pixel_automata.step().cloned(), Some(PixelOrder::Pixel1));
        assert_eq!(ga.pixel_automata.mode(), Some(VideoMode::Mode2));
        ga.tick();
        let rgb = ga.rgb();
        assert!((rgb.r() - 0.0).abs() < 0.0001);
        assert!((rgb.g() - 0.9615).abs() < 0.0001);
        assert!((rgb.b() - 0.4808).abs() < 0.0001);

        dbg!(&ga.pixel_automata);
        assert_eq!(ga.pixel_automata.step().cloned(), Some(PixelOrder::Pixel2));
        ga.tick();
        let rgb = dbg!(ga.rgb());
        assert!((rgb.r() - 0.0).abs() < 0.0001);
        assert!((rgb.g() - 0.9615).abs() < 0.0001);
        assert!((rgb.b() - 41.8269).abs() < 0.0001);

        ga.tick();
        ga.tick();
        ga.tick();
        ga.tick();
        ga.tick();
        ga.tick();

        assert!(ga.pixel_automata.is_done());
    }

    #[test]
    /// We want to check that pixels are generated in black
    fn test_pixels_1() {
        let mode = VideoMode::Mode1;
        let palette: [Ink; 17] = [
            0.into(),
            8.into(),
            13.into(),
            18.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
            0.into(),
        ];
        let byte = 0b10001000;

        let mut ga = GateArray::default();
        ga.set_palette(palette);
        ga.set_video_mode(mode);
        ga.force_active_video_mode(mode);
        ga.set_pin(GateArrayPin::HSYNC, false); // no vsync
        ga.set_pin(GateArrayPin::VSYNC, false); // no hsync
        ga.set_pin(GateArrayPin::DISPEN, true); // no border

        /*
                assert_eq!(ga.active_video_mode(), VideoMode::Mode1);
                ga.set_memory_byte(0);
                check_expected_inks(&mut ga, &[
                    Ink::from(0),
                    Ink::from(0),
                    Ink::from(0),
                    Ink::from(0),
                    Ink::from(0),
                    Ink::from(0),
                    Ink::from(0),
                    Ink::from(0),
                ]);
        */

        ga.set_memory_byte(byte);
        assert_eq!(VideoMode::Mode1, ga.pixel_automata.mode().unwrap());
        let pens = BYTE_TO_MODE1_PENS[byte as usize];
        println!("List of pens to use {:?}", &pens);

        let automata_palette = ga.pixel_automata.palette().unwrap();
        assert_eq!(automata_palette, &palette);
        let inks = [
            automata_palette[pens.0.value() as usize],
            automata_palette[pens.1.value() as usize],
            automata_palette[pens.2.value() as usize],
            automata_palette[pens.3.value() as usize],
        ];
        println!("List of inks to use. {:?}", inks);

        let values = [
            inks[0].to_rgb_levels(),
            inks[1].to_rgb_levels(),
            inks[2].to_rgb_levels(),
            inks[3].to_rgb_levels(),
        ];
        println!("List of levels to generate. {:?}", values);

        check_expected_inks(
            &mut ga,
            &[
                Ink::from(18).to_rgb_levels(),
                Ink::from(18).to_rgb_levels(),
                Ink::from(0).to_rgb_levels(),
                Ink::from(0).to_rgb_levels(),
                Ink::from(0).to_rgb_levels(),
                Ink::from(0).to_rgb_levels(),
                Ink::from(0).to_rgb_levels(),
                Ink::from(0).to_rgb_levels(),
            ],
        );
    }

    fn check_expected_inks(ga: &mut GateArray, expected: &[RgbLevels; 8]) {
        assert!(!ga.pixel_automata.is_done());
        let obtained = (0..8)
            .into_iter()
            .map(|i| {
                ga.tick();
                let rgb = ga.rgb/*()*/; // not yet on the pins
                println!("{}, {:?}, {:?}", i, ga.pixel_automata, rgb);
                rgb
            })
            .collect::<Vec<RgbLevels>>();

        for i in 0..8 {
            println!("{}", i);
            println!("Expected: {:?}", expected[i]);
            println!("Obtained: {:?}", obtained[i]);
        }

        for i in 0..8 {
            println!("pixel {}", i);
            assert_eq!(expected[i], obtained[i]);
        }
    }
}
