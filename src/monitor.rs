//! A CRT monitor, fed one 16MHz tick at a time with what the gate array
//! outputs (RGB levels and C-SYNC), and drawing what its beam would show.
//!
//! Like a real monitor it knows nothing about the CRTC: it only reacts to
//! the synchronisation pulses it receives.
//!  - a pulse has to last a little to be a synchronisation (ACCC 9.3.1: the
//!    2-3 pixel C-HSYNC of R3l=2 or an R3.JIT cut is "much too short to be
//!    detected by the monitor");
//!  - the end of an HSYNC pulse brings the beam back to the left of the next
//!    line; with no pulse, the line oscillator free-runs at its own period;
//!  - a VSYNC pulse brings the beam back to the top; without it the frame
//!    oscillator free-runs too;
//!  - the beam is blanked while it flies back, and the tube shows a limited
//!    number of pixels per line and lines per frame.
use crate::ga::{RgbLevels, Sync};
use image::*;

/// Ticks (1/16 usec) of a line the line oscillator runs when no HSYNC
/// comes: 64 usec.
const RASTER_LINE_LENGTH: usize = 64 * 16;
/// What a CTM 644 shows (ACCC 23.2, 23.3, 24): 48 usec of a line, starting
/// 8 usec (the 15th character after C0=R2, with a 4 usec C-HSYNC) after the
/// end of the C-HSYNC; 272 lines, from the 34th after the start of the VSYNC.
const MONITOR_FIRST_PIXEL: usize = 8 * 16;
const MONITOR_ACTIVE_PIXELS: usize = 48 * 16;
const MONITOR_HEIGHT: usize = 272;
const MONITOR_FIRST_LINE: usize = 33;
/// Lines the frame oscillator runs when no VSYNC comes (a little more than
/// a 50Hz frame, so a VSYNC always arrives first when there is one).
const MONITOR_FREE_RUN_LINES: usize = 350;

/// Once locked, the line oscillator (AFC) only follows an HSYNC that ends
/// in the last quarter of its period: a pulse half a line away (a second,
/// cut-short HSYNC of the R3.JIT tests) is ignored.
const MONITOR_HSYNC_WINDOW_START: usize = RASTER_LINE_LENGTH * 3 / 4;
/// Lines without an accepted HSYNC after which the line oscillator unlocks.
const MONITOR_HSYNC_LOST_LINES: usize = 4;

/// Shortest C-HSYNC pulse (in ticks of 1/16 usec) the monitor reacts to.
const MONITOR_MIN_HSYNC_TICKS: usize = 16;
/// Shortest C-VSYNC pulse the monitor reacts to: it integrates the signal.
const MONITOR_MIN_VSYNC_TICKS: usize = 64;

const HSYNC_DEBUG_COLOR: Rgb<u8> = Rgb([0, 128, 255]);
const VSYNC_DEBUG_COLOR: Rgb<u8> = Rgb([0, 64, 255]);

const HSYNC_COLOR: Rgb<u8> = Rgb([0, 0, 0]);
const VSYNC_COLOR: Rgb<u8> = Rgb([0, 0, 0]);

pub struct Monitor {
    /// What has been drawn since the last vertical synchronisation.
    texture: RgbImage,
    /// The last whole frame, until taken.
    frame: Option<RgbImage>,
    /// Beam position: ticks since the last line start; lines since the last
    /// frame start.
    x: usize,
    line: usize,
    /// Ticks the HSYNC/VSYNC input has been high.
    hsync_run: usize,
    vsync_run: usize,
    /// The VSYNC pulse has already restarted the frame.
    vsync_seen: bool,
    /// The line oscillator follows the HSYNC pulses; counts the lines it ran
    /// free since.
    locked: bool,
    free_lines: usize,
    /// A VSYNC has been seen at least once: only then a frame is whole.
    synchronised: bool,
    /// Ticks since the last frame start, for the frame oscillator.
    vsync_color: Rgb<u8>,
    hsync_color: Rgb<u8>,
}

impl Default for Monitor {
    fn default() -> Self {
        Monitor::with_colors(VSYNC_COLOR, HSYNC_COLOR)
    }
}

impl Monitor {
    fn with_colors(vsync_color: Rgb<u8>, hsync_color: Rgb<u8>) -> Self {
        Monitor {
            texture: RgbImage::new(MONITOR_ACTIVE_PIXELS as u32, MONITOR_HEIGHT as u32),
            frame: None,
            x: 0,
            line: 0,
            hsync_run: 0,
            vsync_run: 0,
            vsync_seen: false,
            locked: false,
            free_lines: 0,
            synchronised: false,
            vsync_color,
            hsync_color
        }
    }

    /// Generate a debug monitor that colours vbl and vsync
    pub fn debug() -> Self {
        Monitor::with_colors(VSYNC_DEBUG_COLOR, HSYNC_DEBUG_COLOR)
    }

    /// Whether a whole frame is waiting to be taken.
    pub fn is_frame_complete(&self) -> bool {
        self.frame.is_some()
    }

    /// The picture being drawn (or, once a frame is complete, the one just
    /// drawn until the beam starts the next).
    pub fn image(&self) -> &RgbImage {
        self.frame.as_ref().unwrap_or(&self.texture)
    }

    /// The last whole frame seen, if any (consumed).
    pub fn take_frame(&mut self) -> Option<RgbImage> {
        self.frame.take()
    }

    /// Handle the information sent from the CPC by the GA, one 16MHz tick.
    pub fn receive(&mut self, rgb: RgbLevels, Sync { vsync, hsync }: Sync) {
        let hsync_pulse = self.hsync_run;
        if hsync {
            self.hsync_run += 1;
        }
        else {
            self.hsync_run = 0;
        }
        if vsync {
            self.vsync_run += 1;
        }
        else {
            self.vsync_run = 0;
            self.vsync_seen = false;
        }

        // vertical synchronisation: the beam flies back to the top
        let mut frame_start = false;
        if self.vsync_run > MONITOR_MIN_VSYNC_TICKS && !self.vsync_seen {
            self.vsync_seen = true;
            frame_start = true;
        }

        // horizontal synchronisation: end of a pulse long enough, or the
        // oscillator running out
        let pulse_end = hsync_pulse > MONITOR_MIN_HSYNC_TICKS
            && !hsync
            && (!self.locked || self.x >= MONITOR_HSYNC_WINDOW_START);
        let free_run = self.x >= RASTER_LINE_LENGTH;
        let line_end = pulse_end || free_run;
        if pulse_end {
            self.locked = true;
            self.free_lines = 0;
        }
        else if free_run {
            self.free_lines += 1;
            if self.free_lines >= MONITOR_HSYNC_LOST_LINES {
                self.locked = false;
            }
        }
        if frame_start {
            self.start_frame();
        }
        if line_end {
            self.x = 0;
            self.line += 1;
            if self.line >= MONITOR_FREE_RUN_LINES {
                // no VSYNC: the frame oscillator rolls the picture
                self.end_frame();
                self.synchronised = true;
            }
        }

        let y = self.line.wrapping_sub(MONITOR_FIRST_LINE);
        let px = self.x.wrapping_sub(MONITOR_FIRST_PIXEL);
        if px < MONITOR_ACTIVE_PIXELS && y < MONITOR_HEIGHT {
            let pixel = if vsync {
                self.vsync_color
            }
            else if hsync {
                self.hsync_color
            }
            else {
                pixel_of(rgb)
            };
            self.texture.put_pixel(px as u32, y as u32, pixel);
        }
        self.x += 1;
    }

    fn start_frame(&mut self) {
        if self.synchronised {
            self.end_frame();
        }
        else {
            self.texture = RgbImage::new(MONITOR_ACTIVE_PIXELS as u32, MONITOR_HEIGHT as u32);
            self.line = 0;
        }
        self.synchronised = true;
    }

    fn end_frame(&mut self) {
        self.frame = Some(std::mem::replace(
            &mut self.texture,
            RgbImage::new(MONITOR_ACTIVE_PIXELS as u32, MONITOR_HEIGHT as u32)
        ));
        self.line = 0;
    }
}

/// RGB intensities (percent) to 8-bit pixels.
fn pixel_of(rgb: RgbLevels) -> Rgb<u8> {
    let to_u8 = |v: f32| -> u8 { (v * 255f32 / 100.0).min(255f32) as u8 };
    Rgb([to_u8(rgb.r()), to_u8(rgb.g()), to_u8(rgb.b())])
}

#[cfg(test)]
mod test {
    use super::*;

    fn feed(mon: &mut Monitor, ticks: usize, rgb: (f32, f32, f32), vsync: bool, hsync: bool) {
        for _ in 0..ticks {
            mon.receive(rgb.into(), Sync { vsync, hsync });
        }
    }

    /// A CRTC-like frame: `lines` lines of 1024 ticks, HSYNC from tick 736
    /// for `hsync_ticks`, VSYNC on the first 16 lines.
    fn frame(mon: &mut Monitor, lines: usize, hsync_ticks: usize) {
        for l in 0..lines {
            let vsync = l < 16;
            for t in 0..RASTER_LINE_LENGTH {
                let hsync = t >= 736 && t < 736 + hsync_ticks;
                let rgb = if hsync || vsync { (0., 0., 0.) } else { (100., 0., 0.) };
                mon.receive(rgb.into(), Sync { vsync, hsync });
            }
        }
    }

    #[test]
    fn test_color() {
        assert_eq!(pixel_of((0., 0., 0.).into()), Rgb([0, 0, 0]));
        assert_eq!(pixel_of((100., 100., 100.).into()), Rgb([255, 255, 255]));
    }

    #[test]
    fn a_frame_is_complete_at_the_next_vsync_only() {
        let mut mon = Monitor::default();
        frame(&mut mon, 312, 64);
        assert!(!mon.is_frame_complete());
        frame(&mut mon, 312, 64);
        assert!(mon.is_frame_complete());
        let image = mon.take_frame().unwrap();
        assert_eq!(image.dimensions(), (MONITOR_ACTIVE_PIXELS as u32, MONITOR_HEIGHT as u32));
        // picture is red, with the first lines after the retrace
        assert_eq!(*image.get_pixel(10, 10), Rgb([255, 0, 0]));
    }

    #[test]
    fn lines_start_after_the_end_of_each_hsync() {
        let mut mon = Monitor::default();
        frame(&mut mon, 312, 64);
        frame(&mut mon, 312, 64);
        let image = mon.take_frame().unwrap();
        // the line restarts at the end of the HSYNC (tick 800 of the CRTC
        // line): the HSYNC itself falls beyond the 832 shown pixels
        assert_eq!(*image.get_pixel(0, 100), Rgb([255, 0, 0]));
        assert_eq!(*image.get_pixel(MONITOR_ACTIVE_PIXELS as u32 - 1, 100), Rgb([255, 0, 0]));
    }

    #[test]
    fn a_too_short_hsync_does_not_synchronise() {
        let mut short = Monitor::default();
        frame(&mut short, 312, 8);
        frame(&mut short, 312, 8);
        // lines free-run on the same period: same line count as the CRTC's
        let image = short.take_frame().unwrap();
        assert_eq!(*image.get_pixel(0, 100), Rgb([255, 0, 0]));
    }

    #[test]
    fn without_vsync_the_frame_oscillator_rolls_the_picture() {
        let mut mon = Monitor::default();
        for _ in 0..(MONITOR_FREE_RUN_LINES + 1) * RASTER_LINE_LENGTH {
            mon.receive((100., 0., 0.).into(), Sync { vsync: false, hsync: false });
        }
        assert!(mon.is_frame_complete());
    }

    #[test]
    fn a_vsync_too_short_is_ignored() {
        let mut mon = Monitor::default();
        frame(&mut mon, 312, 64);
        feed(&mut mon, MONITOR_MIN_VSYNC_TICKS, (0., 0., 0.), true, false);
        assert!(!mon.is_frame_complete());
    }
}
