//! The window backends of the `amsrust` binary.
//!
//! A [`Frontend`] shows pictures and reports host input; it knows nothing of
//! the CPC. Its input is translated to the backend-independent
//! [`amsrust::input::HostInput`], so changing the windowing library only
//! means writing another implementation of this trait.

mod minifb_frontend;

pub use minifb_frontend::MinifbFrontend;

use amsrust::input::HostInput;

pub trait Frontend {
    /// Whether the user has not closed the window.
    fn is_open(&self) -> bool;

    /// Cap `present` (and so input polling) to `fps` calls per second;
    /// 0 for no cap.
    fn set_target_fps(&mut self, fps: usize);

    /// The host keyboard state since the previous poll.
    fn poll_input(&mut self) -> HostInput;

    /// Show `pixels`: 0RGB, `width * height` of them. The size may change
    /// from one call to the other (the window then scales the picture).
    fn present(&mut self, pixels: &[u32], size: (usize, usize)) -> Result<(), String>;
}
