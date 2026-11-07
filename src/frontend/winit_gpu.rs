//! [`Frontend`] on winit and wgpu: the window is drawn by the GPU.
//!
//! The monitor's frame is uploaded as a texture; for a CRT display the shader
//! pass of `amsrust::display::CrtPass` (the one of the GPU filter, see
//! `crt.wgsl`) draws the tube's picture in a texture of its own size, and a
//! blit scales it, letterboxed, to the window with a linear filter. Nothing is
//! read back: the CPU never sees the filtered picture (a recording asks for a
//! CPU filter next to it).
//!
//! winit 0.30 wants to own the event loop; `pump_app_events` lets the
//! emulator keep its own loop, polling the events once per frame.
//!
//! F11, or Ctrl+Cmd+F (macOS's own, where F11 shows the desktop), toggles fullscreen. On
//! Windows and X11, when the monitor offers a video mode at a multiple of 50 Hz (the CPC's
//! rate: 50, 100, 150 or 200 Hz), that mode is taken (exclusive fullscreen), at the monitor's
//! own size and the lowest rate first: the presents then follow the screen's vertical sync
//! (FIFO), each CPC frame shown on 1, 2, 3 or 4 refreshes, and the frontend stops pacing on
//! its own clock. Otherwise, and always on macOS (whose exclusive mode captures the display:
//! nothing shown) and Wayland (which has none), the window goes borderless fullscreen at the
//! desktop's rate, paced the same way when it is a multiple of 50 Hz, by the clock when it
//! is not; the toggle again goes back to the
//! window, as does the window's own control (macOS's green button). F4 closes the emulator.
//! None of them is a CPC key.
//!
//! Keyboard: the keys held are the *logical* ones (what the layout says the
//! key is), as the minifb frontend reports them, with the characters typed:
//! see `amsrust::input` for how the two are combined.

use super::Frontend;
use amsrust::display::{
    wait_for, Blit, CrtParams, CrtPass, DisplayMode, FrameTexture, Warp, RAW_HEIGHT, RAW_WIDTH,
};
use amsrust::input::{HostInput, HostKey};
use image::RgbImage;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::ModifiersState;
use winit::keyboard::{Key, KeyCode, KeyLocation, NamedKey, PhysicalKey};
use winit::monitor::VideoModeHandle;
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Fullscreen, Window, WindowId};

/// The window's initial width, in logical pixels.
const INITIAL_WIDTH: f64 = 1152.0;

/// The picture of a CRT display, in its own texture.
struct CrtState {
    mode: DisplayMode,
    pass: CrtPass,
    target: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (usize, usize),
}

struct Gfx {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    frame: FrameTexture,
    blit: Blit,
    raw_bind_group: wgpu::BindGroup,
    /// How curved the CRTs' glass is, see `DisplayMode::warp`.
    curvature: f32,
    crt: Option<CrtState>,
    /// What the last frame showed, for [`Frontend::redraw`].
    shown: Option<DisplayMode>,
}

impl Gfx {
    fn new(window: Arc<Window>, curvature: f32) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(window)
            .map_err(|e| format!("cannot draw in the window: {e}"))?;
        let adapter = wait_for(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = wait_for(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| format!("cannot use the GPU: {e}"))?;

        let capabilities = surface.get_capabilities(&adapter);
        // The picture is already encoded: no automatic sRGB conversion.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(capabilities.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let frame = FrameTexture::new(&device);
        let blit = Blit::new(&device, format);
        let raw_bind_group = blit.bind_group(&device, frame.view(), false, Warp::FLAT);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            frame,
            blit,
            raw_bind_group,
            curvature,
            crt: None,
            shown: None,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 && (width, height) != (self.config.width, self.config.height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Make `mode`'s pass the current one.
    fn use_mode(&mut self, mode: DisplayMode) {
        let DisplayMode::Crt(profile) = mode else {
            return;
        };
        if self.crt.as_ref().is_some_and(|crt| crt.mode == mode) {
            return;
        }
        let params: CrtParams = profile.params();
        let pass = CrtPass::new(
            &self.device,
            &params,
            &self.frame,
            mode.warp(self.curvature),
        );
        let target =
            CrtPass::create_target(&self.device, &params, wgpu::TextureUsages::TEXTURE_BINDING);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        // the CRT pass already bends the beam under the glass
        let bind_group = self.blit.bind_group(&self.device, &view, true, Warp::FLAT);
        self.crt = Some(CrtState {
            mode,
            pass,
            target,
            bind_group,
            size: params.output_size(),
        });
    }

    /// Draw `mode`'s picture of the frame already uploaded (`again`: the same frame shown on
    /// one more refresh, the CRT's picture of it already drawn). Whether it was shown: not when
    /// the surface had to be configured again (a resize, a fullscreen switch).
    fn draw(&mut self, mode: DisplayMode, again: bool) -> Result<bool, String> {
        let surface_texture = match self.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(false),
            Err(e) => return Err(format!("cannot draw in the window: {e}")),
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        let (bind_group, (picture_w, picture_h)) = match (mode, self.crt.as_ref()) {
            (DisplayMode::Crt(_), Some(crt)) => {
                if !again {
                    crt.pass.encode(
                        &mut encoder,
                        &crt.target
                            .create_view(&wgpu::TextureViewDescriptor::default()),
                    );
                }
                (&crt.bind_group, crt.size)
            }
            _ => (&self.raw_bind_group, (RAW_WIDTH, RAW_HEIGHT)),
        };

        // the picture, as large as the window allows, centred
        let (window_w, window_h) = (self.config.width as f32, self.config.height as f32);
        let scale = (window_w / picture_w as f32).min(window_h / picture_h as f32);
        let (w, h) = (picture_w as f32 * scale, picture_h as f32 * scale);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("window"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_viewport((window_w - w) / 2.0, (window_h - h) / 2.0, w, h, 0.0, 1.0);
            self.blit.draw(&mut pass, bind_group);
        }
        self.queue.submit([encoder.finish()]);
        surface_texture.present();
        self.shown = Some(mode);
        Ok(true)
    }
}

/// F11, or Ctrl+Cmd+F (macOS's fullscreen shortcut: F11 shows its desktop there).
fn is_fullscreen_toggle(event: &KeyEvent, modifiers: ModifiersState) -> bool {
    event.logical_key == Key::Named(NamedKey::F11)
        || (modifiers.control_key()
            && modifiers.super_key()
            && event.physical_key == PhysicalKey::Code(KeyCode::KeyF))
}

/// The physical key, by its place on a US keyboard (`HostInput::placed`), whatever the layout.
fn placed_key(key: PhysicalKey) -> Option<HostKey> {
    use HostKey as H;
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    Some(match code {
        KeyCode::KeyA => H::A,
        KeyCode::KeyB => H::B,
        KeyCode::KeyC => H::C,
        KeyCode::KeyD => H::D,
        KeyCode::KeyE => H::E,
        KeyCode::KeyF => H::F,
        KeyCode::KeyG => H::G,
        KeyCode::KeyH => H::H,
        KeyCode::KeyI => H::I,
        KeyCode::KeyJ => H::J,
        KeyCode::KeyK => H::K,
        KeyCode::KeyL => H::L,
        KeyCode::KeyM => H::M,
        KeyCode::KeyN => H::N,
        KeyCode::KeyO => H::O,
        KeyCode::KeyP => H::P,
        KeyCode::KeyQ => H::Q,
        KeyCode::KeyR => H::R,
        KeyCode::KeyS => H::S,
        KeyCode::KeyT => H::T,
        KeyCode::KeyU => H::U,
        KeyCode::KeyV => H::V,
        KeyCode::KeyW => H::W,
        KeyCode::KeyX => H::X,
        KeyCode::KeyY => H::Y,
        KeyCode::KeyZ => H::Z,
        KeyCode::Digit0 => H::Key0,
        KeyCode::Digit1 => H::Key1,
        KeyCode::Digit2 => H::Key2,
        KeyCode::Digit3 => H::Key3,
        KeyCode::Digit4 => H::Key4,
        KeyCode::Digit5 => H::Key5,
        KeyCode::Digit6 => H::Key6,
        KeyCode::Digit7 => H::Key7,
        KeyCode::Digit8 => H::Key8,
        KeyCode::Digit9 => H::Key9,
        KeyCode::Backquote => H::Backquote,
        KeyCode::Minus => H::Minus,
        KeyCode::Equal => H::Equal,
        KeyCode::BracketLeft => H::LeftBracket,
        KeyCode::BracketRight => H::RightBracket,
        KeyCode::Backslash => H::Backslash,
        KeyCode::Semicolon => H::Semicolon,
        KeyCode::Quote => H::Apostrophe,
        KeyCode::Comma => H::Comma,
        KeyCode::Period => H::Period,
        KeyCode::Slash => H::Slash,
        KeyCode::IntlBackslash => H::IntlBackslash,
        KeyCode::Space => H::Space,
        KeyCode::Enter => H::Enter,
        KeyCode::Tab => H::Tab,
        KeyCode::CapsLock => H::CapsLock,
        KeyCode::Escape => H::Escape,
        KeyCode::Backspace => H::Backspace,
        KeyCode::Delete => H::Delete,
        KeyCode::ShiftLeft => H::LeftShift,
        KeyCode::ShiftRight => H::RightShift,
        KeyCode::ControlLeft => H::LeftCtrl,
        KeyCode::ControlRight => H::RightCtrl,
        KeyCode::AltLeft => H::LeftAlt,
        KeyCode::AltRight => H::RightAlt,
        KeyCode::ArrowUp => H::Up,
        KeyCode::ArrowDown => H::Down,
        KeyCode::ArrowLeft => H::Left,
        KeyCode::ArrowRight => H::Right,
        KeyCode::Numpad0 => H::NumPad0,
        KeyCode::Numpad1 => H::NumPad1,
        KeyCode::Numpad2 => H::NumPad2,
        KeyCode::Numpad3 => H::NumPad3,
        KeyCode::Numpad4 => H::NumPad4,
        KeyCode::Numpad5 => H::NumPad5,
        KeyCode::Numpad6 => H::NumPad6,
        KeyCode::Numpad7 => H::NumPad7,
        KeyCode::Numpad8 => H::NumPad8,
        KeyCode::Numpad9 => H::NumPad9,
        KeyCode::NumpadDecimal => H::NumPadDot,
        KeyCode::NumpadEnter => H::NumPadEnter,
        _ => return None,
    })
}

/// The logical key as the CPC keyboard table (`amsrust::input`) knows it.
fn host_key(event: &KeyEvent) -> Option<HostKey> {
    use HostKey as H;
    let numpad = event.location == KeyLocation::Numpad;
    match &event.logical_key {
        Key::Named(named) => Some(match named {
            NamedKey::Space => H::Space,
            NamedKey::Enter if numpad => H::NumPadEnter,
            NamedKey::Enter => H::Enter,
            NamedKey::Shift if event.location == KeyLocation::Right => H::RightShift,
            NamedKey::Shift => H::LeftShift,
            NamedKey::Control if event.location == KeyLocation::Right => H::RightCtrl,
            NamedKey::Control => H::LeftCtrl,
            // AltGr is not Alt: it types characters (see the typed ones)
            NamedKey::Alt if event.location == KeyLocation::Right => H::RightAlt,
            NamedKey::Alt => H::LeftAlt,
            NamedKey::CapsLock => H::CapsLock,
            NamedKey::Tab => H::Tab,
            NamedKey::Escape => H::Escape,
            NamedKey::Backspace => H::Backspace,
            NamedKey::Delete => H::Delete,
            NamedKey::ArrowUp => H::Up,
            NamedKey::ArrowDown => H::Down,
            NamedKey::ArrowLeft => H::Left,
            NamedKey::ArrowRight => H::Right,
            NamedKey::F1 => H::F1,
            NamedKey::F2 => H::F2,
            NamedKey::F3 => H::F3,
            NamedKey::F7 => H::F7,
            NamedKey::F8 => H::F8,
            NamedKey::F9 => H::F9,
            NamedKey::F6 => H::F6,
            NamedKey::F5 => H::F5,
            NamedKey::F10 => H::F10,
            NamedKey::F12 => H::F12,
            _ => return None,
        }),
        Key::Character(text) => {
            let mut chars = text.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let numpad_digits = [
                H::NumPad0,
                H::NumPad1,
                H::NumPad2,
                H::NumPad3,
                H::NumPad4,
                H::NumPad5,
                H::NumPad6,
                H::NumPad7,
                H::NumPad8,
                H::NumPad9,
            ];
            let digits = [
                H::Key0,
                H::Key1,
                H::Key2,
                H::Key3,
                H::Key4,
                H::Key5,
                H::Key6,
                H::Key7,
                H::Key8,
                H::Key9,
            ];
            let letters = [
                H::A,
                H::B,
                H::C,
                H::D,
                H::E,
                H::F,
                H::G,
                H::H,
                H::I,
                H::J,
                H::K,
                H::L,
                H::M,
                H::N,
                H::O,
                H::P,
                H::Q,
                H::R,
                H::S,
                H::T,
                H::U,
                H::V,
                H::W,
                H::X,
                H::Y,
                H::Z,
            ];
            let c = c.to_ascii_lowercase();
            Some(match c {
                'a'..='z' => letters[c as usize - 'a' as usize],
                '0'..='9' if numpad => numpad_digits[c as usize - '0' as usize],
                '0'..='9' => digits[c as usize - '0' as usize],
                '.' if numpad => H::NumPadDot,
                '-' => H::Minus,
                '[' => H::LeftBracket,
                ']' => H::RightBracket,
                '\\' => H::Backslash,
                ';' => H::Semicolon,
                '\'' => H::Apostrophe,
                ',' => H::Comma,
                '.' => H::Period,
                '/' => H::Slash,
                '`' => H::Backquote,
                '=' => H::Equal,
                _ => return None,
            })
        }
        _ => None,
    }
}

struct App {
    /// Logical size of the window to open.
    size: (f64, f64),
    /// See `Gfx::curvature`.
    curvature: f32,
    window: Option<Arc<Window>>,
    gfx: Option<Gfx>,
    /// Keys held, by physical key, so that a release is matched with its
    /// press whatever the modifiers made of the logical key meanwhile.
    held: HashMap<PhysicalKey, HostKey>,
    /// The physical keys held, by their place (the positional mapping's).
    placed: HashMap<PhysicalKey, HostKey>,
    typed: Vec<char>,
    closed: bool,
    error: Option<String>,
    /// F11 asked for fullscreen (kept here: the window manager may refuse it).
    fullscreen: bool,
    /// The modifiers held, for Ctrl+Cmd+F.
    modifiers: ModifiersState,
    /// In fullscreen at a multiple of 50 Hz (the CPC's rate): how many refreshes show a CPC
    /// frame (1 at 50 Hz, 2 at 100 Hz...), the vertical sync pacing the emulation. `None`: the
    /// clock paces it.
    refreshes_per_frame: Option<u32>,
}

/// How many refreshes at `millihertz` show a CPC frame, if it is a multiple of 50 Hz (within
/// 0.1 Hz per frame shown, a "50 Hz" mode's 49.92 or 50.08): 1 at 50 Hz, 2 at 100 Hz... up to 4
/// (200 Hz). Further off, the sound (paced by its device) would run dry or pile up.
fn refreshes_per_frame(millihertz: u32) -> Option<u32> {
    (1..=4u32).find(|k| (50_000 * k).abs_diff(millihertz) <= 100 * k)
}

/// A video mode of `window`'s monitor at a multiple of 50 Hz, and how many of its refreshes
/// show a CPC frame: at the monitor's own size (or the largest), the fewest refreshes per frame,
/// the deepest colours first.
fn paced_mode(window: &Window) -> Option<(VideoModeHandle, u32)> {
    let monitor = window.current_monitor()?;
    let size = monitor.size();
    monitor
        .video_modes()
        .filter_map(|mode| refreshes_per_frame(mode.refresh_rate_millihertz()).map(|k| (mode, k)))
        .max_by_key(|(mode, k)| {
            (
                mode.size() == size,
                std::cmp::Reverse(*k),
                mode.size().width * mode.size().height,
                mode.bit_depth(),
            )
        })
}

impl App {
    /// F11: fullscreen in a 50 Hz mode when there is one (and the system has an exclusive
    /// mode that shows us), borderless otherwise; back to the window.
    fn toggle_fullscreen(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        // fullscreen too when the system put the window there (macOS's green button)
        if self.fullscreen || window.fullscreen().is_some() {
            window.set_fullscreen(None);
            self.left_fullscreen();
            return;
        }
        self.fullscreen = true;
        self.refreshes_per_frame = None;
        let exclusive = if exclusive_fullscreen_works(event_loop) {
            paced_mode(&window)
        } else {
            None
        };
        if let Some((mode, k)) = exclusive {
            window.set_fullscreen(Some(Fullscreen::Exclusive(mode.clone())));
            // a mode the system turned down: borderless
            if matches!(window.fullscreen(), Some(Fullscreen::Exclusive(_))) {
                self.refreshes_per_frame = Some(k);
                eprintln!(
                    "fullscreen {}x{} at {:.3} Hz: a CPC frame every {k} refresh(es)",
                    mode.size().width,
                    mode.size().height,
                    f64::from(mode.refresh_rate_millihertz()) / 1000.0
                );
            }
        }
        if self.refreshes_per_frame.is_none() {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
            self.entered_borderless();
        }
        window.set_cursor_visible(false);
    }

    /// In borderless fullscreen (asked for, or by the system): the desktop's own rate, if a
    /// multiple of 50 Hz, paces the emulation, else the clock.
    fn entered_borderless(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        self.fullscreen = true;
        let desktop = window
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        self.refreshes_per_frame = desktop.and_then(refreshes_per_frame);
        match (desktop, self.refreshes_per_frame) {
            (Some(rate), Some(k)) => eprintln!(
                "fullscreen: borderless at the desktop's {:.3} Hz: a CPC frame every {k} refresh(es)",
                f64::from(rate) / 1000.0
            ),
            _ => eprintln!("fullscreen: borderless at the desktop's rate (not a multiple of 50 Hz: the clock paces the emulation)")
        }
        window.set_cursor_visible(false);
    }

    /// Out of fullscreen, by the toggle or by the window's own control.
    fn left_fullscreen(&mut self) {
        self.fullscreen = false;
        self.refreshes_per_frame = None;
        if let Some(window) = &self.window {
            window.set_cursor_visible(true);
        }
    }
}

/// Whether an exclusive (video mode) fullscreen shows the picture: Windows' and X11's do;
/// Wayland has none (the request is ignored) and macOS's captures the display, where the
/// window's Metal layer shows nothing; its borderless fullscreen is the idiomatic one.
fn exclusive_fullscreen_works(event_loop: &ActiveEventLoop) -> bool {
    #[cfg(target_os = "windows")]
    {
        let _ = event_loop;
        true
    }
    #[cfg(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "android"),
        not(target_os = "ios")
    ))]
    {
        use winit::platform::x11::ActiveEventLoopExtX11;
        event_loop.is_x11()
    }
    #[cfg(not(any(
        target_os = "windows",
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_os = "ios")
        )
    )))]
    {
        let _ = event_loop;
        false
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("amsrust")
            .with_inner_size(LogicalSize::new(self.size.0, self.size.1));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let window = Arc::new(window);
                match Gfx::new(window.clone(), self.curvature) {
                    Ok(gfx) => self.gfx = Some(gfx),
                    Err(e) => self.error = Some(e),
                }
                self.window = Some(window);
            }
            Err(e) => self.error = Some(format!("failed to open a window: {e}")),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.closed = true,
            WindowEvent::Resized(size) => {
                if let Some(gfx) = &mut self.gfx {
                    gfx.resize(size.width, size.height);
                }
                // in or out of fullscreen by the window's own control (macOS's green button, a
                // window manager's shortcut)
                let system = self
                    .window
                    .as_ref()
                    .is_some_and(|window| window.fullscreen().is_some());
                if self.fullscreen && !system {
                    self.left_fullscreen();
                } else if !self.fullscreen && system {
                    self.entered_borderless();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                match event.state {
                    // F4 closes the emulator
                    ElementState::Pressed if event.logical_key == Key::Named(NamedKey::F4) => {
                        self.closed = true
                    }
                    ElementState::Pressed if is_fullscreen_toggle(&event, self.modifiers) => {
                        if !event.repeat {
                            self.toggle_fullscreen(event_loop);
                        }
                    }
                    ElementState::Pressed => {
                        if let Some(key) = host_key(&event) {
                            self.held.insert(event.physical_key, key);
                        }
                        if let Some(key) = placed_key(event.physical_key) {
                            self.placed.insert(event.physical_key, key);
                        }
                        // what the layout makes of the key and the modifiers
                        if let Some(text) = &event.text {
                            self.typed.extend(text.chars().filter(|c| !c.is_control()));
                        }
                    }
                    ElementState::Released => {
                        self.held.remove(&event.physical_key);
                        self.placed.remove(&event.physical_key);
                    }
                }
            }
            WindowEvent::Focused(false) => {
                self.held.clear();
                self.placed.clear();
            }
            _ => {}
        }
    }
}

pub struct WinitGpuFrontend {
    event_loop: EventLoop<()>,
    app: App,
    /// Time between two presents, if capped.
    frame_time: Option<Duration>,
    deadline: Instant,
}

impl Drop for WinitGpuFrontend {
    /// Out of an exclusive fullscreen first: X11 keeps a video mode the program set (winit
    /// restores the desktop's only when the window leaves it), Windows reverts it at exit.
    fn drop(&mut self) {
        if let Some(window) = &self.app.window {
            if matches!(window.fullscreen(), Some(Fullscreen::Exclusive(_))) {
                window.set_fullscreen(None);
            }
        }
    }
}

impl WinitGpuFrontend {
    /// A window for pictures of `picture` pixels (for its shape and size).
    /// CRTs are seen through their glass, curved `curvature` times their bulge (0: flat).
    pub fn open(picture: (usize, usize), curvature: f32) -> Result<Self, String> {
        let event_loop =
            EventLoop::new().map_err(|e| format!("cannot start the event loop: {e}"))?;
        let width = INITIAL_WIDTH;
        let size = (width, width * picture.1 as f64 / picture.0 as f64);
        let mut frontend = Self {
            event_loop,
            app: App {
                size,
                curvature,
                window: None,
                gfx: None,
                held: HashMap::new(),
                placed: HashMap::new(),
                typed: Vec::new(),
                closed: false,
                error: None,
                fullscreen: false,
                modifiers: ModifiersState::empty(),
                refreshes_per_frame: None,
            },
            frame_time: None,
            deadline: Instant::now(),
        };
        // the window and the GPU come up on the first events
        for _ in 0..100 {
            frontend.pump();
            if frontend.app.gfx.is_some() || frontend.app.error.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        match frontend.app.error.take() {
            Some(e) => Err(e),
            None if frontend.app.gfx.is_none() => Err("the window did not open".to_owned()),
            None => Ok(frontend),
        }
    }

    fn pump(&mut self) {
        if let PumpStatus::Exit(_) = self
            .event_loop
            .pump_app_events(Some(Duration::ZERO), &mut self.app)
        {
            self.app.closed = true;
        }
    }

    /// Wait for the end of the frame when the rate is capped (not in a 50 Hz
    /// fullscreen mode: the vertical sync, FIFO, does it).
    fn pace(&mut self) {
        if self.app.refreshes_per_frame.is_some() {
            // a present that didn't wait for the refresh (a hidden window, another Space):
            // the clock's frame, so that the emulation doesn't run free
            const FRAME: Duration = Duration::from_millis(20);
            let took = self.deadline.elapsed();
            if took < FRAME / 2 {
                std::thread::sleep(FRAME - took);
            }
            self.deadline = Instant::now();
            return;
        }
        if let Some(frame_time) = self.frame_time {
            self.deadline += frame_time;
            let now = Instant::now();
            if self.deadline > now {
                std::thread::sleep(self.deadline - now);
            } else {
                self.deadline = now;
            }
        }
    }

    fn gfx(&mut self) -> Result<&mut Gfx, String> {
        self.app.gfx.as_mut().ok_or_else(|| "no window".to_owned())
    }
}

impl Frontend for WinitGpuFrontend {
    fn is_open(&self) -> bool {
        !self.app.closed
    }

    fn set_target_fps(&mut self, fps: usize) {
        self.frame_time = (fps > 0).then(|| Duration::from_secs_f64(1.0 / fps as f64));
        self.deadline = Instant::now();
    }

    fn knows_key_places(&self) -> bool {
        true
    }

    fn poll_input(&mut self) -> HostInput {
        self.pump();
        HostInput {
            down: self.app.held.values().copied().collect(),
            typed: std::mem::take(&mut self.app.typed),
            placed: self.app.placed.values().copied().collect(),
        }
    }

    fn present(&mut self, _pixels: &[u32], _size: (usize, usize)) -> Result<(), String> {
        Err("this frontend draws the monitor's frames itself (present_frame)".to_owned())
    }

    fn direct(&self) -> bool {
        true
    }

    fn present_frame(&mut self, frame: &RgbImage, mode: DisplayMode) -> Result<(), String> {
        let gfx = self.gfx()?;
        gfx.use_mode(mode);
        gfx.frame.upload(&gfx.queue, frame);
        let mut shown = gfx.draw(mode, false)?;
        // at 100 Hz (or more) the frame on each refresh of its own: the vertical sync paces 50
        // (the CRT's picture drawn by the first draw that was shown)
        for _ in 1..self.app.refreshes_per_frame.unwrap_or(1) {
            shown |= self.gfx()?.draw(mode, shown)?;
        }
        self.pace();
        Ok(())
    }

    fn redraw(&mut self) -> Result<(), String> {
        let gfx = self.gfx()?;
        let mode = gfx.shown.unwrap_or_default();
        gfx.use_mode(mode);
        gfx.draw(mode, false).map(|_| ())
    }
}

#[cfg(test)]
mod test {
    use super::refreshes_per_frame;

    #[test]
    fn multiples_of_50_hz_show_each_frame_on_their_refreshes() {
        assert_eq!(refreshes_per_frame(50_000), Some(1));
        assert_eq!(refreshes_per_frame(49_920), Some(1), "a monitor's 49.92 Hz");
        assert_eq!(refreshes_per_frame(100_000), Some(2));
        assert_eq!(
            refreshes_per_frame(99_850),
            Some(2),
            "0.1 Hz per frame shown"
        );
        assert_eq!(
            refreshes_per_frame(99_300),
            None,
            "too far: the clock paces it"
        );
        assert_eq!(refreshes_per_frame(49_500), None);
        assert_eq!(refreshes_per_frame(150_000), Some(3));
        assert_eq!(refreshes_per_frame(200_000), Some(4));
        assert_eq!(refreshes_per_frame(60_000), None);
        assert_eq!(refreshes_per_frame(59_940), None);
        assert_eq!(refreshes_per_frame(120_000), None);
        assert_eq!(refreshes_per_frame(144_000), None);
        assert_eq!(
            refreshes_per_frame(250_000),
            None,
            "more than 4 refreshes a frame"
        );
    }
}
