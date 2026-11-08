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

    /// Whether [`HostInput::placed`] gives the physical keys (else the positional mapping
    /// takes the keys the host's layout names: positions only on a QWERTY host).
    fn knows_key_places(&self) -> bool {
        false
    }

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

    /// The input of the user interface since the last call (`None`: this frontend has none).
    #[cfg(feature = "gui")]
    fn ui_input(&mut self) -> Option<egui::RawInput> {
        None
    }

    /// What the user interface draws over the next picture, its menu bar `menu_height` points
    /// high.
    #[cfg(feature = "gui")]
    fn set_ui_output(&mut self, output: egui::FullOutput, menu_height: f32) {
        let _ = (output, menu_height);
    }

    /// Whether this frontend opens the debugger's windows as windows of the system.
    #[cfg(feature = "gui")]
    fn has_view_windows(&self) -> bool {
        false
    }

    /// Open a window of the system for each of `views` (closing the others); returns the
    /// views whose window the user closed.
    #[cfg(feature = "gui")]
    fn sync_view_windows(&mut self, views: &[amsrust::ui::NativeView]) -> Vec<u8> {
        let _ = views;
        Vec::new()
    }

    /// The input of the window of view `key` since the last call (`None`: not open yet).
    #[cfg(feature = "gui")]
    fn view_input(&mut self, key: u8) -> Option<egui::RawInput> {
        let _ = key;
        None
    }

    /// Draw the window of view `key`, sized to fit `wanted` points.
    #[cfg(feature = "gui")]
    fn set_view_output(&mut self, key: u8, output: egui::FullOutput, wanted: egui::Vec2) {
        let _ = (key, output, wanted);
    }

    /// The text of the host's clipboard.
    fn clipboard_text(&mut self) -> Option<String> {
        None
    }

    /// Whether the emulation runs at another speed than the real one (the clock paces it then,
    /// not a 50 Hz display's vertical sync).
    fn set_speed_factor(&mut self, on: bool) {
        let _ = on;
    }

    /// How curved the CRTs' glass is (see `DisplayMode::warp`), for a frontend that draws them.
    fn set_curvature(&mut self, curvature: f32) {
        let _ = curvature;
    }

    /// Go fullscreen, or back to the window.
    fn set_fullscreen(&mut self, on: bool) {
        let _ = on;
    }

    /// Whether the window is fullscreen (F11 also switches it).
    fn fullscreen(&self) -> bool {
        false
    }
}
