pub mod chip;

pub mod crtc;
pub mod csl_runner;
pub mod disc_image;
pub mod expansion;
pub mod fdc;
pub mod ga;
pub mod keyboard;
pub mod pal;
pub mod ppi;
pub mod psg;
pub mod shakerland;
pub mod overlay;
pub mod snapshot;
pub mod tape;
pub mod z80;

pub mod memory;

pub mod monitor;

pub mod motherboard;

pub mod counter;
pub mod debug;
// an HTTP server on a socket, with threads: not in a web page
#[cfg(not(target_arch = "wasm32"))]
pub mod debug_server;
pub mod display;
pub mod input;
pub mod breakpoints;
pub mod basic_program;

mod bits;
mod history;
