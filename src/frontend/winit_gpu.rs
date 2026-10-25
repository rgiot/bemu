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
//! F11 toggles fullscreen. When the monitor offers a video mode at its own size
//! and about 50 Hz (the CPC's rate), that mode is taken (exclusive fullscreen):
//! the presents then follow the screen's vertical sync (FIFO), one CPC frame per
//! refresh, and the frontend stops pacing on its own clock. Otherwise the window
//! goes borderless fullscreen at the desktop's rate; F11 again goes back to the
//! window. F4 closes the emulator. Neither is a CPC key.
//!
//! Keyboard: the keys held are the *logical* ones (what the layout says the
//! key is), as the minifb frontend reports them, with the characters typed:
//! see `amsrust::input` for how the two are combined.

use super::Frontend;
use amsrust::display::{wait_for, Blit, CrtParams, CrtPass, DisplayMode, FrameTexture, Warp, RAW_HEIGHT, RAW_WIDTH};
use amsrust::input::{HostInput, HostKey};
use image::RgbImage;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyLocation, NamedKey, PhysicalKey};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::monitor::VideoModeHandle;
use winit::window::{Fullscreen, Window, WindowId};

/// The window's initial width, in logical pixels.
const INITIAL_WIDTH: f64 = 1152.0;

/// The picture of a CRT display, in its own texture.
struct CrtState {
    mode: DisplayMode,
    pass: CrtPass,
    target: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (usize, usize)
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
    shown: Option<DisplayMode>
}

impl Gfx {
    fn new(window: Arc<Window>, curvature: f32) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window).map_err(|e| format!("cannot draw in the window: {e}"))?;
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
            desired_maximum_frame_latency: 2
        };
        surface.configure(&device, &config);

        let frame = FrameTexture::new(&device);
        let blit = Blit::new(&device, format);
        let raw_bind_group = blit.bind_group(&device, frame.view(), false, Warp::FLAT);
        Ok(Self { surface, device, queue, config, frame, blit, raw_bind_group, curvature, crt: None, shown: None })
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
        let pass = CrtPass::new(&self.device, &params, &self.frame, mode.warp(self.curvature));
        let target = CrtPass::create_target(&self.device, &params, wgpu::TextureUsages::TEXTURE_BINDING);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        // the CRT pass already bends the beam under the glass
        let bind_group = self.blit.bind_group(&self.device, &view, true, Warp::FLAT);
        self.crt = Some(CrtState { mode, pass, target, bind_group, size: params.output_size() });
    }

    /// Draw `mode`'s picture of the frame already uploaded.
    fn draw(&mut self, mode: DisplayMode) -> Result<(), String> {
        let surface_texture = match self.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            },
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(e) => return Err(format!("cannot draw in the window: {e}"))
        };
        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        let (bind_group, (picture_w, picture_h)) = match (mode, self.crt.as_ref()) {
            (DisplayMode::Crt(_), Some(crt)) => {
                crt.pass.encode(&mut encoder, &crt.target.create_view(&wgpu::TextureViewDescriptor::default()));
                (&crt.bind_group, crt.size)
            },
            _ => (&self.raw_bind_group, (RAW_WIDTH, RAW_HEIGHT))
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
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store }
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None
            });
            pass.set_viewport((window_w - w) / 2.0, (window_h - h) / 2.0, w, h, 0.0, 1.0);
            self.blit.draw(&mut pass, bind_group);
        }
        self.queue.submit([encoder.finish()]);
        surface_texture.present();
        self.shown = Some(mode);
        Ok(())
    }
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
            NamedKey::F7 => H::F7,
            NamedKey::F9 => H::F9,
            NamedKey::F6 => H::F6,
            NamedKey::F10 => H::F10,
            _ => return None
        }),
        Key::Character(text) => {
            let mut chars = text.chars();
            let (Some(c), None) = (chars.next(), chars.next())
            else {
                return None;
            };
            let numpad_digits = [
                H::NumPad0, H::NumPad1, H::NumPad2, H::NumPad3, H::NumPad4, H::NumPad5, H::NumPad6, H::NumPad7,
                H::NumPad8, H::NumPad9
            ];
            let digits = [H::Key0, H::Key1, H::Key2, H::Key3, H::Key4, H::Key5, H::Key6, H::Key7, H::Key8, H::Key9];
            let letters = [
                H::A, H::B, H::C, H::D, H::E, H::F, H::G, H::H, H::I, H::J, H::K, H::L, H::M, H::N, H::O, H::P, H::Q,
                H::R, H::S, H::T, H::U, H::V, H::W, H::X, H::Y, H::Z
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
                _ => return None
            })
        },
        _ => None
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
    typed: Vec<char>,
    closed: bool,
    error: Option<String>,
    /// F11 asked for fullscreen (kept here: the window manager may refuse it).
    fullscreen: bool,
    /// Fullscreen in a ~50 Hz video mode: the vertical sync paces the frames.
    vsync_paced: bool
}

/// A video mode of `window`'s monitor at its own size (or the largest) and within
/// half a hertz of 50, the deepest colours first.
fn fifty_hertz_mode(window: &Window) -> Option<VideoModeHandle> {
    let monitor = window.current_monitor()?;
    let size = monitor.size();
    monitor
        .video_modes()
        .filter(|mode| (49_500..=50_500).contains(&mode.refresh_rate_millihertz()))
        .max_by_key(|mode| (mode.size() == size, mode.size().width * mode.size().height, mode.bit_depth()))
}

impl App {
    /// F11: fullscreen in a 50 Hz mode when there is one, borderless otherwise; back
    /// to the window.
    fn toggle_fullscreen(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        self.fullscreen = !self.fullscreen;
        if !self.fullscreen {
            window.set_fullscreen(None);
            window.set_cursor_visible(true);
            self.vsync_paced = false;
            return;
        }
        match fifty_hertz_mode(window) {
            Some(mode) => {
                eprintln!(
                    "fullscreen {}x{} at {:.3} Hz",
                    mode.size().width,
                    mode.size().height,
                    f64::from(mode.refresh_rate_millihertz()) / 1000.0
                );
                window.set_fullscreen(Some(Fullscreen::Exclusive(mode)));
                self.vsync_paced = true;
            },
            None => {
                eprintln!("fullscreen: no 50 Hz video mode on this monitor, borderless at the desktop's rate");
                window.set_fullscreen(Some(Fullscreen::Borderless(None)));
                self.vsync_paced = false;
            }
        }
        window.set_cursor_visible(false);
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
                    Err(e) => self.error = Some(e)
                }
                self.window = Some(window);
            },
            Err(e) => self.error = Some(format!("failed to open a window: {e}"))
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.closed = true,
            WindowEvent::Resized(size) => {
                if let Some(gfx) = &mut self.gfx {
                    gfx.resize(size.width, size.height);
                }
            },
            WindowEvent::KeyboardInput { event, .. } => {
                match event.state {
                    // F4 closes the emulator
                    ElementState::Pressed if event.logical_key == Key::Named(NamedKey::F4) => self.closed = true,
                    ElementState::Pressed if event.logical_key == Key::Named(NamedKey::F11) => {
                        if !event.repeat {
                            self.toggle_fullscreen();
                        }
                    },
                    ElementState::Pressed => {
                        if let Some(key) = host_key(&event) {
                            self.held.insert(event.physical_key, key);
                        }
                        // what the layout makes of the key and the modifiers
                        if let Some(text) = &event.text {
                            self.typed.extend(text.chars().filter(|c| !c.is_control()));
                        }
                    },
                    ElementState::Released => {
                        self.held.remove(&event.physical_key);
                    }
                }
            },
            WindowEvent::Focused(false) => self.held.clear(),
            _ => {}
        }
    }
}

pub struct WinitGpuFrontend {
    event_loop: EventLoop<()>,
    app: App,
    /// Time between two presents, if capped.
    frame_time: Option<Duration>,
    deadline: Instant
}

impl WinitGpuFrontend {
    /// A window for pictures of `picture` pixels (for its shape and size).
    /// CRTs are seen through their glass, curved `curvature` times their bulge (0: flat).
    pub fn open(picture: (usize, usize), curvature: f32) -> Result<Self, String> {
        let event_loop = EventLoop::new().map_err(|e| format!("cannot start the event loop: {e}"))?;
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
                typed: Vec::new(),
                closed: false,
                error: None,
                fullscreen: false,
                vsync_paced: false
            },
            frame_time: None,
            deadline: Instant::now()
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
            None => Ok(frontend)
        }
    }

    fn pump(&mut self) {
        if let PumpStatus::Exit(_) = self.event_loop.pump_app_events(Some(Duration::ZERO), &mut self.app) {
            self.app.closed = true;
        }
    }

    /// Wait for the end of the frame when the rate is capped (not in a 50 Hz
    /// fullscreen mode: the vertical sync, FIFO, does it).
    fn pace(&mut self) {
        if self.app.vsync_paced {
            self.deadline = Instant::now();
            return;
        }
        if let Some(frame_time) = self.frame_time {
            self.deadline += frame_time;
            let now = Instant::now();
            if self.deadline > now {
                std::thread::sleep(self.deadline - now);
            }
            else {
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

    fn poll_input(&mut self) -> HostInput {
        self.pump();
        HostInput {
            down: self.app.held.values().copied().collect(),
            typed: std::mem::take(&mut self.app.typed)
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
        gfx.draw(mode)?;
        self.pace();
        Ok(())
    }

    fn redraw(&mut self) -> Result<(), String> {
        let gfx = self.gfx()?;
        let mode = gfx.shown.unwrap_or_default();
        gfx.use_mode(mode);
        gfx.draw(mode)
    }
}
