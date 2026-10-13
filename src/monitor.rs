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
/// Three pixels earlier than that: where AMSpiriT puts the 640 pixels of a
/// standard screen (x=64..703 of its 768 wide picture).
const MONITOR_FIRST_PIXEL: usize = 8 * 16 - 2;
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

/// Pulses (in ticks) too short or too long to be taken for a line sync.
const MONITOR_HPULSE_MIN: usize = 4;
const MONITOR_HPULSE_MAX: usize = 128;
/// A second sync inside a line pulls the picture (the monitor's AFC
/// discriminator), measured on Shaker D3/AT/AY against AmSpiriT: the pull is
/// the pulse width less 9 ticks times 5/12 when it comes after the line sync
/// (the picture moves left) and 4/7 of that when it comes before (to the
/// right); it is flat over most of the half line and falls to nothing at 31
/// characters after (20 before); a pulse within 6 characters belongs to the
/// same sync complex. The beam follows it by 1 to 3 ticks a line.
const MONITOR_PULL_DEAD_WIDTH: i32 = 9;
const MONITOR_PULL_DEADZONE: i64 = 6 * 16;
const MONITOR_PULL_FLAT_LATE: i64 = 422;
const MONITOR_PULL_END_LATE: i64 = 496;
const MONITOR_PULL_FLAT_EARLY: i64 = 12 * 16;
const MONITOR_PULL_END_EARLY: i64 = 20 * 16;

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
    /// Ticks since the beginning of time, to date the sync pulses.
    now: u64,
    /// The other sync pulses (start, width) seen since the last line sync.
    pulses: Vec<(u64, i32)>,
    /// Start of the last line sync.
    last_sync_start: Option<u64>,
    /// How far (ticks, + = right) the second-sync discriminator holds the picture.
    shift: i32,
    /// What the discriminator decided at the last sync: the beam follows it
    /// from the next line (the pulses it looks at come after the flyback).
    next_shift: i32,
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
            now: 0,
            pulses: Vec::new(),
            last_sync_start: None,
            shift: 0,
            next_shift: 0,
            vsync_color,
            hsync_color
        }
    }

    /// Generate a debug monitor that colours vbl and vsync
    pub fn debug() -> Self {
        Monitor::with_colors(VSYNC_DEBUG_COLOR, HSYNC_DEBUG_COLOR)
    }

    /// Where the beam is: (tick in the line, 16 per usec - so 1024 per 64
    /// usec line; line since the frame started). The picture's pixel
    /// (`x`, `y`) is beam (`x + crop_x`, `y + crop_y`), see
    /// [`Self::crop_origin`].
    pub fn beam(&self) -> (usize, usize) {
        (self.x, self.line)
    }

    /// Where the picture starts in beam coordinates (the first pixel of the
    /// visible area: the image's (0, 0)).
    pub fn crop_origin(&self) -> (isize, usize) {
        (MONITOR_FIRST_PIXEL as isize - self.shift as isize, MONITOR_FIRST_LINE)
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
        // the sync pulses, dated, for the second-sync discriminator
        if hsync_pulse > 0 && !hsync && !vsync
            && (MONITOR_HPULSE_MIN..=MONITOR_HPULSE_MAX).contains(&hsync_pulse)
        {
            let start = self.now - hsync_pulse as u64;
            if pulse_end {
                self.discriminate(start);
                self.last_sync_start = Some(start);
                self.pulses.clear();
            }
            else if self.pulses.len() < 8 {
                self.pulses.push((start, hsync_pulse as i32));
            }
        }
        self.now += 1;
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
        let px = (self.x as isize - MONITOR_FIRST_PIXEL as isize + self.shift as isize) as usize;
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

    /// A line sync starting at `start` has been accepted: look at the other
    /// pulse that came since the previous one (the discriminator answers to
    /// exactly one) and let the beam follow it.
    fn discriminate(&mut self, start: u64) {
        let mut target = 0i32;
        if let (Some(previous), [(other, width)]) = (self.last_sync_start, self.pulses.as_slice()) {
            let after = (*other - previous) as i64; // late for the previous sync
            let before = (start - *other) as i64; // early for this one
            let late_pull = ((*width - MONITOR_PULL_DEAD_WIDTH).max(0) * 5 + 6) / 12;
            let taper = |pull: i32, distance: i64, flat: i64, end: i64| -> i32 {
                if distance <= flat {
                    pull
                } else if distance >= end {
                    0
                } else {
                    ((pull as i64 * (end - distance) + (end - flat) / 2) / (end - flat)) as i32
                }
            };
            if after > MONITOR_PULL_DEADZONE && after < MONITOR_PULL_END_LATE {
                target = -taper(late_pull, after, MONITOR_PULL_FLAT_LATE, MONITOR_PULL_END_LATE);
            } else if before > MONITOR_PULL_DEADZONE && before < MONITOR_PULL_END_EARLY {
                let early_pull = (late_pull * 4 + 3) / 7;
                target = taper(early_pull, before, MONITOR_PULL_FLAT_EARLY, MONITOR_PULL_END_EARLY);
            }
        }
        self.shift = self.next_shift;
        if target == 0 {
            // no second sync: the lock takes the picture back at once
            self.next_shift = 0;
        } else {
            let error = target - self.next_shift;
            let step = if error.abs() > 5 { 3 } else { 1 };
            self.next_shift += error.signum() * step.min(error.abs());
        }
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

    /// A second sync 29 characters after the line sync (Shaker AY): the beam
    /// follows it a few ticks a line to the left and the picture goes back
    /// at once when it goes away.
    #[test]
    fn a_second_sync_pulls_the_picture() {
        let mut mon = Monitor::default();
        for l in 0..40 {
            for t in 0..RASTER_LINE_LENGTH {
                let main = t >= 736 && t < 800;
                let other = l >= 10 && t >= 176 && t < 240;
                let hsync = main || other;
                mon.receive((100., 0., 0.).into(), Sync { vsync: false, hsync });
            }
            if l == 30 {
                assert_eq!(mon.shift, -10, "the plateau of the pull");
            }
        }
        for _ in 0..3 {
            for t in 0..RASTER_LINE_LENGTH {
                let hsync = t >= 736 && t < 800;
                mon.receive((100., 0., 0.).into(), Sync { vsync: false, hsync });
            }
        }
        assert_eq!(mon.shift, 0);
    }
}
