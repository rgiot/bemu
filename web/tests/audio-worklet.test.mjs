import assert from 'node:assert/strict';
import { test } from 'node:test';

// Load the actual worklet with the globals of an AudioWorkletGlobalScope.
const RATE = 48000;
let Processor;
globalThis.sampleRate = RATE;
globalThis.currentTime = 0;
globalThis.AudioWorkletProcessor = class {
    constructor() { this.port = { messages: [], postMessage(message) { this.messages.push(message); } }; }
};
globalThis.registerProcessor = (_name, processor) => { Processor = processor; };
await import('../js/audio-worklet.js');

function processor(target = 0.06) {
    return new Processor({ processorOptions: { target } });
}

/** `frames` stereo frames: left = `left`, right = `right` (a constant, or a function of the index). */
function stereo(frames, left, right, from = 0) {
    const samples = new Float32Array(frames * 2);
    for (let i = 0; i < frames; i++) {
        samples[2 * i] = typeof left === 'function' ? left(from + i) : left;
        samples[2 * i + 1] = typeof right === 'function' ? right(from + i) : right;
    }
    return samples;
}

function block(p) {
    const out = [new Float32Array(128), new Float32Array(128)];
    p.process([], [out]);
    return out;
}

test('nothing plays before the target is queued, then the sound starts without a step', () => {
    const p = processor(0.01); // 480 frames
    p.push(stereo(300, 0.5, 0.5));
    assert.ok(block(p)[0].every((v) => v === 0), 'still prebuffering');
    p.push(stereo(300, 0.5, 0.5));
    const [left] = block(p);
    assert.ok(Math.abs(left[0]) < 0.01, 'fades in');
    assert.ok(Math.abs(left[127] - 0.5) < 0.01, 'at full level after the fade');
});

test('the channels never swap, even when the buffer overflows', () => {
    const p = processor(0.01);
    p.push(stereo(RATE, 0.25, -0.25)); // fills the ring
    p.push(stereo(1001, 0.25, -0.25)); // odd count, overflows
    block(p); // the fade-in
    const [left, right] = block(p);
    for (let i = 0; i < 128; i++) {
        assert.ok(Math.abs(left[i] - 0.25) < 1e-6 && Math.abs(right[i] + 0.25) < 1e-6, `frame ${i}`);
    }
});

test('an underrun fades out instead of stepping, then waits for the target again', () => {
    const p = processor(0.01);
    p.push(stereo(600, 0.5, 0.5));
    let out;
    for (let i = 0; i < 6; i++) out = block(p); // 768 frames asked, 600 available
    const [left] = out;
    const biggestStep = Math.max(...Array.from(left.slice(1), (v, i) => Math.abs(v - left[i])));
    assert.ok(biggestStep < 0.02, `no click: ${biggestStep}`);
    assert.equal(left[127], 0);
    assert.equal(p.underruns, 1);
    p.push(stereo(100, 0.5, 0.5));
    assert.ok(block(p)[0].every((v) => v === 0), 'not before the target');
});

test('the playback rate follows a producer slightly slower or faster than the sound card', () => {
    for (const ratio of [0.995, 1.005]) {
        const p = processor(0.06);
        const target = Math.round(0.06 * RATE);
        let produced = 0;
        let owed = 0;
        p.push(stereo(target, 0, 0));
        produced += target;
        for (let n = 0; n < 5 * RATE / 128; n++) { // five seconds
            owed += 128 * ratio;
            if (owed >= 960) { // a 50 Hz frame
                p.push(stereo(960, 0, 0));
                produced += 960;
                owed -= 960;
            }
            block(p);
        }
        assert.equal(p.underruns, 0, `ratio ${ratio}`);
        const fill = p.written - p.position;
        assert.ok(fill > target * 0.4 && fill < target * 2, `ratio ${ratio}: fill ${fill}`);
    }
});

test('it reports its fill level and follows a new target', () => {
    const p = processor(0.01);
    p.push(stereo(2000, 0, 0));
    for (let i = 0; i < 8; i++) block(p);
    const [report] = p.port.messages;
    assert.equal(report.playing, true);
    assert.ok(report.fill > 900 && report.fill < 1000, `${report.fill}`);
    p.receive({ target: 0.1 });
    assert.equal(p.target, 4800);
});

test('the emulator\'s own channel carries the samples in and the reports out', () => {
    const p = processor(0.01);
    const feed = { messages: [], postMessage(message) { this.messages.push(message); } };
    p.receive({ feed });
    feed.onmessage({ data: stereo(2000, 0, 0) });
    for (let i = 0; i < 8; i++) block(p);
    assert.equal(feed.messages.length, 1);
    assert.equal(feed.messages[0].playing, true);
    assert.equal(p.port.messages.length, 0);
});
