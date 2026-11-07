import assert from 'node:assert/strict';
import { copyFile, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { pathToFileURL } from 'node:url';

// Load the actual wrapper with a stub for its generated wasm bindings.
const dir = await mkdtemp(join(tmpdir(), 'bemu-test-'));
try {
    for (const file of ['bemu.js', 'engine.js', 'worker.js']) await copyFile(new URL(`../js/${file}`, import.meta.url), join(dir, file));
    await writeFile(join(dir, 'package.json'), '{"type":"module"}');
    await writeFile(join(dir, 'bemu_web.js'), 'export default async function init(options) { globalThis.testWasmOptions = options; } export class Emulator { static create(...args) { return globalThis.testCreateCore(...args); } }');
    const { Bemu, WorkerTransport } = await import(pathToFileURL(join(dir, 'bemu.js')));
    const { Engine, LocalTransport } = await import(pathToFileURL(join(dir, 'engine.js')));

    function machine(advance = () => {}) {
        const canvas = { addEventListener() {}, removeEventListener() {} };
        const core = {
            on_screenshot() {}, on_csl_done() {}, backend: () => 'canvas2d',
            advance_frame: advance, redraw() {}, audio_samples: () => [], free() {}
        };
        const transport = new LocalTransport(({ emit, onTick }) => new Engine(core, { canvas, emit, onTick }));
        return { emu: new Bemu(canvas, transport, {}), core };
    }

    // The engine alone, with a clock and display refreshes the test drives.
    function engineWith(advance) {
        const core = {
            on_screenshot() {}, on_csl_done() {}, backend: () => 'canvas2d',
            advance_frame: advance, redraw() {}, audio_samples: () => [], free() {}
        };
        const clock = { now: 0, scheduled: null };
        const scheduler = { request: (fn) => { clock.scheduled = fn; return 1; }, cancel() {} };
        const engine = new Engine(core, { scheduler, now: () => clock.now });
        return { engine, core, clock };
    }

    await test('page-driven worker refreshes do not queue while a frame is in flight', () => {
        const messages = [];
        const worker = { postMessage: (message) => messages.push(message) };
        const transport = new WorkerTransport(worker);
        transport.postFrame(10);
        transport.postFrame(20);
        transport.postFrame(30);
        assert.deepEqual(messages, [{ type: 'frame', time: 10 }]);
        worker.onmessage({ data: { type: 'frame-done' } });
        transport.postFrame(40);
        assert.deepEqual(messages.at(-1), { type: 'frame', time: 40 });
        assert.equal(messages.length, 2);
    });

    await test('worker acknowledges page refreshes even when paused', async () => {
        const previousSelf = globalThis.self;
        const previousCreate = globalThis.testCreateCore;
        const messages = [];
        let frames = 0;
        globalThis.self = { navigator: { userAgent: 'Firefox/123' }, postMessage: (message) => messages.push(message) };
        globalThis.testCreateCore = () => ({
            on_screenshot() {}, on_csl_done() {}, backend: () => 'canvas2d',
            advance_frame() { frames++; }, redraw() {}, audio_samples: () => [], free() {}
        });
        try {
            await import(pathToFileURL(join(dir, 'worker.js')));
            await self.onmessage({ data: { type: 'init', id: 1, canvas: {}, config: {}, webgpu: false } });
            await self.onmessage({ data: { type: 'call', id: 2, method: 'start', args: [] } });
            await self.onmessage({ data: { type: 'frame', time: 0 } });
            assert.equal(frames, 1);
            assert.equal(messages.at(-1).type, 'frame-done');
            await self.onmessage({ data: { type: 'call', id: 3, method: 'pause', args: [] } });
            await self.onmessage({ data: { type: 'frame', time: 20 } });
            assert.equal(frames, 1);
            assert.equal(messages.at(-1).type, 'frame-done');
        } finally {
            globalThis.self = previousSelf;
            globalThis.testCreateCore = previousCreate;
        }
    });

    await test('a stale worklet report does not cause frames already in transit to be generated again', () => {
        let frames = 0;
        const { engine, core, clock } = engineWith(() => { frames++; });
        core.enable_audio = () => {};
        core.audio_samples = () => new Float32Array(960 * 2);
        const port = { postMessage() {}, close() {} };
        engine.attachAudio(port, 48000);
        engine.start();
        clock.scheduled(0);
        assert.equal(frames, 4);
        port.onmessage({ data: { fill: 0, received: 0, playing: false, underruns: 0 } });
        assert.equal(frames, 4, 'audio already sent but not reported must remain accounted for');
        port.onmessage({ data: { fill: 1000, received: 3840, playing: true, underruns: 0 } });
        assert.equal(frames, 6, 'a report that acknowledges drained samples triggers a refill');
        engine.destroy();
    });

    await test('audio reports refill even when a 50 Hz display misses a refresh', () => {
        let frames = 0;
        const { engine, core, clock } = engineWith(() => { frames++; });
        core.enable_audio = () => {};
        core.audio_samples = () => new Float32Array(960 * 2);
        const port = { postMessage() {}, close() {} };
        engine.attachAudio(port, 48000);
        engine.start();
        for (let i = 0; i < 35; i++) {
            clock.now = i * 20;
            clock.scheduled(clock.now);
        }
        assert.equal(engine.state().refreshesPerFrame, 1);
        const before = frames;
        port.onmessage({ data: { fill: 0, received: frames * 960, playing: true, underruns: 0 } });
        assert.ok(frames > before, 'audio must refill without waiting for another refresh');
        engine.destroy();
    });

    await test('Canvas 2D redraws audio-driven frames even when the display tick needs no new frame', () => {
        let frames = 0;
        let draws = 0;
        const { engine, core, clock } = engineWith(() => { frames++; });
        core.enable_audio = () => {};
        core.audio_samples = () => new Float32Array(960 * 2);
        core.redraw = () => { draws++; };
        const port = { postMessage() {}, close() {} };
        engine.attachAudio(port, 48000);
        engine.start();
        clock.scheduled(0);
        const firstDraws = draws;
        port.onmessage({ data: { fill: 0, received: frames * 960, playing: true, underruns: 0 } });
        const refilledFrames = frames;
        assert.ok(refilledFrames > 4);
        clock.scheduled(1);
        assert.equal(frames, refilledFrames);
        assert.equal(draws, firstDraws + 1);
        engine.destroy();
    });

    await test('Canvas 2D resizes its physical backing store', () => {
        const { engine, core } = engineWith(() => {});
        const canvas = { width: 10, height: 10 };
        let resized;
        core.resize = (width, height) => { resized = [width, height]; };
        const sized = new Engine(core, { canvas, scheduler: { request: () => 1, cancel() {} } });
        sized.resize(1100, 800);
        assert.deepEqual([canvas.width, canvas.height], [1100, 800]);
        assert.deepEqual(resized, [1100, 800]);
        sized.destroy();
        engine.destroy();
    });

    await test('an old or invalid worklet report stops with a cache error', () => {
        const { engine, core } = engineWith(() => {});
        core.enable_audio = () => {};
        const port = { postMessage() {}, close() {} };
        const errors = [];
        const failing = new Engine(core, {
            scheduler: { request: () => 1, cancel() {} },
            emit: (name, value) => { if (name === 'error') errors.push(value); }
        });
        failing.attachAudio(port, 48000);
        failing.start();
        port.onmessage({ data: { fill: 0, playing: false, underruns: 0 } });
        assert.equal(failing.state().running, false);
        assert.match(errors[0].message, /cached emulator files/);
        failing.destroy();
        engine.destroy();
    });

    await test('module version is propagated to wasm', async () => {
        const url = pathToFileURL(join(dir, 'engine.js'));
        url.search = '?v=cache-test';
        const { loadWasm } = await import(url);
        await loadWasm();
        assert.equal(globalThis.testWasmOptions.module_or_path.search, '?v=cache-test');
        delete globalThis.testWasmOptions;
    });

    await test('normal-speed catch-up yields after a slow frame and retains the debt', () => {
        let frames = 0;
        const { engine, clock } = engineWith(() => { frames++; clock.now += 30; });
        engine.start();
        clock.scheduled(0);
        assert.equal(frames, 1);
        clock.scheduled(100);
        assert.equal(frames, 2, 'a slow refresh must not execute four frames');
        clock.scheduled(101);
        assert.equal(frames, 3, 'unexecuted frames remain owed');
        engine.destroy();
    });

    await test('fast catch-up can run several frames and does not redraw an unchanged Canvas 2D frame', () => {
        let frames = 0;
        let draws = 0;
        const { engine, core, clock } = engineWith(() => { frames++; clock.now++; });
        core.redraw = () => { draws++; };
        engine.start();
        clock.scheduled(0);
        clock.scheduled(1);
        assert.equal(draws, 1);
        clock.scheduled(100);
        assert.equal(frames, 5);
        assert.equal(draws, 2);
        engine.destroy();
    });

    await test('GPU skips unchanged frames but redraws display changes and resizes', () => {
        let draws = 0;
        const { engine, core, clock } = engineWith(() => {});
        core.backend = () => 'webgpu';
        core.redraw = () => { draws++; };
        core.set_display = () => {};
        core.resize = () => {};
        engine.start();
        clock.scheduled(0);
        clock.scheduled(1);
        assert.equal(draws, 1);
        engine.setDisplay('ctm');
        clock.scheduled(2);
        assert.equal(draws, 2);
        engine.resize(1440, 1088);
        clock.scheduled(3);
        assert.equal(draws, 3);
        engine.pause();
        engine.setDisplay('raw');
        assert.equal(draws, 4);
        engine.destroy();
    });

    await test('stats report measured frame and draw costs', () => {
        const clock = { now: 0, scheduled: null };
        const scheduler = { request: (fn) => { clock.scheduled = fn; return 1; }, cancel() {} };
        const core = {
            on_screenshot() {}, on_csl_done() {}, backend: () => 'webgpu', free() {},
            advance_frame() { clock.now += 3; }, redraw() { clock.now += 2; }
        };
        const events = [];
        const engine = new Engine(core, { scheduler, now: () => clock.now, emit: (name, value) => events.push([name, value]) });
        engine.start();
        clock.scheduled(0);
        clock.scheduled(1000);
        const stats = events.find(([name]) => name === 'stats')[1];
        assert.deepEqual(stats.performance, { frameMs: 3, maxFrameMs: 3, drawMs: 2, draws: 1 });
        engine.destroy();
    });

    await test('unavailable AudioWorklet reports a useful error, closes, and can retry', async () => {
        const saved = globalThis.AudioContext;
        const cancel = globalThis.cancelAnimationFrame;
        const warn = console.warn;
        let closed = 0;
        let warnings = 0;
        globalThis.cancelAnimationFrame = () => {};
        console.warn = () => { warnings++; };
        globalThis.AudioContext = class {
            state = 'running';
            async close() { this.state = 'closed'; closed++; }
        };
        const { emu } = machine();
        const errors = [];
        emu.on('audio-error', (error) => errors.push(error));
        try {
            await emu.enableAudio();
            await emu.enableAudio();
            assert.equal(closed, 2);
            assert.equal(errors.length, 2);
            assert.equal(warnings, 2);
            assert.match(errors[0].message, /HTTPS or use localhost/);
        } finally {
            emu.destroy();
            globalThis.AudioContext = saved;
            globalThis.cancelAnimationFrame = cancel;
            console.warn = warn;
        }
    });

    await test('destroying during audio startup closes the context without using the freed core', async () => {
        const saved = globalThis.AudioContext;
        const cancel = globalThis.cancelAnimationFrame;
        let finish;
        let closed = 0;
        globalThis.cancelAnimationFrame = () => {};
        globalThis.AudioContext = class {
            audioWorklet = { addModule: () => new Promise((resolve) => { finish = resolve; }) };
            async close() { closed++; }
        };
        const { emu } = machine();
        try {
            const starting = emu.enableAudio();
            emu.destroy();
            finish();
            await starting;
            assert.equal(closed, 1);
        } finally {
            globalThis.AudioContext = saved;
            globalThis.cancelAnimationFrame = cancel;
        }
    });

    await test('a suspended gesture-created context does not block emulation before it resumes', async () => {
        const savedNode = globalThis.AudioWorkletNode;
        const savedGain = globalThis.GainNode;
        const savedChannel = globalThis.MessageChannel;
        const cancel = globalThis.cancelAnimationFrame;
        let statechange;
        let attached = false;
        let resumed = 0;
        let closed = 0;
        globalThis.cancelAnimationFrame = () => {};
        globalThis.AudioWorkletNode = class {
            port = { postMessage() {} };
            connect(gain) { return gain; }
        };
        globalThis.GainNode = class { connect() {} };
        globalThis.MessageChannel = class { port1 = {}; port2 = { close() {} }; };
        const context = {
            state: 'suspended', sampleRate: 48000, destination: {},
            audioWorklet: { async addModule() {} },
            addEventListener(type, fn) { if (type === 'statechange') statechange = fn; },
            removeEventListener() {},
            async suspend() { this.state = 'suspended'; },
            async resume() { resumed++; },
            async close() { closed++; this.state = 'closed'; }
        };
        const canvas = { width: 768, height: 544, addEventListener() {}, removeEventListener() {} };
        const transport = {
            state: { backend: 'canvas2d', ctm: [0, 0] },
            call(method) {
                if (method === 'attachAudio') {
                    attached = true;
                    return Promise.resolve();
                }
                return Promise.resolve();
            },
            dispose() {}
        };
        const emu = new Bemu(canvas, transport, { audioContext: context });
        try {
            const blocked = [];
            emu.on('audio-blocked', () => blocked.push(true));
            await emu.enableAudio();
            assert.equal(context.state, 'suspended');
            assert.equal(blocked.length, 1);
            assert.equal(attached, false);
            emu.start();
            assert.equal(resumed, 1);
            assert.equal(attached, false);
            await emu.enableAudio();
            assert.equal(resumed, 2);
            context.state = 'running';
            statechange();
            await Promise.resolve();
            assert.equal(attached, true);
            assert.equal(context.state, 'running');
        } finally {
            emu.destroy();
            assert.equal(closed, 1);
            globalThis.AudioWorkletNode = savedNode;
            globalThis.GainNode = savedGain;
            globalThis.MessageChannel = savedChannel;
            globalThis.cancelAnimationFrame = cancel;
        }
    });

    await test('keeps a gesture-created running audio context active during startup', async () => {
        const savedNode = globalThis.AudioWorkletNode;
        const savedGain = globalThis.GainNode;
        const savedChannel = globalThis.MessageChannel;
        const cancel = globalThis.cancelAnimationFrame;
        globalThis.cancelAnimationFrame = () => {};
        globalThis.AudioWorkletNode = class { port = { postMessage() {} }; connect(gain) { return gain; } };
        globalThis.GainNode = class { connect() {} };
        globalThis.MessageChannel = class { port1 = {}; port2 = { close() {} }; };
        let suspended = 0;
        let attached = 0;
        const context = {
            state: 'running', sampleRate: 48000, destination: {},
            audioWorklet: { async addModule() {} },
            addEventListener() {}, removeEventListener() {},
            async suspend() { suspended++; this.state = 'suspended'; },
            async close() { this.state = 'closed'; }
        };
        const canvas = { width: 768, height: 544, addEventListener() {}, removeEventListener() {} };
        const transport = {
            state: { backend: 'canvas2d', ctm: [0, 0] },
            call(method) { if (method === 'attachAudio') attached++; return Promise.resolve(); },
            dispose() {}
        };
        const emu = new Bemu(canvas, transport, { audioContext: context });
        try {
            await emu.enableAudio();
            assert.equal(suspended, 0);
            assert.equal(attached, 1);
        } finally {
            emu.destroy();
            globalThis.AudioWorkletNode = savedNode;
            globalThis.GainNode = savedGain;
            globalThis.MessageChannel = savedChannel;
            globalThis.cancelAnimationFrame = cancel;
        }
    });

    await test('CTM warning follows physical canvas size and display changes', () => {
        const savedWindow = globalThis.window;
        const savedObserver = globalThis.ResizeObserver;
        const cancel = globalThis.cancelAnimationFrame;
        let resize;
        let windowResize;
        let ratioChange;
        let ratioRemovals = 0;
        globalThis.window = {
            devicePixelRatio: 1,
            addEventListener(type, fn) { if (type === 'resize') windowResize = fn; },
            removeEventListener() {},
            matchMedia() {
                return {
                    addEventListener(type, fn) { ratioChange = fn; },
                    removeEventListener() { ratioRemovals++; }
                };
            }
        };
        globalThis.ResizeObserver = class {
            constructor(fn) { resize = fn; }
            observe() {}
            disconnect() {}
        };
        globalThis.cancelAnimationFrame = () => {};
        const canvas = {
            width: 720, height: 544, clientWidth: 720, clientHeight: 544,
            addEventListener() {}, removeEventListener() {}
        };
        let display = 'ctm';
        const core = {
            on_screenshot() {}, on_csl_done() {}, backend: () => 'webgpu',
            ctm_min_width: () => 1440, ctm_min_height: () => 1088,
            ctm_width: () => display === 'ctm' ? 1440 : 0,
            ctm_height: () => display === 'ctm' ? 1088 : 0,
            set_display: (value) => { display = value; },
            configure: (config) => { display = config.display ?? 'raw'; },
            resize() {}, redraw() {}, free() {}
        };
        const transport = new LocalTransport(({ emit, onTick }) => new Engine(core, { canvas, emit, onTick }));
        const emu = new Bemu(canvas, transport, {});
        const warnings = [];
        const sizes = [];
        emu.on('display-warning', (warning) => warnings.push(warning));
        emu.on('display-size', (size) => sizes.push(size));
        try {
            assert.deepEqual(emu.ctmMinimumSize, { width: 1440, height: 1088 });
            assert.deepEqual(emu.displaySize, { width: 720, height: 544 });
            assert.deepEqual(emu.displayWarning, { width: 720, height: 544, requiredWidth: 1440, requiredHeight: 1088 });
            resize([{ devicePixelContentBoxSize: [{ inlineSize: 721, blockSize: 545 }] }]);
            assert.deepEqual(sizes.at(-1), { width: 721, height: 545 });
            assert.equal(emu.displayWarning.width, 721);
            globalThis.window.devicePixelRatio = 2;
            ratioChange();
            assert.equal(emu.displayWarning, null);
            assert.equal(warnings.at(-1), null);
            assert.equal(ratioRemovals, 1);
            windowResize();
            canvas.clientHeight = 543;
            resize();
            assert.equal(emu.displayWarning.height, 1086);
            emu.setDisplay('raw');
            assert.equal(emu.displayWarning, null);
            assert.deepEqual(emu.ctmMinimumSize, { width: 1440, height: 1088 });
            emu.configure({ display: 'ctm' });
            assert.equal(emu.displayWarning.requiredWidth, 1440);
            emu.configure({});
            assert.equal(emu.displayWarning, null);
        } finally {
            emu.destroy();
            globalThis.window = savedWindow;
            globalThis.ResizeObserver = savedObserver;
            globalThis.cancelAnimationFrame = cancel;
        }
    });
} finally {
    await rm(dir, { recursive: true, force: true });
}
