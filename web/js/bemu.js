// bemu: an Amstrad CPC emulator to embed in a web page.
//
//   import { Bemu } from './bemu.js';
//   const emu = await Bemu.create(document.querySelector('#cpc'), { model: '6128', crtc: 1, display: 'ctm' });
//   await emu.insertDisc('A', 'https://example.org/game.dsk');
//   emu.start();
//
// See README.md for the whole API.

import init, { Emulator } from './bemu_web.js';

/** The CPC's frame: 312 lines of 64 µs. */
const FRAME_MS = 312 * 64 / 1000;
/** Sound kept ahead of the speakers, in seconds. */
const AUDIO_TARGET_S = 0.06;
const AUDIO_MAX_S = 0.25;
/** At more than real time, the emulation takes this share of each animation frame (the rest is
 * the page's: drawing, input), between these many milliseconds: on a computer too slow to keep
 * up, the refreshes get longer and so does the emulation's share. */
const BUSY_SHARE = 0.9;
const BUSY_MIN_MS = 40;
const BUSY_MAX_MS = 100;

let wasm;
function loadWasm(url) {
    wasm ??= init(url ? { module_or_path: url } : undefined);
    return wasm;
}

/** The bytes of a URL (string or URL), a Blob/File, an ArrayBuffer or a typed array. */
export async function bytesOf(source) {
    if (source instanceof Uint8Array) return source;
    if (source instanceof ArrayBuffer) return new Uint8Array(source);
    if (ArrayBuffer.isView(source)) return new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
    if (typeof Blob !== 'undefined' && source instanceof Blob) return new Uint8Array(await source.arrayBuffer());
    if (typeof source === 'string' || source instanceof URL) {
        const response = await fetch(source);
        if (!response.ok) throw new Error(`${source}: HTTP ${response.status}`);
        return new Uint8Array(await response.arrayBuffer());
    }
    throw new TypeError('expected a URL, a Blob or bytes');
}

function driveNumber(drive) {
    if (drive === 0 || drive === 1) return drive;
    if (typeof drive === 'string' && /^[ab]$/i.test(drive)) return drive.toUpperCase() === 'A' ? 0 : 1;
    throw new RangeError(`drive must be 'A' or 'B', not ${drive}`);
}

export class Bemu {
    /**
     * An emulator in `target`: a canvas, or an element a canvas is added to (filling it).
     * `config`: { model, crtc, gateArray, ramKb, discInterface, brand, display, blackLevel }.
     * `options`: { webgpu (true), wasmUrl, fullscreenKey ('F11', or null), autoStart (false) }.
     */
    static async create(target, config = {}, options = {}) {
        await loadWasm(options.wasmUrl);
        let canvas = target;
        if (!(target instanceof HTMLCanvasElement)) {
            canvas = document.createElement('canvas');
            canvas.style.width = '100%';
            canvas.style.height = '100%';
            canvas.style.display = 'block';
            target.appendChild(canvas);
        }
        if (canvas.tabIndex < 0) canvas.tabIndex = 0;
        if (!canvas.style.background) canvas.style.background = 'black';
        let core = null;
        if (options.webgpu !== false && await Bemu.#hasWebGpu()) {
            Bemu.#fitCanvas(canvas);
            try {
                core = await Emulator.create(canvas, config, true);
            } catch (error) {
                console.warn('bemu: no WebGPU, the picture is drawn raw on Canvas 2D:', error);
                // a canvas WebGPU was tried on has no 2D context: a new one takes its place
                const fresh = canvas.cloneNode(false);
                canvas.replaceWith(fresh);
                canvas = fresh;
            }
        }
        core ??= await Emulator.create(canvas, config, false);
        const emu = new Bemu(canvas, core, options);
        if (options.autoStart) emu.start();
        return emu;
    }

    #canvas; #core; #options;
    #listeners = new Map();
    #handlers = [];
    #running = false;
    #frameRequest = 0;
    #lastTime = undefined;
    #owed = 0;
    #intervals = [];
    #vsyncPaced = false;
    #audio = null;
    #audioStarting = null;
    #volume = 1;
    #muted = false;
    /** Times real time: 1, more, or Infinity (as fast as possible). */
    #speed = 1;
    /** The speed F10 goes back to from full speed. */
    #lastSpeed = 1;
    #stats = { since: undefined, frames: 0 };
    #resizeObserver = null;
    #destroyed = false;

    constructor(canvas, core, options) {
        this.#canvas = canvas;
        this.#core = core;
        this.#options = { fullscreenKey: 'F11', turboKey: 'F10', ...options };
        core.on_screenshot((name, png) => this.#emit('screenshot', { name, png }));
        core.on_csl_done((report) => this.#emit('csl-done', report));

        this.#listen(canvas, 'keydown', (event) => this.#keyDown(event));
        this.#listen(canvas, 'keyup', (event) => {
            this.#core.key_up(event.code);
            if (this.#isCpcKey(event)) event.preventDefault();
        });
        this.#listen(canvas, 'blur', () => this.#core.release_keys());
        this.#listen(canvas, 'pointerdown', () => {
            canvas.focus();
            this.#startAudio();
        });
        if (this.backend === 'webgpu' && typeof ResizeObserver !== 'undefined') {
            this.#resizeObserver = new ResizeObserver(() => {
                Bemu.#fitCanvas(canvas);
                core.resize(canvas.width, canvas.height);
                if (!this.#running) this.#safely(() => core.redraw());
            });
            this.#resizeObserver.observe(canvas);
        }
    }

    /** 'webgpu' or 'canvas2d' (raw picture only). */
    get backend() { return this.#core.backend(); }
    get canvas() { return this.#canvas; }
    get running() { return this.#running; }
    /** Whether a CSL script is playing. */
    get cslRunning() { return this.#core.csl_running(); }
    /** Whether the frames follow the display's own 50 Hz refresh. */
    get vsyncPaced() { return this.#vsyncPaced; }
    /** Times real time: 1, another factor, or Infinity (full speed). */
    get speed() { return this.#speed; }

    /**
     * `fn(detail)` on 'screenshot' ({ name, png }), 'csl-done' (report), 'error' (Error) or
     * 'stats' (each second while running: { fps, speed }, the frames emulated a second and the
     * times real time they make).
     */
    on(event, fn) {
        if (!this.#listeners.has(event)) this.#listeners.set(event, new Set());
        this.#listeners.get(event).add(fn);
        return this;
    }
    off(event, fn) {
        this.#listeners.get(event)?.delete(fn);
        return this;
    }

    /** A new machine (the discs stay in). */
    configure(config) { this.#core.configure(config); }
    reset() { this.#core.reset(); }
    /** 'raw', 'ctm', 'trinitron', 'gt65' or 'atari'. */
    setDisplay(name) {
        this.#core.set_display(name);
        if (!this.#running) this.#safely(() => this.#core.redraw());
    }

    /** A DSK image in drive 'A' or 'B', from a URL, a File/Blob or bytes. */
    async insertDisc(drive, source) { this.#core.insert_disc(driveNumber(drive), await bytesOf(source)); }
    eject(drive) { this.#core.eject(driveNumber(drive)); }
    /** An SNA snapshot, from a URL, a File/Blob or bytes. */
    async loadSnapshot(source) { this.#core.load_snapshot(await bytesOf(source)); }
    /** A file a CSL script names (disk_insert, snapshot_load, csl_load). */
    async addFile(name, source) { this.#core.add_file(name, await bytesOf(source)); }

    /**
     * Play a CSL script live: `script` is its text (several lines), a URL, a File/Blob or bytes;
     * `files` maps the names it uses to URLs/Files/bytes. 'screenshot' events follow, then
     * 'csl-done'. The emulator is started.
     */
    async runCsl(script, { files = {} } = {}) {
        for (const [name, source] of Object.entries(files)) await this.addFile(name, source);
        const text = typeof script === 'string' && script.includes('\n')
            ? script
            : new TextDecoder().decode(await bytesOf(script));
        this.#core.run_csl(text);
        this.start();
    }
    stopCsl() { this.#core.stop_csl(); }

    /**
     * How fast the emulation runs: 1 (real time, the default), another factor (2: twice as
     * fast...), or 'max' / Infinity (as fast as the computer can, the picture shown at the
     * display's rate). The sound only plays at real time. The `turboKey` (F10) toggles full speed.
     */
    setSpeed(speed) {
        const value = speed === 'max' ? Infinity : Number(speed);
        if (!(value > 0)) throw new RangeError(`speed must be a positive number or 'max', not ${speed}`);
        if (value !== 1) this.#core.audio_samples(); // what is queued is dropped, not played late
        this.#speed = value;
        this.#owed = 0;
    }

    /** Type `text` on the CPC's keyboard (\n is RETURN), as if typed: a paste. */
    typeText(text) { this.#core.type_text(String(text)); }

    /** While paused: one more frame, shown. */
    stepFrame() {
        if (this.#running) return;
        this.#safely(() => {
            this.#core.advance_frame();
            this.#core.audio_samples();
            this.#core.redraw();
        });
    }

    /** The last frame as a PNG file (Uint8Array, the monitor's 768x272 picture), or undefined. */
    screenshot() { return this.#core.screenshot(); }

    /** The DSK image in drive 'A' or 'B' with what the CPC wrote on it (Uint8Array), or undefined. */
    getDisc(drive) { return this.#core.disc_image(driveNumber(drive)); }

    start() {
        if (this.#running || this.#destroyed) return;
        this.#running = true;
        this.#lastTime = undefined;
        this.#owed = 0;
        this.#stats.since = undefined;
        this.#frameRequest = requestAnimationFrame((time) => this.#tick(time));
    }
    pause() {
        this.#running = false;
        cancelAnimationFrame(this.#frameRequest);
        this.#audio?.context.suspend();
    }

    /** The emulator alone on the screen (the browser's Fullscreen API). */
    async fullscreen() {
        if (document.fullscreenElement === this.#canvas) await document.exitFullscreen();
        else await this.#canvas.requestFullscreen();
        this.#canvas.focus();
    }

    /** 0 (silent) to 1. Sound starts with the first click or key in the emulator. */
    setVolume(volume) {
        this.#volume = Math.max(0, Math.min(1, volume));
        this.#applyVolume();
    }
    /** Silence the sound, or bring it back at its volume. */
    setMuted(muted) {
        this.#muted = Boolean(muted);
        this.#applyVolume();
    }
    get muted() { return this.#muted; }

    destroy() {
        this.pause();
        this.#destroyed = true;
        for (const [target, type, fn] of this.#handlers) target.removeEventListener(type, fn);
        this.#handlers = [];
        this.#resizeObserver?.disconnect();
        this.#audio?.context.close();
        this.#audio = null;
        this.#listeners.clear();
        this.#core.free();
    }

    // ---------------------------------------------------------------- internals

    static async #hasWebGpu() {
        try {
            return Boolean(await navigator.gpu?.requestAdapter());
        } catch {
            return false;
        }
    }

    static #fitCanvas(canvas) {
        const ratio = window.devicePixelRatio || 1;
        const width = Math.max(1, Math.round((canvas.clientWidth || 768) * ratio));
        const height = Math.max(1, Math.round((canvas.clientHeight || 544) * ratio));
        if (canvas.width !== width) canvas.width = width;
        if (canvas.height !== height) canvas.height = height;
    }

    #listen(target, type, fn) {
        target.addEventListener(type, fn);
        this.#handlers.push([target, type, fn]);
    }

    #emit(event, detail) {
        for (const fn of this.#listeners.get(event) ?? []) {
            try { fn(detail); } catch (error) { console.error(error); }
        }
    }

    #safely(fn) {
        try { return fn(); } catch (error) {
            this.pause();
            this.#emit('error', error);
            if (!this.#listeners.get('error')?.size) console.error(error);
        }
    }

    #isCpcKey(event) {
        return !event.ctrlKey && !event.metaKey && !/^F\d+$/.test(event.key);
    }

    #keyDown(event) {
        this.#startAudio();
        if (this.#options.fullscreenKey && event.key === this.#options.fullscreenKey) {
            event.preventDefault();
            if (!event.repeat) this.fullscreen();
            return;
        }
        if (this.#options.turboKey && event.key === this.#options.turboKey) {
            event.preventDefault();
            if (!event.repeat) {
                if (this.#speed === Infinity) this.setSpeed(this.#lastSpeed);
                else {
                    this.#lastSpeed = this.#speed;
                    this.setSpeed(Infinity);
                }
            }
            return;
        }
        this.#core.key_down(event.code, event.key, event.ctrlKey || event.metaKey);
        // the page does not scroll or move the focus on the CPC's keys
        if (this.#isCpcKey(event)) event.preventDefault();
    }

    async #startAudio() {
        if (this.#audio || this.#audioStarting || typeof AudioContext === 'undefined') return;
        this.#audioStarting = (async () => {
            const context = new AudioContext({ latencyHint: 'interactive' });
            await context.audioWorklet.addModule(new URL('./audio-worklet.js', import.meta.url));
            const node = new AudioWorkletNode(context, 'bemu-audio', { outputChannelCount: [2] });
            const gain = new GainNode(context, { gain: this.#muted ? 0 : this.#volume });
            node.connect(gain).connect(context.destination);
            this.#core.enable_audio(context.sampleRate);
            this.#audio = { context, node, gain, sent: 0, startedAt: context.currentTime };
            if (!this.#running) context.suspend();
        })().catch((error) => console.warn('bemu: no sound', error));
    }

    /** Seconds of sound queued and not yet played. */
    #audioBuffered() {
        const audio = this.#audio;
        if (!audio) return 0;
        const played = (audio.context.currentTime - audio.startedAt);
        return audio.sent / audio.context.sampleRate - played;
    }

    #applyVolume() {
        if (this.#audio) this.#audio.gain.gain.value = this.#muted ? 0 : this.#volume;
    }

    /**
     * A frame emulated, not drawn; its sound queued at real time, dropped otherwise. `unseen`: no
     * frame of it is shown (above real time, all but the last of a refresh), it runs without
     * any picture.
     */
    #runFrame(unseen = false) {
        if (unseen) this.#core.advance_frame_unseen();
        else this.#core.advance_frame();
        this.#stats.frames++;
        const audio = this.#audio;
        if (!audio) return;
        const samples = this.#core.audio_samples();
        if (samples.length === 0 || this.#speed !== 1) return;
        if (this.#audioBuffered() > AUDIO_MAX_S) return; // too far ahead: drop
        audio.sent = Math.max(audio.sent, (audio.context.currentTime - audio.startedAt) * audio.context.sampleRate);
        audio.sent += samples.length / 2;
        audio.node.port.postMessage(samples, [samples.buffer]);
    }

    #tick(time) {
        if (!this.#running) return;
        this.#frameRequest = requestAnimationFrame((next) => this.#tick(next));
        if (this.#audio?.context.state === 'suspended') this.#audio.context.resume();
        const elapsed = this.#lastTime === undefined ? FRAME_MS : time - this.#lastTime;
        this.#lastTime = time;
        this.#measureRefresh(elapsed);
        this.#safely(() => {
            this.#emitStats(time);
            const budget = Math.min(Math.max(elapsed * BUSY_SHARE, BUSY_MIN_MS), BUSY_MAX_MS);
            if (this.#speed === Infinity) {
                // full speed: frames without picture for most of the refresh, then one shown
                const start = performance.now();
                while (performance.now() - start < budget) this.#runFrame(true);
                this.#runFrame();
                this.#core.redraw();
                return;
            }
            if (this.#speed !== 1) {
                // n times real time, by the browser's clock
                this.#owed = Math.min(this.#owed + elapsed * this.#speed, 4 * this.#speed * FRAME_MS);
                const start = performance.now();
                while (this.#owed >= 2 * FRAME_MS && performance.now() - start < budget) {
                    this.#runFrame(true);
                    this.#owed -= FRAME_MS;
                }
                if (this.#owed >= FRAME_MS) {
                    this.#runFrame();
                    this.#owed -= FRAME_MS;
                }
                this.#core.redraw();
                return;
            }
            if (this.#vsyncPaced) {
                // one CPC frame per refresh of a 50 Hz display
                this.#runFrame();
                this.#core.redraw();
                return;
            }
            let frames;
            if (this.#audio) {
                // the sound's clock: keep a little of it ahead of the speakers
                frames = Math.ceil((AUDIO_TARGET_S - this.#audioBuffered()) * 1000 / FRAME_MS);
            } else {
                this.#owed = Math.min(this.#owed + elapsed, 4 * FRAME_MS);
                frames = Math.floor(this.#owed / FRAME_MS);
                this.#owed -= frames * FRAME_MS;
            }
            frames = Math.max(0, Math.min(frames, 4));
            for (let i = 0; i < frames; i++) this.#runFrame();
            this.#core.redraw();
        });
    }

    /** The 'stats' event, once a second. */
    #emitStats(time) {
        const stats = this.#stats;
        if (stats.since === undefined) {
            stats.since = time;
            stats.frames = 0;
            return;
        }
        if (time - stats.since < 1000) return;
        const fps = stats.frames * 1000 / (time - stats.since);
        stats.since = time;
        stats.frames = 0;
        this.#emit('stats', { fps, speed: fps * FRAME_MS / 1000 });
    }

    /** Whether the display refreshes at about 50 Hz, from the last 60 animation frames. */
    #measureRefresh(elapsed) {
        this.#intervals.push(elapsed);
        if (this.#intervals.length > 60) this.#intervals.shift();
        if (this.#intervals.length < 30) return;
        const sorted = [...this.#intervals].sort((a, b) => a - b);
        const median = sorted[sorted.length >> 1];
        this.#vsyncPaced = Math.abs(1000 / median - 50) < 1.5;
    }
}
