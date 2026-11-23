//! What the hosts do to the sound the machine produced before it reaches a device: the mono fold,
//! the volume, and how much sound they keep queued ahead of the device. The desktop window
//! (`session.rs`, `host_audio.rs`) and the web page (`web/src/emulator.rs`) share it, so they treat
//! the sound the same way (the web page's JavaScript keeps the same figures in `engine.js`,
//! `bemu.js` and `audio-worklet.js`: change them together).

use std::time::Duration;

/// Both channels of each stereo frame replaced by their mean (Audio > Mono). `samples` are
/// interleaved frames of `channels`; a mono stream is left as it is.
pub fn fold_to_mono(samples: &mut [f32], channels: usize) {
    if channels < 2 {
        return;
    }
    for frame in samples.chunks_mut(channels) {
        let mono = frame.iter().sum::<f32>() / frame.len() as f32;
        frame.fill(mono);
    }
}

/// Every sample times `gain` (1: untouched).
pub fn apply_gain(samples: &mut [f32], gain: f32) {
    if gain != 1.0 {
        for sample in samples {
            *sample *= gain;
        }
    }
}

/// The sound settings of a host (Audio menu).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    /// Left and right kept apart; else both are the mix of the two.
    pub stereo: bool,
    /// 0 to 100 (percent).
    pub volume: u8,
}

impl Mix {
    /// The gain of the volume: 1 at 100.
    pub fn gain(&self) -> f32 {
        f32::from(self.volume) / 100.0
    }

    /// Whether [`Self::apply`] changes anything for a stream of `channels`.
    pub fn is_neutral(&self, channels: usize) -> bool {
        self.gain() == 1.0 && (self.stereo || channels < 2)
    }

    /// The mono fold and the volume, in place.
    pub fn apply(&self, samples: &mut [f32], channels: usize) {
        if !self.stereo {
            fold_to_mono(samples, channels);
        }
        apply_gain(samples, self.gain());
    }
}

/// How much sound a host keeps queued ahead of its device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pacing {
    /// The sound kept queued: the emulation waits while more than this is.
    pub target: Duration,
    /// Beyond this, the oldest sound is dropped so the latency cannot grow.
    pub max: Duration,
}

impl Pacing {
    /// The desktop window: 60 ms ahead of the device, 250 ms at most.
    pub const DESKTOP: Pacing = Pacing {
        target: Duration::from_millis(60),
        max: Duration::from_millis(250),
    };
    /// The web page (the audio worklet starts at the same 60 ms, the engine drops a frame beyond
    /// 400 ms).
    pub const WEB: Pacing = Pacing {
        target: Duration::from_millis(60),
        max: Duration::from_millis(400),
    };

    /// Whether the machine is behind real time: under half of the target is queued.
    pub fn is_behind(&self, buffered: Duration) -> bool {
        buffered < self.target / 2
    }

    /// Whether the emulation should wait for the device to play some of what is queued.
    pub fn should_wait(&self, buffered: Duration) -> bool {
        buffered > self.target
    }

    /// Whether the sound about to be queued must be dropped (it would put the latency over `max`).
    pub fn should_drop(&self, buffered: Duration) -> bool {
        buffered > self.max
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn mono_replaces_each_frame_by_its_mean() {
        let mut samples = [1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        fold_to_mono(&mut samples, 2);
        assert_eq!(samples, [0.5, 0.5, 0.5, 0.5, 0.0, 0.0]);
        let mut mono = [0.25, 0.75];
        fold_to_mono(&mut mono, 1);
        assert_eq!(mono, [0.25, 0.75], "a mono stream is left alone");
    }

    #[test]
    fn the_volume_scales_the_samples() {
        let mix = Mix { stereo: true, volume: 50 };
        let mut samples = [1.0, -0.5];
        mix.apply(&mut samples, 2);
        assert_eq!(samples, [0.5, -0.25]);
        assert!(Mix { stereo: true, volume: 100 }.is_neutral(2));
        assert!(Mix { stereo: false, volume: 100 }.is_neutral(1));
        assert!(!Mix { stereo: false, volume: 100 }.is_neutral(2));
        assert!(!mix.is_neutral(2));
    }

    #[test]
    fn mono_and_volume_together() {
        let mix = Mix { stereo: false, volume: 50 };
        let mut samples = [1.0, 0.0];
        mix.apply(&mut samples, 2);
        assert_eq!(samples, [0.25, 0.25]);
    }

    #[test]
    fn the_pacing_waits_drops_and_notices_it_is_behind() {
        let p = Pacing::DESKTOP;
        assert!(p.is_behind(Duration::from_millis(29)));
        assert!(!p.is_behind(Duration::from_millis(30)));
        assert!(p.should_wait(Duration::from_millis(61)));
        assert!(!p.should_wait(Duration::from_millis(60)));
        assert!(p.should_drop(Duration::from_millis(251)));
        assert!(!p.should_drop(Duration::from_millis(250)));
        assert!(Pacing::WEB.max > Pacing::DESKTOP.max);
    }
}
