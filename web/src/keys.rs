//! A browser `KeyboardEvent` (`code`, `key`) as the CPC keyboard table (`amsrust::input`) knows it:
//! the *logical* key, as the desktop windows report it, with the characters typed.

use amsrust::input::HostKey;

/// The key held for `code` (the physical key) and `key` (what the layout makes of it).
pub fn host_key(code: &str, key: &str) -> Option<HostKey> {
    use HostKey as H;
    let numpad = code.starts_with("Numpad");
    let right = code.ends_with("Right");
    Some(match key {
        " " => H::Space,
        "Enter" if numpad => H::NumPadEnter,
        "Enter" => H::Enter,
        "Shift" if right => H::RightShift,
        "Shift" => H::LeftShift,
        "Control" if right => H::RightCtrl,
        "Control" => H::LeftCtrl,
        // AltGr is not Alt: it types characters
        "Alt" if right => H::RightAlt,
        "Alt" => H::LeftAlt,
        "CapsLock" => H::CapsLock,
        "Tab" => H::Tab,
        "Escape" => H::Escape,
        "Backspace" => H::Backspace,
        "Delete" => H::Delete,
        "ArrowUp" => H::Up,
        "ArrowDown" => H::Down,
        "ArrowLeft" => H::Left,
        "ArrowRight" => H::Right,
        _ => {
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next())
            else {
                return None;
            };
            let numpad_digits = [
                H::NumPad0, H::NumPad1, H::NumPad2, H::NumPad3, H::NumPad4, H::NumPad5, H::NumPad6, H::NumPad7,
                H::NumPad8, H::NumPad9
            ];
            let digits = [H::Key0, H::Key1, H::Key2, H::Key3, H::Key4, H::Key5, H::Key6, H::Key7, H::Key8, H::Key9];
            let letters = [
                H::A, H::B, H::C, H::D, H::E, H::F, H::G, H::H, H::I, H::J, H::K, H::L, H::M, H::N, H::O, H::P, H::Q,
                H::R, H::S, H::T, H::U, H::V, H::W, H::X, H::Y, H::Z
            ];
            let c = c.to_ascii_lowercase();
            match c {
                'a'..='z' => letters[c as usize - 'a' as usize],
                '0'..='9' if numpad => numpad_digits[c as usize - '0' as usize],
                '0'..='9' => digits[c as usize - '0' as usize],
                '.' if numpad => H::NumPadDot,
                '-' => H::Minus,
                '[' => H::LeftBracket,
                ']' => H::RightBracket,
                '\\' => H::Backslash,
                ';' => H::Semicolon,
                '\'' => H::Apostrophe,
                ',' => H::Comma,
                '.' => H::Period,
                '/' => H::Slash,
                '`' => H::Backquote,
                '=' => H::Equal,
                _ => return None
            }
        }
    })
}

/// The character `key` types, if it is one (not with Ctrl or Meta: shortcuts).
pub fn typed_char(key: &str, ctrl_or_meta: bool) -> Option<char> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !ctrl_or_meta && !c.is_control() => Some(c),
        _ => None
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn keys_follow_the_layout_and_the_location() {
        assert_eq!(host_key("KeyQ", "a"), Some(HostKey::A), "AZERTY: the key that types a");
        assert_eq!(host_key("Numpad7", "7"), Some(HostKey::NumPad7));
        assert_eq!(host_key("Digit7", "7"), Some(HostKey::Key7));
        assert_eq!(host_key("ShiftRight", "Shift"), Some(HostKey::RightShift));
        assert_eq!(host_key("F11", "F11"), None);
        assert_eq!(typed_char("é", false), Some('é'));
        assert_eq!(typed_char("c", true), None);
        assert_eq!(typed_char("Enter", false), None);
    }
}
