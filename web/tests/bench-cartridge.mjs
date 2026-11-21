// Emulation speed in wasm, in node: a CPC+ cartridge played for some emulated seconds with the
// audio and the monitor's frames, as a page has them. Not a test: it prints the speed.
//
//   cd web
//   cargo build --release --target wasm32-unknown-unknown --features bench
//   wasm-bindgen --target nodejs --out-dir /tmp/bench --out-name b \
//       target/wasm32-unknown-unknown/release/bemu_web.wasm
//   wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
//       --enable-mutable-globals /tmp/bench/b_bg.wasm -o /tmp/bench/b_bg.wasm   # as build.sh does
//   node tests/bench-cartridge.mjs /tmp/bench game.cpr [seconds] [model]
//
// The same cartridge natively: `cargo run --release --example bench_plus -- game.cpr`.
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';

const [dir, cartridge, seconds = '4', model = '6128plus'] = process.argv.slice(2);
if (!dir || !cartridge) {
    console.error('usage: node bench-cartridge.mjs <wasm-bindgen nodejs dir> <game.cpr> [seconds] [model]');
    process.exit(2);
}
const bench = createRequire(import.meta.url)(`${dir}/b.js`);
const cpr = new Uint8Array(readFileSync(cartridge));
// twice: the first run includes the engine's warm-up
for (let run = 0; run < 2; run++) {
    const start = process.hrtime.bigint();
    const frames = bench.bench_cartridge(cpr, model, Number(seconds));
    const wall = Number(process.hrtime.bigint() - start) / 1e9;
    console.log(`${seconds} s emulated in ${wall.toFixed(2)} s: ${(Number(seconds) / wall).toFixed(2)}x real time, ${frames} frames`);
}
