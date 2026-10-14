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
const MONITOR_HSYNC_WINDOW_START: usize = RASTER_LINE_LENGTH - 48;
/// Lines without an accepted HSYNC after which the line oscillator unlocks.
const MONITOR_HSYNC_LOST_LINES: usize = 4;

/// Shortest C-HSYNC pulse (in ticks of 1/16 usec) the monitor reacts to.
const MONITOR_MIN_HSYNC_TICKS: usize = 12;
/// Lines after the beam started a frame from which the frame oscillator accepts a VSYNC
/// (Shaker DT: pulses 156 lines apart are not followed, 312 are).
const MONITOR_VSYNC_WINDOW_START_LINES: usize = 200;
const MONITOR_VSYNC_BLANK_MIN_TICKS: i64 = 15 * 1024;
const MONITOR_VSYNC_BLANK_AFTER_TICKS: i64 = 10 * 1024;
/// Shortest C-VSYNC pulse the monitor reacts to: it integrates the signal.
const MONITOR_MIN_VSYNC_TICKS: usize = 64;

/// Pulses (in ticks) too short or too long to be taken for a line sync.
/// The phase detector of the line oscillator (see `follow_sync`)
const MONITOR_PLL_MAX_PULL: i64 = 12;
const MONITOR_PLL_CAPTURE_EARLY: i64 = 48;
const MONITOR_PLL_CAPTURE_LATE: i64 = 36;

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
/// ... but never less than this for a second sync (Shaker D3 with R3l=3: a 1 usec pulse pulls as much as a 2 usec one)
const MONITOR_PULL_MIN: i32 = 10;
const MONITOR_PULL_DEADZONE: i64 = 6 * 16;
const MONITOR_PULL_FLAT_LATE: i64 = 422;
const MONITOR_PULL_END_LATE: i64 = 496;

const HSYNC_DEBUG_COLOR: Rgb<u8> = Rgb([0, 128, 255]);
const VSYNC_DEBUG_COLOR: Rgb<u8> = Rgb([0, 64, 255]);

const HSYNC_COLOR: Rgb<u8> = Rgb([0, 0, 0]);
const VSYNC_COLOR: Rgb<u8> = Rgb([0, 0, 0]);

pub struct Monitor {
    /// What has been drawn since the last vertical synchronisation.
    texture: RgbImage,
    /// The last whole frame, until taken.
    frame: Option<RgbImage>,
    /// The whole frames, sync and borders included, when asked for (see
    /// [`Monitor::keep_full_frames`]).
    full: Option<FullFrames>,
    /// Beam position: ticks since the last line start; lines since the last
    /// frame start.
    x: usize,
    line: usize,
    /// Ticks the HSYNC/VSYNC input has been high.
    hsync_run: usize,
    /// The HSYNC pulse running began where the line oscillator listens: it blanks the beam
    hsync_listened: bool,
    vsync_run: usize,
    /// The VSYNC pulse has already restarted the frame.
    vsync_seen: bool,
    /// The line oscillator follows the HSYNC pulses; counts the lines it ran
    /// free since.
    locked: bool,
    free_lines: usize,
    /// A VSYNC has been seen at least once: only then a frame is whole.
    synchronised: bool,
    /// The line oscillator went round (free) since the last accepted sync
    ran_free: bool,
    /// Lines left in which a small phase error is still followed (a correction is going on)
    pll_active: u32,
    /// ticks the pulse's end is ahead of the middle-referenced position (see `follow_sync`)
    sync_adj: i64,
    /// Ticks the beam still waits before the next line starts (a sync that came early)
    hold: usize,
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
    /// charge of the vertical sync integrator (ticks): the beam stays blanked while it is not empty
    vint: i64,
    hsync_color: Rgb<u8>,
    /// Level (0-255) the beam draws a black ink at: see [`Monitor::with_black_level`].
    black_level: u8
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
            full: None,
            x: 0,
            line: 0,
            hsync_run: 0,
            hsync_listened: true,
            vsync_run: 0,
            vsync_seen: false,
            locked: false,
            free_lines: 0,
            synchronised: false,
            ran_free: false,
            pll_active: 0,
            sync_adj: 0,
            hold: 0,
            now: 0,
            pulses: Vec::new(),
            last_sync_start: None,
            shift: 0,
            next_shift: 0,
            vsync_color,
            vint: 0,
            hsync_color,
            black_level: 0
        }
    }

    /// A monitor whose black ink is drawn at `level` (0-255, the other
    /// levels scaled between it and white) instead of 0. On a real tube the
    /// black of an ink is a dark grey, while the beam is cut during the
    /// horizontal and vertical blanking and the sync pulses: blacker than
    /// black. Raising the black level keeps them apart in the picture (what
    /// is not drawn, blanking and sync stay 0). The default is 0, which the
    /// tests and the debug API's pictures rely on.
    pub fn with_black_level(mut self, level: u8) -> Self {
        self.black_level = level;
        self
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

    /// Keep the whole frames as well: every tick of every line, sync and
    /// borders included, [`FULL_FRAME_WIDTH`] by [`FULL_FRAME_HEIGHT`] in the
    /// beam's own coordinates ([`Self::beam`]) - AMSpiriT's physical video
    /// buffer. For debuggers; costs a second picture drawn.
    pub fn keep_full_frames(&mut self) {
        if self.full.is_none() {
            self.full = Some(FullFrames {
                drawing: RgbImage::new(FULL_FRAME_WIDTH, FULL_FRAME_HEIGHT),
                done: None
            });
        }
    }

    /// The last whole frame drawn, if kept.
    pub fn full_frame(&self) -> Option<&RgbImage> {
        self.full.as_ref().and_then(|full| full.done.as_ref())
    }

    /// The whole frame being drawn, if kept: black where the beam has not
    /// been yet.
    pub fn full_frame_in_progress(&self) -> Option<&RgbImage> {
        self.full.as_ref().map(|full| &full.drawing)
    }

    /// The visible picture being drawn: black where the beam has not been
    /// yet.
    pub fn image_in_progress(&self) -> &RgbImage {
        &self.texture
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
            if self.hsync_run == 0 {
                self.hsync_listened = !self.locked || self.x >= MONITOR_HSYNC_WINDOW_START;
            }
            self.hsync_run += 1;
        }
        else {
            self.hsync_run = 0;
        }
        // the beam is blanked from the start of a VSYNC pulse for at least 25 lines and for 10 lines
        // after its end (AMSpiriT's black band on Shaker DT: pulses of 1, 8 and 16 lines)
        if vsync && self.vsync_run == 0 {
            self.vint = self.vint.max(MONITOR_VSYNC_BLANK_MIN_TICKS);
        }
        if !vsync && self.vsync_run > 0 {
            self.vint = self.vint.max(MONITOR_VSYNC_BLANK_AFTER_TICKS);
        }
        if self.vint > 0 && !vsync {
            self.vint -= 1;
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
            // like the line oscillator, the frame oscillator only listens to a VSYNC that comes late
            // enough in its period: AMSpiriT shows the first of the two VSYNCs of Shaker DT (156
            // lines apart) as a black band in the middle of a picture that goes on
            frame_start = !self.synchronised || self.line >= MONITOR_VSYNC_WINDOW_START_LINES;
        }

        // horizontal synchronisation: end of a pulse long enough, or the
        // oscillator running out
        let x_before = self.x;
        let was_locked = self.locked;
        let was_locked_now = was_locked;
        let pulse_end = hsync_pulse > MONITOR_MIN_HSYNC_TICKS
            && !hsync
            && (!self.locked || self.x >= MONITOR_HSYNC_WINDOW_START);
        let free_run = self.x >= RASTER_LINE_LENGTH;
        // a sync that comes after the oscillator went round only corrects its phase
        let late_pulse = hsync_pulse > MONITOR_MIN_HSYNC_TICKS
            && !hsync
            && !pulse_end
            && was_locked_now
            && self.ran_free
            && (self.x as i64) < MONITOR_PLL_CAPTURE_LATE;
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
        if late_pulse {
            if x_before > 2 {
                self.free_lines = 0;
            }
            if x_before > 2 || self.pll_active > 0 {
                self.follow_sync(x_before, was_locked);
            } else {
                self.ran_free = false;
            }
        }
        if line_end {
            self.x = 0;
            if pulse_end {
                // the line oscillator is referenced to the middle of the sync pulse (ACCC 14.4: the
                // picture moves half a microsecond when R3l drops by one; Shaker D3 on AMSpiriT)
                self.sync_adj = (hsync_pulse as i64 - 64).clamp(-32, 32) / 2;
                self.follow_sync(x_before, was_locked);
            } else {
                self.ran_free = true;
            }
            self.line += 1;
            if self.line >= MONITOR_FREE_RUN_LINES {
                // no VSYNC: the frame oscillator rolls the picture
                self.end_frame();
                self.synchronised = true;
            }
        }

        if self.hold > 0 {
            self.hold -= 1;
            return;
        }
        let y = self.line.wrapping_sub(MONITOR_FIRST_LINE);
        let px = (self.x as isize - MONITOR_FIRST_PIXEL as isize + self.shift as isize) as usize;
        let visible = px < MONITOR_ACTIVE_PIXELS && y < MONITOR_HEIGHT;
        let in_full = self.full.is_some() && self.x < FULL_FRAME_WIDTH as usize && self.line < FULL_FRAME_HEIGHT as usize;
        if visible || in_full {
            let pixel = if vsync || self.vint > 0 {
                self.vsync_color
            }
            else if hsync && self.hsync_listened {
                self.hsync_color
            }
            else {
                pixel_of(rgb, self.black_level)
            };
            if visible {
                self.texture.put_pixel(px as u32, y as u32, pixel);
            }
            if let (true, Some(full)) = (in_full, &mut self.full) {
                full.drawing.put_pixel(self.x as u32, self.line as u32, pixel);
            }
        }
        self.x += 1;
    }

    /// The line oscillator's phase detector: a sync that comes `error` ticks away from where the
    /// oscillator expected it (negative: early) pulls the start of the line by 3/8 of it, at most
    /// 12 ticks a line, so a moving HSYNC (the waving screens of What a Cool Weekend) shows a smooth
    /// curve; measured against AMSpiriT with validation/fuzz/gen_wave_probe.py. Far outside
    /// that range, or when the oscillator was not locked, the beam simply restarts.
    fn follow_sync(&mut self, x_before: usize, was_locked: bool) {
        let adj = std::mem::take(&mut self.sync_adj);
        let error = if self.ran_free { x_before as i64 } else { x_before as i64 - RASTER_LINE_LENGTH as i64 };
        let error = error - adj;
        self.ran_free = false;
        if !was_locked || error < -MONITOR_PLL_CAPTURE_EARLY || error > MONITOR_PLL_CAPTURE_LATE {
            return;
        }
        if error.abs() > 2 {
            self.pll_active = 12;
        } else if self.pll_active > 0 {
            self.pll_active -= 1;
        }
        let pull = if error.abs() <= 1 { error } else { ((error * 3 + 4 * error.signum()) / 8).clamp(-MONITOR_PLL_MAX_PULL, MONITOR_PLL_MAX_PULL) };
        let start = error - pull + adj; // ticks since the new line start, at this sync
        if start >= 0 {
            self.x = start as usize;
        } else {
            self.hold = (-start) as usize;
        }
    }

    /// A line sync starting at `start` has been accepted: look at the other
    /// pulse that came since the previous one (the discriminator answers to
    /// exactly one) and let the beam follow it.
    fn discriminate(&mut self, start: u64) {
        let mut target = 0i32;
        if let (Some(previous), [(other, width)]) = (self.last_sync_start, self.pulses.as_slice()) {
            let after = (*other - previous) as i64; // late for the previous sync
            let late_pull = (((*width - MONITOR_PULL_DEAD_WIDTH).max(0) * 5 + 6) / 12).max(MONITOR_PULL_MIN);
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
            }
            // a second sync BEFORE the line sync does not pull the picture: AMSpiriT shows
            // no shift for an extra pulse 12 to 28 us ahead (validation/fuzz/gen_pull_probe.py)
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
        if let Some(full) = &mut self.full {
            // The two buffers swap: the one drawn becomes the last frame,
            // the older one is blanked and drawn next.
            let done = full
                .done
                .get_or_insert_with(|| RgbImage::new(FULL_FRAME_WIDTH, FULL_FRAME_HEIGHT));
            std::mem::swap(&mut full.drawing, done);
            full.drawing.fill(0);
        }
        self.line = 0;
    }
}

/// The whole frame: a 64 usec line of 16MHz ticks, and the lines the frame
/// oscillator runs when no VSYNC comes.
pub const FULL_FRAME_WIDTH: u32 = RASTER_LINE_LENGTH as u32;
pub const FULL_FRAME_HEIGHT: u32 = MONITOR_FREE_RUN_LINES as u32;

/// The whole frames: the one being drawn and the last one.
#[derive(Debug, Clone)]
struct FullFrames {
    drawing: RgbImage,
    done: Option<RgbImage>
}

/// RGB intensities (percent) to 8-bit pixels, 0% being drawn at `black`.
fn pixel_of(rgb: RgbLevels, black: u8) -> Rgb<u8> {
    let range = 255.0 - black as f32;
    let to_u8 = |v: f32| -> u8 { (black as f32 + v * range / 100.0).min(255f32) as u8 };
    Rgb([to_u8(rgb.r()), to_u8(rgb.g()), to_u8(rgb.b())])
}

#[cfg(test)]
mod test {
    use super::*;

    /// A black ink is above the blanking and the sync pulses, which stay 0.
    #[test]
    fn a_black_level_lifts_ink_black_only() {
        assert_eq!(pixel_of((0., 0., 0.).into(), 8), Rgb([8, 8, 8]));
        assert_eq!(pixel_of((100., 100., 100.).into(), 8), Rgb([255, 255, 255]));
        let mut mon = Monitor::default().with_black_level(8);
        // lines of black ink, with an HSYNC pulse in each
        for _ in 0..(MONITOR_FIRST_LINE + 3) {
            feed(&mut mon, 736, (0., 0., 0.), false, false);
            feed(&mut mon, 64, (0., 0., 0.), false, true);
            feed(&mut mon, 224, (0., 0., 0.), false, false);
        }
        let picture = mon.image_in_progress();
        let inks = picture.pixels().filter(|p| p.0 == [8, 8, 8]).count();
        let blanked = picture.pixels().filter(|p| p.0 == [0, 0, 0]).count();
        assert!(inks > 0, "no black ink drawn");
        assert!(blanked > 0, "the unvisited part of the frame stays at 0");
    }

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
        assert_eq!(pixel_of((0., 0., 0.).into(), 0), Rgb([0, 0, 0]));
        assert_eq!(pixel_of((100., 100., 100.).into(), 0), Rgb([255, 255, 255]));
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

    /// A pulse the locked line oscillator does not listen to does not blank the beam
    /// (AMSpiriT, validation/fuzz/gen_pull2_probe.py: a narrow extra pulse before a wide sync).
    #[test]
    fn an_ignored_pulse_does_not_blank_the_beam() {
        let mut mon = Monitor::default();
        for l in 0..120 {
            for t in 0..RASTER_LINE_LENGTH {
                let main = t >= 736 && t < 800;
                let other = l >= 30 && t >= 400 && t < 464;
                let hsync = main || other;
                mon.receive((100., 0., 0.).into(), Sync { vsync: false, hsync });
            }
        }
        // the oscillator is locked: the pulse is not listened to, nothing goes black in the lines drawn
        let blacks = (0..MONITOR_ACTIVE_PIXELS as u32)
            .filter(|&x| mon.texture.get_pixel(x, 40).0 == [0, 0, 0])
            .count();
        assert_eq!(blacks, 0);
    }
}
