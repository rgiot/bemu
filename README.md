# amsrust

Amstrad CPC emulator written in Rust.

## Running the emulator

Use the example binary below to test the emulator:

```sh
cargo run --example emulator
```

## Sound

The PSG (AY-3-8912) is emulated with the [`ym2149`](https://crates.io/crates/ym2149)
crate, clocked at the CPC's 1MHz (the gate array's CCLK). The library has no
audio-device dependency: enable an output and collect samples yourself.

```rust
use amsrust::psg::AudioConfig;
motherboard.set_audio_output(Some(AudioConfig::stereo(48_000))); // or ::mono
// ... run the machine ...
let samples: Vec<f32> = motherboard.take_audio_samples(); // interleaved L/R
```

Record a run to a .wav file:

```sh
# type a BASIC command once booted, record 6 emulated seconds
cargo run --release --example dump_wav -- --type 'sound 1,284,100,15\n' --seconds 6 out.wav
# a disc, in stereo at 48kHz
cargo run --release --example dump_wav -- --dsk game.dsk --type 'run"game\n' --seconds 60 --rate 48000 --stereo out.wav
# the whole run of a CSL script
cargo run --release --example dump_wav -- --csl script.csl out.wav
```

Play the sound live in the window with the opt-in `audio` feature (cpal; on
Linux it needs the ALSA headers, e.g. `libasound2-dev`):

```sh
cargo run --release --features audio -- --window --dsk game.dsk
```

The audio device's clock then paces the emulation; `--mute` silences it.
When the emulator runs slower than real time, the sound has gaps (the pitch
stays right); the number of seconds missed is printed when the window closes.

See `src/psg.rs` for the mixing (CPC stereo wiring), the resampling and the
known differences with the AY-3-8912.
