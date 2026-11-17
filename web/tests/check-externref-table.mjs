// Post-build check: the wasm module must export `__wbindgen_externrefs` as an externref table.
// If a post-processing step (wasm-opt) leaves the export pointing at the function table instead,
// the module fails at startup with "WebAssembly.Table.set(): ... function-typed object must be null".
import { readFileSync } from 'node:fs';

const path = process.argv[2] ?? new URL('../pkg/bemu_web_bg.wasm', import.meta.url);
const module = new WebAssembly.Module(readFileSync(path));
const stub = new Proxy({}, { get: () => () => {} });
const imports = Object.fromEntries(WebAssembly.Module.imports(module).map((i) => [i.module, stub]));
const instance = new WebAssembly.Instance(module, imports); // does not run __wbindgen_start
try {
  instance.exports.__wbindgen_externrefs.set(0, undefined);
} catch (e) {
  console.error(`${path}: __wbindgen_externrefs is not an externref table (${e.message})`);
  process.exit(1);
}
console.log('externref table OK');
