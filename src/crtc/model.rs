//! What differs between the CRTC chips found in CPCs, as a strategy the
//! [`super::Crtc`] core consults.
//!
//! The five "CRTC types" (numbering from Longshot's *Amstrad CPC CRTC
//! Compendium*, ACCC) are not five emulators: they share the whole 6845
//! counter chain and differ in a long list of details - which register
//! bits exist, which registers can be read back and how, the widths of the
//! sync pulses, what the display skew field does, and many timing quirks.
//! Each chip implements [`CrtcModel`]; the core never tests "which type am
//! I" but asks the model, so a new chip (or a variant of one, like the two
//! UM6845R revisions) is one new implementation and nothing else.
//!
//! The models are stateless zero-sized values, so `Crtc` just holds a
//! `&'static dyn CrtcModel`: the indirection is a single pointer and the
//! hooks are only called on register accesses and line/frame events, never
//! per pin.

use std::fmt::Debug;

/// Number of registers a CRTC exposes (R0-R17).
pub const NB_REGISTERS: usize = 18;

/// The register file: R0-R17, already masked to what the chip stores.
pub type Registers = [u8; NB_REGISTERS];

/// The five CRTC types (ACCC numbering; 4 is the "pre-ASIC" cost-reduced
/// CPC's chip, numbered after 3 because it was discovered after it).
#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
#[repr(u8)]
pub enum CrtcType {
    /// Hitachi HD6845S / UMC UM6845
    Crtc0 = 0,
    /// UMC UM6845R
    Crtc1 = 1,
    /// Motorola MC6845
    Crtc2 = 2,
    /// Amstrad ASIC 40489 (CPC+)
    Crtc3 = 3,
    /// Amstrad ASIC 40226 ("pre-ASIC" cost-reduced CPC)
    Crtc4 = 4
}

impl Default for CrtcType {
    fn default() -> Self {
        CrtcType::Crtc0
    }
}

impl CrtcType {
    pub const ALL: [CrtcType; 5] = [
        CrtcType::Crtc0,
        CrtcType::Crtc1,
        CrtcType::Crtc2,
        CrtcType::Crtc3,
        CrtcType::Crtc4
    ];

    /// The behaviour of this chip.
    pub fn model(self) -> &'static dyn CrtcModel {
        match self {
            CrtcType::Crtc0 => &super::type0::Crtc0,
            CrtcType::Crtc1 => &super::type1::Crtc1,
            CrtcType::Crtc2 => &super::type2::Crtc2,
            CrtcType::Crtc3 => &super::asic::Asic::CRTC3,
            CrtcType::Crtc4 => &super::asic::Asic::CRTC4
        }
    }
}

impl TryFrom<u8> for CrtcType {
    type Error = String;

    fn try_from(number: u8) -> Result<Self, String> {
        CrtcType::ALL
            .get(usize::from(number))
            .copied()
            .ok_or_else(|| format!("{number} is not a CRTC type (0-4)"))
    }
}

/// What a register read may depend on, gathered by the core.
pub struct ReadContext<'a> {
    pub registers: &'a Registers,
    /// The register number written to `&BC00`, its 5 significant bits.
    pub selected: u8,
    /// Whether the last completed line was in the vertical border by R6
    /// (the "V" bit of the UM6845R status register).
    pub vertical_border: bool,
    /// A random bit, for the floating values some chips return.
    pub noise: bool
}

/// Bits of a pointer register (R12-R17) that read back: the high registers
/// are 6 bits wide (ACCC §21.2), the low ones 8.
pub const fn pointer_mask(register: u8) -> u8 {
    match register {
        12 | 14 | 16 => 0x3f,
        _ => 0xff
    }
}

impl ReadContext<'_> {
    pub fn register(&self, number: u8) -> u8 {
        self.registers.get(usize::from(number)).copied().unwrap_or(0)
    }
}

/// How the display skew field of R8 shifts DISPTMG.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum DisplaySkew {
    /// DISPTMG delayed by that many characters (0-2).
    Delayed(u8),
    /// Skew of 3: the display is never enabled ("non-display").
    Blanked
}

/// The chip-dependent part of a CRTC.
pub trait CrtcModel: Debug + Sync {
    fn crtc_type(&self) -> CrtcType;

    fn name(&self) -> &'static str;

    /// Bits of register `reg` (0-15) the chip stores; 0 for anything else
    /// (ACCC §4.3).
    fn write_mask(&self, reg: u8) -> u8;

    /// The value on the data bus when reading the register port (&BF00).
    /// Registers the chip can't return read as 0 (ACCC §21.2, §28.1.9).
    fn read_register(&self, context: &ReadContext) -> u8;

    /// The value on the data bus when reading the status port (&BE00)
    /// (ACCC §21.3).
    fn read_status(&self, context: &ReadContext) -> u8;

    /// HSYNC width, in characters, from R3. `None`: no HSYNC is generated
    /// (a zero width on CRTC 0 and 1, ACCC §14.6).
    fn hsync_width(&self, r3: u8) -> Option<u8>;

    /// VSYNC width, in scan lines, from R3 (ACCC §14.1: only CRTC 0, 3 and 4
    /// program it; 1 and 2 have a fixed 16).
    fn vsync_width(&self, r3: u8) -> u8;

    /// ACCC §14.4.2: writing R3l=0 while an HSYNC is running cancels it on
    /// CRTC 1; the others treat 0 as a count to reach (after overflowing).
    fn zero_r3_cancels_running_hsync(&self) -> bool {
        false
    }

    /// R3.JIT (ACCC 14.4.4): an R3l write equal to the running C3l stops the
    /// HSYNC at once. Not on the ASICs of CRTC 3 and 4, which synchronise the
    /// HSYNC with the display.
    fn supports_r3_jit(&self) -> bool {
        true
    }

    /// CRTC 0 (ACCC 12.2, 10.3.1): the frame's last line is a latched state,
    /// evaluated (C4==R4 && C9==R9) only while C0<2. Once set it resets C4
    /// and C9 on the next line whatever R4/R9 become; when not set, C4
    /// counts on (up to 127) each time C9==R9, R4 or no R4.
    fn has_last_line_state(&self) -> bool {
        false
    }

    /// The chip has the two interlace modes (R8 bit 0, ACCC 19): an additional line and a
    /// MID-VSYNC on the even frames. CRTC 0 is handled by its own counters.
    fn supports_interlace(&self) -> bool {
        self.has_last_line_state()
    }

    /// CRTC 1 (ACCC 19.5.3): ParityFrame simply switches at each frame start, whatever R8.
    fn frame_parity_toggles(&self) -> bool {
        false
    }

    /// CRTC 0 and 2 (ACCC 17.6.2): when C0 reaches R0 with the display still
    /// on (R1 not reached), they send BORDER ON to the Gate Array 0.5 usec too
    /// early: one byte (half a character) of border before C0 returns to 0.
    fn anticipates_border_byte(&self) -> bool {
        false
    }

    /// ACCC 18.2.2: on CRTC 0 and 2, C4=R6 starts a BORDER that lasts until the
    /// next frame (C4=C9=C0=0), whatever R6 is made later.
    fn latches_r6_border(&self) -> bool {
        false
    }

    /// CRTC 1 (ACCC 18.2.3): like CRTC 0 and 2, C4=R6 latches the BORDER, but R6=0 is a BORDER
    /// of its own while it stays 0 and there is no first line conflict.
    fn r6_zero_is_border(&self) -> bool {
        false
    }

    /// CRTC 1 (ACCC 11.6): the "rupture for dummies" bug of R5.
    fn has_rfd(&self) -> bool {
        false
    }

    /// ACCC 17.5: an R1=0 written while C0=0 is still in time to raise the
    /// BORDER on CRTC 0, 1 and 2 ("too late" on CRTC 3 and 4).
    fn acknowledges_r1_zero_on_c0_zero(&self) -> bool {
        true
    }

    /// What the skew field of R8 does; none on CRTC 1 and 2 (ACCC §19.2).
    fn display_skew(&self, r8: u8) -> DisplaySkew;

    /// The UM6845R keeps reloading the display start address on every line
    /// of the first character row instead of once per frame (ACCC §11.6).
    fn reloads_start_address_every_line_of_first_row(&self) -> bool {
        false
    }
}
