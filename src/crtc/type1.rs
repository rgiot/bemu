//! CRTC 1: UMC UM6845R.

use super::model::{pointer_mask, CrtcModel, CrtcType, DisplaySkew, ReadContext};

/// ACCC §4.3: like CRTC 2, only R3l (HSYNC width) and R8's interlace bits
/// exist.
pub(super) const WRITE_MASK: [u8; 16] = [
    0xff, 0xff, 0xff, 0x0f, 0x7f, 0x1f, 0x7f, 0x7f, 0x03, 0x1f, 0x7f, 0x1f, 0x3f, 0xff, 0x3f, 0xff
];

#[derive(Debug)]
pub struct Crtc1;

impl CrtcModel for Crtc1 {
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

    fn zero_r3_cancels_running_hsync(&self) -> bool {
        true
    }

    fn supports_interlace(&self) -> bool {
        true
    }

    fn frame_parity_toggles(&self) -> bool {
        true
    }

    /// ACCC §14.1: R3h does not exist here, VSYNC is 16 lines.
    fn vsync_width(&self, _r3: u8) -> u8 {
        16
    }

    fn display_skew(&self, _r8: u8) -> DisplaySkew {
        DisplaySkew::Delayed(0)
    }

    fn reloads_start_address_every_line_of_first_row(&self) -> bool {
        true
    }
}
