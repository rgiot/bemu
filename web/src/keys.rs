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

/// The key at `code`'s place, named as on a US keyboard (`HostInput::placed`): the place
/// whatever the layout, for the positional keyboard.
pub fn placed_key(code: &str) -> Option<HostKey> {
    use HostKey as H;
    if let Some(letter) = code.strip_prefix("Key") {
        let letters = [
            H::A, H::B, H::C, H::D, H::E, H::F, H::G, H::H, H::I, H::J, H::K, H::L, H::M, H::N, H::O, H::P, H::Q,
            H::R, H::S, H::T, H::U, H::V, H::W, H::X, H::Y, H::Z
        ];
        let (Some(c @ 'A'..='Z'), None) = ({ let mut cs = letter.chars(); (cs.next(), cs.next()) }) else {
            return None;
        };
        return Some(letters[c as usize - 'A' as usize]);
    }
    let digits = [H::Key0, H::Key1, H::Key2, H::Key3, H::Key4, H::Key5, H::Key6, H::Key7, H::Key8, H::Key9];
    let numpad_digits = [
        H::NumPad0, H::NumPad1, H::NumPad2, H::NumPad3, H::NumPad4, H::NumPad5, H::NumPad6, H::NumPad7, H::NumPad8,
        H::NumPad9
    ];
    let digit = |rest: &str| rest.parse::<usize>().ok().filter(|d| *d < 10);
    if let Some(d) = code.strip_prefix("Digit").and_then(digit) {
        return Some(digits[d]);
    }
    if let Some(d) = code.strip_prefix("Numpad").and_then(digit) {
        return Some(numpad_digits[d]);
    }
    Some(match code {
        "Backquote" => H::Backquote,
        "Minus" => H::Minus,
        "Equal" => H::Equal,
        "BracketLeft" => H::LeftBracket,
        "BracketRight" => H::RightBracket,
        "Backslash" => H::Backslash,
        "Semicolon" => H::Semicolon,
        "Quote" => H::Apostrophe,
        "Comma" => H::Comma,
        "Period" => H::Period,
        "Slash" => H::Slash,
        "IntlBackslash" => H::IntlBackslash,
        "Space" => H::Space,
        "Enter" => H::Enter,
        "Tab" => H::Tab,
        "CapsLock" => H::CapsLock,
        "Escape" => H::Escape,
        "Backspace" => H::Backspace,
        "Delete" => H::Delete,
        "ShiftLeft" => H::LeftShift,
        "ShiftRight" => H::RightShift,
        "ControlLeft" => H::LeftCtrl,
        "ControlRight" => H::RightCtrl,
        "AltLeft" => H::LeftAlt,
        "AltRight" => H::RightAlt,
        "ArrowUp" => H::Up,
        "ArrowDown" => H::Down,
        "ArrowLeft" => H::Left,
        "ArrowRight" => H::Right,
        "NumpadDecimal" => H::NumPadDot,
        "NumpadEnter" => H::NumPadEnter,
        _ => return None
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
        // the places: AZERTY's A key is KeyQ's
        assert_eq!(placed_key("KeyQ"), Some(HostKey::Q));
        assert_eq!(placed_key("Digit7"), Some(HostKey::Key7));
        assert_eq!(placed_key("Numpad7"), Some(HostKey::NumPad7));
        assert_eq!(placed_key("IntlBackslash"), Some(HostKey::IntlBackslash));
        assert_eq!(placed_key("KeyQQ"), None);
        assert_eq!(placed_key("F11"), None);
        assert_eq!(typed_char("é", false), Some('é'));
        assert_eq!(typed_char("c", true), None);
        assert_eq!(typed_char("Enter", false), None);
    }
}
