//! The menus, the command palette and the debugger's windows of the page (`amsrust::ui`), drawn
//! over the picture on WebGPU / WebGL: the page forwards the pointer and the keys
//! ([`WebUi::pointer`], [`WebUi::key`]), the commands run on the machine between two frames,
//! and what only the page can do (pick a file, download one, the speed, the volume,
//! fullscreen) goes back to it as requests (`Emulator::take_ui_requests`).

use amsrust::commands::{
    Command, CommandBus, CommandError, CommandTarget, HostRequest, HostSettings, Recording, Speed,
};
use amsrust::debug_server::DebugServer;
use amsrust::motherboard::Motherboard;
use amsrust::ui::{FileRequest, Gui};
use serde::Serialize;

/// What the page is asked to do.
#[derive(Serialize, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PageRequest {
    /// Choose a file for the command `id`, then `answerFile(id, name, bytes)`.
    PickFile {
        id: String,
        title: String,
        accept: String,
    },
    /// Save `data` as `name` (a download).
    Download {
        name: String,
        #[serde(with = "serde_bytes_as_array")]
        data: Vec<u8>,
    },
    /// The emulation's speed: times real time, or "max".
    Speed {
        value: f64,
    },
    Volume {
        volume: f64,
        muted: bool,
    },
    /// Fullscreen on or off.
    Fullscreen {
        on: bool,
    },
    /// Type the page's clipboard on the CPC keyboard.
    Paste,
    /// The last frame to the clipboard.
    CopyScreenshot,
    OpenUrl {
        url: String,
    },
    /// Play a CSL script, stop it.
    RunCsl {
        text: String,
    },
    StopCsl,
    /// The last frame, as a PNG download.
    Screenshot,
}

mod serde_bytes_as_array {
    pub fn serialize<S: serde::Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytes)
    }
}

/// The interface's state, next to the machine.
pub struct WebUi {
    pub gui: Gui,
    pub bus: CommandBus,
    pub debugger: DebugServer,
    pub settings: HostSettings,
    /// What the page last applied of the settings it applies (speed, volume, fullscreen).
    applied: PageSettings,
    /// Function keys pressed, their commands chosen at the next frame (with the machine).
    chords: Vec<(bool, &'static str)>,
    requests: Vec<HostRequest>,
    pub page: Vec<PageRequest>,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    pixels_per_point: f32,
    start: f64,
    now: f64,
    /// Texture updates of outputs not drawn (no picture yet, a surface to configure again): the
    /// next output draws them first, else the menus lose their font.
    pending_textures: egui::TexturesDelta,
}

/// The settings the page applies itself.
#[derive(Clone, Copy, PartialEq)]
struct PageSettings {
    speed: Speed,
    volume: u8,
    muted: bool,
    fullscreen: bool,
}

impl PageSettings {
    fn of(settings: &HostSettings) -> Self {
        Self {
            speed: settings.speed,
            volume: settings.volume,
            muted: settings.muted,
            fullscreen: settings.fullscreen,
        }
    }
}

/// The function keys whose commands the page runs (F10 and F11 are the page's own).
const FUNCTION_KEYS: [&str; 6] = ["F5", "F6", "F7", "F8", "F9", "F12"];

/// The commands' receiver in the page.
pub struct Target<'a> {
    pub motherboard: &'a mut Motherboard,
    pub debugger: &'a mut DebugServer,
    pub settings: &'a mut HostSettings,
    pub requests: &'a mut Vec<HostRequest>,
    pub csl_running: bool,
}

impl CommandTarget for Target<'_> {
    fn motherboard(&self) -> &Motherboard {
        self.motherboard
    }
    fn motherboard_mut(&mut self) -> &mut Motherboard {
        self.motherboard
    }
    fn debugger(&self) -> Option<&DebugServer> {
        Some(self.debugger)
    }
    fn debugger_mut(&mut self) -> Option<&mut DebugServer> {
        Some(self.debugger)
    }
    fn debugger_and_motherboard(&mut self) -> Option<(&mut DebugServer, &mut Motherboard)> {
        Some((self.debugger, self.motherboard))
    }
    fn settings(&self) -> Option<&HostSettings> {
        Some(self.settings)
    }
    fn settings_mut(&mut self) -> Option<&mut HostSettings> {
        Some(self.settings)
    }
    fn request(&mut self, request: HostRequest) -> Result<(), CommandError> {
        if !self.can(&request) {
            return Err(CommandError::Unsupported);
        }
        self.requests.push(request);
        Ok(())
    }
    fn can(&self, request: &HostRequest) -> bool {
        match request {
            HostRequest::StartRecording(..) | HostRequest::StopRecording(_) => false,
            HostRequest::DebugServer(_) | HostRequest::SavePrintout | HostRequest::Quit => false,
            HostRequest::StopCsl => self.csl_running,
            _ => true,
        }
    }
    fn is_recording(&self, _: Recording) -> bool {
        false
    }
    fn has_file_system(&self) -> bool {
        false
    }
    fn csl_running(&self) -> bool {
        self.csl_running
    }
}

impl WebUi {
    pub fn new(settings: HostSettings) -> Self {
        Self {
            gui: Gui::new(),
            bus: CommandBus::default(),
            debugger: DebugServer::offline(),
            applied: PageSettings::of(&settings),
            chords: Vec::new(),
            settings,
            requests: Vec::new(),
            page: Vec::new(),
            events: Vec::new(),
            modifiers: egui::Modifiers::default(),
            pixels_per_point: 1.0,
            start: js_sys::Date::now(),
            now: 0.0,
            pending_textures: egui::TexturesDelta::default(),
        }
    }

    pub fn set_pixels_per_point(&mut self, ppp: f32) {
        if ppp > 0.0 {
            self.pixels_per_point = ppp;
        }
    }

    /// A pointer event at (`x`, `y`) CSS pixels of the canvas: "move", "down", "up" or "leave",
    /// `button` 0 (primary), 1 (middle), 2 (secondary). Whether the interface is under it.
    pub fn pointer(&mut self, kind: &str, x: f32, y: f32, button: u8) -> bool {
        let pos = egui::pos2(x, y);
        let button = match button {
            1 => egui::PointerButton::Middle,
            2 => egui::PointerButton::Secondary,
            _ => egui::PointerButton::Primary,
        };
        match kind {
            "move" => self.events.push(egui::Event::PointerMoved(pos)),
            "down" | "up" => {
                self.events.push(egui::Event::PointerMoved(pos));
                self.events.push(egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: kind == "down",
                    modifiers: self.modifiers,
                });
            }
            _ => self.events.push(egui::Event::PointerGone),
        }
        self.gui.wants_pointer() || self.gui.wants_keyboard()
    }

    pub fn wheel(&mut self, dx: f32, dy: f32) {
        self.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(-dx, -dy),
            modifiers: self.modifiers,
        });
    }

    /// A key of the page: whether the interface takes it (else it is the CPC's).
    #[allow(clippy::too_many_arguments)]
    pub fn key(&mut self, code: &str, key: &str, pressed: bool, ctrl: bool, shift: bool, alt: bool, meta: bool) -> bool {
        self.modifiers = egui::Modifiers { alt, ctrl, shift, mac_cmd: meta, command: ctrl || meta };
        // a command's chord (Ctrl+Shift+P): run now, so that the next keys go to the palette
        if pressed && (ctrl || alt || meta) {
            if let Some(id) = shortcut(key, ctrl || meta, shift, alt) {
                if id == "palette-open" {
                    self.gui.handle(&HostRequest::Open(amsrust::commands::View::Palette));
                } else {
                    self.bus.push_id(id);
                }
                return true;
            }
        }
        let taken = self.gui.wants_keyboard();
        // the function keys of the commands (F8 pauses...), not the CPC's
        if !taken && !(ctrl || alt || meta) {
            if let Some(name) = FUNCTION_KEYS.iter().find(|name| **name == key) {
                if pressed {
                    self.chords.push((shift, name));
                }
                return true;
            }
        }
        if let Some(egui_key) = egui::Key::from_name(key).or_else(|| egui::Key::from_name(code.trim_start_matches("Key"))) {
            self.events.push(egui::Event::Key {
                key: egui_key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: self.modifiers
            });
        }
        if taken && pressed && key.chars().count() == 1 && !ctrl && !meta {
            self.events.push(egui::Event::Text(key.to_owned()));
        }
        taken
    }

    /// Lay out the interface for a canvas of `size` pixels, running its commands on `target`'s
    /// machine first.
    pub fn frame(
        &mut self,
        size: (u32, u32),
        motherboard: &mut Motherboard,
        csl_running: bool,
    ) -> egui::FullOutput {
        self.now = (js_sys::Date::now() - self.start) / 1000.0;
        let mut target = Target {
            motherboard,
            debugger: &mut self.debugger,
            settings: &mut self.settings,
            requests: &mut self.requests,
            csl_running,
        };
        for (shift, key) in std::mem::take(&mut self.chords) {
            let chord = amsrust::commands::Chord {
                ctrl: false,
                shift,
                alt: false,
                key,
            };
            let found = amsrust::commands::shortcut_command(chord, &target).or_else(|| {
                amsrust::commands::shortcut_command(
                    amsrust::commands::Chord {
                        shift: false,
                        ..chord
                    },
                    &target,
                )
            });
            if let Some(id) = found {
                self.bus.push_id(id);
            }
        }
        self.bus.run(&mut target);
        let ppp = self.pixels_per_point;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.0 as f32 / ppp, size.1 as f32 / ppp),
            )),
            time: Some(self.now),
            modifiers: self.modifiers,
            events: std::mem::take(&mut self.events),
            focused: true,
            ..Default::default()
        };
        self.gui.context().set_pixels_per_point(ppp);
        let mut output = self.gui.run(raw, &target, &mut self.bus);
        if !self.pending_textures.is_empty() {
            let mut textures = std::mem::take(&mut self.pending_textures);
            textures.append(std::mem::take(&mut output.textures_delta));
            output.textures_delta = textures;
        }
        for FileRequest {
            id,
            title,
            extensions,
            ..
        } in self.gui.take_file_requests()
        {
            let accept = extensions
                .iter()
                .map(|ext| format!(".{ext}"))
                .collect::<Vec<_>>()
                .join(",");
            self.page.push(PageRequest::PickFile {
                id: id.to_owned(),
                title: title.to_owned(),
                accept,
            });
        }
        let reports: Vec<_> = self.bus.take_reports().collect();
        self.gui.report(reports.into_iter());
        for request in std::mem::take(&mut self.requests) {
            if self.gui.handle(&request) {
                continue;
            }
            self.page.extend(match request {
                HostRequest::SaveFile { name, data } => Some(PageRequest::Download { name, data }),
                HostRequest::Screenshot(_) => Some(PageRequest::Screenshot),
                HostRequest::OpenUrl(url) => Some(PageRequest::OpenUrl { url }),
                HostRequest::RunCsl { text, .. } => Some(PageRequest::RunCsl { text }),
                HostRequest::StopCsl => Some(PageRequest::StopCsl),
                HostRequest::PasteClipboard => Some(PageRequest::Paste),
                HostRequest::CopyScreenshot => Some(PageRequest::CopyScreenshot),
                _ => None,
            });
        }
        self.apply();
        output
    }

    /// The settings the page applies itself.
    fn apply(&mut self) {
        let wanted = PageSettings::of(&self.settings);
        let applied = self.applied;
        if wanted.speed != applied.speed {
            self.page.push(PageRequest::Speed {
                value: match wanted.speed {
                    Speed::Max => f64::INFINITY,
                    Speed::Factor(factor) => f64::from(factor),
                },
            });
        }
        if wanted.volume != applied.volume || wanted.muted != applied.muted {
            self.page.push(PageRequest::Volume {
                volume: f64::from(wanted.volume) / 100.0,
                muted: wanted.muted,
            });
        }
        if wanted.fullscreen != applied.fullscreen {
            self.page.push(PageRequest::Fullscreen { on: wanted.fullscreen });
        }
        self.applied = wanted;
    }

    /// The page changed the speed (F10, its own controls): the menus follow, nothing to send.
    pub fn page_speed(&mut self, speed: Speed) {
        self.settings.speed = speed;
        self.applied.speed = speed;
    }

    /// The page went in or out of fullscreen (F11, Escape).
    pub fn page_fullscreen(&mut self, on: bool) {
        self.settings.fullscreen = on;
        self.applied.fullscreen = on;
    }

    /// The page's volume (its own controls).
    pub fn page_volume(&mut self, volume: f64, muted: bool) {
        self.settings.volume = (volume * 100.0).round().clamp(0.0, 100.0) as u8;
        self.settings.muted = muted;
        self.applied.volume = self.settings.volume;
        self.applied.muted = muted;
    }

    /// An output that was not drawn: its texture updates go with the next one.
    pub fn not_drawn(&mut self, output: egui::FullOutput) {
        let mut textures = std::mem::take(&mut self.pending_textures);
        textures.append(output.textures_delta);
        self.pending_textures = textures;
    }

    /// A file the page picked for the command `id`.
    pub fn answer_file(&mut self, id: &str, name: String, data: Vec<u8>) {
        if let Ok(command) = Command::with_arg(id, amsrust::commands::Arg::Bytes { name, data }) {
            self.bus.push(command);
        }
    }

    pub fn paused(&self) -> bool {
        self.debugger.paused()
    }
}

/// The command whose chord this is (Ctrl+Shift+P: the palette): the CPC does not get the key.
fn shortcut(key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<&'static str> {
    amsrust::commands::all()
        .iter()
        .find(|spec| {
            spec.shortcut
                .and_then(amsrust::commands::Chord::parse)
                .is_some_and(|chord| {
                    chord.ctrl == ctrl
                        && chord.shift == shift
                        && chord.alt == alt
                        && chord.key.eq_ignore_ascii_case(key)
                })
        })
        .map(|spec| spec.id)
}
