//! The window backends of the `amsrust` binary.
//!
//! A [`Frontend`] shows pictures and reports host input; it knows nothing of
//! the CPC. Its input is translated to the backend-independent
//! [`amsrust::input::HostInput`], so changing the windowing library only
//! means writing another implementation of this trait.

mod minifb_frontend;
#[cfg(feature = "gpu-window")]
mod winit_gpu;

pub use minifb_frontend::MinifbFrontend;
#[cfg(feature = "gpu-window")]
pub use winit_gpu::WinitGpuFrontend;

use amsrust::display::DisplayMode;
use amsrust::input::HostInput;
use image::RgbImage;

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

    /// Whether this frontend filters and shows the monitor's frames itself
    /// ([`Self::present_frame`], on the GPU, with no read-back) rather than
    /// showing the pixels it is given ([`Self::present`]).
    fn direct(&self) -> bool {
        false
    }

    /// Filter (as `mode` says) and show the monitor's `frame`.
    fn present_frame(&mut self, _frame: &RgbImage, _mode: DisplayMode) -> Result<(), String> {
        Err("this frontend does not draw the monitor's frames itself".to_owned())
    }

    /// Show the last picture again (the emulation is paused).
    fn redraw(&mut self) -> Result<(), String> {
        Ok(())
    }
}
