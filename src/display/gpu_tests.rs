//! The kernels on a GPU adapter against the CPU backend (a software Vulkan like lavapipe will do),
//! and the whole path of the application's frame: [`GpuFrame`], [`CrtPass`], a render pass, a
//! readback. Skipped where there is no adapter.

use super::crt::{CrtProfile, Warp, SRC_HEIGHT, SRC_WIDTH};
use super::gpu::{wait_for, CrtPass, GpuContext, GpuFrame};
use super::kernel_crt::{Backend, KernelCrt};
use super::DisplayFilter;
use image::{Rgb, RgbImage};

fn pattern() -> RgbImage {
    RgbImage::from_fn(SRC_WIDTH as u32, SRC_HEIGHT as u32, |x, y| {
        Rgb([(x * 7 % 256) as u8, (y * 5 % 256) as u8, ((x / 3 + y) * 11 % 256) as u8])
    })
}

/// A pseudo-random frame (a fixed LCG): no structure for the kernels to hide behind.
fn noise() -> RgbImage {
    let mut state = 0x2545_f491u32;
    RgbImage::from_fn(SRC_WIDTH as u32, SRC_HEIGHT as u32, |_, _| {
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        };
        Rgb([next(), next(), next()])
    })
}

/// The worst difference of two pictures, and how many channel values differ by more than 3.
fn compare(a: &[u32], b: &[u32]) -> (i32, usize) {
    let (mut worst, mut off) = (0, 0);
    for (p, q) in a.iter().zip(b) {
        for shift in [0, 8, 16] {
            let d = (((p >> shift) & 255) as i32 - ((q >> shift) & 255) as i32).abs();
            worst = worst.max(d);
            off += (d > 3) as usize;
        }
    }
    (worst, off)
}

#[test]
fn the_gpu_gives_the_cpu_backends_picture() {
    for (profile, curvature) in [(CrtProfile::Ctm644, 0.0), (CrtProfile::Ctm644, 1.0), (CrtProfile::Gt65, 1.0)] {
        let warp = Warp::of(profile, curvature);
        let Ok(mut gpu) = KernelCrt::new(profile.params(), warp, Backend::Gpu) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let mut cpu = KernelCrt::new(profile.params(), warp, Backend::Cpu).unwrap();
        let (w, h) = cpu.output_size();
        for src in [pattern(), noise()] {
            let (mut a, mut b) = (vec![0; w * h], vec![0; w * h]);
            cpu.render(&src, &mut a);
            gpu.render(&src, &mut b);
            let (worst, off) = compare(&a, &b);
            assert!(worst <= 6 && off < w * h * 3 / 1000, "{profile:?} {curvature}: worst {worst}, {off} values off by more than 3");
        }
    }
}

/// What the window and the page draw: the frame uploaded as bytes, the kernels, the pass that
/// shows their buffer in a texture, read back here.
#[test]
fn the_application_path_draws_the_kernels_picture() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = wait_for(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let (device, queue) = wait_for(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let Ok(ctx) = GpuContext::new(&instance, &adapter, &device, &queue) else {
        eprintln!("skipped: no compute shaders");
        return;
    };
    let params = CrtProfile::Ctm644.params();
    let warp = Warp::of(CrtProfile::Ctm644, 1.0);
    let pass = CrtPass::new(&ctx, &device, &params, warp).unwrap();
    let mut frame = GpuFrame::new(&ctx).unwrap();
    let (w, h) = params.output_size();
    let target = CrtPass::create_target(&device, &params, wgpu::TextureUsages::COPY_SRC);
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let padded = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut cpu = KernelCrt::new(params.clone(), warp, Backend::Cpu).unwrap();
    // two frames in a row: the second must replace the first (the buffers are shared)
    for src in [pattern(), noise()] {
        frame.upload(&queue, &src);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        pass.encode(&mut encoder, &view, &frame);
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded as u32), rows_per_image: None },
            },
            wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        staging.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let shown: Vec<u32> = {
            let data = staging.slice(..).get_mapped_range().unwrap();
            (0..h)
                .flat_map(|y| (0..w).map(move |x| (y, x)))
                .map(|(y, x)| {
                    let p = &data[y * padded + x * 4..][..4];
                    (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
                })
                .collect()
        };
        staging.unmap();
        let mut want = vec![0; w * h];
        cpu.render(&src, &mut want);
        let (worst, off) = compare(&want, &shown);
        assert!(worst <= 6 && off < w * h * 3 / 1000, "worst {worst}, {off} values off by more than 3");
    }
}

/// The first kernel's output (the light of every slot) is the same on both backends: a read of the
/// line above the picture (the top rows) once differed on a GPU, whose rule for a read out of
/// bounds is not the CPU's.
#[test]
fn the_slot_lights_are_the_same_on_both_backends() {
    let profile = CrtProfile::Ctm644;
    let Ok(mut gpu) = KernelCrt::new(profile.params(), Warp::FLAT, Backend::Gpu) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let mut cpu = KernelCrt::new(profile.params(), Warp::FLAT, Backend::Cpu).unwrap();
    let w = profile.params().output_size().0;
    for (name, src) in [("pattern", pattern()), ("noise", noise())] {
        let (a, b) = (cpu.slot_lights(&src), gpu.slot_lights(&src));
        let differing: Vec<(usize, f32)> = a
            .iter()
            .zip(&b)
            .enumerate()
            .filter(|(_, (p, q))| (**p - **q).abs() > 0.02)
            .map(|(i, (p, q))| (i, (p - q).abs()))
            .collect();
        let worst = differing.iter().map(|d| d.1).fold(0.0, f32::max);
        let first = differing.first().map(|d| (d.0 % w, d.0 / w));
        if name == "pattern" {
            // a smooth picture: the same, whatever the backend rounds to
            assert!(differing.is_empty(), "slot lights differ at {first:?}");
        } else {
            // noise puts a pixel's phase on the edge of a table bucket now and then (the two
            // backends round differently): rare, and small (the weight of one far tap)
            assert!(worst < 0.1 && differing.len() * 1000 < a.len(), "{} slot lights differ, by {worst} at worst, first at {first:?}", differing.len());
        }
    }
}

/// The picture with the hardware acceleration off is made on the CPU (and says so), and is the
/// picture the GPU makes (to rounding); with it on again, the GPU makes it.
#[test]
fn the_picture_is_made_on_the_cpu_when_the_acceleration_is_off() {
    use super::picture::Picture;
    use super::DisplayMode;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let Ok(adapter) = wait_for(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipped: no GPU adapter");
        return;
    };
    let (device, queue) = wait_for(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let Ok(ctx) = GpuContext::new(&instance, &adapter, &device, &queue) else {
        eprintln!("skipped: no compute shaders");
        return;
    };
    let profile = CrtProfile::Ctm644;
    let (w, h) = profile.params().output_size();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut picture = Picture::new(&device, format, Some(ctx));
    picture.adapter = "test adapter".into();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let padded = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let draw = |picture: &mut Picture| -> Vec<u32> {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        picture.draw(
            &device, &queue, &mut encoder, &view, (w as u32, h as u32), (0.0, 0.0),
            DisplayMode::Crt(profile), Warp::FLAT, false,
        );
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded as u32), rows_per_image: None },
            },
            wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        staging.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let shown = {
            let data = staging.slice(..).get_mapped_range().unwrap();
            (0..h)
                .flat_map(|y| (0..w).map(move |x| (y, x)))
                .map(|(y, x)| {
                    let p = &data[y * padded + x * 4..][..4];
                    (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
                })
                .collect()
        };
        staging.unmap();
        shown
    };
    let src = pattern();
    let mut cpu = KernelCrt::new(profile.params(), Warp::FLAT, Backend::Cpu).unwrap();
    let mut want = vec![0; w * h];
    cpu.render(&src, &mut want);

    picture.upload(&queue, &src);
    let on_gpu = draw(&mut picture);
    assert_eq!(picture.renderer(), "GPU · test adapter");
    let (worst, off) = compare(&want, &on_gpu);
    assert!(worst <= 6 && off < w * h * 3 / 1000, "GPU: worst {worst}, {off} off");

    // off: the CPU backend's picture itself (a paused picture is made from the frame kept)
    picture.hardware = false;
    let on_cpu = draw(&mut picture);
    assert_eq!(picture.renderer(), "CPU (software rendering)");
    assert_eq!(compare(&want, &on_cpu).1, 0, "the CPU's picture is the CPU backend's");
    picture.upload(&queue, &noise());
    let _ = draw(&mut picture);
    assert_eq!(picture.renderer(), "CPU (software rendering)");

    // on again: the GPU draws the last frame (noise), not a stale one
    picture.hardware = true;
    let back = draw(&mut picture);
    assert_eq!(picture.renderer(), "GPU · test adapter");
    let mut want_noise = vec![0; w * h];
    cpu.render(&noise(), &mut want_noise);
    let (worst, off) = compare(&want_noise, &back);
    assert!(worst <= 6 && off < w * h * 3 / 1000, "back on the GPU: worst {worst}, {off} off");
}
