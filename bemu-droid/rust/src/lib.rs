//! bemu-droid: the bemu Amstrad CPC emulator on Android, its native side.
//!
//! Everything of the emulator stays in Rust (the machine, the CTM's CRT on the GPU, the sound,
//! the media and their autorun); the app (Kotlin, `org.bemu.droid`) shows the surface and the
//! touch controls, and calls the functions of [`jni_api`] through its `Native` object.

pub mod config;
pub mod governor;
pub mod machine;

#[cfg(target_os = "android")]
mod audio;
#[cfg(target_os = "android")]
mod gfx;
#[cfg(target_os = "android")]
mod jni_api;
#[cfg(target_os = "android")]
mod runner;
