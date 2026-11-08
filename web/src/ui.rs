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
    Fullscreen,
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
    applied: HostSettings,
    requests: Vec<HostRequest>,
    pub page: Vec<PageRequest>,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    pixels_per_point: f32,
    start: f64,
    now: f64,
}

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
            // the page has the clipboard, the worker does not
            HostRequest::PasteClipboard | HostRequest::CopyScreenshot => false,
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
            applied: settings.clone(),
            settings,
            requests: Vec::new(),
            page: Vec::new(),
            events: Vec::new(),
            modifiers: egui::Modifiers::default(),
            pixels_per_point: 1.0,
            start: js_sys::Date::now(),
            now: 0.0,
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
        let output = self.gui.run(raw, &target, &mut self.bus);
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
                _ => None,
            });
        }
        self.apply();
        output
    }

    /// The settings the page applies itself.
    fn apply(&mut self) {
        let (wanted, applied) = (&self.settings, &self.applied);
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
            self.page.push(PageRequest::Fullscreen);
        }
        self.applied = self.settings.clone();
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
