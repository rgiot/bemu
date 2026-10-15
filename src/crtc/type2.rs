//! CRTC 2: Motorola MC6845.

use super::model::{pointer_mask, CrtcModel, CrtcType, DisplaySkew, ReadContext};
use super::type1::WRITE_MASK;

#[derive(Debug)]
pub struct Crtc2;

impl CrtcModel for Crtc2 {
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

    fn ghost_vsync_during_hsync(&self) -> bool {
        true
    }

    fn crtc_type(&self) -> CrtcType {
        CrtcType::Crtc2
    }

    fn anticipates_border_byte(&self) -> bool {
        true
    }

    fn latches_r6_border(&self) -> bool {
        true
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
            _ => 0
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
            width => width
        })
    }

    /// ACCC §14.1: fixed at 16 lines.
    fn vsync_width(&self, _r3: u8) -> u8 {
        16
    }

    fn display_skew(&self, _r8: u8) -> DisplaySkew {
        DisplaySkew::Delayed(0)
    }
}
