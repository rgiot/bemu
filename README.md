# amsrust

Amstrad CPC emulator written in Rust.

## Running the emulator

Use the example binary below to test the emulator:

```sh
cargo run --example emulator
```

### Machines

`--model 464|664|6128` (default 6128; the CPC+ below) picks the firmware and BASIC (1.0 on the
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

### CPC+ and GX4000

`--model 6128plus|464plus|gx4000` is a CPC+: its ASIC (`src/asic.rs`) replaces the CRTC
(always type 3), the Gate Array, the PAL and the PPI, and a cartridge replaces the ROMs. The
6128+ has 128K and a disc interface, the 464+ and the GX4000 64K (Arnold V specification).
The system cartridge (firmware, BASIC, AMSDOS, Burnin' Rubber; French with `--french`) is built
in; `--cart game.cpr` (or a zip archive, `archive.zip#game.cpr`) puts another one in the slot
and makes the machine a 6128+ unless `--model` names another CPC+. In a CSL script:
`cpc_model 4|5|6` and `rom_config C 0 'game.cpr'` (`rom_dir`), at the next `reset`.

```sh
cargo run --release -- --cart sonic.cpr --window
cargo run --release -- --model 6128plus --dsk demo.dsk --window   # f1 for BASIC, then RUN"
```

What the ASIC adds is emulated: the lock (the sequence on `&BCxx`), RMR2 (`&7Fxx`, `&A0-&BF`:
the cartridge page of the lower ROM, where it is, the register page at `&4000`), the cartridge
pages at `&C000` (`&DFxx`, `&80-&9F`), the 32 colours of 4096, the 16 sprites, the soft scroll
(horizontal, vertical, the wider border), the screen split, the programmable raster interrupt,
the vectored interrupts (IVR, DCSR) and the 3 sound DMA channels. Not yet: what an `IN` on the
Gate Array or CRTC ports writes on a CPC+, the snapshot's `CPC+` chunk, the analogue
joysticks (they read as nothing plugged), the vectored interrupt bug.

### Discs and drives

`--dsk` (drive A) and `--dsk-b` take a DSK or extended DSK, an HFE image, or a zip archive:
`--dsk games.zip` takes its first DSK/HFE file, `--dsk 'games.zip#Disk 1 side B.dsk'` that file of
the archive. The same names work in a CSL `disk_insert`, with `--watch` (the archive is watched)
and in the debug server's `/api/media`.

An HFE image (version 1 or 3) stays what it is, the MFM bits of each track. It is read, decoded and
written through the HxC floppy library (the `hxcfe` crate, `src/hfe.rs`): the FDC reads the sectors
the library finds on each track, where they lie on it (CRC errors, deleted or missing marks, weak
bits of HFE v3 as they are), sector writes go through the library and Format Track writes the whole
track.

`--save-discs` writes the discs back to their files when the CPC wrote to them: when the drive
motor stops, when another disc replaces it and at the end. An HFE is saved as HFE v3, a DSK as an
extended DSK, a disc taken from a zip archive replaces its entry (the other files are kept).

Drives are 3" ones by default, as on a 664/6128 and the DDI-1's FD-1: one head, so the FDC's side
select is ignored and the side read is the side put up; F5 in the window turns the disc of drive A
over (Shift+F5: drive B), which reads the other side of a double-sided image. `--drive-a 3.5`
(`--drive-b 3.5`) makes it a double-sided 3.5" drive: the FDC selects the side, multi-track
commands go on with side 1, Sense Drive Status reports two sides, and the head reaches track 82
(42 on a 3" drive). The debug server's `POST /api/drive?drive=0&kind=3.5` and `&flip=1` do the
same. `validation/demos/batman` has both: the 3" set and the one disc version for a 3.5" drive.

```sh
cargo run --release -- --window --drive-a 3.5 --dsk 'batman_forever.zip#Batman Forever (One disk version).dsk'
```

## Menus and command palette

The window (`--window`, the default winit frontend) has a menu bar with every command of the
emulator, from `doc/menu-ui.csv` (the menu specification, compiled into the binary by
`build.rs`): media, debugger, machine, sound, video, tools. Ctrl+Shift+P opens the command
palette: type a few letters of a command (`crtc 1`, `flip b`, `snapshot`), Enter runs it. A
command that needs something asks it (an address, a value, a file in a small file browser).
`--no-menu` hides the menu bar (Tools > Show menu bar, or the palette, shows it again).

The machine changes while it runs: another CRTC takes over the registers and counters of the
previous one, more or less RAM keeps what is in the RAM left, cards are plugged and unplugged
with the power on, a CPC+ gives back its CRTC when it becomes a CPC again.

The function keys, the menus, the palette and the debug server's routes all run the same
commands (`src/commands`). Debug > View opens the debugger's windows (registers, CRTC, Gate
Array, PSG, FDC, memory, disassembly, history, breakpoints, symbols); breakpoints, stepping (in,
over, out, a frame) and counters work without the debug server, which Debug > Debug server
starts on demand. The commands the emulator cannot do yet (a Gotek drive, recording a tape) are
in the menus, greyed out.

The minifb frontend (`--frontend minifb`) has no menu: its function keys run the same commands.

## CPC Booster+ and MiniBooster

`--booster` plugs a CPC Booster+ (Antitec / Dirty Minds) on the expansion port, `--miniboster` its
variant (`INP(&FF01)` reads 238 instead of 85: how software tells them apart). The ports are
&FF00-&FF27, the registers those of the board's BIOS (version 1.2), followed one by one:

| ports | |
|---|---|
| &FF00, &FF01 | IN: 170, 85 (238); OUT: reset the board |
| &FF02, &FF03, &FF26, &FF27 | PWM channels (the card's stereo sound; &FF26 buffered pairs, &FF27 both); the first channel is on the left |
| &FF04-&FF0B | UART: baud rate, data, flags (RXC, TXC, UDRE, DOR), frame format, auto-polling send, wait for a byte, time out, mode |
| &FF1C, &FF1D | the 255 bytes receive buffer |
| &FF0C-&FF0E | 512 bytes of EEPROM |
| &FF0F-&FF11 | ADC: frequency, channel, sample |
| &FF12 | keyboard decoder: 10 lines of the scan in, the key out |
| &FF13-&FF1B | the 16K of program memory: read by pages or by address, written by pages (the BIOS update) |
| &FF1E-&FF20 | the 5 bits TTL port |
| &FF21-&FF24 | 8x8 multiplier |
| &FF25 | the text of the version |

- `--booster-uart loopback` wires TX to RX; `listen:127.0.0.1:2323` or `connect:host:port` make a TCP
  socket the serial line (a null modem: with `socat` or a terminal on the other side). The frames last
  what the baud rate says, so the flags behave.
- `--booster-adc file.wav` (PCM, 8 or 16 bits) or raw unsigned bytes: what the ADC samples, looped.
- `--booster-eeprom file`, `--booster-flash file`: the EEPROM and the program memory are kept in
  these files (a dump of the real BIOS can be given: it is read through the ports).
- Not modelled: the time the board makes the CPC wait (the ADC conversion, the time out of &FF09),
  the RS485 pin, the connectors of the MiniBooster. The documentation does not say which PWM channel is
  which: the first is on the left.

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
quit; `--debug-paused` starts it paused, so a debugger sets its breakpoints
before anything runs). `GET /api/doc` lists the implemented endpoints;
extensions to the AMSpiriT API are flagged there, and `ping` answers
`"frontend":"bemu"`. The API documents itself: `/api/doc?endpoint=ram` gives
an endpoint's query parameters, body fields, answer and an example,
`/api/doc?format=openapi` the whole API as OpenAPI 3 (for Swagger UI or client
generators), and `/api/doc?format=html` a page to read in a browser.

Beyond the first endpoints (`ping`, `state`, `z80`, `ram` on every 16K bank,
`exec`, `step`, `config`, `keytype`), it streams Server-Sent Events
(`GET /api/events`: `frame`, `z80_bp`, `pause`, `reset`, `watch`), takes
breakpoints on physical locations (`z80_bp` with `Bnn:hhhh`; CPU addresses are
resolved to the RAM bank or ROM mapped at that time) and, as an extension,
advanced breakpoints as rasm/basm/ACE describe them (`/api/breakpoints`:
memory/IO, read/write, stop or watch, masks, sizes, values, and conditions
in [ACE's language](https://roudoudou.com/ACE-DL/breakpoints.html), as basm,
rasm and ACE write them - `A = &12 AND NOT FlagZ`, `LineCount > 100 AND
CrtcR12 = &30`, with the registers, flags, CRTC counters and registers, the
beam and the memory mapping - or with C's operators (`A == #12 &&
peek(HL) <> 0`, `value & %1000`); see `amsrust::breakpoints::Condition`). A snapshot's
breakpoints (REMU, ACE `BRKC`, WinAPE `WABP` and rasm/basm `BRKS` chunks) are
set when it is loaded, `--sna` or `POST /api/media`. Also: `history`, `screenshot` (PNG: the visible
area, or with `crop=0` the whole 1024x350 frame, sync and borders included;
`live=1` the frame being drawn, `full=0` over the previous one), `media`
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

## Keyboard

The host's keys press the CPC's: the letters, digits and punctuation, RETURN (Enter), the
keypad's ENTER, DEL (Backspace), CLR (Delete), ESC, TAB, CAPS LOCK, SHIFT, CONTROL (Ctrl),
COPY (Alt) and the cursor keys; the keypad's digits and dot are the CPC's f0-f9 and f.
`--keyboard` picks how (F12 switches it in the window), as VICE's mappings:
- **symbolic** (the default): a key types the character it types on your keyboard, whatever
  its language and the CPC's (`--french`: the French 6128, AZERTY);
- **positional**: a key presses the CPC key at its place, for games (on Linux, in the default
  window, give `--host-keyboard-layout azerty` or `qwertz` if yours isn't QWERTY).

On the CPC, the cursor keys alone move only inside the line being typed: at `Ready` they move
nothing. To edit a line already on the screen, hold SHIFT with the cursor keys to move the copy
cursor to it, then press COPY (Alt) to copy its characters into the line being typed, as on a
real CPC. That is also how a character the CPC has no key for is typed: the `~` of an M4's 8.3
name (`COLUMN~1.DSK`, as `CAT` shows it) is copied from the screen, or written `CHR$(126)`.

## Example scripts

`examples/` holds scripts that set a machine up and run something on it (`--window`); each
one's header says more.

| script | what it shows |
|---|---|
| `examples/unidos.sh FOLDER [RUN]` | UniDOS on a Nova and an Albireo, the USB key a host folder |
| `examples/uniload.sh [FOLDER]` | UniLoad, Roudoudou's game launcher, on UniDOS |
| `examples/willy.sh --pack [FOLDER] [PATH]` | Willy's OPL3: PulkoMandy's vgmplay plays a tune of the OPL3 player pack |
| `examples/willy.sh [--s2p FILE.sf2] [DSK]` | Willy's OPL3 and S2P, from BASIC |
| `examples/rulezcharge.sh [SD_FOLDER]` | RulezCharge on the M4, downloading from cpcrulez.fr |
| `examples/dandanator.sh [ROMSET.rom]` | the CPC Dandanator Mini |

UniLoad and the OPL3 player pack are their authors' archives, not part of bemu: without a
FOLDER, the scripts download them from roudoudou.com (about 60 MB each, once) into
`examples/downloads/`. `examples/willy.sh --pack` alone plays Wolfenstein 3D's title tune;
PATH picks another (`MUS0F/ALONEDRK/01INTRO.VGM`), or `-` leaves the player's prompt to you.

## Licence

bemu is licensed under either of the [MIT licence](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option. The ROMs it embeds and the
LGPL-licensed OPL3 core (the `willy` feature) are not: see [NOTICE.md](NOTICE.md).

Unless you explicitly state otherwise, any contribution you submit for inclusion in bemu is
dual-licensed as above, without any additional terms or conditions.
