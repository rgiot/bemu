//! What a phone too slow for the emulation gives up, as the time a frame takes says: first the
//! CRT (the CTM644's picture, the most demanding), then drawing every frame (one in two drawn, the
//! emulation and the sound at full speed). Plain Rust, tested on the host; the runner feeds it.

use std::collections::VecDeque;
use std::time::Duration;

/// A CPC frame: 50.08 Hz.
const FRAME: Duration = Duration::from_micros(19_968);

/// The frames averaged: 2 s.
const WINDOW: usize = 100;

/// How long the phone must stay too slow (or fast enough again) before a change: 3 s.
const SUSTAINED: usize = 150;

/// Below this share of a frame, a phone skipping frames draws them all again.
const FAST_ENOUGH: f64 = 0.7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Nothing changes.
    Keep,
    /// The CRT is too slow: the raw picture instead.
    DropCrt,
    /// Draw one frame in two (`true`), or all of them again (`false`).
    SkipFrames(bool),
}

pub struct Governor {
    /// The time the last frames took (emulation and drawing, not the waits).
    busy: VecDeque<Duration>,
    /// Frames in a row too slow (or fast enough, while skipping).
    streak: usize,
    /// The CRT may be dropped (not once the user kept it, nor after it was).
    pub crt_droppable: bool,
    skipping: bool,
}

impl Default for Governor {
    fn default() -> Self {
        Self {
            busy: VecDeque::with_capacity(WINDOW),
            streak: 0,
            crt_droppable: true,
            skipping: false,
        }
    }
}

impl Governor {
    /// Whether frames are skipped (one in two drawn).
    pub fn skipping(&self) -> bool {
        self.skipping
    }

    /// The average time of the last frames.
    pub fn average(&self) -> Duration {
        if self.busy.is_empty() {
            return Duration::ZERO;
        }
        self.busy.iter().sum::<Duration>() / self.busy.len() as u32
    }

    /// Start measuring again (the machine or the display changed, the app came back).
    pub fn reset(&mut self) {
        self.busy.clear();
        self.streak = 0;
    }

    /// A frame took `busy`, `crt` drawn: what to give up (or take back).
    pub fn frame(&mut self, busy: Duration, crt: bool) -> Decision {
        if self.busy.len() == WINDOW {
            self.busy.pop_front();
        }
        self.busy.push_back(busy);
        if self.busy.len() < WINDOW {
            return Decision::Keep;
        }
        let average = self.average();
        let slow = average > FRAME;
        let fast = average.as_secs_f64() < FRAME.as_secs_f64() * FAST_ENOUGH;
        let wanted = if self.skipping { fast } else { slow };
        self.streak = if wanted { self.streak + 1 } else { 0 };
        if self.streak < SUSTAINED {
            return Decision::Keep;
        }
        self.streak = 0;
        self.busy.clear();
        if self.skipping {
            self.skipping = false;
            Decision::SkipFrames(false)
        } else if crt && self.crt_droppable {
            self.crt_droppable = false;
            Decision::DropCrt
        } else {
            self.skipping = true;
            Decision::SkipFrames(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(governor: &mut Governor, ms: u64, frames: usize, crt: bool) -> Vec<Decision> {
        (0..frames)
            .map(|_| governor.frame(Duration::from_millis(ms), crt))
            .filter(|d| *d != Decision::Keep)
            .collect()
    }

    #[test]
    fn a_fast_phone_keeps_everything() {
        let mut governor = Governor::default();
        assert!(run(&mut governor, 12, 2000, true).is_empty());
    }

    #[test]
    fn a_slow_phone_drops_the_crt_then_skips_frames() {
        let mut governor = Governor::default();
        // a moment of slowness (2 s) changes nothing
        assert!(run(&mut governor, 30, 100, true).is_empty());
        assert!(run(&mut governor, 10, 200, true).is_empty());
        // too slow for long: the CRT goes, once
        assert_eq!(run(&mut governor, 30, 400, true), [Decision::DropCrt]);
        // still too slow with the raw picture: one frame drawn in two
        assert_eq!(run(&mut governor, 25, 400, false), [Decision::SkipFrames(true)]);
        assert!(governor.skipping());
        // fast enough again: all frames drawn
        assert_eq!(run(&mut governor, 8, 400, false), [Decision::SkipFrames(false)]);
    }

    #[test]
    fn a_crt_kept_by_the_user_is_not_dropped() {
        let mut governor = Governor::default();
        governor.crt_droppable = false;
        assert_eq!(run(&mut governor, 30, 400, true), [Decision::SkipFrames(true)]);
    }
}
