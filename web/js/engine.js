// The emulation loop of bemu: the wasm core, its pacing and its sound feed. It runs in a Web
// Worker (worker.js), so the page's own work never makes the emulation or the sound stutter, or
// in the page itself when workers cannot draw (LocalTransport).

import init, { Emulator } from './bemu_web.js';

/** The CPC's frame: 312 lines of 64 µs. */
export const FRAME_MS = 312 * 64 / 1000;
/** Sound kept ahead of the speakers, in seconds. */
const AUDIO_TARGET_S = 0.06;
/** After an underrun the target grows by this factor, up to the maximum: a slow or busy machine
 * trades latency for continuity. */
const AUDIO_TARGET_GROWTH = 1.5;
const AUDIO_TARGET_MAX_S = 0.2;
const AUDIO_MAX_S = 0.4;
/** At more than real time, the emulation takes this share of each animation frame (the rest is
 * the page's: drawing, input), between these many milliseconds: on a computer too slow to keep
 * up, the refreshes get longer and so does the emulation's share. */
const BUSY_SHARE = 0.9;
const BUSY_MIN_MS = 40;
const BUSY_MAX_MS = 100;
/** Limit catch-up work at normal speed so a slow machine can still service input and audio. */
const REALTIME_BUDGET_MS = FRAME_MS;

let wasm;
export function loadWasm(url) {
    wasm ??= init(url ? { module_or_path: url } : undefined);
    return wasm;
}

/** The core, drawing in `canvas` (a canvas, or an OffscreenCanvas). */
export async function createCore(canvas, config, webgpu, wasmUrl) {
    await loadWasm(wasmUrl);
    return Emulator.create(canvas, config, webgpu);
}

const defaultScheduler = {
    request: (callback) => requestAnimationFrame(callback),
    cancel: (id) => cancelAnimationFrame(id)
};

export class Engine {
    #core; #canvas; #emit; #onTick; #scheduler; #now;
    #running = false;
    /** A frame was emulated by the sound's clock and not drawn yet. */
    #fresh = false;
    #frameRequest = 0;
    #lastTime = undefined;
    #owed = 0;
    #intervals = [];
    /** 1, 2, 3 or 4 refreshes of a display at 50, 100, 150 or 200 Hz per CPC frame; 0 at
     * another rate. */
    #refreshesPerFrame = 0;
    #refreshes = 0;
    #audio = null;
    /** Times real time: 1, more, or Infinity (as fast as possible). */
    #speed = 1;
    #stats = { since: undefined, frames: 0 };

    /**
     * `core`: the wasm Emulator. `emit(event, detail)`: what the page hears ('screenshot',
     * 'csl-done', 'stats', 'error'). `onTick()`: after each refresh. `scheduler`: { request(callback), cancel(id) } for the
     * display's refreshes. `canvas`: resized when the page's size changes (WebGPU).
     */
    constructor(core, { canvas, emit = () => {}, onTick = () => {}, scheduler = defaultScheduler, now = () => performance.now() } = {}) {
        this.#core = core;
        this.#canvas = canvas;
        this.#emit = emit;
        this.#onTick = onTick;
        this.#scheduler = scheduler;
        this.#now = now;
        core.on_screenshot((name, png) => emit('screenshot', { name, png }));
        core.on_csl_done((report) => emit('csl-done', report));
    }

    /** What the page may read at once: it is sent whenever it changes. */
    state() {
        const tape = this.#core.tape_position?.();
        return {
            backend: this.#core.backend(),
            running: this.#running,
            speed: this.#speed,
            refreshesPerFrame: this.#refreshesPerFrame,
            cslRunning: this.#core.csl_running?.() ?? false,
            ctm: [this.#core.ctm_width?.() ?? 0, this.#core.ctm_height?.() ?? 0],
            tape: tape ? { position: tape[0], duration: tape[1], playing: tape[2] !== 0 } : null,
            driveActive: this.#core.drive_active?.() ?? false,
            driveTransfers: this.#core.drive_transfers?.() ?? 0
        };
    }

    // ------------------------------------------------------------ the page's calls

    configure(config) { this.#core.configure(config); }
    reset() { this.#core.reset(); }
    setDisplay(name) {
        this.#core.set_display(name);
        this.#redrawIfPaused();
    }
    setCurvature(curvature) {
        this.#core.set_curvature(curvature);
        this.#redrawIfPaused();
    }
    setKeyboard(mapping) { this.#core.set_keyboard(mapping); }
    resize(width, height) {
        if (this.state().backend !== 'webgpu') return;
        if (this.#canvas) {
            if (this.#canvas.width !== width) this.#canvas.width = width;
            if (this.#canvas.height !== height) this.#canvas.height = height;
        }
        this.#core.resize(width, height);
        this.#redrawIfPaused();
    }
    insertDisc(drive, bytes, entry) { this.#core.insert_disc(drive, bytes, entry); }
    swapDisc(drive, bytes, entry) { this.#core.swap_disc(drive, bytes, entry); }
    eject(drive) { this.#core.eject(drive); }
    insertTape(bytes) { return this.#core.insert_tape(bytes); }
    ejectTape() { this.#core.eject_tape(); }
    tapeControl(key) { this.#core.tape_control(key); }
    loadSnapshot(bytes) { this.#core.load_snapshot(bytes); }
    addFile(name, bytes) { this.#core.add_file(name, bytes); }
    runCsl(text) {
        this.#core.run_csl(text);
        this.start();
    }
    stopCsl() { this.#core.stop_csl(); }
    typeText(text) { this.#core.type_text(text); }
    keyDown(code, key, ctrlOrMeta) { this.#core.key_down(code, key, ctrlOrMeta); }
    keyUp(code) { this.#core.key_up(code); }
    releaseKeys() { this.#core.release_keys(); }
    screenshot() { return this.#core.screenshot(); }
    getDisc(drive) { return this.#core.disc_image(drive); }
    saveSnapshot() { return this.#core.save_snapshot(); }

    setSpeed(value) {
        if (value !== 1) this.#core.audio_samples(); // what is queued is dropped, not played late
        this.#speed = value;
        this.#owed = 0;
    }

    /** While paused: one more frame, shown. */
    stepFrame() {
        if (this.#running) return;
        this.#safely(() => {
            this.#core.advance_frame();
            this.#core.audio_samples();
            this.#core.redraw();
        });
    }

    start() {
        if (this.#running) return;
        this.#running = true;
        this.#lastTime = undefined;
        this.#owed = 0;
        this.#stats.since = undefined;
        this.#frameRequest = this.#scheduler.request((time) => this.tick(time));
    }

    pause() {
        this.#running = false;
        this.#scheduler.cancel(this.#frameRequest);
    }

    /**
     * Sound goes to the page's audio worklet through `port` (a MessagePort): samples out, the
     * worklet's reports of its queue in.
     */
    attachAudio(port, sampleRate) {
        this.#audio?.port.close?.();
        this.#core.enable_audio(sampleRate);
        const audio = {
            port, rate: sampleRate, target: AUDIO_TARGET_S,
            // the worklet's last report, when it came, and the frames sent since
            fill: 0, arrivedAt: this.#now(), playing: false, underruns: 0, sentSince: 0
        };
        port.onmessage = (event) => this.#audioReport(audio, event.data);
        this.#audio = audio;
    }

    destroy() {
        this.pause();
        if (this.#audio) {
            this.#audio.port.onmessage = null;
            this.#audio.port.close?.();
            this.#audio = null;
        }
        this.#core.free();
    }

    // ---------------------------------------------------------------- internals

    #redrawIfPaused() {
        if (!this.#running) this.#safely(() => this.#core.redraw());
    }

    #safely(fn) {
        try { return fn(); } catch (error) {
            this.pause();
            this.#emit('error', error);
        }
    }

    /** Seconds of sound queued and not yet played, from the worklet's last report. */
    #audioBuffered() {
        const audio = this.#audio;
        if (!audio) return 0;
        const played = audio.playing ? Math.max(0, this.#now() - audio.arrivedAt) / 1000 : 0;
        return Math.max(0, (audio.fill + audio.sentSince) / audio.rate - played);
    }

    #audioReport(audio, report) {
        audio.fill = report.fill;
        audio.arrivedAt = this.#now();
        audio.playing = report.playing;
        audio.sentSince = 0;
        if (report.underruns > audio.underruns) {
            audio.underruns = report.underruns;
            audio.target = Math.min(audio.target * AUDIO_TARGET_GROWTH, AUDIO_TARGET_MAX_S);
            audio.port.postMessage({ target: audio.target });
        }
        this.#refill();
    }

    /**
     * On the sound's clock (its reports come every ~20 ms whatever the display does): emulate the
     * frames the speakers will miss. Only when the emulation is not paced by a steady 50 Hz display.
     */
    #refill() {
        const audio = this.#audio;
        if (!audio || !this.#running || this.#speed !== 1 || this.#refreshesPerFrame > 0) return;
        const frames = Math.max(0, Math.min(4, Math.ceil((audio.target - this.#audioBuffered()) * 1000 / FRAME_MS)));
        const start = this.#now();
        for (let i = 0; i < frames && (i === 0 || this.#now() - start < REALTIME_BUDGET_MS); i++) {
            this.#safely(() => this.#runFrame());
            this.#fresh = true;
        }
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
        audio.sentSince += samples.length / 2;
        audio.port.postMessage(samples, [samples.buffer]);
    }

    /** One refresh of the display, at `time` (milliseconds). */
    tick(time) {
        if (!this.#running) return;
        this.#frameRequest = this.#scheduler.request((next) => this.tick(next));
        const elapsed = this.#lastTime === undefined ? FRAME_MS : time - this.#lastTime;
        this.#lastTime = time;
        this.#measureRefresh(elapsed);
        this.#safely(() => {
            this.#emitStats(time);
            const budget = Math.min(Math.max(elapsed * BUSY_SHARE, BUSY_MIN_MS), BUSY_MAX_MS);
            if (this.#speed === Infinity) {
                // full speed: frames without picture for most of the refresh, then one shown
                const start = this.#now();
                while (this.#now() - start < budget) this.#runFrame(true);
                this.#runFrame();
                this.#core.redraw();
                return;
            }
            if (this.#speed !== 1) {
                // n times real time, by the browser's clock
                this.#owed = Math.min(this.#owed + elapsed * this.#speed, 4 * this.#speed * FRAME_MS);
                const start = this.#now();
                while (this.#owed >= 2 * FRAME_MS && this.#now() - start < budget) {
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
            if (this.#refreshesPerFrame > 0) {
                // one CPC frame every refresh of a 50 Hz display, every other one at 100 Hz...:
                // each frame on the screen as long as the others (the last one stays shown)
                if (++this.#refreshes >= this.#refreshesPerFrame) {
                    this.#refreshes = 0;
                    // the sound is not this clock's: a frame more when it runs short, one less
                    // when it runs ahead
                    const audio = this.#audio;
                    const buffered = audio ? this.#audioBuffered() : 0;
                    const frame = FRAME_MS / 1000;
                    if (!audio || buffered < audio.target + 2 * frame) {
                        if (audio && buffered < audio.target - frame) this.#runFrame(true);
                        this.#runFrame();
                    }
                    this.#core.redraw();
                }
                return;
            }
            let frames;
            if (this.#audio) {
                // the sound's clock: keep a little of it ahead of the speakers
                frames = Math.ceil((this.#audio.target - this.#audioBuffered()) * 1000 / FRAME_MS);
            } else {
                this.#owed = Math.min(this.#owed + elapsed, 4 * FRAME_MS);
                frames = Math.floor(this.#owed / FRAME_MS);
            }
            frames = Math.max(0, Math.min(frames, 4));
            const start = this.#now();
            let completed = 0;
            while (completed < frames && (completed === 0 || this.#now() - start < REALTIME_BUDGET_MS)) {
                this.#runFrame();
                completed++;
            }
            if (!this.#audio) this.#owed -= completed * FRAME_MS;
            if (completed || this.#fresh || this.#core.backend() === 'webgpu') this.#core.redraw();
            this.#fresh = false;
        });
        this.#onTick();
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
        const hertz = 1000 / median;
        this.#refreshesPerFrame = [1, 2, 3, 4].find((k) => Math.abs(hertz - 50 * k) < 1.5 * k) ?? 0;
    }
}

/**
 * The page's side of the engine when it runs in the page: calls reach it at once, its events
 * and state come back at once. The Worker's counterpart is in bemu.js (and worker.js).
 */
export class LocalTransport {
    #engine;
    #onEvent = () => {};
    #onState = () => {};
    #lastState = '';

    /** `makeEngine({ emit, onTick })` gives the engine, wired to the page. */
    constructor(makeEngine) {
        this.#engine = makeEngine({
            emit: (event, detail) => this.#onEvent(event, detail),
            onTick: () => this.#publish()
        });
    }

    set onEvent(fn) { this.#onEvent = fn; }
    set onState(fn) { this.#onState = fn; }
    get state() { return this.#engine.state(); }

    async call(method, args = []) {
        try {
            return this.#engine[method](...args);
        } finally {
            this.#publish();
        }
    }

    #publish() {
        const state = this.#engine.state();
        const key = JSON.stringify(state);
        if (key !== this.#lastState) {
            this.#lastState = key;
            this.#onState(state);
        }
    }

    dispose() { this.#engine.destroy(); }
}
