//! The bemu Amstrad CPC emulator in a web page: the WebAssembly side of the library.
//!
//! [`Emulator`] is driven by the JavaScript module `js/bemu.js` (what a page imports): it
//! runs the machine a frame at a time and draws it in the page's canvas, takes the keys and
//! the files (DSK, SNA, CSL) the page gives it as bytes, and hands back the sound samples.
//! See `README.md`.

mod config;
mod keys;
// the browser's canvas, WebGPU and JavaScript callbacks
#[cfg(target_arch = "wasm32")]
mod emulator;
#[cfg(target_arch = "wasm32")]
mod gfx;
#[cfg(any(target_arch = "wasm32", test))]
mod scanlines;
#[cfg(target_arch = "wasm32")]
mod ui;
#[cfg(any(target_arch = "wasm32", test))]
mod viewport;

#[cfg(target_arch = "wasm32")]
pub use emulator::Emulator;

// a headless speed measure, to run the module in node (see `build.sh`'s BENCH note)
#[cfg(all(target_arch = "wasm32", feature = "bench"))]
mod bench;
