//! Post-processing of the monitor's frames for display and recording.
//!
//! The [`crate::monitor::Monitor`] produces an "ideal" picture, one pixel per
//! 16MHz tick; a [`DisplayFilter`] turns it into what is shown (and recorded).
//! [`RawFilter`] is that ideal picture, with its scanlines doubled; other
//! filters approximate a real CRT. Filters only ever see completed frames, so
//! the screenshots of the test suites (which use the monitor's frames
//! directly) are not affected.

// The CRT is written once, as a CubeCL kernel (`kernel.rs`): it runs on the GPU (Vulkan, Metal,
// DX12 and a page's WebGPU, drawn by `CrtPass`) and, for the filters that read the picture back
// (`KernelCrt`: recordings, screenshots), on CubeCL's CPU backend when there is no GPU.
pub mod crt;
#[cfg(feature = "crt-gpu")]
mod gpu;
#[cfg(feature = "crt-gpu")]
mod kernel;
#[cfg(all(feature = "crt-gpu", not(target_arch = "wasm32")))]
mod kernel_crt;
#[cfg(feature = "crt-gpu")]
mod picture;
pub mod resize;

pub use crt::{CrtParams, CrtProfile, Warp};
#[cfg(all(feature = "crt-gpu", not(target_arch = "wasm32")))]
pub use gpu::wait_for;
#[cfg(all(feature = "crt-gpu", not(target_arch = "wasm32")))]
pub use kernel_crt::{Backend, KernelCrt};
#[cfg(feature = "crt-gpu")]
pub use gpu::{Blit, CrtPass, FrameTexture, GpuContext, GpuFrame, Resampler};
#[cfg(feature = "crt-gpu")]
pub use picture::{picture_scale, Picture};

use image::RgbImage;
use std::str::FromStr;

/// Width of the raw output: the monitor's active width (`48 * 8 * 2`).
pub const RAW_WIDTH: usize = 48 * 8 * 2; // 768
/// Height of the raw output: each of the monitor's 272 lines drawn twice,
/// for a roughly correct 4:3 aspect instead of a squashed one.
pub const RAW_HEIGHT: usize = 272 * 2;

/// How large the picture is shown in its room.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PictureSize {
    /// A whole factor of the picture (the profile's resolution for a CRT): every CPC pixel the
    /// same size, the largest that fits.
    #[default]
    Whole,
    /// A whole factor too, but a CRT is drawn directly at that resolution (its slots and
    /// scanlines finer) instead of enlarging the picture of the profile.
    Integer,
    /// The largest size that fits, by any factor: a CRT is drawn at the finest resolution that
    /// covers the room, then resized without moiré.
    Maximal,
}

impl PictureSize {
    /// From the settings' two switches (`Maximal` first).
    pub fn of(maximal: bool, integer: bool) -> Self {
        match (maximal, integer) {
            (true, _) => Self::Maximal,
            (false, true) => Self::Integer,
            (false, false) => Self::Whole,
        }
    }
}

/// What `--display` selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayMode {
    /// The monitor's ideal picture, scanlines doubled.
    #[default]
    Raw,
    /// An approximation of a CRT, see [`crt`].
    Crt(CrtProfile),
}

impl DisplayMode {
    pub const NAMES: &'static str = "raw|ctm|trinitron|gt65|mm12|atari";

    pub fn is_raw(self) -> bool {
        self == Self::Raw
    }

    /// Size of the pictures of this mode.
    pub fn output_size(self) -> (usize, usize) {
        match self {
            Self::Raw => (RAW_WIDTH, RAW_HEIGHT),
            Self::Crt(profile) => profile.params().output_size(),
        }
    }

    /// The next mode, cycling through all of them (what F9 does).
    pub fn next(self) -> Self {
        match self {
            Self::Raw => Self::Crt(CrtProfile::Ctm644),
            Self::Crt(CrtProfile::Ctm644) => Self::Crt(CrtProfile::Trinitron),
            Self::Crt(CrtProfile::Trinitron) => Self::Crt(CrtProfile::Gt65),
            Self::Crt(CrtProfile::Gt65) => Self::Crt(CrtProfile::Mm12),
            Self::Crt(CrtProfile::Mm12) => Self::Crt(CrtProfile::AtariColor),
            Self::Crt(CrtProfile::AtariColor) => Self::Raw,
        }
    }

    /// The filter implementing this mode, on the CPU (CubeCL's CPU backend: no adapter needed).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn filter(self) -> Box<dyn DisplayFilter> {
        self.filter_on(false, 0.0)
    }

    /// The filter implementing this mode: the CRT kernels on the GPU if `gpu` and possible (an
    /// adapter is available), else on the CPU backend (the other way round if `gpu` is false: the
    /// CPU's picture does not depend on the machine). A CRT is seen through its glass, curved
    /// `curvature` times its profile's bulge (0: flat), see [`Warp`]. The raw picture where no CRT
    /// can be rendered (built without `crt-gpu`, no adapter and no software rendering).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn filter_on(self, gpu: bool, curvature: f32) -> Box<dyn DisplayFilter> {
        let warp = self.warp(curvature);
        if let Self::Crt(profile) = self {
            #[cfg(feature = "crt-gpu")]
            {
                let order = if gpu {
                    [Backend::Gpu, Backend::Cpu]
                } else {
                    [Backend::Cpu, Backend::Gpu]
                };
                let mut errors = Vec::new();
                for backend in order {
                    match KernelCrt::new(profile.params(), warp, backend) {
                        Ok(filter) => return Box::new(filter),
                        Err(e) => errors.push(e),
                    }
                }
                eprintln!("no CRT rendering ({}): the raw picture is shown", errors.join("; "));
            }
            #[cfg(not(feature = "crt-gpu"))]
            {
                let _ = (profile, warp, gpu);
                eprintln!("built without the crt-gpu feature: the raw picture is shown");
            }
            return Box::new(RawFilter);
        }
        // raw: the doubled picture, never curved (see `warp`)
        Box::new(RawFilter)
    }

    /// The glass of this mode, curved `curvature` times its profile's bulge: flat for raw.
    pub fn warp(self, curvature: f32) -> Warp {
        match self {
            Self::Raw => Warp::FLAT,
            Self::Crt(profile) => Warp::of(profile, curvature),
        }
    }
}

impl FromStr for DisplayMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "raw" => Ok(Self::Raw),
            other => other
                .parse()
                .map(Self::Crt)
                .map_err(|_| format!("unknown display {other:?} (expected {})", Self::NAMES)),
        }
    }
}

/// Turns a monitor frame into the picture to display, in 0RGB pixels.
pub trait DisplayFilter {
    /// `(width, height)` of the picture produced for the monitor's frames.
    /// Fixed over the filter's life, so a window or a recording can be sized
    /// from it.
    fn output_size(&self) -> (usize, usize);

    /// Fill `dst` (`width * height` pixels, see [`Self::output_size`]) from
    /// `src`. A `src` smaller than expected is letterboxed in black.
    fn render(&mut self, src: &RgbImage, dst: &mut [u32]);

    /// A freshly allocated, filtered `src`.
    fn render_to_image(&mut self, src: &RgbImage) -> RgbImage {
        let (width, height) = self.output_size();
        let mut buffer = vec![0u32; width * height];
        self.render(src, &mut buffer);
        let mut image = RgbImage::new(width as u32, height as u32);
        for (pixel, packed) in image.pixels_mut().zip(&buffer) {
            pixel.0 = [(packed >> 16) as u8, (packed >> 8) as u8, *packed as u8];
        }
        image
    }
}

/// The monitor's picture as is, each scanline drawn twice.
pub struct RawFilter;

impl DisplayFilter for RawFilter {
    fn output_size(&self) -> (usize, usize) {
        (RAW_WIDTH, RAW_HEIGHT)
    }

    fn render(&mut self, src: &RgbImage, dst: &mut [u32]) {
        dst.fill(0);
        let width = (src.width() as usize).min(RAW_WIDTH);
        let height = (src.height() as usize).min(RAW_HEIGHT / 2);
        for y in 0..height {
            for x in 0..width {
                let pixel = src.get_pixel(x as u32, y as u32);
                let packed = (pixel[0] as u32) << 16 | (pixel[1] as u32) << 8 | pixel[2] as u32;
                dst[(y * 2) * RAW_WIDTH + x] = packed;
                dst[(y * 2 + 1) * RAW_WIDTH + x] = packed;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_size_comes_from_the_two_switches() {
        assert_eq!(PictureSize::of(false, false), PictureSize::Whole);
        assert_eq!(PictureSize::of(false, true), PictureSize::Integer);
        assert_eq!(PictureSize::of(true, true), PictureSize::Maximal);
    }

    #[test]
    fn raw_doubles_each_line_and_letterboxes() {
        let mut src = RgbImage::new(2, 1);
        src.put_pixel(0, 0, image::Rgb([1, 2, 3]));
        src.put_pixel(1, 0, image::Rgb([255, 0, 0]));
        let mut dst = vec![7u32; RAW_WIDTH * RAW_HEIGHT];
        RawFilter.render(&src, &mut dst);
        assert_eq!(dst[0], 0x010203);
        assert_eq!(dst[1], 0xFF0000);
        assert_eq!(dst[RAW_WIDTH], 0x010203);
        assert_eq!(dst[2], 0);
        assert_eq!(dst[2 * RAW_WIDTH], 0);
    }

    #[test]
    fn mode_parses() {
        assert_eq!("raw".parse::<DisplayMode>(), Ok(DisplayMode::Raw));
        assert_eq!(
            "ctm".parse::<DisplayMode>(),
            Ok(DisplayMode::Crt(CrtProfile::Ctm644))
        );
        // GT64 and GT65 are the same green tube
        assert_eq!(
            "gt64".parse::<DisplayMode>(),
            Ok(DisplayMode::Crt(CrtProfile::Gt65))
        );
        assert_eq!(
            "mm12".parse::<DisplayMode>(),
            Ok(DisplayMode::Crt(CrtProfile::Mm12))
        );
        assert!("nope".parse::<DisplayMode>().is_err());
    }

    #[test]
    fn next_cycles_through_every_mode() {
        let start = DisplayMode::Raw;
        let mut mode = start.next();
        let mut seen = vec![start];
        while mode != start {
            assert!(!seen.contains(&mode));
            seen.push(mode);
            mode = mode.next();
        }
        assert_eq!(seen.len(), 6);
    }

    #[test]
    fn render_to_image_has_the_output_size() {
        let image = RawFilter.render_to_image(&RgbImage::new(768, 272));
        assert_eq!(
            (image.width() as usize, image.height() as usize),
            (RAW_WIDTH, RAW_HEIGHT)
        );
    }
}
