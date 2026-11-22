//! Spike: the CRT model of bemu (crt.wgsl) as one CubeCL kernel, compared with the CPU renderer.
use amsrust::display::crt::{GlowField, SRC_HEIGHT, SRC_WIDTH};
use amsrust::display::{CpuCrt, CrtParams, CrtProfile, DisplayFilter, Warp};
use cubecl::prelude::*;
use cubecl_runtime::runtime::Runtime;
use cubecl::wgpu::wgpu;
use image::{Rgb, RgbImage};

// parameter slots
const OUT_W: usize = 0;
const OUT_H: usize = 1;
const SLOT_PX: usize = 2;
const ROWS: usize = 3;
const PITCH: usize = 4;
const FILL_X: usize = 5;
const FILL_Y: usize = 6;
const CORNER: usize = 7;
const STAGGER: usize = 8;
const SIGMA_H: usize = 9;
const SIGMA_V: usize = 10;
const SPILL: usize = 11;
const HALATION: usize = 12;
const SCALE: usize = 13;
const X_RATIO: usize = 14;
const MONO: usize = 15;
const TINT: usize = 16; // 3 values
const VEIL: usize = 19;
const VIGNETTE: usize = 20;
const GLOW: usize = 21;
const WARP_KX: usize = 22;
const WARP_KY: usize = 23;
const WARP_CORNER: usize = 24;
const EDGED: usize = 25;

#[cube]
fn gauss(d: f32, sigma: f32) -> f32 {
    f32::exp(-d * d / (2.0 * sigma * sigma))
}

/// Linear light of gun `c` of the source pixel (i, j); black outside.
#[cube]
fn gun(frame: &[u32], pr: &[f32], i: i32, j: i32, c: u32) -> f32 {
    let mut out = 0.0f32;
    if i >= 0 && i < 768 && j >= 0 && j < 272 {
        let v = frame[(j * 768 + i) as usize];
        let r = (v & 255u32) as f32 / 255.0;
        let g = ((v >> 8u32) & 255u32) as f32 / 255.0;
        let b = ((v >> 16u32) & 255u32) as f32 / 255.0;
        if pr[MONO] > 0.5 {
            out = 0.299 * r * r + 0.587 * g * g + 0.114 * b * b;
        } else {
            let mut s = r;
            if c == 1u32 {
                s = g;
            } else if c == 2u32 {
                s = b;
            }
            out = s * s;
        }
    }
    out
}

#[cube]
fn slot_distance(pr: &[f32], x: i32, y: i32) -> f32 {
    let sw = pr[SLOT_PX] as i32;
    let slot_w = pr[SLOT_PX];
    let slot = x / sw;
    let triad = slot / 3;
    let mut shift = 0.0f32;
    if triad % 2 == 1 {
        shift = pr[STAGGER] * pr[PITCH];
    }
    let px = (x as f32 + 0.5) - slot as f32 * slot_w - slot_w / 2.0;
    let t = y as f32 + 0.5 - shift;
    let py = (t - pr[PITCH] * f32::floor(t / pr[PITCH])) - pr[PITCH] / 2.0;
    let hx = pr[FILL_X] * slot_w / 2.0;
    let hy = pr[FILL_Y] * pr[PITCH] / 2.0;
    let r = f32::max(pr[CORNER] * f32::min(hx, hy), 0.0);
    let qx = f32::abs(px) - (hx - r);
    let qy = f32::abs(py) - (hy - r);
    let ox = f32::max(qx, 0.0);
    let oy = f32::max(qy, 0.0);
    f32::sqrt(ox * ox + oy * oy) + f32::min(f32::max(qx, qy), 0.0) - r
}

/// The light of the slot at output column `x`, row `y` (flat glass), in channel `ch`.
#[cube]
fn light(frame: &[u32], pr: &[f32], x: i32, y: i32, ch: u32) -> f32 {
    let mut out = 0.0f32;
    if x >= 0 && x < pr[OUT_W] as i32 {
        let sw = pr[SLOT_PX] as i32;
        let slot = x / sw;
        let mut c = (slot % 3) as u32;
        if pr[MONO] > 0.5 {
            c = 1u32;
        }
        // the beam's raster bends under the fixed mask: the slot reads where its centre's warped place is
        let ow = pr[OUT_W];
        let oh = pr[OUT_H];
        let qx = ((slot as f32 + 0.5) * sw as f32 / ow) * 2.0 - 1.0;
        let qy = ((y as f32 + 0.5) / oh) * 2.0 - 1.0;
        let wx = (qx * (1.0 + pr[WARP_KX] * qy * qy) + 1.0) / 2.0;
        let wy = (qy * (1.0 + pr[WARP_KY] * qx * qx) + 1.0) / 2.0;
        let bx = wx * ow;
        let by = wy * oh;
        let u = bx / pr[X_RATIO];
        let first = f32::floor(u - 0.5) as i32 - 1;
        let mut sum = 0.0f32;
        for t in 0..4 {
            sum += gauss(u - ((first + t) as f32 + 0.5), pr[SIGMA_H]);
        }
        let f = by / pr[ROWS] - 0.5;
        let line = f32::floor(f) as i32;
        let a = gauss(f - line as f32, pr[SIGMA_V]);
        let b = gauss(line as f32 + 1.0 - f, pr[SIGMA_V]);
        let mut la = 0.0f32;
        let mut lb = 0.0f32;
        for t in 0..4 {
            let w = gauss(u - ((first + t) as f32 + 0.5), pr[SIGMA_H]) / sum;
            la += w * gun(frame, pr, first + t, line, c);
            lb += w * gun(frame, pr, first + t, line + 1, c);
        }
        let level = a * la + b * lb;
        let cov = f32::clamp(0.5 - (slot_distance(pr, x, y) - pr[SPILL] * level), 0.0, 1.0);
        let l = level * cov * pr[SCALE];
        if pr[MONO] > 0.5 {
            out = pr[TINT + ch as usize] * l;
        } else if c == ch {
            out = l;
        }
    }
    out
}

#[cube]
fn glow_cell(glow: &[f32], x: i32, y: i32, c: u32) -> f32 {
    glow[((y * 192 + x) * 4) as usize + c as usize]
}

#[cube]
fn glow_at(glow: &[f32], u: f32, v: f32, c: u32) -> f32 {
    let gx = f32::clamp(u * 192.0 - 0.5, 0.0, 191.0);
    let gy = f32::clamp(v * 68.0 - 0.5, 0.0, 67.0);
    let x0 = f32::floor(gx) as i32;
    let y0 = f32::floor(gy) as i32;
    let x1 = i32::min(x0 + 1, 191);
    let y1 = i32::min(y0 + 1, 67);
    let fx = gx - x0 as f32;
    let fy = gy - y0 as f32;
    let top = glow_cell(glow, x0, y0, c) * (1.0 - fx) + glow_cell(glow, x1, y0, c) * fx;
    let bottom = glow_cell(glow, x0, y1, c) * (1.0 - fx) + glow_cell(glow, x1, y1, c) * fx;
    top * (1.0 - fy) + bottom * fy
}

#[cube]
fn channel_value(frame: &[u32], glow: &[f32], pr: &[f32], x: i32, y: i32, ch: u32) -> f32 {
    let sw = pr[SLOT_PX] as i32;
    let h = pr[HALATION];
    let mut rgb = (1.0 - 2.0 * h) * light(frame, pr, x, y, ch)
        + h * (light(frame, pr, x - sw, y, ch) + light(frame, pr, x + sw, y, ch));
    let u = (x as f32 + 0.5) / pr[OUT_W];
    let v = (y as f32 + 0.5) / pr[OUT_H];
    let mut tint = 1.0f32;
    let mut g = glow_at(glow, u, v, ch);
    if pr[MONO] > 0.5 {
        tint = pr[TINT + ch as usize];
        g = (0.299 * glow_at(glow, u, v, 0u32) + 0.587 * glow_at(glow, u, v, 1u32) + 0.114 * glow_at(glow, u, v, 2u32)) * tint;
    }
    rgb += pr[VEIL] * tint + pr[GLOW] * g;
    let dx = u * 2.0 - 1.0;
    let dy = v * 2.0 - 1.0;
    rgb *= f32::max(1.0 - pr[VIGNETTE] * (dx * dx + dy * dy), 0.0);
    // the glass's rounded edge, smoothed over about a pixel
    let qx = u * 2.0 - 1.0;
    let qy = v * 2.0 - 1.0;
    let wx = qx * (1.0 + pr[WARP_KX] * qy * qy);
    let wy = qy * (1.0 + pr[WARP_KY] * qx * qx);
    let ex = f32::abs(wx) - (1.0 - pr[WARP_CORNER]);
    let ey = f32::abs(wy) - (1.0 - pr[WARP_CORNER]);
    let ox = f32::max(ex, 0.0);
    let oy = f32::max(ey, 0.0);
    let distance = f32::sqrt(ox * ox + oy * oy) + f32::min(f32::max(ex, ey), 0.0) - pr[WARP_CORNER];
    let pixel = 2.0 / f32::max(pr[OUT_W], pr[OUT_H]);
    let edge = f32::clamp(0.5 - distance / pixel, 0.0, 1.0);
    let alpha = (1.0 - pr[EDGED]) + pr[EDGED] * edge;
    f32::sqrt(f32::clamp(rgb, 0.0, 1.0)) * alpha
}

#[cube(launch_unchecked)]
fn crt(frame: &[u32], glow: &[f32], pr: &[f32], out: &mut [u32]) {
    let i = ABSOLUTE_POS;
    if i < out.len() {
        let w = pr[OUT_W] as usize;
        let x = (i % w) as i32;
        let y = (i / w) as i32;
        let r = (channel_value(frame, glow, pr, x, y, 0u32) * 255.0 + 0.5) as u32;
        let g = (channel_value(frame, glow, pr, x, y, 1u32) * 255.0 + 0.5) as u32;
        let b = (channel_value(frame, glow, pr, x, y, 2u32) * 255.0 + 0.5) as u32;
        out[i] = (r << 16u32) | (g << 8u32) | b;
    }
}

fn test_picture() -> RgbImage {
    let mut src = RgbImage::new(SRC_WIDTH as u32, SRC_HEIGHT as u32);
    for (x, y, p) in src.enumerate_pixels_mut() {
        *p = Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x / 3 + y) * 11 % 256) as u8]);
    }
    src
}

fn pack(p: &CrtParams, warp: Warp) -> Vec<f32> {
    let (w, h) = p.output_size();
    let mut v = vec![0.0f32; 26];
    v[OUT_W] = w as f32;
    v[OUT_H] = h as f32;
    v[SLOT_PX] = p.slot_px() as f32;
    v[ROWS] = p.rows_per_line as f32;
    v[PITCH] = p.pitch_rows() as f32;
    v[FILL_X] = p.slot_fill.0;
    v[FILL_Y] = p.slot_fill.1;
    v[CORNER] = p.corner_radius;
    v[STAGGER] = p.stagger;
    v[SIGMA_H] = p.beam_sigma_h;
    v[SIGMA_V] = p.beam_sigma_v;
    v[SPILL] = p.spill;
    v[HALATION] = p.halation;
    v[SCALE] = p.gain * p.white_norm();
    v[X_RATIO] = p.x_ratio();
    v[MONO] = p.phosphor.map_or(0.0, |_| 1.0);
    let t = p.phosphor.unwrap_or([1.0; 3]);
    v[TINT..TINT + 3].copy_from_slice(&t);
    v[VEIL] = p.veil_light();
    v[VIGNETTE] = p.vignette;
    v[GLOW] = p.glow_light();
    v[WARP_KX] = warp.k.0;
    v[WARP_KY] = warp.k.1;
    v[WARP_CORNER] = warp.corner;
    v[EDGED] = if warp.is_flat() { 0.0 } else { 1.0 };
    v
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

/// The whole path of a frame without readback: the CRT kernel writes its picture in a buffer of the
/// app's wgpu device, and a fullscreen pass of that device reads the buffer as storage and draws
/// it (here into a texture, read back to check it against the CPU renderer).
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
    let frame: Vec<u32> = src.pixels().map(|p| p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16).collect();
    let glow = GlowField::new(&src).texels();
    let pr = pack(&params, warp);
    let frame_h = client.create_from_slice(u32::as_bytes(&frame));
    let glow_h = client.create_from_slice(f32::as_bytes(&glow));
    let pr_h = client.create_from_slice(f32::as_bytes(&pr));
    let out_h = client.empty(w * h * 4);
    let n = (w * h) as u32;
    unsafe {
        crt::launch_unchecked(&client, CubeCount::Static(n.div_ceil(256), 1, 1), CubeDim::new_1d(256),
            BufferArg::from_raw_parts(frame_h, frame.len()), BufferArg::from_raw_parts(glow_h, glow.len()),
            BufferArg::from_raw_parts(pr_h, pr.len()), BufferArg::from_raw_parts(out_h.clone(), w * h));
    }
    let resource = client.get_resource::<WgpuServer<AutoCompiler>>(out_h).unwrap();
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
        let frame: Vec<u32> = src.pixels().map(|p| p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16).collect();
        let glow = GlowField::new(&src).texels();
        let pr = pack(&params, warp);

        let mut cpu = CpuCrt::new(params.clone()).curved(warp);
        let mut want = vec![0u32; w * h];
        let t = std::time::Instant::now();
        cpu.render(&src, &mut want);
        let cpu_ms = t.elapsed().as_secs_f64() * 1e3;

        let frame_h = client.create_from_slice(u32::as_bytes(&frame));
        let glow_h = client.create_from_slice(f32::as_bytes(&glow));
        let pr_h = client.create_from_slice(f32::as_bytes(&pr));
        let out_h = client.empty(w * h * 4);
        let n = (w * h) as u32;
        let mut got_bytes = Vec::new();
        let mut best = f64::MAX;
        for _ in 0..4 {
            let t = std::time::Instant::now();
            unsafe {
                crt::launch_unchecked(
                    &client,
                    CubeCount::Static(n.div_ceil(256), 1, 1),
                    CubeDim::new_1d(256),
                    BufferArg::from_raw_parts(frame_h.clone(), frame.len()),
                    BufferArg::from_raw_parts(glow_h.clone(), glow.len()),
                    BufferArg::from_raw_parts(pr_h.clone(), pr.len()),
                    BufferArg::from_raw_parts(out_h.clone(), w * h),
                );
            }
            got_bytes = client.read_one(out_h.clone()).unwrap().to_vec();
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
        println!("{profile:?} {} {w}x{h}: worst {worst}, {off} values off by >3 of {}; cpu {cpu_ms:.1} ms, cubecl(lavapipe) {best:.1} ms", if warp.is_flat() { "flat" } else { "curved" }, w * h * 3);
    }
}
