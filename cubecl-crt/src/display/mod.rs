//! The kernels of the application itself (`src/display/kernel.rs`), where `super::crt` is
//! `amsrust`'s.
pub use amsrust::display::crt;
#[path = "../../../src/display/kernel.rs"]
pub mod kernel;
