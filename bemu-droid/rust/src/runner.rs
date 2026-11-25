//! The emulation thread: it owns the machine, the GPU surface and the sound, and runs a frame
//! after the other while the app shows the surface. The UI thread talks to it by
//! [`Command`]s (and the touch input by [`Input`], read once a frame).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use amsrust::audio_mix::Pacing;
use ndk::native_window::NativeWindow;

use crate::audio::AudioOut;
use crate::config::Config;
use crate::gfx::Gfx;
use crate::governor::{Decision, Governor};
use crate::machine::{Input, Machine};
use crate::performance::Performance;

/// A frame of the CPC: 50.08 Hz.
const FRAME: Duration = Duration::from_micros(19_968);

pub enum Command {
    /// The surface to draw in (none: it is gone, drop everything drawn in it); answered once done.
    Surface(Option<NativeWindow>, Sender<()>),
    /// The surface's new size.
    Resize(u32, u32),
    /// The app in the background (or back).
    Pause(bool),
    /// A medium: the machine restarts with it. What is typed for it, or why it was refused.
    Load(String, Vec<u8>, Sender<Result<Option<String>, String>>),
    Reset,
    Configure(Config, Sender<Result<(), String>>),
    /// The user keeps the CRT, too slow as it is.
    KeepCrt,
    /// The machine saved (the app going to the background); answered once written.
    Save(Sender<Result<(), String>>),
    TypeText(String),
    Quit,
}

/// The emulation thread, its machine built there (a machine stays on its thread); why it could
/// not be.
pub fn spawn(config: Config, sd: PathBuf, state: PathBuf, input: Arc<Input>) -> Result<Sender<Command>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let (built, outcome) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("bemu".to_owned())
        // the FDC and cubecl like a big stack
        .stack_size(16 << 20)
        .spawn(move || {
            // the machine as the app left it, if Android ended the app meanwhile
            let restored = Machine::restore(config.clone(), sd.clone(), &state);
            if restored.is_some() {
                log::info!("the machine saved is back");
            }
            Machine::forget_state(&state);
            match restored.map(Ok).unwrap_or_else(|| Machine::new(config, sd)) {
            Ok(machine) => {
                let _ = built.send(Ok(()));
                Runner::new(machine, input, state).run(receiver)
            }
            Err(e) => {
                let _ = built.send(Err(e));
            }
        }})
        .map_err(|e| e.to_string())?;
    outcome.recv().map_err(|e| e.to_string())??;
    Ok(sender)
}

struct Runner {
    machine: Machine,
    /// What a phone too slow gives up.
    governor: Governor,
    /// Frames run, for the skipping (one in two drawn).
    frames: u64,
    /// Where the machine is saved.
    state: PathBuf,
    input: Arc<Input>,
    // the surface's drawing before the window it draws in
    gfx: Gfx,
    window: Option<NativeWindow>,
    size: (u32, u32),
    paused: bool,
    audio: Option<AudioOut>,
    next_frame: Instant,
    performance: Performance,
    last_frame_end: Option<Instant>,
}

impl Runner {
    fn new(machine: Machine, input: Arc<Input>, state: PathBuf) -> Self {
        let mut runner = Self {
            machine,
            governor: Governor::default(),
            frames: 0,
            state,
            input,
            gfx: Gfx::default(),
            window: None,
            size: (1, 1),
            paused: false,
            audio: None,
            next_frame: Instant::now(),
            performance: Performance::default(),
            last_frame_end: None,
        };
        runner.update_audio();
        runner
    }

    fn running(&self) -> bool {
        self.gfx.attached() && !self.paused
    }

    fn reset_performance(&mut self) {
        self.performance = Performance::default();
        self.last_frame_end = None;
        let status = if self.paused {
            "Emulation paused"
        } else if !self.gfx.attached() {
            "Graphics surface unavailable"
        } else {
            "Collecting measurements..."
        };
        match self.input.diagnostics.lock() {
            Ok(mut diagnostics) => *diagnostics = status.to_owned(),
            Err(error) => log::error!("cannot reset diagnostics: {error}"),
        }
    }

    /// Sound on when the settings and the app want it, the machine producing what the device plays.
    fn update_audio(&mut self) {
        let wanted = self.machine.config.audio && self.running();
        if wanted && self.audio.is_none() {
            match AudioOut::open() {
                Ok(audio) => self.audio = Some(audio),
                Err(e) => log::warn!("no sound: {e}"),
            }
        } else if !wanted {
            self.audio = None;
        }
        let config = self.audio.as_ref().map(AudioOut::config);
        self.machine.set_audio(config);
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            // stopped: wait for a command; running: take those waiting
            let command = if self.running() {
                commands.try_recv().ok()
            } else {
                match commands.recv() {
                    Ok(command) => Some(command),
                    Err(_) => return,
                }
            };
            if let Some(command) = command {
                if !self.handle(command) {
                    return;
                }
                continue;
            }
            self.frame();
        }
    }

    /// `false` to quit.
    fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Surface(window, done) => {
                self.gfx.detach();
                self.window = None;
                if let Some(window) = window {
                    self.size = (window.width().max(1) as u32, window.height().max(1) as u32);
                    // the window outlives the drawing: both kept here, the drawing detached first
                    match unsafe { self.gfx.attach(window.ptr().cast(), self.size) } {
                        Ok(()) => {
                            self.input.crt.store(self.gfx.has_crt(), std::sync::atomic::Ordering::Relaxed);
                            self.window = Some(window);
                        }
                        Err(e) => log::error!("cannot draw: {e}"),
                    }
                }
                self.next_frame = Instant::now();
                self.governor.reset();
                self.update_audio();
                self.reset_performance();
                let _ = done.send(());
            }
            Command::Resize(width, height) => {
                self.size = (width, height);
                self.gfx.resize(self.size);
                self.reset_performance();
            }
            Command::Pause(paused) => {
                self.paused = paused;
                self.governor.reset();
                self.next_frame = Instant::now();
                self.update_audio();
                self.reset_performance();
            }
            Command::Load(name, bytes, reply) => {
                let result = self.machine.load(&name, bytes);
                if let Err(e) = &result {
                    log::warn!("{e}");
                }
                self.reset_performance();
                let _ = reply.send(result);
            }
            Command::Reset => {
                if let Err(e) = self.machine.reset() {
                    log::error!("reset: {e}");
                }
                self.reset_performance();
            }
            Command::Configure(config, reply) => {
                let result = self.machine.configure(config);
                if result.is_ok() {
                    self.update_audio();
                    self.next_frame = Instant::now();
                    self.governor.reset();
                    self.reset_performance();
                }
                let _ = reply.send(result);
            }
            Command::KeepCrt => {
                self.governor.crt_droppable = false;
                self.machine.config.display = "ctm".to_owned();
                self.governor.reset();
                self.reset_performance();
            }
            Command::Save(reply) => {
                let result = self.machine.save_state(&self.state);
                if let Err(e) = &result {
                    log::warn!("the machine could not be saved: {e}");
                }
                let _ = reply.send(result);
            }
            Command::TypeText(text) => self.machine.type_text(&text),
            Command::Quit => return false,
        }
        true
    }

    /// One frame run, drawn, its sound queued; then the wait the sound (or the clock) asks.
    fn frame(&mut self) {
        let start = Instant::now();
        let picture = self.machine.run_frame(&self.input);
        let emulation = start.elapsed();
        let samples = self.machine.take_audio();
        if let Some(audio) = &self.audio {
            audio.push(&samples, Pacing::DESKTOP.max);
        }
        let (mode, warp) = self.machine.display();
        self.frames += 1;
        // a phone too slow draws one frame in two (the machine and its sound run them all)
        let attempted_draw = !(self.governor.skipping() && self.frames % 2 == 1);
        let draw_start = Instant::now();
        if attempted_draw {
            if let Err(e) = self.gfx.show(picture.as_ref(), mode, warp) {
                log::error!("{e}");
            }
        }
        let drawing = if attempted_draw { draw_start.elapsed() } else { Duration::ZERO };
        let crt = !mode.is_raw() && self.gfx.has_crt();
        let changed = match self.governor.frame(start.elapsed(), crt) {
            Decision::Keep => false,
            Decision::DropCrt => {
                log::warn!("{:?} a frame with the CRT: the picture raw", self.governor.average());
                self.machine.config.display = "raw".to_owned();
                self.input.crt_dropped.store(true, std::sync::atomic::Ordering::Relaxed);
                true
            }
            Decision::SkipFrames(skip) => {
                log::warn!("one frame drawn in two: {skip}");
                true
            }
        };
        let pacing_start = Instant::now();
        match &self.audio {
            // the device's clock: wait while enough sound is ahead
            Some(audio) => {
                let start = Instant::now();
                while Pacing::DESKTOP.should_wait(audio.buffered()) && start.elapsed() < FRAME * 3 {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            // the CPC's 50 Hz
            None => {
                self.next_frame += FRAME;
                let now = Instant::now();
                if self.next_frame > now {
                    std::thread::sleep(self.next_frame - now);
                } else if now - self.next_frame > FRAME * 5 {
                    // too far behind (a slow phone): no catching up
                    self.next_frame = now;
                }
            }
        }
        let pacing = pacing_start.elapsed();
        if changed {
            self.reset_performance();
            return;
        }
        let end = Instant::now();
        let elapsed = end.duration_since(self.last_frame_end.unwrap_or(start));
        self.last_frame_end = Some(end);
        if let Some(performance) = self.performance.record(elapsed, emulation, drawing, pacing, attempted_draw) {
            let sound = match &self.audio {
                Some(audio) => format!("on (buffer {:.1} ms)", audio.buffered().as_secs_f64() * 1000.0),
                None if self.machine.config.audio => "requested, unavailable".to_owned(),
                None => "off".to_owned(),
            };
            let report = format!(
                "{}\nDisplay requested: {} | frame skipping: {}\nSound: {}\n{}\nDraw submission is CPU time, not GPU execution time.",
                self.gfx.diagnostics(), self.machine.config.display, self.governor.skipping(), sound, performance
            );
            match self.input.diagnostics.lock() {
                Ok(mut diagnostics) => *diagnostics = report,
                Err(error) => log::error!("cannot update diagnostics: {error}"),
            }
        }
    }
}
