//! Lossless WebP pictures (the screenshots of a CSL run): a still, or an animation of whole frames.
//!
//! Through libwebp (`webp-animation`), at its best lossless setting.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use image::RgbImage;

use crate::csl_runner::ScreenshotSink;

/// An encoder at libwebp's best setting: lossless, or lossy at `quality` (0-100).
fn encoder(width: u32, height: u32, lossy: Option<f32>) -> Result<webp_animation::Encoder, String> {
    use webp_animation::prelude::*;
    Encoder::new_with_options(
        (width, height),
        EncoderOptions {
            encoding_config: Some(EncodingConfig {
                encoding_type: match lossy {
                    Some(_) => EncodingType::Lossy(LossyEncodingConfig::default()),
                    None => EncodingType::Lossless,
                },
                quality: lossy.unwrap_or(100.0),
                method: 6,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())
}

fn rgba(image: &RgbImage) -> Vec<u8> {
    image
        .pixels()
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}

/// `frames` (all of the same size) as a lossless WebP file: a still for one frame, else a looping
/// animation, each frame shown `frame_ms`.
pub fn encode(frames: &[RgbImage], frame_ms: u32) -> Result<Vec<u8>, String> {
    encode_with(frames, frame_ms, None)
}

/// [`encode`], lossy at `quality` (0-100) when given.
fn encode_with(frames: &[RgbImage], frame_ms: u32, lossy: Option<f32>) -> Result<Vec<u8>, String> {
    let first = frames.first().ok_or("no frame to encode")?;
    let (width, height) = first.dimensions();
    let mut encoder = encoder(width, height, lossy)?;
    for (n, frame) in frames.iter().enumerate() {
        if frame.dimensions() != (width, height) {
            return Err("the frames of an animation must have the same size".into());
        }
        encoder
            .add_frame(&rgba(frame), (n as u32 * frame_ms) as i32)
            .map_err(|e| e.to_string())?;
    }
    let data = encoder
        .finalize((frames.len() as u32 * frame_ms) as i32)
        .map_err(|e| e.to_string())?;
    Ok(data.to_vec())
}

/// The frames of a WebP file (one for a still).
pub fn decode(data: &[u8]) -> Result<Vec<RgbImage>, String> {
    let decoder = webp_animation::Decoder::new(data).map_err(|e| e.to_string())?;
    decoder
        .into_iter()
        .map(|frame| {
            let (width, height) = frame.dimensions();
            let rgb: Vec<u8> = frame
                .data()
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            RgbImage::from_raw(width, height, rgb).ok_or_else(|| "a frame of the wrong size".into())
        })
        .collect()
}

/// The thumbnails of a screenshot (a PNG, or a WebP still or animation): one `<name>-<width>.webp`
/// per width in `out_dir`, resized without moiré ([`crate::display::resize`]), lossy (quality 92
/// for a still, 82 for the frames of an animation, each shown `frame_ms`), as the Benediction
/// website shows them. The files written.
pub fn thumbnails(
    source: &Path,
    out_dir: &Path,
    widths: &[u32],
    frame_ms: u32,
) -> Result<Vec<PathBuf>, String> {
    use crate::display::resize::{height_at, resize};
    let bytes = std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let frames = if bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        decode(&bytes)?
    } else {
        vec![image::load_from_memory(&bytes)
            .map_err(|e| format!("{}: {e}", source.display()))?
            .to_rgb8()]
    };
    let stem = source
        .file_stem()
        .ok_or_else(|| format!("{}: no file name", source.display()))?
        .to_string_lossy();
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let quality = if frames.len() == 1 { 92.0 } else { 82.0 };
    let mut written = Vec::new();
    for &width in widths {
        let small: Vec<RgbImage> = frames
            .iter()
            .map(|frame| resize(frame, width, height_at(frame, width)))
            .collect();
        let target = out_dir.join(format!("{stem}-{width}.webp"));
        let data = encode_with(&small, frame_ms, Some(quality))?;
        std::fs::write(&target, data).map_err(|e| format!("{}: {e}", target.display()))?;
        written.push(target);
    }
    Ok(written)
}

/// What the CSL runner's screenshots become.
pub enum Pictures {
    /// One PNG file each.
    Png,
    /// The frames of one lossless WebP file, kept here for [`Animation::write`].
    Webp(Animation),
}

/// The frames of a WebP file (an animation, or a still of one frame), shared between the sink and who writes the file at the end.
#[derive(Clone, Default)]
pub struct Animation(Rc<RefCell<Vec<RgbImage>>>);

impl Animation {
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The animation, each frame shown `frame_ms`, in `path`.
    pub fn write(&self, path: &Path, frame_ms: u32) -> Result<(), String> {
        let bytes = encode(&self.0.borrow(), frame_ms)?;
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// A [`ScreenshotSink`] writing PNG or WebP files, or collecting the frames of an animation.
pub struct PictureFiles(pub Pictures);

impl ScreenshotSink for PictureFiles {
    fn save(&mut self, dir: &Path, file: &str, frame: &RgbImage) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        match &self.0 {
            Pictures::Png => {
                let path = dir.join(file);
                frame.save(&path).map_err(|e| e.to_string())?;
                Ok(path)
            }
            Pictures::Webp(animation) => {
                animation.0.borrow_mut().push(frame.clone());
                Ok(dir.join(file))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(seed: u8) -> RgbImage {
        RgbImage::from_fn(17, 9, |x, y| {
            image::Rgb([x as u8 * 9 + seed, y as u8 * 20, seed])
        })
    }

    #[test]
    fn thumbnails_are_made_of_each_frame() {
        let dir = std::env::temp_dir().join(format!("bemu-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let full = dir.join("shot.webp");
        let frame =
            |n: u8| RgbImage::from_fn(880, 544, |x, _| image::Rgb([n, (x % 3) as u8 * 100, 0]));
        std::fs::write(&full, encode(&[frame(10), frame(200)], 500).unwrap()).unwrap();
        let written = thumbnails(&full, &dir.join("thumbs"), &[440, 220], 500).unwrap();
        assert_eq!(written.len(), 2);
        let small = decode(&std::fs::read(&written[0]).unwrap()).unwrap();
        assert_eq!(small.len(), 2);
        assert_eq!(small[0].dimensions(), (440, 272));
        assert!(written[1].ends_with("shot-220.webp"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_still_is_a_riff_webp() {
        let webp = encode(&[picture(1)], 0).unwrap();
        assert_eq!(&webp[..4], b"RIFF");
        assert_eq!(&webp[8..12], b"WEBP");
    }

    #[test]
    fn an_animation_has_a_frame_per_picture() {
        let webp = encode(&[picture(1), picture(2), picture(3)], 40).unwrap();
        assert_eq!(&webp[8..12], b"WEBP");
        assert_eq!(webp.windows(4).filter(|w| *w == b"ANMF").count(), 3);
    }

    #[test]
    fn frames_of_different_sizes_are_refused() {
        assert!(encode(&[picture(1), RgbImage::new(3, 3)], 40).is_err());
    }
}
