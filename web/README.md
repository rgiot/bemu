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
cargo install wasm-bindgen-cli --version 0.2.130   # the wasm-bindgen of web/Cargo.lock
# Install Binaryen (e.g. brew install binaryen on macOS) for wasm-opt.
web/build.sh                                       # gives web/pkg
```

- `web/pkg` is the library: copy it next to your page, or publish it (it has a `package.json`).
- Binaryen's `wasm-opt` is required on the `PATH` (or set `WASM_OPT` to its executable). The
  build uses release optimization, whole-program LTO and `wasm-opt -O3` for speed.
- Like the desktop build, it needs cpclib next to the repository (`../rust.cpcdemotools`).

The JavaScript scheduling and audio-startup regression tests run with `node --test web/tests/`.

**Trying it:**

```sh
cd web && python3 -m http.server 8000
```

then open <http://localhost:8000/demo/index.html>. The page has to be served over HTTP: ES modules
and `fetch` don't work from `file://`.
Raw output uses the largest whole-number physical-pixel scale that fits, centred in the canvas,
on both GPU and Canvas 2D. If the canvas is smaller than 768 × 544 physical pixels, downscaling
is unavoidable and some source pixels will not occupy equal space. Enlarge the canvas or use
fullscreen for uniform raw pixels.
For sound on other devices, serve the page and the audio worklet over HTTPS. Browsers generally
only expose AudioWorklet in a secure context; `localhost` is a development exception, but a LAN
address served over plain HTTP is not. Configure the server to serve `.wasm` as `application/wasm`
for streaming compilation; the browser can still load it with another MIME type, but startup is
slower.

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
  | `keyboard` | `'symbolic'`: a key types the character it types on your keyboard (whatever its language and the CPC's), for typing; `'positional'`: a key presses the CPC key at its place (`KeyboardEvent.code`), for games | `'symbolic'` |

- **`options`**:
  - `webgpu` (default `true`): `false` keeps to Canvas 2D.
  - `worker` (default `true`): runs the emulation in a Web Worker; `emu.threaded` tells which mode is used. Without WebGPU (or with a software adapter) the CRT shaders run on WebGL 2 when it exists, else Canvas 2D shows the raw picture only.
  - `wasmUrl`: where `bemu_web_bg.wasm` is, if not next to `bemu.js`.
  - `audioContext`: an `AudioContext` created during a user gesture, to satisfy browser autoplay
    policies while the emulator loads asynchronously. The emulator owns and closes it after
    successful creation; if `Bemu.create` fails, the caller remains responsible for closing it.
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
| `setKeyboard(mapping)` | `'symbolic'` or `'positional'`, as the `keyboard` configuration |
| `insertDisc(drive, source)`, `eject(drive)` | a DSK image in drive `'A'` or `'B'` |
| `swapDisc(drive, source, entry)` | changes the disc of the running machine: the new disc is in the drive at once |
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
| `driveActive` | `true` while the drive motor runs, i.e. the drive is being accessed |
| `fullscreen()` | toggle fullscreen (also `F11` on the emulator) |
| `enableAudio()` | start the sound from a click or key handler of the page, instead of waiting for the first click in the emulator |
| `setVolume(v)`, `setMuted(muted)` | `0` to `1`; silence the sound or bring it back |
| `on(event, fn)`, `off(event, fn)` | events, see below |
| `destroy()` | stop and release everything: listeners, sound, wasm memory |

**Properties:**
- `backend`: `'webgpu'` or `'canvas2d'`.
- `canvas`.
- `displayWarning`: `null`, or `{ width, height, requiredWidth, requiredHeight }` in physical
  pixels when the WebGPU CTM canvas is too small for its phosphor mask. Read this after creation
  to display an initial warning.
- `displaySize`: `{ width, height }` in physical pixels, including in worker mode.
- `ctmMinimumSize`: the `{ width, height }` needed for the CTM mask, or `null` on Canvas 2D.
- `running`.
- `cslRunning`.
- `vsyncPaced`: whether frames follow the display's refresh (50 Hz, or 100, 150, 200), see "Timing".
- `refreshesPerFrame`: then how many refreshes show a CPC frame (1 at 50 Hz, 2 at 100 Hz…), else `0`.
- `speed`: `1`, another factor, or `Infinity` (full speed).
- `muted`.
- `tape`: `{ position, duration, playing }` (seconds), or `undefined` without a tape.

**Events:**
- `'screenshot'`: `{ name, png }`, where `png` is a `Uint8Array` holding a PNG file;
- `'csl-done'`: `{ screenshots, ssmCodes, ignored, elapsedUs }`, or `{ error }`;
- `'error'`: an `Error` from the emulation, which pauses.
- `'audio-error'`: an error starting sound (the emulation keeps running). A later user gesture
  retries the audio setup.
- `'audio-blocked'`: the audio context is suspended; the page can prompt for a click that calls
  `enableAudio()`. Emulation keeps running until sound is available.
- `'display-warning'`: the current `displayWarning` when it changes, including `null` when the
  warning clears. Embedding pages should display it; the demo shows a warning below the canvas.
- `'display-size'`: the new `displaySize` after a physical canvas resize.
- `'stats'`: once a second while running, `{ fps, speed }`: the frames emulated in that second,
  and the times real time they make (`1` is a real CPC). It also includes `audio`, either `null`
  or `{ underruns, bufferedMs, targetMs, playing }`. An increasing `underruns` count indicates
  that the audio buffer ran dry; it does not detect every possible audio glitch. `performance`
  contains `frameMs` (average), `maxFrameMs`, `drawMs` (average CPU-side draw call duration),
  `draws` (count), `drawFps` and `refreshFps` (per second), and `replaced` (frame advances
  replaced by another before a draw). These count draw submissions and scheduled callbacks,
  not frames actually presented by the browser or GPU. Draw timing does not measure asynchronous
  GPU completion.

To bypass stale cached assets when testing, add a new `?bemuVersion=...` value to the demo URL
and reload. The demo passes this value to the emulator module, which propagates its query string
to the worker, engine, audio worklet, bindings and default WASM URL. Embedding pages can do the
same by importing `bemu.js?v=...` and changing the version whenever they deploy a new bundle.
Serve the page itself with appropriate cache headers or reload it before testing a new version.
An explicitly configured `wasmUrl` is used as supplied. The exported `DIAGNOSTIC_REVISION`
identifies the diagnostic protocol, not the precise build of every asset.

### CSL scripts

`runCsl(script, { files })` plays a script live on a machine of its own.
- **`script`**: the script's text (several lines), a URL, or a `File`/`Blob`/bytes.
- **`files`**: maps the names the script uses to their sources. Files can also be given
  beforehand with `addFile`.
- **Names:** a file is found by its name only, whatever directory and case the script gives.
  A disc a script names is also looked for inside the zip archives given (a production's
  download, whose DSK is in a directory), by its file name.
  When a script names a disc it wasn't given and exactly one disc (or zip archive) was given, that one is used,
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
  key in the emulator. AudioWorklet also needs HTTPS or localhost; errors are reported through
  `'audio-error'` and in the console.
- **Fullscreen:** the browser's Fullscreen API on the canvas.

### Timing

A web page cannot set the display's refresh rate (the browser's fullscreen only enlarges the
page: the screen keeps the desktop's rate), so the library measures it.
- **About 50 Hz, 100, 150 or 200 Hz** (a monitor set to it): the browser targets one CPC frame
  every 1, 2, 3 or 4 refreshes. The sound card's clock is independent, so audio refills can produce
  frames between refreshes. A pending frame is drawn at the next refresh before advancing again;
  actual presentation and equal frame durations depend on the browser and available performance.
- **Otherwise** (60, 120, 144 Hz…): the sound's clock paces the emulation, as on the desktop,
  and each refresh shows the latest frame: at 60 Hz, one frame in five is shown twice, which
  a scrolling shows as a slight judder. Before sound starts, the browser's clock paces it, at
  50 frames per second.

On a computer that cannot keep up, catch-up work at normal speed is limited between browser
refreshes to keep input and audio responsive. The emulator still slows down if a single CPC frame
takes longer than real time; the limit does not make the emulation itself faster.

### Sound quality

The worklet plays from a ring buffer. It starts once about 60 ms are queued (and again after an
underrun), changes its playback speed by at most 1% to follow the queue instead of dropping or
repeating samples, and fades out or in around a gap rather than clicking. After an underrun the
queue target grows (up to 200 ms): a slow or busy machine trades latency for continuity.

### Drawing
- **WebGPU:** used when the browser has it. The picture fills the canvas, letterboxed, with the
  CRT profiles drawn on the GPU (the shader of the desktop's `--crt-gpu`), seen through the
  tube's glass, flat by default; `curvature: 1` bulges the picture and rounds the corners, as
  on the desktop. The raw display is never curved. CTM's 1440×1088 phosphor texture uses
  pixel-aligned integer scaling when the canvas drawing buffer is large enough, with black
  margins. On a smaller canvas the picture remains visible but is downscaled and may show
  aliasing; enlarge the canvas or use fullscreen. Sizes are physical pixels (CSS pixels times
  device pixel ratio), not necessarily CSS pixels. Browser zoom or compositing can still resample
  the canvas itself.
- **Canvas 2D:** used otherwise. It shows the raw picture at 768×544, and the page scales the
  canvas.

### Files from other sites

URLs are loaded with `fetch`, so a file on another site needs that site to allow it (CORS).
Otherwise, let the user pick the file, or download it through your own server.
