//! The 6845-family CRTCs of the CPC (ACCC numbering 0 to 4).
//!
//! `core.rs` holds what every chip has (counters, registers, pins, video pointers), `model.rs`
//! the [`CrtcChip`] trait, and each chip its own state and behaviour: `type0.rs` (HD6845S),
//! `type1.rs` (UM6845R), `type2.rs` (MC6845) and `asic.rs` (CRTC 3 and 4). [`Crtc`] runs the
//! character clock and calls the chip where they differ.

mod asic;
mod core;
mod model;
mod type0;
mod type1;
mod type2;

pub use self::core::Core;
pub use model::{CrtcChip, CrtcType, DisplaySkew, ReadContext, Registers, NB_REGISTERS};
use model::PreviousRegisters;

use std::ops::{Deref, DerefMut};

use crate::chip::*;

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

/// A CRTC: the shared 6845 machinery ([`Core`]) driven at each character clock, and the chip
/// ([`CrtcChip`]: `type0.rs`, `type1.rs`, `type2.rs`, `asic.rs`) called wherever the chips
/// differ. Reads through [`Core`] (counters, sync states) work on it directly.
#[derive(Debug)]
pub struct Crtc {
    core: Core,
    chip: Box<dyn CrtcChip>,
}

impl Deref for Crtc {
    type Target = Core;

    fn deref(&self) -> &Core {
        &self.core
    }
}

impl DerefMut for Crtc {
    fn deref_mut(&mut self) -> &mut Core {
        &mut self.core
    }
}

impl Default for Crtc {
    fn default() -> Self {
        Crtc::from(CrtcType::default())
    }
}

impl From<&CrtcType> for Crtc {
    fn from(from: &CrtcType) -> Self {
        Crtc::from(*from)
    }
}

impl From<CrtcType> for Crtc {
    fn from(from: CrtcType) -> Self {
        let mut crtc = Crtc {
            core: Core::default(),
            chip: from.new_chip(),
        };
        crtc.reset();
        crtc
    }
}

impl CrtcType {
    /// A chip of this type, in its power-on state.
    pub fn new_chip(self) -> Box<dyn CrtcChip> {
        match self {
            CrtcType::Crtc0 => Box::new(type0::Crtc0::new()),
            CrtcType::Crtc1 => Box::new(type1::Crtc1::default()),
            CrtcType::Crtc2 => Box::new(type2::Crtc2::default()),
            CrtcType::Crtc3 | CrtcType::Crtc4 => Box::new(asic::Asic::new(self)),
        }
    }
}

impl Crtc {
    pub fn r0(&self) -> u8 {
        self.regs[0]
    }
    pub fn r1(&self) -> u8 {
        self.regs[1]
    }
    pub fn r2(&self) -> u8 {
        self.regs[2]
    }
    pub fn r3(&self) -> u8 {
        self.regs[3]
    }
    pub fn r4(&self) -> u8 {
        self.regs[4]
    }
    pub fn r5(&self) -> u8 {
        self.regs[5]
    }
    pub fn r6(&self) -> u8 {
        self.regs[6]
    }
    pub fn r7(&self) -> u8 {
        self.regs[7]
    }
    pub fn r8(&self) -> u8 {
        self.regs[8]
    }
    pub fn r9(&self) -> u8 {
        self.regs[9]
    }
    pub fn r10(&self) -> u8 {
        self.regs[10]
    }
    pub fn r11(&self) -> u8 {
        self.regs[11]
    }
    pub fn r12(&self) -> u8 {
        self.regs[12]
    }
    pub fn r13(&self) -> u8 {
        self.regs[13]
    }
    pub fn r14(&self) -> u8 {
        self.regs[14]
    }
    pub fn r15(&self) -> u8 {
        self.regs[15]
    }
    pub fn r16(&self) -> u8 {
        self.regs[16]
    }
    pub fn r17(&self) -> u8 {
        self.regs[17]
    }
    pub fn ar(&self) -> u8 {
        self.ar
    }

    /// Which chip this is.
    pub fn crtc_type(&self) -> CrtcType {
        self.chip.crtc_type()
    }

    /// The chip's name (HD6845S, UM6845R...).
    pub fn chip_name(&self) -> &'static str {
        self.chip.name()
    }
}

impl Crtc {
    /// Execute the CRTC register selection
    pub fn select_crtc_register(&mut self) {
        let reg = self.get_data_from_pins();
        self.core.ar = reg & 0b11111;
    }

    /// Execute the CRTC register write (the ASICs take it later, see `asic.rs`).
    pub fn write_crtc_register(&mut self) {
        let val = self.get_data_from_pins();
        let r = self.core.ar;
        if self.is_register_writable(r) && !self.chip.queue_write(r, val) {
            self.do_write_register(r, val);
        }
    }

    /// Execute the CRTC register status (read of &BE00): what the data bus
    /// carries is the chip's business.
    pub fn status_register(&mut self) {
        let data = self.chip.read_status(&Self::read_context(&mut self.core));
        self.set_data_on_pins(data);
    }

    /// Execute the CRTC register read (read of &BF00).
    pub fn read_crtc_register(&mut self) {
        let data = self.chip.read_register(&Self::read_context(&mut self.core));
        self.set_data_on_pins(data);
    }

    /// What a register read may depend on. Advances the noise source, as a
    /// floating bus would return something different each time.
    fn read_context(core: &mut Core) -> ReadContext<'_> {
        // xorshift32
        core.noise ^= core.noise << 13;
        core.noise ^= core.noise >> 17;
        core.noise ^= core.noise << 5;
        ReadContext {
            registers: &core.regs,
            selected: core.ar,
            vertical_border: core.vertical_border,
            noise: core.noise & 1 != 0
        }
    }

    /// Whether register `r` can be written on this CRTC (R0-R15; R16/R17
    /// are the read-only light pen).
    pub fn is_register_writable(&self, r: u8) -> bool {
        r <= 15
    }

    /// Store `v` in register `r`, keeping only the bits this chip has, with the side effects of
    /// the write. Registers that don't exist are ignored.
    pub fn do_write_register(&mut self, r: u8, v: u8) {
        {
            use std::sync::OnceLock;
            static TRACE: OnceLock<bool> = OnceLock::new();
            if *TRACE.get_or_init(|| std::env::var("AMSRUST_TRACE_REG").is_ok()) {
                eprintln!("W R{r}<-{v} C0={} C4={} C9={} ph={} hs={} hsc={}", self.hcc, self.vcc.value(), self.vlc.value(),
                    self.core.phase_ticks, self.core.hsync(), self.core.hsc);
            }
        }
        let (hcc, vcc, vlc) = (self.core.hcc, self.core.vcc.value(), self.core.vlc.value());
        if let Some(log) = self.core.write_log.as_mut() {
            log.push((hcc, vcc, vlc, r, v));
        }
        if !self.chip.accepts_write(&self.core, r) {
            return;
        }
        let mask = self.chip.write_mask(r);
        let previous = PreviousRegisters {
            r0: self.core.regs[0],
            r3l: self.core.regs[3] & 0x0f,
            r5: self.core.regs[5],
            r8: self.core.regs[8],
        };
        self.chip.before_write(&mut self.core, r, v & mask);
        if let Some(register) = self.core.regs.get_mut(usize::from(r)) {
            *register = v & mask;
        }
        self.chip.register_written(&mut self.core, r, previous);

        // R3.JIT, ACCC §14.4.4 / §14.5.4: R3l written (by an OUT(C),r8, on CRTC
        // 0, 1 and 2) with exactly the running C3l, while it was above it,
        // stops the HSYNC at once instead of letting the 4-bit counter wrap.
        if r == 3 && self.core.hsync() && self.chip.supports_r3_jit() {
            let new_r3l = self.core.regs[3] & 0x0f;
            if new_r3l == self.core.hsc && previous.r3l > self.core.hsc {
                self.core.off(CrtcPin::HS);
            }
        }
        // ACCC §17.5: R1=0 written after C0=0 is too late for this line
        if r == 1 && self.core.regs[1] == 0 && self.core.hcc > 0 {
            self.core.r1_zero_late = true;
        }
        // ACCC §17.4: "the condition C0=R1 is considered immediately on a
        // line", and it can be met several times in one line by reprogramming
        // R1: the video pointer is updated (on the last scan line of the
        // row) and the BORDER starts at once.
        if r == 1 && self.core.hcc == self.core.regs[1]
            && (self.core.regs[1] != 0 || self.core.hcc != 0 || self.chip.acknowledges_r1_zero_on_c0_zero())
        {
            self.chip.r1_reached(&mut self.core);
            self.core.border_history[0] = true;
            self.core.r1_latch = true;
            if let DisplaySkew::Delayed(chars) = self.chip.display_skew(self.core.regs[8]) {
                if self.core.border_history[usize::from(chars)] {
                    self.core.set_pin(CrtcPin::DE, false);
                }
            }
        }
        // R2.JIT, ACCC §14.7.1: "if the update of R2 occurs while C0=R2
        // (during the 3rd usec of an OUT(C),r8) then the CRTC sends the HSYNC
        // signal later to the Gate Array". The compare of C0 with R2 already
        // went by at the start of this character; the HSYNC it would have
        // raised starts now, part-way through it.
        if r == 2 && self.core.hcc == self.core.regs[2] && !self.core.hsync() && self.hsync_width().is_some() {
            let core = &mut self.core;
            core.hsc = 0;
            core.on(CrtcPin::HS);
            core.hdc = 0;
            // a write on the clock edge is taken before the character is counted: at the end of
            // the previous one
            core.hsync_late_ticks = if core.phase_ticks == 0 { 16 } else { core.phase_ticks };
        }
    }

    /// The motherboard tells where in the character clock this tick falls (every tick).
    pub fn set_phase_ticks(&mut self, phase: u8) {
        self.core.phase_ticks = phase;
        if self.get_pin(CrtcPin::CS) || self.get_pin(CrtcPin::EN) || self.get_pin(CrtcPin::RW) {
            self.core.write_taken = false;
        }
        if let Some(due) = self.chip.due_writes() {
            for (r, v) in due {
                self.do_write_register(r, v);
            }
        }
    }

    pub fn phase_ticks(&self) -> u8 {
        self.core.phase_ticks
    }

    /// Ticks by which the running HSYNC started late (R2.JIT), else 0.
    pub fn hsync_late_ticks(&self) -> u8 {
        self.core.hsync_late_ticks
    }

    /// Start recording the register writes (diagnostics).
    pub fn start_write_log(&mut self) {
        self.core.write_log = Some(Vec::new());
    }

    /// The writes recorded since the last call, as (C0, C4, C9, register, value).
    pub fn take_write_log(&mut self) -> Vec<(u8, u8, u8, u8, u8)> {
        self.core.write_log.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Register `r` as last written (0 past R17), for debuggers: unlike
    /// [`Self::do_read_register`], whatever the CRTC type lets the CPU read.
    pub fn register_value(&self, r: u8) -> u8 {
        self.regs.get(usize::from(r)).copied().unwrap_or(0)
    }

    /// The register the next data write goes to (diagnostics).
    pub fn selected_register(&self) -> u8 {
        self.ar
    }

    /// ACCC 17.6.2: on this character (C0=R0 reached with the display still
    /// on) only the first of its two bytes carries video, the second is
    /// BORDER (CRTC 0 and 2, without an R8 display skew).
    pub fn second_byte_is_border(&self) -> bool {
        let skew = (self.regs[8] >> 4) & 3;
        self.chip.anticipates_border_byte()
            && self.hcc == self.regs[0]
            && self.get_pin(CrtcPin::DE)
            && (skew == 0 || skew == 3)
    }

    /// ACCC 18.3.2: on the first line of a frame with R6=0, CRTC 0 and 2
    /// alternate a byte of video and a byte of BORDER (the second half of
    /// each character is BORDER) as long as R1 has not been reached.
    pub fn r6_conflict_second_byte_is_border(&self) -> bool {
        self.chip.has_r6_zero_conflict()
            && self.vcc == 0
            && self.vlc == 0
            && self.regs[6] == 0
            && self.get_pin(CrtcPin::DE)
    }

    /// The CRTC 0 "last line of the frame" state (diagnostics).
    pub fn last_line(&self) -> bool {
        self.chip.last_line()
    }

    /// Register `r` as stored (tools)
    pub fn regs_r(&self, r: usize) -> u8 {
        self.regs[r]
    }

    /// The value of C9 as it goes out on RA0-RA4.
    pub fn c9(&self) -> u8 {
        self.chip.c9_vma(&self.core)
    }

    /// The register as stored (no chip-dependent read behaviour: for that
    /// see [`Crtc::read_crtc_register`]).
    pub fn do_read_register(&self, r: u8) -> u8 {
        self.regs.get(usize::from(r)).copied().unwrap_or(0xff)
    }

    /// C0 has gone past R0: the line is over (on CRTC 3/4 also when R0 was lowered below C0).
    fn line_total_reached(&self) -> bool {
        self.hcc == self.regs[0].wrapping_add(1)
            || (self.chip.line_ends_past_r0() && self.regs[0] != 0xff && self.hcc > self.regs[0])
    }

    /// VSync in lines
    fn vsync_height(&self) -> u8 {
        self.chip.vsync_width(self.regs[3])
    }

    /// `Some(blank)` while R3l=2: the Gate Array's blanking follows this window instead of the HSYNC pin
    pub fn hsync_blank_window(&self) -> Option<bool> {
        self.chip.hsync_blank_window(&self.core)
    }

    /// R3l=2 on a CRTC 0: its HSYNC is too short for the monitor (see the Gate Array's mode latch)
    pub fn hsync_is_two_wide(&self) -> bool {
        self.chip.hsync_is_two_wide(&self.core)
    }

    /// HSync width in chars; `None` when no HSYNC is produced.
    fn hsync_width(&self) -> Option<u8> {
        self.chip.hsync_width(self.regs[3])
    }

    /// Run all the CRTC machinery for one character clock.
    fn update_memory_address(&mut self) {
        let chip = &mut self.chip;
        let core = &mut self.core;
        chip.new_character(core);

        // ACCC §14.4: C3l is a 4-bit counter. Lowering R3l below it during HSYNC (R3.JIT) makes
        // it overflow and reach R3l on the next lap (R3l=0 included: "0 is treated as a value to
        // reach"), except on CRTC 1 where R3l=0 cancels the HSYNC.
        let hsync_before = core.hsync();
        if core.hsync() {
            core.hsc = (core.hsc + 1) & 0x0f;
            chip.hsync_counted(core);
            let r3l = core.regs[3] & 0x0f;
            if core.hsc == r3l || (r3l == 0 && chip.zero_r3_cancels_running_hsync()) {
                chip.hsync_ended(core);
                core.off(CrtcPin::HS);
            }
        }
        chip.character_starts(core);

        core.hcc = core.hcc.wrapping_add(1);
        core.hdc = core.hdc.wrapping_add(1);
        // CRTC-VMA (ACCC §17.3): a real counter, advanced once per character whatever the display
        // state, so a mid-frame R1/R12/R13 change only affects rows from that point on.
        core.crtc_vma = core.crtc_vma.wrapping_add(1) & 0x3fff;

        // ACCC §17.1/§17.4: VMA' takes where the next row starts the moment this row's active
        // portion ends (C0=R1) on its last scan line. `==` rather than `>=`: a mid-line R1 rewrite
        // (ACCC §17.3) can make it true a second time on the same line. When the counter does not
        // reach R0+1 it wraps to 0 instead (ACCC §17.2.2: with R1>R0, VMA' is stuck).
        let line_end = core.hcc == core.regs[0].wrapping_add(1)
            || (chip.line_ends_past_r0() && core.regs[0] != 0xff && core.hcc > core.regs[0]);
        if core.hcc == core.regs[1] && !line_end {
            chip.r1_reached(core);
        }

        if line_end {
            core.hcc = 0;
            core.line_ends += 1;

            // the end of the VSYNC, independent of the vertical adjustment (R3 and R5 are
            // unrelated and can overlap)
            if core.vsync() {
                if core.vsync_skips_first_line_end {
                    core.vsync_skips_first_line_end = false;
                } else {
                    // a 4 bit counter: an R3 lowered below the count makes it go round
                    // (16 lines more), and a width of 16 is the counter back at 0
                    core.vsc = (core.vsc + 1) & 0x0f;
                    if core.vsc == chip.vsync_width(core.regs[3]) & 0x0f {
                        core.vsync_running = false;
                        core.off(CrtcPin::VS);
                        core.vsync_ended_on_r7 = core.vcc == core.regs[7];
                    }
                }
            }

            chip.line_wraps(core);
            let c4_before = core.vcc.value();
            let adjust_before = core.in_vertical_adjust;
            let new_frame = chip.count_line(core);
            // a new row (or frame, C4 back to 0 with R4=0, or adjustment, C4 kept on R4 on
            // CRTC 3/4) may start a VSYNC again
            if new_frame || core.vcc.value() != c4_before || (core.in_vertical_adjust && !adjust_before) {
                core.vsync_ended_on_r7 = false;
            }
            chip.line_started(core, new_frame, c4_before);

            // Sampled once per line, at C0=R0 (ACCC §21.3.3); an R6 of 0
            // while the frame is running is not detected by the chip.
            core.vertical_border = core.regs[6] != 0 && core.vcc >= core.regs[6];
        }
        chip.character_counted(core);

        // ACCC §19.2.3/§19.2.4: the horizontal BORDER condition (C0 reached R1 - or R0 if R1 was
        // not reached, CRTC 0 and 2) goes through a delay line of 0 to 2 characters set by the R8
        // skew field.
        let skew = chip.display_skew(core.regs[8]);
        let replaced_by_r0 = chip.anticipates_border_byte()
            && core.hcc == core.regs[0]
            && matches!(skew, DisplaySkew::Delayed(d) if d > 0);
        core.border_history.rotate_right(1);
        // ACCC §17.3-§17.5: on every CRTC the BORDER R1 is a state set by the C0=R1 equality (or
        // R1=0), cleared when a line starts; raising R1 afterwards does not bring the display back
        // before the next line, and an R1 lowered under C0 does not set it (Shaker AO_H on CRTC
        // 2, 3, 4: R1=0 then R1=40 during a line)
        if core.hcc == 0 {
            core.r1_zero_late = false;
            core.r1_latch = false;
        }
        if core.hcc == core.regs[1] || (core.regs[1] == 0 && !core.r1_zero_late) {
            core.r1_latch = true;
        }
        let by_r1 = core.r1_latch;
        core.border_history[0] = by_r1 || replaced_by_r0;
        let horizontal_border = match skew {
            DisplaySkew::Delayed(chars) => core.border_history[usize::from(chars)],
            DisplaySkew::Blanked => true,
        };
        let vertical_border = chip.border_r6(core);
        core.set_pin(CrtcPin::DE, !(horizontal_border || vertical_border));

        // ACCC 15.1: during a HSYNC the C0=R2 condition is not managed, so a HSYNC cannot restart
        // within a HSYNC (and a zero width produces none on the chips that treat it that way)
        chip.before_hsync_check(core);
        if core.hcc == core.regs[2] && !core.hsync() && !chip.hsync_start_blocked()
            && chip.hsync_width(core.regs[3]).is_some()
        {
            chip.hsync_starting(core);
            core.hsc = 0;
            core.on(CrtcPin::HS);
            core.hsync_late_ticks = 0;
            core.hdc = 0;
        }

        // ACCC §19.7.2: in interlace, on an even frame the VSYNC starts in the middle of the line
        // (C0=R0/2)
        let mid_vsync_wait = core.interlaced() && !core.parity_frame && core.hcc < core.regs[0] / 2;
        chip.before_vsync_check(core);
        if core.vcc != core.regs[7] {
            core.vsync_ended_on_r7 = false;
        }
        let vsync_wanted = core.vcc == core.regs[7]
            && !core.vsync()
            && !core.vsync_ended_on_r7
            && !mid_vsync_wait
            && chip.vsync_may_start_here(core)
            && !chip.vsync_blocked();
        if vsync_wanted && chip.vsync_refused() {
            // ACCC §13.2.2: refused, and this C4=R7 occurrence is spent
            core.vsync_ended_on_r7 = true;
        } else if vsync_wanted {
            core.vsc = 0;
            // ACCC 16.4.1: the line counter of a VSYNC begun mid-line starts at the next C0=0
            core.vsync_skips_first_line_end = chip.vsync_counts_from_next_line() && core.hcc > 0;
            core.vsync_running = true;
            core.fields = core.fields.wrapping_add(1);
            // the HSYNC window runs up to the character where the HSYNC ends
            let ghost = chip.ghost_vsync_during_hsync() && core.regs[2] != 0 && (core.hsync() || hsync_before);
            core.set_pin(CrtcPin::VS, !ghost);
        }
        chip.after_vsync_check(core);

        // ACCC §17.4/§20.3: at the start of every line VMA takes VMA' (the frame's first line
        // is special-cased per chip)
        if core.hcc == 0 {
            chip.reload_vma(core);
        }
        chip.after_vma_reload(core);
        self.set_address_on_pins();
    }

    /// MA and RA are variables manipulated during the process
    /// they are directly wired to the pin; this fonction replicate them here
    /// should not be called outside of Crtc. Public only for test reasons
    pub fn set_address_on_pins(&mut self) {
        self.set_value_in_pins(
            self.core.crtc_vma,
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

        let c9 = self.chip.c9_vma(&self.core);
        self.set_value_in_pins(
            c9,
            &[
                CrtcPin::RA4,
                CrtcPin::RA3,
                CrtcPin::RA2,
                CrtcPin::RA1,
                CrtcPin::RA0,
            ],
        )
    }

    /// Seed `crtc_vma`/`crtc_vma_prime` as if `row` whole character rows had
    /// already played out from `base_address` with R1 held constant the
    /// whole time - a shortcut for tests that want to start mid-frame (at
    /// the current `hcc`/`vlc`) without actually driving every tick of the
    /// rows before it. Public only for test reasons.
    pub fn seed_vma_for_test(&mut self, base_address: u16, row: u16) {
        let stride = self.regs[1] as u16;
        let row_start = base_address.wrapping_add(row.wrapping_mul(stride)) & 0x3fff;
        self.core.crtc_vma = row_start.wrapping_add(self.hcc as u16) & 0x3fff;
        self.core.crtc_vma_prime = row_start.wrapping_add(stride) & 0x3fff;
    }
}

/// The CRTC counters of a version 3 snapshot (offsets &A9-&B1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrtcCounters {
    /// C0
    pub hcc: u8,
    /// C4
    pub vcc: u8,
    /// C9
    pub vlc: u8,
    /// the vertical total adjust counter
    pub vtac: u8,
    /// C3l
    pub hsc: u8,
    /// C3h
    pub vsc: u8,
    pub hsync: bool,
    pub vsync: bool,
    pub in_vertical_adjust: bool
}

impl Crtc {
    /// Restores the CRTC from a snapshot: the registers as stored (no write side
    /// effect: they are not written in the middle of a frame), the selected
    /// register and, from a version 3 snapshot, the counters. Without counters
    /// the CRTC restarts at the top of a frame.
    pub fn restore(&mut self, regs: &[u8; 18], selected: u8, counters: Option<CrtcCounters>) {
        self.reset();
        for (r, &v) in regs.iter().enumerate() {
            self.core.regs[r] = v & self.chip.write_mask(r as u8);
        }
        let c = counters.unwrap_or_default();
        let core = &mut self.core;
        core.ar = selected & 0b11111;
        core.hcc = c.hcc;
        core.vcc = (c.vcc & 0x7f).into();
        core.vlc = (c.vlc & 0x1f).into();
        core.vtac = c.vtac;
        core.hsc = c.hsc & 0x0f;
        core.vsc = c.vsc & 0x0f;
        core.in_vertical_adjust = c.in_vertical_adjust;
        core.vsync_running = c.vsync;
        core.set_pin(CrtcPin::VS, c.vsync);
        core.set_pin(CrtcPin::HS, c.hsync);
        self.chip.restored(&mut self.core);
        // VMA as the standard frame has it: the row starts R1 characters after the previous one
        let row = if c.in_vertical_adjust { 0 } else { u16::from(c.vcc & 0x7f) };
        let row_start = self.core.base_address().wrapping_add(row * u16::from(self.regs[1])) & 0x3fff;
        self.core.crtc_vma = row_start.wrapping_add(u16::from(c.hcc)) & 0x3fff;
        self.core.crtc_vma_prime = if self.chip.c9_is_last(&self.core) && c.hcc > self.regs[1] {
            row_start.wrapping_add(u16::from(self.regs[1])) & 0x3fff
        } else {
            row_start
        };
    }

    /// The registers as stored, as a snapshot stores them.
    pub fn registers(&self) -> [u8; 18] {
        let mut regs = [0; 18];
        regs.copy_from_slice(&self.regs[..18]);
        regs
    }

    /// The counters, as a version 3 snapshot stores them.
    pub fn counters(&self) -> CrtcCounters {
        CrtcCounters {
            hcc: self.hcc,
            vcc: self.vcc.value(),
            vlc: self.vlc.value(),
            vtac: self.vtac,
            hsc: self.hsc,
            vsc: self.vsc,
            hsync: self.hsync(),
            vsync: self.vsync(),
            in_vertical_adjust: self.in_vertical_adjust
        }
    }

    /// The CURSOR output (on the CPC, only on the expansion port: the PlayCity counts it): in
    /// the displayed area, MA on R14:R15 and RA within R10..R11, as R10's bits 6-5 make it blink
    /// (00 steady, 01 off, 10 every 16 fields, 11 every 32). The skew of R8 is not applied.
    pub fn cursor(&self) -> bool {
        if !self.get_pin(CrtcPin::DE) {
            return false;
        }
        let core = &self.core;
        let address = u16::from(core.regs[14] & 0x3f) << 8 | u16::from(core.regs[15]);
        if core.crtc_vma & 0x3fff != address {
            return false;
        }
        let ra = self.chip.c9_vma(core);
        if ra < core.regs[10] & 0x1f || ra > core.regs[11] & 0x1f {
            return false;
        }
        match core.regs[10] >> 5 & 3 {
            0 => true,
            1 => false,
            2 => core.fields & 8 != 0,
            _ => core.fields & 16 != 0
        }
    }

    ///  	The Video display address. This is derived from the MA register in the CRTC, the horizontal character count HCC and the current row address (RA) register in the CRTC. This is effectively the CPC memory address of the data displayed on the monitor.
    pub fn vma(&self) -> u16 {
        self.get_address_from_pins()
    }
}

impl Chip for Crtc {
    type PinType = CrtcPin;

    fn pins_buffer_mut(&mut self) -> &mut PinsBuffer {
        &mut self.core.pins
    }
    fn pins_buffer(&self) -> &PinsBuffer {
        &self.core.pins
    }

    fn must_execute_tick(&self) -> bool {
        !self.get_pin(CrtcPin::CS) & !self.get_pin(CrtcPin::EN)
            && (self.get_pin(CrtcPin::RW) || self.chip.samples_write_at(self.core.phase_ticks))
    }

    /// Execute the requested command from the CPU
    fn execute_tick(&mut self) {
        // a write is latched once per bus cycle, however long the CPU holds it
        if !self.get_pin(CrtcPin::RW) {
            if self.core.write_taken {
                return;
            }
            self.core.write_taken = true;
        }
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
        self.core.reset();
        self.chip.reset(&mut self.core);
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

        assert_eq!(crtc.regs[1], 40);
        assert_eq!(crtc.regs[4], 38);
        assert_eq!(crtc.regs[9], 7);

        crtc.set_pin(CrtcPin::CCLK, true);
        crtc.set_pin(CrtcPin::CS, false);
        crtc.set_pin(CrtcPin::RW, false);
        crtc.set_pin(CrtcPin::RS, false);
        crtc.set_pin(CrtcPin::EN, false);

        crtc.regs[12] = 0x30;
        crtc.regs[13] = 0x00;
        crtc.vcc.reset();
        crtc.vlc.reset();
        crtc.hcc = 0;
        crtc.crtc_vma = 0x3000;
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

        assert_eq!(crtc.regs[1], 40);
        assert_eq!(crtc.regs[4], 38);
        assert_eq!(crtc.regs[9], 7);

        crtc.set_pin(CrtcPin::CCLK, true);
        crtc.set_pin(CrtcPin::CS, false);
        crtc.set_pin(CrtcPin::RW, false);
        crtc.set_pin(CrtcPin::RS, false);
        crtc.set_pin(CrtcPin::EN, false);

        crtc.regs[12] = 0x00;
        crtc.regs[13] = 0x00;
        crtc.vcc.reset();
        crtc.vlc.reset();
        crtc.hcc = 0;
        crtc.crtc_vma = 0x0000;
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

        for char_line in 0..=crtc.regs[4] {
            for scan_line in 0..=crtc.regs[9] {
                for char in 0..=crtc.regs[0] {
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
                    assert!(crtc.vcc <= crtc.regs[4]);
                    assert!(crtc.vlc.value() <= crtc.regs[9]);
                    assert!(crtc.hcc <= crtc.regs[0]);

                    assert!(crtc.vcc == char_line);
                    assert!(crtc.vlc.value() == scan_line);
                    assert!(crtc.hcc == char);

                    // Test for border
                    if crtc.hcc >= crtc.regs[1] || crtc.vcc >= crtc.regs[6] {
                        assert!(!crtc.get_pin(CrtcPin::DE));
                    } else {
                        assert!(crtc.get_pin(CrtcPin::DE));
                    }

                    // Test for hsync
                    if crtc.hcc >= crtc.regs[2] && crtc.hcc < crtc.regs[2] + crtc.hsync_width().unwrap() {
                        assert!(crtc.get_pin(CrtcPin::HS));
                    } else {
                        assert!(!crtc.get_pin(CrtcPin::HS));
                    }

                    // VSYNC duration is expressed in raster lines, not in character lines.
                    let raster_line = (crtc.vcc.value() as u16) * (crtc.regs[9] as u16 + 1)
                        + crtc.vlc.value() as u16;
                    let vsync_start = (crtc.regs[7] as u16) * (crtc.regs[9] as u16 + 1);
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
            assert_eq!(Some(hsync_duration), crtc.hsync_width());

            let mut hsync_wait = 0;
            while !crtc.hsync() {
                hsync_wait += 1;
                crtc.update_memory_address();
            }
            assert_eq!(hsync_duration + hsync_wait, 64);
        }
    }

    /// ACCC §14.4: lowering R3l below the running 4-bit HSYNC counter makes
    /// it overflow, so the HSYNC lasts until the counter reaches R3l again
    /// (16 - old + new chars from its start), not 256 chars.
    #[test]
    fn test_hsync_r3_lowered_during_hsync_overflows() {
        let mut crtc = Crtc::default();
        crtc.update_memory_address();
        while !crtc.hsync() {
            crtc.update_memory_address();
        }
        // let the counter run past the new value, then lower R3l below it
        for _ in 0..5 {
            crtc.update_memory_address();
        }
        assert!(crtc.hsync());
        crtc.regs[3] = (crtc.regs[3] & 0xf0) | 2;
        let mut extra = 0;
        while crtc.hsync() {
            extra += 1;
            assert!(extra <= 16, "HSYNC never ended");
            crtc.update_memory_address();
        }
        assert!(extra > 5, "HSYNC must overflow the 4-bit counter, got {extra}");
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

    fn crtc0() -> Crtc {
        Crtc::from(CrtcType::Crtc0)
    }

    fn step_until_hcc(crtc: &mut Crtc, hcc: u8) {
        for _ in 0..100_000 {
            crtc.update_memory_address();
            if crtc.hcc == hcc {
                return;
            }
        }
        panic!("C0 never reached {hcc}");
    }

    /// ACCC §17.4/§17.5: once the BORDER R1 has started on a line (here an
    /// R1=0 on C0=0), raising R1 does not bring the display back before the
    /// next line.
    #[test]
    fn test_crtc0_border_r1_is_latched_for_the_line() {
        let mut crtc = crtc0();
        step_until_hcc(&mut crtc, 0);
        crtc.do_write_register(1, 0);
        assert!(!crtc.get_pin(CrtcPin::DE));
        step_until_hcc(&mut crtc, 3);
        crtc.do_write_register(1, 40);
        step_until_hcc(&mut crtc, 8);
        assert!(!crtc.get_pin(CrtcPin::DE), "the border must stay until the next line");
        step_until_hcc(&mut crtc, 2);
        assert!(crtc.get_pin(CrtcPin::DE), "the next line displays again");
    }

    /// ACCC §13.7.2.1: R0 enlarged out of 1 on C0=1 while C9=R9 overflows C4
    /// on C0=2, even if C4<>R4.
    #[test]
    fn test_crtc0_r0_enlarged_from_one_steps_c4() {
        let mut crtc = crtc0();
        crtc.do_write_register(9, 0);
        crtc.do_write_register(4, 100);
        crtc.do_write_register(0, 1);
        step_until_hcc(&mut crtc, 1);
        while crtc.vlc.value() != 0 {
            step_until_hcc(&mut crtc, 1);
        }
        let before = crtc.vcc.value();
        crtc.do_write_register(0, 63);
        crtc.update_memory_address();
        assert_eq!(crtc.hcc, 2);
        assert_eq!(crtc.vcc.value(), (before + 1) & 0x7f);
    }

    /// ACCC §13.2.4: R0=0 written on the C0=0 that has just updated the counters makes it the
    /// first C0=0 with R0=0 (C9<>R9: nothing more moves), then C9 and C4 are frozen (Shaker A7 and
    /// BCAPS B1 on the real CPC).
    #[test]
    fn test_crtc0_r0_zero_freezes_the_counters() {
        let mut crtc = crtc0();
        step_until_hcc(&mut crtc, 0);
        for _ in 0..3 * 64 {
            crtc.update_memory_address();
        }
        step_until_hcc(&mut crtc, 0);
        let (c4, c9) = (crtc.vcc.value(), crtc.vlc.value());
        assert!(c9 < 6, "need room to count");
        crtc.do_write_register(0, 0);
        for _ in 0..50 {
            crtc.update_memory_address();
        }
        assert_eq!(crtc.vcc.value(), c4);
        assert_eq!(crtc.vlc.value(), c9);
    }

    /// One line of 64 characters
    fn run_line(crtc: &mut Crtc) {
        let ends = crtc.line_ends;
        while crtc.line_ends == ends {
            crtc.update_memory_address();
        }
    }

    /// The VSYNC line counter is 4 bits: an R3 lowered below the count makes it
    /// go round.
    #[test]
    fn test_crtc0_vsync_counter_goes_round_when_r3_is_lowered() {
        let mut crtc = crtc0();
        while !crtc.vsync() {
            crtc.update_memory_address();
        }
        for _ in 0..4 {
            run_line(&mut crtc);
        }
        crtc.do_write_register(3, 0x20 | 0x0e);
        let mut more = 0;
        while crtc.vsync() {
            run_line(&mut crtc);
            more += 1;
        }
        assert_eq!(more, 16 - 4 + 2);
    }

    /// ACCC §11.2.2: on CRTC 0 C9 itself counts the additional lines, and goes
    /// out on RA.
    #[test]
    fn test_crtc0_c9_counts_the_additional_lines() {
        let mut crtc = crtc0();
        crtc.do_write_register(5, 3);
        let mut seen = Vec::new();
        for _ in 0..400 {
            run_line(&mut crtc);
            if crtc.vcc.value() == crtc.regs[4] + 1 {
                seen.push(crtc.c9());
            }
        }
        assert!(seen.starts_with(&[0, 1, 2]), "{seen:?}");
    }

    /// ACCC §16.4.1: a VSYNC triggered by R7 in the middle of a line does not
    /// count that line.
    #[test]
    fn test_crtc0_vsync_started_mid_line_lasts_one_line_more() {
        let mut crtc = crtc0();
        crtc.do_write_register(7, 99);
        while crtc.vcc.value() != 31 {
            run_line(&mut crtc);
        }
        step_until_hcc(&mut crtc, 20);
        crtc.do_write_register(7, 31);
        crtc.update_memory_address();
        assert!(crtc.vsync());
        let mut lines = 0;
        while crtc.vsync() {
            run_line(&mut crtc);
            lines += 1;
        }
        assert_eq!(lines, 1 + 8);
    }

    /// Steps character by character, with a bound so a missing VSYNC fails instead of hanging.
    fn step_while(crtc: &mut Crtc, vsync: bool) {
        for _ in 0..1_000_000 {
            if crtc.vsync() != vsync {
                return;
            }
            crtc.update_memory_address();
        }
        panic!("VSYNC stuck {}", if vsync { "high" } else { "low" });
    }

    /// (VSYNC length, VSYNC period) in scan lines, after a few frames to settle.
    fn vsync_and_frame_lines(crtc: &mut Crtc) -> (u64, u64) {
        for _ in 0..3 {
            step_while(crtc, false);
            step_while(crtc, true);
        }
        step_while(crtc, false);
        let start = crtc.line_ends;
        step_while(crtc, true);
        let high = crtc.line_ends - start;
        step_while(crtc, false);
        (high, crtc.line_ends - start)
    }

    fn crtc_with(kind: CrtcType, regs: &[(u8, u8)]) -> Crtc {
        let mut crtc = Crtc::from(kind);
        for &(r, v) in regs {
            crtc.do_write_register(r, v);
        }
        crtc
    }

    const ALL_TYPES: [CrtcType; 5] = [CrtcType::Crtc0, CrtcType::Crtc1, CrtcType::Crtc2, CrtcType::Crtc3, CrtcType::Crtc4];

    /// C9 is a 5-bit counter: rows of 32 lines (R9=31) still end, on every CRTC
    /// (AMSpiriT Lite and CPCSE: 8 rows of 32 lines make a 256-line frame).
    #[test]
    fn test_rows_of_32_lines() {
        for kind in ALL_TYPES {
            let mut crtc = crtc_with(kind, &[(9, 31), (4, 7), (7, 2), (6, 6)]);
            let width = u64::from(crtc.vsync_height());
            assert_eq!(vsync_and_frame_lines(&mut crtc), (width, 8 * 32), "{kind:?}");
        }
    }

    /// C4 is a 7-bit counter: with R4=127 the frame still ends after row 127 and
    /// takes its R5 lines (AMSpiriT Lite and CPCSE: 128 + 4 lines).
    #[test]
    fn test_r4_127_keeps_the_adjust_lines() {
        for kind in ALL_TYPES {
            let mut crtc = crtc_with(kind, &[(9, 0), (4, 127), (5, 4), (7, 100), (6, 50)]);
            assert_eq!(vsync_and_frame_lines(&mut crtc).1, 128 + 4, "{kind:?}");
        }
    }

    /// A VSYNC that ends while C4 still equals R7 does not start again on the
    /// same row (AMSpiriT Lite and CPCSE: R3=&1E gives 1 line, not the whole row).
    #[test]
    fn test_vsync_does_not_restart_on_the_same_row() {
        for kind in [CrtcType::Crtc0, CrtcType::Crtc3, CrtcType::Crtc4] {
            for (r3, lines) in [(0x1e, 1), (0x4e, 4)] {
                let mut crtc = crtc_with(kind, &[(3, r3)]);
                assert_eq!(vsync_and_frame_lines(&mut crtc), (lines, 39 * 8), "{kind:?} R3={r3:#x}");
            }
        }
        // rows longer than the 16 lines of a CRTC 1/2 VSYNC
        for kind in ALL_TYPES {
            let mut crtc = crtc_with(kind, &[(9, 31), (4, 7), (7, 2), (6, 6)]);
            let width = u64::from(crtc.vsync_height());
            assert_eq!(vsync_and_frame_lines(&mut crtc).0, width, "{kind:?}");
        }
    }

    /// One-row frames (R4=0): C4 stays 0, yet each frame has its VSYNC. On CRTC 3/4
    /// C4 also stays 0 for the R5 lines (ACCC §18.2.4), where a VSYNC starts again and
    /// runs on into the next frame's: 16 lines (AMSpiriT Lite and CPCSE).
    #[test]
    fn test_vsync_every_frame_of_a_single_row() {
        for kind in ALL_TYPES {
            let mut crtc = crtc_with(kind, &[(4, 0), (9, 31), (5, 8), (7, 0), (6, 0)]);
            let width = u64::from(crtc.vsync_height());
            let high = if matches!(kind, CrtcType::Crtc3 | CrtcType::Crtc4) { 8 + width } else { width };
            assert_eq!(vsync_and_frame_lines(&mut crtc), (high, 32 + 8), "{kind:?}");
        }
    }

    /// A reset forgets the transient state of the previous run.
    #[test]
    fn test_reset_clears_the_vsync_state() {
        let mut crtc = crtc_with(CrtcType::Crtc0, &[(3, 0x1e)]);
        step_while(&mut crtc, false);
        step_while(&mut crtc, true);
        crtc.reset();
        assert!(!crtc.vsync_ended_on_r7 && !crtc.chip.vsync_blocked() && !crtc.vsync_skips_first_line_end);
    }

    /// ACCC §13.2.2: a CRTC 0 line must reach C0=2 for the next C0=0 to start a
    /// VSYNC, so with R0=1 there is none (AMSpiriT Lite and CPCSE); CRTC 1 has it.
    #[test]
    fn test_crtc0_no_vsync_with_r0_1() {
        let regs = [(0, 1), (1, 1), (2, 1), (3, 0x81)];
        let mut crtc = crtc_with(CrtcType::Crtc0, &regs);
        for _ in 0..200_000 {
            crtc.update_memory_address();
            assert!(!crtc.vsync());
        }
        let mut crtc = crtc_with(CrtcType::Crtc1, &regs);
        step_while(&mut crtc, false);
    }

    /// ACCC §16.4.3: on CRTC 2 a VSYNC condition met during the HSYNC (R0=31, the
    /// HSYNC of R2=24 runs past C0=0) is a GHOST VSYNC: it runs, the pin stays low
    /// (CPCSE). With R2=0 it is a real one.
    #[test]
    fn test_crtc2_ghost_vsync_during_hsync() {
        let mut crtc = crtc_with(CrtcType::Crtc2, &[(0, 31), (1, 20), (2, 24)]);
        let (ran, _) = vsync_and_frame_lines(&mut crtc);
        assert_eq!(ran, 16);
        for _ in 0..100_000 {
            crtc.update_memory_address();
            assert!(!crtc.get_pin(CrtcPin::VS));
        }
        let mut crtc = crtc_with(CrtcType::Crtc2, &[(0, 31), (1, 20), (2, 0)]);
        step_while(&mut crtc, false);
        assert!(crtc.get_pin(CrtcPin::VS));
        // the other chips have no ghost
        let mut crtc = crtc_with(CrtcType::Crtc0, &[(0, 31), (1, 20), (2, 24)]);
        step_while(&mut crtc, false);
        assert!(crtc.get_pin(CrtcPin::VS));
    }

    /// Interlace sync (R8=1, and R8=3 on CRTC 2): VSYNC every 312.5 lines, the
    /// half line alternating (AMSpiriT Lite and CPCSE), on CRTC 1 to 4.
    #[test]
    fn test_interlace_sync_on_crtc_1_to_4() {
        for kind in [CrtcType::Crtc1, CrtcType::Crtc2, CrtcType::Crtc3, CrtcType::Crtc4] {
            for r8 in [1, 3] {
                if r8 == 3 && kind != CrtcType::Crtc2 {
                    continue; // interlace video: CRTC 1 has its own, CRTC 3/4 not emulated
                }
                let mut crtc = crtc_with(kind, &[(8, r8)]);
                vsync_and_frame_lines(&mut crtc);
                // characters between VSYNC starts: 312.5 lines of 64 on average
                let mut chars = Vec::new();
                for _ in 0..4 {
                    let mut n = 0u64;
                    while crtc.vsync() {
                        crtc.update_memory_address();
                        n += 1;
                    }
                    while !crtc.vsync() {
                        crtc.update_memory_address();
                        n += 1;
                    }
                    chars.push(n);
                }
                assert_eq!(chars.iter().sum::<u64>(), 4 * 20000, "{kind:?} R8={r8}: {chars:?}");
            }
        }
    }

    /// CRTC 3/4 end the line as soon as R0 is lowered below C0; CRTC 0 lets C0 count on to
    /// 255 (AMSpiriT Lite and CPCSE, Shaker A5-A7: R0=3 ruptures).
    #[test]
    fn test_asic_line_ends_when_r0_is_lowered_below_c0() {
        for (kind, at_once) in [(CrtcType::Crtc4, true), (CrtcType::Crtc3, true), (CrtcType::Crtc0, false)] {
            let mut crtc = crtc_with(kind, &[]);
            while crtc.hcc != 40 {
                crtc.update_memory_address();
            }
            let ends = crtc.line_ends;
            crtc.do_write_register(0, 10);
            crtc.update_memory_address();
            assert_eq!(crtc.line_ends == ends + 1, at_once, "{kind:?}");
            if !at_once {
                assert_eq!(crtc.hcc, 41);
            }
        }
    }

    /// CRTC 2 (ACCC §13.4): the frame starts at VMA', which takes R12/R13 at C0=R1 on the
    /// frame's last line. R12/R13 written after that only count from the frame after
    /// (AMSpiriT Lite and CPCSE: Shaker AO, R1>R0, starts every frame from the stale VMA').
    /// CRTC 0 takes R12/R13 at the frame's first line.
    #[test]
    fn test_crtc2_frame_starts_at_vma_prime() {
        for (kind, delayed) in [(CrtcType::Crtc2, true), (CrtcType::Crtc0, false)] {
            let mut crtc = crtc_with(kind, &[]);
            // the last line of the frame, past C0=R1
            while !(crtc.vcc == crtc.regs[4] && crtc.vlc == crtc.regs[9] && crtc.hcc == crtc.regs[1] + 2) {
                crtc.update_memory_address();
            }
            crtc.do_write_register(12, 0x10);
            while !(crtc.vcc == 0 && crtc.vlc == 0 && crtc.hcc == 1) {
                crtc.update_memory_address();
            }
            let expected = if delayed { 0x3000 } else { 0x1000 };
            assert_eq!(crtc.crtc_vma & 0x3fc0, expected, "{kind:?}");
        }
    }

    /// CRTC 1: R6=0 is a live BORDER. Set on the last line, it is not latched by the
    /// next frame's C4=R6=0: R6 back to 25 brings the display back (Shaker A3,
    /// AMSpiriT Lite and CPCSE). Written on the first line of a frame it is final.
    #[test]
    fn test_crtc1_r6_zero_border_is_not_latched() {
        let mut crtc = crtc_with(CrtcType::Crtc1, &[]);
        while !(crtc.vcc == crtc.regs[4] && crtc.vlc == crtc.regs[9]) {
            crtc.update_memory_address();
        }
        crtc.do_write_register(6, 0);
        while crtc.vcc != 1 {
            crtc.update_memory_address();
            assert!(!crtc.get_pin(CrtcPin::DE) || crtc.hcc >= crtc.regs[1] || crtc.vcc != 0, "R6=0: border");
        }
        crtc.do_write_register(6, 25);
        while crtc.hcc != 5 {
            crtc.update_memory_address();
        }
        assert!(crtc.get_pin(CrtcPin::DE), "R6 back to 25: display");

        // R6=0 written on the first line of the frame: BORDER until the next frame
        while !(crtc.vcc == 0 && crtc.vlc == 0 && crtc.hcc == 5) {
            crtc.update_memory_address();
        }
        crtc.do_write_register(6, 0);
        while crtc.vcc != 1 {
            crtc.update_memory_address();
        }
        crtc.do_write_register(6, 25);
        while crtc.hcc != 5 {
            crtc.update_memory_address();
        }
        assert!(!crtc.get_pin(CrtcPin::DE), "R6=0 on C4=C9=0: final border");
    }
}
