// The emulator's Web Worker: it owns the wasm core, draws in the OffscreenCanvas the page handed
// over and feeds the audio worklet directly, so a busy page cannot stall them. The page's side is
// WorkerTransport (bemu.js).

const engineUrl = new URL('./engine.js', import.meta.url);
engineUrl.search = new URL(import.meta.url).search;
const engineModule = import(engineUrl);

let engine;
let pendingFrame = null;
let lastState = '';

// Firefox's worker requestAnimationFrame is starved: its pictures and inputs come late.
const hasFrames = typeof self.requestAnimationFrame === 'function' && !/\bFirefox\//.test(self.navigator?.userAgent ?? '');
// Without requestAnimationFrame in workers, the page sends the display's refreshes.
const scheduler = hasFrames
    ? { request: (callback) => self.requestAnimationFrame(callback), cancel: (id) => self.cancelAnimationFrame(id) }
    : { request: (callback) => { pendingFrame = callback; return 1; }, cancel: () => { pendingFrame = null; } };

function publishState() {
    const state = engine.state();
    const key = JSON.stringify(state);
    if (key !== lastState) {
        lastState = key;
        self.postMessage({ type: 'state', state });
    }
}

function transferablesOf(value) {
    return ArrayBuffer.isView(value) ? [value.buffer] : [];
}

async function init({ canvas, config, webgpu, wasmUrl }) {
    const { createCore, Engine } = await engineModule;
    const core = await createCore(canvas, config, webgpu, wasmUrl);
    engine = new Engine(core, {
        canvas,
        scheduler,
        emit: (event, detail) => {
            const message = event === 'error' ? { message: detail?.message ?? String(detail) } : detail;
            self.postMessage({ type: 'event', event, detail: message }, transferablesOf(message?.png));
        },
        onTick: publishState
    });
    publishState();
    return { hasFrames };
}

self.onmessage = async ({ data }) => {
    const { type, id } = data;
    if (type === 'frame') {
        const callback = pendingFrame;
        pendingFrame = null;
        try {
            callback?.(data.time);
        } finally {
            self.postMessage({ type: 'frame-done' });
        }
        return;
    }
    try {
        let value;
        if (type === 'init') value = await init(data);
        else if (type === 'call') value = engine[data.method](...data.args);
        else if (type === 'dispose') {
            engine?.destroy();
            self.close();
            return;
        }
        if (type === 'call') publishState();
        self.postMessage({ type: 'result', id, value }, transferablesOf(value));
    } catch (error) {
        self.postMessage({ type: 'failure', id, message: error?.message ?? String(error) });
    }
};
