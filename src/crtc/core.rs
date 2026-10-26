//! The state every CRTC has: the 6845 counter chain, the registers, the pins, the video
//! pointers and the BORDER/sync bookkeeping that the chips share. What a chip does with it
//! (and its own extra state) lives in `type0.rs`, `type1.rs`, `type2.rs` and `asic.rs`.

use super::model::Registers;
use super::CrtcPin;
use crate::bits::*;
use crate::chip::PinsBuffer;

#[derive(Debug)]
pub struct Core {
    pub(super) pins: PinsBuffer,

    /// Address Register / Selection of the working register
    pub(super) ar: u8,
    /// R0-R17, masked to the bits the chip stores (see [`super::CrtcChip::write_mask`]):
    /// R0 horizontal total, R1 horizontal displayed, R2 HSYNC position,
    /// R3 sync widths, R4 vertical total, R5 vertical total adjust,
    /// R6 vertical displayed, R7 VSYNC position, R8 interlace and skew,
    /// R9 maximum scan line, R10/R11 cursor, R12/R13 display start,
    /// R14/R15 cursor address, R16/R17 light pen.
    pub(super) regs: Registers,

    /// C0, the horizontal character counter: compared with R1 (end of the displayed characters),
    /// R2 (HSYNC) and R0 (end of the line).
    pub hcc: u8,
    /// C4, the vertical character counter (rows): compared with R6 (BORDER), R7 (VSYNC) and R4.
    pub vcc: Bit8Max7,
    /// C9, the scan line in the row, compared with R9.
    pub vlc: Bit8Max5,
    /// C3h, the VSYNC line counter.
    pub vsc: u8,
    /// Characters since the HSYNC started (tools).
    pub hdc: u8,
    /// The adjustment line counter (C5 on the chips that have it), up to R5.
    pub vtac: u8,
    /// C3l, the HSYNC character counter.
    pub hsc: u8,
    /// The frame is in its vertical adjustment (R5 lines and/or the interlace line).
    pub(super) in_vertical_adjust: bool,
    /// An extra interlace line follows the R5 lines of this frame.
    pub(super) interlace_extra_line: bool,

    /// ACCC 16.4.1: a VSYNC begun in the middle of a line does not count that line (CRTC 0)
    pub(super) vsync_skips_first_line_end: bool,
    /// The VSYNC ended while C4 still equals R7: it does not start again
    /// before C4 moves (next row or frame) or R7 changes (AMSpiriT, CPCSE)
    pub(super) vsync_ended_on_r7: bool,
    /// The VSYNC runs (counted by C3h). The VS pin follows it, except for a ghost VSYNC (CRTC 2).
    pub(super) vsync_running: bool,
    /// The write of the current bus cycle was latched (see `Crtc::execute_tick`)
    pub(super) write_taken: bool,
    /// The VSYNCs begun, for the cursor's blinking.
    pub(super) fields: u8,

    /// Number of line ends (C0 back to 0 through R0) since reset; for tests and tools
    pub line_ends: u64,
    /// Motherboard ticks since the last character clock edge (0..15).
    pub(super) phase_ticks: u8,
    /// How many ticks late the running HSYNC rose compared to its C0=R2
    /// character (R2.JIT, ACCC 14.7.1: the monitor sync does not move).
    pub(super) hsync_late_ticks: u8,

    /// BORDER R1 latched on this line (CRTC 0 and 1).
    pub(super) r1_latch: bool,
    /// R1=0 came after C0=0: not considered until the next line.
    pub(super) r1_zero_late: bool,
    /// The horizontal BORDER condition of the last characters, newest first:
    /// the R8 display skew delays it by 1 or 2 characters (ACCC §19.2.3).
    pub(super) border_history: [bool; 3],
    /// Whether the last completed line was in the vertical border by R6
    /// (the UM6845R status register's V bit).
    pub(super) vertical_border: bool,
    /// The BORDER by R6, latched until the next frame on the chips that latch it.
    pub(super) r6_border: bool,
    /// R6 was 0 during this frame's first line: the C4=R6 conflict of ACCC §18.3.2 (CRTC 0, 2).
    pub(super) r6_zero_seen: bool,

    /// Interlace (ACCC §19): the parity of the frame (true = odd), ParityR6 that anticipates the
    /// next frame's (CRTC 0, 2), the parity of the C9 lines in interlace video mode, and whether
    /// the lines currently counted are counted in that mode.
    pub(super) parity_frame: bool,
    pub(super) parity_r6: bool,
    pub(super) parity_c9: bool,
    pub(super) ivm_lines: bool,

    /// CRTC-VMA (ACCC §17.1/§20.2): the live 14-bit video memory address
    /// counter. Genuinely incremented once per character, whatever the
    /// display state - a real 6845 has no idea it's drawing a border, it
    /// just keeps counting (ACCC §17.3: "the VMA pointer continues to
    /// count", even throughout the BORDER). This is what actually goes out
    /// on the MA/RA pins, not a formula recomputed from `vcc`/`hcc` - which
    /// matters the moment a program changes R1 or R12/R13 mid-frame
    /// (split-screen/dynamic-R1 techniques, ACCC §17.3, §20.3).
    pub(super) crtc_vma: u16,
    /// CRTC-VMA' (ACCC §17.1): where the *next* row starts. Latched from
    /// `crtc_vma` when C0 reaches R1 on that row's last scan line (C9=R9) -
    /// so a row whose R1 was never reached that line (R1>R0) never gets its
    /// next-row address updated, and repeats verbatim (ACCC §17.2.2/§17.4).
    /// Transferred into `crtc_vma` at the start of every following line
    /// (C0=0); the very first line of a frame is special-cased per chip.
    pub(super) crtc_vma_prime: u16,

    /// Diagnostics: every register write as (C0, C4, C9, register, value).
    pub(super) write_log: Option<Vec<(u8, u8, u8, u8, u8)>>,
    /// State of the noise source behind the floating values some chips
    /// return (xorshift32: reproducible, unlike an OS random number).
    pub(super) noise: u32,
}

impl Default for Core {
    fn default() -> Self {
        Core {
            pins: Default::default(),
            ar: 0,
            regs: Default::default(),
            hcc: 0,
            vcc: Default::default(),
            vlc: Default::default(),
            vsc: 0,
            hdc: 0,
            vtac: 0,
            hsc: 0,
            in_vertical_adjust: false,
            interlace_extra_line: false,
            vsync_skips_first_line_end: false,
            vsync_ended_on_r7: false,
            vsync_running: false,
            write_taken: false,
            fields: 0,
            line_ends: 0,
            phase_ticks: 0,
            hsync_late_ticks: 0,
            r1_latch: false,
            r1_zero_late: false,
            border_history: [false; 3],
            vertical_border: false,
            r6_border: false,
            r6_zero_seen: false,
            parity_frame: false,
            parity_r6: false,
            parity_c9: false,
            ivm_lines: false,
            crtc_vma: 0,
            crtc_vma_prime: 0,
            write_log: None,
            noise: 0x2545_f491,
        }
    }
}

impl Core {
    pub(super) fn set_pin(&mut self, pin: CrtcPin, value: bool) {
        let pin: u64 = pin.into();
        let buffer = self.pins.current_mut();
        *buffer = (*buffer & !pin) | (pin & u64::from(value).wrapping_neg());
    }

    pub(super) fn get_pin(&self, pin: CrtcPin) -> bool {
        let pin: u64 = pin.into();
        self.pins.current() & pin != 0
    }

    pub(super) fn on(&mut self, pin: CrtcPin) {
        self.set_pin(pin, true);
    }

    pub(super) fn off(&mut self, pin: CrtcPin) {
        self.set_pin(pin, false);
    }

    /// Check if the CRTC as generated a HSYNC
    pub fn hsync(&self) -> bool {
        self.get_pin(CrtcPin::HS)
    }

    /// The VSYNC is running (a ghost VSYNC of CRTC 2 runs without the VS pin).
    pub fn vsync(&self) -> bool {
        self.vsync_running
    }

    /// R12/R13 as a 14-bit address (ACCC §20.1).
    pub(super) fn base_address(&self) -> u16 {
        (((self.regs[12] as u16) << 8).wrapping_add(self.regs[13] as u16)) & 0x3fff
    }

    /// R8 bit 0: one of the two interlace modes (ACCC §19.3).
    pub(super) fn interlaced(&self) -> bool {
        self.regs[8] & 1 != 0
    }

    /// An adjustment line has ended (C5 counts on to R5, plus the interlace line, CRTC 1-4):
    /// true when the adjustment is over and a new frame starts.
    pub(super) fn count_adjust_line(&mut self) -> bool {
        let extra = u8::from(self.interlace_extra_line);
        self.vtac += 1;
        if self.vtac >= self.regs[5].saturating_add(extra) {
            self.end_adjustment();
            true
        } else {
            false
        }
    }

    /// A row has ended (CRTC 1-4): C4 moves on; on the frame's last row the frame ends, or the
    /// adjustment (R5 lines, then the interlace line when `extra`) starts. True on a new frame.
    pub(super) fn end_row(&mut self, last_row: bool, extra: bool, adjust_keeps_c4: bool) -> bool {
        // compared before the increment: C4 is a 7-bit counter, so with R4=127 the
        // incremented value (0) never equals R4+1 (128)
        self.vcc += 1;
        if last_row {
            if self.regs[5] == 0 && !extra {
                self.vcc.reset();
                return true;
            }
            if adjust_keeps_c4 {
                self.vcc = self.regs[4].into();
            }
            self.in_vertical_adjust = true;
            self.interlace_extra_line = extra;
            self.vtac = 0;
        }
        false
    }

    /// ACCC §18.2.2: R6 written with the value of C4 starts the BORDER at once (CRTC 0, 1, 2; not
    /// on the frame's first line, where R6 conflicts with C4=0, §18.3).
    pub(super) fn latch_r6_written_with_c4(&mut self) {
        if self.vcc == self.regs[6] && !(self.vcc == 0 && self.vlc == 0) {
            self.r6_border = true;
            self.set_pin(CrtcPin::DE, false);
        }
    }

    /// ACCC §18.2.2/§18.3.2 (CRTC 0 and 2): C4=R6 latches the BORDER until the next frame; with
    /// C4=R6=C9=0 it comes and goes at each character and only becomes final once C0=R1.
    pub(super) fn latched_r6_border(&mut self) -> bool {
        let first_line = self.vcc == 0 && self.vlc == 0;
        if self.hcc == 0 && first_line {
            self.r6_border = false;
            self.r6_zero_seen = false;
        }
        if self.vcc == self.regs[6] {
            if first_line {
                self.r6_zero_seen = true;
            } else {
                self.r6_border = true;
            }
        }
        if self.r6_zero_seen && first_line && self.hcc >= self.regs[1] {
            self.r6_border = true;
        }
        self.r6_border
    }

    /// ACCC §19.5.2/§19.5.4 (CRTC 0 and 2): ParityFrame takes ParityR6 when a frame starts;
    /// ParityR6 anticipates the next frame's parity each time C4 reaches R6.
    pub(super) fn parity_from_r6(&mut self, new_frame: bool, c4_before: u8) {
        if new_frame {
            self.parity_frame = self.parity_r6;
            self.parity_c9 = self.parity_frame;
        }
        if self.vcc == self.regs[6] && (new_frame || self.vcc.value() != c4_before) {
            self.parity_r6 = !self.parity_frame;
        }
    }

    /// The adjustment is over: the next line is the first one of a new frame.
    pub(super) fn end_adjustment(&mut self) {
        self.in_vertical_adjust = false;
        self.interlace_extra_line = false;
        self.vtac = 0;
        self.vcc.reset();
        self.vlc.reset();
    }

    /// The registers' power-on values, the counters on the last line of a frame.
    pub(super) fn reset(&mut self) {
        self.crtc_vma = 0;
        self.crtc_vma_prime = 0;
        self.regs[..18].copy_from_slice(&[63, 40, 46, 128 + 14, 38, 0, 25, 30, 0, 7, 0, 0, 48, 0, 0, 0, 0, 0]);
        self.vcc = self.regs[4].into();
        self.vlc = self.regs[9].into();
        self.hcc = self.regs[0];
        self.vtac = 0;
        self.in_vertical_adjust = false;
        self.vsc = 0;
        self.hsc = 0;
        self.hdc = 0;
        self.vsync_skips_first_line_end = false;
        self.vsync_ended_on_r7 = false;
        self.vsync_running = false;
        self.r1_latch = false;
        self.r1_zero_late = false;
        self.hsync_late_ticks = 0;
        self.vertical_border = false;
        self.border_history = [false; 3];
        self.r6_border = false;
        self.r6_zero_seen = false;
        self.parity_frame = false;
        self.parity_r6 = false;
        self.parity_c9 = false;
        self.ivm_lines = false;
        self.interlace_extra_line = false;
        self.set_pin(CrtcPin::DE, true);
        self.set_pin(CrtcPin::HS, false);
        self.set_pin(CrtcPin::VS, false);
    }
}
