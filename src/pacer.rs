//! The pacing of the emulation loop of the web page: which frames to run at each refresh of the
//! display and each report of the sound, which pictures to make, how much sound to keep queued. It
//! was the heart of `web/js/engine.js`; here it is plain Rust with no browser in it (the machine,
//! the clock and the sound sit behind [`Env`]), so `cargo test` plays it with a fake clock and its
//! constants are the library's (`FRAME_T_STATES`, `audio_mix::Pacing`).

use crate::audio_mix::Pacing;
use crate::frame::FRAME_T_STATES;

/// The CPC's frame in milliseconds: 312 lines of 64 µs (a Z80 does 4000 T-states a millisecond).
pub const FRAME_MS: f64 = FRAME_T_STATES as f64 / 4000.0;
/// After an underrun the sound's target grows by this factor, up to the maximum: a slow or busy
/// machine trades latency for continuity.
const AUDIO_TARGET_GROWTH: f64 = 1.5;
const AUDIO_TARGET_MAX_S: f64 = 0.2;
/// At more than real time, the emulation takes this share of each animation frame (the rest is the
/// page's: drawing, input), between these many milliseconds: on a computer too slow to keep up,
/// the refreshes get longer and so does the emulation's share.
const BUSY_SHARE: f64 = 0.9;
const BUSY_MIN_MS: f64 = 40.0;
const BUSY_MAX_MS: f64 = 100.0;
/// Limit catch-up work at normal speed so a slow machine can still service input and sound.
const REALTIME_BUDGET_MS: f64 = FRAME_MS;
/// Frame skip 'auto': behind, the sound has less than this share of its target queued. A machine
/// too slow for real time then stops making the picture of some frames, so the program keeps its
/// 50 Hz: at most [`MAX_HIDDEN_FRAMES`] in a row, a picture at least every third frame.
const BEHIND_SHARE: f64 = 0.5;
const MAX_HIDDEN_FRAMES: u32 = 2;

/// What the pacer runs on: the page's clock, the machine and the sound.
pub trait Env {
    /// The clock, in milliseconds (`performance.now()`).
    fn now(&mut self) -> f64;
    /// Run the machine one frame; `unseen`: without making its picture (about a quarter faster).
    fn run_frame(&mut self, unseen: bool) -> Result<(), String>;
    /// The sound of the frames run since the last call (stereo, interleaved).
    fn take_audio(&mut self) -> Vec<f32>;
    /// Show the last frame.
    fn draw(&mut self) -> Result<(), String>;
}

/// The sound feed: what the worklet last reported, and what was sent since.
struct Audio {
    /// Sample frames a second.
    rate: f64,
    /// Seconds of sound to keep ahead of the speakers.
    target: f64,
    fill: f64,
    arrived_at: f64,
    playing: bool,
    underruns: f64,
    /// Sample frames sent / acknowledged by the worklet, so that those in transit count.
    sent: f64,
    received: f64,
}

/// What was done in the last second, for the page's 'stats' event.
#[derive(Default, Clone, Copy)]
struct Counters {
    since: Option<f64>,
    frames: u32,
    frame_ms: f64,
    max_frame_ms: f64,
    draws: u32,
    draw_ms: f64,
    replaced: u32,
    refreshes: u32,
    hidden: u32,
}

/// The 'stats' event: once a second.
#[derive(Debug, Clone, PartialEq)]
pub struct StatsReport {
    pub fps: f64,
    /// Times real time.
    pub speed: f64,
    pub frame_ms: f64,
    pub max_frame_ms: f64,
    pub draw_ms: f64,
    pub draws: u32,
    pub draw_fps: f64,
    pub refresh_fps: f64,
    pub replaced: u32,
    pub hidden: f64,
    /// (underruns, buffered ms, target ms, playing), with a sound feed.
    pub audio: Option<(f64, f64, f64, bool)>,
}

pub struct Pacer {
    running: bool,
    /// The frame or its display settings changed since the last draw.
    fresh: bool,
    /// A generated picture is waiting for a draw, independently of display-setting changes.
    frame_pending: bool,
    last_time: Option<f64>,
    owed: f64,
    intervals: Vec<f64>,
    /// 1, 2, 3 or 4 refreshes of a display at 50, 100, 150 or 200 Hz per CPC frame; 0 at another rate.
    refreshes_per_frame: u32,
    refreshes: u32,
    audio: Option<Audio>,
    /// Times real time: 1, more, or infinity (as fast as possible).
    speed: f64,
    counters: Counters,
    hidden_run: u32,
    frame_skip: bool,
    /// The sound made since the page last took it, to send to the worklet.
    outgoing: Vec<f32>,
}

impl Default for Pacer {
    fn default() -> Self {
        Self::new()
    }
}

impl Pacer {
    pub fn new() -> Self {
        Self {
            running: false,
            fresh: false,
            frame_pending: false,
            last_time: None,
            owed: 0.0,
            intervals: Vec::new(),
            refreshes_per_frame: 0,
            refreshes: 0,
            audio: None,
            speed: 1.0,
            counters: Counters::default(),
            hidden_run: 0,
            frame_skip: false,
            outgoing: Vec::new(),
        }
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn refreshes_per_frame(&self) -> u32 {
        self.refreshes_per_frame
    }

    /// Whether a machine behind may skip the picture of some frames ('auto'; off by default).
    pub fn set_frame_skip(&mut self, on: bool) {
        self.frame_skip = on;
    }

    pub fn start(&mut self) {
        if self.running {
            return;
        }
        self.running = true;
        self.last_time = None;
        self.owed = 0.0;
        self.counters.since = None;
    }

    pub fn pause(&mut self) {
        self.running = false;
    }

    /// The frame or the display settings changed: draw at the next chance (now, while paused).
    pub fn redraw_if_paused<E: Env>(&mut self, env: &mut E) -> Result<(), String> {
        self.fresh = true;
        if !self.running {
            self.draw(env)?;
        }
        Ok(())
    }

    /// While paused: one more frame, shown.
    pub fn step_frame<E: Env>(&mut self, env: &mut E) -> Result<(), String> {
        if self.running {
            return Ok(());
        }
        env.run_frame(false)?;
        env.take_audio();
        self.fresh = true;
        self.frame_pending = true;
        self.draw(env)
    }

    pub fn set_speed<E: Env>(&mut self, env: &mut E, value: f64) {
        if value != 1.0 {
            env.take_audio(); // what is queued is dropped, not played late
        }
        self.speed = value;
        self.owed = 0.0;
    }

    /// The sound goes to a worklet playing at `rate`.
    pub fn attach_audio<E: Env>(&mut self, env: &mut E, rate: f64) {
        self.audio = Some(Audio {
            rate,
            target: Pacing::WEB.target.as_secs_f64(),
            fill: 0.0,
            arrived_at: env.now(),
            playing: false,
            underruns: 0.0,
            sent: 0.0,
            received: 0.0,
        });
        self.outgoing.clear();
    }

    pub fn detach_audio(&mut self) {
        self.audio = None;
        self.outgoing.clear();
    }

    /// The sound made since the last call, for the worklet.
    pub fn take_outgoing(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.outgoing)
    }

    /// Seconds of sound queued and not yet played, from the worklet's last report.
    pub fn audio_buffered(&self, now: f64) -> f64 {
        let Some(audio) = &self.audio else { return 0.0 };
        let played = if audio.playing { (now - audio.arrived_at).max(0.0) / 1000.0 } else { 0.0 };
        ((audio.fill + audio.sent - audio.received) / audio.rate - played).max(0.0)
    }

    /// The worklet's report of its queue. Returns the new sound target (seconds) when an underrun
    /// made it grow, for the worklet. Frames the speakers will miss are emulated at once.
    pub fn audio_report<E: Env>(
        &mut self,
        env: &mut E,
        fill: f64,
        playing: bool,
        received: f64,
        underruns: f64,
    ) -> Result<Option<f64>, String> {
        if !fill.is_finite() || !received.is_finite() || !underruns.is_finite() {
            return Err("Invalid audio worklet report. Reload the page to update cached emulator files.".into());
        }
        let now = env.now();
        let Some(audio) = &mut self.audio else { return Ok(None) };
        audio.fill = fill;
        audio.arrived_at = now;
        audio.playing = playing;
        audio.received = received;
        let mut new_target = None;
        if underruns > audio.underruns {
            audio.underruns = underruns;
            audio.target = (audio.target * AUDIO_TARGET_GROWTH).min(AUDIO_TARGET_MAX_S);
            new_target = Some(audio.target);
        }
        self.refill(env)?;
        Ok(new_target)
    }

    /// On the sound's clock (its reports come every ~20 ms whatever the display does): emulate the
    /// frames the speakers will miss, including when display refreshes are delayed.
    fn refill<E: Env>(&mut self, env: &mut E) -> Result<(), String> {
        let (target, now) = (self.audio.as_ref().map(|a| a.target), env.now());
        let Some(target) = target else { return Ok(()) };
        if !self.running || self.speed != 1.0 {
            return Ok(());
        }
        let frames = ((target - self.audio_buffered(now)) * 1000.0 / FRAME_MS).ceil().clamp(0.0, 4.0) as u32;
        let start = env.now();
        let mut i = 0;
        while i < frames && (i == 0 || env.now() - start < REALTIME_BUDGET_MS) {
            let unseen = self.hide_when_behind(env);
            self.run_frame(env, unseen)?;
            i += 1;
        }
        Ok(())
    }

    /// Whether the next frame is made without a picture, because the machine cannot keep real time
    /// (frame skip 'auto' only): the sound has less than half its target queued (without sound:
    /// more than a frame owed). A frame is shown at least every `MAX_HIDDEN_FRAMES + 1`.
    fn hide_when_behind<E: Env>(&mut self, env: &mut E) -> bool {
        let behind = self.frame_skip
            && self.speed == 1.0
            && match &self.audio {
                Some(audio) => self.audio_buffered(env.now()) < audio.target * BEHIND_SHARE,
                None => self.owed >= 2.0 * FRAME_MS,
            };
        if behind && self.hidden_run < MAX_HIDDEN_FRAMES {
            self.hidden_run += 1;
            return true;
        }
        self.hidden_run = 0;
        false
    }

    /// A frame emulated, not drawn; its sound queued at real time, dropped otherwise. `unseen`: no
    /// picture is made.
    fn run_frame<E: Env>(&mut self, env: &mut E, unseen: bool) -> Result<(), String> {
        let start = env.now();
        env.run_frame(unseen)?;
        let frame_ms = env.now() - start;
        self.counters.frame_ms += frame_ms;
        self.counters.max_frame_ms = self.counters.max_frame_ms.max(frame_ms);
        if !unseen {
            if self.frame_pending {
                self.counters.replaced += 1;
            }
            self.frame_pending = true;
            self.fresh = true;
        }
        self.counters.frames += 1;
        if unseen && self.speed == 1.0 {
            self.counters.hidden += 1;
        }
        if self.audio.is_none() {
            return Ok(());
        }
        let samples = env.take_audio();
        if samples.is_empty() || self.speed != 1.0 {
            return Ok(());
        }
        if self.audio_buffered(env.now()) > Pacing::WEB.max.as_secs_f64() {
            return Ok(()); // too far ahead: drop
        }
        if let Some(audio) = &mut self.audio {
            audio.sent += samples.len() as f64 / 2.0;
        }
        self.outgoing.extend_from_slice(&samples);
        Ok(())
    }

    fn draw<E: Env>(&mut self, env: &mut E) -> Result<(), String> {
        if !self.fresh {
            return Ok(());
        }
        let start = env.now();
        env.draw()?;
        self.counters.draw_ms += env.now() - start;
        self.counters.draws += 1;
        self.fresh = false;
        self.frame_pending = false;
        Ok(())
    }

    /// One refresh of the display, at `time` (milliseconds). The 'stats' event it makes, if any.
    pub fn tick<E: Env>(&mut self, env: &mut E, time: f64) -> Result<Option<StatsReport>, String> {
        if !self.running {
            return Ok(None);
        }
        let elapsed = self.last_time.map_or(FRAME_MS, |last| time - last);
        self.last_time = Some(time);
        self.measure_refresh(elapsed);
        let report = self.stats_report(env, time);
        self.counters.refreshes += 1;
        self.refresh(env, elapsed)?;
        Ok(report)
    }

    fn refresh<E: Env>(&mut self, env: &mut E, elapsed: f64) -> Result<(), String> {
        // Audio reports may have produced a picture since the last refresh. Present it before
        // doing more emulation: replacing it here loses a whole display opportunity.
        if self.speed == 1.0 && self.audio.is_some() && self.frame_pending {
            return self.draw(env);
        }
        let budget = (elapsed * BUSY_SHARE).max(BUSY_MIN_MS).min(BUSY_MAX_MS);
        if self.speed == f64::INFINITY {
            // full speed: frames without picture for most of the refresh, then one shown
            let start = env.now();
            while env.now() - start < budget {
                self.run_frame(env, true)?;
            }
            self.run_frame(env, false)?;
            return self.draw(env);
        }
        if self.speed != 1.0 {
            // n times real time, by the browser's clock
            self.owed = (self.owed + elapsed * self.speed).min(4.0 * self.speed * FRAME_MS);
            let start = env.now();
            while self.owed >= 2.0 * FRAME_MS && env.now() - start < budget {
                self.run_frame(env, true)?;
                self.owed -= FRAME_MS;
            }
            if self.owed >= FRAME_MS {
                self.run_frame(env, false)?;
                self.owed -= FRAME_MS;
            }
            return self.draw(env);
        }
        if self.refreshes_per_frame > 0 {
            // one CPC frame every refresh of a 50 Hz display, every other one at 100 Hz...: each
            // frame on the screen as long as the others (the last one stays shown)
            self.refreshes += 1;
            if self.refreshes >= self.refreshes_per_frame {
                self.refreshes = 0;
                let behind_target = match &self.audio {
                    None => true,
                    Some(audio) => self.audio_buffered(env.now()) < audio.target,
                };
                if behind_target {
                    let unseen = self.hide_when_behind(env);
                    self.run_frame(env, unseen)?;
                }
                self.draw(env)?;
            }
            return Ok(());
        }
        let frames = match &self.audio {
            // the sound's clock: keep a little of it ahead of the speakers
            Some(audio) => ((audio.target - self.audio_buffered(env.now())) * 1000.0 / FRAME_MS).ceil(),
            None => {
                self.owed = (self.owed + elapsed).min(4.0 * FRAME_MS);
                (self.owed / FRAME_MS).floor()
            }
        }
        .clamp(0.0, 4.0) as u32;
        let start = env.now();
        let mut completed = 0;
        while completed < frames && (completed == 0 || env.now() - start < REALTIME_BUDGET_MS) {
            let unseen = self.hide_when_behind(env);
            self.run_frame(env, unseen)?;
            completed += 1;
        }
        if self.audio.is_none() {
            self.owed -= completed as f64 * FRAME_MS;
        }
        self.draw(env)
    }

    /// The 'stats' event, once a second.
    fn stats_report<E: Env>(&mut self, env: &mut E, time: f64) -> Option<StatsReport> {
        let Some(since) = self.counters.since else {
            self.counters = Counters { since: Some(time), ..Counters::default() };
            return None;
        };
        if time - since < 1000.0 {
            return None;
        }
        let c = self.counters;
        let span = time - since;
        let fps = c.frames as f64 * 1000.0 / span;
        let report = StatsReport {
            fps,
            speed: fps * FRAME_MS / 1000.0,
            frame_ms: if c.frames > 0 { c.frame_ms / c.frames as f64 } else { 0.0 },
            max_frame_ms: c.max_frame_ms,
            draw_ms: if c.draws > 0 { c.draw_ms / c.draws as f64 } else { 0.0 },
            draws: c.draws,
            draw_fps: c.draws as f64 * 1000.0 / span,
            refresh_fps: c.refreshes as f64 * 1000.0 / span,
            replaced: c.replaced,
            hidden: c.hidden as f64 * 1000.0 / span,
            audio: self.audio.as_ref().map(|a| {
                (a.underruns, self.audio_buffered(env.now()) * 1000.0, a.target * 1000.0, a.playing)
            }),
        };
        self.counters = Counters { since: Some(time), ..Counters::default() };
        Some(report)
    }

    /// Whether the display refreshes at about 50 Hz, from the last 60 animation frames.
    fn measure_refresh(&mut self, elapsed: f64) {
        self.intervals.push(elapsed);
        if self.intervals.len() > 60 {
            self.intervals.remove(0);
        }
        if self.intervals.len() < 30 {
            return;
        }
        let mut sorted = self.intervals.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let hertz = 1000.0 / sorted[sorted.len() >> 1];
        self.refreshes_per_frame = (1..=4u32)
            .find(|&k| (hertz - 50.0 * k as f64).abs() < 1.5 * k as f64)
            .unwrap_or(0);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// A machine whose frames cost `cost` ms of the fake clock and make `audio` samples.
    struct Fake {
        now: f64,
        cost: f64,
        audio: usize,
        frames: u32,
        unseen: u32,
        draws: u32,
        pending_audio: usize,
    }

    impl Fake {
        fn new(cost: f64) -> Self {
            Fake { now: 0.0, cost, audio: 0, frames: 0, unseen: 0, draws: 0, pending_audio: 0 }
        }
    }

    impl Env for Fake {
        fn now(&mut self) -> f64 {
            self.now
        }
        fn run_frame(&mut self, unseen: bool) -> Result<(), String> {
            self.now += self.cost;
            self.frames += 1;
            self.unseen += unseen as u32;
            self.pending_audio += self.audio;
            Ok(())
        }
        fn take_audio(&mut self) -> Vec<f32> {
            vec![0.0; std::mem::take(&mut self.pending_audio)]
        }
        fn draw(&mut self) -> Result<(), String> {
            self.draws += 1;
            Ok(())
        }
    }

    fn tick(p: &mut Pacer, env: &mut Fake, time: f64) -> Option<StatsReport> {
        env.now = env.now.max(time);
        p.tick(env, time).unwrap()
    }

    #[test]
    fn the_frame_is_a_fiftieth_of_a_second() {
        assert!((FRAME_MS - 19.968).abs() < 1e-9);
    }

    #[test]
    fn normal_speed_catch_up_yields_after_a_slow_frame_and_retains_the_debt() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(30.0));
        p.start();
        tick(&mut p, &mut env, 0.0);
        assert_eq!(env.frames, 1);
        tick(&mut p, &mut env, 100.0);
        assert_eq!(env.frames, 2, "a slow refresh must not execute four frames");
        tick(&mut p, &mut env, 101.0);
        assert_eq!(env.frames, 3, "unexecuted frames remain owed");
    }

    #[test]
    fn fast_catch_up_runs_several_frames_and_does_not_redraw_an_unchanged_frame() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(1.0));
        p.start();
        tick(&mut p, &mut env, 0.0);
        tick(&mut p, &mut env, 1.0);
        assert_eq!(env.draws, 1);
        tick(&mut p, &mut env, 100.0);
        assert_eq!(env.frames, 5);
        assert_eq!(env.draws, 2);
    }

    /// A worklet that has played nothing yet.
    fn silent_report(p: &mut Pacer, env: &mut Fake, received: f64, fill: f64, playing: bool) -> Option<f64> {
        p.audio_report(env, fill, playing, received, 0.0).unwrap()
    }

    #[test]
    fn a_stale_worklet_report_does_not_generate_frames_already_in_transit_again() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(0.0));
        env.audio = 960 * 2;
        p.attach_audio(&mut env, 48_000.0);
        p.start();
        tick(&mut p, &mut env, 0.0);
        assert_eq!(env.frames, 4);
        silent_report(&mut p, &mut env, 0.0, 0.0, false);
        assert_eq!(env.frames, 4, "audio already sent but not reported must remain accounted for");
        silent_report(&mut p, &mut env, 3840.0, 1000.0, true);
        assert_eq!(env.frames, 6, "a report that acknowledges drained samples triggers a refill");
        assert_eq!(p.take_outgoing().len(), 6 * 960 * 2, "all the sound made goes to the worklet");
    }

    #[test]
    fn audio_reports_refill_even_when_a_50_hz_display_misses_a_refresh() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(0.0));
        env.audio = 960 * 2;
        p.attach_audio(&mut env, 48_000.0);
        p.start();
        for i in 0..35 {
            tick(&mut p, &mut env, i as f64 * 20.0);
        }
        assert_eq!(p.refreshes_per_frame(), 1);
        let before = env.frames;
        silent_report(&mut p, &mut env, env.frames as f64 * 960.0, 0.0, true);
        assert!(env.frames > before, "audio must refill without waiting for another refresh");
    }

    #[test]
    fn an_underrun_grows_the_target_up_to_its_maximum_and_a_bad_report_is_an_error() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(0.0));
        p.attach_audio(&mut env, 48_000.0);
        let first = p.audio_report(&mut env, 0.0, true, 0.0, 1.0).unwrap().unwrap();
        assert!((first - 0.09).abs() < 1e-9);
        let second = p.audio_report(&mut env, 0.0, true, 0.0, 2.0).unwrap().unwrap();
        assert!((second - 0.135).abs() < 1e-9);
        let third = p.audio_report(&mut env, 0.0, true, 0.0, 3.0).unwrap().unwrap();
        assert!((third - 0.2).abs() < 1e-9, "capped at 0.2 s");
        assert_eq!(p.audio_report(&mut env, 0.0, true, 0.0, 3.0).unwrap(), None, "no new underrun");
        assert!(p.audio_report(&mut env, f64::NAN, true, 0.0, 0.0).is_err());
    }

    #[test]
    fn behind_the_sound_some_frames_are_made_without_a_picture_but_one_is_shown_every_third() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(0.0));
        env.audio = 960 * 2;
        p.set_frame_skip(true);
        p.attach_audio(&mut env, 48_000.0);
        p.start();
        // the worklet reports an empty queue, again and again: always behind
        for i in 0..30 {
            tick(&mut p, &mut env, i as f64 * 20.0);
            silent_report(&mut p, &mut env, 0.0, 0.0, true);
            p.take_outgoing();
        }
        assert!(env.unseen > 0, "some frames without a picture");
        assert!(env.unseen * 3 <= env.frames * 2 + 2, "at most two in a row: {} of {}", env.unseen, env.frames);
    }

    #[test]
    fn every_picture_is_made_by_default_however_far_behind_the_sound() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(0.0));
        env.audio = 960 * 2;
        p.attach_audio(&mut env, 48_000.0);
        p.start();
        for i in 0..30 {
            tick(&mut p, &mut env, i as f64 * 20.0);
            silent_report(&mut p, &mut env, 0.0, 0.0, true);
        }
        assert_eq!(env.unseen, 0);
    }

    #[test]
    fn frames_are_not_hidden_above_real_time() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(1.0));
        p.set_frame_skip(true);
        p.start();
        p.set_speed(&mut env, 2.0);
        tick(&mut p, &mut env, 0.0);
        tick(&mut p, &mut env, 100.0);
        // above real time the frames of a refresh but the last are hidden by design (speed > 1),
        // never counted as skipped frames
        let skipped = p.counters.hidden;
        assert_eq!(skipped, 0, "hidden frames are counted at real time only");
    }

    #[test]
    fn full_speed_makes_frames_without_a_picture_and_shows_the_last() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(1.0));
        p.start();
        p.set_speed(&mut env, f64::INFINITY);
        tick(&mut p, &mut env, 0.0);
        assert!(env.unseen > 10, "a refresh of unseen frames: {}", env.unseen);
        assert_eq!(env.draws, 1);
    }

    #[test]
    fn the_refresh_rate_is_snapped_to_50_100_150_or_200_hz() {
        for (ms, k) in [(20.0, 1), (10.0, 2), (1000.0 / 150.0, 3), (5.0, 4), (16.7, 0)] {
            let (mut p, mut env) = (Pacer::new(), Fake::new(0.0));
            p.start();
            for i in 0..40 {
                tick(&mut p, &mut env, i as f64 * ms);
            }
            assert_eq!(p.refreshes_per_frame(), k, "{ms} ms between refreshes");
        }
    }

    #[test]
    fn the_stats_come_once_a_second() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(1.0));
        p.start();
        let mut reports = Vec::new();
        for i in 0..120 {
            if let Some(r) = tick(&mut p, &mut env, i as f64 * 20.0) {
                reports.push(r);
            }
        }
        assert_eq!(reports.len(), 2, "after 2.4 s of refreshes: one a second");
        let r = &reports[0];
        assert!((r.fps - 50.0).abs() < 2.0, "{}", r.fps);
        assert!((r.speed - 1.0).abs() < 0.05, "{}", r.speed);
        assert!((r.frame_ms - 1.0).abs() < 1e-9);
        assert!(r.audio.is_none());
    }

    #[test]
    fn a_paused_pacer_steps_one_frame_and_shows_it() {
        let (mut p, mut env) = (Pacer::new(), Fake::new(0.0));
        p.step_frame(&mut env).unwrap();
        assert_eq!((env.frames, env.draws), (1, 1));
        p.start();
        p.step_frame(&mut env).unwrap();
        assert_eq!(env.frames, 1, "not while running");
    }
}
