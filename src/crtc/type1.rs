//! CRTC 1: UMC UM6845R.
//!
//! No last line state: C4 is compared with R4 whenever C9 reaches R9. Its own: R12/R13 reloaded
//! on every line of the first row, VMA' taken at the end of the C0=R1 character (cancelled by an
//! R1.JIT), the "rupture for dummies" of R5, R6=0 as a live BORDER, the interlace video counting
//! of ACCC 19.8.2 and a frame parity that simply toggles.

use super::core::Core;
use super::model::{pointer_mask, CrtcChip, CrtcType, DisplaySkew, PreviousRegisters, ReadContext};
use super::CrtcPin;

/// ACCC §4.3: like CRTC 2, only R3l (HSYNC width) and R8's interlace bits
/// exist.
pub(super) const WRITE_MASK: [u8; 16] = [
    0xff, 0xff, 0xff, 0x0f, 0x7f, 0x1f, 0x7f, 0x7f, 0x03, 0x1f, 0x7f, 0x1f, 0x3f, 0xff, 0x3f, 0xff
];

#[derive(Debug, Default)]
pub struct Crtc1 {
    /// ACCC 11.6: a rupture for dummies is running: VMA is reloaded from R12/R13 at the start of
    /// every line until the C9=R9 test at C0=R1 passes (it depends on the frame parity)
    rfd_active: bool,
    /// ACCC 17.4.2: the address VMA' will take at the end of this character if C0=R1 still holds
    vma_latch_pending: Option<u16>,
}

impl Crtc1 {
    /// VMA' takes the address of the next row. During an RFD (ACCC 11.6.1) the test is made with
    /// the frame parity: on an even frame it fails and the row repeats, on an odd one it passes,
    /// VMA' is updated and the RFD is over.
    fn commit_next_row_address(&mut self, core: &mut Core, vma: u16) {
        if self.rfd_active {
            if !core.parity_frame {
                return;
            }
            self.rfd_active = false;
        }
        core.crtc_vma_prime = vma;
    }
}

impl CrtcChip for Crtc1 {
    fn crtc_type(&self) -> CrtcType {
        CrtcType::Crtc1
    }

    fn name(&self) -> &'static str {
        "UM6845R"
    }

    fn write_mask(&self, reg: u8) -> u8 {
        WRITE_MASK.get(usize::from(reg)).copied().unwrap_or(0)
    }

    /// ACCC §21.2.2: only R14-R17 can be read (5 significant bits of the
    /// register number); the others return 0 - except register 31 (all five
    /// bits set) which returns a non-zero value ("I got 127 or 255").
    fn read_register(&self, context: &ReadContext) -> u8 {
        match context.selected & 0x1f {
            number @ 14..=17 => context.register(number) & pointer_mask(number),
            0x1f => if context.noise { 0xff } else { 0x7f },
            _ => 0
        }
    }

    /// ACCC §21.3.1/§21.3.3: the only chip with a status register:
    /// `x L V x x x x x`. L (bit 6) is 1 for a light pen reading and 0 when
    /// R16/R17 can be read; V (bit 5) is 1 while the BORDER by R6 holds.
    /// The other bits are unused and read 0.
    fn read_status(&self, context: &ReadContext) -> u8 {
        if context.vertical_border { 1 << 5 } else { 0 }
    }

    /// ACCC §14.6: with R3l = 0, CRTC 1 does not produce an HSYNC.
    fn hsync_width(&self, r3: u8) -> Option<u8> {
        Some(r3 & 0x0f).filter(|width| *width != 0)
    }

    /// ACCC §14.1: R3h does not exist here, VSYNC is 16 lines.
    fn vsync_width(&self, _r3: u8) -> u8 {
        16
    }

    fn display_skew(&self, _r8: u8) -> DisplaySkew {
        DisplaySkew::Delayed(0)
    }

    fn reset(&mut self, _core: &mut Core) {
        *self = Crtc1::default();
    }

    // ---- register writes ----

    fn register_written(&mut self, core: &mut Core, r: u8, previous: PreviousRegisters) {
        // ACCC §19.5.3: the parities follow R8 as soon as it is written
        if r == 8 {
            let was_ivm = previous.r8 & 3 == 3;
            let is_ivm = core.regs[8] & 3 == 3;
            if was_ivm != is_ivm {
                let c4_odd_r9_even = core.vcc.value() & 1 != 0 && core.regs[9] & 1 == 0;
                let c9 = self.c9_vma(core);
                // 3rd microsecond: ParityC9 = C9.0 xor (C4.0 and not R9.0)
                let mut parity_c9 = (c9 & 1 != 0) ^ c4_odd_r9_even;
                if is_ivm {
                    // 4th microsecond: IVM becomes active
                    if !core.parity_frame {
                        parity_c9 = c4_odd_r9_even;
                    }
                    core.parity_frame = core.parity_frame && (parity_c9 ^ c4_odd_r9_even);
                    core.vlc = ((c9 & !1) | u8::from(parity_c9)).into();
                    core.parity_c9 = parity_c9;
                } else {
                    // IVM goes idle: ParityFrame = ParityC9, C9 keeps its bit 0 as ParityC9
                    core.parity_frame = parity_c9;
                    core.vlc = ((c9 & !1) | u8::from(parity_c9)).into();
                }
                core.ivm_lines = is_ivm;
            }
        }
        // ACCC §11.6: R5 raised from 0 on C0=R0 starts a rupture for dummies
        if r == 5 && previous.r5 & 0x1f == 0 && core.regs[5] & 0x1f != 0 && core.hcc == core.regs[0] {
            self.rfd_active = true;
        }
        if r == 6 {
            core.latch_r6_written_with_c4();
            // ACCC §18.3.3: R6=0 written on the first line of a frame (C4=R6=0) makes the BORDER
            // final for the rest of the frame
            if core.regs[6] == 0 && core.vcc == 0 && core.vlc == 0 {
                core.r6_border = true;
                core.set_pin(CrtcPin::DE, false);
            }
        }
    }

    // ---- HSYNC ----

    fn zero_r3_cancels_running_hsync(&self) -> bool {
        true
    }

    /// ACCC 17.4.2: the character where C0=R1 has run its course: VMA' is updated only if R1
    /// still names it (no R1.JIT during it).
    fn new_character(&mut self, core: &mut Core) {
        if let Some(vma) = self.vma_latch_pending.take() {
            if core.hcc == core.regs[1] {
                self.commit_next_row_address(core, vma);
            }
        }
    }

    // ---- lines and frames ----

    fn count_line(&mut self, core: &mut Core) -> bool {
        if core.in_vertical_adjust {
            return core.count_adjust_line();
        }
        // interlace video mode (ACCC 19.8.2): C9 is incremented by 1 more when R9 is even, the
        // row ends when C9 and R9 match without bit 0, and C9 then restarts from ParityC9
        let ivm_now = core.regs[8] & 3 == 3;
        core.ivm_lines = ivm_now;
        let row_end = if ivm_now {
            let r9 = core.regs[9];
            let mut c9 = core.vlc.value();
            if r9 & 1 == 0 {
                c9 = (c9 + 1) & 0x1f;
            }
            let end = c9 & 0x1e == r9 & 0x1e;
            if end {
                if r9 & 1 == 0 {
                    core.parity_c9 = !core.parity_c9;
                }
                c9 = u8::from(core.parity_c9);
            } else {
                c9 = (c9 + 1 + (r9 & 1)) & 0x1f;
            }
            core.vlc = c9.into();
            end
        } else {
            // compared before the increment: C9 is a 5-bit counter, so with R9=31 the
            // incremented value (0) never equals R9+1 (32)
            let end = core.vlc == core.regs[9];
            core.vlc += 1;
            end
        };
        if !row_end {
            return false;
        }
        if !ivm_now {
            core.vlc.reset();
        }
        // ACCC 19.6.2: an additional line follows the R5 lines on the even frames of the
        // interlace modes
        let last_row = core.vcc == core.regs[4];
        let extra = core.interlaced() && !core.parity_frame;
        core.end_row(last_row, extra, false)
    }

    /// ACCC 19.5.3: ParityFrame simply switches at each frame start, whatever R8.
    fn line_started(&mut self, core: &mut Core, new_frame: bool, _c4_before: u8) {
        if new_frame {
            core.parity_frame = !core.parity_frame;
        }
    }

    /// ACCC 19.8.2: in IVM the test excludes bit 0, and is made on C9 after its first-stage
    /// increment (C9+1 when R9 is even), as the row end in `count_line`: with R9=6 a row counted
    /// 1, 3, 5 ends on C9=5. Comparing the bare C9 missed that row's last line, so VMA' was not
    /// taken and the next row repeated it (Shaker B1 R9E6 *2 screens: CPCSyntaxError and AMSpiriT
    /// advance the address by a row there).
    fn c9_is_last(&self, core: &Core) -> bool {
        if core.ivm_lines {
            let r9 = core.regs[9];
            let c9 = (core.vlc.value() + u8::from(r9 & 1 == 0)) & 0x1f;
            c9 & 0x1e == r9 & 0x1e
        } else {
            core.vlc == core.regs[9]
        }
    }

    // ---- BORDER ----

    /// ACCC 18.2.3: C4=R6 latches the BORDER until the next frame; R6=0 is a live BORDER, not a
    /// latched one: C4=R6=0 at the frame start does not latch it, so the display comes back as
    /// soon as R6 leaves 0 (Shaker A3, "border state not irreversible", AMSpiriT Lite and CPCSE).
    fn border_r6(&mut self, core: &mut Core) -> bool {
        if core.hcc == 0 && core.vcc == 0 && core.vlc == 0 {
            core.r6_border = false;
        }
        if core.vcc == core.regs[6] && core.regs[6] != 0 {
            core.r6_border = true;
        }
        core.r6_border || core.regs[6] == 0
    }

    // ---- video pointer ----

    /// ACCC 17.4.2: VMA' takes its value at the end of the character where C0=R1 held, an R1
    /// written during that character (R1.JIT) cancels it.
    fn r1_reached(&mut self, core: &mut Core) {
        if self.c9_is_last(core) {
            self.vma_latch_pending = Some(core.crtc_vma);
        }
    }

    /// ACCC §17.4.2: while C4=0, VMA *itself* (not VMA') is reloaded from R12/R13 directly, line
    /// by line - the offset can be changed on each of that row's scan lines, not just once (and
    /// during a rupture for dummies).
    fn reload_vma(&mut self, core: &mut Core) {
        if core.vcc == 0 || self.rfd_active {
            core.crtc_vma = core.base_address();
            return;
        }
        core.crtc_vma = core.crtc_vma_prime;
    }

    /// ACCC 17.5/20.3.2: with R1=0 the condition C0=R1 is met at the first character of the
    /// line: on its last scan line VMA' takes VMA as it is at the beginning of this line.
    fn after_vma_reload(&mut self, core: &mut Core) {
        if core.hcc == 0 && core.regs[1] == 0 && self.c9_is_last(core) {
            self.vma_latch_pending = Some(core.crtc_vma);
        }
    }
}
