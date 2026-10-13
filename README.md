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

## Debug server

`--debug-server` serves an HTTP debug API compatible with the
[AMSpiriT-Lite web API](https://github.com/AMSpiriT-Emulator/amspirit-releases/blob/main/docs/lite/web_api.md)
on `127.0.0.1:6128` (`--debug-port N` to change it), for debugger front ends
such as `cpclib-dap`. It runs with `--window` or headless (real time, Ctrl+C to
quit). `GET /api/doc` lists the implemented endpoints; extensions to the
AMSpiriT API are flagged there, and `ping` answers `"frontend":"bemu"`.

```sh
cargo run --release -- --debug-server &
curl -s -X POST http://127.0.0.1:6128/api/z80_bp -d '0x38'     # break on RST 38
curl -s http://127.0.0.1:6128/api/z80                          # registers once stopped
curl -s -X POST http://127.0.0.1:6128/api/step                 # one instruction
curl -s -X POST http://127.0.0.1:6128/api/config -d '{"paused":false}'
```

