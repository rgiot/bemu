// bemu: an Amstrad CPC emulator to embed in a web page.
//
//   import { Bemu } from './bemu.js';
//   const emu = await Bemu.create(document.querySelector('#cpc'), { model: '6128', crtc: 1, display: 'ctm' });
//   await emu.insertDisc('A', 'https://example.org/game.dsk');
//   emu.start();
//
// The emulation runs in a Web Worker (worker.js, engine.js) that draws in the canvas and feeds the
// sound itself; this page-side class forwards the calls and the keys. Where a worker cannot draw
// (no OffscreenCanvas), it runs in the page instead, with the same API.
//
// See README.md for the whole API.

import { createCore, Engine, FRAME_MS, LocalTransport } from './engine.js';

/** Sound kept ahead of the speakers, in seconds (the engine adapts it, see engine.js). */
const AUDIO_TARGET_S = 0.06;

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

/** The page's side of an engine running in a Web Worker. */
export class WorkerTransport {
    #worker;
    #pending = new Map();
    #nextId = 1;
    #state = null;
    onEvent = () => {};
    onState = () => {};
    /** Whether the page has to send the display's refreshes (workers without requestAnimationFrame). */
    needsFrames = false;

    /** Starts a worker that draws in `canvas` (control of it moves to the worker). */
    static async create({ canvas, config, webgpu, wasmUrl }) {
        const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
        const transport = new WorkerTransport(worker);
        try {
            const offscreen = canvas.transferControlToOffscreen();
            const { hasFrames } = await transport.#request({ type: 'init', canvas: offscreen, config, webgpu, wasmUrl }, [offscreen]);
            transport.needsFrames = !hasFrames;
            return transport;
        } catch (error) {
            worker.terminate();
            throw error;
        }
    }

    constructor(worker) {
        this.#worker = worker;
        worker.onmessage = ({ data }) => this.#receive(data);
        worker.onerror = (event) => {
            event.preventDefault?.();
            this.#failAll(new Error(event.message || 'the emulator worker failed'));
        };
        worker.onmessageerror = () => this.#failAll(new Error('the emulator worker sent an unreadable message'));
    }

    get state() { return this.#state; }

    call(method, args = [], transfer = []) {
        return this.#request({ type: 'call', method, args }, transfer);
    }

    postFrame(time) { this.#worker.postMessage({ type: 'frame', time }); }

    dispose() {
        this.#failAll(new Error('the emulator was destroyed'));
        this.#worker.postMessage({ type: 'dispose' });
    }

    #request(message, transfer = []) {
        const id = this.#nextId++;
        return new Promise((resolve, reject) => {
            this.#pending.set(id, { resolve, reject });
            this.#worker.postMessage({ ...message, id }, transfer);
        });
    }

    #failAll(error) {
        for (const { reject } of this.#pending.values()) reject(error);
        this.#pending.clear();
    }

    #receive(data) {
        switch (data.type) {
            case 'state':
                this.#state = data.state;
                this.onState(data.state);
                break;
            case 'event':
                this.onEvent(data.event, data.event === 'error' ? new Error(data.detail.message) : data.detail);
                break;
            case 'result':
            case 'failure': {
                const request = this.#pending.get(data.id);
                this.#pending.delete(data.id);
                if (data.type === 'result') request?.resolve(data.value);
                else request?.reject(new Error(data.message));
                break;
            }
        }
    }
}

export class Bemu {
    /**
     * An emulator in `target`: a canvas, or an element a canvas is added to (filling it).
     * `config`: { model, crtc, gateArray, ramKb, discInterface, brand, display, blackLevel, keyboard }.
     * `options`: { webgpu (true), worker (true), wasmUrl, fullscreenKey ('F11', or null), autoStart (false) }.
     * With `worker`, the emulation runs in a Web Worker when the browser can draw in one.
     */
    static async create(target, config = {}, options = {}) {
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

        const canWorker = options.worker !== false && typeof Worker !== 'undefined'
            && typeof OffscreenCanvas !== 'undefined' && typeof canvas.transferControlToOffscreen === 'function';
        const canGpu = options.webgpu !== false && await Bemu.#hasWebGpu();
        // Each way fails differently (WebGPU, workers, drawing in a worker): the best that works.
        const attempts = [];
        if (canWorker && canGpu) attempts.push({ worker: true, webgpu: true });
        if (canWorker) attempts.push({ worker: true, webgpu: false });
        if (canGpu) attempts.push({ worker: false, webgpu: true });
        attempts.push({ worker: false, webgpu: false });

        let failure;
        for (const [index, attempt] of attempts.entries()) {
            if (index > 0) {
                // a canvas that was tried on can no longer be used for another context
                const fresh = canvas.cloneNode(false);
                canvas.replaceWith(fresh);
                canvas = fresh;
            }
            try {
                const size = Bemu.#measure(canvas);
                if (attempt.webgpu) {
                    canvas.width = size[0];
                    canvas.height = size[1];
                }
                const transport = attempt.worker
                    ? await WorkerTransport.create({ canvas, config, webgpu: attempt.webgpu, wasmUrl: options.wasmUrl })
                    : await Bemu.#localTransport(canvas, config, attempt.webgpu, options.wasmUrl);
                const emu = new Bemu(canvas, transport, options, size);
                if (options.autoStart) emu.start();
                return emu;
            } catch (error) {
                failure = error;
                console.warn(`bemu: ${attempt.worker ? 'worker' : 'page'}, ${attempt.webgpu ? 'WebGPU' : 'Canvas 2D'} failed:`, error);
            }
        }
        throw failure;
    }

    static async #localTransport(canvas, config, webgpu, wasmUrl) {
        const core = await createCore(canvas, config, webgpu, wasmUrl);
        return new LocalTransport(({ emit, onTick }) => new Engine(core, { canvas, emit, onTick }));
    }

    #canvas; #transport; #options;
    #listeners = new Map();
    #handlers = [];
    #running = false;
    #frameLoop = 0;
    #audio = null;
    #audioStarting = null;
    #volume = 1;
    #muted = false;
    /** Times real time: 1, more, or Infinity (as fast as possible). */
    #speed = 1;
    /** The speed F10 goes back to from full speed. */
    #lastSpeed = 1;
    #state;
    /** The canvas's drawing buffer in physical pixels (a worker's canvas cannot be read). */
    #size;
    #resizeObserver = null;
    #pixelRatioQuery = null;
    #pixelRatioChanged = null;
    #destroyed = false;
    #displayWarningKey = '';

    constructor(canvas, transport, options, size = [canvas.width, canvas.height]) {
        this.#canvas = canvas;
        this.#transport = transport;
        this.#size = size;
        this.#state = transport.state;
        this.#options = { fullscreenKey: 'F11', turboKey: 'F10', ...options };
        transport.onEvent = (event, detail) => this.#onEvent(event, detail);
        transport.onState = (state) => {
            this.#state = state;
            this.#updateDisplayWarning();
        };

        this.#listen(canvas, 'keydown', (event) => this.#keyDown(event));
        this.#listen(canvas, 'keyup', (event) => {
            this.#send('keyUp', event.code);
            if (this.#isCpcKey(event)) event.preventDefault();
        });
        this.#listen(canvas, 'blur', () => this.#send('releaseKeys'));
        this.#listen(canvas, 'pointerdown', () => {
            canvas.focus();
            this.#startAudio();
        });
        if (this.backend === 'webgpu') {
            const resize = () => {
                const next = Bemu.#measure(canvas);
                if (next[0] === this.#size[0] && next[1] === this.#size[1]) return;
                this.#size = next;
                this.#send('resize', next[0], next[1]);
                this.#updateDisplayWarning();
            };
            if (typeof ResizeObserver !== 'undefined') {
                this.#resizeObserver = new ResizeObserver(resize);
                this.#resizeObserver.observe(canvas);
            }
            this.#listen(window, 'resize', resize);
            if (window.matchMedia) {
                this.#pixelRatioChanged = () => {
                    this.#watchPixelRatio();
                    resize();
                };
                this.#watchPixelRatio();
            }
        }
        this.#displayWarningKey = JSON.stringify(this.displayWarning);
    }

    /** 'webgpu' or 'canvas2d' (raw picture only). */
    get backend() { return this.#state.backend; }
    get canvas() { return this.#canvas; }
    get running() { return this.#running; }
    /** Whether the emulation runs in a Web Worker (else in the page). */
    get threaded() { return this.#transport instanceof WorkerTransport; }
    /** Whether a CSL script is playing. */
    get cslRunning() { return this.#state.cslRunning; }
    /** Whether the frames follow the display's own refresh (50 Hz, or 100, 150 or 200). */
    get vsyncPaced() { return this.#state.refreshesPerFrame > 0; }
    /** At 50, 100, 150 or 200 Hz, how many refreshes show a CPC frame (1 to 4); else 0. */
    get refreshesPerFrame() { return this.#state.refreshesPerFrame; }
    /** Times real time: 1, another factor, or Infinity (full speed). */
    get speed() { return this.#speed; }
    /** A CTM scaling warning, or null when the physical canvas is large enough. */
    get displayWarning() {
        if (this.backend !== 'webgpu') return null;
        const [width, height] = this.#state.ctm;
        if (!width || (this.#size[0] >= width && this.#size[1] >= height)) return null;
        return { width: this.#size[0], height: this.#size[1], requiredWidth: width, requiredHeight: height };
    }

    /**
     * `fn(detail)` on 'screenshot' ({ name, png }), 'csl-done' (report), 'error' (Error),
     * 'audio-error' (Error), 'audio-ready' (sound started, after an error or not), 'display-warning' (see displayWarning), or 'stats' (each second while running: { fps, speed }, the frames
     * emulated a second and the times real time they make).
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

    // The calls below reach the emulator asynchronously: they return a promise, rejected when the
    // emulator refuses (a bad configuration, a disc that cannot be read...).

    /** A new machine (the discs stay in). */
    configure(config) { return this.#transport.call('configure', [config]); }
    reset() { return this.#transport.call('reset'); }
    /** 'raw', 'ctm', 'trinitron', 'gt65' or 'atari'. */
    setDisplay(name) { return this.#transport.call('setDisplay', [name]); }

    /** How curved a CRT's glass is, times its profile's bulge: 0 (flat) to 4, 1 by default. */
    setCurvature(curvature) { return this.#transport.call('setCurvature', [curvature]); }

    /** 'symbolic' (a key types its character) or 'positional' (the CPC key at its place, for games). */
    setKeyboard(mapping) { return this.#transport.call('setKeyboard', [mapping]); }

    /**
     * A DSK image in drive 'A' or 'B', from a URL, a File/Blob or bytes: a zip, 7z or tar.gz
     * archive gives its first DSK, or the one named `entry`.
     */
    async insertDisc(drive, source, entry) {
        await this.#transport.call('insertDisc', [driveNumber(drive), await bytesOf(source), entry]);
    }
    /**
     * Changes the disc of the running machine like a person: the drive stays empty for 1.5 s of
     * emulated time (a program can see the change), then the new disc goes in.
     */
    async swapDisc(drive, source, entry) {
        await this.#transport.call('swapDisc', [driveNumber(drive), await bytesOf(source), entry]);
    }
    eject(drive) { return this.#transport.call('eject', [driveNumber(drive)]); }
    /** A CDT tape in the cassette deck (PLAY down), from a URL, a File/Blob or bytes. */
    async insertTape(source) { return this.#transport.call('insertTape', [await bytesOf(source)]); }
    ejectTape() { return this.#transport.call('ejectTape'); }
    /** The deck's keys: 'play', 'stop' or 'rewind'. */
    tapeControl(key) { return this.#transport.call('tapeControl', [key]); }
    /** { position, duration } in seconds and whether PLAY is down, or undefined without a tape. */
    get tape() { return this.#state.tape ?? undefined; }

    /** An SNA snapshot, from a URL, a File/Blob or bytes. */
    async loadSnapshot(source) { await this.#transport.call('loadSnapshot', [await bytesOf(source)]); }
    /** A file a CSL script names (disk_insert, snapshot_load, csl_load). */
    async addFile(name, source) { await this.#transport.call('addFile', [name, await bytesOf(source)]); }

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
        await this.#transport.call('runCsl', [text]);
        this.#setRunning(true);
    }
    stopCsl() { return this.#transport.call('stopCsl'); }

    /**
     * How fast the emulation runs: 1 (real time, the default), another factor (2: twice as
     * fast...), or 'max' / Infinity (as fast as the computer can, the picture shown at the
     * display's rate). The sound only plays at real time. The `turboKey` (F10) toggles full speed.
     */
    setSpeed(speed) {
        const value = speed === 'max' ? Infinity : Number(speed);
        if (!(value > 0)) throw new RangeError(`speed must be a positive number or 'max', not ${speed}`);
        this.#speed = value;
        return this.#transport.call('setSpeed', [value]);
    }

    /** Type `text` on the CPC's keyboard (\n is RETURN), as if typed: a paste. */
    typeText(text) { return this.#transport.call('typeText', [String(text)]); }

    /** While paused: one more frame, shown. */
    stepFrame() {
        if (this.#running) return Promise.resolve();
        return this.#transport.call('stepFrame');
    }

    /** The last frame as a PNG file (a Promise of a Uint8Array, the monitor's 768x272 picture), or undefined. */
    screenshot() { return this.#transport.call('screenshot'); }

    /** The DSK image in drive 'A' or 'B' with what the CPC wrote on it (a Promise of a Uint8Array), or undefined. */
    getDisc(drive) { return this.#transport.call('getDisc', [driveNumber(drive)]); }
    /** Whether the drive motor runs (the drive is being accessed). */
    get driveActive() { return this.#state.driveActive; }
    /** How many disc transfers began so far (wrapping): the drive light flashes when it changes. */
    get driveTransfers() { return this.#state.driveTransfers; }
    /** Press (down true) or release a key of the CPC by its KeyboardEvent `code` and `key`, as a virtual keyboard does. */
    pressKey(code, key, down) {
        if (down) this.#send('keyDown', code, key, false);
        else this.#send('keyUp', code);
    }

    /** The machine's state as an SNA snapshot file (a Promise of a Uint8Array, version 3), for loadSnapshot later. */
    saveSnapshot() { return this.#transport.call('saveSnapshot'); }

    start() {
        if (this.#destroyed) return;
        this.#setRunning(true);
    }
    pause() {
        this.#setRunning(false);
    }

    /** The emulator alone on the screen (the browser's Fullscreen API). */
    async fullscreen() {
        if (document.fullscreenElement === this.#canvas) await document.exitFullscreen();
        else await this.#canvas.requestFullscreen();
        this.#canvas.focus();
    }

    /**
     * Start the sound now, from a click or key handler of the page (browsers only allow it after
     * one), rather than at the first click or key in the emulator.
     */
    enableAudio() { return this.#startAudio(); }

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
        if (this.#destroyed) return;
        this.pause();
        this.#destroyed = true;
        for (const [target, type, fn] of this.#handlers) target.removeEventListener(type, fn);
        this.#handlers = [];
        this.#resizeObserver?.disconnect();
        this.#pixelRatioQuery?.removeEventListener('change', this.#pixelRatioChanged);
        this.#audio?.context.close();
        this.#audio = null;
        this.#listeners.clear();
        this.#transport.dispose();
    }

    // ---------------------------------------------------------------- internals

    static async #hasWebGpu() {
        try {
            return Boolean(await navigator.gpu?.requestAdapter());
        } catch {
            return false;
        }
    }

    /** The canvas's drawing buffer for its size on the screen: physical pixels. */
    static #measure(canvas) {
        const ratio = window.devicePixelRatio || 1;
        return [
            Math.max(1, Math.round((canvas.clientWidth || 768) * ratio)),
            Math.max(1, Math.round((canvas.clientHeight || 544) * ratio))
        ];
    }

    #watchPixelRatio() {
        this.#pixelRatioQuery?.removeEventListener('change', this.#pixelRatioChanged);
        this.#pixelRatioQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
        this.#pixelRatioQuery.addEventListener('change', this.#pixelRatioChanged);
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

    #onEvent(event, detail) {
        if (event === 'error') {
            // the engine stopped itself
            this.#setRunning(false, false);
            if (!this.#listeners.get('error')?.size) console.error(detail);
        }
        this.#emit(event, detail);
    }

    /** A call whose outcome only matters when it fails: input, resizing. */
    #send(method, ...args) {
        this.#transport.call(method, args).catch((error) => {
            if (this.#destroyed) return;
            this.#emit('error', error);
            if (!this.#listeners.get('error')?.size) console.error(error);
        });
    }

    #setRunning(running, tellEngine = true) {
        if (running === this.#running && tellEngine) {
            // the engine may have stopped itself since: ask again
            if (!running) return;
        }
        this.#running = running;
        if (tellEngine) this.#send(running ? 'start' : 'pause');
        if (running) this.#audio?.context.resume();
        else this.#audio?.context.suspend();
        this.#driveFrames();
    }

    /** Workers without requestAnimationFrame get the display's refreshes from the page. */
    #driveFrames() {
        cancelAnimationFrame(this.#frameLoop);
        if (!this.#running || !this.#transport.needsFrames) return;
        const loop = (time) => {
            this.#transport.postFrame(time);
            this.#frameLoop = requestAnimationFrame(loop);
        };
        this.#frameLoop = requestAnimationFrame(loop);
    }

    #updateDisplayWarning() {
        const warning = this.displayWarning;
        const key = JSON.stringify(warning);
        if (key !== this.#displayWarningKey) {
            this.#displayWarningKey = key;
            this.#emit('display-warning', warning);
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
        this.#send('keyDown', event.code, event.key, event.ctrlKey || event.metaKey);
        // the page does not scroll or move the focus on the CPC's keys
        if (this.#isCpcKey(event)) event.preventDefault();
    }

    async #startAudio() {
        if (this.#destroyed) return;
        if (this.#audio) {
            if (this.#running && this.#audio.context.state === 'suspended') this.#audio.context.resume();
            return;
        }
        if (this.#audioStarting) return this.#audioStarting;
        this.#audioStarting = (async () => {
            let context;
            try {
                if (typeof AudioContext === 'undefined') throw new Error('Web Audio is not available in this browser.');
                context = new AudioContext({ latencyHint: 'interactive' });
                if (!context.audioWorklet) {
                    throw new Error('AudioWorklet is unavailable. Serve this page over HTTPS or use localhost, and check browser support.');
                }
                await context.audioWorklet.addModule(new URL('./audio-worklet.js', import.meta.url));
                if (this.#destroyed) {
                    await context.close();
                    return;
                }
                const node = new AudioWorkletNode(context, 'bemu-audio', {
                    outputChannelCount: [2],
                    processorOptions: { target: AUDIO_TARGET_S }
                });
                const gain = new GainNode(context, { gain: this.#muted ? 0 : this.#volume });
                node.connect(gain).connect(context.destination);
                if (!this.#running) await context.suspend();
                if (this.#destroyed) {
                    await context.close();
                    return;
                }
                // the emulator talks to the worklet directly, not through this page
                const channel = new MessageChannel();
                node.port.postMessage({ feed: channel.port1 }, [channel.port1]);
                await this.#transport.call('attachAudio', [channel.port2, context.sampleRate], [channel.port2]);
                if (this.#destroyed) {
                    await context.close();
                    return;
                }
                this.#audio = { context, node, gain };
                this.#emit('audio-ready');
            } catch (error) {
                if (context && !this.#audio && context.state !== 'closed') {
                    try { await context.close(); } catch (closeError) { console.warn('bemu: cannot close audio context', closeError); }
                }
                if (!this.#destroyed) {
                    console.warn('bemu: no sound', error);
                    this.#emit('audio-error', error);
                }
            }
        })().finally(() => { this.#audioStarting = null; });
        return this.#audioStarting;
    }

    #applyVolume() {
        if (this.#audio) this.#audio.gain.gain.value = this.#muted ? 0 : this.#volume;
    }
}

export { FRAME_MS };
