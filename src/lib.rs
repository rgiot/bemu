pub mod asic;
pub mod chip;

#[cfg(feature = "booster")]
pub mod booster;
pub(crate) mod calendar;
#[cfg(feature = "albireo")]
pub mod ch376;
pub mod crtc;
pub mod csl_runner;
#[cfg(feature = "dandanator")]
pub mod dandanator;
pub mod disc_image;
pub mod dmp2000;
mod dmp2000_charsets;
pub mod expansion;
pub mod expansion_port;
pub mod fdc;
pub mod ga;
pub mod hfe;
pub mod keyboard;
#[cfg(feature = "m4")]
pub mod m4;
#[cfg(all(feature = "m4-wifi", not(target_arch = "wasm32")))]
pub mod m4_net;
#[cfg(feature = "nova")]
pub mod nova;
pub mod overlay;
pub mod pal;
pub mod ppi;
pub mod psg;
pub mod roms;
pub mod shakerland;
pub mod snapshot;
#[cfg(any(feature = "playcity", feature = "digiblaster"))]
pub mod sound_cards;
pub mod sound_export;
pub mod symbols;
pub mod tape;
#[cfg(not(target_arch = "wasm32"))]
pub mod webp;
#[cfg(feature = "willy")]
pub mod willy;
pub mod z80;

pub mod memory;

pub mod monitor;

pub mod motherboard;

pub mod counter;
pub mod debug;
// an HTTP server on a socket, with threads: not in a web page
pub mod basic_program;
pub mod breakpoints;
pub mod commands;
pub mod debug_server;
pub mod display;
pub mod input;
pub mod joystick;
#[cfg(feature = "gui")]
pub mod ui;

mod bits;
mod history;
