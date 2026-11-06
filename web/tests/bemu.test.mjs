import assert from 'node:assert/strict';
import { copyFile, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { pathToFileURL } from 'node:url';

// Load the actual wrapper with a stub for its generated wasm bindings.
const dir = await mkdtemp(join(tmpdir(), 'bemu-test-'));
try {
    await copyFile(new URL('../js/bemu.js', import.meta.url), join(dir, 'bemu.js'));
    await writeFile(join(dir, 'package.json'), '{"type":"module"}');
    await writeFile(join(dir, 'bemu_web.js'), 'export default async function init() {} export class Emulator {}');
    const { Bemu } = await import(pathToFileURL(join(dir, 'bemu.js')));

    function machine(advance = () => {}) {
        const canvas = { addEventListener() {}, removeEventListener() {} };
        const core = {
            on_screenshot() {}, on_csl_done() {}, backend: () => 'canvas2d',
            advance_frame: advance, redraw() {}, audio_samples: () => [], free() {}
        };
        return { emu: new Bemu(canvas, core, {}), core };
    }

    await test('normal-speed catch-up yields after a slow frame and retains the debt', () => {
        const saved = {
            performance: Object.getOwnPropertyDescriptor(globalThis, 'performance'),
            requestAnimationFrame: globalThis.requestAnimationFrame,
            cancelAnimationFrame: globalThis.cancelAnimationFrame
        };
        let now = 0;
        let scheduled;
        let frames = 0;
        Object.defineProperty(globalThis, 'performance', { configurable: true, value: { now: () => now } });
        globalThis.requestAnimationFrame = (fn) => { scheduled = fn; return 1; };
        globalThis.cancelAnimationFrame = () => {};
        const { emu } = machine(() => { frames++; now += 30; });
        try {
            emu.start();
            scheduled(0);
            assert.equal(frames, 1);
            scheduled(100);
            assert.equal(frames, 2, 'a slow refresh must not execute four frames');
            scheduled(101);
            assert.equal(frames, 3, 'unexecuted frames remain owed');
        } finally {
            emu.destroy();
            Object.defineProperty(globalThis, 'performance', saved.performance);
            globalThis.requestAnimationFrame = saved.requestAnimationFrame;
            globalThis.cancelAnimationFrame = saved.cancelAnimationFrame;
        }
    });

    await test('fast catch-up can run several frames and does not redraw an unchanged Canvas 2D frame', () => {
        const saved = {
            performance: Object.getOwnPropertyDescriptor(globalThis, 'performance'),
            requestAnimationFrame: globalThis.requestAnimationFrame,
            cancelAnimationFrame: globalThis.cancelAnimationFrame
        };
        let now = 0;
        let scheduled;
        let frames = 0;
        let draws = 0;
        Object.defineProperty(globalThis, 'performance', { configurable: true, value: { now: () => now } });
        globalThis.requestAnimationFrame = (fn) => { scheduled = fn; return 1; };
        globalThis.cancelAnimationFrame = () => {};
        const { emu, core } = machine(() => { frames++; now++; });
        core.redraw = () => { draws++; };
        try {
            emu.start();
            scheduled(0);
            scheduled(1);
            assert.equal(draws, 1);
            scheduled(100);
            assert.equal(frames, 5);
            assert.equal(draws, 2);
        } finally {
            emu.destroy();
            Object.defineProperty(globalThis, 'performance', saved.performance);
            globalThis.requestAnimationFrame = saved.requestAnimationFrame;
            globalThis.cancelAnimationFrame = saved.cancelAnimationFrame;
        }
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
            ctm_width: () => display === 'ctm' ? 1440 : 0,
            ctm_height: () => display === 'ctm' ? 1088 : 0,
            set_display: (value) => { display = value; },
            configure: (config) => { display = config.display ?? 'raw'; },
            resize() {}, redraw() {}, free() {}
        };
        const emu = new Bemu(canvas, core, {});
        const warnings = [];
        emu.on('display-warning', (warning) => warnings.push(warning));
        try {
            assert.deepEqual(emu.displayWarning, { width: 720, height: 544, requiredWidth: 1440, requiredHeight: 1088 });
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
