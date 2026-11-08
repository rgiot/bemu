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
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
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

/// The user interface (egui) of the window: its input, and the last frame to draw.
struct UiLayer {
    ctx: egui::Context,
    state: Option<egui_winit::State>,
    painter: Option<amsrust::ui::Painter>,
    output: Option<egui::FullOutput>,
    /// The last frame's shapes, drawn again on the extra refreshes of a 100-200 Hz fullscreen.
    shapes: Vec<egui::epaint::ClippedShape>,
    pixels_per_point: f32,
    /// The menu bar's height in pixels: the picture goes below.
    top_pixels: f32,
}

/// A debugger's window, a window of the system: its surface on the main window's GPU device,
/// its own interface.
struct ViewWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    state: egui_winit::State,
    ctx: egui::Context,
    painter: Option<amsrust::ui::Painter>,
    /// The size last asked for its content, in points.
    fitted: Option<egui::Vec2>,
}

impl ViewWindow {
    fn open(
        event_loop: &ActiveEventLoop,
        gfx: &Gfx,
        title: &str,
        ctx: egui::Context,
    ) -> Result<Self, String> {
        let attributes = Window::default_attributes()
            .with_title(format!("bemu: {title}"))
            .with_inner_size(LogicalSize::new(460.0, 520.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| format!("cannot open the {title} window: {e}"))?,
        );
        let surface = gfx
            .instance
            .create_surface(window.clone())
            .map_err(|e| format!("cannot draw in the {title} window: {e}"))?;
        let capabilities = surface.get_capabilities(&gfx.adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(capabilities.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            // it must not hold back the emulation: no wait for the refresh
            present_mode: if capabilities
                .present_modes
                .contains(&wgpu::PresentMode::Mailbox)
            {
                wgpu::PresentMode::Mailbox
            } else if capabilities
                .present_modes
                .contains(&wgpu::PresentMode::Immediate)
            {
                wgpu::PresentMode::Immediate
            } else {
                wgpu::PresentMode::Fifo
            },
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gfx.device, &config);
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gfx.device.limits().max_texture_dimension_2d as usize),
        );
        Ok(Self {
            window,
            surface,
            config,
            state,
            ctx,
            painter: None,
            fitted: None,
        })
    }

    /// As large as its content (`wanted` points), up to most of a screen: a longer one scrolls.
    fn fit(&mut self, wanted: egui::Vec2) {
        const MAX: egui::Vec2 = egui::vec2(1000.0, 900.0);
        let size = wanted.max(egui::vec2(160.0, 60.0)).min(MAX).ceil();
        // the user resized it, or it was fitted already: changes of a few points are left
        if self
            .fitted
            .is_some_and(|fitted| (fitted - size).abs().max_elem() < 4.0)
        {
            return;
        }
        self.fitted = Some(size);
        let _ = self
            .window
            .request_inner_size(LogicalSize::new(f64::from(size.x), f64::from(size.y)));
    }

    fn resize(&mut self, gfx: &Gfx, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&gfx.device, &self.config);
    }

    /// Draw a frame of its interface.
    fn draw(&mut self, gfx: &Gfx, mut output: egui::FullOutput) {
        self.state
            .handle_platform_output(&self.window, std::mem::take(&mut output.platform_output));
        let surface_texture = match self.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&gfx.device, &self.config);
                return;
            }
            Err(_) => return,
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gfx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        // the panel covers it all: a clear, then the shapes
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("view"),
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
        }));
        let format = self.config.format;
        let painter = self
            .painter
            .get_or_insert_with(|| amsrust::ui::Painter::new(&gfx.device, format));
        painter.paint_shapes(
            &gfx.device,
            &gfx.queue,
            &mut encoder,
            &view,
            [self.config.width, self.config.height],
            &self.ctx,
            output.shapes,
            output.pixels_per_point,
            &output.textures_delta,
        );
        gfx.queue.submit([encoder.finish()]);
        surface_texture.present();
    }
}

struct Gfx {
    /// Kept for the surfaces of the debugger's windows.
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
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
            instance,
            adapter,
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
    fn draw(
        &mut self,
        mode: DisplayMode,
        again: bool,
        ui: Option<&mut UiLayer>,
    ) -> Result<bool, String> {
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

        // the picture, as large as the window allows below the menu bar, centred
        let top = ui.as_ref().map_or(0.0, |ui| ui.top_pixels);
        let (window_w, window_h) = (
            self.config.width as f32,
            (self.config.height as f32 - top).max(1.0),
        );
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
            pass.set_viewport(
                (window_w - w) / 2.0,
                top + (window_h - h) / 2.0,
                w,
                h,
                0.0,
                1.0,
            );
            self.blit.draw(&mut pass, bind_group);
        }
        // the menus and windows over it
        if let Some(ui) = ui {
            // a new frame of the interface (its texture updates once), else the last one again
            let textures = match ui.output.take() {
                Some(output) => {
                    ui.shapes = output.shapes;
                    ui.pixels_per_point = output.pixels_per_point;
                    output.textures_delta
                }
                None => egui::TexturesDelta::default(),
            };
            let painter = ui
                .painter
                .get_or_insert_with(|| amsrust::ui::Painter::new(&self.device, self.config.format));
            painter.paint_shapes(
                &self.device,
                &self.queue,
                &mut encoder,
                &view,
                [self.config.width, self.config.height],
                &ui.ctx,
                ui.shapes.clone(),
                ui.pixels_per_point,
                &textures,
            );
        }
        self.queue.submit([encoder.finish()]);
        surface_texture.present();
        self.shown = Some(mode);
        Ok(true)
    }
}

/// Whether `event` is the chord of a command with Ctrl or Alt (Ctrl+Shift+P, the palette):
/// the interface runs it, the CPC does not get the key.
fn is_ui_shortcut(event: &KeyEvent, modifiers: ModifiersState) -> bool {
    let (ctrl, alt, shift) = (
        modifiers.control_key() || modifiers.super_key(),
        modifiers.alt_key(),
        modifiers.shift_key(),
    );
    if !(ctrl || alt) {
        return false;
    }
    let Key::Character(text) = event.key_without_modifiers() else {
        return false;
    };
    amsrust::commands::all().iter().any(|spec| {
        spec.shortcut
            .and_then(amsrust::commands::Chord::parse)
            .is_some_and(|chord| {
                chord.ctrl == ctrl
                    && chord.alt == alt
                    && chord.shift == shift
                    && chord.key.eq_ignore_ascii_case(text.as_str())
            })
    })
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
    /// The keys pressed since the last poll, held or not by then (a press and its release
    /// between two frames still counts).
    pressed: Vec<HostKey>,
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
    ui: Option<UiLayer>,
    /// Fullscreen asked by a command (F11 is the window's): done on the next events.
    wanted_fullscreen: Option<bool>,
    /// The debugger's windows open, by view, and the ones to have (opened in the event loop).
    views: HashMap<u8, ViewWindow>,
    wanted_views: Vec<(u8, &'static str, egui::Context)>,
    /// The views whose window the user closed.
    closed_views: Vec<u8>,
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

impl App {
    /// Open the debugger's windows wanted, close the others.
    fn sync_views(&mut self, event_loop: &ActiveEventLoop) {
        let wanted = &self.wanted_views;
        self.views
            .retain(|key, _| wanted.iter().any(|(wanted, ..)| wanted == key));
        let Some(gfx) = &self.gfx else { return };
        for (key, title, ctx) in wanted {
            if self.views.contains_key(key) {
                continue;
            }
            match ViewWindow::open(event_loop, gfx, title, ctx.clone()) {
                Ok(view) => {
                    self.views.insert(*key, view);
                }
                Err(e) => {
                    eprintln!("{e}");
                    self.closed_views.push(*key);
                }
            }
        }
    }

    /// An event of a debugger's window: `true` when `id` is one.
    fn view_event(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        let Some((&key, view)) = self.views.iter_mut().find(|(_, v)| v.window.id() == id) else {
            return false;
        };
        let _ = view.state.on_window_event(&view.window, event);
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.views.remove(&key);
                self.wanted_views.retain(|(wanted, ..)| *wanted != key);
                self.closed_views.push(key);
            }
            WindowEvent::Resized(size) => {
                if let Some(gfx) = &self.gfx {
                    view.resize(gfx, size.width, size.height);
                }
            }
            _ => {}
        }
        true
    }
}

impl ApplicationHandler for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.sync_views(event_loop);
        if let Some(on) = self.wanted_fullscreen.take() {
            let now = self.fullscreen
                || self
                    .window
                    .as_ref()
                    .is_some_and(|window| window.fullscreen().is_some());
            if on != now {
                self.toggle_fullscreen(event_loop);
            }
        }
    }

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
                    Ok(gfx) => {
                        if let Some(ui) = &mut self.ui {
                            ui.state = Some(egui_winit::State::new(
                                ui.ctx.clone(),
                                egui::ViewportId::ROOT,
                                &window,
                                Some(window.scale_factor() as f32),
                                None,
                                Some(gfx.device.limits().max_texture_dimension_2d as usize),
                            ));
                        }
                        self.gfx = Some(gfx)
                    }
                    Err(e) => self.error = Some(e),
                }
                self.window = Some(window);
            }
            Err(e) => self.error = Some(format!("failed to open a window: {e}")),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // a debugger's window: its own interface, nothing for the CPC
        if self.view_event(id, &event) {
            return;
        }
        // the interface first: what it takes (a click on a menu, a key in a field) is not the CPC's
        if let (Some(ui), Some(window)) = (&mut self.ui, &self.window) {
            if let Some(state) = &mut ui.state {
                let response = state.on_window_event(window, &event);
                let typing = ui.ctx.wants_keyboard_input() || ui.ctx.is_popup_open();
                if let WindowEvent::KeyboardInput { event: key, .. } = &event {
                    if key.state == ElementState::Pressed
                        && (response.consumed || typing || is_ui_shortcut(key, self.modifiers))
                    {
                        return;
                    }
                }
            }
        }
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
                            self.pressed.push(key);
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
    /// Not at the real speed: the clock paces, not the vertical sync.
    speed_factor: bool,
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
    pub fn open(
        picture: (usize, usize),
        curvature: f32,
        ui: Option<egui::Context>,
    ) -> Result<Self, String> {
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
                pressed: Vec::new(),
                closed: false,
                error: None,
                fullscreen: false,
                modifiers: ModifiersState::empty(),
                refreshes_per_frame: None,
                wanted_fullscreen: None,
                views: HashMap::new(),
                wanted_views: Vec::new(),
                closed_views: Vec::new(),
                ui: ui.map(|ctx| UiLayer {
                    ctx,
                    state: None,
                    painter: None,
                    output: None,
                    shapes: Vec::new(),
                    pixels_per_point: 1.0,
                    top_pixels: 0.0,
                }),
            },
            frame_time: None,
            deadline: Instant::now(),
            speed_factor: false,
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
        // the vertical sync paces the real speed only: 25 % or 200 % go by the clock
        if self.app.refreshes_per_frame.is_some() && !self.speed_factor {
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

    fn set_speed_factor(&mut self, on: bool) {
        self.speed_factor = on;
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
        let mut down: Vec<HostKey> = self.app.held.values().copied().collect();
        for key in self.app.pressed.drain(..) {
            if !down.contains(&key) {
                down.push(key);
            }
        }
        HostInput {
            down,
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
        let gfx = self
            .app
            .gfx
            .as_mut()
            .ok_or_else(|| "no window".to_owned())?;
        let mut shown = gfx.draw(mode, false, self.app.ui.as_mut())?;
        // at 100 Hz (or more) the frame on each refresh of its own: the vertical sync paces 50
        // (the CRT's picture drawn by the first draw that was shown)
        let refreshes = if self.speed_factor {
            1
        } else {
            self.app.refreshes_per_frame.unwrap_or(1)
        };
        for _ in 1..refreshes {
            let gfx = self
                .app
                .gfx
                .as_mut()
                .ok_or_else(|| "no window".to_owned())?;
            shown |= gfx.draw(mode, shown, self.app.ui.as_mut())?;
        }
        self.pace();
        Ok(())
    }

    fn redraw(&mut self) -> Result<(), String> {
        let gfx = self
            .app
            .gfx
            .as_mut()
            .ok_or_else(|| "no window".to_owned())?;
        let mode = gfx.shown.unwrap_or_default();
        gfx.use_mode(mode);
        gfx.draw(mode, false, self.app.ui.as_mut()).map(|_| ())
    }

    fn ui_input(&mut self) -> Option<egui::RawInput> {
        let window = self.app.window.as_ref()?;
        Some(
            self.app
                .ui
                .as_mut()?
                .state
                .as_mut()?
                .take_egui_input(window),
        )
    }

    fn set_ui_output(&mut self, mut output: egui::FullOutput, menu_height: f32) {
        let (Some(ui), Some(window)) = (self.app.ui.as_mut(), self.app.window.as_ref()) else {
            return;
        };
        if let Some(state) = &mut ui.state {
            state.handle_platform_output(window, std::mem::take(&mut output.platform_output));
        }
        ui.top_pixels = menu_height * output.pixels_per_point;
        // textures not drawn yet are kept for the next draw
        if let Some(previous) = ui.output.take() {
            let mut textures = previous.textures_delta;
            textures.append(std::mem::take(&mut output.textures_delta));
            output.textures_delta = textures;
        }
        ui.output = Some(output);
    }

    fn has_view_windows(&self) -> bool {
        true
    }

    fn sync_view_windows(&mut self, views: &[amsrust::ui::NativeView]) -> Vec<u8> {
        let closed = std::mem::take(&mut self.app.closed_views);
        self.app.wanted_views = views
            .iter()
            .filter(|view| !closed.contains(&view.key))
            .map(|view| (view.key, view.title, view.ctx.clone()))
            .collect();
        closed
    }

    fn view_input(&mut self, key: u8) -> Option<egui::RawInput> {
        let view = self.app.views.get_mut(&key)?;
        Some(view.state.take_egui_input(&view.window))
    }

    fn set_view_output(&mut self, key: u8, output: egui::FullOutput, wanted: egui::Vec2) {
        if let (Some(view), Some(gfx)) = (self.app.views.get_mut(&key), self.app.gfx.as_ref()) {
            view.fit(wanted);
            view.draw(gfx, output);
        }
    }

    fn set_curvature(&mut self, curvature: f32) {
        if let Some(gfx) = &mut self.app.gfx {
            if gfx.curvature != curvature {
                gfx.curvature = curvature;
                // the CRT pass again, with its new glass
                gfx.crt = None;
            }
        }
    }

    fn clipboard_text(&mut self) -> Option<String> {
        self.app.ui.as_mut()?.state.as_mut()?.clipboard_text()
    }

    fn set_fullscreen(&mut self, on: bool) {
        if on != self.fullscreen() {
            self.app.wanted_fullscreen = Some(on);
            self.pump();
        }
    }

    fn fullscreen(&self) -> bool {
        self.app.fullscreen
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
