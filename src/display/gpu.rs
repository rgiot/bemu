//! GPU renderer of the CRT model of [`super::crt`] (`--features crt-gpu`).
//!
//! The approach is inspired by the rendering source code of the ACE-DL
//! emulator (`CTM_Render.c`), see `crt.rs`; the model is the same analytic one as the CPU's,
//! evaluated per output pixel by a CubeCL kernel (`kernel.rs`: Vulkan, Metal, DX12 and the
//! browser's WebGPU from one Rust source), so it works at any output size and needs no precomputed
//! table. The kernel runs on the application's own wgpu device ([`GpuContext`]) and writes the
//! picture in a buffer; a full-screen triangle draws that buffer in a texture.
//!
//! [`GpuCrt`] is a [`DisplayFilter`] that reads the picture back, so it can
//! feed any frontend, the recorder and the screenshots; a frontend that
//! presents the texture itself would skip that read-back.

use super::crt::{CrtParams, GlowField, Warp, GLOW_H, GLOW_W, SRC_HEIGHT, SRC_WIDTH};
#[cfg(not(target_arch = "wasm32"))]
use super::DisplayFilter;
use image::RgbImage;

/// The [`Warp`] of `blit.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WarpUniforms {
    k: [f32; 2],
    corner: f32,
    edged: f32,
    /// 1: the texels kept sharp at any zoom (`blit.wgsl`).
    sharp: f32,
    pad: [f32; 3],
}

/// The last pass of a frontend drawing on the GPU (`blit.wgsl`): a picture, the monitor's frame
/// or a [`CrtPass`]'s, drawn over the viewport set, through the glass of a [`Warp`].
pub struct Blit {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
}

impl Blit {
    /// A blit drawing into targets of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
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
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let linear = sampler(wgpu::FilterMode::Linear);
        Self {
            pipeline,
            layout,
            linear,
        }
    }

    /// What draws `view` through `warp`: a CRT's picture (`smooth`) or the raw one, both sampled
    /// so their texels stay sharp blocks at any zoom (`blit.wgsl`: exactly nearest at a whole
    /// zoom, the edges mixed over one screen pixel otherwise).
    pub fn bind_group(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        smooth: bool,
        warp: Warp,
    ) -> wgpu::BindGroup {
        let uniforms = WarpUniforms {
            k: [warp.k.0, warp.k.1],
            corner: warp.corner,
            edged: if warp.is_flat() { 0.0 } else { 1.0 },
            sharp: 1.0,
            pad: [0.0; 3],
        };
        let _ = smooth;
        let buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("warp"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        // linear: the shader places the samples itself
        let sampler = &self.linear;
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
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
    rgba: Vec<u8>,
}

impl FrameTexture {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("monitor frame"),
            size: wgpu::Extent3d {
                width: SRC_WIDTH as u32,
                height: SRC_HEIGHT as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            rgba: vec![0; SRC_WIDTH * SRC_HEIGHT * 4],
        }
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
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SRC_WIDTH as u32 * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: SRC_WIDTH as u32,
                height: SRC_HEIGHT as u32,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// The frame as the kernel takes it (words `0xBBGGRR`, padded with black to the monitor's size) and
/// its glow cells ([`GlowField`]).
pub fn crt_inputs(src: &RgbImage) -> (Vec<u32>, Vec<f32>) {
    let mut frame = vec![0u32; SRC_WIDTH * SRC_HEIGHT];
    let width = (src.width() as usize).min(SRC_WIDTH);
    for y in 0..(src.height() as usize).min(SRC_HEIGHT) {
        let row = &src.as_raw()[y * src.width() as usize * 3..][..width * 3];
        for (x, p) in row.chunks_exact(3).enumerate() {
            frame[y * SRC_WIDTH + x] = p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16;
        }
    }
    (frame, GlowField::new(src).texels())
}

type Server = cubecl::wgpu::WgpuServer<cubecl::wgpu::AutoCompiler>;

/// CubeCL running on the application's own wgpu device: the kernel's picture is a buffer of that
/// device, which a render pass of the application then reads.
#[derive(Clone)]
pub struct GpuContext {
    client: cubecl::client::Client,
}

impl GpuContext {
    /// Fails (with a message) when the device cannot run compute shaders (a WebGL page).
    pub fn new(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Self, String> {
        use cubecl_runtime::runtime::Runtime;
        if !adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        {
            return Err("this GPU cannot run compute shaders".into());
        }
        let setup = cubecl::wgpu::WgpuSetup {
            instance: instance.clone(),
            backend: adapter.get_info().backend,
            adapter: adapter.clone(),
            device: device.clone(),
            queue: queue.clone(),
        };
        let cube = cubecl::wgpu::try_init_device(setup, cubecl::wgpu::RuntimeOptions::default())
            .map_err(|e| format!("cannot start CubeCL on this device: {e:?}"))?;
        Ok(Self {
            client: <cubecl::wgpu::WgpuRuntime as Runtime>::client(&cube),
        })
    }

    /// The buffer of the device behind `handle`, and where the handle starts in it.
    fn buffer(&self, handle: &cubecl::server::Handle) -> (wgpu::Buffer, u64) {
        let managed = self
            .client
            .get_resource::<Server>(handle.clone())
            .expect("a buffer of the device");
        let resource = managed.resource();
        (resource.buffer.clone(), resource.offset)
    }
}

/// Draws the words of a buffer (`0xRRGGBB`, row after row) in a texture of the picture's size.
const SHOW: &str = r#"
@group(0) @binding(0) var<storage, read> pixels: array<u32>;
struct Size { w: u32, h: u32 }
@group(0) @binding(1) var<uniform> size: Size;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}
@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let v = pixels[u32(pos.y) * size.w + u32(pos.x)];
    return vec4<f32>(f32((v >> 16u) & 255u), f32((v >> 8u) & 255u), f32(v & 255u), 255.0) / 255.0;
}
"#;

/// The CRT pass: the CubeCL kernel of `kernel.rs` computes the picture of the frame uploaded, in a
/// buffer, and a pass draws it into a `Rgba8Unorm` target of [`CrtParams::output_size`].
pub struct CrtPass {
    ctx: GpuContext,
    out_len: usize,
    // the handles keep the buffers alive
    frame: cubecl::server::Handle,
    glow: cubecl::server::Handle,
    params: cubecl::server::Handle,
    out: cubecl::server::Handle,
    frame_buffer: (wgpu::Buffer, u64),
    glow_buffer: (wgpu::Buffer, u64),
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl CrtPass {
    /// Seen through the glass `warp`: the beam's raster bends under the fixed mask. The frame is
    /// black until [`Self::upload`].
    pub fn new(
        ctx: &GpuContext,
        device: &wgpu::Device,
        params: &CrtParams,
        warp: Warp,
    ) -> Result<Self, String> {
        let client = &ctx.client;
        let (w, h) = params.output_size();
        let frame = client.create_from_slice(&vec![0u8; SRC_WIDTH * SRC_HEIGHT * 4]);
        let glow = client.create_from_slice(&vec![0u8; GLOW_W * GLOW_H * 16]);
        let packed = super::kernel::pack(params, warp);
        let params_handle = client.create_from_slice(bytemuck::cast_slice(&packed));
        let out = client.empty(w * h * 4);
        let frame_buffer = ctx.buffer(&frame);
        let glow_buffer = ctx.buffer(&glow);
        let out_buffer = ctx.buffer(&out);

        let align = device.limits().min_storage_buffer_offset_alignment as u64;
        if out_buffer.1 % align != 0 {
            return Err(format!("the picture's buffer is at {}, not a multiple of {align}", out_buffer.1));
        }
        let size = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crt picture size"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        size.slice(..)
            .get_mapped_range_mut()
            .expect("mapped range")
            .copy_from_slice(bytemuck::cast_slice(&[w as u32, h as u32, 0u32, 0u32]));
        size.unmap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crt picture"),
            source: wgpu::ShaderSource::Wgsl(SHOW.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &out_buffer.0,
                        offset: out_buffer.1,
                        size: wgpu::BufferSize::new((w * h * 4) as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: size.as_entire_binding(),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crt picture"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            ctx: ctx.clone(),
            out_len: w * h,
            frame,
            glow,
            params: params_handle,
            out,
            frame_buffer,
            glow_buffer,
            pipeline,
            bind_group,
        })
    }

    /// The monitor's next frame, as [`crt_inputs`] makes it.
    pub fn upload(&self, queue: &wgpu::Queue, frame: &[u32], glow: &[f32]) {
        // what the kernel still has to read is submitted before the new frame is written
        let _ = self.ctx.client.flush();
        queue.write_buffer(&self.frame_buffer.0, self.frame_buffer.1, bytemuck::cast_slice(frame));
        queue.write_buffer(&self.glow_buffer.0, self.glow_buffer.1, bytemuck::cast_slice(glow));
    }

    /// A texture the pass can draw in, of the picture's size.
    pub fn create_target(
        device: &wgpu::Device,
        params: &CrtParams,
        extra: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        let (w, h) = params.output_size();
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("crt picture"),
            size: wgpu::Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | extra,
            view_formats: &[],
        })
    }

    /// Run the kernel (submitted now), then draw its picture into `target` with `encoder`.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        use cubecl::prelude::*;
        let n = self.out_len as u32;
        // SAFETY: the buffers are the sizes the kernel reads and writes (`kernel.rs`)
        unsafe {
            super::kernel::crt::launch_unchecked(
                &self.ctx.client,
                CubeCount::Static(n.div_ceil(256), 1, 1),
                CubeDim::new_1d(256),
                BufferArg::from_raw_parts(self.frame.clone(), SRC_WIDTH * SRC_HEIGHT),
                BufferArg::from_raw_parts(self.glow.clone(), GLOW_W * GLOW_H * 4),
                BufferArg::from_raw_parts(self.params.clone(), 26),
                BufferArg::from_raw_parts(self.out.clone(), self.out_len),
            );
        }
        let _ = self.ctx.client.flush();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("crt"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
    pass: CrtPass,
    target: wgpu::Texture,
    readback: wgpu::Buffer,
    /// Bytes per row of `readback`, a multiple of wgpu's copy alignment.
    padded_row: usize,
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
        Self::curved(params, Warp::FLAT)
    }

    /// Seen through the glass `warp` (see [`CrtPass::new`]).
    pub fn curved(params: CrtParams, warp: Warp) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| format!("cannot use the GPU: {e}"))?;

        let (out_w, out_h) = params.output_size();
        let ctx = GpuContext::new(&instance, &adapter, &device, &queue)?;
        let pass = CrtPass::new(&ctx, &device, &params, warp)?;
        let target = CrtPass::create_target(&device, &params, wgpu::TextureUsages::COPY_SRC);
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let padded_row = (out_w * 4).div_ceil(align) * align;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read back"),
            size: (padded_row * out_h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self {
            params,
            device,
            queue,
            pass,
            target,
            readback,
            padded_row,
        })
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

        let (frame, glow) = crt_inputs(src);
        self.pass.upload(&self.queue, &frame, &glow);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.pass.encode(
            &mut encoder,
            &self
                .target
                .create_view(&wgpu::TextureViewDescriptor::default()),
        );
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row as u32),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: out_w as u32,
                height: out_h as u32,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("cannot map the read-back buffer")
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("GPU wait failed");
        {
            let data = slice.get_mapped_range().expect("mapped range");
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
            *pixel = Rgb([
                (x * 7 % 256) as u8,
                (y * 5 % 256) as u8,
                ((x / 3 + y) * 11 % 256) as u8,
            ]);
        }
        // flat, and through each glass (the beam bent under the mask: `kernel.rs` and `cpu.rs`)
        let cases = [CrtProfile::Ctm644, CrtProfile::Trinitron]
            .into_iter()
            .flat_map(|profile| [(profile, Warp::FLAT), (profile, Warp::of(profile, 1.0))]);
        for (profile, warp) in cases {
            let params = profile.params();
            let mut gpu = match GpuCrt::curved(params.clone(), warp) {
                Ok(gpu) => gpu,
                Err(e) => {
                    eprintln!("skipped: {e}");
                    return;
                }
            };
            let mut cpu = CpuCrt::new(params).curved(warp);
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
            assert!(
                off < w * h / 1000,
                "{profile:?} {warp:?}: {off} channel values differ by more than 3 (worst {worst})"
            );
        }
    }

    /// The blit's glass is the CPU's [`crate::display::Curved`]: the same pixels are lit.
    #[test]
    fn blit_warp_matches_cpu() {
        use crate::display::{Curved, RawFilter, RAW_HEIGHT, RAW_WIDTH};
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) =
            block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let mut src = RgbImage::new(SRC_WIDTH as u32, SRC_HEIGHT as u32);
        src.pixels_mut()
            .for_each(|pixel| *pixel = Rgb([200, 100, 50]));
        let warp = Warp::of(CrtProfile::Ctm644, 1.0);

        let mut frame = FrameTexture::new(&device);
        frame.upload(&queue, &src);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let blit = Blit::new(&device, format);
        let bind_group = blit.bind_group(&device, frame.view(), false, warp);
        let (w, h) = (RAW_WIDTH as u32, RAW_HEIGHT as u32);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (w * h * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
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
            blit.draw(&mut pass, &bind_group);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let gpu: Vec<u32> = slice
            .get_mapped_range()
            .expect("mapped range")
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
            .filter(|(p, q)| {
                (0..3).any(|c| ((*p >> (c * 8)) & 255).abs_diff((*q >> (c * 8)) & 255) > 24)
            })
            .count();
        assert!(off < gpu.len() / 500, "{off} pixels differ");
    }
}

/// The uniforms of `resample.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ResampleParams {
    src: [f32; 2],
    dst: [f32; 2],
    axis: f32,
    /// The blur before the Lanczos, in source texels ([`super::resize::blur_sigma`]).
    sigma: f32,
    pad: [f32; 2],
}

/// A picture resized by any factor without moiré (`resample.wgsl`: the Benediction website's
/// thumbnails - a Gaussian blur of a third of the reduction, then Lanczos), in two passes: along x
/// into a texture of its own ([`Self::prepare`]), then along y into the caller's pass and
/// viewport ([`Self::draw`]).
pub struct Resampler {
    along_x: wgpu::RenderPipeline,
    along_y: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    x_params: wgpu::Buffer,
    y_params: wgpu::Buffer,
    /// The first pass's picture: the destination's width, the source's height.
    middle: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    y_bind_group: Option<wgpu::BindGroup>,
}

impl Resampler {
    /// The format of the first pass's texture.
    const MIDDLE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// A resampler drawing into targets of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resample"),
            source: wgpu::ShaderSource::Wgsl(include_str!("resample.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resample"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("resample"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("resample"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let buffer = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("resample"),
                size: std::mem::size_of::<ResampleParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            along_x: pipeline(Self::MIDDLE),
            along_y: pipeline(format),
            x_params: buffer(),
            y_params: buffer(),
            layout,
            middle: None,
            y_bind_group: None,
        }
    }

    /// The first pass: `source` (`src` texels) resized along x to `dst`'s width; [`Self::draw`]
    /// then draws it `dst` large.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        src: (u32, u32),
        dst: (u32, u32),
    ) {
        let middle_size = (dst.0.max(1), src.1.max(1));
        if self
            .middle
            .as_ref()
            .is_none_or(|(.., size)| *size != middle_size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("resample along x"),
                size: wgpu::Extent3d {
                    width: middle_size.0,
                    height: middle_size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: Self::MIDDLE,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.y_bind_group = Some(self.bind_group(device, &view, &self.y_params));
            self.middle = Some((texture, view, middle_size));
        }
        let params = |src: (u32, u32), dst: (u32, u32), axis: f32| {
            let (n_src, n_dst) = if axis < 0.5 {
                (src.0, dst.0)
            } else {
                (src.1, dst.1)
            };
            ResampleParams {
                src: [src.0 as f32, src.1 as f32],
                dst: [dst.0 as f32, dst.1 as f32],
                axis,
                sigma: super::resize::blur_sigma(n_src as f32 / n_dst.max(1) as f32),
                pad: [0.0; 2],
            }
        };
        queue.write_buffer(
            &self.x_params,
            0,
            bytemuck::bytes_of(&params(src, middle_size, 0.0)),
        );
        queue.write_buffer(
            &self.y_params,
            0,
            bytemuck::bytes_of(&params(middle_size, dst, 1.0)),
        );
        let x_bind_group = self.bind_group(device, source, &self.x_params);
        let (_, middle_view, _) = self.middle.as_ref().expect("made above");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("resample along x"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: middle_view,
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
        pass.set_pipeline(&self.along_x);
        pass.set_bind_group(0, &x_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The second pass, along y, over `pass`'s viewport (the `dst` of [`Self::prepare`]).
    pub fn draw(&self, pass: &mut wgpu::RenderPass) {
        if let Some(bind_group) = &self.y_bind_group {
            pass.set_pipeline(&self.along_y);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resample"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }
}
