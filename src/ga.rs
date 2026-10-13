use std::ops::BitAndAssign;

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
    Neg_M1 = 1 << 16,
    /// CPU wants to access the memory
    Neg_MREQ = 1 << 17,
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

impl From<cpclib::image::ga::Pen> for Pen {
    fn from(p: cpclib::image::ga::Pen) -> Pen {
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
            let pens = cpclib::image::pixels::mode0::byte_to_pens(byte as u8);
            (Pen::from(pens[0]), Pen::from(pens[1]))
        });

    /// LUT to retreive the pens in a given byte in mode 1
    static ref BYTE_TO_MODE1_PENS: [(Pen, Pen, Pen, Pen); 256] =
    array_init(|byte|{
        let pens = cpclib::image::pixels::mode1::byte_to_pens(byte as u8);
        (
            Pen::from(pens[0]),
            Pen::from(pens[1]),
            Pen::from(pens[2]),
            Pen::from(pens[3]),
        )
    });

    /// LUT to retreive the pens in a given byte in mode 2
    static ref BYTE_TO_MODE2_PENS: [(Pen, Pen, Pen, Pen, Pen, Pen, Pen, Pen); 256] =
    array_init(|byte|{
        let pens = cpclib::image::pixels::mode2::byte_to_pens(byte as u8);
        (
            Pen::from(pens[0]),
            Pen::from(pens[1]),
            Pen::from(pens[2]),
            Pen::from(pens[3]),
            Pen::from(pens[4]),
            Pen::from(pens[5]),
            Pen::from(pens[6]),
            Pen::from(pens[7]),
        )
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

impl From<cpclib::image::ga::Ink> for Ink {
    fn from(i: cpclib::image::ga::Ink) -> Ink {
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
        // The gate array only wires 27 distinct colours to its 5-bit ink
        // selector; the 5 remaining codes are documented hardware
        // duplicates of an existing colour rather than new ones (the CPC's own
        // ROM ink table agrees): raw codes 1, 8, 9, 16, 17 give firmware inks
        // 13, 7, 25, 1, 19, the same as raw codes 0, 5, 3, 4, 2 respectively.
        // `Ink::from_hardware_color_number` only knows the 27 primary codes,
        // so the duplicates are resolved to their primary code here first.
        const DUPLICATES: [(u8, u8); 5] = [(1, 0), (8, 5), (9, 3), (16, 4), (17, 2)];
        let hard = self.0 & 0b000_11111;
        let hard = DUPLICATES.iter().find(|(dup, _)| *dup == hard).map_or(hard, |(_, primary)| *primary);
        cpclib::image::ga::Ink::from_hardware_color_number(hard).into()
    }
}

/// The Gate Array reacts to the CRTC's HSYNC this many pixel-M2 ticks late
/// (display blanking, HSYNC end processing). With it the display resumes at
/// the 4th pixel-M2 of a byte, as the ACCC describes (9.3.4.3.1 and
/// 9.3.4.3.3, CRTC 0/1/2: "display resumes from the 4th Pixel-M2 of the 5th
/// VRAM byte"; the mode itself changes one pixel after the display is
/// restored).
pub const HSYNC_GA_DELAY_TICKS: usize = 2;

/// Shortest HSYNC (in GA ticks, 16 per microsecond) that updates the video
/// mode: ACCC 9.3.1, "a HSYNC of at least 2 usec is required for the graphic
/// mode update" (R3 = 1 never changes the mode). The GA counts whole
/// characters: an R2.JIT HSYNC of R3=2 starts part-way through a character
/// and is a little shorter than 2 usec on the pin, yet updates the mode
/// (Shaker AR), while R3=1 is exactly one character long.
pub const MODE_UPDATE_MIN_HSYNC_TICKS: u64 = 17;

/// The latch at the end of a short HSYNC (R3 below the 6 usec cap) misses the
/// mode writes of the last this many ticks: the Gate Array's own period is
/// "very slightly less than 2 usec" for R3=2 (ACCC 9.3.1), a little before the
/// CRTC's HSYNC ends. HBL's multi-mode lines write `OUT mode` 6 to 8 ticks
/// before that end and the real machine still latches the previous mode,
/// while Shaker AE (cap path, R3=14) puts the limit exactly on the cap.
pub const MODE_WRITE_DELAY_TICKS: usize = 8;

/// The Gate Array takes the video mode when its HSYNC-GA period ends, which is
/// R3 usec after the HSYNC opens but "capped at 6 usec" (ACCC 9.3.1; the
/// chronogram of 9.3.2, R2=46/R3=14, has the last `OUT` landing on C0=51 win,
/// 6 usec after C0=46). Shaker AE agrees (see `latch_mode`).
pub const MODE_LATCH_CAP_TICKS: u64 = 6 * 16;

/// The acknowledge of an interrupt is heard this much later when the
/// interrupted instruction ends with at most one idle T-state.
pub const ACK_LATE_TICKS: u32 = 16;

/// The INT line rises 1 usec (16 ticks) after the CRTC's HSYNC ends, ACCC
/// 27.6.1; the GA already hears of that end `HSYNC_GA_DELAY_TICKS` late.
pub const INTERRUPT_AFTER_HSYNC_TICKS: u32 = 16 - HSYNC_GA_DELAY_TICKS as u32;

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

    /// How many pixels of this byte have already been generated before
    /// this step (0 for `Pixel1`, ..., 7 for `Pixel8`). Used to find the
    /// GATE ARRAY's own raw bit-serial position when a mode switch lands
    /// mid-byte (see [`PixelAutomata::reconstruct_for_mode_switch`]).
    fn already_generated(&self) -> u8 {
        match self {
            PixelOrder::Pixel1 => 0,
            PixelOrder::Pixel2 => 1,
            PixelOrder::Pixel3 => 2,
            PixelOrder::Pixel4 => 3,
            PixelOrder::Pixel5 => 4,
            PixelOrder::Pixel6 => 5,
            PixelOrder::Pixel7 => 6,
            PixelOrder::Pixel8 => 7,
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

    /// Reconstruct this automaton in place for a video-mode switch that
    /// lands while it's still mid-byte (ACCC §9.3.4.3): the GATE ARRAY
    /// can't "un-display" whatever pixels it already generated of this
    /// byte under the old mode, but the pixels it *hasn't* generated yet
    /// must now come from the new mode instead - and any bit already
    /// consumed by the old mode is gone, so the new mode reconstructs its
    /// remaining pixel(s) from whatever's left, with "missed" bit
    /// positions padded by a value that depends on which GATE ARRAY chip
    /// is fitted (`GateArrayType::missed_bit_pad`).
    ///
    /// **Only implemented when the OLD mode is MODE 2.** Verified
    /// against the ACCC's own worked tables for both chip families (its
    /// MODE 2 -> MODE 0 tables match exactly once GA40007/8's one
    /// Pixel-M2 "lead" - `GateArrayType::reconstruction_lead` - is
    /// applied). MODE 0 and MODE 3 as the *old* mode never need this at
    /// all: their own ACCC tables show zero missing bits in every
    /// transition (every destination mode gets fully real data), so
    /// falling through to "just switch mode, keep the same real byte" is
    /// already correct for them, not a shortcut. MODE 1 as the old mode
    /// genuinely does need an analogous reconstruction per the ACCC, but
    /// repeated attempts to re-derive its exact bit-selection rule from
    /// the compendium's diagrams alone produced results contradicting the
    /// ACCC's own stated valid colour-number ranges - a real,
    /// unresolved gap, not a simplification, tracked in the
    /// `project_gate_array_versions` memory (cross-check against Shaker
    /// Module B's own "R3 JIT" test and/or AMSpiriT before attempting it
    /// again, rather than the PDF diagrams alone).
    pub fn reconstruct_for_mode_switch(&self, new_mode: VideoMode, ga_type: GateArrayType) -> Self {
        if self.mode() == Some(new_mode) {
            // No real transition (the diagonal entries in every one of the
            // ACCC's own tables): nothing was ever missed, so this must
            // stay a pure no-op regardless of what the caller does.
            return match self {
                PixelAutomata::AvailablePixel(step, byte, mode, palette) => {
                    PixelAutomata::AvailablePixel(*step, *byte, *mode, *palette)
                }
                PixelAutomata::NeedToReadMemory => PixelAutomata::NeedToReadMemory,
            };
        }
        match self {
            PixelAutomata::AvailablePixel(step, byte, VideoMode::Mode1, palette)
                if *step == PixelOrder::Pixel5 =>
            {
                // ACCC 9.3.4.3.3 (CRTC 0/1/2): the display resumes on the 4th
                // pixel-M2 of the byte, still in MODE 1 colour (the last half
                // of pixel B, already produced here), and the new mode
                // starts on the 5th with bits recovered from the byte
                // (A0..A3, B0.., etc. are the MODE 1 pixel bits: pixel A =
                // (b3,b7), B = (b2,b6), C = (b1,b5), D = (b0,b4)):
                //   -> MODE 0: one pixel, colour (pad, b3, b1, b5)
                //   -> MODE 1: pixels C and D, unchanged
                //   -> MODE 2: three pixels b5, b4, b3
                //   -> MODE 3: one pixel, colour (b1, b5)
                // `pad` is 0 on a GA 40010 and 1 on a 40007/8.
                let bit = |n: u8| (byte >> n) & 1;
                let pad = ga_type.missed_bit_pad();
                let virtual_byte = match new_mode {
                    // MODE 0's second pixel (slots 5-8) reads b0,b4,b2,b6 as
                    // colour bits 3,2,1,0
                    VideoMode::Mode0 | VideoMode::Mode3 => {
                        (bit(5) << 6) | (bit(3) << 4) | (bit(1) << 2) | pad
                    }
                    // MODE 2 slots 5..8 read bits 3..0
                    VideoMode::Mode2 => (bit(5) << 3) | (bit(4) << 2) | (bit(3) << 1) | bit(2),
                    VideoMode::Mode1 => *byte,
                };
                PixelAutomata::AvailablePixel(PixelOrder::Pixel5, virtual_byte, new_mode, *palette)
            }
            PixelAutomata::AvailablePixel(step, byte, VideoMode::Mode2, palette) => {
                // MODE 2 pixels run one pixel-M2 ahead of the other modes'
                // (ACCC 9.3.4.3.1: it resumes on the 5th pixel, bit counter 5)
                let consumed = step.already_generated() + 1;
                let missed = consumed.saturating_sub(ga_type.reconstruction_lead());
                if missed == 0 {
                    return PixelAutomata::AvailablePixel(PixelOrder::Pixel1, *byte, new_mode, *palette);
                }
                let remaining = 8 - missed;
                let pad_byte = if ga_type.missed_bit_pad() == 1 { 0xFFu8 } else { 0x00u8 };
                let real_bits = byte & ((1u8 << remaining) - 1);
                let virtual_byte = (real_bits << missed) | (pad_byte & ((1u8 << missed) - 1));
                // Resume from Pixel1, not from wherever the old byte's own
                // count had reached: the ACCC is explicit that the new
                // mode treats the recovered bits "as if they were the 1st
                // bits of a byte" (§9.3.4.3.1), not a continuation of the
                // old mode's own pixel count.
                PixelAutomata::AvailablePixel(PixelOrder::Pixel1, virtual_byte, new_mode, *palette)
            }
            PixelAutomata::AvailablePixel(step, byte, _old_mode, palette) => {
                PixelAutomata::AvailablePixel(*step, *byte, new_mode, *palette)
            }
            PixelAutomata::NeedToReadMemory => PixelAutomata::NeedToReadMemory,
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


/// Handle the hsync information to generate hsync signal
/// two nops  after hsync
#[derive(Debug)]
pub enum HSyncCounter {
    NoHsync,
    HSync(usize),
}

impl Default for HSyncCounter {
    fn default() -> Self {
        Self::NoHsync
    }
}

impl HSyncCounter {
    pub fn inc(&mut self) {
        match self {
            Self::NoHsync => {
                *self = Self::HSync(0);
            },
            Self::HSync(ref mut count) => {
                *count = count.wrapping_add(1);
            }
        }
    }
}
/// Handle the interruption generation
#[derive(Debug)]
pub enum InterruptionSignal {
    NoSignal,
    Signal(usize),
}

/// The two "classic" (pre-Plus) Gate Array chip families found in real
/// CPCs (ACCC §9.3.4.3, confirmed against the compendium's own tables, not
/// inferred): the 40007 and 40008 (grouped together throughout the ACCC -
/// every table and note treats them identically) versus the 40010. They
/// share the same pin-level behaviour this emulator currently models
/// (register decode, INKR/border timing, HSYNC/VSYNC generation, the R52
/// interrupt counter, ...) - the one *documented* difference between them
/// is how the Gate Array reconstructs a colour number when the CRTC's
/// video mode (R comes from the firmware, not a CRTC register: `OUT
/// &7Fxx`) changes *in the middle of a byte already being displayed*.
///
/// When that happens, the pixel pipeline is partway through decoding one
/// VRAM byte under the *old* mode's bit grouping, and must resume under
/// the *new* mode's - some of that byte's bits were already consumed by
/// the old mode and can't be "un-read". The 40010 pads those already-used
/// bits with 0 when building the new colour number; the 40007/40008 pad
/// them with 1 instead (ACCC §9.3.4.3, "Bits missed by the GATE ARRAY
/// 40010 are considered 0 ... GATE ARRAY 40007 and 40008 ... Missed bits
/// are considered to be 1" - each with its own worked example and a full
/// VRAM-byte-to-displayed-pixels table per CRTC type and per mode-to-mode
/// transition, ACCC §9.3.4.3-.5).
///
/// **Not yet implemented**: mid-byte mode switching itself doesn't exist
/// in `PixelAutomata` yet (it locks in one mode for a whole byte the
/// moment it's constructed - see [`PixelAutomata::new`]), so there is
/// nowhere in the pixel pipeline yet for this distinction to apply. This
/// type exists so the configuration surface ("which chip is fitted") is
/// selectable and tested now, ready for that behaviour once a mid-byte
/// mode-switch mechanism is built - see `project_gate_array_versions`
/// memory for the detailed, page-referenced spec of what to implement,
/// verified against the ACCC's own tables/diagrams directly (not
/// transcribed from OCR/text-extracted PDF text, which mangles these
/// multi-column tables) before writing it down.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
pub enum GateArrayType {
    /// Ferranti 40007 / 40008 ULA.
    Ga40007_8,
    /// The later 40010 - what real-world CPC 6128s mostly shipped with
    /// (ACCC: "The 6128's are mostly equipped with 40010 version 36AA"),
    /// and what this emulator already modelled exclusively before this
    /// type existed, so it stays the default (no behaviour change).
    #[default]
    Ga40010
}

impl GateArrayType {
    /// The value a "missed" (already-consumed-by-the-old-mode) bit is
    /// padded with when reconstructing a colour number after a mid-byte
    /// mode switch (ACCC §9.3.4.3: "Bits missed by the GATE ARRAY 40010
    /// are considered 0 ... [GATE ARRAY 40007 and 40008] Missed bits are
    /// considered to be 1"). See [`PixelAutomata::reconstruct_for_mode_switch`].
    fn missed_bit_pad(&self) -> u8 {
        match self {
            GateArrayType::Ga40010 => 0,
            GateArrayType::Ga40007_8 => 1,
        }
    }

    /// GA40007/40008 recognise the reconstruction boundary 0.0625us (= 1
    /// Pixel-M2) earlier than GA40010 (ACCC §9.3.4.3.1: "The GATE ARRAY
    /// 40007 and 40008 are ahead of 0.0625usec ... when they recover the
    /// bits allowing for the constitution of the colour number"), so by
    /// the time a mode switch is recognised, they have one *more*
    /// genuinely real bit available and one *fewer* position to pad.
    fn reconstruction_lead(&self) -> u8 {
        match self {
            GateArrayType::Ga40010 => 0,
            GateArrayType::Ga40007_8 => 1,
        }
    }
}

/// GateArray emulation of the CPC.
/// Version 40010 is targeted; it suffers of a shift in pixel rendering for mode 2
/// TODO ensure the following sentence is applied
/// The effect of an INKR command appears almost immediatly always with the same timings, but the change is not in sync with the character display of the CRTC so the new color appears somehow in the middle of a character (the CRTC display unit).
#[derive(Debug)]
pub struct GateArray {
    pub (crate) clock: u8,
    pins: PinsBuffer,
    /// Which of the two classic Gate Array chip families is fitted - see
    /// [`GateArrayType`]'s own doc comment for what this does (and
    /// doesn't, yet) change.
    ga_type: GateArrayType,

    vsync_counter: VSyncCounter,
    /// HSYNC ends to wait after a VSYNC start before the R52 sync: 2, or 1
    /// when the VSYNC rises inside an HSYNC with R52 above 50.
    vsync_sync_hsyncs: usize,
    /// The CRTC's HSYNC ended in the very tick the VSYNC rose: the Gate Array
    /// hears of that end a little later, but it comes before the VSYNC and
    /// does not count for the R52 sync (CPCSyntaxError's order of events,
    /// seen on Checkmate).
    vsync_ignores_next_end: bool,
    hsync_counter: HSyncCounter,
    /// Number of HSYNC falling edges seen so far (diagnostics).
    hsync_ends: u64,
    monitor_hsync_advance: usize,
    hsync_restart: Option<u8>,
    rgb_previous: RgbLevels,
    rgb_delayed: bool,
    /// GA clock ticks since power-on (timestamps of the RMR mode writes).
    abs_tick: u64,
    /// The CRTC HSYNC as the GA sees it (see [`HSYNC_GA_DELAY_TICKS`]).
    hsync_pipe: [bool; HSYNC_GA_DELAY_TICKS],
    hsync_ga: bool,
    hsync_ga_prev: bool,
    /// Video mode decided at the last HSYNC end, applied one pixel later.
    pending_mode: Option<VideoMode>,
    /// The RMR video mode over the last ticks, newest first: a mode write
    /// only reaches the latch `MODE_WRITE_DELAY_TICKS` after it was made.
    mode_history: [VideoMode; MODE_WRITE_DELAY_TICKS + 1],
    /// The mode of this HSYNC pulse was already taken (6 usec cap reached).
    mode_latched: bool,
    /// Ticks left before the INT line rises (see `schedule_interrupt`).
    interrupt_delay: Option<u32>,
    /// T-states the last executed instruction left idle (see `z80::tstates`).
    last_idle_t: u8,
    ack_armed: bool,
    ack_countdown: Option<u32>,
    interrupt_signal_counter: InterruptionSignal,

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
            ga_type: GateArrayType::default(),

            vsync_counter: VSyncCounter::NoVsync,
            vsync_sync_hsyncs: 2,
            vsync_ignores_next_end: false,
            hsync_counter: HSyncCounter::NoHsync,
            hsync_ends: 0,
            monitor_hsync_advance: 0,
            hsync_restart: None,
            rgb_previous: (0., 0., 0.).into(),
            rgb_delayed: false,
            abs_tick: 0,
            hsync_pipe: [false; HSYNC_GA_DELAY_TICKS],
            hsync_ga: false,
            hsync_ga_prev: false,
            pending_mode: None,
            mode_history: [VideoMode::Mode1; MODE_WRITE_DELAY_TICKS + 1],
            mode_latched: false,
            interrupt_delay: None,
            last_idle_t: 0,
            ack_armed: false,
            ack_countdown: None,

            interrupt_signal_counter: InterruptionSignal::NoSignal,

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
        ga.reset();
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
        self.clock = (self.clock + 1) % 16 ;
        self.divide_clock();
    }

    pub fn divide_clock(&mut self) {
        self.set_pin(GateArrayPin::NCPU, ((self.clock + 0) % 4) >= 2); // this delay is needed to let time to CRTC to get cpu infos from pins
        self.set_pin(GateArrayPin::CCLK, !(((self.clock + 1) % 16) >= 8));
    }

    /// Switches to `new_mode` while the pixel pipeline may be mid-byte: the
    /// byte being displayed was fetched under the old mode, so what is left of
    /// it is reconstructed (ACCC 9.3.4.3, see
    /// `PixelAutomata::reconstruct_for_mode_switch`).
    fn apply_video_mode(&mut self, new_mode: VideoMode) {
        if new_mode != self.current_mode {
            self.pixel_automata = self
                .pixel_automata
                .reconstruct_for_mode_switch(new_mode, self.ga_type);
        }
        self.current_mode = new_mode;
    }

    /// Takes the video mode the Gate Array will switch to: the RMR mode as it
    /// stands now, the writes of this very tick not being seen yet. It is the
    /// end of the HSYNC-GA period (see [`MODE_LATCH_CAP_TICKS`]); the switch
    /// itself happens one pixel later, in the middle of a byte (ACCC 9.3.4).
    ///
    /// Calibrated against AMSpiriT (validated on real hardware) with Shaker's
    /// AE test (mode 2->0, R3=14: `OUT`s landing up to 6 usec after the HSYNC
    /// start count), and consistent with HBL's multi-mode rupture, whose
    /// `OUT mode2` lands right at the end of an R3=2 HSYNC and must not replace
    /// the mode0 written long before.
    fn latch_mode(&mut self, lag: usize) {
        let new_mode = self.mode_history[lag];
        self.mode_latched = true;
        if new_mode != self.current_mode {
            self.pending_mode = Some(new_mode);
        }
    }

    /// ACCC 27.6.1: "an interrupt always starts 1 usec AFTER THE END OF THE
    /// HSYNC regardless of the CRTC" - R52 itself counts on the HSYNC end,
    /// only the INT line it raises comes up a microsecond later.
    fn schedule_interrupt(&mut self) {
        self.interrupt_delay = Some(INTERRUPT_AFTER_HSYNC_TICKS);
    }

    /// The CRTC HSYNC as the GA sees it (a few pixels late, see
    /// [`HSYNC_GA_DELAY_TICKS`]): what blanks the display.
    pub fn hsync_ga(&self) -> bool {
        self.hsync_ga
    }

    /// HSYNC falling edges seen so far (diagnostics).
    pub fn hsync_ends(&self) -> u64 {
        self.hsync_ends
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

    /// Whether the pixel pipeline needs a fresh byte fetched for it now.
    ///
    /// A character is always exactly 2 bytes of video memory, whatever the
    /// mode (Pierre Guerrier's CRTC compendium; ACCC §9.1's "2 pixels..8
    /// pixels" tables are all per *byte*, and a character is 2 bytes
    /// regardless), so this must fetch exactly twice every 16-tick
    /// character clock (see `manage_clocks`/`divide_clock`, which derive
    /// CCLK from this same `self.clock`) - once at its start and once
    /// halfway through. This used to ask the pixel automaton itself
    /// whether it had run out of steps (`pixel_automata.is_done()`): the
    /// automaton free-runs its own 8-steps-per-byte count with nothing
    /// tying it to the character clock, so *if* it ever got out of phase
    /// with it (there was no structural reason it couldn't), it would
    /// happily keep fetching/displaying on its own, wrong, schedule
    /// forever after - concretely, during the border the fetch address
    /// (VMA) keeps advancing a full character at a time exactly like
    /// during active display (ACCC §17.3), so by the time a border ends
    /// and a row's real first character begins, an out-of-phase automaton
    /// could still be mid-byte on an already-prefetched-ahead byte,
    /// showing that stale pen (which can be a wildly different value from
    /// some effect elsewhere on the same line) for the first pixels of the
    /// row instead of the correct one.
    ///
    /// Deriving the answer from `self.clock` directly instead - the same
    /// counter CCLK itself is derived from - makes that drift structurally
    /// impossible rather than something to detect and correct after the
    /// fact: whatever the automaton's own state, a fresh byte gets loaded
    /// right on the character clock's own two fetch points, every time.
    pub fn need_memory_byte(&self) -> bool {
        self.clock == 7 || self.clock == 15
    }

    /// Fast-forward `self.clock` to the fetch phase (see [`GateArray::
    /// need_memory_byte`]'s own doc), so the very next call fetches
    /// immediately.
    ///
    /// Only for tests that manually pose a `Crtc` mid-frame (e.g. `hcc`
    /// poked directly to some row's last character) without actually
    /// driving every tick since reset: a fresh `GateArray::default()`
    /// starts at `clock == 0`, 7 ticks short of its first fetch point,
    /// which real, continuously-running hardware (and this same emulator
    /// under normal use) is never actually in - the CRTC and GA reset
    /// together and stay in phase from tick 0. Without this, such a test
    /// sees `is_done()` genuinely true with nothing fetched yet on its very
    /// first tick, i.e. a `generate_pixel()` that hasn't got a byte to show
    /// regardless of border/display state - a gap in the test's own setup,
    /// not a real hardware condition.
    pub fn seed_clock_for_test(&mut self) {
        self.clock = 7;
    }

    /// Convert the CRTC exposed address into the byte address currently consumed by GA.
    ///
    /// The CRTC address and CCLK phase can both carry pair-position information depending
    /// on where in the cycle we sample. Normalize to the 2-byte pair first, then pick the
    /// byte from CCLK to avoid duplicating the same byte twice.
    pub fn video_fetch_address(&self, crtc_address: u16) -> u16 {
        let pair_base = crtc_address & !1;
        if !self.get_pin(GateArrayPin::CCLK) {
            pair_base
        } else {
            pair_base | 1
        }
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
            // the last pixel of a delayed (non MODE 2) byte still comes out
            self.rgb = if self.rgb_delayed { self.rgb_previous } else { (0., 0., 0.).into() };
            self.rgb_previous = (0., 0., 0.).into();
            self.rgb_delayed = false;
        } else {
            // Always advance the automaton, even while blanked by HSYNC/
            // VSYNC - real hardware's pixel serialiser keeps running
            // through the blanking period too (it's what lets a mode
            // switch land "mid-byte" at all: see ACCC §9.3.4.3 and
            // `PixelAutomata::reconstruct_for_mode_switch`, which reads
            // how many pixels of the current byte are `already_generated`
            // to know exactly where the switch landed). Only the
            // *displayed* output is blanked below - not consumption.
            let mode = self.pixel_automata.mode();
            let computed = self
                .pixel_automata
                .generate_pixel(!self.get_pin(GateArrayPin::DISPEN));
            // ACCC §9.3 (GA 40007/8/10): the pixels of MODE 2 are displayed one
            // pixel earlier than those of the other modes (the BORDER stops and
            // starts one pixel earlier too); the other modes are what we delay
            let pixel = if mode == Some(VideoMode::Mode2) { computed } else { self.rgb_previous };
            self.rgb_previous = computed;
            self.rgb_delayed = mode != Some(VideoMode::Mode2);
            if self.hsync_ga || self.get_pin(GateArrayPin::VSYNC) {
                self.rgb = (0., 0., 0.).into();
            } else {
                self.rgb = pixel;
            }
        }
    }

    /// Generate the synchronization data for the CTM
    /// TODO understand/find how it is really done
    fn generate_sync(&mut self) {

        let hsync = match self.hsync_counter {
            HSyncCounter::NoHsync => false,
            HSyncCounter::HSync(n) => (32..96).contains(&(n + self.monitor_hsync_advance)), // from 2 nops during 4 nops
        };

        self.sync = Sync {
            vsync: self.get_pin(GateArrayPin::VSYNC),
            hsync,
        }
    }

    /// When the hsync ends, the GA update video mode and manage the interruption counter.
    /// it also manage a counter for the interuptions and generate one when it reaches 52
    /// TODO check who disable the interrupt (GA or z80?)
    fn manage_hsync_end(&mut self) {
        self.hsync_ends += 1;
        let width = match self.hsync_counter {
            HSyncCounter::HSync(n) => n as u64 + 1,
            HSyncCounter::NoHsync => 0,
        };
        self.hsync_counter = HSyncCounter::NoHsync;

        // the mode is normally taken when the HSYNC-GA period ends: here only
        // if the 6 usec cap was not reached first (see `latch_mode`)
        if !self.mode_latched && width >= MODE_UPDATE_MIN_HSYNC_TICKS {
            self.latch_mode(MODE_WRITE_DELAY_TICKS);
        }
        self.mode_latched = false;
        self.r52.inc();
        // ACCC §27.3.2: while a VSYNC sync is pending (2 HSYNC ends after its
        // start) the 52 count is not looked at; the sync then resets R52 and
        // requests an interrupt if bit 5 was set (Shaker BR/B, CPCSyntaxError's
        // order of events seen on Checkmate).
        let pending = match self.vsync_counter {
            VSyncCounter::VSync(ref mut count) if *count < self.vsync_sync_hsyncs => {
                if self.vsync_ignores_next_end {
                    self.vsync_ignores_next_end = false;
                } else {
                    *count += 1;
                }
                Some(*count >= self.vsync_sync_hsyncs)
            }
            _ => None,
        };
        match pending {
            Some(true) => {
                let interrupt = self.r52.bit(5);
                self.r52.reset();
                if interrupt {
                    self.schedule_interrupt();
                }
            }
            Some(false) => {}
            None => {
                if *self.r52.deref() >= 52 {
                    self.r52.reset();
                    self.schedule_interrupt();
                }
            }
        }
    }

    fn request_interrupt(&mut self) {
  //      dbg!("GA request interrupt");
        self.interrupt_signal_counter = InterruptionSignal::Signal(0);
        self.set_pin(GateArrayPin::INTERRUPT, false); //activate when low
    }

    /// Cancel an already-*requested* interrupt that the CPU hasn't
    /// acknowledged yet - the manual-clear path (`RMR` bit 4, below in
    /// `write_register`) needs this in addition to resetting R52 itself.
    /// Cross-checked against the "chips" project's pin-level am40010.h
    /// (floooh/chips, `~/Perso/CPC/tiny8bit/chips/chips/am40010.h`,
    /// `_am40010_tick`'s `AM40010_CONFIG_IRQRESET` handling): real
    /// hardware's manual clear sets its `intr` flip-flop (our
    /// `interrupt_signal_counter`) false outright, not just the counter -
    /// otherwise an interrupt already in flight at the moment of a manual
    /// clear still reaches the CPU instead of being cancelled by it.
    fn cancel_interrupt(&mut self) {
        self.interrupt_signal_counter = InterruptionSignal::NoSignal;
        self.set_pin(GateArrayPin::INTERRUPT, true); //inactive when high
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

    /// Which of the two classic Gate Array chip families is fitted - see
    /// [`GateArrayType`]'s own doc comment.
    pub fn gate_array_type(&self) -> GateArrayType {
        self.ga_type
    }

    /// Changes which of the two classic Gate Array chip families is
    /// fitted. See [`GateArrayType`]'s own doc comment for what this
    /// currently does (selects the configuration) and doesn't yet
    /// (change any actual pixel decode - the one documented behavioural
    /// difference needs a mid-byte mode-switch mechanism that doesn't
    /// exist yet).
    pub fn set_gate_array_type(&mut self, ga_type: GateArrayType) {
        self.ga_type = ga_type;
    }

    pub fn r52(&self) -> u8 {
        *self.r52.deref()
    }

    /// When the CPU acknowledge the interrupt (eg. it is going to jump to the interrupt vector), the Gate Array will reset bit5 of the counter, so the next interrupt can't occur closer than 32 HSync. 
    /// An R2.JIT HSYNC rises late for the black zone, but the monitor is
    /// synchronised as if it had risen on its character (ACCC 14.7.1).
    /// The CRTC began its HSYNC over (see `Crtc::take_hsync_restart`): count it from now
    pub fn restart_hsync(&mut self, ticks_ago: u8) {
        self.hsync_restart = Some(ticks_ago);
    }

    pub fn set_monitor_hsync_advance(&mut self, ticks: u8) {
        self.monitor_hsync_advance = usize::from(ticks);
    }

    pub fn set_last_idle_t(&mut self, idle: u8) {
        self.last_idle_t = idle;
    }

    fn handle_cpu_acknowledge_of_interruption(&mut self) {
 //       dbg!("CPU acknowledge interruption");
       self.r52.bitand_assign(0b011111);
    }
}
impl Chip for GateArray {
    type PinType = GateArrayPin;

    fn must_execute_tick(&self) -> bool {
        // M1 low together with IORQ is an interrupt acknowledge, not an I/O
        // cycle: the address bus then spells the PC, which decodes to the
        // GA's port whenever PC is in 0x4000-0x7FFF, and the data bus
        // floating must never be taken for a register write (it used to
        // randomly switch ROMs/modes/RAM config at every interrupt).
        !self.get_pin(GateArrayPin::A15)
            && self.get_pin(GateArrayPin::A14)
            && !self.get_pin(GateArrayPin::Neg_IORQ)
            && self.get_pin(GateArrayPin::Neg_M1)
    }

    /// Change the screen mode when hsync is set
    fn pre_tick(&mut self) {
        self.abs_tick += 1;
        self.mode_history.rotate_right(1);
        self.mode_history[0] = self.rmr.video_mode();
        if self.is_upfront(GateArrayPin::VSYNC) {
            //vsync occured
            self.vsync_counter = VSyncCounter::VSync(0);
            let pin_active = self.get_pin(GateArrayPin::HSYNC);
            self.vsync_sync_hsyncs = if pin_active && self.r52.deref() > &50 { 1 } else { 2 };
            self.vsync_ignores_next_end = !pin_active && self.hsync_ga;
        }

        // the HSYNC as the GA sees it, a few pixels after the CRTC raised it
        self.hsync_ga_prev = self.hsync_ga;
        self.hsync_ga = self.hsync_pipe[0];
        self.hsync_pipe.rotate_left(1);
        self.hsync_pipe[HSYNC_GA_DELAY_TICKS - 1] = self.get_pin(GateArrayPin::HSYNC);

        if let Some(left) = self.interrupt_delay {
            if left <= 1 {
                self.interrupt_delay = None;
                self.request_interrupt();
            } else {
                self.interrupt_delay = Some(left - 1);
            }
        }

        // a mode decided at the previous HSYNC end changes one pixel after
        // the display came back (the first pixel is still in the old mode)
        if let Some(new_mode) = self.pending_mode.take() {
            self.apply_video_mode(new_mode);
        }

        if self.is_upfront(GateArrayPin::VSYNC) {
            self.hsync_counter = HSyncCounter::HSync(0);
        }
        else if self.hsync_ga_prev && !self.hsync_ga {
            self.manage_hsync_end();
        }
        else if self.hsync_ga {
            self.hsync_counter.inc();
        }
        if let Some(ago) = self.hsync_restart.take() {
            if self.hsync_ga {
                self.hsync_counter = HSyncCounter::HSync(usize::from(ago));
                self.mode_latched = false;
            }
        }
        if self.hsync_ga && !self.mode_latched {
            if let HSyncCounter::HSync(n) = self.hsync_counter {
                if n as u64 + 1 >= MODE_LATCH_CAP_TICKS {
                    self.latch_mode(0);
                }
            }
        }

        // XXX play with the modulo to handle the appropriate timing
        if !self.get_pin(GateArrayPin::Neg_M1) && !self.get_pin(GateArrayPin::Neg_IORQ) {
            // the request is held until the CPU answers it
            self.interrupt_signal_counter = InterruptionSignal::NoSignal;
            self.set_pin(GateArrayPin::INTERRUPT, true);
            if !self.ack_armed && self.clock % 4 == 0 {
                self.ack_armed = true;
                // ACCC §27.3.1/§27.7.1: the Gate Array hears of the acknowledge
                // on the Z80A's own last T-state of the interrupted
                // instruction. One that ends with 2 or more idle T-states
                // is heard in its last microsecond; the others a
                // microsecond later (SHAKER BR/A on a real CPC, and the
                // same rule in CPCSyntaxError's timing).
                if self.last_idle_t <= 1 {
                    self.ack_countdown = Some(ACK_LATE_TICKS);
                } else {
                    self.handle_cpu_acknowledge_of_interruption();
                }
            }
        } else {
            self.ack_armed = false;
        }
        if let Some(left) = self.ack_countdown {
            if left <= 1 {
                self.ack_countdown = None;
                self.handle_cpu_acknowledge_of_interruption();
            } else {
                self.ack_countdown = Some(left - 1);
            }
        }
       
        self.generate_pixel();
        self.generate_sync();
    }



    /// React to the port addressing of the CPU
    fn execute_tick(&mut self) {
        if !self.get_pin(GateArrayPin::Neg_RD) {
            // The real gate array has no data-output driver at all on a
            // read - it's write-only from the CPU's perspective, so a read
            // that decodes to it (partial address decoding makes this a
            // routine occurrence, not a software bug: real firmware/games
            // do it constantly, e.g. reading the FDC or PPI status while
            // the GA's own incomplete decoding also matches) leaves the
            // bus floating. Nothing to do here - there used to be an
            // eprintln!() on this branch, printed on every such tick (often
            // hundreds of thousands of times a second for real software),
            // which was pure debug noise and, worse, slowed emulation down
            // by roughly an order of magnitude (stderr I/O in a hot path).
        }
        else {
            // out expected
            self.write_register(self.get_data_from_pins());
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
        self.on(GateArrayPin::INTERRUPT);
    }
}

impl GateArray {
    /// Feed one byte through the gate array's own register-select logic,
    /// the way an `OUT` to its port does (see [`Chip::execute_tick`]):
    /// dispatches on its top bits to PENR/INKR/RMR. Public so a snapshot
    /// loader can replay a `.SNA`'s recorded GA bytes exactly as the real
    /// chip would have seen them, rather than reconstructing the resulting
    /// pen/palette/mode state by hand.
    pub fn write_register(&mut self, command: u8) {
        if PENR::is_addressed(command) {
            self.penr = PENR(command);
        } else if INKR::is_addressed(command) {
            self.inkr = INKR(command);
            self.set_ink(self.pen(), self.ink());
        } else if RMR::is_addressed(command) {
            self.rmr = RMR(command);
            if self.interrupt_counter() {
                self.r52.reset();
                self.cancel_interrupt();
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /*
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
    }*/

    /// Cross-checks [`PixelAutomata::reconstruct_for_mode_switch`]'s
    /// virtual-byte construction against the ACCC §9.3.4.3.1 MODE 2 -> MODE
    /// 0 worked tables, independently for both GA types (the two tables
    /// give genuinely different bit references, not just a 0-vs-1 swap, so
    /// this isn't just testing the padding constant).
    fn mode2_automaton_at(consumed: u8, byte: u8) -> PixelAutomata {
        let step = match consumed {
            0 => PixelOrder::Pixel1,
            1 => PixelOrder::Pixel2,
            2 => PixelOrder::Pixel3,
            3 => PixelOrder::Pixel4,
            4 => PixelOrder::Pixel5,
            5 => PixelOrder::Pixel6,
            6 => PixelOrder::Pixel7,
            7 => PixelOrder::Pixel8,
            _ => unreachable!(),
        };
        PixelAutomata::AvailablePixel(step, byte, VideoMode::Mode2, [Ink(0); 17])
    }

    #[test]
    fn mode2_to_mode0_reconstruction_matches_accc_ga40010_table() {
        // ACCC: GA40010's bit counter reaches 5 by the time a MODE 2 byte's
        // display resumes; missed bits (the top 5) are padded with 0, and
        // the new colour number is (A3,A2,A1,A0) = (0, b0, 0, b2).
        let byte = 0b0000_0101u8; // b2=1, b1=0, b0=1
        let automaton = mode2_automaton_at(4, byte);
        let reconstructed = automaton.reconstruct_for_mode_switch(VideoMode::Mode0, GateArrayType::Ga40010);
        // virtual byte: bits[7,6,5] = [b2,b1,b0] = [1,0,1], bits[4..0] = 0
        assert_eq!(reconstructed.byte(), Some(0b1010_0000));
        assert_eq!(reconstructed.step(), Some(&PixelOrder::Pixel1));
        assert_eq!(reconstructed.mode(), Some(VideoMode::Mode0));
    }

    #[test]
    fn mode2_to_mode0_reconstruction_matches_accc_ga4000708_table() {
        // GA40007/8 recognise the boundary 1 Pixel-M2 earlier (bit counter
        // at 4, not 5): one more real bit (b3) is available, missed bits
        // (the top 4) are padded with 1 instead of 0.
        let byte = 0b0000_1101u8; // b3=1, b2=1, b1=0, b0=1
        let automaton = mode2_automaton_at(4, byte);
        let reconstructed = automaton.reconstruct_for_mode_switch(VideoMode::Mode0, GateArrayType::Ga40007_8);
        // virtual byte: bits[7,6,5,4] = [b3,b2,b1,b0] = [1,1,0,1], bits[3..0] = 1111
        assert_eq!(reconstructed.byte(), Some(0b1101_1111));
        assert_eq!(reconstructed.step(), Some(&PixelOrder::Pixel1));
    }

    /// ACCC 9.3.4.3.3: MODE 1 -> 0/2/3 with the display back on the 4th
    /// pixel-M2 (4 already produced). Byte b7..b0 = 1011_0110.
    #[test]
    fn mode1_to_other_modes_matches_accc_table() {
        let byte = 0b1011_0110u8; // b5=1 b4=1 b3=0 b2=1 b1=1 b0=0
        let at = || PixelAutomata::AvailablePixel(PixelOrder::Pixel5, byte, VideoMode::Mode1, [Ink(0); 17]);
        // MODE 0: colour (pad, b3, b1, b5) = (0,0,1,1) on a 40010, (1,0,1,1) on a 40007/8
        for (ga, pad) in [(GateArrayType::Ga40010, 0u8), (GateArrayType::Ga40007_8, 1u8)] {
            let r = at().reconstruct_for_mode_switch(VideoMode::Mode0, ga);
            let v = r.byte().unwrap();
            let pen = BYTE_TO_MODE0_PENS[v as usize].1.value();
            assert_eq!(pen, (pad << 3) | (0 << 2) | (1 << 1) | 1, "mode 0, pad {pad}");
            assert_eq!(r.step(), Some(&PixelOrder::Pixel5));
        }
        // MODE 3: colour (b1, b5) = (1, 1)
        let r = at().reconstruct_for_mode_switch(VideoMode::Mode3, GateArrayType::Ga40010);
        assert_eq!(BYTE_TO_MODE0_PENS[r.byte().unwrap() as usize].1.value() & 3, 0b11);
        // MODE 2: pixels b5, b4, b3 then the continuation (b2) on slots 5..8
        let r = at().reconstruct_for_mode_switch(VideoMode::Mode2, GateArrayType::Ga40010);
        let pens = BYTE_TO_MODE2_PENS[r.byte().unwrap() as usize];
        assert_eq!([pens.4.value(), pens.5.value(), pens.6.value(), pens.7.value()], [1, 1, 0, 1]);
        // MODE 1 -> MODE 1 never gets here with a change; the byte stays real
        let r = at().reconstruct_for_mode_switch(VideoMode::Mode1, GateArrayType::Ga40010);
        assert_eq!(r.byte(), Some(byte));
    }

    #[test]
    fn mode2_to_mode2_never_reconstructs_since_new_mode_equals_old_mode() {
        // Staying in MODE 2 is the trivial diagonal case: the ACCC's own
        // tables show no missing bits and no GA-type split there (the
        // caller in `manage_hsync_end` never invokes reconstruction at all
        // when the mode doesn't actually change - this test documents that
        // the function itself is also a safe no-op if it somehow were).
        let byte = 0b1011_0110u8;
        let automaton = mode2_automaton_at(4, byte);
        let reconstructed = automaton.reconstruct_for_mode_switch(VideoMode::Mode2, GateArrayType::Ga40010);
        assert_eq!(reconstructed.byte(), Some(byte));
    }

    #[test]
    fn mode0_and_mode3_sources_never_get_ga_dependent_padding() {
        // Confirmed from the ACCC's own tables: MODE 0 and MODE 3 as the
        // *old* mode show zero missing bits in every destination mode, for
        // both chip families - so reconstruction must be a pure pass-
        // through of the real byte regardless of GA type.
        let byte = 0b1010_1100u8;
        for old_mode in [VideoMode::Mode0, VideoMode::Mode3] {
            // Pixel5 matches the ACCC's own stated resumption point for
            // MODE 0/3 sources ("4th Pixel-M2 of the 5th byte", i.e. 4
            // pixels already generated) - unlike MODE 2, these old modes'
            // own bit-to-pixel grouping is preserved (not reset to Pixel1)
            // because there's nothing to reconstruct: the position itself
            // is what picks the correct still-real "second half" of the
            // byte (e.g. `get_mode0_pen`'s Pixel5-8 branch).
            let automaton = PixelAutomata::AvailablePixel(PixelOrder::Pixel5, byte, old_mode, [Ink(0); 17]);
            for ga_type in [GateArrayType::Ga40010, GateArrayType::Ga40007_8] {
                let reconstructed = automaton.reconstruct_for_mode_switch(VideoMode::Mode1, ga_type);
                assert_eq!(reconstructed.byte(), Some(byte));
                assert_eq!(reconstructed.step(), Some(&PixelOrder::Pixel5));
            }
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

        // Drive an OUT cycle toward GA (A15=0, A14=1, IORQ active, RD inactive).
        ga.set_data_on_pins(0x8d); // Mode 1
        ga.set_pin(GateArrayPin::A15, false);
        ga.set_pin(GateArrayPin::A14, true);
        ga.set_pin(GateArrayPin::Neg_IORQ, false);
        ga.set_pin(GateArrayPin::Neg_M1, true);
        ga.set_pin(GateArrayPin::Neg_RD, true);
        assert!(ga.must_execute_tick());
        ga.tick();

        assert_eq!(ga.active_video_mode(), VideoMode::Mode0);
        assert_eq!(ga.next_video_mode(), VideoMode::Mode1);
        assert_eq!(ga.lower_rom_pagging(), Flag::Disable);
        assert_eq!(ga.upper_rom_pagging(), Flag::Disable);
        // a long HSYNC pulse, so the Mode 1 write is well before its middle
        ga.set_pin(GateArrayPin::Neg_IORQ, true);
        ga.save_pins_state();
        ga.set_pin(GateArrayPin::HSYNC, true);
        ga.tick();
        for _ in 0..64 {
            ga.save_pins_state();
            ga.tick();
        }
        // the HSYNC ends; the GA sees it a few ticks late, takes the mode (Mode 1)
        // and switches one pixel later
        ga.set_pin(GateArrayPin::HSYNC, false);
        for _ in 0..(HSYNC_GA_DELAY_TICKS + 3) {
            ga.save_pins_state();
            ga.tick();
        }
        // a Mode 2 write after that waits for the next HSYNC
        ga.set_data_on_pins(0x8e);
        ga.set_pin(GateArrayPin::A15, false);
        ga.set_pin(GateArrayPin::A14, true);
        ga.set_pin(GateArrayPin::Neg_IORQ, false);
        ga.set_pin(GateArrayPin::Neg_M1, true);
        ga.set_pin(GateArrayPin::Neg_RD, true);
        ga.save_pins_state();
        ga.tick();
        ga.set_pin(GateArrayPin::Neg_IORQ, true);

        assert_eq!(ga.active_video_mode(), VideoMode::Mode1);
        assert_eq!(ga.next_video_mode(), VideoMode::Mode2);
        assert_eq!(ga.lower_rom_pagging(), Flag::Disable);
        assert_eq!(ga.upper_rom_pagging(), Flag::Disable);
    }

    /// An interrupt acknowledge (IORQ and M1 both low) with PC in
    /// 0x4000-0x7FFF decodes to the GA's port: it must not be a write.
    #[test]
    fn interrupt_acknowledge_is_not_a_register_write() {
        let mut ga = GateArray::default();
        ga.set_data_on_pins(0x8d); // would be RMR: mode 1, both ROMs enabled
        ga.set_pin(GateArrayPin::A15, false);
        ga.set_pin(GateArrayPin::A14, true);
        ga.set_pin(GateArrayPin::Neg_IORQ, false);
        ga.set_pin(GateArrayPin::Neg_M1, false);
        ga.set_pin(GateArrayPin::Neg_RD, true);
        assert!(!ga.must_execute_tick());
        ga.tick();
        assert_eq!(ga.next_video_mode(), VideoMode::Mode0);
        // a real write of 0x8d would have disabled both ROMs
        assert_eq!(ga.lower_rom_pagging(), Flag::Enable);
        assert_eq!(ga.upper_rom_pagging(), Flag::Enable);
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
        // the pixels of the other modes than MODE 2 come out one tick late
        let delayed = ga.pixel_automata.mode() != Some(VideoMode::Mode2);
        if delayed {
            ga.tick();
        }
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
