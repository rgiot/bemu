pub mod chip;

pub mod crtc;
pub mod csl_runner;
pub mod disc_image;
pub mod dmp2000;
mod dmp2000_charsets;
pub mod expansion;
pub mod fdc;
pub mod hfe;
pub mod ga;
pub mod keyboard;
pub mod pal;
pub mod ppi;
pub mod psg;
pub mod shakerland;
pub mod overlay;
pub mod snapshot;
pub mod expansion_port;
#[cfg(any(feature = "playcity", feature = "digiblaster"))]
pub mod sound_cards;
#[cfg(feature = "albireo")]
pub mod ch376;
#[cfg(feature = "m4")]
pub mod m4;
#[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
pub mod m4_net;
#[cfg(feature = "nova")]
pub mod nova;
pub(crate) mod calendar;
pub mod roms;
#[cfg(feature = "willy")]
pub mod willy;
#[cfg(feature = "dandanator")]
pub mod dandanator;
#[cfg(feature = "booster")]
pub mod booster;
pub mod symbols;
pub mod sound_export;
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
