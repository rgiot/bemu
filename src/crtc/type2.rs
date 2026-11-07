//! CRTC 2: Motorola MC6845.
//!
//! Its own: the three states of the "Last Line" (ACCC 12.4.1), many treatments inhibited during
//! HSYNC (no Last Line, GHOST VSYNC), VMA' taking R12/R13 at C0=R1 on the frame's last line, the
//! ParityR6 parity and the interlace additional line rules of ACCC 19.5.4/19.6.3.

use super::core::Core;
use super::model::{pointer_mask, CrtcChip, CrtcType, DisplaySkew, PreviousRegisters, ReadContext};
use super::type1::WRITE_MASK;

#[derive(Debug)]
pub struct Crtc2 {
    /// ACCC 12.4.1: "Last Line" (C4 and C9 go to 0 on the next line, decided on C0==0 or by an
    /// R4/R9 update and never cancelled during the line), "Last Line Management" (an R4/R9
    /// update may still set it: not on the frame's first line) and "Previous Last Line"
    /// (C4==R4 and C9==R9 on the last character of the HSYNC: the next C0==0 does not take the
    /// line as a last one again).
    last_line: bool,
    last_line_management: bool,
    previous_last_line: bool,
    /// ACCC 19.6.3: IVM switched on during an odd frame's line 0 made it the interlace line
    /// alone, with no R5 line before it.
    lone_extra_line: bool,
    /// ACCC 19.8.3: C9.IVM, the line counter that builds the video address in IVM (C9 itself
    /// counts as usual): back to 0 when C9 is at R9 or at R9/2, so twice per character row.
    c9_ivm: u8,
}

impl Default for Crtc2 {
    fn default() -> Self {
        Crtc2 {
            last_line: false,
            last_line_management: true,
            previous_last_line: false,
            lone_extra_line: false,
            c9_ivm: 0,
        }
    }
}

impl Crtc2 {
    /// C0==0 (ACCC 12.4.1): the line is a last line when C4==R4 and C9==R9 (the R9 of the
    /// previous line: an update on this C0 comes too late), unless the previous line already was
    /// one or an HSYNC is on C0==0, starting there or still running from the previous line (ACCC
    /// 15.4 note 1, 15.6); R4/R9 updates may still make it one, except on a frame's first line.
    fn line_start(&mut self, core: &Core) {
        self.last_line = !core.in_vertical_adjust
            && core.vcc == core.regs[4]
            && core.vlc == core.regs[9]
            && !self.previous_last_line
            && core.regs[2] != 0
            && !core.hsync();
        self.last_line_management = !(core.vcc.value() == 0 && core.vlc.value() == 0);
    }

    /// The last line of the frame: C4=R4 and C9=R9, or the last line of the adjustment.
    fn is_frame_last_line(&self, core: &Core) -> bool {
        if core.in_vertical_adjust {
            core.vtac.saturating_add(1)
                >= core.regs[5].saturating_add(u8::from(core.interlace_extra_line))
        } else {
            core.vcc == core.regs[4] && self.c9_is_last(core)
        }
    }
}

impl CrtcChip for Crtc2 {
    fn crtc_type(&self) -> CrtcType {
        CrtcType::Crtc2
    }

    fn name(&self) -> &'static str {
        "MC6845"
    }

    /// ACCC §4.3: the same register bits as the UM6845R.
    fn write_mask(&self, reg: u8) -> u8 {
        WRITE_MASK.get(usize::from(reg)).copied().unwrap_or(0)
    }

    /// ACCC §21.2.2 / §28.1.9: R14-R17 can be read (5 significant bits of
    /// the register number), anything else returns 0.
    fn read_register(&self, context: &ReadContext) -> u8 {
        match context.selected & 0x1f {
            number @ 14..=17 => context.register(number) & pointer_mask(number),
            _ => 0,
        }
    }

    /// ACCC §21.3.2: no status register - "my CPC CRTC 2 always returns 255
    /// on this port".
    fn read_status(&self, _context: &ReadContext) -> u8 {
        0xff
    }

    /// ACCC §14.6: on CRTC 2, 3 and 4 "it is impossible not to generate an
    /// HSYNC": R3l = 0 gives 16 characters.
    fn hsync_width(&self, r3: u8) -> Option<u8> {
        Some(match r3 & 0x0f {
            0 => 16,
            width => width,
        })
    }

    /// ACCC §14.1: fixed at 16 lines.
    fn vsync_width(&self, _r3: u8) -> u8 {
        16
    }

    fn display_skew(&self, _r8: u8) -> DisplaySkew {
        DisplaySkew::Delayed(0)
    }

    fn reset(&mut self, _core: &mut Core) {
        // the Last Line states are not reset (as before: they are re-evaluated on the next line)
        self.lone_extra_line = false;
    }

    // ---- register writes ----

    fn register_written(&mut self, core: &mut Core, r: u8, previous: PreviousRegisters) {
        // ACCC 12.4.1: an R4/R9 update outside HSYNC can make the current line the last one
        if matches!(r, 4 | 9)
            && !self.last_line
            && self.last_line_management
            && !core.hsync()
            && !core.in_vertical_adjust
            && !(r == 9 && core.hcc == 0)
        {
            self.last_line = core.vcc == core.regs[4] && core.vlc == core.regs[9];
        }
        // ACCC §19.5.4/§19.6.3: IVM switched on during the first line of an odd frame, whatever
        // C0, makes that line the additional line; a new line 0 follows it
        if r == 8
            && previous.r8 & 3 != 3
            && core.regs[8] & 3 == 3
            && core.vcc == 0
            && core.vlc == 0
            && !core.in_vertical_adjust
            && core.parity_frame
        {
            core.in_vertical_adjust = true;
            core.interlace_extra_line = true;
            self.lone_extra_line = true;
            core.vtac = 0;
        }
        // ACCC §19.6.3: the interlace mode left during the additional line cancels it: C4 (past
        // R4) is not sent back to 0, C9 counts on to R9 and C4 on until R4 again
        if r == 8
            && previous.r8 & 1 != 0
            && core.regs[8] & 1 == 0
            && core.in_vertical_adjust
            && core.interlace_extra_line
            && (self.lone_extra_line || core.vtac >= core.regs[5])
        {
            core.in_vertical_adjust = false;
            core.interlace_extra_line = false;
            self.lone_extra_line = false;
            core.vtac = 0;
        }
        if r == 6 {
            core.latch_r6_written_with_c4();
        }
    }

    // ---- HSYNC ----

    /// ACCC 12.4.1: on the HSYNC's last character, "Previous Last Line" takes C4==R4 && C9==R9,
    /// and a false condition re-authorises the "Last Line Management".
    fn hsync_ended(&mut self, core: &mut Core) {
        self.previous_last_line = core.vcc == core.regs[4] && core.vlc == core.regs[9];
        if !self.previous_last_line {
            self.last_line_management = true;
        }
    }

    // ---- lines and frames ----

    fn count_line(&mut self, core: &mut Core) -> bool {
        if core.in_vertical_adjust {
            if self.lone_extra_line {
                // the interlace line comes after the R5 lines: R5 written on it is too late
                self.lone_extra_line = false;
                core.end_adjustment();
                return true;
            }
            return core.count_adjust_line();
        }
        core.ivm_lines = false;
        // ACCC 19.8.3: C9.IVM steps beside C9 at every line end, IVM or not
        let (c9, r9) = (core.vlc.value(), core.regs[9] & 0x1f);
        self.c9_ivm = if c9 == r9 || c9 == r9 >> 1 {
            0
        } else {
            (self.c9_ivm + 1) & 0x1f
        };
        // compared before the increment: C9 is a 5-bit counter, so with R9=31 the incremented
        // value (0) never equals R9+1 (32)
        let end = core.vlc == core.regs[9];
        core.vlc += 1;
        // a "Last Line" sends C9 (and C4) back to 0 whatever R9 became since
        if !(end || self.last_line) {
            return false;
        }
        core.vlc.reset();
        // ACCC 19.6.3: the interlace line ends the frames whose ParityR6 is odd
        let extra = core.interlaced() && core.parity_r6;
        core.end_row(self.last_line, extra, false)
    }

    fn line_started(&mut self, core: &mut Core, new_frame: bool, c4_before: u8) {
        if new_frame {
            self.c9_ivm = 0;
        }
        self.line_start(core);
        core.parity_from_r6(new_frame, c4_before);
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

    fn ghost_vsync_during_hsync(&self) -> bool {
        true
    }

    // ---- video pointer ----

    /// ACCC §13.4, §20.3.3: the frame starts at VMA', which takes R12/R13 at C0=R1 on the frame's
    /// last line (or last adjustment line). ACCC 19.8.3: in IVM (R8=3) VMA' takes VMA when C9.IVM
    /// reaches R9 (without its parity), "VMA' can be updated without C4 incrementing"; otherwise
    /// only on C9=R9 (Shaker B1 R9E6 H2: R9=6 written on C9=7 with IVM, the next line reads the
    /// next row, as on CPCSyntaxError and AMSpiriT).
    fn r1_reached(&mut self, core: &mut Core) {
        if self.is_frame_last_line(core) {
            core.crtc_vma_prime = core.base_address();
        } else if core.regs[8] & 3 == 3 {
            if self.c9_ivm == (core.regs[9] & 0x1f) >> 1 {
                core.crtc_vma_prime = core.crtc_vma;
            }
        } else if self.c9_is_last(core) {
            core.crtc_vma_prime = core.crtc_vma;
        }
    }

    /// ACCC 19.8.3: in IVM the address takes C9.VMA = C9.IVM*2 with the frame's parity.
    fn c9_vma(&self, core: &Core) -> u8 {
        if core.regs[8] & 3 == 3 {
            ((self.c9_ivm << 1) | u8::from(core.parity_frame)) & 0x1f
        } else {
            core.vlc.value()
        }
    }

    /// No R12/R13 at the frame's first line: VMA' already holds it.
    fn reload_vma(&mut self, core: &mut Core) {
        core.crtc_vma = core.crtc_vma_prime;
    }
}
