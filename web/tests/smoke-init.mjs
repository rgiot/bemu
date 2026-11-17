// CI smoke test: the built package must initialise its WebAssembly module (as a browser would).
import { readFileSync } from 'node:fs';
import init from '../pkg/bemu_web.js';

const wasm = readFileSync(new URL('../pkg/bemu_web_bg.wasm', import.meta.url));
await init({ module_or_path: wasm });
console.log('wasm init OK');
