//! Drawing in the app's `SurfaceView`: wgpu on Vulkan (GLES when it has none), the monitor's
//! picture drawn by `amsrust::display::Picture`, the very code of the desktop window and the web
//! page. The CRT (the CTM644's mask and glow) needs compute shaders: without them (GLES) the
//! picture is raw.
//!
//! The GPU (its device, CubeCL and the CRT's kernels, the picture) is made once, with the first
//! surface; the surface itself comes and goes with the app (in the background it is destroyed).

use amsrust::display::{DisplayMode, Picture, Warp};
use image::RgbImage;
use raw_window_handle::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};

type Window = std::ptr::NonNull<std::ffi::c_void>;

/// The storage buffers the CRT's kernels bind in a compute stage (the kernel `slots`: 8).
const CRT_STORAGE_BUFFERS: u32 = 8;

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    picture: Picture,
    /// Whether the CRT can be drawn (compute shaders).
    crt: bool,
}

struct Target {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

#[derive(Default)]
pub struct Gfx {
    gpu: Option<Gpu>,
    target: Option<Target>,
}

/// The surface of `window`.
///
/// # Safety
/// `window` must be a valid `ANativeWindow` that outlives the surface.
unsafe fn surface_of(instance: &wgpu::Instance, window: Window) -> Result<wgpu::Surface<'static>, String> {
    instance
        .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(window)),
        })
        .map_err(|e| format!("no surface: {e}"))
}

/// The surface's format for the picture: not sRGB (the picture is already encoded).
fn format_of(capabilities: &wgpu::SurfaceCapabilities) -> Option<wgpu::TextureFormat> {
    capabilities
        .formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .or_else(|| capabilities.formats.first().copied())
}

impl Gfx {
    /// Whether a surface is drawn in.
    pub fn attached(&self) -> bool {
        self.target.is_some()
    }

    /// Whether the CRT (CTM...) can be drawn, or only the raw picture.
    pub fn has_crt(&self) -> bool {
        self.gpu.as_ref().is_some_and(|gpu| gpu.crt)
    }

    pub fn diagnostics(&self) -> String {
        match (&self.gpu, &self.target) {
            (Some(gpu), Some(target)) => {
                let info = gpu.adapter.get_info();
                let picture = gpu.picture.renderer();
                format!(
                    "Graphics: {} ({:?}, {:?})\nSurface: {}x{}, {:?}\nCRT compute available: {}\nLast picture: {}",
                    info.name, info.backend, info.device_type,
                    target.config.width, target.config.height, target.config.present_mode, gpu.crt,
                    if picture.is_empty() { "not drawn yet" } else { picture }
                )
            }
            _ => "Graphics: surface unavailable".to_owned(),
        }
    }

    /// Draw in the window `window` (an `ANativeWindow*`), `size` pixels; the GPU made the first time.
    ///
    /// # Safety
    /// `window` must be a valid `ANativeWindow` that outlives the attachment ([`Self::detach`]).
    pub unsafe fn attach(&mut self, window: Window, size: (u32, u32)) -> Result<(), String> {
        self.target = None;
        if let Some(gpu) = &self.gpu {
            let surface = surface_of(&gpu.instance, window)?;
            let capabilities = surface.get_capabilities(&gpu.adapter);
            if format_of(&capabilities) == Some(gpu.format) {
                self.target = Some(Target::new(surface, &gpu.device, &capabilities, gpu.format, size));
                return Ok(());
            }
            // a surface the GPU made for the first one cannot draw in: everything made again
            self.gpu = None;
        }
        let mut last_error = String::new();
        for backends in [wgpu::Backends::VULKAN, wgpu::Backends::GL] {
            match Gpu::new(backends, window, size) {
                Ok((gpu, target)) => {
                    self.gpu = Some(gpu);
                    self.target = Some(target);
                    return Ok(());
                }
                Err(e) => {
                    log::warn!("{backends:?}: {e}");
                    last_error = e;
                }
            }
        }
        Err(last_error)
    }

    /// The surface is going: nothing drawn in it any more (the GPU stays).
    pub fn detach(&mut self) {
        self.target = None;
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        if let (Some(gpu), Some(target)) = (&self.gpu, &mut self.target) {
            let (width, height) = size;
            if width > 0 && height > 0 && (width, height) != (target.config.width, target.config.height) {
                target.config.width = width;
                target.config.height = height;
                target.surface.configure(&gpu.device, &target.config);
            }
        }
    }

    /// `frame` uploaded, then shown as `mode` through `warp`, letterboxed in the surface.
    pub fn show(&mut self, frame: Option<&RgbImage>, mode: DisplayMode, warp: Warp) -> Result<(), String> {
        let (Some(gpu), Some(target)) = (&mut self.gpu, &self.target) else {
            return Ok(());
        };
        let mode = if gpu.crt { mode } else { DisplayMode::Raw };
        if let Some(frame) = frame {
            gpu.picture.upload(&gpu.queue, frame);
        }
        let surface_texture = match target.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                target.surface.configure(&gpu.device, &target.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            other => return Err(format!("cannot draw: {other:?}")),
        };
        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.picture.draw(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &view,
            (target.config.width, target.config.height),
            (0.0, 0.0),
            mode,
            warp,
            frame.is_none(),
        );
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(surface_texture);
        Ok(())
    }
}

impl Gpu {
    /// The GPU of `backends`, and its first target, `window`.
    unsafe fn new(backends: wgpu::Backends, window: Window, size: (u32, u32)) -> Result<(Self, Target), String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = surface_of(&instance, window)?;
        let adapter = amsrust::display::wait_for(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("no adapter: {e}"))?;
        // the CRT's kernels bind up to 8 storage buffers in a compute stage: a GPU allowing fewer
        // (the emulator's SwiftShader: 4) shows the raw picture rather than kernels it refuses
        let supported = adapter.limits();
        let compute = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
            && supported.max_storage_buffers_per_shader_stage >= CRT_STORAGE_BUFFERS;
        if !compute {
            log::info!(
                "no CRT: {} storage buffers per shader stage, {CRT_STORAGE_BUFFERS} needed",
                supported.max_storage_buffers_per_shader_stage
            );
        }
        let request = |limits: wgpu::Limits| {
            amsrust::display::wait_for(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            }))
        };
        // all the GPU allows (the downlevel defaults allow 4 storage buffers: too few for the CRT)
        let (device, queue) = request(supported.clone())
            .or_else(|_| request(wgpu::Limits::downlevel_defaults().using_resolution(supported.clone())))
            .or_else(|_| request(wgpu::Limits::downlevel_webgl2_defaults().using_resolution(supported.clone())))
            .map_err(|e| format!("no device: {e}"))?;
        let compute = compute
            && device.limits().max_storage_buffers_per_shader_stage >= CRT_STORAGE_BUFFERS;

        let capabilities = surface.get_capabilities(&adapter);
        let format = format_of(&capabilities).ok_or("the surface has no format")?;
        let target = Target::new(surface, &device, &capabilities, format, size);

        let cube = if compute {
            match amsrust::display::GpuContext::new(&instance, &adapter, &device, &queue) {
                Ok(cube) => Some(cube),
                Err(e) => {
                    log::warn!("no CRT picture: {e}");
                    None
                }
            }
        } else {
            None
        };
        let crt = cube.is_some();
        let mut picture = Picture::new(&device, format, cube);
        let info = adapter.get_info();
        picture.adapter = format!("{} ({:?})", info.name, info.backend);
        log::info!("drawing with {} (CRT: {crt})", picture.adapter);
        let gpu = Self {
            instance,
            adapter,
            device,
            queue,
            format,
            picture,
            crt,
        };
        Ok((gpu, target))
    }
}

impl Target {
    fn new(
        surface: wgpu::Surface<'static>,
        device: &wgpu::Device,
        capabilities: &wgpu::SurfaceCapabilities,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        let present_mode = if capabilities.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::default(),
            desired_maximum_frame_latency: 2,
        };
        surface.configure(device, &config);
        Self { surface, config }
    }
}
