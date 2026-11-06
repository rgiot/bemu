//! RGB to RGBA conversion for the Canvas 2D fallback.

use amsrust::display::RAW_WIDTH;
use image::RgbImage;

pub(crate) fn double_scanlines(frame: &RgbImage, rgba: &mut [u8]) {
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let row_bytes = RAW_WIDTH * 4;
    for (y, rows) in rgba.chunks_exact_mut(row_bytes * 2).enumerate() {
        let (row, duplicate) = rows.split_at_mut(row_bytes);
        let line = if y < height {
            &frame.as_raw()[y * width * 3..(y + 1) * width * 3]
        } else {
            &[]
        };
        let pixels = (line.len() / 3).min(RAW_WIDTH);
        for (src, dst) in line[..pixels * 3].chunks_exact(3).zip(row.chunks_exact_mut(4)) {
            dst[..3].copy_from_slice(src);
        }
        row[pixels * 4..].chunks_exact_mut(4).for_each(|pixel| pixel[..3].fill(0));
        // The second output row is the same scanline, so convert RGB only once.
        duplicate.copy_from_slice(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amsrust::display::RAW_HEIGHT;

    #[test]
    fn doubles_and_pads_scanlines_without_leaking_previous_frame() {
        let frame = RgbImage::from_raw(2, 1, vec![1, 2, 3, 4, 5, 6]).unwrap();
        let mut rgba = vec![255; RAW_WIDTH * RAW_HEIGHT * 4];
        double_scanlines(&frame, &mut rgba);
        for y in 0..2 {
            let row = &rgba[y * RAW_WIDTH * 4..(y + 1) * RAW_WIDTH * 4];
            assert_eq!(&row[..12], &[1, 2, 3, 255, 4, 5, 6, 255, 0, 0, 0, 255]);
        }
        assert_eq!(&rgba[2 * RAW_WIDTH * 4..2 * RAW_WIDTH * 4 + 4], &[0, 0, 0, 255]);
        double_scanlines(&RgbImage::new(0, 0), &mut rgba);
        assert_eq!(&rgba[..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn crops_frames_wider_and_taller_than_the_canvas() {
        let mut frame = RgbImage::new((RAW_WIDTH + 1) as u32, (RAW_HEIGHT / 2 + 1) as u32);
        frame.put_pixel((RAW_WIDTH - 1) as u32, (RAW_HEIGHT / 2 - 1) as u32, image::Rgb([7, 8, 9]));
        let mut rgba = vec![255; RAW_WIDTH * RAW_HEIGHT * 4];
        double_scanlines(&frame, &mut rgba);
        let last = rgba.len() - 4;
        assert_eq!(&rgba[last..], &[7, 8, 9, 255]);
    }

    #[test]
    fn matches_per_pixel_conversion_for_different_image_sizes() {
        for (width, height) in [(0, 0), (3, 5), (RAW_WIDTH, RAW_HEIGHT / 2), (RAW_WIDTH + 1, RAW_HEIGHT / 2 + 1)] {
            let frame = RgbImage::from_fn(width as u32, height as u32, |x, y| {
                image::Rgb([x as u8, y as u8, (x ^ y) as u8])
            });
            let mut actual = vec![255; RAW_WIDTH * RAW_HEIGHT * 4];
            let mut expected = actual.clone();
            for y in 0..RAW_HEIGHT {
                for x in 0..RAW_WIDTH {
                    if x < width && y / 2 < height {
                        let at = (y * RAW_WIDTH + x) * 4;
                        expected[at..at + 3].copy_from_slice(&frame.get_pixel(x as u32, (y / 2) as u32).0);
                    } else {
                        let at = (y * RAW_WIDTH + x) * 4;
                        expected[at..at + 3].fill(0);
                    }
                }
            }
            double_scanlines(&frame, &mut actual);
            assert_eq!(actual, expected, "image {width}x{height}");
        }
    }
}
