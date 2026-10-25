# bemu in a web page

A library to embed the bemu Amstrad CPC emulator in any web page. It is a WebAssembly module with a
small JavaScript API, `bemu.js`.

The embedding page can:
- choose the machine: model, CRTC, Gate Array, RAM, disc interface and brand;
- choose the display: raw, or a CRT drawn on WebGPU;
- give DSK and SNA files, as URLs or as bytes;
- optionally play a CSL script live, getting its screenshots back.

The library takes care of the keyboard, the sound and fullscreen. `demo/index.html` is a minimal
page that uses it.

```html
<div id="cpc" style="width: 768px; aspect-ratio: 768 / 544"></div>
<script type="module">
  import { Bemu } from './pkg/bemu.js';

  const emu = await Bemu.create(document.querySelector('#cpc'), { model: '6128', crtc: 1, display: 'ctm' });
  await emu.insertDisc('A', 'https://example.org/demo.dsk');
  emu.start();
</script>
```

## Building

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129   # the wasm-bindgen of Cargo.lock
web/build.sh                                       # gives web/pkg
```

- `web/pkg` is the library: copy it next to your page, or publish it (it has a `package.json`).
- If Binaryen's `wasm-opt` is on the `PATH` (or in `WASM_OPT`), the module comes out about 25%
  smaller and 10% faster.
- Like the desktop build, it needs cpclib next to the repository (`../rust.cpcdemotools`).

**Trying it:**

```sh
cd web && python3 -m http.server 8000
```

then open <http://localhost:8000/demo/index.html>. The page has to be served over HTTP: ES modules
and `fetch` don't work from `file://`.

## API

`Bemu.create(target, config?, options?)` returns a `Promise<Bemu>`.
- **`target`**: a `<canvas>`, or an element that a canvas is added to (filling it).
- **`config`** (every field optional):

  | field | values | default |
  |---|---|---|
  | `model` | `'464'`, `'664'`, `'6128'` | `'6128'` |
  | `crtc` | `0` to `4` | `0` |
  | `gateArray` | `'40007'`, `'40008'`, `'40010'` | `'40010'` |
  | `ramKb` | `64`, `128`, … `576` (standard extra banks), or `4160` (a 4 MB expansion) | the model's |
  | `discInterface` | `true` / `false` | the model's |
  | `playcity` | `true` plugs a PlayCity: two more AYs (right, left) and a CTC (raster NMI, timer interrupts) | `false` |
  | `digiblaster` | `true` plugs a Digiblaster: an 8-bit DAC on the printer port | `false` |
  | `language` | `'en'`, or `'fr'`: the French 6128's ROMs and its AZERTY keyboard (typed text follows it) | `'en'` |
  | `brand` | `'amstrad'`, `'schneider'`, `'awa'`, `'orion'`, `'saisho'`, `'solavox'`, `'triumph'`, `'isp'` | `'amstrad'` |
  | `display` | `'raw'`, `'ctm'`, `'trinitron'`, `'gt65'`, `'atari'` | `'raw'` |
  | `curvature` | `0` (flat) to `4`: how curved a CRT's glass is, times its profile's bulge (WebGPU) | `0` |
  | `blackLevel` | `0` to `255`: the grey a black ink is drawn at | `8` |

- **`options`**:
  - `webgpu` (default `true`): `false` keeps to Canvas 2D.
  - `wasmUrl`: where `bemu_web_bg.wasm` is, if not next to `bemu.js`.
  - `fullscreenKey` (default `'F11'`, or `null` for none).
  - `turboKey` (default `'F10'`, or `null` for none): toggles full speed, see "Speed".
  - `autoStart` (default `false`).

**Methods** (a file `source` is a URL, a `File`/`Blob`, an `ArrayBuffer` or a typed array):

| method | |
|---|---|
| `start()`, `pause()`, `reset()` | run, stop, switch the machine off and on |
| `stepFrame()` | while paused: one more frame, shown |
| `setSpeed(speed)` | `1` (real time), another factor (`2`, `0.5`…), or `'max'` (full speed), see "Speed" |
| `configure(config)` | a new machine; the discs stay in |
| `setDisplay(name)` | `'raw'` or a CRT profile |
| `setCurvature(c)` | how curved a CRT's glass is, as the `curvature` configuration |
| `insertDisc(drive, source)`, `eject(drive)` | a DSK image in drive `'A'` or `'B'` |
| `insertTape(source)`, `ejectTape()` | a CDT tape in the cassette deck, PLAY down: it plays when the CPC starts the motor (`RUN"` on a 464, `\|TAPE` first on a 664/6128); returns what was skipped of it |
| `tapeControl(key)` | the deck's keys: `'play'`, `'stop'`, `'rewind'` |
| `loadSnapshot(source)` | an SNA snapshot |
| `saveSnapshot()` | the machine's state as an SNA file (`Uint8Array`, version 3), to save or give back to `loadSnapshot` |
| `addFile(name, source)` | a file a CSL script names (`disk_insert`, `tape_insert`, `snapshot_load`, `csl_load`) |
| `runCsl(script, { files })` | play a CSL script live, see below |
| `stopCsl()` | stop the script; its machine goes on |
| `typeText(text)` | type `text` on the CPC's keyboard (`\n` is RETURN), a paste; not while a script plays |
| `screenshot()` | the last frame as a PNG file (`Uint8Array`, the raw 768×272 picture), or `undefined` |
| `getDisc(drive)` | the DSK image in drive `'A'` or `'B'`, with what the CPC wrote on it (`Uint8Array`), or `undefined` |
| `fullscreen()` | toggle fullscreen (also `F11` on the emulator) |
| `setVolume(v)`, `setMuted(muted)` | `0` to `1`; silence the sound or bring it back |
| `on(event, fn)`, `off(event, fn)` | events, see below |
| `destroy()` | stop and release everything: listeners, sound, wasm memory |

**Properties:**
- `backend`: `'webgpu'` or `'canvas2d'`.
- `canvas`.
- `running`.
- `cslRunning`.
- `vsyncPaced`: whether frames follow a 50 Hz display, see "Timing".
- `speed`: `1`, another factor, or `Infinity` (full speed).
- `muted`.
- `tape`: `{ position, duration, playing }` (seconds), or `undefined` without a tape.

**Events:**
- `'screenshot'`: `{ name, png }`, where `png` is a `Uint8Array` holding a PNG file;
- `'csl-done'`: `{ screenshots, ssmCodes, ignored, elapsedUs }`, or `{ error }`;
- `'error'`: an `Error` from the emulation, which pauses.
- `'stats'`: once a second while running, `{ fps, speed }`: the frames emulated in that second,
  and the times real time they make (`1` is a real CPC).

### CSL scripts

`runCsl(script, { files })` plays a script live on a machine of its own.
- **`script`**: the script's text (several lines), a URL, or a `File`/`Blob`/bytes.
- **`files`**: maps the names the script uses to their sources. Files can also be given
  beforehand with `addFile`.
- **Names:** a file is found by its name only, whatever directory and case the script gives.
  When a script names a disc it wasn't given and exactly one disc was given, that one is used,
  with a note in the report. The desktop runner does the same with the discs next to the script.
- **Machine:** the script starts from the configuration. Its own `crtc_select`, `gate_array` and
  `memory_exp` apply at its `reset`.
- **Keyboard:** while the script plays, the keyboard is the script's. Afterwards the machine stays
  as the script left it.
- **Screenshots** are named as the desktop's `amsrust` names them (`AMSRUST_<crtc>_<code>.png`
  for SSM codes).

```js
emu.on('screenshot', ({ name, png }) => upload(name, png));
emu.on('csl-done', (report) => console.log(report));
await emu.runCsl('https://example.org/SHAKER-A-0.CSL', { files: { 'shaker27.dsk': 'https://example.org/shaker27.dsk' } });
```

### Speed

`setSpeed(speed)`, or the `turboKey` (F10) on the emulator, which toggles full speed:
- **`1`**: real time, paced as described in "Timing". The only speed with sound.
- **another factor** (`2`, `4`, `0.5`…): that many times real time, by the browser's clock.
- **`'max'`**: as fast as the computer can. The page gives most of each display refresh to
  the emulation and shows one frame of it. Useful to skip a loader or play a CSL script quickly.

Above real time, the frames nobody will see run without any picture (the Gate Array makes no
pixels), about a third faster than shown ones; only the last frame of each refresh is made and
shown. The machine runs exactly the same either way. A CSL script keeps all its pictures, since
its screenshots need them.

At any speed but `1` the sound is dropped: it would play late or at the wrong pitch. The
'stats' event tells how fast it actually runs.

```js
emu.setSpeed('max');
await emu.insertDisc('A', 'game.dsk');
emu.typeText('run"game\n');
emu.on('stats', ({ speed }) => console.log(`${speed.toFixed(1)}× real time`));
setTimeout(() => emu.setSpeed(1), 10000); // back to real time once it has loaded
```

### Keyboard, sound, fullscreen
- **Keyboard:**
  - The emulator takes the keys while its canvas has the focus (a click gives it).
  - Keys follow the host layout, as in the desktop windows: a key typing `a` is the CPC's `A`.
    Typed characters (`é`, `@`…) go through the CPC's own key combinations.
  - Function keys and Ctrl/Meta shortcuts are left to the browser, except the emulator's own:
    `F10` (full speed) and `F11` (fullscreen), which the options can change or turn off.
- **Sound:** browsers only allow it after a click or a key, so it starts with the first click or
  key in the emulator.
- **Fullscreen:** the browser's Fullscreen API on the canvas.

### Timing

A web page cannot set the display's refresh rate, so the library measures it.
- **About 50 Hz** (for example a monitor set to 50 Hz): one CPC frame is shown per refresh, as
  the desktop's 50 Hz fullscreen does. Scrolling is then smooth.
- **Otherwise:** the sound's clock paces the emulation, as on the desktop. Before sound starts,
  the browser's clock does, at 50 frames per second.

### Drawing
- **WebGPU:** used when the browser has it. The picture fills the canvas, letterboxed, with the
  CRT profiles drawn on the GPU (the shader of the desktop's `--crt-gpu`), seen through the
  tube's glass, flat by default; `curvature: 1` bulges the picture and rounds the corners, as
  on the desktop. The raw display is never curved.
- **Canvas 2D:** used otherwise. It shows the raw picture at 768×544, and the page scales the
  canvas.

### Files from other sites

URLs are loaded with `fetch`, so a file on another site needs that site to allow it (CORS).
Otherwise, let the user pick the file, or download it through your own server.
