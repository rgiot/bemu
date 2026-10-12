//! Sound output for the `amsrust` binary's `--window` mode (part of the
//! binary, not of the library: the library only produces samples, see
//! `amsrust::psg::AudioConfig`).
//!
//! The emulator pushes the PSG samples of each video frame into a
//! [`FrameQueue`]; the audio device's callback pulls them from its own
//! thread. With the `audio` cargo feature, [`AudioOut`] opens the default
//! output device through cpal. Its clock then paces the emulation: the
//! window loop waits while more than [`TARGET_LATENCY`] of sound is
//! queued, instead of relying on minifb's 50Hz frame limiter, so the
//! emulated machine runs exactly at the speed the device plays it.
//!
//! When the emulator cannot keep up with real time the queue runs dry:
//! the last frame is then held (no click) until samples come again, which
//! is heard as gaps, never as the wrong pitch.

#![cfg_attr(not(feature = "audio"), allow(dead_code))]

use std::collections::VecDeque;
use std::time::Duration;

/// How much sound the window loop keeps queued ahead of the device.
pub const TARGET_LATENCY: Duration = Duration::from_millis(60);
/// Beyond this, the oldest samples are dropped so the latency can't grow.
pub const MAX_LATENCY: Duration = Duration::from_millis(250);

/// Interleaved samples (`channels` per frame) between the emulator and the
/// audio device.
pub struct FrameQueue {
    channels: usize,
    samples: VecDeque<f32>,
    /// What the device gets while the queue is empty.
    last_frame: Vec<f32>,
    max_frames: usize,
    /// Device frames that had to be filled with `last_frame`.
    missing_frames: u64
}

impl FrameQueue {
    pub fn new(channels: usize, max_frames: usize) -> Self {
        assert!(channels > 0);
        Self {
            channels,
            samples: VecDeque::with_capacity(max_frames * channels),
            last_frame: vec![0.0; channels],
            max_frames,
            missing_frames: 0
        }
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    pub fn missing_frames(&self) -> u64 {
        self.missing_frames
    }

    /// Queue whole frames, dropping the oldest ones beyond `max_frames`.
    pub fn push(&mut self, samples: &[f32]) {
        debug_assert_eq!(samples.len() % self.channels, 0);
        self.samples.extend(samples);
        let excess = self.frames().saturating_sub(self.max_frames);
        self.samples.drain(..excess * self.channels);
    }

    /// Fill `out`, interleaved with `device_channels` per frame: mono is
    /// copied to every device channel; stereo goes to the first two and
    /// any further channel gets their average.
    pub fn pop_into(&mut self, out: &mut [f32], device_channels: usize) {
        for frame in out.chunks_mut(device_channels) {
            if self.samples.len() >= self.channels {
                for sample in self.last_frame.iter_mut() {
                    *sample = self.samples.pop_front().unwrap();
                }
            }
            else {
                self.missing_frames += 1;
            }
            match self.last_frame[..] {
                [mono] => frame.fill(mono),
                [left, right, ..] => {
                    for (channel, sample) in frame.iter_mut().enumerate() {
                        *sample = match channel {
                            0 => left,
                            1 => right,
                            _ => (left + right) / 2.0
                        };
                    }
                },
                [] => unreachable!()
            }
        }
    }
}

#[cfg(feature = "audio")]
pub use device::AudioOut;

#[cfg(feature = "audio")]
mod device {
    use super::{FrameQueue, MAX_LATENCY};
    use amsrust::psg::AudioConfig;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SampleFormat, SizedSample};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// The default output device, playing a [`FrameQueue`].
    pub struct AudioOut {
        config: AudioConfig,
        queue: Arc<Mutex<FrameQueue>>,
        // Playback stops when the stream is dropped.
        _stream: cpal::Stream
    }

    impl AudioOut {
        pub fn open() -> Result<Self, String> {
            let device = cpal::default_host()
                .default_output_device()
                .ok_or("no audio output device")?;
            let supported = device.default_output_config().map_err(|e| e.to_string())?;
            let stream_config = supported.config();
            let rate = stream_config.sample_rate;
            // Match the device's own rate: no second resampling stage.
            let config = if stream_config.channels >= 2 {
                AudioConfig::stereo(rate)
            }
            else {
                AudioConfig::mono(rate)
            };
            let max_frames = (rate as f64 * MAX_LATENCY.as_secs_f64()) as usize;
            let queue = Arc::new(Mutex::new(FrameQueue::new(config.nb_channels(), max_frames)));

            let stream = match supported.sample_format() {
                SampleFormat::F32 => build::<f32>(&device, &stream_config, &queue),
                SampleFormat::F64 => build::<f64>(&device, &stream_config, &queue),
                SampleFormat::I16 => build::<i16>(&device, &stream_config, &queue),
                SampleFormat::I32 => build::<i32>(&device, &stream_config, &queue),
                SampleFormat::U16 => build::<u16>(&device, &stream_config, &queue),
                SampleFormat::U8 => build::<u8>(&device, &stream_config, &queue),
                SampleFormat::I8 => build::<i8>(&device, &stream_config, &queue),
                other => Err(format!("unsupported sample format {other}"))
            }?;
            stream.play().map_err(|e| e.to_string())?;

            Ok(Self { config, queue, _stream: stream })
        }

        /// What the PSG must produce for this device.
        pub fn config(&self) -> AudioConfig {
            self.config
        }

        pub fn push(&self, samples: &[f32]) {
            if let Ok(mut queue) = self.queue.lock() {
                queue.push(samples);
            }
        }

        /// How much sound is queued ahead of the device.
        pub fn buffered(&self) -> Duration {
            let frames = self.queue.lock().map_or(0, |queue| queue.frames());
            Duration::from_secs_f64(frames as f64 / self.config.sample_rate as f64)
        }

        pub fn missing_frames(&self) -> u64 {
            self.queue.lock().map_or(0, |queue| queue.missing_frames())
        }
    }

    fn build<T: SizedSample + FromSample<f32>>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        queue: &Arc<Mutex<FrameQueue>>
    ) -> Result<cpal::Stream, String> {
        let queue = Arc::clone(queue);
        let device_channels = config.channels as usize;
        // Reused between callbacks: allocates only when the device asks for
        // a larger buffer than before.
        let mut scratch = Vec::new();
        device
            .build_output_stream(
                *config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    scratch.resize(data.len(), 0.0f32);
                    match queue.lock() {
                        Ok(mut queue) => queue.pop_into(&mut scratch, device_channels),
                        Err(_) => scratch.fill(0.0)
                    }
                    for (out, &sample) in data.iter_mut().zip(scratch.iter()) {
                        *out = T::from_sample(sample);
                    }
                },
                |error| eprintln!("audio: {error}"),
                None
            )
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn mono_is_copied_to_every_device_channel() {
        let mut queue = FrameQueue::new(1, 100);
        queue.push(&[0.1, 0.2]);
        let mut out = [0.0; 6];
        queue.pop_into(&mut out, 3);
        assert_eq!(out, [0.1, 0.1, 0.1, 0.2, 0.2, 0.2]);
        assert_eq!(queue.frames(), 0);
    }

    #[test]
    fn stereo_goes_to_the_first_two_channels() {
        let mut queue = FrameQueue::new(2, 100);
        queue.push(&[0.2, 0.4]);
        let mut surround = [0.0; 4];
        queue.pop_into(&mut surround, 4);
        assert_eq!(surround, [0.2, 0.4, 0.3, 0.3]);

        queue.push(&[0.2, 0.4]);
        let mut single = [0.0; 1];
        queue.pop_into(&mut single, 1);
        assert_eq!(single, [0.2]);
    }

    #[test]
    fn an_empty_queue_holds_the_last_frame() {
        let mut queue = FrameQueue::new(2, 100);
        let mut out = [9.0; 4];
        queue.pop_into(&mut out, 2);
        assert_eq!(out, [0.0; 4], "silence before anything was queued");

        queue.push(&[0.5, -0.5]);
        let mut out = [0.0; 6];
        queue.pop_into(&mut out, 2);
        assert_eq!(out, [0.5, -0.5, 0.5, -0.5, 0.5, -0.5]);
        assert_eq!(queue.missing_frames(), 4);
    }

    #[test]
    fn the_oldest_frames_are_dropped_beyond_the_maximum() {
        let mut queue = FrameQueue::new(2, 3);
        queue.push(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 5.0, 5.0]);
        assert_eq!(queue.frames(), 3);
        let mut out = [0.0; 6];
        queue.pop_into(&mut out, 2);
        assert_eq!(out, [3.0, 3.0, 4.0, 4.0, 5.0, 5.0]);
    }
}
