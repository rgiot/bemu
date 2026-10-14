//! Video capture for the `amsrust` binary's `--record` option (part of the
//! binary, not of the library).
//!
//! Every frame the monitor completes is piped as raw RGB to an external
//! `ffmpeg`, at the CPC's own frame rate: a frame is 312 lines of 64us,
//! 19968us, i.e. 15625/312 (~50.08) frames per second. Each scanline is
//! doubled, as in the window, for a 4:3 picture. The PSG samples go to a
//! temporary raw file; when the recording ends a second `ffmpeg` pass muxes
//! them with the video. Both are produced in emulated time, so picture and
//! sound stay in sync even when the emulation runs slower than real time.
//!
//! The container and codecs are chosen by `ffmpeg` from the output file's
//! extension (`.mp4`, `.mkv`, `.webm`, ...).

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use amsrust::psg::AudioConfig;

/// The CPC frame rate, as a fraction ffmpeg understands.
const FRAME_RATE: &str = "15625/312";

pub struct Recorder {
    output: PathBuf,
    width: usize,
    height: usize,
    /// The encoder fed with raw frames (video only).
    encoder: Child,
    frames_in: BufWriter<ChildStdin>,
    video_only: PathBuf,
    audio: Option<(AudioConfig, PathBuf, BufWriter<std::fs::File>)>,
    frames: usize,
    /// Reused for the doubled RGB rows of each frame.
    rgb: Vec<u8>
}

impl Recorder {
    /// Start recording `width`x`height` frames (before line doubling), with
    /// the PSG's samples in the `audio` format if any.
    pub fn start(output: &Path, width: usize, height: usize, audio: Option<AudioConfig>) -> Result<Self, String> {
        check_ffmpeg()?;
        let video_only = temporary(output, "video.mkv");
        let size = format!("{width}x{}", height * 2);
        let mut command = Command::new("ffmpeg");
        command
            .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24"])
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
                let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                Some((config, path, BufWriter::new(file)))
            },
            None => None
        };

        Ok(Self {
            output: output.to_owned(),
            width,
            height,
            encoder,
            frames_in,
            video_only,
            audio,
            frames: 0,
            rgb: Vec::with_capacity(width * height * 2 * 3)
        })
    }

    /// Add one frame (any size: cropped or padded with black to the
    /// recording's).
    pub fn frame(&mut self, frame: &image::RgbImage) -> Result<(), String> {
        self.rgb.clear();
        let row_bytes = self.width * 3;
        let copied = (frame.width() as usize).min(self.width) * 3;
        for y in 0..self.height {
            let start = self.rgb.len();
            if y < frame.height() as usize {
                let row = &frame.as_raw()[y * frame.width() as usize * 3..][..copied];
                self.rgb.extend_from_slice(row);
            }
            self.rgb.resize(start + row_bytes, 0);
            self.rgb.extend_from_within(start..start + row_bytes);
        }
        self.frames_in
            .write_all(&self.rgb)
            .map_err(|e| format!("ffmpeg stopped: {e}"))?;
        self.frames += 1;
        Ok(())
    }

    /// Add PSG samples (interleaved as in the recording's `AudioConfig`).
    pub fn samples(&mut self, samples: &[f32]) -> Result<(), String> {
        let Some((_, path, out)) = &mut self.audio
        else {
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
        self.frames_in.flush().map_err(|e| format!("ffmpeg stopped: {e}"))?;
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
        mux.args(["-y", "-loglevel", "error", "-i"]).arg(&self.video_only);
        if let Some((config, path, mut out)) = self.audio.take() {
            out.flush().map_err(|e| format!("{}: {e}", path.display()))?;
            mux.args(["-f", "s16le", "-ar", &config.sample_rate.to_string()])
                .args(["-ac", &config.nb_channels().to_string(), "-i"])
                .arg(&path);
            self.audio = Some((config, path, out));
        }
        // yuv420p: the pixel format every player can read.
        mux.args(["-pix_fmt", "yuv420p"]).arg(&self.output);
        let status = mux.status().map_err(|e| format!("cannot start ffmpeg: {e}"))?;

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

fn check_ffmpeg() -> Result<(), String> {
    match Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).status() {
        Ok(status) if status.success() => Ok(()),
        _ => Err("--record needs ffmpeg in the PATH (https://ffmpeg.org)".to_owned())
    }
}

/// A scratch file next to the output: `<output>.<suffix>`.
fn temporary(output: &Path, suffix: &str) -> PathBuf {
    let mut name = output.file_name().unwrap_or_default().to_owned();
    name.push(format!(".{suffix}"));
    output.with_file_name(name)
}
