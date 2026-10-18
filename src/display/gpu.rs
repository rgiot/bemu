//! GPU renderer of the CRT model of [`super::crt`] (`--features crt-gpu`).
//!
//! The approach is inspired by the rendering source code of the ACE-DL
//! emulator (`CTM_Render.c`), see `crt.rs`; the shader, `crt.wgsl`, is the
//! same analytic model as the CPU's, evaluated per output pixel, so it works
//! at any output size and needs no precomputed table: the monitor's frame is
//! uploaded as a texture and one full-screen triangle draws the picture.
//!
//! [`GpuCrt`] is a [`DisplayFilter`] that reads the picture back, so it can
//! feed any frontend, the recorder and the screenshots; a frontend that
//! presents the texture itself would skip that read-back.

use super::crt::{CrtParams, Warp, SRC_HEIGHT, SRC_WIDTH};
#[cfg(not(target_arch = "wasm32"))]
use super::DisplayFilter;
use image::RgbImage;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    out_w: u32,
    out_h: u32,
    slot_px: u32,
    rows_per_line: u32,
    pitch: f32,
    fill_x: f32,
    fill_y: f32,
    corner: f32,
    stagger: f32,
    sigma_h: f32,
    sigma_v: f32,
    spill: f32,
    halation: f32,
    scale: f32,
    x_ratio: f32,
    pad: f32
}

impl Uniforms {
    fn new(p: &CrtParams) -> Self {
        let (out_w, out_h) = p.output_size();
        Self {
            out_w: out_w as u32,
            out_h: out_h as u32,
            slot_px: p.slot_px() as u32,
            rows_per_line: p.rows_per_line as u32,
            pitch: p.pitch_rows() as f32,
            fill_x: p.slot_fill.0,
            fill_y: p.slot_fill.1,
            corner: p.corner_radius,
            stagger: p.stagger,
            sigma_h: p.beam_sigma_h,
            sigma_v: p.beam_sigma_v,
            spill: p.spill,
            halation: p.halation,
            scale: p.gain * p.white_norm(),
            x_ratio: p.x_ratio(),
            pad: 0.0
        }
    }
}

/// The [`Warp`] of `blit.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WarpUniforms {
    k: [f32; 2],
    corner: f32,
    edged: f32
}

/// The last pass of a frontend drawing on the GPU (`blit.wgsl`): a picture, the monitor's frame
/// or a [`CrtPass`]'s, drawn over the viewport set, through the glass of a [`Warp`].
pub struct Blit {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler
}

impl Blit {
    /// A blit drawing into targets of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into())
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None
                    },
                    count: None
                }
            ]
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[]
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[]
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())]
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None
        });
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let (nearest, linear) = (sampler(wgpu::FilterMode::Nearest), sampler(wgpu::FilterMode::Linear));
        Self { pipeline, layout, nearest, linear }
    }

    /// What draws `view`: sampled linearly if `smooth` (a CRT's picture), else as is (the raw
    /// one), through `warp`.
    pub fn bind_group(&self, device: &wgpu::Device, view: &wgpu::TextureView, smooth: bool, warp: Warp) -> wgpu::BindGroup {
        let uniforms = WarpUniforms {
            k: [warp.k.0, warp.k.1],
            corner: warp.corner,
            edged: if warp.is_flat() { 0.0 } else { 1.0 }
        };
        let buffer = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor {
            label: Some("warp"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM
        });
        let sampler = if smooth { &self.linear } else { &self.nearest };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() }
            ]
        })
    }

    /// Draw the picture of `bind_group` over `pass`'s viewport.
    pub fn draw(&self, pass: &mut wgpu::RenderPass, bind_group: &wgpu::BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The monitor's frame as a texture, padded with black to the monitor's size.
pub struct FrameTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    rgba: Vec<u8>
}

impl FrameTexture {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("monitor frame"),
            size: wgpu::Extent3d {
                width: SRC_WIDTH as u32,
                height: SRC_HEIGHT as u32,
                depth_or_array_layers: 1
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[]
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view, rgba: vec![0; SRC_WIDTH * SRC_HEIGHT * 4] }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Upload `src` (any size up to the monitor's: the rest is black).
    pub fn upload(&mut self, queue: &wgpu::Queue, src: &RgbImage) {
        self.rgba.fill(0);
        let width = (src.width() as usize).min(SRC_WIDTH);
        for y in 0..(src.height() as usize).min(SRC_HEIGHT) {
            let row = &src.as_raw()[y * src.width() as usize * 3..][..width * 3];
            for (x, pixel) in row.chunks_exact(3).enumerate() {
                self.rgba[(y * SRC_WIDTH + x) * 4..][..3].copy_from_slice(pixel);
            }
        }
        queue.write_texture(
            self.texture.as_image_copy(),
            &self.rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SRC_WIDTH as u32 * 4), rows_per_image: None },
            wgpu::Extent3d { width: SRC_WIDTH as u32, height: SRC_HEIGHT as u32, depth_or_array_layers: 1 }
        );
    }
}

/// The shader pass of `crt.wgsl`: draws the CRT picture of a [`FrameTexture`]
/// into a `Rgba8Unorm` target of [`CrtParams::output_size`].
pub struct CrtPass {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup
}

impl CrtPass {
    pub fn new(device: &wgpu::Device, params: &CrtParams, frame: &FrameTexture) -> Self {
        let uniforms = Uniforms::new(params);
        let uniform_buffer = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor {
            label: Some("crt parameters"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crt"),
            source: wgpu::ShaderSource::Wgsl(include_str!("crt.wgsl").into())
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None
                    },
                    count: None
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false
                    },
                    count: None
                }
            ]
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(frame.view()) }
            ]
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[]
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crt"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[]
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())]
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None
        });
        Self { pipeline, bind_group }
    }

    /// A texture the pass can draw in, of the picture's size.
    pub fn create_target(device: &wgpu::Device, params: &CrtParams, extra: wgpu::TextureUsages) -> wgpu::Texture {
        let (w, h) = params.output_size();
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("crt picture"),
            size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | extra,
            view_formats: &[]
        })
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("crt"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store }
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The CRT filter on the GPU, with the picture read back (a [`DisplayFilter`]). Not in a web
/// page, where nothing waits for the GPU: there the page presents [`CrtPass`]'s target itself.
#[cfg(not(target_arch = "wasm32"))]
pub struct GpuCrt {
    params: CrtParams,
    device: wgpu::Device,
    queue: wgpu::Queue,
    frame: FrameTexture,
    pass: CrtPass,
    target: wgpu::Texture,
    readback: wgpu::Buffer,
    /// Bytes per row of `readback`, a multiple of wgpu's copy alignment.
    padded_row: usize
}

#[cfg(not(target_arch = "wasm32"))]
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

/// Wait for a future of the (native) wgpu API, which are ready at once.
#[cfg(not(target_arch = "wasm32"))]
pub fn wait_for<F: std::future::Future>(future: F) -> F::Output {
    block_on(future)
}

#[cfg(not(target_arch = "wasm32"))]
impl GpuCrt {
    /// Fails (with a message) when no usable adapter is available.
    pub fn new(params: CrtParams) -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| format!("cannot use the GPU: {e}"))?;

        let (out_w, out_h) = params.output_size();
        let frame = FrameTexture::new(&device);
        let pass = CrtPass::new(&device, &params, &frame);
        let target = CrtPass::create_target(&device, &params, wgpu::TextureUsages::COPY_SRC);
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let padded_row = (out_w * 4).div_ceil(align) * align;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read back"),
            size: (padded_row * out_h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false
        });
        Ok(Self { params, device, queue, frame, pass, target, readback, padded_row })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl DisplayFilter for GpuCrt {
    fn output_size(&self) -> (usize, usize) {
        self.params.output_size()
    }

    fn render(&mut self, src: &RgbImage, dst: &mut [u32]) {
        let (out_w, out_h) = self.params.output_size();
        assert_eq!(dst.len(), out_w * out_h);

        self.frame.upload(&self.queue, src);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.pass.encode(&mut encoder, &self.target.create_view(&wgpu::TextureViewDescriptor::default()));
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row as u32),
                    rows_per_image: None
                }
            },
            wgpu::Extent3d { width: out_w as u32, height: out_h as u32, depth_or_array_layers: 1 }
        );
        self.queue.submit([encoder.finish()]);

        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.expect("cannot map the read-back buffer"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU wait failed");
        {
            let data = slice.get_mapped_range();
            for (y, row) in dst.chunks_exact_mut(out_w).enumerate() {
                let bytes = &data[y * self.padded_row..][..out_w * 4];
                for (pixel, rgba) in row.iter_mut().zip(bytes.chunks_exact(4)) {
                    *pixel = (rgba[0] as u32) << 16 | (rgba[1] as u32) << 8 | rgba[2] as u32;
                }
            }
        }
        self.readback.unmap();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::display::{CpuCrt, CrtProfile};
    use image::Rgb;

    /// The shader is the CPU model evaluated per pixel: both must agree to
    /// within the rounding of their different `exp`/`sqrt`. Skipped when the
    /// machine has no GPU (or software Vulkan) adapter.
    #[test]
    fn gpu_matches_cpu() {
        let mut src = RgbImage::new(SRC_WIDTH as u32, SRC_HEIGHT as u32);
        for (x, y, pixel) in src.enumerate_pixels_mut() {
            *pixel = Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x / 3 + y) * 11 % 256) as u8]);
        }
        for profile in [CrtProfile::Ctm644, CrtProfile::Trinitron] {
            let params = profile.params();
            let mut gpu = match GpuCrt::new(params.clone()) {
                Ok(gpu) => gpu,
                Err(e) => {
                    eprintln!("skipped: {e}");
                    return;
                }
            };
            let mut cpu = CpuCrt::new(params);
            let (w, h) = cpu.output_size();
            let (mut a, mut b) = (vec![0; w * h], vec![0; w * h]);
            cpu.render(&src, &mut a);
            gpu.render(&src, &mut b);
            let mut worst = 0;
            let mut off = 0usize;
            for (p, q) in a.iter().zip(&b) {
                for shift in [0, 8, 16] {
                    let d = (((p >> shift) & 255) as i32 - ((q >> shift) & 255) as i32).abs();
                    worst = worst.max(d);
                    off += (d > 3) as usize;
                }
            }
            assert!(off < w * h / 1000, "{profile:?}: {off} channel values differ by more than 3 (worst {worst})");
        }
    }

    /// The blit's glass is the CPU's [`crate::display::Curved`]: the same pixels are lit.
    #[test]
    fn blit_warp_matches_cpu() {
        use crate::display::{Curved, RawFilter, RAW_HEIGHT, RAW_WIDTH};
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let Ok(adapter) = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut src = RgbImage::new(SRC_WIDTH as u32, SRC_HEIGHT as u32);
        src.pixels_mut().for_each(|pixel| *pixel = Rgb([200, 100, 50]));
        let warp = Warp::of(CrtProfile::Ctm644, 1.0);

        let mut frame = FrameTexture::new(&device);
        frame.upload(&queue, &src);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let blit = Blit::new(&device, format);
        let bind_group = blit.bind_group(&device, frame.view(), false, warp);
        let (w, h) = (RAW_WIDTH as u32, RAW_HEIGHT as u32);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[]
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (w * h * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
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
            blit.draw(&mut pass, &bind_group);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None }
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }
        );
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let gpu: Vec<u32> = slice
            .get_mapped_range()
            .chunks_exact(4)
            .map(|rgba| (rgba[0] as u32) << 16 | (rgba[1] as u32) << 8 | rgba[2] as u32)
            .collect();

        let mut cpu = vec![0; gpu.len()];
        Curved::new(Box::new(RawFilter), warp).render(&src, &mut cpu);
        assert_eq!(gpu[0], 0);
        assert_eq!(gpu[gpu.len() / 2 + w as usize / 2], 0xc8_64_32);
        let off = gpu
            .iter()
            .zip(&cpu)
            .filter(|(p, q)| (0..3).any(|c| ((*p >> (c * 8)) & 255).abs_diff((*q >> (c * 8)) & 255) > 24))
            .count();
        assert!(off < gpu.len() / 500, "{off} pixels differ");
    }
}
