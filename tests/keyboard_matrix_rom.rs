//! Checks the keyboard matrix against the firmware's own key translation
//! table (default table at &1EEF of the lower ROM, indexed `line * 8 +
//! bit`): the ROM is the authoritative statement of which key is wired to
//! which matrix position.

use amsrust::keyboard::CpcKey::{self, *};
use amsrust::motherboard::MotherboardBuilder;

const TRANSLATION_TABLE: u16 = 0x1eef;

/// Each key with the code the firmware's normal (unshifted) table gives it.
const KEYS: &[(CpcKey, u8)] = &[
    (CursorUp, 0xf0),
    (CursorRight, 0xf3),
    (CursorDown, 0xf1),
    (CursorLeft, 0xf2),
    (Copy, 0xe0),
    (Clr, 0x10),
    (Del, 0x7f),
    (Return, 0x0d),
    (Tab, 0x09),
    (Esc, 0xfc),
    (CapsLock, 0xfd),
    (Space, 0x20),
    (Keypad0, 0x80),
    (Keypad1, 0x81),
    (Keypad2, 0x82),
    (Keypad3, 0x83),
    (Keypad4, 0x84),
    (Keypad5, 0x85),
    (Keypad6, 0x86),
    (Keypad7, 0x87),
    (Keypad8, 0x88),
    (Keypad9, 0x89),
    (KeypadDot, 0x8a),
    (KeypadEnter, 0x8b),
    (Caret, b'^'),
    (Minus, b'-'),
    (At, b'@'),
    (Semicolon, b';'),
    (Colon, b':'),
    (Slash, b'/'),
    (Dot, b'.'),
    (Comma, b','),
    (OpenBracket, b'['),
    (CloseBracket, b']'),
    (Backslash, b'\\'),
    (Digit0, b'0'),
    (Digit1, b'1'),
    (Digit2, b'2'),
    (Digit3, b'3'),
    (Digit4, b'4'),
    (Digit5, b'5'),
    (Digit6, b'6'),
    (Digit7, b'7'),
    (Digit8, b'8'),
    (Digit9, b'9'),
    (A, b'a'),
    (B, b'b'),
    (C, b'c'),
    (D, b'd'),
    (E, b'e'),
    (F, b'f'),
    (G, b'g'),
    (H, b'h'),
    (I, b'i'),
    (J, b'j'),
    (K, b'k'),
    (L, b'l'),
    (M, b'm'),
    (N, b'n'),
    (O, b'o'),
    (P, b'p'),
    (Q, b'q'),
    (R, b'r'),
    (S, b's'),
    (T, b't'),
    (U, b'u'),
    (V, b'v'),
    (W, b'w'),
    (X, b'x'),
    (Y, b'y'),
    (Z, b'z'),
    // Shift and Control carry no character: the firmware table holds &FF there
    (Shift, 0xff),
    (Control, 0xff),
];

#[test]
fn every_key_sits_where_the_firmware_translation_table_says() {
    let motherboard = MotherboardBuilder::default().build();

    for &(key, expected_code) in KEYS {
        let (line, bit) = key.matrix_position();
        let address = TRANSLATION_TABLE + u16::from(line) * 8 + u16::from(bit);
        let rom_code = motherboard.read_lower_rom_byte(address);
        assert_eq!(
            rom_code, expected_code,
            "{key:?} is at line {line} bit {bit}, where the ROM table holds {rom_code:#04x} not {expected_code:#04x}"
        );
    }
}

#[test]
fn no_two_keys_share_a_matrix_position() {
    let mut seen = std::collections::HashMap::new();
    for &(key, _) in KEYS {
        if let Some(other) = seen.insert(key.matrix_position(), key) {
            panic!(
                "{key:?} and {other:?} share matrix position {:?}",
                key.matrix_position()
            );
        }
    }
}
