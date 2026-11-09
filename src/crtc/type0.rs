//! CRTC 0: Hitachi HD6845S, and the UMC UM6845 (which behaves exactly like
//! it, ACCC §4.2 note 3).
//!
//! Its vertical logic works on precise positions of C0 (ACCC §13.2.1): on C0=0 and 1 it decides
//! whether the line is the frame's last one (a latched state), on C0=2 whether the additional
//! management (R5 lines, interlace line) goes on; with R0<2 those positions are never reached and
//! the counters freeze (R0=0) or overflow (R0=1). It also has the BLOCKED VSYNC of ACCC 16.4.1.

use super::core::Core;
use super::model::{pointer_mask, CrtcChip, CrtcType, DisplaySkew, PreviousRegisters, ReadContext};

/// ACCC §4.3: which bits each register stores. R3 holds both the VSYNC and
/// the HSYNC width, R8 the two skew fields and the interlace mode.
const WRITE_MASK: [u8; 16] = [
    0xff, 0xff, 0xff, 0xff, 0x7f, 0x1f, 0x7f, 0x7f, 0xf3, 0x1f, 0x7f, 0x1f, 0x3f, 0xff, 0x3f, 0xff,
];

/// C0 below which an R7 written with the value of C4 does not start a VSYNC
const BLOCKED_R7_C0: u8 = 2;

#[derive(Debug, Default)]
pub struct Crtc0 {
    /// ACCC 12.2, 10.3.1: the frame's last line, evaluated (C4==R4 && C9==R9) only while C0<2.
    /// Once set it resets C4 and C9 on the next line whatever R4/R9 become; when not set, C4
    /// counts on (up to 127) each time C9==R9, R4 or no R4.
    last_line: bool,
    /// The additional line management armed by default on a last line found at C0<2, disarmed
    /// when C0 reaches 2 (so never on a 2 character line), and the R5 > 0 request seen before
    /// C0=3 (ACCC 13.2.1).
    additional_armed: bool,
    additional_r5: bool,
    /// The running adjustment was asked for by R5>0 (it ends when the count equals R5, read
    /// live) and not by the additional management alone.
    adjust_counts_to_r5: bool,
    /// ACCC §13.7.2: C4 steps at C0=2 after an R0 enlargement out of 1.
    r0_enlarge_steps_c4: bool,
    /// ACCC §13.7.2.2: C9 counts on from the last line up to R5.
    r0_enlarge_c9_to_r5: bool,
    /// With R0=0 (ACCC §13.2.4): how many times C0 has wrapped to 0 since R0 became 0 (0, 1, 2
    /// and more); C9 is frozen from the second one on.
    r0_zero_wraps: u8,
    /// C9 had reached R9 on the first of them: C4 moves on the second.
    r0_zero_c4_pending: bool,
    /// R0=0 at this line end: C9 and C4 are not counted.
    counters_frozen: bool,
    /// ACCC 16.4.1.1: an R7 written with C4's value on C0<2 is a BLOCKED VSYNC: no VSYNC until C4
    /// or R7 changes. (ACCC 16.3's wider "no second VSYNC while the equality is unchanged" made
    /// Shaker CT worse against AMSpiriT, so it is not applied.)
    r7_match: bool,
    /// ACCC §13.2.2: the line reached C0=2, which validates the VSYNC test of the next C0=0
    /// (cancelled there): with R0<2 no VSYNC starts.
    vsync_authorized: bool,
    /// Characters left of the blanking window of an R3l=2 HSYNC, which the Gate Array draws a
    /// character before the CRTC's pulse (see `hsync_blank_window`)
    early_blank_chars: u8,
    /// The running HSYNC began as an R3l=2 one (its character-early blanking window was opened)
    two_wide_pulse: bool,
    /// The HSYNC ended as C0 reached its current value: C0=R2 is not looked at on that character
    /// (Shaker AT E: R2 moved during the pulse to where it ends does not start another one; CRTCs
    /// 1-4 start it, AT E and BCOPY A there).
    hsync_ended_on_char: bool,
    /// C0 is at 0 or 1 after a 255 -> 0 wrap (R0 below C0): no line started, so the last-line
    /// state is not assessed again (Overflow's previews write R0 = 15 at C0 = 183 on the frame's
    /// last line, and AMSpiriT, like the real CPC, keeps that line's state through the wrap)
    c0_wrapped: bool,
    /// A line ended on this character (what tells C0 reaching 0 by a line end from a wrap)
    line_just_ended: bool,
}

impl Crtc0 {
    pub fn new() -> Self {
        Crtc0 {
            vsync_authorized: true,
            ..Default::default()
        }
    }

    /// R8=3: interlace sync and video mode.
    fn ivm(core: &Core) -> bool {
        core.regs[8] & 3 == 3
    }

    /// ACCC 12.2: while C0<2, CRTC 0 decides whether this is the frame's last line; it no
    /// longer looks at R4/R9 afterwards.
    fn assess_last_line(&mut self, core: &Core) {
        if !core.in_vertical_adjust {
            self.last_line = core.vcc == core.regs[4] && self.c9_is_last(core);
            self.additional_armed = self.last_line;
            self.additional_r5 = self.last_line && core.regs[5] > 0;
        }
    }

    /// The last two characters of a row's last line (the OUTI's 4th and 5th microseconds of
    /// Shaker B6 A reach C0=R0-1 and R0 here): they compare R7 with the next row's C4.
    fn ends_row_for_r7(&self, core: &Core) -> bool {
        core.hcc.wrapping_add(1) >= core.regs[0]
            && core.hcc <= core.regs[0]
            && self.c9_is_last(core)
    }

    /// ACCC 13.2.1: C0=2 on the last line decides the additional lines; an R5 programmed before
    /// C0=3 asks for them.
    fn assess_additional_line(&mut self, core: &Core) {
        if self.last_line && !core.in_vertical_adjust {
            if core.hcc == 2 {
                self.additional_armed = false;
            }
            if core.hcc <= 2 {
                self.additional_r5 = core.regs[5] > 0;
            }
        }
    }

    /// The vertical counting of a line whose counters are managed (C0 reached 1 or more).
    fn count_managed_line(&mut self, core: &mut Core) -> bool {
        let mut new_frame = false;
        if self.r0_enlarge_c9_to_r5 {
            self.last_line = false;
            core.vlc += 1;
            if core.vlc.value() == core.regs[5] & 0x1f {
                self.r0_enlarge_c9_to_r5 = false;
                core.vcc.reset();
                core.vlc.reset();
                new_frame = true;
            }
        } else if core.in_vertical_adjust {
            // VCC/VLC are held here; only the adjust counter advances, one raster line at a time,
            // until it has topped up the frame by R5 extra lines.
            let extra = u8::from(core.interlace_extra_line);
            let ended = if self.adjust_counts_to_r5 {
                // ACCC 11.3: there is no C5, C9 counts on to R5, read live: an R5 lowered below
                // the count makes the 5 bit counter overflow and go round up to the new value
                core.vtac = (core.vtac + 1) & 0x1f;
                core.vtac == (core.regs[5].wrapping_add(extra)) & 0x1f
            } else {
                core.vtac += 1;
                core.vtac >= core.regs[5].saturating_add(extra)
            };
            if ended {
                self.last_line = false;
                core.end_adjustment();
                new_frame = true;
            }
        } else {
            // ACCC 12.2 / 10.3.1: the latched last-line state, or else C9==R9 at this very
            // moment decides everything
            let ivm_now = Self::ivm(core);
            let end_of_row = if self.last_line {
                true
            } else {
                let r9 = core.regs[9];
                let pf = u8::from(core.parity_frame);
                match (core.ivm_lines, ivm_now) {
                    (false, false) => core.vlc == r9,
                    // the line where R8 goes to 3: C9 against R9 with the parity
                    (false, true) => core.vlc.value() == (r9 | pf),
                    (true, true) => self.c9_is_last(core),
                    // back to normal: C9.VMA against R9
                    (true, false) => self.c9_vma(core) == r9,
                }
            };
            if self.last_line {
                self.last_line = false;
                core.vlc.reset();
                core.vcc += 1;
                let extra = core.interlaced() && core.parity_r6;
                if !(self.additional_armed || self.additional_r5 || extra) {
                    core.vcc.reset();
                    new_frame = true;
                } else {
                    core.in_vertical_adjust = true;
                    self.adjust_counts_to_r5 = self.additional_r5;
                    core.interlace_extra_line = extra;
                    core.vtac = 0;
                }
            } else if end_of_row {
                core.vlc.reset();
                core.vcc += 1;
            } else {
                core.vlc += 1;
            }
            core.ivm_lines = ivm_now;
            // parity of the C9 lines of the next row
            core.parity_c9 = if ivm_now && core.regs[9] & 1 != 0 {
                (core.vcc.value() & 1 != 0) ^ core.parity_frame
            } else {
                core.parity_frame
            };
        }
        new_frame
    }
}

impl CrtcChip for Crtc0 {
    fn crtc_type(&self) -> CrtcType {
        CrtcType::Crtc0
    }

    fn name(&self) -> &'static str {
        "HD6845S"
    }

    fn write_mask(&self, reg: u8) -> u8 {
        WRITE_MASK.get(usize::from(reg)).copied().unwrap_or(0)
    }

    /// ACCC §21.2.1: only the 5 low bits of the register number count; R12
    /// to R17 can be read (R12, R14, R16 are 6 bits wide), anything else
    /// reads 0.
    fn read_register(&self, context: &ReadContext) -> u8 {
        match context.selected & 0x1f {
            number @ 12..=17 => context.register(number) & pointer_mask(number),
            _ => 0,
        }
    }

    /// ACCC §21.3.2: no status register; "my CPC CRTC 0 randomly returns 255
    /// or 127 on this port".
    fn read_status(&self, context: &ReadContext) -> u8 {
        if context.noise {
            0xff
        } else {
            0x7f
        }
    }

    /// ACCC §14.6: with R3l = 0, CRTC 0 does not produce an HSYNC.
    fn hsync_width(&self, r3: u8) -> Option<u8> {
        Some(r3 & 0x0f).filter(|width| *width != 0)
    }

    /// ACCC §14.1: R3h, 0 meaning 16.
    fn vsync_width(&self, r3: u8) -> u8 {
        match r3 >> 4 {
            0 => 16,
            width => width,
        }
    }

    /// ACCC §19.1: R8 bits 5-4.
    fn display_skew(&self, r8: u8) -> DisplaySkew {
        skew_from_r8(r8)
    }

    fn reset(&mut self, _core: &mut Core) {
        // the counters sit on the last line of the frame
        *self = Crtc0 {
            last_line: true,
            ..Crtc0::new()
        };
    }

    // ---- register writes ----

    /// ACCC §13.2.1: C9 is no longer managed while R0=0, and R4, R5 and R9 updates are ignored
    /// with it (R8 still is).
    fn accepts_write(&self, core: &Core, r: u8) -> bool {
        !(core.regs[0] == 0 && matches!(r, 4 | 5 | 9))
    }

    /// ACCC 16.4.1.1: R7 given the value of C4 on C0=0/1 is a BLOCKED VSYNC: it does not start,
    /// nor can it until C4 or R7 changes. On a row's last line, C0=R0 is already the next row's
    /// C0<2 (ACCC 16.4; Shaker B6 A, "R7 LAST CHANCE 4TH uSec ON C0=0": the real CRTC 0 prints #1E).
    fn before_write(&mut self, core: &mut Core, r: u8, value: u8) {
        if r == 7
            && !core.vsync()
            && core.vcc.value() == value
            && core.vcc.value() != core.regs[7]
            && (core.hcc < BLOCKED_R7_C0 || self.ends_row_for_r7(core))
        {
            self.r7_match = true;
        }
    }

    fn register_written(&mut self, core: &mut Core, r: u8, previous: PreviousRegisters) {
        // ACCC §13.2.4: R0=0 written on the C0=0 that has just updated the counters makes it the
        // first C0=0 with R0=0; the next wrap is the second (C4 moves once if C9 reached R9)
        if r == 0
            && core.regs[0] == 0
            && previous.r0 > 1
            && core.hcc == 0
            && self.r0_zero_wraps == 0
        {
            let hiccup = !core.in_vertical_adjust && self.c9_is_last(core);
            self.r0_zero_c4_pending = hiccup;
            if hiccup && core.vcc == core.regs[4] {
                self.r0_enlarge_c9_to_r5 = true;
            }
            self.r0_zero_wraps = 1;
        }
        // ACCC §13.7.2: R0 enlarged out of 1 when C0=1 and C9=R9 overflows C4
        // (even if C4<>R4); the step itself lands at C0=2.
        if r == 0
            && core.hcc == 1
            && previous.r0 == 1
            && core.regs[0] > 1
            && self.c9_is_last(core)
            && !core.in_vertical_adjust
        {
            self.r0_enlarge_steps_c4 = true;
        }
        // an early window of an R3l=2 HSYNC that turns out wide (R3 raised on the very
        // character R2 names, HBL's main sync) is not drawn early
        if r == 3
            && self.early_blank_chars > 0
            && core.hcc == core.regs[2]
            && !self.hsync_is_two_wide(core)
        {
            self.early_blank_chars = 0;
            self.two_wide_pulse = false;
        }
        if r == 6 {
            core.latch_r6_written_with_c4();
        }
        // CRTC 0 keeps assessing the last line while C0<2 (ACCC 12.2)
        if (r == 4 || r == 9) && core.hcc < 2 {
            self.assess_last_line(core);
        }
        if r == 5 && core.hcc <= 2 {
            self.assess_additional_line(core);
        }
    }

    // ---- HSYNC ----

    fn hsync_counted(&mut self, core: &mut Core) {
        // an R3l=2 written during the pulse (HBL's panel: R3=7 then 2 on C0=R2+1) makes it a
        // short pulse too, not blanked from then on - if it still ends at 2 (written later, C3l
        // has passed 2 and the pulse runs on: Shaker AY C keeps it black)
        if self.hsync_is_two_wide(core) && core.hsc <= 2 {
            self.two_wide_pulse = true;
        }
    }

    fn new_character(&mut self, _core: &mut Core) {
        self.hsync_ended_on_char = false;
    }

    fn hsync_ended(&mut self, _core: &mut Core) {
        self.hsync_ended_on_char = true;
    }

    fn character_starts(&mut self, _core: &mut Core) {
        self.early_blank_chars = self.early_blank_chars.saturating_sub(1);
    }

    /// R3l=2: the Gate Array blanks and takes the mode a character before the pulse (AMSpiriT
    /// probe on HBL's R3=2 pulses), the pulse itself keeps its place for the interrupt counter.
    fn before_hsync_check(&mut self, core: &mut Core) {
        if self.hsync_is_two_wide(core) && core.hcc.wrapping_add(1) == core.regs[2] && !core.hsync()
        {
            self.early_blank_chars = 2;
            self.two_wide_pulse = true;
        }
        if !core.hsync() && self.early_blank_chars == 0 {
            self.two_wide_pulse = false;
        }
    }

    fn hsync_start_blocked(&self) -> bool {
        self.hsync_ended_on_char
    }

    fn hsync_starting(&mut self, core: &mut Core) {
        // R3l=2 written just before the pulse (HBL's panel writes R3=2 on C0=R2-1): too late for
        // the early window, the pulse itself is still not blanked
        if self.hsync_is_two_wide(core) {
            self.two_wide_pulse = true;
        }
    }

    fn hsync_blank_window(&self, core: &Core) -> Option<bool> {
        if self.early_blank_chars > 0 {
            Some(true)
        } else {
            (self.two_wide_pulse && core.hsync()).then_some(false)
        }
    }

    fn hsync_is_two_wide(&self, core: &Core) -> bool {
        self.hsync_width(core.regs[3]) == Some(2)
    }

    // ---- lines and frames ----

    fn line_wraps(&mut self, core: &mut Core) {
        self.line_just_ended = true;
        // ACCC §13.2.1/§13.2.4: with R0=0, C0 never reaches 1 and C9 stops being managed: the
        // first wrap to C0=0 updates the counters as usual (and arms C4 if C9 reached R9), the
        // second only moves C4 once, then everything is frozen.
        self.counters_frozen = core.regs[0] == 0;
        if self.counters_frozen {
            if self.r0_zero_wraps == 0 {
                // First C0=0 with R0=0: C9 is compared with R9 for the first time; on C9=R9 it
                // stays put and C4 moves on the next one (13.2.6), starting the additional
                // management when C4=R4. Nothing moves at all while an adjustment is running
                // (13.2.3).
                let hiccup = !core.in_vertical_adjust && self.c9_is_last(core);
                if !hiccup && !core.in_vertical_adjust {
                    core.vlc += 1;
                }
                self.r0_zero_c4_pending = hiccup;
                if hiccup && core.vcc == core.regs[4] {
                    self.r0_enlarge_c9_to_r5 = true;
                }
            } else if self.r0_zero_wraps == 1 && self.r0_zero_c4_pending {
                core.vcc += 1;
                self.r0_zero_c4_pending = false;
            }
            self.r0_zero_wraps = self.r0_zero_wraps.saturating_add(1);
        } else {
            self.r0_zero_wraps = 0;
            self.r0_zero_c4_pending = false;
        }
    }

    fn count_line(&mut self, core: &mut Core) -> bool {
        !self.counters_frozen && self.count_managed_line(core)
    }

    fn line_started(&mut self, core: &mut Core, new_frame: bool, c4_before: u8) {
        core.parity_from_r6(new_frame, c4_before);
    }

    fn character_counted(&mut self, core: &mut Core) {
        if core.hcc == 2 && self.r0_enlarge_steps_c4 {
            self.r0_enlarge_steps_c4 = false;
            core.vcc += 1;
            if core.vcc == core.regs[6] && core.regs[6] != 0 {
                core.vertical_border = true;
            }
            // ACCC §13.7.2.2: on a last line the additional management stays
            // active whatever R5 is, and C9 goes on counting (no reset) up to R5
            if self.last_line {
                self.r0_enlarge_c9_to_r5 = true;
            }
        }
        if core.hcc == 0 {
            self.c0_wrapped = !self.line_just_ended;
            self.line_just_ended = false;
        }
        if core.hcc < 2 && !self.c0_wrapped {
            self.assess_last_line(core);
        }
        self.assess_additional_line(core);
    }

    fn takes_edge_write_before_character(&self) -> bool {
        false
    }

    fn r0_lowered_on_c0_ends_line(&self) -> bool {
        true
    }

    fn c9_is_last(&self, core: &Core) -> bool {
        if core.ivm_lines {
            let r9 = core.regs[9];
            self.c9_vma(core) == (r9 + ((r9 ^ u8::from(core.parity_c9)) & 1)) & 0x1f
        } else {
            core.vlc == core.regs[9]
        }
    }

    /// ACCC §19.8.1: in IVM, C9 is shifted left with the parity in bit 0; ACCC 11.2.2: there is
    /// no C5, C9 itself counts the lines of an R5 adjustment (and so goes out on RA).
    fn c9_vma(&self, core: &Core) -> u8 {
        if core.in_vertical_adjust && self.adjust_counts_to_r5 && !core.ivm_lines {
            core.vtac & 0x1f
        } else if core.ivm_lines {
            (core.vlc.value() << 1 | u8::from(core.parity_c9)) & 0x1f
        } else {
            core.vlc.value()
        }
    }

    // ---- BORDER ----

    fn border_r6(&mut self, core: &mut Core) -> bool {
        core.latched_r6_border()
    }

    fn anticipates_border_byte(&self) -> bool {
        true
    }

    fn has_r6_zero_conflict(&self) -> bool {
        true
    }

    // ---- VSYNC ----

    /// A blocked VSYNC is released when C4 and R7 differ.
    fn before_vsync_check(&mut self, core: &mut Core) {
        if !core.vsync() && core.vcc != core.regs[7] {
            self.r7_match = false;
        }
    }

    fn vsync_blocked(&self) -> bool {
        self.r7_match
    }

    fn vsync_refused(&self) -> bool {
        !self.vsync_authorized
    }

    fn vsync_counts_from_next_line(&self) -> bool {
        true
    }

    fn after_vsync_check(&mut self, core: &mut Core) {
        if core.hcc == 2 {
            self.vsync_authorized = true;
        } else if core.hcc == 0 {
            self.vsync_authorized = false;
        }
    }

    // ---- tools and snapshots ----

    fn restored(&mut self, core: &mut Core) {
        self.adjust_counts_to_r5 = core.in_vertical_adjust;
        self.last_line = false;
    }

    fn last_line(&self) -> bool {
        self.last_line
    }
}

/// The skew field shared by the chips that have one (CRTC 0, 3, 4).
pub(super) fn skew_from_r8(r8: u8) -> DisplaySkew {
    match (r8 >> 4) & 3 {
        3 => DisplaySkew::Blanked,
        delay => DisplaySkew::Delayed(delay),
    }
}
