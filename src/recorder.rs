//! Video capture for the `amsrust` binary's `--record` option (part of the
//! binary, not of the library).
//!
//! Every frame the monitor completes is piped as raw RGB to an external
//! `ffmpeg`, at the CPC's own frame rate: a frame is 312 lines of 64us,
//! 19968us, i.e. 15625/312 (~50.08) frames per second. The frames are the
//! ones the `--display` filter produces, as in the window. The PSG samples go to a
//! temporary raw file; when the recording ends a second `ffmpeg` pass muxes
//! them with the video. Both are produced in emulated time, so picture and
//! sound stay in sync even when the emulation runs slower than real time.
//!
//! The container is chosen by `ffmpeg` from the output file's extension (`.mp4`,
//! `.mkv`, `.webm`, ...). For `.mp4`, `.mkv` and `.mov` the final pass follows ACE's
//! settings: 2160 lines high, bt601 -> bt709, x264 at CRF 25, MP3 audio. Its crop is not
//! applied (our frames are the `--display` filter's), only its display aspect, which
//! differs on a CPC+ (its wider picture).

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use amsrust::psg::AudioConfig;

/// The CPC frame rate, as a fraction ffmpeg understands.
const FRAME_RATE: &str = "15625/312";

/// The height of the final video.
const VIDEO_HEIGHT: usize = 2160;

/// ACE's output sizes (3008x2160 and 3064x2160): the display aspect of the picture.
const CPC_ASPECT: (usize, usize) = (3008, 2160);
const PLUS_ASPECT: (usize, usize) = (3064, 2160);

pub struct Recorder {
    output: PathBuf,
    width: usize,
    height: usize,
    /// The encoder fed with raw frames (video only).
    encoder: Child,
    frames_in: BufWriter<ChildStdin>,
    video_only: PathBuf,
    audio: Option<(AudioConfig, PathBuf, BufWriter<std::fs::File>)>,
    plus: bool,
    frames: usize,
    /// Reused for the RGB bytes of each frame.
    rgb: Vec<u8>,
}

impl Recorder {
    /// Start recording `width`x`height` frames, with the PSG's samples in
    /// the `audio` format if any. `plus` is for a CPC+, whose picture is wider.
    pub fn start(
        output: &Path,
        width: usize,
        height: usize,
        audio: Option<AudioConfig>,
        plus: bool,
    ) -> Result<Self, String> {
        check_ffmpeg()?;
        let video_only = temporary(output, "video.mkv");
        let size = format!("{width}x{height}");
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
            ])
            .args(["-s", &size, "-framerate", FRAME_RATE, "-i", "-"])
            // Lossless intermediate: the final pass picks the real codec.
            .args(["-c:v", "ffv1"])
            .arg(&video_only)
            .stdin(Stdio::piped());
        detach_from_terminal_signals(&mut command);
        let encoder = command
            .spawn()
            .map_err(|e| format!("cannot start ffmpeg: {e}"))?;
        let mut encoder = encoder;
        let frames_in = BufWriter::new(encoder.stdin.take().unwrap());

        let audio = match audio {
            Some(config) => {
                let path = temporary(output, "audio.raw");
                let file =
                    std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                Some((config, path, BufWriter::new(file)))
            }
            None => None,
        };

        Ok(Self {
            output: output.to_owned(),
            width,
            height,
            encoder,
            frames_in,
            video_only,
            audio,
            plus,
            frames: 0,
            rgb: Vec::with_capacity(width * height * 3),
        })
    }

    /// Add one frame of 0RGB pixels, `width * height` of them.
    pub fn frame(&mut self, pixels: &[u32]) -> Result<(), String> {
        assert_eq!(
            pixels.len(),
            self.width * self.height,
            "the frame is not the recording's size"
        );
        self.rgb.clear();
        for &pixel in pixels {
            self.rgb
                .extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]);
        }
        self.frames_in
            .write_all(&self.rgb)
            .map_err(|e| format!("ffmpeg stopped: {e}"))?;
        self.frames += 1;
        Ok(())
    }

    /// Add PSG samples (interleaved as in the recording's `AudioConfig`).
    pub fn samples(&mut self, samples: &[f32]) -> Result<(), String> {
        let Some((_, path, out)) = &mut self.audio else {
            return Ok(());
        };
        for &sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            out.write_all(&value.to_le_bytes())
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(())
    }

    /// Close the encoder and write the final file. Returns the number of
    /// frames recorded.
    pub fn finish(mut self) -> Result<usize, String> {
        self.frames_in
            .flush()
            .map_err(|e| format!("ffmpeg stopped: {e}"))?;
        drop(self.frames_in);
        let status = self.encoder.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!(
                "the ffmpeg video encoder failed ({status}) while recording {}; its own message, if any, is above. \
                 The frames received so far are in {}",
                self.output.display(),
                self.video_only.display()
            ));
        }

        let mut mux = Command::new("ffmpeg");
        mux.args(["-y", "-loglevel", "error", "-i"])
            .arg(&self.video_only);
        if let Some((config, path, mut out)) = self.audio.take() {
            out.flush()
                .map_err(|e| format!("{}: {e}", path.display()))?;
            mux.args(["-f", "s16le", "-ar", &config.sample_rate.to_string()])
                .args(["-ac", &config.nb_channels().to_string(), "-i"])
                .arg(&path);
            self.audio = Some((config, path, out));
        }
        if uses_ace_settings(&self.output) {
            let (aspect_width, aspect_height) = if self.plus { PLUS_ASPECT } else { CPC_ASPECT };
            // an even width, as yuv420p needs
            let video_width = VIDEO_HEIGHT * aspect_width / aspect_height / 2 * 2;
            let filter = format!(
                "colorspace=all=bt709:iall=bt601-6-625:fast=1,scale={video_width}:{VIDEO_HEIGHT},setsar=1:1"
            );
            mux.args(["-vf", &filter])
                .args([
                    "-colorspace",
                    "1",
                    "-color_primaries",
                    "1",
                    "-color_trc",
                    "1",
                ])
                .args(["-vcodec", "libx264", "-crf", "25"]);
            if self.audio.is_some() {
                mux.args(["-codec:a", "libmp3lame", "-qscale:a", "0"]);
            }
        }
        // yuv420p: the pixel format every player can read.
        mux.args(["-pix_fmt", "yuv420p"]).arg(&self.output);
        let status = mux
            .status()
            .map_err(|e| format!("cannot start ffmpeg: {e}"))?;

        let _ = std::fs::remove_file(&self.video_only);
        if let Some((_, path, _)) = &self.audio {
            let _ = std::fs::remove_file(path);
        }
        if !status.success() {
            return Err(format!(
                "ffmpeg failed to write {} ({status}); its own message, if any, is above",
                self.output.display()
            ));
        }
        Ok(self.frames)
    }
}

/// Ctrl+C is sent to the whole foreground process group: without this the
/// encoder would be killed at the same time as the emulator, before the last
/// frames are flushed, and fail with an unexplained exit status 255.
fn detach_from_terminal_signals(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
}

/// The containers the x264 + MP3 settings suit; the others (`.webm`...) keep ffmpeg's codecs.
fn uses_ace_settings(output: &Path) -> bool {
    output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["mp4", "mkv", "mov"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

fn check_ffmpeg() -> Result<(), String> {
    match Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        _ => Err("--record needs ffmpeg in the PATH (https://ffmpeg.org)".to_owned()),
    }
}

/// A scratch file next to the output: `<output>.<suffix>`.
fn temporary(output: &Path, suffix: &str) -> PathBuf {
    let mut name = output.file_name().unwrap_or_default().to_owned();
    name.push(format!(".{suffix}"));
    output.with_file_name(name)
}
