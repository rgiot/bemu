//! [`Frontend`](super::Frontend) on minifb.

use super::Frontend;
use amsrust::input::{HostInput, HostKey};
use minifb::{InputCallback, Key, Window, WindowOptions};
use std::cell::RefCell;
use std::rc::Rc;

/// Characters typed, as resolved by the OS under the active layout and
/// modifiers (`Xutf8LookupString` on X11). A `Clone` handle sharing the same
/// queue, since minifb needs to own a boxed callback while the frontend also
/// drains it.
#[derive(Default, Clone)]
struct CharQueue(Rc<RefCell<Vec<char>>>);

impl InputCallback for CharQueue {
    fn add_char(&mut self, uni_char: u32) {
        // Code points that are not valid characters should not happen.
        if let Some(c) = char::from_u32(uni_char) {
            self.0.borrow_mut().push(c);
        }
    }
}

pub struct MinifbFrontend {
    window: Window,
    typed: CharQueue
}

impl MinifbFrontend {
    pub fn open(title: &str, width: usize, height: usize) -> Result<Self, String> {
        let mut window = Window::new(title, width, height, WindowOptions { resize: true, ..WindowOptions::default() })
            .map_err(|e| format!("failed to open a window: {e}"))?;
        let typed = CharQueue::default();
        window.set_input_callback(Box::new(typed.clone()));
        Ok(Self { window, typed })
    }
}

impl Frontend for MinifbFrontend {
    /// Until the window is closed, or F4 pressed.
    fn is_open(&self) -> bool {
        self.window.is_open() && !self.window.is_key_down(Key::F4)
    }

    fn set_target_fps(&mut self, fps: usize) {
        self.window.set_target_fps(fps);
    }

    fn poll_input(&mut self) -> HostInput {
        HostInput {
            down: self.window.get_keys().into_iter().filter_map(host_key).collect(),
            typed: std::mem::take(&mut *self.typed.0.borrow_mut())
        }
    }

    fn present(&mut self, pixels: &[u32], (width, height): (usize, usize)) -> Result<(), String> {
        self.window
            .update_with_buffer(pixels, width, height)
            .map_err(|e| format!("failed to redraw the window: {e}"))
    }
}

/// Lists keys named the same in minifb's `Key` and `HostKey`.
macro_rules! same_names {
    ($($name:ident),* $(,)?) => {
        fn host_key(key: Key) -> Option<HostKey> {
            match key {
                $(Key::$name => Some(HostKey::$name),)*
                _ => None
            }
        }
    };
}

same_names!(
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9,
    Space, Enter, LeftShift, RightShift, LeftCtrl, RightCtrl, LeftAlt, RightAlt,
    CapsLock, Tab, Escape, Backspace, Delete, Up, Down, Left, Right,
    Minus, LeftBracket, RightBracket, Backslash, Semicolon, Apostrophe, Comma,
    Period, Slash, Backquote, Equal,
    NumPad0, NumPad1, NumPad2, NumPad3, NumPad4, NumPad5, NumPad6, NumPad7, NumPad8, NumPad9,
    NumPadDot, NumPadEnter, F7, F8, F9, F10,
    F6, F5
);
