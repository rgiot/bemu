//! What the five CRTC chips found in CPCs share, and the [`CrtcChip`] trait each of them
//! implements in its own file (`type0.rs`, `type1.rs`, `type2.rs`, `asic.rs`).
//!
//! The "CRTC types" (numbering from Longshot's *Amstrad CPC CRTC Compendium*, ACCC) share the
//! 6845 counter chain kept in [`super::core::Core`], but each runs its own vertical logic (last
//! line states, adjustment, interlace counting), its own BORDER and VSYNC rules and its own
//! register side effects, with its own extra state. [`super::Crtc`] drives the character clock
//! and calls the chip at each of those points.

use std::fmt::Debug;

use super::core::Core;

/// Number of registers a CRTC exposes (R0-R17).
pub const NB_REGISTERS: usize = 18;

/// The register file: R0-R17, already masked to what the chip stores.
pub type Registers = [u8; NB_REGISTERS];

/// The five CRTC types (ACCC numbering; 4 is the "pre-ASIC" cost-reduced
/// CPC's chip, numbered after 3 because it was discovered after it).
#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash)]
#[repr(u8)]
#[derive(Default)]
pub enum CrtcType {
    /// Hitachi HD6845S / UMC UM6845
    #[default]
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


impl CrtcType {
    pub const ALL: [CrtcType; 5] = [
        CrtcType::Crtc0,
        CrtcType::Crtc1,
        CrtcType::Crtc2,
        CrtcType::Crtc3,
        CrtcType::Crtc4
    ];
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
    pub noise: bool,
    /// The counters, for the status registers of the CRTCs 3 and 4 (ACCC §21.3.4): C0, C4, C9,
    /// C3h (the VSYNC's line, while it runs), VMA and VMA', and the VSYNCs begun.
    pub c0: u8,
    pub c4: u8,
    pub c9: u8,
    pub vsync_line: Option<u8>,
    pub vma: u16,
    pub vma_prime: u16,
    pub fields: u8
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


/// The registers as they were before a write, for the side effects that depend on the change.
#[derive(Clone, Copy, Debug)]
pub struct PreviousRegisters {
    pub r0: u8,
    pub r3l: u8,
    pub r5: u8,
    pub r8: u8
}

/// One CRTC chip: what it is (register bits, read-back, sync widths, skew) and how it runs the
/// counters of [`Core`] at each point of the character clock where the chips differ. The
/// default methods are the plain 6845 behaviour; each chip overrides what it does its own way.
pub trait CrtcChip: Debug + Send {
    fn crtc_type(&self) -> CrtcType;

    fn name(&self) -> &'static str;

    /// Bits of register `reg` (0-15) the chip stores; 0 for anything else (ACCC §4.3).
    fn write_mask(&self, reg: u8) -> u8;

    /// The value on the data bus when reading the register port (&BF00).
    /// Registers the chip can't return read as 0 (ACCC §21.2, §28.1.9).
    fn read_register(&self, context: &ReadContext) -> u8;

    /// The value on the data bus when reading the status port (&BE00) (ACCC §21.3).
    fn read_status(&self, context: &ReadContext) -> u8;

    /// HSYNC width, in characters, from R3. `None`: no HSYNC is generated
    /// (a zero width on CRTC 0 and 1, ACCC §14.6).
    fn hsync_width(&self, r3: u8) -> Option<u8>;

    /// VSYNC width, in scan lines, from R3 (ACCC §14.1: only CRTC 0, 3 and 4
    /// program it; 1 and 2 have a fixed 16).
    fn vsync_width(&self, r3: u8) -> u8;

    /// What the skew field of R8 does; none on CRTC 1 and 2 (ACCC §19.2).
    fn display_skew(&self, r8: u8) -> DisplaySkew;

    /// Back to the power-on state (the core is reset first).
    fn reset(&mut self, _core: &mut Core) {}

    // ---- register writes ----

    /// Whether the chip takes a write (a register select or a data write) on this tick, `phase`
    /// ticks into the character. A 6845 takes it as soon as its enable is active; a write outside
    /// the chip's window waits for the next one while the CPU still holds it.
    fn samples_write_at(&self, _phase: u8) -> bool {
        true
    }

    /// A write the chip applies later (the ASICs): true when it is queued.
    fn queue_write(&mut self, _r: u8, _v: u8) -> bool {
        false
    }

    /// The queued writes that apply on this tick, if any (called every tick).
    fn due_writes(&mut self) -> Option<Vec<(u8, u8)>> {
        None
    }

    /// Whether a write to register `r` is taken at all now.
    fn accepts_write(&self, _core: &Core, _r: u8) -> bool {
        true
    }

    /// Just before register `r` takes `value` (already masked).
    fn before_write(&mut self, _core: &mut Core, _r: u8, _value: u8) {}

    /// Register `r` has just been written: the chip's own side effects.
    fn register_written(&mut self, _core: &mut Core, _r: u8, _previous: PreviousRegisters) {}

    /// R3.JIT (ACCC 14.4.4): an R3l write equal to the running C3l stops the HSYNC at once.
    fn supports_r3_jit(&self) -> bool {
        true
    }

    /// ACCC 17.5: an R1=0 written while C0=0 is still in time to raise the BORDER on CRTC 0, 1
    /// and 2 ("too late" on CRTC 3 and 4).
    fn acknowledges_r1_zero_on_c0_zero(&self) -> bool {
        true
    }

    // ---- HSYNC ----

    /// First thing on each character, before the HSYNC is counted.
    fn new_character(&mut self, _core: &mut Core) {}

    /// ACCC §14.4.2: writing R3l=0 while an HSYNC is running cancels it on CRTC 1; the others
    /// treat 0 as a count to reach (after overflowing).
    fn zero_r3_cancels_running_hsync(&self) -> bool {
        false
    }

    /// A character of the running HSYNC has been counted (C3l is up to date).
    fn hsync_counted(&mut self, _core: &mut Core) {}

    /// The HSYNC has just ended (its last character).
    fn hsync_ended(&mut self, _core: &mut Core) {}

    /// Each character, once the HSYNC count is done and before C0 moves.
    fn character_starts(&mut self, _core: &mut Core) {}

    /// Before C0=R2 is looked at, each character.
    fn before_hsync_check(&mut self, _core: &mut Core) {}

    /// C0=R2 does not start an HSYNC on this character.
    fn hsync_start_blocked(&self) -> bool {
        false
    }

    /// An HSYNC is starting (C0=R2).
    fn hsync_starting(&mut self, _core: &mut Core) {}

    /// `Some(blank)` while R3l=2 on CRTC 0: the Gate Array's blanking follows this window
    /// instead of the HSYNC pin.
    fn hsync_blank_window(&self, _core: &Core) -> Option<bool> {
        None
    }

    /// R3l=2 on a CRTC 0: its HSYNC is too short for the monitor (see the Gate Array).
    fn hsync_is_two_wide(&self, _core: &Core) -> bool {
        false
    }

    // ---- lines and frames ----

    /// CRTC 3/4: an R0 lowered below C0 ends the line at once (C0 >= R0); the other chips
    /// let C0 count on to 255 and wrap (AMSpiriT Lite and CPCSE, Shaker A3/A5-A7 R0 ruptures).
    fn line_ends_past_r0(&self) -> bool {
        false
    }

    /// C0 has just gone back to 0 through R0, before anything is counted (CRTC 0's R0=0 freeze).
    fn line_wraps(&mut self, _core: &mut Core) {}

    /// C0 has gone back to 0 through R0: move C9/C4 (and the adjustment); true when a new frame
    /// starts.
    fn count_line(&mut self, core: &mut Core) -> bool;

    /// After `count_line`: the states a chip evaluates at the start of a line (parities, CRTC 2's
    /// Last Line).
    fn line_started(&mut self, _core: &mut Core, _new_frame: bool, _c4_before: u8) {}

    /// Each character, after the line end (if any) has been counted.
    fn character_counted(&mut self, _core: &mut Core) {}

    /// Whether C9 is on the last line of the character row.
    fn c9_is_last(&self, core: &Core) -> bool {
        core.vlc == core.regs[9]
    }

    /// C9 as it goes out on RA0-RA4 to build the video address.
    fn c9_vma(&self, core: &Core) -> u8 {
        core.vlc.value()
    }

    // ---- BORDER ----

    /// The BORDER by R6, evaluated each character.
    fn border_r6(&mut self, core: &mut Core) -> bool {
        core.vcc >= core.regs[6]
    }

    /// CRTC 0 and 2 (ACCC 17.6.2): when C0 reaches R0 with the display still on (R1 not
    /// reached), they send BORDER ON to the Gate Array 0.5 usec too early: one byte of border
    /// before C0 returns to 0.
    fn anticipates_border_byte(&self) -> bool {
        false
    }

    /// ACCC 18.3.2: on the first line of a frame with R6=0, CRTC 0 and 2 alternate a byte of video
    /// and a byte of BORDER.
    fn has_r6_zero_conflict(&self) -> bool {
        false
    }

    // ---- VSYNC ----

    /// Each character, before a VSYNC start is looked at.
    fn before_vsync_check(&mut self, _core: &mut Core) {}

    /// The VSYNC may start on this character (C4=R7 holds).
    fn vsync_may_start_here(&self, _core: &Core) -> bool {
        true
    }

    /// The C4=R7 condition is blocked (CRTC 0's BLOCKED VSYNC).
    fn vsync_blocked(&self) -> bool {
        false
    }

    /// The VSYNC is refused this time and the C4=R7 occurrence is spent (CRTC 0, R0<2).
    fn vsync_refused(&self) -> bool {
        false
    }

    /// A VSYNC begun in the middle of a line does not count that line (CRTC 0, ACCC 16.4.1).
    fn vsync_counts_from_next_line(&self) -> bool {
        false
    }

    /// CRTC 2 (ACCC §16.4.3): a VSYNC condition met while an HSYNC runs makes a GHOST VSYNC.
    fn ghost_vsync_during_hsync(&self) -> bool {
        false
    }

    /// Each character, after the VSYNC start has been looked at.
    fn after_vsync_check(&mut self, _core: &mut Core) {}

    // ---- video pointer ----

    /// C0=R1 (a row's last line: VMA' takes the next row's address).
    fn r1_reached(&mut self, core: &mut Core) {
        if self.c9_is_last(core) {
            core.crtc_vma_prime = core.crtc_vma;
        }
    }

    /// At C0=0: VMA takes VMA' (the frame's first line takes R12/R13 first).
    fn reload_vma(&mut self, core: &mut Core) {
        if core.vcc == 0 && core.vlc == 0 {
            core.crtc_vma_prime = core.base_address();
        }
        core.crtc_vma = core.crtc_vma_prime;
    }

    /// Each character, once VMA has been reloaded (CRTC 1's R1=0 latch).
    fn after_vma_reload(&mut self, _core: &mut Core) {}

    // ---- tools and snapshots ----

    /// The counters have been restored from a snapshot.
    fn restored(&mut self, _core: &mut Core) {}

    /// The CRTC 0 "last line of the frame" state (diagnostics).
    fn last_line(&self) -> bool {
        false
    }
}
