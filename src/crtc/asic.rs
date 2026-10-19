//! CRTC 3 and 4: the CRTC emulated by Amstrad's ASICs - the 40489 of the
//! CPC+ (3) and the 40226 of the cost-reduced "pre-ASIC" machines (4). The
//! Compendium almost always writes "CRTC's 3 and 4": the two share this
//! model and it keeps the [`CrtcType`] only for the few places where it
//! names one and not the other.

use super::model::{pointer_mask, CrtcModel, CrtcType, DisplaySkew, ReadContext};
use super::type0::skew_from_r8;

/// ACCC §4.3: as CRTC 0 (R10/R11 are read-only status registers here but
/// their cursor bits are still stored).
const WRITE_MASK: [u8; 16] = [
    0xff, 0xff, 0xff, 0xff, 0x7f, 0x1f, 0x7f, 0x7f, 0xf3, 0x1f, 0x7f, 0x1f, 0x3f, 0xff, 0x3f, 0xff
];

#[derive(Debug)]
pub struct Asic {
    crtc_type: CrtcType
}

impl Asic {
    pub const CRTC3: Asic = Asic {
        crtc_type: CrtcType::Crtc3
    };
    pub const CRTC4: Asic = Asic {
        crtc_type: CrtcType::Crtc4
    };
}

impl CrtcModel for Asic {
    fn line_ends_past_r0(&self) -> bool {
        true
    }

    /// Interlace sync (R8 bit 0): the period alternates by half a line, as on CRTC 1
    /// (AMSpiriT Lite and CPCSE).
    fn supports_interlace(&self) -> bool {
        true
    }

    fn frame_parity_toggles(&self) -> bool {
        true
    }

    fn has_interlace_video(&self) -> bool {
        false
    }

    fn c9_counts_like_asic(&self) -> bool {
        true
    }

    fn r6_checked_at_line_start(&self) -> bool {
        true
    }

    fn adjust_keeps_c4(&self) -> bool {
        true
    }

    fn acknowledges_r1_zero_on_c0_zero(&self) -> bool {
        false
    }

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

    /// ACCC 14.4.4: "this technique does not work on CRTC's 3 and 4".
    fn supports_r3_jit(&self) -> bool {
        false
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
}
