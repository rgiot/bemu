//! CRTC 0: Hitachi HD6845S, and the UMC UM6845 (which behaves exactly like
//! it, ACCC §4.2 note 3).

use super::model::{pointer_mask, CrtcModel, CrtcType, DisplaySkew, ReadContext};

/// ACCC §4.3: which bits each register stores. R3 holds both the VSYNC and
/// the HSYNC width, R8 the two skew fields and the interlace mode.
const WRITE_MASK: [u8; 16] = [
    0xff, 0xff, 0xff, 0xff, 0x7f, 0x1f, 0x7f, 0x7f, 0xf3, 0x1f, 0x7f, 0x1f, 0x3f, 0xff, 0x3f, 0xff
];

#[derive(Debug)]
pub struct Crtc0;

impl CrtcModel for Crtc0 {
    fn crtc_type(&self) -> CrtcType {
        CrtcType::Crtc0
    }

    fn anticipates_border_byte(&self) -> bool {
        true
    }

    fn latches_r6_border(&self) -> bool {
        true
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
            _ => 0
        }
    }

    /// ACCC §21.3.2: no status register; "my CPC CRTC 0 randomly returns 255
    /// or 127 on this port".
    fn read_status(&self, context: &ReadContext) -> u8 {
        if context.noise { 0xff } else { 0x7f }
    }

    /// ACCC §14.6: with R3l = 0, CRTC 0 does not produce an HSYNC.
    fn hsync_width(&self, r3: u8) -> Option<u8> {
        Some(r3 & 0x0f).filter(|width| *width != 0)
    }

    /// ACCC §14.1: R3h, 0 meaning 16.
    fn vsync_width(&self, r3: u8) -> u8 {
        match r3 >> 4 {
            0 => 16,
            width => width
        }
    }

    fn has_last_line_state(&self) -> bool {
        true
    }

    /// ACCC §19.1: R8 bits 5-4.
    fn display_skew(&self, r8: u8) -> DisplaySkew {
        skew_from_r8(r8)
    }
}

/// The skew field shared by the chips that have one (CRTC 0, 3, 4).
pub(super) fn skew_from_r8(r8: u8) -> DisplaySkew {
    match (r8 >> 4) & 3 {
        3 => DisplaySkew::Blanked,
        delay => DisplaySkew::Delayed(delay)
    }
}
