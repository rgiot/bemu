//! Spike and benchmark: the CRT model of bemu as the CubeCL kernels of the application itself
//! (`src/display/kernel.rs`, included as it is), compared with the CPU renderer.
mod display;
use amsrust::display::crt::{SRC_HEIGHT, SRC_WIDTH};
use amsrust::display::{CpuCrt, CrtProfile, DisplayFilter, Warp};
use cubecl::prelude::*;
use cubecl::wgpu::wgpu;
use cubecl_runtime::runtime::Runtime;
use display::kernel::Pipeline;
use image::{Rgb, RgbImage};

fn test_picture() -> RgbImage {
    let mut src = RgbImage::new(SRC_WIDTH as u32, SRC_HEIGHT as u32);
    for (x, y, p) in src.enumerate_pixels_mut() {
        *p = Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x / 3 + y) * 11 % 256) as u8]);
    }
    src
}

/// The app's own wgpu device, shared with CubeCL (`init_device`), and the output buffer of a
/// kernel taken back as a `wgpu::Buffer` of that device: what a render pass would read directly.
fn shared_device_check() {
    use cubecl::wgpu::{init_device, RuntimeOptions, WgpuDevice, WgpuServer, WgpuSetup};
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let setup = WgpuSetup { instance, backend: adapter.get_info().backend, adapter, device: device.clone(), queue: queue.clone() };
    let cube_device: WgpuDevice = init_device(setup, RuntimeOptions::default());
    let client = <cubecl::wgpu::WgpuRuntime as Runtime>::client(&cube_device);
    let data: Vec<u32> = (0..1024u32).collect();
    let input = client.create_from_slice(u32::as_bytes(&data));
    let out = client.empty(1024 * 4);
    unsafe {
        copy_kernel::launch_unchecked(&client, CubeCount::Static(4, 1, 1), CubeDim::new_1d(256),
            BufferArg::from_raw_parts(input, 1024), BufferArg::from_raw_parts(out.clone(), 1024));
    }
    let resource = client.get_resource::<WgpuServer<cubecl::wgpu::AutoCompiler>>(out.clone()).unwrap();
    let res = resource.resource();
    println!("shared device: buffer usage {:?}, size {}, offset {}", res.buffer.usage(), res.buffer.size(), res.offset);
    // read it with our own wgpu queue, not with CubeCL's
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: 4096, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(&res.buffer, res.offset, &staging, 0, 4096);
    queue.submit([enc.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let got: Vec<u32> = bytemuck::cast_slice(&staging.slice(..).get_mapped_range().unwrap()).to_vec();
    let ok = got.iter().enumerate().all(|(i, &v)| v == i as u32 * 2);
    println!("read back through our own wgpu queue: {}", if ok { "same data: OK" } else { "DIFFERENT" });
}

#[cube(launch_unchecked)]
fn copy_kernel(input: &[u32], out: &mut [u32]) {
    if ABSOLUTE_POS < out.len() {
        out[ABSOLUTE_POS] = input[ABSOLUTE_POS] * 2u32;
    }
}

const QUAD: &str = r#"
@group(0) @binding(0) var<storage, read> pix: array<u32>;
struct Dim { w: u32, h: u32 }
@group(0) @binding(1) var<uniform> dim: Dim;
@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}
@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let v = pix[u32(pos.y) * dim.w + u32(pos.x)];
    return vec4<f32>(f32((v >> 16u) & 255u), f32((v >> 8u) & 255u), f32(v & 255u), 255.0) / 255.0;
}
"#;

/// The whole path of a frame without readback: the kernels write the picture in a buffer of the
/// app's wgpu device, and a fullscreen pass of that device reads the buffer as storage and draws it
/// (here into a texture, read back to check it against the CPU renderer).
fn quad_pass_check() {
    use cubecl::wgpu::{init_device, AutoCompiler, RuntimeOptions, WgpuServer, WgpuSetup};
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let setup = WgpuSetup { instance, backend: adapter.get_info().backend, adapter, device: device.clone(), queue: queue.clone() };
    let client = <cubecl::wgpu::WgpuRuntime as Runtime>::client(&init_device(setup, RuntimeOptions::default()));

    let (profile, warp) = (CrtProfile::Ctm644, Warp::of(CrtProfile::Ctm644, 1.0));
    let params = profile.params();
    let (w, h) = params.output_size();
    let src = test_picture();
    let frame = client.create_from_slice(src.as_raw());
    let pipeline = Pipeline::new(&client, &params, warp);
    pipeline.run(&client, &frame, src.as_raw().len() / 4, src.width(), src.height());
    let bytes = client.read_one(pipeline.out().clone()).unwrap();
    let px = u32::from_bytes(&bytes);
    let sum = px.iter().fold(0u64, |a, &v| a + v as u64);
    println!("native {w}x{h} sum {sum} first {:06x} {:06x} {:06x}", px[0], px[700 * w + 500], px[w * h - 1]);
    let resource = client.get_resource::<WgpuServer<AutoCompiler>>(pipeline.out().clone()).unwrap();
    let res = resource.resource();
    // the pass
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(QUAD.into()) });
    let dim = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    queue.write_buffer(&dim, 0, bytemuck::cast_slice(&[w as u32, h as u32, 0, 0]));
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[
        wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
        wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
    ] });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[
        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &res.buffer, offset: res.offset, size: wgpu::BufferSize::new((w * h * 4) as u64) }) },
        wgpu::BindGroupEntry { binding: 1, resource: dim.as_entire_binding() },
    ] });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None, layout: Some(&pl),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
        primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: wgpu::MultisampleState::default(),
        multiview_mask: None, cache: None });
    let target = device.create_texture(&wgpu::TextureDescriptor { label: None,
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
    let view = target.create_view(&Default::default());
    let padded = (w * 4).div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (padded * h) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    enc.copy_texture_to_buffer(target.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &staging, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded as u32), rows_per_image: None } },
        wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 });
    queue.submit([enc.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = staging.slice(..).get_mapped_range().unwrap();

    let mut cpu = CpuCrt::new(params.clone()).curved(warp);
    let mut want = vec![0u32; w * h];
    cpu.render(&src, &mut want);
    let (mut worst, mut off) = (0i32, 0usize);
    for y in 0..h {
        for x in 0..w {
            let px = &data[y * padded + x * 4..][..4];
            let q = want[y * w + x];
            for (k, s) in [16, 8, 0].into_iter().enumerate() {
                let d = (px[k] as i32 - ((q >> s) & 255) as i32).abs();
                worst = worst.max(d);
                off += (d > 3) as usize;
            }
        }
    }
    let cpu_sum = want.iter().fold(0u64, |a, &v| a + v as u64);
    println!("CPU renderer sum {cpu_sum} first {:06x} {:06x} {:06x}", want[0], want[700 * w + 500], want[w * h - 1]);
    println!("quad pass of the kernel's buffer vs the CPU renderer: worst {worst}, {off} values off by >3 of {}", w * h * 3);
}

fn main() {
    quad_pass_check();
    shared_device_check();
    type R = cubecl::wgpu::WgpuRuntime;
    let client = R::client(&Default::default());
    for (profile, warp) in [
        (CrtProfile::Ctm644, Warp::FLAT),
        (CrtProfile::Ctm644, Warp::of(CrtProfile::Ctm644, 1.0)),
        (CrtProfile::Gt65, Warp::of(CrtProfile::Gt65, 1.0)),
    ] {
        let params = profile.params();
        let (w, h) = params.output_size();
        let src = test_picture();
        let mut cpu = CpuCrt::new(params.clone()).curved(warp);
        let mut want = vec![0u32; w * h];
        let t = std::time::Instant::now();
        cpu.render(&src, &mut want);
        let cpu_ms = t.elapsed().as_secs_f64() * 1e3;

        let frame = client.create_from_slice(src.as_raw());
        let words = src.as_raw().len() / 4;
        let pipeline = Pipeline::new(&client, &params, warp);
        let mut got_bytes = Vec::new();
        let mut best = f64::MAX;
        for _ in 0..4 {
            let t = std::time::Instant::now();
            pipeline.run(&client, &frame, words, src.width(), src.height());
            got_bytes = client.read_one(pipeline.out().clone()).unwrap().to_vec();
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
        }
        let got = u32::from_bytes(&got_bytes);
        let (mut worst, mut off) = (0i32, 0usize);
        for (p, q) in want.iter().zip(got) {
            for s in [0, 8, 16] {
                let d = (((p >> s) & 255) as i32 - ((q >> s) & 255) as i32).abs();
                worst = worst.max(d);
                off += (d > 3) as usize;
            }
        }
        println!("{profile:?} {} {w}x{h}: worst {worst}, {off} values off by >3 of {}; cpu {cpu_ms:.1} ms (all of it), gpu {best:.1} ms (glow + picture + readback; the CPU only copies the frame)", if warp.is_flat() { "flat" } else { "curved" }, w * h * 3);
    }
}
