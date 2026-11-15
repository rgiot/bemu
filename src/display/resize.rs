//! A picture resized without moiré, the way of the Benediction website's thumbnails: a Gaussian
//! blur in proportion to the reduction, then Lanczos (a = 3). The CTM's scanlines and phosphor
//! mask have a period near a pixel: shrunk as they are, they beat into stripes. The GPU does it
//! for the window and the page (`Resampler`, `resample.wgsl`), [`resize`] for files (the
//! thumbnails of `--thumbnails`); [`blur_sigma`] is their common rule.

use image::RgbImage;

/// The Gaussian blur before a reduction by `reduction` (source pixels per output pixel), in
/// source pixels: none when enlarging. 0.37 source pixel per unit of reduction for the
/// thumbnails (3 times smaller and more, the mask far below a pixel), up to 0.6 for a small
/// reduction (a window a little smaller than the picture), where the mask falls just above two
/// output pixels, at the edge of what a screen shows, and 0.37 leaves it beating against them.
pub fn blur_sigma(reduction: f32) -> f32 {
    if reduction <= 1.0 {
        return 0.0;
    }
    // 0.6 up to a reduction of 1.5, 0.37 from 3, in between linearly
    let t = ((reduction - 1.5) / 1.5).clamp(0.0, 1.0);
    (0.6 + (0.37 - 0.6) * t) * reduction
}

/// `picture` resized to `width` x `height`: blurred by [`blur_sigma`] of the larger reduction,
/// then resampled with Lanczos.
pub fn resize(picture: &RgbImage, width: u32, height: u32) -> RgbImage {
    let reduction = (picture.width() as f32 / width.max(1) as f32)
        .max(picture.height() as f32 / height.max(1) as f32);
    let sigma = blur_sigma(reduction);
    let blurred;
    let source = if sigma > 0.0 {
        blurred = image::imageops::blur(picture, sigma);
        &blurred
    } else {
        picture
    };
    image::imageops::resize(source, width, height, image::imageops::FilterType::Lanczos3)
}

/// The height of `picture` at `width`, its proportions kept.
pub fn height_at(picture: &RgbImage, width: u32) -> u32 {
    ((picture.height() as f64 * width as f64 / picture.width().max(1) as f64).round() as u32).max(1)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_blur_follows_the_reduction() {
        assert_eq!(blur_sigma(1.0), 0.0);
        assert_eq!(blur_sigma(0.5), 0.0);
        assert!((blur_sigma(1.4) - 0.6 * 1.4).abs() < 1e-6);
        // the website's thumbnails: 0.37 per unit
        assert!((blur_sigma(3.27) - 0.37 * 3.27).abs() < 1e-6);
        assert!(blur_sigma(2.25) > 0.37 * 2.25 && blur_sigma(2.25) < 0.6 * 2.25);
    }

    #[test]
    fn a_mask_does_not_beat_once_shrunk() {
        // a CTM-like mask: columns of period 3 (bright, bright, dark)
        let mask = RgbImage::from_fn(1440, 64, |x, _| {
            let v = if x % 3 == 2 { 40 } else { 220 };
            image::Rgb([v, v, v])
        });
        for width in [1000, 440] {
            let small = resize(&mask, width, height_at(&mask, width));
            // the columns of a middle row: nearly even, no stripes left
            let row: Vec<f32> = (8..width - 8)
                .map(|x| small.get_pixel(x, small.height() / 2)[0] as f32)
                .collect();
            let mean = row.iter().sum::<f32>() / row.len() as f32;
            let spread = row.iter().map(|v| (v - mean).abs()).fold(0.0, f32::max);
            // the mask's contrast is 180: at most a tenth of it left
            assert!(spread < 18.0, "{width}: the mask still beats by {spread}");
        }
    }
}
