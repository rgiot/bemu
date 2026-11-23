//! Drawing in the page's canvas: WebGPU or WebGL when available, Canvas 2D otherwise.
//!
//! On WebGPU the monitor's frame is uploaded as a texture and, for a CRT display, the shader
//! pass of `amsrust::display::CrtPass` draws the tube's picture in a texture of its own size; a
//! blit scales the picture, letterboxed, to the canvas (as the desktop's `--frontend winit`
//! window does). Canvas 2D shows the raw picture only.

use amsrust::display::{DisplayMode, Picture, PictureSize, RAW_HEIGHT, RAW_WIDTH};
use image::RgbImage;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData, OffscreenCanvas, OffscreenCanvasRenderingContext2d};

use crate::scanlines::double_scanlines;

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// The monitor's picture: the very code of the desktop window.
    picture: Picture,
    painter: Option<amsrust::ui::Painter>
}

/// The page's canvas, or the one a page handed to a worker.
pub enum Canvas {
    Element(HtmlCanvasElement),
    Offscreen(OffscreenCanvas)
}

impl Canvas {
    /// From a `<canvas>` or an `OffscreenCanvas`.
    pub fn from_js(value: wasm_bindgen::JsValue) -> Result<Self, String> {
        match value.dyn_into::<HtmlCanvasElement>() {
            Ok(element) => Ok(Self::Element(element)),
            Err(value) => value
                .dyn_into::<OffscreenCanvas>()
                .map(Self::Offscreen)
                .map_err(|_| "expected a canvas or an OffscreenCanvas".to_owned())
        }
    }

    fn size(&self) -> (u32, u32) {
        match self {
            Self::Element(canvas) => (canvas.width(), canvas.height()),
            Self::Offscreen(canvas) => (canvas.width(), canvas.height())
        }
    }
}

enum Context2d {
    Element(CanvasRenderingContext2d),
    Offscreen(OffscreenCanvasRenderingContext2d)
}

/// The raw picture on Canvas 2D: each scanline twice, RGBA.
pub struct Flat {
    context: Context2d,
    source: Canvas,
    source_context: Context2d,
    size: (u32, u32),
    rgba: Vec<u8>
}

pub enum Gfx {
    Gpu(Box<Gpu>),
    Flat(Flat)
}

impl Gfx {
    /// WebGPU if `webgpu`, else Canvas 2D. A canvas WebGPU failed on has no 2D context any more:
    /// the page tries again with a new one.
    pub async fn new(canvas: Canvas, webgpu: bool) -> Result<Self, String> {
        if webgpu {
            return Ok(Self::Gpu(Box::new(Gpu::new(canvas).await?)));
        }
        let size = (canvas.size().0.max(1), canvas.size().1.max(1));
        let (context, source, source_context) = match canvas {
            Canvas::Element(canvas) => {
                let context = canvas.get_context("2d").map_err(|e| format!("no 2D context: {e:?}"))?
                    .ok_or("no 2D context")?.dyn_into::<CanvasRenderingContext2d>()
                    .map_err(|_| "no 2D context")?;
                let source = web_sys::window().and_then(|window| window.document())
                    .ok_or("no document for 2D canvas")?.create_element("canvas")
                    .map_err(|e| format!("cannot create 2D canvas: {e:?}"))?
                    .dyn_into::<HtmlCanvasElement>().map_err(|_| "cannot create 2D canvas")?;
                source.set_width(RAW_WIDTH as u32);
                source.set_height(RAW_HEIGHT as u32);
                let source_context = source.get_context("2d").map_err(|e| format!("no 2D source context: {e:?}"))?
                    .ok_or("no 2D source context")?.dyn_into::<CanvasRenderingContext2d>()
                    .map_err(|_| "no 2D source context")?;
                (Context2d::Element(context), Canvas::Element(source), Context2d::Element(source_context))
            },
            Canvas::Offscreen(canvas) => {
                let context = canvas.get_context("2d").map_err(|e| format!("no 2D context: {e:?}"))?
                    .ok_or("no 2D context")?.dyn_into::<OffscreenCanvasRenderingContext2d>()
                    .map_err(|_| "no 2D context")?;
                let source = OffscreenCanvas::new(RAW_WIDTH as u32, RAW_HEIGHT as u32)
                    .map_err(|e| format!("cannot create 2D canvas: {e:?}"))?;
                let source_context = source.get_context("2d").map_err(|e| format!("no 2D source context: {e:?}"))?
                    .ok_or("no 2D source context")?.dyn_into::<OffscreenCanvasRenderingContext2d>()
                    .map_err(|_| "no 2D source context")?;
                (Context2d::Offscreen(context), Canvas::Offscreen(source), Context2d::Offscreen(source_context))
            }
        };
        Ok(Self::Flat(Flat { context, source, source_context, size, rgba: vec![255; RAW_WIDTH * RAW_HEIGHT * 4] }))
    }

    /// Compile the CRT's kernels now (a page that starts on a CRT display), not in the middle of
    /// its first frame.
    pub fn prewarm(&mut self, mode: DisplayMode, curvature: f32) {
        if let Self::Gpu(gpu) = self {
            gpu.picture.prewarm(&gpu.device, &gpu.queue, mode, mode.warp(curvature));
        }
    }

    /// How large the picture is shown in the canvas.
    pub fn set_picture_size(&mut self, size: PictureSize) {
        if let Self::Gpu(gpu) = self {
            gpu.picture.size = size;
        }
    }

    /// Hardware acceleration of the CRT (Video > GPU acceleration): off, a page shows the raw picture.
    pub fn set_hardware(&mut self, on: bool) {
        if let Self::Gpu(gpu) = self {
            gpu.picture.hardware = on;
        }
    }

    /// What made the last picture, for the status bar ("GPU · name (WebGPU)", the raw picture).
    pub fn renderer(&self) -> String {
        match self {
            Self::Gpu(gpu) => gpu.picture.renderer().to_owned(),
            Self::Flat(_) => "Canvas 2D (raw picture, no GPU)".to_owned(),
        }
    }

    /// "webgpu" or "canvas2d".
    pub fn backend(&self) -> &'static str {
        match self {
            Self::Gpu(_) => "webgpu",
            Self::Flat(_) => "canvas2d"
        }
    }

    /// The canvas's drawing buffer is now `width` x `height`.
    pub fn resize(&mut self, width: u32, height: u32) {
        match self {
            Self::Gpu(gpu) => gpu.resize(width, height),
            Self::Flat(flat) => flat.size = (width, height)
        }
    }

    /// Show `frame` (the monitor's 768x272 picture) as `mode` says, with CRT effects on the GPU.
    pub fn show(&mut self, frame: &RgbImage, mode: DisplayMode, curvature: f32) -> Result<(), String> {
        self.show_with_ui(frame, mode, curvature, None).map(drop)
    }

    /// [`Self::show`], the interface drawn over the picture (on the GPU), the picture below its
    /// menu bar.
    pub fn show_with_ui(
        &mut self,
        frame: &RgbImage,
        mode: DisplayMode,
        curvature: f32,
        ui: Option<(&amsrust::ui::Gui, egui::FullOutput)>
    ) -> Result<Option<egui::FullOutput>, String> {
        match self {
            Self::Gpu(gpu) => {
                gpu.picture.upload(&gpu.queue, frame);
                gpu.draw(mode, mode.warp(curvature), ui)
            },
            Self::Flat(flat) => flat.show(frame).map(|()| ui.map(|(_, output)| output))
        }
    }

    /// The canvas's drawing buffer's size.
    pub fn size(&self) -> (u32, u32) {
        match self {
            Self::Gpu(gpu) => (gpu.config.width, gpu.config.height),
            Self::Flat(flat) => flat.size
        }
    }
}

impl Flat {
    fn show(&mut self, frame: &RgbImage) -> Result<(), String> {
        double_scanlines(frame, &mut self.rgba);
        let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&self.rgba), RAW_WIDTH as u32, RAW_HEIGHT as u32)
            .map_err(|e| format!("{e:?}"))?;
        match &self.source_context {
            Context2d::Element(context) => context.put_image_data(&image, 0.0, 0.0),
            Context2d::Offscreen(context) => context.put_image_data(&image, 0.0, 0.0)
        }
        .map_err(|e| format!("{e:?}"))?;
        let (x, y, w, h, _) = crate::viewport::fit(self.size, (RAW_WIDTH, RAW_HEIGHT), true);
        match (&self.context, &self.source) {
            (Context2d::Element(context), Canvas::Element(source)) => {
                context.clear_rect(0.0, 0.0, self.size.0 as f64, self.size.1 as f64);
                context.set_image_smoothing_enabled(false);
                context.draw_image_with_html_canvas_element_and_dw_and_dh(source, x as f64, y as f64, w as f64, h as f64)
            },
            (Context2d::Offscreen(context), Canvas::Offscreen(source)) => {
                context.clear_rect(0.0, 0.0, self.size.0 as f64, self.size.1 as f64);
                context.set_image_smoothing_enabled(false);
                context.draw_image_with_offscreen_canvas_and_dw_and_dh(source, x as f64, y as f64, w as f64, h as f64)
            },
            _ => unreachable!("2D source and destination must use the same canvas type")
        }.map_err(|e| format!("cannot draw 2D frame: {e:?}"))
    }
}

impl Gpu {
    async fn new(canvas: Canvas) -> Result<Self, String> {
        let (width, height) = (canvas.size().0.max(1), canvas.size().1.max(1));
        let webgpu = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // wgpu chooses WebGPU when navigator.gpu exists, even if requestAdapter returns null.
        // Probe before creating a surface so a failed WebGPU context cannot lock the canvas.
        let instance = if webgpu.request_adapter(&wgpu::RequestAdapterOptions::default()).await.is_ok() {
            webgpu
        } else {
            wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::GL,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            })
        };
        let target = match canvas {
            Canvas::Element(canvas) => wgpu::SurfaceTarget::Canvas(canvas),
            Canvas::Offscreen(canvas) => wgpu::SurfaceTarget::OffscreenCanvas(canvas)
        };
        let surface = instance
            .create_surface(target)
            .map_err(|e| format!("no WebGPU canvas: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no WebGPU adapter: {e}"))?;
        // The CRT's kernel needs compute shaders and storage buffers: the limits of WebGPU, not of
        // the WebGL fallback (where the picture stays raw).
        let webgpu = adapter.get_info().backend == wgpu::Backend::BrowserWebGpu;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: if webgpu {
                    wgpu::Limits::default().using_resolution(adapter.limits())
                } else {
                    wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
                },
                ..Default::default()
            })
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
            color_space: wgpu::SurfaceColorSpace::default(),
            desired_maximum_frame_latency: 2
        };
        surface.configure(&device, &config);

        let cube = if webgpu {
            match amsrust::display::GpuContext::new(&instance, &adapter, &device, &queue) {
                Ok(cube) => Some(cube),
                Err(e) => {
                    web_sys::console::warn_1(&format!("no CRT picture: {e}").into());
                    None
                }
            }
        } else {
            None
        };
        let mut picture = Picture::new(&device, format, cube);
        let info = adapter.get_info();
        picture.adapter = if info.name.is_empty() { "WebGPU".to_owned() } else { format!("{} (WebGPU)", info.name) };
        Ok(Self { surface, device, queue, config, picture, painter: None })
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 && (width, height) != (self.config.width, self.config.height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Draw `mode`'s picture of the frame already uploaded, and the interface over it. The
    /// interface's output comes back when it could not be drawn (its texture updates are kept).
    fn draw(
        &mut self,
        mode: DisplayMode,
        warp: amsrust::display::Warp,
        ui: Option<(&amsrust::ui::Gui, egui::FullOutput)>
    ) -> Result<Option<egui::FullOutput>, String> {
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(ui.map(|(_, output)| output));
            },
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(ui.map(|(_, output)| output))
            },
            other => return Err(format!("cannot draw in the canvas: {other:?}"))
        };
        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        // below the menu bar, above the status bar
        let bars = ui.as_ref().map_or((0.0, 0.0), |(gui, output)| {
            (gui.menu_height() * output.pixels_per_point, gui.status_height() * output.pixels_per_point)
        });
        self.picture.draw(
            &self.device,
            &self.queue,
            &mut encoder,
            &view,
            (self.config.width, self.config.height),
            bars,
            mode,
            warp,
            false
        );
        if let Some((gui, output)) = ui {
            let painter = self
                .painter
                .get_or_insert_with(|| amsrust::ui::Painter::new(&self.device, self.config.format));
            painter.paint(&self.device, &self.queue, &mut encoder, &view, [self.config.width, self.config.height], gui.context(), output);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(surface_texture);
        Ok(None)
    }
}
