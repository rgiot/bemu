// The emulation loop of bemu: the wasm core, its pacing and its sound feed. It runs in a Web
// Worker (worker.js), so the page's own work never makes the emulation or the sound stutter, or
// in the page itself when workers cannot draw (LocalTransport).

const bindingsUrl = new URL('./bemu_web.js', import.meta.url);
bindingsUrl.search = new URL(import.meta.url).search;
const { default: init, Emulator } = await import(bindingsUrl);

/** The CPC's frame: 312 lines of 64 µs (the pacer in Rust, `amsrust::pacer`, has the same figure
 * from the library's `FRAME_T_STATES`: this is for the page's own use). */
export const FRAME_MS = 312 * 64 / 1000;

let wasm;
export function loadWasm(url) {
    const defaultUrl = new URL('./bemu_web_bg.wasm', import.meta.url);
    defaultUrl.search = new URL(import.meta.url).search;
    wasm ??= init({ module_or_path: url || defaultUrl });
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
    #core; #canvas; #emit; #onTick; #scheduler;
    #frameRequest = 0;
    /** The sound feed: the worklet's port and its sample rate (the pacing is the core's, in Rust). */
    #audio = null;
    /** Times real time: 1, more, or Infinity (as fast as possible). */
    #speed = 1;

    /**
     * `core`: the wasm Emulator, which holds the pacing (`pacer_*`: which frames to run at each
     * refresh and each report of the sound). `emit(event, detail)`: what the page hears
     * ('screenshot', 'csl-done', 'stats', 'error'). `onTick()`: after each refresh. `scheduler`:
     * { request(callback), cancel(id) } for the display's refreshes. `canvas`: resized when the
     * page's size changes. `now`: the clock the pacing reads, in milliseconds.
     */
    constructor(core, { canvas, emit = () => {}, onTick = () => {}, scheduler = defaultScheduler, now = () => performance.now() } = {}) {
        this.#core = core;
        this.#canvas = canvas;
        this.#emit = emit;
        this.#onTick = onTick;
        this.#scheduler = scheduler;
        core.set_clock(now);
        core.on_screenshot((name, png) => emit('screenshot', { name, png }));
        core.on_csl_done((report) => emit('csl-done', report));
    }

    /** What the page may read at once: it is sent whenever it changes. */
    state() {
        const tape = this.#core.tape_position?.();
        return {
            backend: this.#core.backend(),
            running: this.#core.pacer_running(),
            speed: this.#speed,
            refreshesPerFrame: this.#core.pacer_refreshes_per_frame(),
            cslRunning: this.#core.csl_running?.() ?? false,
            ctm: [this.#core.ctm_width?.() ?? 0, this.#core.ctm_height?.() ?? 0],
            ctmMinimum: [this.#core.ctm_min_width?.() ?? 0, this.#core.ctm_min_height?.() ?? 0],
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
    /** 'auto': a machine too slow for real time makes no picture for some frames; 'off': every picture. */
    setFrameSkip(mode) {
        const on = mode === 'auto';
        this.#core.pacer_frame_skip(on);
        // the menus' Video > Skip frames when too slow follows
        this.#core.ui_frame_skip?.(on);
    }
    resize(width, height) {
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
    // cards on the expansion port: they stay plugged through reset and configure
    insertDandanator(bytes) { this.#core.insert_dandanator(bytes); }
    removeDandanator() { this.#core.remove_dandanator(); }
    dandanatorFlash() { return this.#core.dandanator_flash(); }
    plugNova() { this.#core.plug_nova(); }
    unplugNova() { this.#core.unplug_nova(); }
    insertAlbireo(bytes) { this.#core.insert_albireo(bytes); }
    removeAlbireo() { this.#core.remove_albireo(); }
    // the M4 board, its SD card the page's files in memory
    plugM4() { this.#core.plug_m4(); }
    unplugM4() { this.#core.unplug_m4(); }
    m4PutFile(path, bytes) { this.#core.m4_put_file(path, bytes); }
    m4GetFile(path) { return this.#core.m4_get_file(path); }
    m4List(path) { return this.#core.m4_list(path); }
    m4Remove(path) { return this.#core.m4_remove(path); }
    m4MakeDir(path) { this.#core.m4_make_dir(path); }
    // records what the CPC writes to the cassette port; stopTapeRecording gives the CDT (or undefined)
    startTapeRecording() { this.#core.start_tape_recording(); }
    stopTapeRecording() { return this.#core.stop_tape_recording(); }
    // what a DMP2000 printed: [{ name, data }], or undefined without a printer
    printout(prefix) { return this.#core.printout(prefix); }
    // an upper ROM in a slot (0-255), found by the firmware at the next reset; removeRom empties the slot
    insertRom(slot, bytes) { this.#core.insert_rom(slot, bytes); }
    removeRom(slot) { this.#core.remove_rom(slot); }
    // a CPC+ cartridge (a CPR, or a zip holding one) in; the machine restarts to boot it. Throws on a CPC.
    insertCartridge(bytes, entry) { this.#core.insert_cartridge(bytes, entry); }
    // the cartridge out; the machine restarts
    ejectCartridge() { this.#core.eject_cartridge(); }
    loadSnapshot(bytes) { this.#core.load_snapshot(bytes); }
    addFile(name, bytes) { this.#core.add_file(name, bytes); }
    runCsl(text) {
        this.#core.run_csl(text);
        this.start();
    }
    stopCsl() { this.#core.stop_csl(); }
    typeText(text) { this.#core.type_text(text); }
    keyDown(code, key, ctrlOrMeta) { this.#core.key_down(code, key, ctrlOrMeta); }
    /** A key with its modifiers: the menus take it when they have the focus. */
    keyEvent(code, key, pressed, ctrl, shift, alt, meta) {
        const taken = this.#core.key_event(code, key, pressed, ctrl, shift, alt, meta);
        if (taken) this.#redrawIfPaused();
        return taken;
    }
    /** The pointer over the canvas (CSS pixels): the menus, the palette, the debugger's windows. */
    uiPointer(kind, x, y, button) {
        const over = this.#core.ui_pointer(kind, x, y, button);
        this.#redrawIfPaused();
        return over;
    }
    uiWheel(dx, dy) {
        this.#core.ui_wheel(dx, dy);
        this.#redrawIfPaused();
    }
    uiScale(ratio) { this.#core.ui_scale(ratio); }
    /** The file the page picked for a menu command. */
    answerFile(id, name, bytes) {
        this.#core.answer_file(id, name, bytes);
        this.#redrawIfPaused();
    }
    keyUp(code) { this.#core.key_up(code); }
    releaseKeys() { this.#core.release_keys(); }
    screenshot() { return this.#core.screenshot(); }
    getDisc(drive) { return this.#core.disc_image(drive); }
    saveSnapshot() { return this.#core.save_snapshot(); }
    /** The debug API, as the desktop's HTTP server answers it (see `Emulator::api`). */
    api(method, path, query, body) { return this.#core.api(method, path, query ?? '', body ?? new Uint8Array()); }
    /** The debug events since the previous call, as `/api/events` streams them. */
    debugEvents() { return this.#core.debug_events(); }

    setSpeed(value) {
        this.#core.pacer_set_speed(value); // what is queued of the sound is dropped, not played late
        this.#speed = value;
        this.#core.ui_speed?.(value); // the menus show it
    }
    /** The page is fullscreen or not, its volume: the menus show them. */
    uiFullscreen(on) { this.#core.ui_fullscreen?.(on); }
    uiVolume(volume, muted) { this.#core.ui_volume?.(volume, muted); }

    /** While paused: one more frame, shown. */
    stepFrame() {
        if (this.#core.pacer_running()) return;
        this.#safely(() => {
            this.#core.pacer_step_frame();
            this.#uiRequests();
        });
    }

    start() {
        if (this.#core.pacer_running()) return;
        this.#core.pacer_start();
        this.#frameRequest = this.#scheduler.request((time) => this.tick(time));
    }

    pause() {
        this.#core.pacer_pause();
        this.#scheduler.cancel(this.#frameRequest);
    }

    /**
     * Sound goes to the page's audio worklet through `port` (a MessagePort): samples out, the
     * worklet's reports of its queue in.
     */
    attachAudio(port, sampleRate) {
        this.#audio?.port.close?.();
        this.#core.enable_audio(sampleRate);
        this.#core.pacer_attach_audio(sampleRate);
        const audio = { port, rate: sampleRate };
        port.onmessage = (event) => this.#audioReport(audio, event.data);
        this.#audio = audio;
    }

    destroy() {
        this.pause();
        if (this.#audio) {
            this.#audio.port.onmessage = null;
            this.#audio.port.close?.();
            this.#audio = null;
            this.#core.pacer_detach_audio();
        }
        this.#core.free();
    }

    // ---------------------------------------------------------------- internals

    #redrawIfPaused() {
        this.#safely(() => {
            this.#core.pacer_redraw_if_paused();
            if (!this.#core.pacer_running()) this.#uiRequests();
        });
    }

    /** What the menus asked: the speed and the scripts here, the rest by the page. */
    #uiRequests() {
        const requests = this.#core.take_ui_requests?.() ?? [];
        for (const request of requests) {
            switch (request.type) {
                case 'speed':
                    this.setSpeed(request.value === null ? Infinity : request.value);
                    this.#emit('ui-request', request);
                    break;
                case 'frameSkip':
                    this.setFrameSkip(request.on ? 'auto' : 'off');
                    this.#emit('ui-request', request);
                    break;
                case 'runCsl': this.runCsl(request.text); break;
                case 'stopCsl': this.stopCsl(); break;
                case 'copyScreenshot': {
                    const png = this.#core.screenshot();
                    if (png) this.#emit('ui-request', { type: 'copyImage', data: png });
                    break;
                }
                case 'screenshot': {
                    const png = this.#core.screenshot();
                    if (png) this.#emit('ui-request', { type: 'download', name: 'bemu.png', data: png });
                    break;
                }
                default: this.#emit('ui-request', request);
            }
        }
    }

    #safely(fn) {
        try { return fn(); } catch (error) {
            this.pause();
            this.#emit('error', error);
        }
    }

    /** The sound the core made, to the worklet. */
    #flushAudio() {
        if (!this.#audio) return;
        const samples = this.#core.pacer_take_outgoing();
        if (samples.length > 0) this.#audio.port.postMessage(samples, [samples.buffer]);
    }

    /** The worklet's report of its queue; the pacing, in the core, emulates what the speakers will miss. */
    #audioReport(audio, report) {
        if (!report || !Number.isFinite(report.received) || !Number.isFinite(report.fill) ||
            !Number.isFinite(report.underruns)) {
            this.pause();
            this.#emit('error', new Error('Invalid audio worklet report. Reload the page to update cached emulator files.'));
            return;
        }
        this.#safely(() => {
            const target = this.#core.pacer_audio_report(report.fill, !!report.playing, report.received, report.underruns);
            if (!Number.isNaN(target)) audio.port.postMessage({ target });
            this.#flushAudio();
        });
    }

    /** One refresh of the display, at `time` (milliseconds). */
    tick(time) {
        if (!this.#core.pacer_running()) return;
        this.#frameRequest = this.#scheduler.request((next) => this.tick(next));
        this.#safely(() => {
            const stats = this.#core.pacer_tick(time);
            this.#flushAudio();
            this.#uiRequests();
            if (stats) this.#emit('stats', stats);
        });
        this.#onTick();
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
