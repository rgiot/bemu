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

Sound is played live in the window by default (the `audio` feature, on; build with `--no-default-features` where there is no audio library; on
Linux it needs the ALSA headers, e.g. `libasound2-dev`):

```sh
cargo run --release -- --window --dsk game.dsk
```

The audio device's clock then paces the emulation; `--mute` silences it.
When the emulator runs slower than real time, the sound has gaps (the pitch
stays right); the number of seconds missed is printed when the window closes.

See `src/psg.rs` for the mixing (CPC stereo wiring), the resampling and the
known differences with the AY-3-8912.

## Video capture

`--record <file>` records a video with sound through `ffmpeg` (which must be
installed; the file extension picks the format), at the CPC's exact frame rate
(~50.08 fps), picture and sound in sync in emulated time:

```sh
# 30 emulated seconds (--seconds), headless
cargo run --release -- --dsk demo.dsk --record demo.mp4 --seconds 30
# what the window shows and plays, until it is closed
cargo run --release -- --window --dsk game.dsk --record game.mkv
```

Ctrl+C stops a recording early; the file is still finished properly.
