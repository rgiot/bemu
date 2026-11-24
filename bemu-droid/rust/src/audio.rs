//! The sound, played through AAudio (cpal): the PSG's samples, stereo at the device's rate,
//! queued for the device's callback. The queue also paces the emulation, as on the desktop
//! (`amsrust::audio_mix::Pacing`): the machine waits while enough sound is ahead.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use amsrust::psg::AudioConfig;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

/// Interleaved stereo samples waiting for the device.
#[derive(Default)]
struct Queue {
    samples: VecDeque<f32>,
}

pub struct AudioOut {
    rate: u32,
    queue: Arc<Mutex<Queue>>,
    // playing until dropped
    _stream: cpal::Stream,
}

impl AudioOut {
    pub fn open() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let queue = Arc::new(Mutex::new(Queue::default()));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config, &queue),
            SampleFormat::I16 => build::<i16>(&device, &config, &queue),
            SampleFormat::I32 => build::<i32>(&device, &config, &queue),
            other => Err(format!("unsupported sample format {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            rate: config.sample_rate,
            queue,
            _stream: stream,
        })
    }

    /// What the PSG must produce: stereo at the device's rate.
    pub fn config(&self) -> AudioConfig {
        AudioConfig::stereo(self.rate)
    }

    /// Queue stereo samples, the oldest dropped beyond `max` of sound.
    pub fn push(&self, samples: &[f32], max: Duration) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.samples.extend(samples);
            let max = (max.as_secs_f64() * self.rate as f64) as usize * 2;
            let excess = queue.samples.len().saturating_sub(max);
            queue.samples.drain(..excess - excess % 2);
        }
    }

    /// How much sound is queued ahead of the device.
    pub fn buffered(&self) -> Duration {
        let samples = self.queue.lock().map_or(0, |queue| queue.samples.len());
        Duration::from_secs_f64(samples as f64 / 2.0 / self.rate as f64)
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: &Arc<Mutex<Queue>>,
) -> Result<cpal::Stream, String> {
    let queue = Arc::clone(queue);
    let channels = usize::from(config.channels.max(1));
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut queue = queue.lock().ok();
                for frame in data.chunks_mut(channels) {
                    let (left, right) = match queue.as_mut() {
                        Some(queue) if queue.samples.len() >= 2 => {
                            (queue.samples.pop_front().unwrap_or(0.0), queue.samples.pop_front().unwrap_or(0.0))
                        }
                        _ => (0.0, 0.0),
                    };
                    match frame {
                        [mono] => *mono = T::from_sample((left + right) / 2.0),
                        [l, r, rest @ ..] => {
                            *l = T::from_sample(left);
                            *r = T::from_sample(right);
                            for other in rest {
                                *other = T::from_sample(0.0);
                            }
                        }
                        [] => {}
                    }
                }
            },
            |error| log::error!("audio: {error}"),
            None,
        )
        .map_err(|e| e.to_string())
}
