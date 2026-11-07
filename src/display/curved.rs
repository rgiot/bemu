//! The tube's curved glass on the CPU: [`Curved`] warps the picture of another filter, as the
//! GPU blit (`blit.wgsl`) does.

use super::crt::Warp;
use super::DisplayFilter;
use image::RgbImage;

/// One output pixel: the flat picture's pixels it mixes, and how.
#[derive(Clone, Copy)]
struct Tap {
    /// The top left one of the 2×2 mixed (the others are right of and below it).
    index: u32,
    /// Weights of the right column and of the bottom row, and the glass's coverage, in 256ths.
    fx: u16,
    fy: u16,
    alpha: u16,
}

/// `inner`'s picture seen through the glass `warp`.
pub struct Curved {
    inner: Box<dyn DisplayFilter>,
    flat: Vec<u32>,
    taps: Vec<Tap>,
}

impl Curved {
    pub fn new(inner: Box<dyn DisplayFilter>, warp: Warp) -> Self {
        let (width, height) = inner.output_size();
        let pixel = (1.0 / width as f32, 1.0 / height as f32);
        let taps = (0..width * height)
            .map(|i| {
                let (x, y) = (i % width, i / width);
                let u = (x as f32 + 0.5) * pixel.0;
                let v = (y as f32 + 0.5) * pixel.1;
                let (su, sv, alpha) = warp.source(u, v, pixel);
                // pixel centres, clamped to the picture
                let sx = (su * width as f32 - 0.5).clamp(0.0, (width - 1) as f32);
                let sy = (sv * height as f32 - 0.5).clamp(0.0, (height - 1) as f32);
                let (left, top) = (
                    (sx as usize).min(width.saturating_sub(2)),
                    (sy as usize).min(height.saturating_sub(2)),
                );
                Tap {
                    index: (top * width + left) as u32,
                    fx: ((sx - left as f32) * 256.0).round() as u16,
                    fy: ((sy - top as f32) * 256.0).round() as u16,
                    alpha: (alpha * 256.0).round() as u16,
                }
            })
            .collect();
        Self {
            inner,
            flat: vec![0; width * height],
            taps,
        }
    }
}

/// `a` and `b` mixed, `f` 256ths of `b`, channel by channel (8 bits each, spread out so they
/// don't overflow into each other).
#[inline(always)]
fn mix(a: u32, b: u32, f: u32) -> u32 {
    const RB: u32 = 0x00ff_00ff;
    const G: u32 = 0x0000_ff00;
    let rb = ((a & RB) * (256 - f) + (b & RB) * f) >> 8;
    let g = ((a & G) * (256 - f) + (b & G) * f) >> 8;
    (rb & RB) | (g & G)
}

impl DisplayFilter for Curved {
    fn output_size(&self) -> (usize, usize) {
        self.inner.output_size()
    }

    fn render(&mut self, src: &RgbImage, dst: &mut [u32]) {
        self.inner.render(src, &mut self.flat);
        let (width, _) = self.inner.output_size();
        let (flat, taps) = (&self.flat, &self.taps);
        let bands = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        let band = taps.len().div_ceil(bands).max(1);
        std::thread::scope(|scope| {
            for (dst, taps) in dst.chunks_mut(band).zip(taps.chunks(band)) {
                scope.spawn(move || {
                    for (out, tap) in dst.iter_mut().zip(taps) {
                        let i = tap.index as usize;
                        let top = mix(flat[i], flat[i + 1], tap.fx as u32);
                        let bottom = mix(flat[i + width], flat[i + width + 1], tap.fx as u32);
                        let pixel = mix(top, bottom, tap.fy as u32);
                        *out = mix(0, pixel, tap.alpha as u32);
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::crt::CrtProfile;
    use crate::display::RawFilter;

    #[test]
    fn flat_glass_changes_nothing() {
        let warp = Warp::of(CrtProfile::Ctm644, 0.0);
        assert!(warp.is_flat());
        let (u, v, alpha) = warp.source(0.3, 0.9, (0.001, 0.001));
        assert!((u - 0.3).abs() < 1e-6 && (v - 0.9).abs() < 1e-6 && alpha == 1.0);
    }

    #[test]
    fn curved_glass_keeps_the_centre_and_darkens_the_corners() {
        let mut src = RgbImage::new(768, 272);
        for pixel in src.pixels_mut() {
            pixel.0 = [200, 100, 50];
        }
        let mut curved = Curved::new(Box::new(RawFilter), Warp::of(CrtProfile::Ctm644, 1.0));
        let (width, height) = curved.output_size();
        let mut dst = vec![0u32; width * height];
        curved.render(&src, &mut dst);
        assert_eq!(dst[height / 2 * width + width / 2], 0x00c8_6432);
        assert_eq!(dst[0], 0);
        assert_eq!(dst[width * height - 1], 0);
        // the middle of an edge bulges out: still lit
        assert_eq!(dst[height / 2 * width], 0x00c8_6432);
    }
}
