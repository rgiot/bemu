//! The monitor's picture drawn by the GPU in a window or a canvas: the one implementation of the
//! desktop's winit window and of the web page.
//!
//! A frame is uploaded as a texture; a CRT display has its shader pass draw the tube's picture in
//! a texture of its own, at the resolution the room asks for ([`PictureSize`]); the picture is
//! then drawn over a viewport of the target, centred, resized without moiré when it is not shown
//! at a whole factor. The frontend sets the surface, and draws its interface over the picture.

use image::RgbImage;

use super::crt::{CrtParams, Warp, SRC_HEIGHT, SRC_WIDTH};
use super::gpu::{Blit, CrtPass, FrameTexture, GpuContext, GpuFrame, Resampler};
use super::{DisplayFilter, DisplayMode, PictureSize, RAW_HEIGHT, RAW_WIDTH};

/// The most a CRT's picture is drawn larger than its profile's (a texture of 4320 x 3264).
const MAX_CRT_LEVEL: usize = 3;

/// The picture of a CRT display, in its own texture.
struct CrtState {
    mode: DisplayMode,
    warp: Warp,
    /// How many times larger than the profile's picture it is drawn (`CrtParams::scaled`).
    level: usize,
    pass: CrtPass,
    target: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (usize, usize),
}

/// The CRT on CubeCL's CPU backend (a page has none: it is the GPU or nothing).
#[cfg(not(target_arch = "wasm32"))]
fn cpu_filter(params: &CrtParams, warp: Warp) -> Result<Box<dyn DisplayFilter>, String> {
    super::KernelCrt::new(params.clone(), warp, super::Backend::Cpu).map(|f| Box::new(f) as Box<dyn DisplayFilter>)
}

#[cfg(target_arch = "wasm32")]
fn cpu_filter(_: &CrtParams, _: Warp) -> Result<Box<dyn DisplayFilter>, String> {
    Err("a page has no CPU rendering".into())
}

/// The picture of a CRT display made on the CPU (CubeCL's CPU backend), for when the hardware
/// acceleration is off: read back, then uploaded as a texture.
struct SoftState {
    mode: DisplayMode,
    warp: Warp,
    filter: Box<dyn DisplayFilter>,
    buffer: Vec<u32>,
    rgba: Vec<u8>,
    target: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (usize, usize),
    /// The picture is to be made again (a frame arrived, or the state is new).
    dirty: bool,
}

/// The picture's textures and passes, for the frames of one surface.
pub struct Picture {
    frame: FrameTexture,
    blit: Blit,
    raw_bind_group: wgpu::BindGroup,
    crt: Option<CrtState>,
    /// CubeCL on the device (the CRT's kernel), if it can run compute shaders: without it a CRT
    /// display shows the raw picture.
    cube: Option<GpuContext>,
    /// The last frame uploaded, as the CRT's kernels read it.
    gpu_frame: Option<GpuFrame>,
    /// A frame arrived (or a pass is new) since the CRT's picture was made: the picture is made
    /// again only then, so a redraw for the interface's sake costs the CRT nothing.
    crt_dirty: bool,
    /// The last draw showed the CRT (the raw frame's texture is then left alone).
    crt_shown: bool,
    /// The raw texture is behind the last frame (it was not uploaded while the CRT was shown).
    raw_stale: bool,
    /// The last frame, kept only while the CRT is shown, for the raw texture when it comes back.
    last_rgb: Option<RgbImage>,
    /// The picture resized without moiré (a CRT's at a fractional zoom, any below x1).
    resampler: Option<Resampler>,
    format: wgpu::TextureFormat,
    pub size: PictureSize,
    /// Hardware acceleration: the CRT is made by the GPU (default). Off, it is made on the CPU
    /// (CubeCL's CPU backend, where the build has one), slower but the same on every machine.
    pub hardware: bool,
    /// The GPU's name, for [`Self::renderer`].
    pub adapter: String,
    software: Option<SoftState>,
    /// The last frame, kept while the hardware acceleration is off.
    soft_frame: Option<RgbImage>,
    /// The CPU rendering cannot be had (no CPU backend in this build): the GPU draws.
    software_unavailable: bool,
    renderer: String,
}

impl Picture {
    /// A picture drawn in targets of `format`; `cube`: CubeCL on `device` (see [`GpuContext::new`]).
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        cube: Option<GpuContext>,
    ) -> Self {
        let gpu_frame = cube.as_ref().and_then(|ctx| match GpuFrame::new(ctx) {
            Ok(frame) => Some(frame),
            Err(e) => {
                eprintln!("no CRT picture: {e}");
                None
            }
        });
        let cube = cube.filter(|_| gpu_frame.is_some());
        let frame = FrameTexture::new(device);
        let blit = Blit::new(device, format);
        let raw_bind_group = blit.bind_group(device, frame.view(), false, Warp::FLAT);
        Self {
            frame,
            blit,
            raw_bind_group,
            crt: None,
            gpu_frame,
            crt_dirty: false,
            crt_shown: false,
            raw_stale: false,
            last_rgb: None,
            cube,
            resampler: None,
            format,
            size: PictureSize::default(),
            hardware: true,
            adapter: String::new(),
            software: None,
            soft_frame: None,
            software_unavailable: false,
            renderer: String::new(),
        }
    }

    /// What made the last picture drawn, in words for the interface ("GPU · name", "CPU
    /// (software)", the raw picture).
    pub fn renderer(&self) -> &str {
        &self.renderer
    }

    /// The CPU's picture of `mode` through `warp`, made if it is not the current one. Whether
    /// there is one (the hardware acceleration is off and the build can render on the CPU).
    fn use_software(&mut self, device: &wgpu::Device, mode: DisplayMode, warp: Warp) -> bool {
        let DisplayMode::Crt(profile) = mode else {
            return false;
        };
        if self.hardware || self.software_unavailable || cfg!(target_arch = "wasm32") {
            return false;
        }
        if self.software.as_ref().is_some_and(|s| s.mode == mode && s.warp == warp) {
            return true;
        }
        let params = profile.params();
        let filter = match cpu_filter(&params, warp) {
            Ok(filter) => filter,
            Err(e) => {
                eprintln!("no CRT on the CPU ({e}): the GPU draws it");
                self.software_unavailable = true;
                return false;
            }
        };
        let (w, h) = params.output_size();
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("crt picture (CPU)"),
            size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.blit.bind_group(device, &view, true, Warp::FLAT);
        self.software = Some(SoftState {
            mode,
            warp,
            filter,
            buffer: vec![0; w * h],
            rgba: [0, 0, 0, 255].repeat(w * h),
            target,
            bind_group,
            size: (w, h),
            dirty: true,
        });
        true
    }

    /// The monitor's next frame.
    pub fn upload(&mut self, queue: &wgpu::Queue, frame: &RgbImage) {
        // kept whatever is shown: a switch of the renderer (raw, GPU, CPU) draws the last frame
        match &mut self.last_rgb {
            Some(last) => last.clone_from(frame),
            None => self.last_rgb = Some(frame.clone()),
        }
        if self.crt_shown {
            // the raw texture is not drawn: its conversion waits for the switch back
            self.raw_stale = true;
        } else {
            self.frame.upload(queue, frame);
            self.raw_stale = false;
        }
        if !self.hardware && !self.software_unavailable && !cfg!(target_arch = "wasm32") {
            // the CPU makes the picture, from the frame as it is
            match &mut self.soft_frame {
                Some(last) => last.clone_from(frame),
                None => self.soft_frame = Some(frame.clone()),
            }
            if let Some(soft) = &mut self.software {
                soft.dirty = true;
            }
        }
        if let Some(gpu_frame) = &mut self.gpu_frame {
            if self.hardware || self.software_unavailable || self.software.is_none() {
                gpu_frame.upload(queue, frame);
                self.crt_dirty = true;
            }
        }
    }

    /// Make the CRT's pass for `mode` now, and run it once on a black frame: the kernels are
    /// compiled (WGSL, shader modules, pipelines) before the first frame that shows them, not in
    /// the middle of it. Nothing is drawn.
    pub fn prewarm(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mode: DisplayMode, warp: Warp) {
        if !matches!(mode, DisplayMode::Crt(_)) {
            return;
        }
        self.use_mode(device, queue, mode, 1, warp);
        if let (Some(crt), Some(frame)) = (self.crt.as_ref(), self.gpu_frame.as_ref()) {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            crt.pass.encode(&mut encoder, &crt.target.create_view(&wgpu::TextureViewDescriptor::default()), frame);
            queue.submit([encoder.finish()]);
            // the first real frame makes the picture again
            self.crt_dirty = true;
        }
    }

    /// How large a CRT's picture is drawn for `room`, in whole multiples of its profile's.
    fn level(&self, mode: DisplayMode, room: (f32, f32)) -> usize {
        let DisplayMode::Crt(profile) = mode else {
            return 1;
        };
        let (width, height) = profile.params().output_size();
        let fit = (room.0 / width as f32).min(room.1 / height as f32);
        match self.size {
            PictureSize::Whole => 1,
            PictureSize::Integer => (fit.floor() as usize).clamp(1, MAX_CRT_LEVEL),
            PictureSize::Maximal => (fit.ceil() as usize).clamp(1, MAX_CRT_LEVEL),
        }
    }

    /// Make `mode`'s pass the current one, drawn `level` times larger, through the glass `warp`.
    /// Whether it was made now (its picture is to be drawn).
    fn use_mode(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mode: DisplayMode, level: usize, warp: Warp) -> bool {
        let DisplayMode::Crt(profile) = mode else {
            return false;
        };
        let params: CrtParams = profile.params().scaled(level);
        if let Some(crt) = self.crt.as_mut().filter(|crt| crt.mode == mode && crt.level == level) {
            if crt.warp != warp {
                // only the glass changed: its four numbers are rewritten, nothing is rebuilt
                crt.pass.set_warp(queue, &params, warp);
                crt.warp = warp;
                self.crt_dirty = true;
            }
            return false;
        }
        let Some(cube) = &self.cube else {
            return false;
        };
        let pass = match CrtPass::new(cube, device, &params, warp) {
            Ok(pass) => pass,
            Err(e) => {
                eprintln!("no CRT picture: {e}");
                self.cube = None;
                return false;
            }
        };
        let target = CrtPass::create_target(device, &params, wgpu::TextureUsages::TEXTURE_BINDING);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        // the CRT pass already bends the beam under the glass
        let bind_group = self.blit.bind_group(device, &view, true, Warp::FLAT);
        self.crt = Some(CrtState {
            mode,
            warp,
            level,
            pass,
            target,
            bind_group,
            size: params.output_size(),
        });
        self.crt_dirty = true;
        true
    }

    /// Draw `mode`'s picture of the frame uploaded, over `view` (`target` pixels) between the bars
    /// (`bars`: the pixels the interface keeps above and below it), through the glass `warp`.
    /// `again`: the same frame shown on one more refresh, the CRT's picture of it already drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        target: (u32, u32),
        bars: (f32, f32),
        mode: DisplayMode,
        warp: Warp,
        again: bool,
    ) {
        let (top, bottom) = bars;
        let (window_w, window_h) = (target.0 as f32, (target.1 as f32 - top - bottom).max(1.0));
        let level = self.level(mode, (window_w, window_h));
        let _ = again; // what changed is tracked here (`crt_dirty`), whoever calls
        // the GPU takes over again: the frame it was not given while the CPU drew
        if self.hardware {
            if let (Some(frame), Some(gpu_frame)) = (self.soft_frame.take(), self.gpu_frame.as_mut()) {
                gpu_frame.upload(queue, &frame);
                self.crt_dirty = true;
            }
        }
        let soft = self.use_software(device, mode, warp);
        // a page has no CPU rendering: with the acceleration off it shows the raw picture
        let raw_only = !soft && !self.hardware && cfg!(target_arch = "wasm32") && matches!(mode, DisplayMode::Crt(_));
        if !soft && !raw_only {
            self.use_mode(device, queue, mode, level, warp);
        }
        let gpu_crt = !soft && !raw_only && matches!(mode, DisplayMode::Crt(_)) && self.crt.is_some();
        if soft {
            if let (Some(state), Some(frame)) = (self.software.as_mut(), self.soft_frame.as_ref().or(self.last_rgb.as_ref())) {
                if state.dirty {
                    state.filter.render(frame, &mut state.buffer);
                    for (rgba, word) in state.rgba.chunks_exact_mut(4).zip(&state.buffer) {
                        rgba[0] = (word >> 16) as u8;
                        rgba[1] = (word >> 8) as u8;
                        rgba[2] = *word as u8;
                    }
                    queue.write_texture(
                        state.target.as_image_copy(),
                        &state.rgba,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(state.size.0 as u32 * 4),
                            rows_per_image: Some(state.size.1 as u32),
                        },
                        wgpu::Extent3d {
                            width: state.size.0 as u32,
                            height: state.size.1 as u32,
                            depth_or_array_layers: 1,
                        },
                    );
                    state.dirty = false;
                }
            }
        }
        if let (true, Some(crt)) = (gpu_crt, self.crt.as_ref()) {
            if let (true, Some(frame)) = (self.crt_dirty, self.gpu_frame.as_ref()) {
                crt.pass.encode(
                    encoder,
                    &crt.target.create_view(&wgpu::TextureViewDescriptor::default()),
                    frame,
                );
                self.crt_dirty = false;
            }
        }
        let crt_shown = gpu_crt || soft;
        // the raw picture comes back: its texture catches up with the last frame
        if !crt_shown && self.raw_stale {
            if let Some(last) = &self.last_rgb {
                self.frame.upload(queue, last);
            }
            self.raw_stale = false;
        }
        self.crt_shown = crt_shown;
        // the CRT's picture, whoever made it: its view, its bind group, its size
        let shown: Option<(wgpu::TextureView, wgpu::BindGroup, (usize, usize))> = if soft {
            self.software.as_ref().map(|s| {
                (s.target.create_view(&wgpu::TextureViewDescriptor::default()), s.bind_group.clone(), s.size)
            })
        } else if gpu_crt {
            self.crt.as_ref().map(|c| {
                (c.target.create_view(&wgpu::TextureViewDescriptor::default()), c.bind_group.clone(), c.size)
            })
        } else {
            None
        };
        self.renderer = if soft {
            "CPU (software rendering)".to_owned()
        } else if gpu_crt {
            let unavailable = if self.software_unavailable && !self.hardware {
                " (software rendering unavailable in this build)"
            } else {
                ""
            };
            if self.adapter.is_empty() {
                format!("GPU{unavailable}")
            } else {
                format!("GPU · {}{unavailable}", self.adapter)
            }
        } else if raw_only {
            "raw picture (GPU acceleration is off)".to_owned()
        } else if matches!(mode, DisplayMode::Crt(_)) {
            "raw picture (the GPU cannot run the CRT)".to_owned()
        } else {
            "raw picture".to_owned()
        };
        let (picture_w, picture_h) = match &shown {
            Some((_, _, size)) => *size,
            None => (RAW_WIDTH, RAW_HEIGHT),
        };
        let scale = picture_scale(
            (window_w, window_h),
            (picture_w as f32, picture_h as f32),
            self.size == PictureSize::Maximal,
        );
        // whole pixels: the picture's pixels fall on the target's
        let (w, h) = (
            (picture_w as f32 * scale).round(),
            (picture_h as f32 * scale).round(),
        );
        // shrunk, or a CRT's scanlines and mask at a fractional zoom: the website's resampling
        // (the mask's period below a pixel beats into stripes otherwise)
        let whole = scale >= 1.0 && (scale - scale.round()).abs() < 1e-4;
        let resampled = scale < 1.0 || (crt_shown && !whole);
        if resampled {
            let (source, size) = match &shown {
                Some((view, _, size)) => (view.clone(), (size.0 as u32, size.1 as u32)),
                None => (
                    self.frame.view().clone(),
                    (SRC_WIDTH as u32, SRC_HEIGHT as u32),
                ),
            };
            let format = self.format;
            self.resampler
                .get_or_insert_with(|| Resampler::new(device, format))
                .prepare(device, queue, encoder, &source, size, (w as u32, h as u32));
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("picture"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
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
            multiview_mask: None,
        });
        pass.set_viewport(
            ((window_w - w) / 2.0).floor(),
            (top + (window_h - h) / 2.0).floor(),
            w,
            h,
            0.0,
            1.0,
        );
        match (&self.resampler, resampled) {
            (Some(resampler), true) => resampler.draw(&mut pass),
            _ => {
                let bind_group = match &shown {
                    Some((_, bind_group, _)) => bind_group,
                    None => &self.raw_bind_group,
                };
                self.blit.draw(&mut pass, bind_group)
            }
        }
    }
}

/// How much the picture is zoomed in a room: the largest factor that fits, a whole one unless
/// `free`. A room smaller than the picture shrinks it by any factor (the filter averages then).
pub fn picture_scale(room: (f32, f32), picture: (f32, f32), free: bool) -> f32 {
    let fit = (room.0 / picture.0).min(room.1 / picture.1);
    if free || fit < 1.0 {
        fit
    } else {
        fit.floor()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_scale_is_whole_unless_free_or_shrunk() {
        let picture = (768.0, 544.0);
        assert_eq!(picture_scale((1920.0, 1080.0), picture, false), 1.0);
        assert!((picture_scale((1920.0, 1080.0), picture, true) - 1080.0 / 544.0).abs() < 1e-4);
        assert_eq!(picture_scale((3840.0, 2160.0), picture, false), 3.0);
        assert_eq!(picture_scale((384.0, 272.0), picture, false), 0.5);
    }
}
