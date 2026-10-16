#!/bin/sh
# Build the web library in web/pkg: the wasm module, its bindings, bemu.js and the audio worklet.
# Needs the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown) and the
# wasm-bindgen CLI of the same version as the wasm-bindgen crate in Cargo.lock
# (cargo install wasm-bindgen-cli --version <it>).
set -e
cd "$(dirname "$0")"
target_dir="${CARGO_TARGET_DIR:-target}"
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg --out-name bemu_web \
    "$target_dir/wasm32-unknown-unknown/release/bemu_web.wasm"
# Binaryen's wasm-opt (WASM_OPT, else on the PATH), if there: about 25% smaller and 10% faster
wasm_opt="${WASM_OPT:-$(command -v wasm-opt || true)}"
if [ -n "$wasm_opt" ]; then
    "$wasm_opt" -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
        --enable-mutable-globals pkg/bemu_web_bg.wasm -o pkg/bemu_web_bg.wasm
fi
cp js/bemu.js js/audio-worklet.js pkg/
cat > pkg/package.json <<JSON
{
  "name": "bemu",
  "version": "$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)",
  "description": "The bemu Amstrad CPC emulator, to embed in a web page",
  "type": "module",
  "module": "bemu.js",
  "main": "bemu.js",
  "files": ["bemu.js", "audio-worklet.js", "bemu_web.js", "bemu_web_bg.wasm", "bemu_web.d.ts"]
}
JSON
echo "web/pkg is ready"
