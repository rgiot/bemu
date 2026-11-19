//! The monitor's picture drawn by the GPU in a window or a canvas: the one implementation of the
//! desktop's winit window and of the web page.
//!
//! A frame is uploaded as a texture; a CRT display has its shader pass draw the tube's picture in
//! a texture of its own, at the resolution the room asks for ([`PictureSize`]); the picture is
//! then drawn over a viewport of the target, centred, resized without moiré when it is not shown
//! at a whole factor. The frontend sets the surface, and draws its interface over the picture.

use image::RgbImage;

use super::crt::{CrtParams, Warp, SRC_HEIGHT, SRC_WIDTH};
use super::gpu::{Blit, CrtPass, FrameTexture, Resampler};
use super::{DisplayMode, PictureSize, RAW_HEIGHT, RAW_WIDTH};

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

/// The picture's textures and passes, for the frames of one surface.
pub struct Picture {
    frame: FrameTexture,
    blit: Blit,
    raw_bind_group: wgpu::BindGroup,
    crt: Option<CrtState>,
    /// The picture resized without moiré (a CRT's at a fractional zoom, any below x1).
    resampler: Option<Resampler>,
    format: wgpu::TextureFormat,
    pub size: PictureSize,
}

impl Picture {
    /// A picture drawn in targets of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let frame = FrameTexture::new(device);
        let blit = Blit::new(device, format);
        let raw_bind_group = blit.bind_group(device, frame.view(), false, Warp::FLAT);
        Self {
            frame,
            blit,
            raw_bind_group,
            crt: None,
            resampler: None,
            format,
            size: PictureSize::default(),
        }
    }

    /// The monitor's next frame.
    pub fn upload(&mut self, queue: &wgpu::Queue, frame: &RgbImage) {
        self.frame.upload(queue, frame);
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
    fn use_mode(&mut self, device: &wgpu::Device, mode: DisplayMode, level: usize, warp: Warp) -> bool {
        let DisplayMode::Crt(profile) = mode else {
            return false;
        };
        if self
            .crt
            .as_ref()
            .is_some_and(|crt| crt.mode == mode && crt.level == level && crt.warp == warp)
        {
            return false;
        }
        let params: CrtParams = profile.params().scaled(level);
        let pass = CrtPass::new(device, &params, &self.frame, warp);
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
        let made = self.use_mode(device, mode, level, warp);
        let crt_shown = matches!(mode, DisplayMode::Crt(_)) && self.crt.is_some();
        if let (true, Some(crt)) = (crt_shown, self.crt.as_ref()) {
            if !again || made {
                crt.pass.encode(
                    encoder,
                    &crt.target.create_view(&wgpu::TextureViewDescriptor::default()),
                );
            }
        }
        let (picture_w, picture_h) = match self.crt.as_ref().filter(|_| crt_shown) {
            Some(crt) => crt.size,
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
            let (source, size) = match self.crt.as_ref().filter(|_| crt_shown) {
                Some(crt) => (
                    crt.target
                        .create_view(&wgpu::TextureViewDescriptor::default()),
                    (crt.size.0 as u32, crt.size.1 as u32),
                ),
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
                let bind_group = match self.crt.as_ref().filter(|_| crt_shown) {
                    Some(crt) => &crt.bind_group,
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
