//! The machine of the app, without the phone: built from the settings, given media (started by
//! themselves), the touch keyboard and joystick applied, run a frame at a time. Plain Rust, tested
//! on the host; `runner.rs` drives it on Android.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use amsrust::autorun::{self, MediaKind};
use amsrust::display::{DisplayMode, Warp};
use amsrust::fdc::FdcDrive;
use amsrust::frame::FRAME_T_STATES;
use amsrust::joystick::JoystickState;
use amsrust::keyboard::{key_at, NB_LINES};
use amsrust::monitor::Monitor;
use amsrust::motherboard::{CpcModel, Motherboard};
use amsrust::psg::AudioConfig;
use amsrust::typist::Typist;
use image::RgbImage;

use crate::config::Config;

/// The joystick's bits (see [`Input::joystick`]).
pub const JOY_UP: u8 = 1;
pub const JOY_DOWN: u8 = 2;
pub const JOY_LEFT: u8 = 4;
pub const JOY_RIGHT: u8 = 8;
pub const JOY_FIRE1: u8 = 16;
pub const JOY_FIRE2: u8 = 32;

/// What the touch screen holds, written by the UI thread, read once a frame.
#[derive(Default)]
pub struct Input {
    /// The keys held, a bit per key of each matrix line (1: held), by the firmware's key number
    /// (line * 8 + bit).
    keys: [AtomicU8; NB_LINES],
    /// The joystick: [`JOY_UP`] | [`JOY_FIRE1`]...
    joystick: AtomicU8,
    /// The CRT can be drawn (the GPU has compute shaders).
    pub crt: AtomicBool,
    /// The phone was too slow for the CRT: the picture is raw now (the app says so once).
    pub crt_dropped: AtomicBool,
}

impl Input {
    /// The key of number `index` (line * 8 + bit) held or released.
    pub fn key(&self, index: u8, down: bool) {
        let (line, bit) = (usize::from(index / 8), index % 8);
        if let Some(line) = self.keys.get(line) {
            if down {
                line.fetch_or(1 << bit, Ordering::Relaxed);
            } else {
                line.fetch_and(!(1 << bit), Ordering::Relaxed);
            }
        }
    }

    pub fn release_keys(&self) {
        for line in &self.keys {
            line.store(0, Ordering::Relaxed);
        }
    }

    pub fn joystick(&self, bits: u8) {
        self.joystick.store(bits, Ordering::Relaxed);
    }
}

/// A medium given, kept for the next power on.
#[derive(Clone)]
struct Medium {
    kind: MediaKind,
    name: String,
    bytes: Vec<u8>,
}

/// The files of a saved machine (see [`Machine::save_state`]).
const STATE_FILE: &str = "state.json";
const SNAPSHOT_FILE: &str = "state.sna";
const MEDIUM_FILE: &str = "medium.bin";

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedState {
    config: Config,
    /// The medium's name and kind (its bytes: [`MEDIUM_FILE`]).
    medium: Option<(String, String)>,
}

fn kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Disc => "disc",
        MediaKind::Tape => "tape",
        MediaKind::Cartridge => "cartridge",
        MediaKind::Snapshot => "snapshot",
    }
}

fn kind_of(name: &str) -> Option<MediaKind> {
    Some(match name {
        "disc" => MediaKind::Disc,
        "tape" => MediaKind::Tape,
        "cartridge" => MediaKind::Cartridge,
        "snapshot" => MediaKind::Snapshot,
        _ => return None,
    })
}

pub struct Machine {
    pub config: Config,
    sd: PathBuf,
    pub motherboard: Motherboard,
    monitor: Monitor,
    typist: Option<Typist>,
    medium: Option<Medium>,
    audio: Option<AudioConfig>,
}

impl Machine {
    /// A machine as `config` says, the M4's SD card in `sd`, switched on.
    pub fn new(config: Config, sd: PathBuf) -> Result<Self, String> {
        let motherboard = config.build(None, &sd)?;
        Ok(Self {
            config,
            sd,
            motherboard,
            monitor: Monitor::default(),
            typist: None,
            medium: None,
            audio: None,
        })
    }

    /// The sound wanted (none: silent).
    pub fn set_audio(&mut self, audio: Option<AudioConfig>) {
        self.audio = audio;
        self.motherboard.set_audio_output(audio);
    }

    pub fn display(&self) -> (DisplayMode, Warp) {
        let mode = self.config.display().unwrap_or(DisplayMode::Raw);
        (mode, mode.warp(self.config.curvature()))
    }

    /// New settings: the machine switched off and on again with them, its medium in it.
    pub fn configure(&mut self, config: Config) -> Result<(), String> {
        let old = std::mem::replace(&mut self.config, config);
        if let Err(e) = self.power_on() {
            self.config = old;
            self.power_on()?;
            return Err(e);
        }
        Ok(())
    }

    /// Switched off and on, the medium in it started again.
    pub fn reset(&mut self) -> Result<(), String> {
        self.power_on().map(drop)
    }

    /// The medium `name` (its bytes): a disc in drive A (DSK, HFE, or in an archive), a tape, a
    /// cartridge, or a snapshot. The machine restarts with it and, with autorun, starts it. What
    /// is typed for it comes back.
    pub fn load(&mut self, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String> {
        let kind = autorun::media_kind(name, &bytes)
            .ok_or_else(|| format!("{name}: not a disc, tape, cartridge or snapshot"))?;
        if kind == MediaKind::Snapshot {
            amsrust::snapshot::load_sna_bytes(&mut self.motherboard, &bytes)
                .map_err(|e| format!("{name}: {e}"))?;
            self.typist = None;
            return Ok(None);
        }
        let old = self.medium.replace(Medium {
            kind,
            name: name.to_owned(),
            bytes,
        });
        match self.power_on() {
            Ok(typed) => Ok(typed),
            Err(e) => {
                self.medium = old;
                let _ = self.power_on();
                Err(e)
            }
        }
    }

    /// The machine built again, the medium inserted and its autorun typed; what is typed.
    fn power_on(&mut self) -> Result<Option<String>, String> {
        self.power_on_with(true)
    }

    /// [`Self::power_on`], the autorun typed only if `autorun` (not for a machine restored).
    fn power_on_with(&mut self, autorun: bool) -> Result<Option<String>, String> {
        let medium = self.medium.clone();
        let cartridge = medium.as_ref().filter(|m| m.kind == MediaKind::Cartridge);
        // a cartridge needs a CPC+
        let model = match (cartridge, self.config.model()?) {
            (Some(_), model) if !model.is_plus() => Some(CpcModel::Cpc6128Plus),
            _ => None,
        };
        let mut motherboard = self.config.build(model, &self.sd)?;
        let mut typed = None;
        if let Some(medium) = &medium {
            let name = medium.name.as_str();
            match medium.kind {
                MediaKind::Disc => {
                    let disc = amsrust::disc_image::load_disc_bytes(&medium.bytes, name, None)?;
                    typed = autorun::disc_command(&disc);
                    motherboard.fdc_mut().insert_image(FdcDrive::A, Some(disc));
                }
                MediaKind::Tape => {
                    let tape = amsrust::tape::Tape::from_cdt(&medium.bytes).map_err(|e| format!("{name}: {e}"))?;
                    motherboard.insert_tape(Some(tape));
                    typed = Some(autorun::tape_command(motherboard.has_disc_interface()));
                }
                MediaKind::Cartridge => {
                    let cartridge = amsrust::asic::Cartridge::from_bytes(&medium.bytes, name, None)?;
                    motherboard.insert_cartridge(Some(cartridge));
                    motherboard.restart();
                }
                MediaKind::Snapshot => {}
            }
        }
        motherboard.set_audio_output(self.audio);
        let typed = typed.filter(|_| self.config.autorun && autorun);
        // typed as soon as the firmware waits for a key (BASIC's prompt)
        self.typist = typed.as_deref().map(|text| autorun::typist(text, &motherboard));
        self.motherboard = motherboard;
        self.monitor = Monitor::default();
        Ok(typed)
    }

    /// The machine saved in `dir` (the app going to the background, which Android may then end):
    /// the settings, the medium (the disc as the CPC wrote it, if it did) and a snapshot.
    pub fn save_state(&mut self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        // the disc as the CPC wrote it: what the medium is from now on
        if let Some(medium) = self.medium.as_mut().filter(|m| m.kind == MediaKind::Disc) {
            if self.motherboard.fdc().is_modified(FdcDrive::A) {
                if let Some(disc) = self.motherboard.fdc().image(FdcDrive::A) {
                    medium.bytes = disc.to_bytes();
                    if !medium.name.to_ascii_lowercase().ends_with(".dsk") {
                        medium.name.push_str(".dsk");
                    }
                }
            }
        }
        let sna = amsrust::snapshot::save_sna_bytes(&self.motherboard)?;
        let state = SavedState {
            config: self.config.clone(),
            medium: self.medium.as_ref().map(|m| (m.name.clone(), kind_name(m.kind).to_owned())),
        };
        let write = |name: &str, bytes: &[u8]| {
            // written aside, then renamed: a state half written is never read
            let path = dir.join(name);
            let partial = dir.join(format!("{name}.partial"));
            std::fs::write(&partial, bytes)
                .and_then(|()| std::fs::rename(&partial, &path))
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        if let Some(medium) = &self.medium {
            write(MEDIUM_FILE, &medium.bytes)?;
        }
        write(SNAPSHOT_FILE, &sna)?;
        write(STATE_FILE, serde_json::to_string(&state).map_err(|e| e.to_string())?.as_bytes())
    }

    /// The machine saved in `dir` by [`Self::save_state`], if it was saved with `config` (other
    /// settings: a new machine instead), restored as it was, without its autorun.
    pub fn restore(config: Config, sd: PathBuf, dir: &Path) -> Option<Self> {
        let state: SavedState = serde_json::from_slice(&std::fs::read(dir.join(STATE_FILE)).ok()?).ok()?;
        if state.config != config {
            return None;
        }
        let sna = std::fs::read(dir.join(SNAPSHOT_FILE)).ok()?;
        let mut machine = Self::new(config, sd).ok()?;
        if let Some((name, kind)) = state.medium {
            machine.medium = Some(Medium {
                kind: kind_of(&kind)?,
                name,
                bytes: std::fs::read(dir.join(MEDIUM_FILE)).ok()?,
            });
        }
        machine.power_on_with(false).ok()?;
        amsrust::snapshot::load_sna_bytes(&mut machine.motherboard, &sna).ok()?;
        Some(machine)
    }

    /// Forget the machine saved in `dir` (it could not be restored, or it is outdated).
    pub fn forget_state(dir: &Path) {
        for name in [STATE_FILE, SNAPSHOT_FILE, MEDIUM_FILE] {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }

    /// Text typed on the keyboard, as `--type` does (CSL escapes: `\(RET)`...).
    pub fn type_text(&mut self, text: &str) {
        let now = self.motherboard.t_state_counter().value() as f64
            / amsrust::typist::T_STATES_PER_SECOND as f64;
        self.typist = Some(Typist::new(text, now, self.motherboard.keyboard_layout()));
    }

    /// Whether something is being typed for the user (the autorun).
    pub fn typing(&self) -> bool {
        self.typist.as_ref().is_some_and(|typist| !typist.is_done())
    }

    /// The keys and the joystick of `input` on the matrix, then what the typist holds.
    fn apply(&mut self, input: &Input) {
        let matrix = self.motherboard.keyboard_mut();
        matrix.release_all();
        for (line, held) in input.keys.iter().enumerate() {
            let held = held.load(Ordering::Relaxed);
            for bit in 0..8 {
                if held & (1 << bit) != 0 {
                    if let Some(key) = key_at(line * 8 + bit) {
                        matrix.set_pressed(key, true);
                    }
                }
            }
        }
        let joy = input.joystick.load(Ordering::Relaxed);
        if joy != 0 {
            let axis = |minus: u8, plus: u8| i8::from(joy & plus != 0) - i8::from(joy & minus != 0);
            JoystickState {
                x: axis(JOY_LEFT, JOY_RIGHT),
                y: axis(JOY_UP, JOY_DOWN),
                fire: [joy & JOY_FIRE1 != 0, joy & JOY_FIRE2 != 0, false],
            }
            .apply_to_matrix(&mut self.motherboard, 9);
        }
        if let Some(typist) = &mut self.typist {
            typist.update(&mut self.motherboard);
            if typist.is_done() {
                self.typist = None;
            }
        }
    }

    /// One frame of the machine, `input` held: its picture (none when the CRTC made none).
    pub fn run_frame(&mut self, input: &Input) -> Option<RgbImage> {
        self.apply(input);
        amsrust::frame::run_frame(&mut self.motherboard, &mut self.monitor, None, None, 2 * FRAME_T_STATES)
    }

    /// The sound of the frames run since the last call (stereo, interleaved).
    pub fn take_audio(&mut self) -> Vec<f32> {
        self.motherboard.take_audio_samples()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sd() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bemu-droid-machine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A data disc with GAME.BIN: LD A,&42 : LD (&9000),A : DI : JR $.
    fn game_disc() -> Vec<u8> {
        use cpclib_disc::amsdos::{AmsdosAddBehavior, AmsdosFile, AmsdosFileName, AmsdosManagerMut};
        let mut disc = cpclib_disc::builder::single_head_data_dsk();
        let name = AmsdosFileName::try_from("GAME.BIN").unwrap();
        let code = [0x3e, 0x42, 0x32, 0x00, 0x90, 0xf3, 0x18, 0xfe];
        let file = AmsdosFile::binary_file_from_buffer(&name, 0x4000, 0x4000, &code).unwrap();
        AmsdosManagerMut::new_from_disc(&mut disc, cpclib_disc::edsk::Head::A)
            .add_file(&file, None, false, false, AmsdosAddBehavior::FailIfPresent)
            .unwrap();
        let mut bytes = Vec::new();
        disc.to_buffer(&mut bytes);
        bytes
    }

    #[test]
    fn a_disc_given_starts_by_itself() {
        let mut machine = Machine::new(Config::default(), sd()).unwrap();
        let typed = machine.load("game.dsk", game_disc()).unwrap();
        assert_eq!(typed.as_deref(), Some("RUN\"GAME.BIN\\(RET)"));
        let input = Input::default();
        let mut frames = 0;
        while machine.motherboard.t_state_counter().value() < 10 * 4_000_000 {
            if machine.run_frame(&input).is_some() {
                frames += 1;
            }
        }
        assert!(frames > 400, "{frames} frames");
        assert!(!machine.typing());
        assert_eq!(machine.motherboard.read_bank_byte(2, 0x1000), Some(0x42), "GAME.BIN ran");
    }

    #[test]
    fn the_touch_keys_and_joystick_reach_the_matrix() {
        let mut machine = Machine::new(Config::default(), sd()).unwrap();
        let input = Input::default();
        // A: line 8, bit 5
        input.key(8 * 8 + 5, true);
        input.joystick(JOY_UP | JOY_FIRE1);
        machine.apply(&input);
        let matrix = machine.motherboard.keyboard_mut();
        assert!(matrix.is_pressed(amsrust::keyboard::CpcKey::A));
        assert!(matrix.is_pressed(amsrust::keyboard::CpcKey::Joy0Up));
        assert!(matrix.is_pressed(amsrust::keyboard::CpcKey::Joy0Fire1));
        input.key(8 * 8 + 5, false);
        input.joystick(0);
        machine.apply(&input);
        let matrix = machine.motherboard.keyboard_mut();
        assert!(!matrix.is_pressed(amsrust::keyboard::CpcKey::A));
        assert!(!matrix.is_pressed(amsrust::keyboard::CpcKey::Joy0Up));
    }

    #[test]
    fn a_machine_saved_comes_back_as_it_was() {
        let dir = sd().join("state");
        Machine::forget_state(&dir);
        let mut machine = Machine::new(Config::default(), sd()).unwrap();
        machine.load("game.dsk", game_disc()).unwrap();
        let input = Input::default();
        while machine.motherboard.t_state_counter().value() < 7 * 4_000_000 {
            machine.run_frame(&input);
        }
        assert_eq!(machine.motherboard.read_bank_byte(2, 0x1000), Some(0x42), "GAME.BIN ran");
        // the RAM and the CPU as they are, and a sector "written"
        machine.motherboard.write_bank_byte(1, 0x0123, 0x99);
        let pc = machine.motherboard.pc();
        machine.save_state(&dir).unwrap();

        let restored = Machine::restore(Config::default(), sd(), &dir).expect("restored");
        assert_eq!(restored.motherboard.read_bank_byte(2, 0x1000), Some(0x42));
        assert_eq!(restored.motherboard.read_bank_byte(1, 0x0123), Some(0x99));
        assert_eq!(restored.motherboard.pc(), pc);
        assert!(restored.motherboard.fdc().image(FdcDrive::A).is_some(), "the disc is back in");
        assert!(restored.typist.is_none(), "no autorun for a machine restored");
        // other settings: not this machine
        let other = Config { model: "464".to_owned(), ..Config::default() };
        assert!(Machine::restore(other, sd(), &dir).is_none());
        Machine::forget_state(&dir);
        assert!(Machine::restore(Config::default(), sd(), &dir).is_none());
    }

    #[test]
    fn a_cartridge_switches_to_a_cpc_plus_and_bad_files_are_refused() {
        let mut machine = Machine::new(Config::default(), sd()).unwrap();
        assert!(machine.load("notes.txt", b"hello".to_vec()).is_err());
        assert!(machine.load("broken.dsk", b"MV - CPC garbage".to_vec()).is_err());
        assert!(!machine.motherboard.model().is_plus(), "the machine as it was");
    }
}
