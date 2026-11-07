//! A timing overlay: where on the picture the CPU wrote to the CRTC and the Gate Array, to see
//! where a raster effect's writes land and where one breaks.
//!
//! Each write is marked at the beam's place when the instruction ended (the `OUT` writes in its
//! last cycles): a short bar of its colour, from the beam to the right, on that line.
//! - CRTC: register selected (`&BCxx`) grey, register written (`&BDxx`) white; R12/R13 (the
//!   screen address) orange, R0-R9 (the frame's geometry) cyan;
//! - Gate Array: pen selected dark yellow, ink written yellow, RMR (mode, ROMs) magenta, RAM
//!   banking (MMR) green.

use image::RgbImage;

/// What a write was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    CrtcSelect,
    /// R0-R9: the frame's timing and geometry.
    CrtcGeometry,
    /// R12-R13: the screen address.
    CrtcAddress,
    /// The other registers.
    CrtcOther,
    PenSelect,
    Ink,
    Rmr,
    Mmr,
}

impl MarkKind {
    pub fn colour(self) -> [u8; 3] {
        match self {
            Self::CrtcSelect => [128, 128, 128],
            Self::CrtcGeometry => [0, 255, 255],
            Self::CrtcAddress => [255, 140, 0],
            Self::CrtcOther => [255, 255, 255],
            Self::PenSelect => [160, 160, 0],
            Self::Ink => [255, 255, 0],
            Self::Rmr => [255, 0, 255],
            Self::Mmr => [0, 255, 0],
        }
    }
}

/// A write, in picture coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub x: isize,
    pub y: isize,
    pub kind: MarkKind,
}

/// Length of a mark, in pixels (a CRTC character is 16).
const MARK_WIDTH: isize = 8;

/// The marks of the frame being drawn.
#[derive(Default, Clone, Debug)]
pub struct Overlay {
    marks: Vec<Mark>,
    /// The CRTC register selected, for the writes that follow.
    crtc_register: u8,
}

impl Overlay {
    /// Note the I/O write of `value` at `port`, the beam at `beam` (the monitor's coordinates) and
    /// the picture starting at `origin` in them.
    pub fn note(&mut self, port: u16, value: u8, beam: (usize, usize), origin: (isize, usize)) {
        let kind = if port & 0x4000 == 0 {
            // CRTC: A14 low, A9-A8 the function
            match (port >> 8) & 3 {
                0 => {
                    self.crtc_register = value & 0x1f;
                    MarkKind::CrtcSelect
                }
                1 => match self.crtc_register {
                    0..=9 => MarkKind::CrtcGeometry,
                    12 | 13 => MarkKind::CrtcAddress,
                    _ => MarkKind::CrtcOther,
                },
                _ => return,
            }
        } else if port & 0x8000 == 0 {
            // Gate Array: A15 low, A14 high; the function in the value's top bits
            match value >> 6 {
                0 => MarkKind::PenSelect,
                1 => MarkKind::Ink,
                2 => MarkKind::Rmr,
                _ => MarkKind::Mmr,
            }
        } else {
            return;
        };
        self.marks.push(Mark {
            x: beam.0 as isize - origin.0,
            y: beam.1 as isize - origin.1 as isize,
            kind,
        });
    }

    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// Draw the marks on `frame` (the monitor's picture), then start a new frame.
    pub fn draw_and_clear(&mut self, frame: &mut RgbImage) {
        let (width, height) = (frame.width() as isize, frame.height() as isize);
        for mark in &self.marks {
            if !(0..height).contains(&mark.y) {
                continue;
            }
            for x in mark.x.max(0)..(mark.x + MARK_WIDTH).min(width) {
                frame.put_pixel(x as u32, mark.y as u32, image::Rgb(mark.kind.colour()));
            }
        }
        self.marks.clear();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn writes_are_told_apart_and_drawn_where_the_beam_was() {
        let mut overlay = Overlay::default();
        let origin = (100, 30);
        overlay.note(0xbc00, 12, (200, 40), origin);
        overlay.note(0xbd00, 0x30, (216, 40), origin);
        overlay.note(0x7f00, 0x10, (300, 50), origin);
        overlay.note(0x7f00, 0x54, (308, 50), origin);
        overlay.note(0x7f00, 0x8d, (316, 50), origin);
        overlay.note(0xf782, 0x82, (316, 50), origin); // the PPI: not marked
        let kinds: Vec<_> = overlay.marks().iter().map(|mark| mark.kind).collect();
        assert_eq!(
            kinds,
            vec![
                MarkKind::CrtcSelect,
                MarkKind::CrtcAddress,
                MarkKind::PenSelect,
                MarkKind::Ink,
                MarkKind::Rmr
            ]
        );
        assert_eq!((overlay.marks()[1].x, overlay.marks()[1].y), (116, 10));

        let mut frame = RgbImage::new(768, 272);
        overlay.draw_and_clear(&mut frame);
        assert_eq!(frame.get_pixel(116, 10).0, MarkKind::CrtcAddress.colour());
        assert_eq!(
            frame.get_pixel(116 + MARK_WIDTH as u32, 10).0,
            [0, 0, 0],
            "a short bar"
        );
        assert!(overlay.marks().is_empty());
    }
}
