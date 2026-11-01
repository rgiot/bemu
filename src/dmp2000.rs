//! The Amstrad DMP2000: a 9-needle dot-matrix printer on the CPC's printer port.
//!
//! Centronics as the CPC wires it: the latch (&EFxx) puts D0-D6 and, inverted, /STROBE (bit 7)
//! on the cable; the printer takes the byte on the falling edge of /STROBE and holds BUSY (the
//! PPI's port B bit 6) high while it works on it.
//!
//! What it receives is parsed as the DMP2000's Epson-compatible codes (its manual, Appendix 1)
//! and "printed" on a sheet of fan-fold paper: every dot a needle strikes is drawn on a
//! 240 dpi image of the sheet, with the tractor holes on its sides. The text is kept as well,
//! and the bytes as they came (`.prn`).
//!
//! The paper: 9.5 inch wide (a 0.5 inch strip with its holes on each side), 11 inch long by
//! default. The head's 9 needles are 1/72 inch apart (the vertical unit is 1/216 inch), and
//! horizontally a dot is 1/120 inch (1/240 for the graphics' densest modes).
//!
//! Known differences with the real printer: the characters are the CPC's own 8x8 font (the
//! DMP2000's ROM is not available), proportional spacing and NLQ are approximated, the
//! international sets only change the setting, the print buffer (DEL, CAN) is not kept, and
//! the timings (BUSY) are short, not the 105 characters per second of the head.

use std::any::Any;

use crate::expansion_port::{PrinterDevice, PrinterPin, PrinterPort};

/// Pixels per inch of the images, both ways.
pub const DPI: i32 = 240;
/// Paper width: tractor strip, 8.5 inch, tractor strip.
const PAPER_WIDTH_PX: i32 = DPI * 19 / 2;
/// The strips with the holes are 0.5 inch wide.
const STRIP_PX: i32 = DPI / 2;
/// Where the head's leftmost position is, from the paper's left edge: 0.75 inch.
const LEFT_PX: i32 = DPI * 3 / 4;
/// Widest line: 8 inch.
const LINE_PX: i32 = DPI * 8;
/// Vertical unit: 1/216 inch.
const FEED_UNITS_PER_INCH: i32 = 216;
/// A needle every 1/72 inch.
const PIN_PITCH: i32 = 3;
/// 1/6 inch.
const DEFAULT_FEED: i32 = 36;
/// The first line of a page is one line under the perforation.
const TOP_OF_FORM: i32 = DEFAULT_FEED;
const DEFAULT_PAGE_LENGTH: i32 = 11 * FEED_UNITS_PER_INCH;

/// Cycles the printer is BUSY for, in microseconds. Short ones: the head's speed is not
/// emulated.
const BUSY_BYTE_US: u32 = 40;

const ESC: u8 = 27;

/// Pixels of a paper position in 1/216 inch.
fn feed_px(units: i32) -> f32 {
    units as f32 * DPI as f32 / FEED_UNITS_PER_INCH as f32
}

/// A sheet: the ink the needles left, 0 (none) to 255 (a lot).
#[derive(Clone)]
pub struct Page {
    ink: Vec<u8>,
    height: i32,
    inked: bool
}

impl Page {
    fn new(height_units: i32) -> Self {
        let height = feed_px(height_units).round() as i32;
        Self { ink: vec![0; (PAPER_WIDTH_PX * height) as usize], height, inked: false }
    }

    /// A needle's dot.
    fn dot(&mut self, cx: f32, cy: f32, radius: f32, strength: f32) {
        let (x0, x1) = ((cx - radius - 1.0).floor() as i32, (cx + radius + 1.0).ceil() as i32);
        let (y0, y1) = ((cy - radius - 1.0).floor() as i32, (cy + radius + 1.0).ceil() as i32);
        for y in y0.max(0)..=y1.min(self.height - 1) {
            for x in x0.max(0)..=x1.min(PAPER_WIDTH_PX - 1) {
                let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    let pixel = &mut self.ink[(y * PAPER_WIDTH_PX + x) as usize];
                    *pixel = (f32::from(*pixel) + coverage * strength * 255.0).min(255.0) as u8;
                    self.inked = true;
                }
            }
        }
    }

    pub fn has_ink(&self) -> bool {
        self.inked
    }

    pub fn width(&self) -> u32 {
        PAPER_WIDTH_PX as u32
    }

    pub fn height(&self) -> u32 {
        self.height as u32
    }

    /// The sheet as it looks: paper, tractor strips and holes, perforations, ink. RGB.
    pub fn to_rgb(&self) -> Vec<u8> {
        const PAPER: [f32; 3] = [243.0, 241.0, 233.0];
        const STRIP: [f32; 3] = [234.0, 232.0, 224.0];
        const INK: [f32; 3] = [22.0, 24.0, 44.0];
        const TABLE: [f32; 3] = [58.0, 60.0, 66.0];
        const PERFORATION: [f32; 3] = [196.0, 194.0, 186.0];

        let hole_radius = DPI as f32 * 0.08;
        let mut rgb = Vec::with_capacity((PAPER_WIDTH_PX * self.height * 3) as usize);
        for y in 0..self.height {
            for x in 0..PAPER_WIDTH_PX {
                let in_strip = !(STRIP_PX..PAPER_WIDTH_PX - STRIP_PX).contains(&x);
                let mut colour = if in_strip { STRIP } else { PAPER };

                // the tear-off perforations: dashes along the strips and across the page's ends
                let dash = (y / 4) % 2 == 0;
                let across = (x / 4) % 2 == 0;
                if ((x == STRIP_PX || x == PAPER_WIDTH_PX - STRIP_PX) && dash)
                    || ((y == 0 || y == self.height - 1) && across)
                {
                    colour = PERFORATION;
                }

                // the holes, every half inch
                for hole_x in [STRIP_PX / 2, PAPER_WIDTH_PX - STRIP_PX / 2] {
                    let dx = (x - hole_x) as f32;
                    let dy = (y.rem_euclid(DPI / 2) - DPI / 4) as f32;
                    let distance = (dx * dx + dy * dy).sqrt();
                    let hole = (hole_radius + 0.5 - distance).clamp(0.0, 1.0);
                    if hole > 0.0 {
                        for (value, table) in colour.iter_mut().zip(TABLE) {
                            *value = *value * (1.0 - hole) + table * hole;
                        }
                    }
                    else if distance < hole_radius + 2.0 {
                        // the hole's rim
                        for value in &mut colour {
                            *value *= 0.93;
                        }
                    }
                }

                let ink = f32::from(self.ink[(y * PAPER_WIDTH_PX + x) as usize]) / 255.0 * 0.93;
                for (value, inked) in colour.iter().zip(INK) {
                    rgb.push((value * (1.0 - ink) + inked * ink).round() as u8);
                }
            }
        }
        rgb
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_png(&self, path: &std::path::Path) -> std::io::Result<()> {
        let image = image::RgbImage::from_raw(self.width(), self.height(), self.to_rgb())
            .expect("an image as big as its pixels");
        image.save(path).map_err(|e| std::io::Error::other(e.to_string()))
    }
}

/// What the parser waits for.
#[derive(Clone, Debug)]
enum Parse {
    Idle,
    /// After ESC.
    Esc,
    /// ESC `command`, and the `want` parameters it takes.
    Params { command: u8, want: usize, got: Vec<u8> },
    /// ESC D, B or b: a list of numbers ended by 0 (the channel first for b).
    List { command: u8, channel: Option<u8>, wants_channel: bool, items: Vec<u8> },
    /// Bit image data, `remaining` bytes of it.
    Graphics { mode: u8, nine_pin: bool, remaining: usize, column: Vec<u8>, dots: i32 },
    /// ESC & 0 first last, then 12 bytes (attribute, 11 columns) a character.
    Download { header: Vec<u8>, data: Vec<u8> }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EighthBit {
    Accept,
    Set,
    Unset
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    Super,
    Sub
}

/// A user defined character: 11 columns of needles, the first of them.
#[derive(Clone, Debug)]
struct Download {
    start: u8,
    columns: [u8; 11]
}

/// A glyph, seen by the needles: the pins (bit n: the needle n) struck in each column.
struct Glyph {
    columns: Vec<u16>,
    /// Columns before the first and after the last the proportional spacing drops.
    first: usize,
    last: usize
}

pub struct Dmp2000 {
    /// The 256 8x8 glyphs, a row a byte (bit 7 the leftmost column).
    font: Vec<[u8; 8]>,
    raw: Vec<u8>,
    /// /STROBE was low at the last latch.
    strobe_active: bool,
    busy_us: u32,
    parse: Parse,

    elite: bool,
    condensed: bool,
    double_width: bool,
    bold: bool,
    double_strike: bool,
    italic: bool,
    underline: bool,
    nlq: bool,
    proportional: bool,
    script: Option<Script>,
    control_code_printing: bool,
    eighth_bit: EighthBit,
    extended_codes: bool,
    international_set: u8,
    download_selected: bool,
    downloads: Vec<Option<Download>>,
    /// The density the ESC K, L, Y, Z set (ESC ? changes it).
    density_modes: [u8; 4],

    /// The head, in 1/240 inch from its leftmost position.
    x: i32,
    /// The head, in 1/216 inch from the top of the sheet.
    y: i32,
    page: usize,
    left: i32,
    right: i32,
    feed: i32,
    page_length: i32,
    skip_perforation: i32,
    horizontal_tabs: Vec<i32>,
    vertical_tabs: Vec<Vec<i32>>,
    channel: usize,

    pages: Vec<Page>,
    /// The line being printed: (x, character).
    line: Vec<(i32, char)>,
    text: String
}

impl Dmp2000 {
    /// The characters come from the CPC's lower ROM (its 8x8 font at &3800).
    pub fn with_os_rom(os_rom: &[u8]) -> Self {
        let font = (0..256)
            .map(|code| {
                let mut glyph = [0u8; 8];
                if let Some(rows) = os_rom.get(0x3800 + code * 8..0x3800 + code * 8 + 8) {
                    glyph.copy_from_slice(rows);
                }
                glyph
            })
            .collect();
        let mut printer = Self {
            font,
            raw: Vec::new(),
            strobe_active: false,
            busy_us: 0,
            parse: Parse::Idle,
            elite: false,
            condensed: false,
            double_width: false,
            bold: false,
            double_strike: false,
            italic: false,
            underline: false,
            nlq: false,
            proportional: false,
            script: None,
            control_code_printing: false,
            eighth_bit: EighthBit::Accept,
            extended_codes: false,
            international_set: 0,
            download_selected: false,
            downloads: vec![None; 256],
            density_modes: [0, 1, 2, 3],
            x: 0,
            y: TOP_OF_FORM,
            page: 0,
            left: 0,
            right: LINE_PX,
            feed: DEFAULT_FEED,
            page_length: DEFAULT_PAGE_LENGTH,
            skip_perforation: 0,
            horizontal_tabs: Vec::new(),
            vertical_tabs: vec![Vec::new(); 8],
            channel: 0,
            pages: Vec::new(),
            line: Vec::new(),
            text: String::new()
        };
        printer.reset_settings();
        printer
    }

    /// Switching it on again (ESC @): the paper stays where it is.
    fn reset_settings(&mut self) {
        self.parse = Parse::Idle;
        self.elite = false;
        self.condensed = false;
        self.double_width = false;
        self.bold = false;
        self.double_strike = false;
        self.italic = false;
        self.underline = false;
        self.nlq = false;
        self.proportional = false;
        self.script = None;
        self.control_code_printing = false;
        self.eighth_bit = EighthBit::Accept;
        self.extended_codes = false;
        self.international_set = 0;
        self.download_selected = false;
        self.downloads = vec![None; 256];
        self.density_modes = [0, 1, 2, 3];
        self.left = 0;
        self.right = LINE_PX;
        self.feed = DEFAULT_FEED;
        self.page_length = DEFAULT_PAGE_LENGTH;
        self.skip_perforation = 0;
        self.horizontal_tabs.clear();
        for tabs in &mut self.vertical_tabs {
            tabs.clear();
        }
        self.channel = 0;
    }

    /// Every byte the CPC sent.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// What was printed, as text (the graphics and the styles are lost).
    pub fn text(&self) -> String {
        let mut text = self.text.clone();
        let line = Self::line_to_text(&self.line);
        if !line.trim().is_empty() {
            text.push_str(&line);
            text.push('\n');
        }
        text
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// The sheets with something printed on them.
    pub fn inked_pages(&self) -> impl Iterator<Item = (usize, &Page)> {
        self.pages.iter().enumerate().filter(|(_, page)| page.has_ink())
    }

    /// `prefix.txt` (the text), `prefix.prn` (the bytes), `prefix_001.png`... (the sheets).
    /// Returns the files written.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_files(&self, prefix: &str) -> std::io::Result<Vec<std::path::PathBuf>> {
        let mut written = Vec::new();
        let path = std::path::PathBuf::from(format!("{prefix}.txt"));
        std::fs::write(&path, self.text())?;
        written.push(path);
        let path = std::path::PathBuf::from(format!("{prefix}.prn"));
        std::fs::write(&path, &self.raw)?;
        written.push(path);
        for (index, page) in self.inked_pages() {
            let path = std::path::PathBuf::from(format!("{prefix}_{:03}.png", index + 1));
            page.save_png(&path)?;
            written.push(path);
        }
        Ok(written)
    }

    // ---- the protocol ----

    /// The printer's BUSY line.
    pub fn is_busy(&self) -> bool {
        self.busy_us > 0
    }

    /// The CPC's byte, strobed.
    pub fn receive(&mut self, byte: u8) {
        self.raw.push(byte);
        self.busy_us = BUSY_BYTE_US;
        let byte = match self.eighth_bit {
            EighthBit::Accept => byte,
            EighthBit::Set => byte | 0x80,
            EighthBit::Unset => byte & 0x7f
        };
        let parse = std::mem::replace(&mut self.parse, Parse::Idle);
        match parse {
            Parse::Idle => self.idle_byte(byte),
            Parse::Esc => self.escape(byte),
            Parse::Params { command, want, mut got } => {
                got.push(byte);
                if command == b'C' && got.len() == 1 && got[0] == 0 {
                    // by inches: one more
                    self.parse = Parse::Params { command, want: 2, got };
                }
                else if got.len() < want {
                    self.parse = Parse::Params { command, want, got };
                }
                else {
                    self.command(command, &got);
                }
            },
            Parse::List { command, channel, wants_channel, mut items } => {
                if wants_channel && channel.is_none() {
                    self.parse = Parse::List { command, channel: Some(byte), wants_channel, items };
                }
                else if byte == 0 || items.len() >= 32 {
                    self.list(command, channel, &items);
                }
                else {
                    items.push(byte);
                    self.parse = Parse::List { command, channel, wants_channel, items };
                }
            },
            Parse::Graphics { mode, nine_pin, remaining, mut column, dots } => {
                column.push(byte);
                let bytes_per_column = if nine_pin { 2 } else { 1 };
                let mut dots = dots;
                if column.len() == bytes_per_column {
                    self.graphics_column(mode, &column, dots);
                    column.clear();
                    dots += 1;
                }
                if remaining > 1 {
                    self.parse = Parse::Graphics { mode, nine_pin, remaining: remaining - 1, column, dots };
                }
                else {
                    // the head stays at the end of the dots
                    self.x += (dots as f32 * Self::dot_pitch(mode)).round() as i32;
                }
            },
            Parse::Download { mut header, mut data } => {
                if header.len() < 3 {
                    header.push(byte);
                    if header.len() == 3 && header[2] < header[1] {
                        return;
                    }
                    self.parse = Parse::Download { header, data };
                }
                else {
                    data.push(byte);
                    let count = usize::from(header[2] - header[1]) + 1;
                    if data.len() < count * 12 {
                        self.parse = Parse::Download { header, data };
                    }
                    else {
                        for (index, character) in data.chunks(12).enumerate() {
                            let code = usize::from(header[1]) + index;
                            let mut columns = [0u8; 11];
                            columns.copy_from_slice(&character[1..12]);
                            self.downloads[code] = Some(Download { start: character[0] >> 4 & 7, columns });
                        }
                    }
                }
            }
        }
    }

    /// Time passes: one microsecond.
    pub fn advance_us(&mut self) {
        self.busy_us = self.busy_us.saturating_sub(1);
    }

    fn idle_byte(&mut self, byte: u8) {
        match byte {
            ESC => self.parse = Parse::Esc,
            0x07 | 0x00 | 0x11 | 0x13 | 0x18 | 0x7f => {}, // BEL, NUL, DC1, DC3, CAN, DEL
            0x08 => self.x = (self.x - self.cell_width()).max(self.left),
            0x09 => self.horizontal_tab(),
            0x0a => self.line_feed(),
            0x0b => self.vertical_tab(),
            0x0c => self.form_feed(),
            0x0d => self.x = self.left,
            0x0e => self.double_width = true,
            0x0f => self.condensed = true,
            0x12 => self.condensed = false,
            0x14 => self.double_width = false,
            0..=0x1f if !self.control_code_printing => {},
            _ => self.print_character(byte)
        }
    }

    fn escape(&mut self, byte: u8) {
        match byte {
            // no parameters
            b'@' => {
                self.reset_settings();
            },
            b'<' => self.x = self.left,
            b'=' => self.eighth_bit = EighthBit::Unset,
            b'>' => self.eighth_bit = EighthBit::Set,
            b'#' => self.eighth_bit = EighthBit::Accept,
            b'0' => self.feed = 27,
            b'1' => self.feed = 21,
            b'2' => self.feed = DEFAULT_FEED,
            b'4' => self.italic = true,
            b'5' => self.italic = false,
            b'6' => self.extended_codes = true,
            b'7' => self.extended_codes = false,
            b'8' | b'9' => {},
            b'E' => self.bold = true,
            b'F' => self.bold = false,
            b'G' => self.double_strike = true,
            b'H' => self.double_strike = false,
            b'M' => self.elite = true,
            b'P' => self.elite = false,
            b'O' => self.skip_perforation = 0,
            b'T' => self.script = None,
            0x0e => self.double_width = true,
            0x0f => self.condensed = true,
            b'D' => self.parse = Parse::List { command: byte, channel: None, wants_channel: false, items: Vec::new() },
            b'B' => self.parse = Parse::List { command: byte, channel: None, wants_channel: false, items: Vec::new() },
            b'b' => self.parse = Parse::List { command: byte, channel: None, wants_channel: true, items: Vec::new() },
            b'&' => self.parse = Parse::Download { header: Vec::new(), data: Vec::new() },
            _ => {
                if let Some(want) = Self::parameter_count(byte) {
                    self.parse = Parse::Params { command: byte, want, got: Vec::new() };
                }
                // an unknown code is ignored
            }
        }
    }

    fn parameter_count(command: u8) -> Option<usize> {
        Some(match command {
            b'!' | b'-' | b'/' | b'3' | b'A' | b'C' | b'I' | b'J' | b'N' | b'Q' | b'R' | b'S' | b'U' | b'W'
            | b'i' | b'j' | b'l' | b'p' | b's' | b'x' => 1,
            b'?' | b'%' | b'K' | b'L' | b'Y' | b'Z' => 2,
            b'*' | b'^' | b'{' | b':' => 3,
            _ => return None
        })
    }

    fn command(&mut self, command: u8, params: &[u8]) {
        let n = params[0];
        match command {
            b'!' => {
                self.elite = n & 1 != 0;
                self.condensed = n & 4 != 0;
                self.bold = n & 8 != 0;
                self.double_strike = n & 16 != 0;
                self.double_width = n & 32 != 0;
                self.italic = n & 64 != 0;
                self.underline = n & 128 != 0;
            },
            b'-' => self.underline = n & 1 != 0,
            b'/' => self.channel = usize::from(n & 7),
            b'3' => self.feed = i32::from(n),
            b'A' => self.feed = i32::from(n) * 3,
            b'C' => {
                if params.len() == 2 {
                    self.page_length = i32::from(params[1]).clamp(1, 22) * FEED_UNITS_PER_INCH;
                }
                else {
                    self.page_length = i32::from(n).clamp(1, 127) * self.feed;
                }
            },
            b'I' => self.control_code_printing = n & 1 != 0,
            b'J' => self.advance_paper(i32::from(n)),
            b'j' => self.advance_paper(-i32::from(n)),
            b'N' => self.skip_perforation = i32::from(n).clamp(1, 127) * self.feed,
            b'Q' => self.right = (i32::from(n) * self.plain_cell_width()).clamp(1, LINE_PX),
            b'l' => self.left = (i32::from(n) * self.plain_cell_width()).min(LINE_PX),
            b'R' => self.international_set = n.min(8),
            b'S' => self.script = Some(if n & 1 != 0 { Script::Sub } else { Script::Super }),
            b'U' | b's' | b'i' => {},
            b'W' => self.double_width = n & 1 != 0,
            b'p' => self.proportional = n & 1 != 0,
            b'x' => self.nlq = n & 1 != 0,
            b'%' => self.download_selected = n & 1 != 0,
            b':' => {}, // the internal set is what the download set falls back to
            b'?' => {
                let which = match n {
                    b'K' => Some(0),
                    b'L' => Some(1),
                    b'Y' => Some(2),
                    b'Z' => Some(3),
                    _ => None
                };
                if let Some(which) = which {
                    self.density_modes[which] = params[1];
                }
            },
            b'K' | b'L' | b'Y' | b'Z' => {
                let which = match command {
                    b'K' => 0,
                    b'L' => 1,
                    b'Y' => 2,
                    _ => 3
                };
                let count = usize::from(params[0]) + 256 * usize::from(params[1]);
                self.start_graphics(self.density_modes[which], false, count);
            },
            b'*' => {
                let count = usize::from(params[1]) + 256 * usize::from(params[2]);
                self.start_graphics(n, false, count);
            },
            b'^' | b'{' => {
                let count = usize::from(params[1]) + 256 * usize::from(params[2]);
                // 0: 480 dots, 4: 960 dots
                self.start_graphics(if n == 4 { 1 } else { 0 }, true, count);
            },
            _ => {}
        }
    }

    fn list(&mut self, command: u8, channel: Option<u8>, items: &[u8]) {
        match command {
            b'D' => {
                self.horizontal_tabs = items.iter().map(|&column| i32::from(column)).collect();
            },
            b'B' => {
                let feed = self.feed;
                self.vertical_tabs[0] = items.iter().map(|&line| i32::from(line) * feed).collect();
            },
            b'b' => {
                let feed = self.feed;
                let channel = usize::from(channel.unwrap_or(0) & 7);
                self.vertical_tabs[channel] = items.iter().map(|&line| i32::from(line) * feed).collect();
            },
            _ => {}
        }
    }

    // ---- the paper ----

    fn line_feed(&mut self) {
        self.flush_line();
        self.advance_paper(self.feed);
    }

    /// Feed (or unfeed, negative) the paper, a page further when the foot of the page is passed.
    fn advance_paper(&mut self, units: i32) {
        self.y += units;
        if self.y < 0 {
            self.y = 0;
        }
        let foot = self.page_length - self.skip_perforation - self.feed.max(DEFAULT_FEED);
        if units > 0 && self.y > foot {
            self.next_page();
        }
    }

    fn next_page(&mut self) {
        self.page += 1;
        self.y = TOP_OF_FORM;
    }

    fn form_feed(&mut self) {
        self.flush_line();
        self.next_page();
        self.x = self.left;
    }

    fn vertical_tab(&mut self) {
        self.flush_line();
        let top = TOP_OF_FORM;
        let next = self.vertical_tabs[self.channel].iter().map(|&tab| top + tab).find(|&tab| tab > self.y);
        match next {
            Some(tab) => self.y = tab,
            None => self.advance_paper(self.feed)
        }
    }

    fn horizontal_tab(&mut self) {
        let cell = self.plain_cell_width();
        let column = self.x / cell.max(1);
        let target = if self.horizontal_tabs.is_empty() {
            Some((column / 8 + 1) * 8)
        }
        else {
            self.horizontal_tabs.iter().copied().find(|&tab| tab > column)
        };
        if let Some(target) = target {
            self.x = (target * cell).min(self.right);
        }
    }

    /// Text of the line, then forgotten.
    fn flush_line(&mut self) {
        let line = Self::line_to_text(&self.line);
        self.text.push_str(&line);
        self.text.push('\n');
        self.line.clear();
    }

    /// The characters of a line, a column (10 cpi) each, in the order of the head's positions;
    /// a narrower pitch makes them follow each other.
    fn line_to_text(line: &[(i32, char)]) -> String {
        let mut sorted = line.to_vec();
        sorted.sort_by_key(|&(x, _)| x);
        let mut cells: Vec<(i32, char)> = Vec::new();
        let mut last_x = None;
        for (x, character) in sorted {
            if last_x == Some(x) {
                // struck again: the last one wins
                cells.last_mut().expect("a first one").1 = character;
                continue;
            }
            let mut column = (x + 12) / 24;
            if let Some(&(previous, _)) = cells.last() {
                column = column.max(previous + 1);
            }
            cells.push((column, character));
            last_x = Some(x);
        }
        let mut text = String::new();
        for (column, character) in cells {
            while (text.chars().count() as i32) < column {
                text.push(' ');
            }
            text.push(character);
        }
        text.trim_end().to_string()
    }

    // ---- the characters ----

    /// Width of a character cell with the pitch the printer has, 1/240 inch, double width
    /// included.
    fn cell_width(&self) -> i32 {
        let plain = self.plain_cell_width();
        if self.double_width {
            plain * 2
        }
        else {
            plain
        }
    }

    /// The same, not double.
    fn plain_cell_width(&self) -> i32 {
        match (self.elite, self.condensed) {
            (false, false) => 24, // 10 cpi
            (true, false) => 20,  // 12 cpi
            (false, true) => 14,  // 17 cpi
            (true, true) => 12    // 20 cpi
        }
    }

    fn glyph(&self, code: u8) -> Glyph {
        if self.download_selected {
            if let Some(download) = &self.downloads[usize::from(code)] {
                let columns: Vec<u16> = download
                    .columns
                    .iter()
                    .map(|&data| (0..8).filter(|pin| data & (0x80 >> pin) != 0).map(|pin| 1u16 << pin).sum::<u16>())
                    .collect();
                let first = usize::from(download.start).min(columns.len());
                return Glyph { columns, first, last: 10 };
            }
        }
        let rows = &self.font[usize::from(code)];
        let columns: Vec<u16> = (0..8)
            .map(|column| (0..8).filter(|row| rows[*row] & (0x80 >> column) != 0).map(|row| 1u16 << row).sum::<u16>())
            .collect();
        let used: Vec<usize> = (0..8).filter(|&column| columns[column] != 0).collect();
        let (first, last) = (used.first().copied().unwrap_or(0), used.last().copied().unwrap_or(0));
        Glyph { columns, first, last }
    }

    fn print_character(&mut self, code: u8) {
        let glyph = self.glyph(code);
        let cell = self.cell_width();
        let advance = if self.proportional && !self.download_selected {
            let columns = if code == b' ' { 5 } else { glyph.last.saturating_sub(glyph.first) + 1 + 2 };
            let column_width = self.plain_cell_width() as f32 / 12.0 * if self.double_width { 2.0 } else { 1.0 };
            (columns as f32 * column_width).round() as i32
        }
        else {
            cell
        };

        if self.x + advance > self.right {
            // the line is full: a new one starts
            self.flush_line();
            self.x = self.left;
            self.advance_paper(self.feed);
        }

        let printable = if (0x20..0x7f).contains(&code) || code >= 0xa0 {
            Some(char::from(code))
        }
        else {
            None
        };
        if let Some(character) = printable {
            self.line.push((self.x, character));
        }

        self.strike_glyph(&glyph);
        self.x += advance;
    }

    fn strike_glyph(&mut self, glyph: &Glyph) {
        if glyph.columns.iter().all(|&pins| pins == 0) && !self.underline {
            return;
        }
        let double = if self.double_width { 2.0 } else { 1.0 };
        let column_pitch = self.plain_cell_width() as f32 / 12.0 * double;
        let shift = if self.proportional && !self.download_selected {
            glyph.first as f32 * column_pitch - column_pitch
        }
        else {
            0.0
        };
        let passes: &[(f32, f32)] = match (self.bold || self.nlq, self.double_strike || self.nlq) {
            (false, false) => &[(0.0, 0.0)],
            (true, false) => &[(0.0, 0.0), (1.0, 0.0)],
            (false, true) => &[(0.0, 0.0), (0.0, 1.0)],
            (true, true) => &[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
        };
        let strength = if passes.len() == 1 { 0.85 } else { 0.6 };
        let radius = if self.nlq { 1.2 } else { 1.5 } * if self.condensed { 0.9 } else { 1.0 };

        for (column, &pins) in glyph.columns.iter().enumerate() {
            for pin in 0..8 {
                if pins & (1 << pin) == 0 {
                    continue;
                }
                // sub and superscripts squeeze the 8 needles in 4
                let row = match self.script {
                    None => pin as f32,
                    Some(Script::Super) => pin as f32 / 2.0,
                    Some(Script::Sub) => pin as f32 / 2.0 + 5.0
                };
                let slant = if self.italic { (7.0 - pin as f32) * column_pitch / 4.0 } else { 0.0 };
                // a double width dot is struck twice, side by side
                let sides: &[f32] = if self.double_width { &[0.0, 0.5] } else { &[0.0] };
                for (&(dx, dy), &side) in passes.iter().flat_map(|pass| sides.iter().map(move |side| (pass, side))) {
                    let x = side * column_pitch + LEFT_PX as f32 + self.x as f32 + column as f32 * column_pitch + 1.0 - shift + slant + dx;
                    let y = feed_px(self.y) + feed_px(PIN_PITCH) * row + dy;
                    self.strike(x, y, radius, strength);
                }
            }
        }
        if self.underline {
            let pitch = column_pitch.max(1.0);
            let mut x = 0.0;
            let cell = self.cell_width() as f32;
            while x < cell {
                self.strike(LEFT_PX as f32 + self.x as f32 + x + 1.0, feed_px(self.y) + feed_px(PIN_PITCH) * 8.0, 1.2, 0.85);
                x += pitch;
            }
        }
    }

    fn strike(&mut self, x: f32, y: f32, radius: f32, strength: f32) {
        while self.pages.len() <= self.page {
            let length = self.page_length;
            self.pages.push(Page::new(length));
        }
        let page = &mut self.pages[self.page];
        page.dot(x, y, radius, strength);
    }

    // ---- the graphics ----

    /// Pixels between two dots of the bit image mode.
    fn dot_pitch(mode: u8) -> f32 {
        match mode {
            0 => 4.0,      // 480 dots in 8 inch
            1 | 2 => 2.0,  // 960
            3 => 1.0,      // 1920
            4 => 3.75,     // 640
            5 => 8.0 * DPI as f32 / 576.0,
            6 => 8.0 * DPI as f32 / 720.0,
            _ => 4.0
        }
    }

    fn start_graphics(&mut self, mode: u8, nine_pin: bool, count: usize) {
        if count == 0 {
            return;
        }
        let bytes = count * if nine_pin { 2 } else { 1 };
        self.parse = Parse::Graphics { mode, nine_pin, remaining: bytes, column: Vec::new(), dots: 0 };
    }

    fn graphics_column(&mut self, mode: u8, column: &[u8], dots: i32) {
        let pitch = Self::dot_pitch(mode);
        let x = self.x as f32 + dots as f32 * pitch;
        if x >= self.right as f32 || x >= LINE_PX as f32 {
            return; // past the end of the line: ignored
        }
        let radius = (pitch * 0.7).clamp(1.0, 1.5);
        for pin in 0..8 {
            if column[0] & (0x80 >> pin) != 0 {
                let y = feed_px(self.y) + feed_px(PIN_PITCH) * pin as f32;
                self.strike(LEFT_PX as f32 + x + 1.0, y, radius, 0.9);
            }
        }
        if column.len() == 2 && column[1] & 0x80 != 0 {
            let y = feed_px(self.y) + feed_px(PIN_PITCH) * 8.0;
            self.strike(LEFT_PX as f32 + x + 1.0, y, radius, 0.9);
        }
    }
}

impl PrinterDevice for Dmp2000 {
    fn update(&mut self, port: &PrinterPort) {
        // the byte is taken when /STROBE falls
        let active = !port.get_pin(PrinterPin::Neg_STROBE);
        if active && !self.strobe_active {
            self.receive(port.data());
        }
        self.strobe_active = active;
    }

    fn busy(&self) -> Option<bool> {
        Some(self.is_busy())
    }

    fn advance_us(&mut self) {
        Dmp2000::advance_us(self);
    }

    fn has_sound(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn printer() -> Dmp2000 {
        Dmp2000::with_os_rom(crate::roms::OS6128)
    }

    fn send(printer: &mut Dmp2000, bytes: &[u8]) {
        for &byte in bytes {
            printer.receive(byte);
        }
    }

    fn ink(printer: &Dmp2000) -> u64 {
        printer.pages().iter().flat_map(|page| page.ink.iter()).map(|&ink| u64::from(ink)).sum()
    }

    #[test]
    fn text_comes_out_as_text() {
        let mut printer = printer();
        send(&mut printer, b"Hello\r\n  World\r\n");
        assert_eq!(printer.text(), "Hello\n  World\n");
        assert!(ink(&printer) > 0);
    }

    #[test]
    fn a_new_page_after_a_form_feed() {
        let mut printer = printer();
        send(&mut printer, b"one\x0ctwo");
        assert_eq!(printer.inked_pages().map(|(index, _)| index).collect::<Vec<_>>(), vec![0, 1]);
    }

    #[test]
    fn the_page_breaks_by_itself() {
        let mut printer = printer();
        for _ in 0..70 {
            send(&mut printer, b"line\r\n");
        }
        assert_eq!(printer.inked_pages().count(), 2);
    }

    #[test]
    fn escape_sequences_are_not_printed() {
        let mut printer = printer();
        send(&mut printer, &[ESC, b'E', b'A', ESC, b'F', ESC, b'-', 1, b'B', ESC, b'3', 20, b'C']);
        assert_eq!(printer.text(), "ABC\n");
    }

    #[test]
    fn the_bit_image_is_dots_whatever_the_bytes() {
        let mut printer = printer();
        // 3 columns of single density: control codes are dots too
        send(&mut printer, &[ESC, b'K', 3, 0, 0x80, 0x0d, 0x01]);
        assert_eq!(printer.text(), "");
        assert!(ink(&printer) > 0);
        // and back to characters
        send(&mut printer, b"\rA");
        assert_eq!(printer.text(), "A\n");
    }

    #[test]
    fn bit_image_dots_keep_their_pins_apart() {
        let mut printer = printer();
        send(&mut printer, &[ESC, b'K', 1, 0, 0x80]);
        let top = ink(&printer);
        let mut other = Dmp2000::with_os_rom(crate::roms::OS6128);
        send(&mut other, &[ESC, b'K', 1, 0, 0x01]);
        // the same dot, a needle (7/72 inch) lower: the same ink, not the same place
        assert!(top.abs_diff(ink(&other)) * 50 < top);
        let rows = |printer: &Dmp2000| -> Vec<i32> {
            let page = &printer.pages()[0];
            (0..page.height).filter(|&y| (0..PAPER_WIDTH_PX).any(|x| page.ink[(y * PAPER_WIDTH_PX + x) as usize] > 0)).collect()
        };
        assert!(rows(&other)[0] > rows(&printer)[0] + 15);
    }

    #[test]
    fn a_nine_pin_column_takes_two_bytes() {
        let mut printer = printer();
        send(&mut printer, &[ESC, b'^', 0, 2, 0, 0xff, 0x00, 0x80, 0x00, b'A']);
        assert_eq!(printer.text(), "A\n");
    }

    #[test]
    fn the_head_goes_where_the_tabs_say() {
        let mut printer = printer();
        send(&mut printer, b"a\tb\r\n");
        assert_eq!(printer.text(), format!("a{}b\n", " ".repeat(7)));
    }

    #[test]
    fn margins_and_automatic_line_feeds() {
        let mut printer = printer();
        send(&mut printer, &[ESC, b'Q', 5]);
        send(&mut printer, b"abcdefgh");
        assert_eq!(printer.text(), "abcde\nfgh\n");
    }

    #[test]
    fn the_strobe_gives_the_byte() {
        let mut port = PrinterPort::default();
        port.plug(Box::new(printer()));
        // data, then /STROBE low (bit 7 set) and high again
        for value in [b'H', b'H' | 0x80, b'H', b'i', b'i' | 0x80, b'i'] {
            port.latch(value);
        }
        let device = port.device().unwrap().as_any().downcast_ref::<Dmp2000>().unwrap();
        assert_eq!(device.raw(), b"Hi");
    }

    #[test]
    fn busy_goes_down_by_itself() {
        let mut printer = printer();
        assert!(!printer.is_busy());
        printer.receive(b'A');
        assert!(printer.is_busy());
        for _ in 0..BUSY_BYTE_US {
            printer.advance_us();
        }
        assert!(!printer.is_busy());
    }

    #[test]
    fn the_sheet_has_its_holes() {
        let mut printer = printer();
        send(&mut printer, b"x");
        let page = &printer.pages()[0];
        let rgb = page.to_rgb();
        assert_eq!(rgb.len(), (page.width() * page.height() * 3) as usize);
        // the first hole's centre is dark, the paper beside it is not
        let pixel = |x: i32, y: i32| rgb[((y * PAPER_WIDTH_PX + x) * 3) as usize];
        assert!(pixel(STRIP_PX / 2, DPI / 4) < 100);
        assert!(pixel(PAPER_WIDTH_PX / 2, DPI * 5) > 200);
    }
}
