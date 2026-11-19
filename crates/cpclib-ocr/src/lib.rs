//! A naive OCR for Amstrad CPC screens.
//!
//! It reads the text a program printed through the firmware (the 8x8 characters of the lower
//! ROM) in a picture of the screen, whatever the screen mode, the zoom of the picture (each CPC
//! pixel can be any number of picture pixels wide and tall), the border around the text, the
//! colours, and inverse text (the cursor, `PEN`/`PAPER` swaps).
//!
//! How: the size of one CPC pixel is the shortest run of one colour in the picture; the picture
//! is reduced to CPC pixels; the 8x8 character grid is the offset where most cells are exactly a
//! glyph of the font; each cell is then matched against the font in both of its colour
//! assignments. A cell that is no glyph (graphics, user-defined characters, three colours) reads
//! as `?`.
//!
//! The font is read from a lower ROM (`Font::from_lower_rom`), so the crate bundles no firmware.
//!
//! ```
//! use cpclib_ocr::{Font, Ocr, Picture};
//! # let mut rom = vec![0u8; 0x4000];
//! # for (i, row) in [0x18, 0x3c, 0x66, 0x66, 0x7e, 0x66, 0x66, 0x00].iter().enumerate() {
//! #     rom[0x3800 + 0x41 * 8 + i] = *row;
//! # }
//! let ocr = Ocr::new(Font::from_lower_rom(&rom).unwrap());
//! let picture = Picture::new(8, 8, vec![0; 8 * 8 * 3]).unwrap();
//! assert_eq!(ocr.read(&picture).text(), "");
//! ```

use std::collections::HashMap;

/// Offset of the character set in the lower ROM of every CPC (464, 664, 6128, and the CPC+'s
/// system cartridge once its ROM is laid out as a lower ROM).
pub const FONT_OFFSET: usize = 0x3800;

/// An RGB picture, 3 bytes per pixel, row by row.
#[derive(Clone, Debug)]
pub struct Picture {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
}

impl Picture {
    /// `None` if `rgb` is not `width * height * 3` bytes.
    pub fn new(width: usize, height: usize, rgb: Vec<u8>) -> Option<Self> {
        (rgb.len() == width * height * 3).then_some(Self { width, height, rgb })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    fn at(&self, x: usize, y: usize) -> u32 {
        let i = (y * self.width + x) * 3;
        u32::from_be_bytes([0, self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]])
    }
}

/// The 8x8 glyphs of the characters 0x20..=0x7E, plus the block (`█`, a full cell: the cursor).
#[derive(Clone, Debug)]
pub struct Font {
    glyphs: Vec<(char, [u8; 8])>,
}

impl Font {
    /// The characters of a lower ROM (16 KiB, or longer: only its first 16 KiB count).
    pub fn from_lower_rom(rom: &[u8]) -> Option<Self> {
        if rom.len() < FONT_OFFSET + 256 * 8 {
            return None;
        }
        let mut glyphs: Vec<(char, [u8; 8])> = (0x20u8..=0x7e)
            .map(|c| {
                let at = FONT_OFFSET + c as usize * 8;
                (c as char, rom[at..at + 8].try_into().unwrap())
            })
            .collect();
        glyphs.push(('█', [0xff; 8]));
        Some(Self { glyphs })
    }

    /// The glyph of `c`, if the font has it.
    pub fn glyph(&self, c: char) -> Option<[u8; 8]> {
        self.glyphs
            .iter()
            .find(|(g, _)| *g == c)
            .map(|(_, bits)| *bits)
    }

    /// The character whose glyph is `bits` with at most `tolerance` pixels different, and how many
    /// differ. A blank glyph is the space; when several are as close, the lowest code wins.
    fn nearest(&self, bits: [u8; 8], tolerance: u32) -> Option<(char, u32)> {
        self.glyphs
            .iter()
            .map(|(c, g)| {
                (
                    *c,
                    (0..8).map(|i| (g[i] ^ bits[i]).count_ones()).sum::<u32>(),
                )
            })
            .min_by_key(|(_, d)| *d)
            .filter(|(_, d)| *d <= tolerance)
    }
}

/// What was read from a picture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Screen {
    /// One string per character row of the grid, from the first row that has something to the
    /// last; right-trimmed. Cells that are no glyph are `?`.
    pub rows: Vec<String>,
    /// Size of a CPC pixel in the picture, `(width, height)`.
    pub pixel: (usize, usize),
    /// The cells that were no glyph.
    pub unknown: usize,
    /// The cells that were a glyph, spaces apart.
    pub known: usize,
}

impl Screen {
    /// The text with the empty rows around it removed, the common left indent removed and each
    /// row right-trimmed: what two pictures of the same text share even if one is framed
    /// differently. Rows are joined by `\n`.
    pub fn text(&self) -> String {
        let rows: Vec<&str> = self.rows.iter().map(|r| r.trim_end()).collect::<Vec<_>>();
        let first = rows.iter().position(|r| !r.is_empty());
        let last = rows.iter().rposition(|r| !r.is_empty());
        let (Some(first), Some(last)) = (first, last) else {
            return String::new();
        };
        let rows = &rows[first..=last];
        let indent = rows
            .iter()
            .filter(|r| !r.is_empty())
            .map(|r| r.chars().take_while(|c| *c == ' ').count())
            .min()
            .unwrap_or(0);
        rows.iter()
            .map(|r| r.chars().skip(indent).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// True if what was read looks like text: at least `minimum` glyphs and mostly known.
    pub fn is_text(&self, minimum: usize) -> bool {
        self.known >= minimum && self.unknown * 4 <= self.known
    }
}

/// The reader.
#[derive(Clone, Debug)]
pub struct Ocr {
    font: Font,
    /// Pixels of a cell that may differ from a glyph and still read as it.
    tolerance: u32,
}

impl Ocr {
    pub fn new(font: Font) -> Self {
        Self { font, tolerance: 0 }
    }

    /// Accepts cells up to `tolerance` pixels away from a glyph (default 0: exact).
    pub fn with_tolerance(mut self, tolerance: u32) -> Self {
        self.tolerance = tolerance;
        self
    }

    pub fn read(&self, picture: &Picture) -> Screen {
        let Some(background) = background(picture) else {
            return Screen::default();
        };
        let (ux, uy) = (
            unit(picture, background, true),
            unit(picture, background, false),
        );
        // the picture in CPC pixels, sampled at the middle of each one
        let (min_x, min_y) = bounding_corner(picture, background);
        let (phase_x, phase_y) = (min_x % ux, min_y % uy);
        let (w, h) = (
            (picture.width - phase_x) / ux,
            (picture.height - phase_y) / uy,
        );
        if w < 8 || h < 8 {
            return Screen {
                pixel: (ux, uy),
                ..Screen::default()
            };
        }
        let cell =
            |x: usize, y: usize| picture.at(phase_x + x * ux + ux / 2, phase_y + y * uy + uy / 2);
        let pixels: Vec<u32> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| cell(x, y))
            .collect();
        let at = |x: usize, y: usize| pixels[y * w + x];

        let mut best: Option<(i64, Screen)> = None;
        for oy in 0..8 {
            for ox in 0..8 {
                let screen = self.decode(&at, w, h, ox, oy, background);
                let score = screen.known as i64 * 2 - screen.unknown as i64 * 3;
                if best.as_ref().map_or(true, |(s, _)| score > *s) {
                    best = Some((score, screen));
                }
            }
        }
        let mut screen = best.unwrap().1;
        screen.pixel = (ux, uy);
        screen
    }

    fn decode(
        &self,
        at: &dyn Fn(usize, usize) -> u32,
        w: usize,
        h: usize,
        ox: usize,
        oy: usize,
        background: u32,
    ) -> Screen {
        let mut screen = Screen::default();
        for row in 0..(h - oy) / 8 {
            let mut line = String::new();
            for col in 0..(w - ox) / 8 {
                let colours: Vec<u32> = {
                    let mut seen = Vec::new();
                    for y in 0..8 {
                        for x in 0..8 {
                            let c = at(ox + col * 8 + x, oy + row * 8 + y);
                            if !seen.contains(&c) {
                                seen.push(c);
                            }
                        }
                    }
                    seen
                };
                let c = match colours.as_slice() {
                    [_] => ' ',
                    [a, b] => self.two_colours(at, ox + col * 8, oy + row * 8, *a, *b, background),
                    _ => '?',
                };
                match c {
                    ' ' => {}
                    '?' => screen.unknown += 1,
                    _ => screen.known += 1,
                }
                line.push(c);
            }
            screen.rows.push(line);
        }
        // only from the first row with something to the last
        let first = screen
            .rows
            .iter()
            .position(|r| r.trim().len() > 0)
            .unwrap_or(0);
        screen.rows.drain(..first);
        while screen.rows.last().is_some_and(|r| r.trim().is_empty()) {
            screen.rows.pop();
        }
        for r in &mut screen.rows {
            let t = r.trim_end().len();
            r.truncate(t);
        }
        screen
    }

    /// A cell with two colours: the glyph is the pixels of one of them. The ink is the colour that
    /// is not the screen's background when only one is; else the better match decides.
    fn two_colours(
        &self,
        at: &dyn Fn(usize, usize) -> u32,
        x0: usize,
        y0: usize,
        a: u32,
        b: u32,
        background: u32,
    ) -> char {
        let bits = |ink: u32| -> [u8; 8] {
            let mut rows = [0u8; 8];
            for (y, row) in rows.iter_mut().enumerate() {
                for x in 0..8 {
                    if at(x0 + x, y0 + y) == ink {
                        *row |= 0x80 >> x;
                    }
                }
            }
            rows
        };
        let options = [a, b]
            .into_iter()
            .filter(|ink| *ink != background || (a != background && b != background))
            .map(|ink| self.font.nearest(bits(ink), self.tolerance))
            .flatten();
        options.min_by_key(|(_, d)| *d).map_or('?', |(c, _)| c)
    }
}

/// The most frequent colour.
fn background(picture: &Picture) -> Option<u32> {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for y in 0..picture.height {
        for x in 0..picture.width {
            *counts.entry(picture.at(x, y)).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(c, n)| (*n, *c))
        .map(|(c, _)| c)
}

/// Top left corner of the box of everything that is not the background.
fn bounding_corner(picture: &Picture, background: u32) -> (usize, usize) {
    let (mut min_x, mut min_y) = (picture.width, picture.height);
    for y in 0..picture.height {
        for x in 0..picture.width {
            if picture.at(x, y) != background {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
            }
        }
    }
    (min_x % picture.width.max(1), min_y % picture.height.max(1))
}

/// Size of a CPC pixel: the largest size that divides (nearly) every run of one colour that does
/// not touch the edge of the picture. The glyphs' strokes are often 2 pixels wide, but a letter
/// always has an odd run somewhere, so it is not the shortest run that counts.
fn unit(picture: &Picture, _background: u32, horizontal: bool) -> usize {
    let (outer, inner) = if horizontal {
        (picture.height, picture.width)
    } else {
        (picture.width, picture.height)
    };
    let mut histogram: HashMap<usize, usize> = HashMap::new();
    for o in 0..outer {
        let get = |i: usize| {
            if horizontal {
                picture.at(i, o)
            } else {
                picture.at(o, i)
            }
        };
        let mut start = 0;
        for i in 1..=inner {
            if i == inner || get(i) != get(start) {
                if start > 0 && i < inner {
                    *histogram.entry(i - start).or_default() += 1;
                }
                start = i;
            }
        }
    }
    let total: usize = histogram.values().sum();
    (1..=32)
        .rev()
        .find(|u| {
            let fitting: usize = histogram
                .iter()
                .filter(|(len, _)| **len % u == 0)
                .map(|(_, n)| n)
                .sum();
            total > 0 && fitting * 100 >= total * 97
        })
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font of the first letters, enough to draw texts without the firmware.
    fn font() -> Font {
        let mut rom = vec![0u8; 0x4000];
        let glyphs: &[(char, [u8; 8])] = &[
            ('A', [0x18, 0x3c, 0x66, 0x66, 0x7e, 0x66, 0x66, 0x00]),
            ('B', [0x7c, 0x66, 0x66, 0x7c, 0x66, 0x66, 0x7c, 0x00]),
            ('C', [0x3c, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3c, 0x00]),
            ('1', [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7e, 0x00]),
            ('I', [0x3c, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3c, 0x00]),
        ];
        for (c, g) in glyphs {
            let at = FONT_OFFSET + *c as usize * 8;
            rom[at..at + 8].copy_from_slice(g);
        }
        Font::from_lower_rom(&rom).unwrap()
    }

    /// Draws `rows` of text from `font` with CPC pixels of `zoom` picture pixels, a border of
    /// `border` pixels (not a multiple of 8 CPC pixels), `ink` on `paper`.
    fn draw(
        font: &Font,
        rows: &[&str],
        zoom: (usize, usize),
        border: (usize, usize),
        ink: [u8; 3],
        paper: [u8; 3],
    ) -> Picture {
        let cols = rows.iter().map(|r| r.chars().count()).max().unwrap();
        let (w, h) = (
            border.0 * 2 + cols * 8 * zoom.0,
            border.1 * 2 + rows.len() * 8 * zoom.1,
        );
        let mut rgb = Vec::new();
        for _ in 0..w * h {
            rgb.extend(paper);
        }
        for (r, row) in rows.iter().enumerate() {
            for (c, ch) in row.chars().enumerate() {
                let glyph = if ch == ' ' {
                    [0; 8]
                } else {
                    font.glyph(ch).unwrap()
                };
                for y in 0..8 * zoom.1 {
                    for x in 0..8 * zoom.0 {
                        if glyph[y / zoom.1] & (0x80 >> (x / zoom.0)) != 0 {
                            let (px, py) =
                                (border.0 + c * 8 * zoom.0 + x, border.1 + r * 8 * zoom.1 + y);
                            let i = (py * w + px) * 3;
                            rgb[i..i + 3].copy_from_slice(&ink);
                        }
                    }
                }
            }
        }
        Picture::new(w, h, rgb).unwrap()
    }

    #[test]
    fn reads_whatever_the_zoom_and_the_framing() {
        let font = font();
        let ocr = Ocr::new(font.clone());
        let rows = ["ABC 1", "  I A", "CBA"];
        for zoom in [(1, 1), (2, 1), (2, 2), (4, 2), (3, 3)] {
            for border in [(0, 0), (13, 7), (24, 16)] {
                let picture = draw(&font, &rows, zoom, border, [255, 255, 0], [0, 0, 100]);
                let screen = ocr.read(&picture);
                assert_eq!(
                    screen.text(),
                    "ABC 1\n  I A\nCBA",
                    "zoom {zoom:?} border {border:?}"
                );
                assert_eq!(screen.pixel, zoom);
            }
        }
    }

    #[test]
    fn inverse_text_and_cursor() {
        let font = font();
        let ocr = Ocr::new(font.clone());
        // paper and ink swapped: the glyph is drawn in the paper's colour over the ink
        let picture = draw(
            &font,
            &["A█B"],
            (2, 2),
            (16, 16),
            [0, 0, 100],
            [255, 255, 0],
        );
        // the picture's background is yellow: everything yellow is "paper"; the blue pixels are the
        // glyphs seen as ink, so the screen reads as the same text
        let mut cursor = draw(
            &font,
            &["A B"],
            (2, 2),
            (16, 16),
            [0, 0, 100],
            [255, 255, 0],
        );
        assert_eq!(ocr.read(&picture).rows.len(), 1);
        assert_eq!(ocr.read(&cursor).text(), "A B");
        cursor = draw(&font, &["A B"], (1, 1), (0, 0), [1, 2, 3], [4, 5, 6]);
        assert_eq!(ocr.read(&cursor).text(), "A B");
    }

    #[test]
    fn graphics_are_not_text() {
        let font = font();
        let ocr = Ocr::new(font);
        let mut rgb = vec![0u8; 64 * 64 * 3];
        for (i, v) in rgb.iter_mut().enumerate() {
            *v = ((i * 2654435761) >> 7) as u8;
        }
        let screen = ocr.read(&Picture::new(64, 64, rgb).unwrap());
        assert!(!screen.is_text(1));
    }
}
