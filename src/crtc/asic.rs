//! CRTC 3 and 4: the CRTC emulated by Amstrad's ASICs - the 40489 of the
//! CPC+ (3) and the 40226 of the cost-reduced "pre-ASIC" machines (4). The
//! Compendium almost always writes "CRTC's 3 and 4": the two share this
//! implementation and it keeps the [`CrtcType`] only for the few places where it
//! names one and not the other.
//!
//! Their own: register writes landing 1 usec after the I/O (ACCC 13.5), ">=" comparisons where
//! the 6845s test equality (C0 against R0, C9 against R9), the interlace video counting of ACCC
//! 19.8.4, a VSYNC that only starts at a row's start and R6 only tested when a line starts.

use super::core::Core;
use super::model::{pointer_mask, CrtcChip, CrtcType, DisplaySkew, PreviousRegisters, ReadContext};
use super::type0::skew_from_r8;

/// ACCC §4.3: as CRTC 0 (R10/R11 are read-only status registers here but
/// their cursor bits are still stored).
const WRITE_MASK: [u8; 16] = [
    0xff, 0xff, 0xff, 0xff, 0x7f, 0x1f, 0x7f, 0x7f, 0xf3, 0x1f, 0x7f, 0x1f, 0x3f, 0xff, 0x3f, 0xff
];

/// Ticks of a character (1 us).
const CHARACTER_TICKS: u8 = 16;

#[derive(Debug)]
pub struct Asic {
    crtc_type: CrtcType,
    /// Register writes waiting for the next character edge (ticks left, register, value).
    pending: Vec<(u8, u8, u8)>,
}

impl Asic {
    pub fn new(crtc_type: CrtcType) -> Self {
        Asic {
            crtc_type,
            pending: Vec::new(),
        }
    }
}

impl CrtcChip for Asic {
    fn crtc_type(&self) -> CrtcType {
        self.crtc_type
    }

    fn name(&self) -> &'static str {
        match self.crtc_type {
            CrtcType::Crtc3 => "ASIC 40489",
            _ => "ASIC 40226"
        }
    }

    fn write_mask(&self, reg: u8) -> u8 {
        WRITE_MASK.get(usize::from(reg)).copied().unwrap_or(0)
    }

    /// ACCC §21.2.3: only the 3 low bits of the register number count, and
    /// they index a table of 8 registers: 0 R16, 1 R17, 2 R10 (status 1),
    /// 3 R11 (status 2), 4 R12, 5 R13, 6 R14, 7 R15. So register 4 also reads
    /// R12 (and 12, 20...).
    fn read_register(&self, context: &ReadContext) -> u8 {
        match context.selected & 7 {
            0 => context.register(16) & pointer_mask(16),
            1 => context.register(17) & pointer_mask(17),
            // TODO the status registers (ACCC §21.3.4) report live counter
            // events; they read as "nothing happening" until modelled.
            2 | 3 => 0,
            number => {
                let register = 8 + number;
                context.register(register) & pointer_mask(register)
            }
        }
    }

    /// ACCC §21.3.1: "a mirror of the read port".
    fn read_status(&self, context: &ReadContext) -> u8 {
        self.read_register(context)
    }

    /// ACCC §14.6: R3l = 0 gives an HSYNC of 16 characters.
    fn hsync_width(&self, r3: u8) -> Option<u8> {
        Some(match r3 & 0x0f {
            0 => 16,
            width => width
        })
    }

    /// ACCC §14.1: R3h, 0 meaning 16.
    fn vsync_width(&self, r3: u8) -> u8 {
        match r3 >> 4 {
            0 => 16,
            width => width
        }
    }

    fn display_skew(&self, r8: u8) -> DisplaySkew {
        skew_from_r8(r8)
    }

    // ---- register writes ----

    /// ACCC 4.4.4, 13.5: the ASIC samples the CPU bus once per microsecond, on the character clock
    /// edge, before it counts the character. An `OUTI`, whose IORQ is active from the start of its
    /// 5th microsecond, is taken on that edge as on the other CRTCs, while an `OUT (C),r`, active
    /// from the 2nd T-state of its 3rd microsecond, misses it and is taken on the next one: "1 us
    /// delay" (Shaker A4 on a real 6128+: "OUTI ON C0vs=#3b:00").
    fn samples_write_at(&self, phase: u8) -> bool {
        phase == 0
    }

    /// A register taken on the edge reaches the comparators on the next one, R3 excepted: the
    /// HSYNC width is compared with it at once, so an R3 equal to the C3 the HSYNC is about to reach
    /// ends it on this edge (Shaker BI F-H on the real CPC+ and cost-down 6128: whether by
    /// `OUT (C),r` or `OUTI`), while an R3=0 taken on the HSYNC's first edge comes too late to stop
    /// it (BI A/E: 16 characters). Taken at once, R0, R1 and the others act a character early
    /// (Shaker A3, AO H and BP move away from the real CPC).
    fn queue_write(&mut self, r: u8, v: u8) -> bool {
        if r == 3 {
            return false;
        }
        self.pending.push((CHARACTER_TICKS, r, v));
        true
    }

    fn due_writes(&mut self) -> Option<Vec<(u8, u8)>> {
        if self.pending.is_empty() {
            return None;
        }
        for w in self.pending.iter_mut() {
            w.0 -= 1;
        }
        let due = self.pending.iter().filter(|w| w.0 == 0).map(|w| (w.1, w.2)).collect();
        self.pending.retain(|w| w.0 != 0);
        Some(due)
    }

    /// ACCC §19.8.4: IVM switched on takes ParityC9 from the current C9.
    fn register_written(&mut self, core: &mut Core, r: u8, previous: PreviousRegisters) {
        if r == 8 && previous.r8 & 3 != 3 && core.regs[8] & 3 == 3 {
            core.parity_c9 = core.vlc.value() & 1 != 0;
        }
    }

    /// ACCC 14.4.4: "this technique does not work on CRTC's 3 and 4".
    fn supports_r3_jit(&self) -> bool {
        false
    }

    /// ACCC 17.5: R1=0 on C0=0 is "too late".
    fn acknowledges_r1_zero_on_c0_zero(&self) -> bool {
        false
    }

    // ---- lines and frames ----

    fn line_ends_past_r0(&self) -> bool {
        true
    }

    /// ACCC 10.3.4, 19.8.4: an R9 lowered under C9 ends the row too; in IVM the next row starts
    /// on ParityC9, which follows ParityFrame at the frame start and swaps on each C4 when R9 is
    /// odd. ACCC 11.1: C4 stays on R4 through the adjustment.
    fn count_line(&mut self, core: &mut Core) -> bool {
        if core.in_vertical_adjust {
            return core.count_adjust_line();
        }
        let ivm_now = core.regs[8] & 3 == 3;
        core.ivm_lines = ivm_now;
        let r9 = core.regs[9];
        let end = core.vlc.value() >= r9;
        if !end {
            if ivm_now {
                core.vlc = (((core.vlc.value() + 2) | u8::from(core.parity_c9)) & 0x1f).into();
            } else {
                core.vlc += 1;
            }
            return false;
        }
        if core.vcc == core.regs[4] {
            // the frame start (`line_started`, after any R5 or interlace line, whose C9 is 0)
            // sets ParityC9
            core.vlc.reset();
        } else {
            if r9 & 1 != 0 {
                core.parity_c9 = !core.parity_c9;
            }
            core.vlc = u8::from(ivm_now && core.parity_c9).into();
        }
        // ACCC 19.6.4: the additional line on the even frames of the interlace modes
        let last_row = core.vcc == core.regs[4];
        let extra = core.interlaced() && !core.parity_frame;
        core.end_row(last_row, extra, true)
    }

    /// ACCC §19.5.5: ParityFrame switches at each frame start, ParityC9 = ParityFrame then, and in
    /// IVM the frame's first line is C9 = ParityC9.
    fn line_started(&mut self, core: &mut Core, new_frame: bool, _c4_before: u8) {
        if new_frame {
            core.parity_frame = !core.parity_frame;
            core.parity_c9 = core.parity_frame;
            if core.regs[8] & 3 == 3 {
                core.vlc = u8::from(core.parity_c9).into();
            }
        }
    }

    /// ACCC 10.3.4, 19.8.4: the row ends on C9 >= R9, in IVM or not.
    fn c9_is_last(&self, core: &Core) -> bool {
        core.vlc.value() >= core.regs[9]
    }

    // ---- BORDER ----

    /// ACCC 18.2.4: C4=R6 is only tested when C0 goes to 0; it then sets the BORDER until the next
    /// frame, and an R6 written during the line waits for the next line.
    fn border_r6(&mut self, core: &mut Core) -> bool {
        if core.hcc == 0 {
            let first_line = core.vcc == 0
                && !core.in_vertical_adjust
                && core.vlc.value() == u8::from(core.ivm_lines && core.parity_c9);
            if first_line {
                core.r6_border = false;
            }
            if core.vcc == core.regs[6] {
                core.r6_border = true;
            }
        }
        core.r6_border
    }

    // ---- VSYNC ----

    /// ACCC §16.4.4: only on a row's first line (C9=0, or ParityC9 in IVM) at C0=0 (C0=R0/2 for
    /// the MID-VSYNC): an R7 set to C4 later in the row does nothing.
    fn vsync_may_start_here(&self, core: &Core) -> bool {
        core.vlc.value() == u8::from(core.ivm_lines && core.parity_c9)
            && core.hcc == if core.interlaced() && !core.parity_frame { core.regs[0] / 2 } else { 0 }
    }
}
