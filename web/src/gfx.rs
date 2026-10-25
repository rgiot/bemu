//! Drawing in the page's canvas: WebGPU when the browser has it, Canvas 2D otherwise.
//!
//! On WebGPU the monitor's frame is uploaded as a texture and, for a CRT display, the shader
//! pass of `amsrust::display::CrtPass` draws the tube's picture in a texture of its own size; a
//! blit scales the picture, letterboxed, to the canvas (as the desktop's `--frontend winit`
//! window does). Canvas 2D shows the raw picture only.

use amsrust::display::{Blit, CrtParams, CrtPass, DisplayMode, FrameTexture, Warp, RAW_HEIGHT, RAW_WIDTH};
use image::RgbImage;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

/// The picture of a CRT display, in its own texture.
struct CrtState {
    mode: DisplayMode,
    warp: Warp,
    pass: CrtPass,
    target: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (usize, usize)
}

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    frame: FrameTexture,
    blit: Blit,
    raw_bind_group: wgpu::BindGroup,
    crt: Option<CrtState>
}

/// The raw picture on Canvas 2D: each scanline twice, RGBA.
pub struct Flat {
    context: CanvasRenderingContext2d,
    rgba: Vec<u8>
}

pub enum Gfx {
    Gpu(Box<Gpu>),
    Flat(Flat)
}

impl Gfx {
    /// WebGPU if `webgpu`, else Canvas 2D. A canvas WebGPU failed on has no 2D context any more:
    /// the page tries again with a new one.
    pub async fn new(canvas: HtmlCanvasElement, webgpu: bool) -> Result<Self, String> {
        if webgpu {
            return Ok(Self::Gpu(Box::new(Gpu::new(canvas).await?)));
        }
        canvas.set_width(RAW_WIDTH as u32);
        canvas.set_height(RAW_HEIGHT as u32);
        let context = canvas
            .get_context("2d")
            .map_err(|e| format!("no 2D context: {e:?}"))?
            .ok_or("no 2D context")?
            .dyn_into::<CanvasRenderingContext2d>()
            .map_err(|_| "no 2D context")?;
        Ok(Self::Flat(Flat { context, rgba: vec![255; RAW_WIDTH * RAW_HEIGHT * 4] }))
    }

    /// "webgpu" or "canvas2d".
    pub fn backend(&self) -> &'static str {
        match self {
            Self::Gpu(_) => "webgpu",
            Self::Flat(_) => "canvas2d"
        }
    }

    /// The canvas's drawing buffer is now `width` x `height` (WebGPU only: Canvas 2D keeps the
    /// raw picture's size, the page scales the element).
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Self::Gpu(gpu) = self {
            gpu.resize(width, height);
        }
    }

    /// Show `frame` (the monitor's 768x272 picture) as `mode` says, a CRT through its glass
    /// curved `curvature` times its bulge (WebGPU only).
    pub fn show(&mut self, frame: &RgbImage, mode: DisplayMode, curvature: f32) -> Result<(), String> {
        match self {
            Self::Gpu(gpu) => {
                gpu.use_mode(mode, mode.warp(curvature));
                gpu.frame.upload(&gpu.queue, frame);
                gpu.draw(mode)
            },
            Self::Flat(flat) => flat.show(frame)
        }
    }
}

impl Flat {
    fn show(&mut self, frame: &RgbImage) -> Result<(), String> {
        let (width, height) = (frame.width() as usize, frame.height() as usize);
        let raw = frame.as_raw();
        for y in 0..RAW_HEIGHT {
            let row = &mut self.rgba[y * RAW_WIDTH * 4..(y + 1) * RAW_WIDTH * 4];
            let src = y / 2;
            if src >= height {
                row.chunks_exact_mut(4).for_each(|pixel| pixel[..3].fill(0));
                continue;
            }
            let line = &raw[src * width * 3..(src + 1) * width * 3];
            for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                match line.get(x * 3..x * 3 + 3) {
                    Some(rgb) => pixel[..3].copy_from_slice(rgb),
                    None => pixel[..3].fill(0)
                }
            }
        }
        let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&self.rgba), RAW_WIDTH as u32, RAW_HEIGHT as u32)
            .map_err(|e| format!("{e:?}"))?;
        self.context.put_image_data(&image, 0.0, 0.0).map_err(|e| format!("{e:?}"))
    }
}

impl Gpu {
    async fn new(canvas: HtmlCanvasElement) -> Result<Self, String> {
        let (width, height) = (canvas.width().max(1), canvas.height().max(1));
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..Default::default()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|e| format!("no WebGPU canvas: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no WebGPU adapter: {e}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|e| format!("cannot use WebGPU: {e}"))?;

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
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2
        };
        surface.configure(&device, &config);

        let frame = FrameTexture::new(&device);
        let blit = Blit::new(&device, format);
        let raw_bind_group = blit.bind_group(&device, frame.view(), false, Warp::FLAT);
        Ok(Self { surface, device, queue, config, frame, blit, raw_bind_group, crt: None })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 && (width, height) != (self.config.width, self.config.height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Make `mode`'s pass the current one.
    fn use_mode(&mut self, mode: DisplayMode, warp: Warp) {
        let DisplayMode::Crt(profile) = mode else {
            return;
        };
        if self.crt.as_ref().is_some_and(|crt| crt.mode == mode && crt.warp == warp) {
            return;
        }
        let params: CrtParams = profile.params();
        let pass = CrtPass::new(&self.device, &params, &self.frame, warp);
        let target = CrtPass::create_target(&self.device, &params, wgpu::TextureUsages::TEXTURE_BINDING);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        // the CRT pass already bends the beam under the glass
        let bind_group = self.blit.bind_group(&self.device, &view, true, Warp::FLAT);
        self.crt = Some(CrtState { mode, warp, pass, target, bind_group, size: params.output_size() });
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
            Err(e) => return Err(format!("cannot draw in the canvas: {e}"))
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

        // the picture, as large as the canvas allows, centred
        let (canvas_w, canvas_h) = (self.config.width as f32, self.config.height as f32);
        let scale = (canvas_w / picture_w as f32).min(canvas_h / picture_h as f32);
        let (w, h) = (picture_w as f32 * scale, picture_h as f32 * scale);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("canvas"),
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
            pass.set_viewport((canvas_w - w) / 2.0, (canvas_h - h) / 2.0, w, h, 0.0, 1.0);
            self.blit.draw(&mut pass, bind_group);
        }
        self.queue.submit([encoder.finish()]);
        surface_texture.present();
        Ok(())
    }
}

