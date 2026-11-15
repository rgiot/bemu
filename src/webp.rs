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

/// What the CSL runner's screenshots become.
pub enum Pictures {
    /// One PNG file each.
    Png,
    /// The frames of the WebP files, kept here for [`Animation::write`].
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

    /// The animations, each frame shown `frame_ms`: the frames of the picture's own size in
    /// `path` (lossless), those a `screenshot` zoomed down (`screenshot 440px`: thumbnails) in
    /// `<stem>-<width>.webp` beside it, a file per width, lossy (quality 92 for a still, 82 for
    /// the frames of an animation, as the Benediction website shows them). The files written,
    /// with their frame counts.
    pub fn write(&self, path: &Path, frame_ms: u32) -> Result<Vec<(PathBuf, usize)>, String> {
        let frames = self.0.borrow();
        let full = frames.iter().map(RgbImage::width).max().unwrap_or(0);
        // the widths, in the order they came
        let mut widths: Vec<u32> = Vec::new();
        for frame in frames.iter() {
            if !widths.contains(&frame.width()) {
                widths.push(frame.width());
            }
        }
        let mut written = Vec::new();
        for width in widths {
            let group: Vec<RgbImage> = frames
                .iter()
                .filter(|frame| frame.width() == width)
                .cloned()
                .collect();
            let (target, lossy) = if width == full {
                (path.to_path_buf(), None)
            } else {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let quality = if group.len() == 1 { 92.0 } else { 82.0 };
                (
                    path.with_file_name(format!("{stem}-{width}.webp")),
                    Some(quality),
                )
            };
            let bytes = encode_with(&group, frame_ms, lossy)?;
            std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
            written.push((target, group.len()));
        }
        Ok(written)
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
    fn zoomed_screenshots_make_animations_of_their_own() {
        let dir = std::env::temp_dir().join(format!("bemu-anim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let animation = Animation::default();
        let mut sink = PictureFiles(Pictures::Webp(animation.clone()));
        let sized =
            |w: u32, seed: u8| RgbImage::from_fn(w, w / 2, |x, _| image::Rgb([seed, x as u8, 0]));
        // two moments, each the full picture and its thumbnail
        for seed in [10, 200] {
            sink.save(&dir, "a.png", &sized(64, seed)).unwrap();
            sink.save(&dir, "a-16.png", &sized(16, seed)).unwrap();
        }
        let written = animation.write(&dir.join("shot.webp"), 500).unwrap();
        assert_eq!(written.len(), 2);
        assert!(written[0].0.ends_with("shot.webp") && written[0].1 == 2);
        assert!(written[1].0.ends_with("shot-16.webp") && written[1].1 == 2);
        let full = std::fs::read(&written[0].0).unwrap();
        assert_eq!(full.windows(4).filter(|w| *w == b"ANMF").count(), 2);
        // the thumbnails are lossy (VP8 frames, not VP8L)
        let small = std::fs::read(&written[1].0).unwrap();
        assert!(small.windows(4).any(|w| w == b"VP8 "));
        assert!(!full.windows(4).any(|w| w == b"VP8 "));
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
