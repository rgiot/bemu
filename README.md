# amsrust

Amstrad CPC emulator written in Rust.

## Running the emulator

Use the example binary below to test the emulator:

```sh
cargo run --example emulator
```

### Machines

`--model 464|664|6128` (default 6128) picks the firmware and BASIC (1.0 on the
464, 1.1 on the others). The 464 and 664 have 64K: `--ram 128` (up to 576)
adds a RAM expansion, banked through the same `&7Fxx` register as the 6128's
second 64K. The 664 and 6128 have a disc interface; `--ddi` adds a DDI-1
(AMSDOS and the FDC) to a 464, and `--no-disc` removes it. A snapshot
(version 2 or later) runs on the model it was taken on, with its extra RAM
as an expansion on a 464/664. In the library:
`MotherboardBuilder::set_model`, `set_extra_ram_pages`, `set_disc_interface`.

```sh
cargo run --release -- --model 464 --ram 128 --ddi --window --dsk game.dsk
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

Beyond the first endpoints (`ping`, `state`, `z80`, `ram` on every 16K bank,
`exec`, `step`, `config`, `keytype`), it streams Server-Sent Events
(`GET /api/events`: `frame`, `z80_bp`, `pause`, `reset`, `watch`), takes
breakpoints on physical locations (`z80_bp` with `Bnn:hhhh`; CPU addresses are
resolved to the RAM bank or ROM mapped at that time) and, as an extension,
advanced breakpoints as rasm/basm/ACE describe them (`/api/breakpoints`:
memory/IO, read/write, stop or watch, masks, sizes, values, and conditions
such as `A == #12 && peek(HL) <> 0` or `value & %1000`). A snapshot's
breakpoints (REMU, ACE `BRKC`, WinAPE `WABP` chunks) are set when it is loaded,
`--sna` or `POST /api/media`. Also: `history`, `screenshot` (PNG), `media`
(sna, dsk, bin), `quit`.

The chips are inspected one by one (`memmap`, `crtc`, `ga`, `psg`, `fdc`).
BASIC programs are debugged natively, BASIC 1.0 (464) and 1.1 (664, 6128): `basic` loads a program (`?reset=1`,
`?run=1`), `basic_state`, `basic_listing` (lines of statements, with the
addresses the ROM runs them from) and `basic_export` read it, `basic_bp`
(line breakpoints), `basic_step` (`?mode=line`) and `basic_runto` (`?line=`,
`?addr=`) stop it, reported by `basic_bp` events.

The beam is where the monitor draws (`GET /api/beam`): AMSpiriT's video buffer
coordinates (x 0-1023, 16 per NOP; y 0-349 lines since the VSYNC), and the
same in NOPs: `nop` 0-63 in the `line`, `frame_nop` 0-19967 (64 NOPs a line,
19968 a frame). `raster_bp` stops when the beam goes past a position, given
either way: `?x=&y=`, `?line=&nop=`, `?frame_nop=`, or `?nop=` alone for every
line (`add=1` keeps the others, `enable=0` clears them). Named NOP counters
(`/api/counters`, an extension) count the NOPs run while they run: create,
reset, start, stop, delete; stop events report them and the beam.
Advanced breakpoints drive them: `"start"`, `"stop"`, `"reset"` name the
counters a hit acts on, and `"run":"count"` only does that (no stop, no
event), so the NOPs between two points of a running program are measured
without stopping it. `"after": N` ignores the first N hits, and each
breakpoint reports its `hits`.

```sh
curl -s -X POST http://127.0.0.1:6128/api/breakpoints -d '{"addr":"0x4000","run":"count","reset":"t","start":"t"}'
curl -s -X POST http://127.0.0.1:6128/api/breakpoints -d '{"addr":"0x4100","run":"count","stop":"t"}'
curl -s http://127.0.0.1:6128/api/counters      # NOPs from 0x4000 to 0x4100, last time through
```

```sh
curl -s -X POST 'http://127.0.0.1:6128/api/raster_bp?line=100&nop=0'  # stop at line 100
curl -s -X POST http://127.0.0.1:6128/api/counters -d '{"name":"loop"}'
curl -s -X POST 'http://127.0.0.1:6128/api/basic?run=1' --data-binary @prog.bas
curl -s -X POST http://127.0.0.1:6128/api/basic_bp -d '20,40'
```

```sh
cargo run --release -- --debug-server &
curl -sN http://127.0.0.1:6128/api/events &                   # follow the events
curl -s -X POST http://127.0.0.1:6128/api/breakpoints \
     -d '{"kind":"mem","access":"w","run":"watch","addr":"0xC000","size":16384}' # screen writes
curl -s -X POST http://127.0.0.1:6128/api/z80_bp -d '0x38'     # break on RST 38
curl -s http://127.0.0.1:6128/api/z80                          # registers once stopped
curl -s -X POST http://127.0.0.1:6128/api/step                 # one instruction
curl -s -X POST http://127.0.0.1:6128/api/config -d '{"paused":false}'
```

