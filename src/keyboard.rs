//! The CPC keyboard matrix, modelled independently of any host UI/windowing
//! library.
//!
//! A frontend (winit, sdl2, a browser via wasm, a CSL/DAP-driven test
//! harness, ...) never touches `Ppi`/`Psg` state directly to "press a key".
//! It translates whatever key representation *its own* input library hands
//! it into a [`CpcKey`] and calls [`KeyboardMatrix::set_pressed`] - that
//! translation function is the only thing that needs to change when the UI
//! library changes; this module and everything downstream of it (PSG
//! register 14, `CslInstruction::KeyboardWrite`, ...) stay untouched.
//!
//! The (line, bit) of every key comes from the firmware itself: the default
//! key translation table at &1EEF in the lower ROM is indexed
//! `line * 8 + bit`, so it is the authoritative description of which key is
//! wired where (see `tests/keyboard_matrix_rom.rs`, which checks this file
//! against the ROM's own table).

/// A physical CPC key, identified by what it is, not by any host input
/// library's keycode.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpcKey {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Space,
    Return,
    Shift,
    Control,
    CapsLock,
    Tab,
    Esc,
    Del,
    Clr,
    Copy,
    CursorUp,
    CursorDown,
    CursorLeft,
    CursorRight,
    /// `^` (the pound sign on some keyboards)
    Caret,
    Minus,
    At,
    Colon,
    Semicolon,
    Slash,
    Dot,
    Comma,
    OpenBracket,
    CloseBracket,
    Backslash,
    /// Numeric keypad `0`-`9` (the firmware's "f0"-"f9" function keys)
    Keypad0,
    Keypad1,
    Keypad2,
    Keypad3,
    Keypad4,
    Keypad5,
    Keypad6,
    Keypad7,
    Keypad8,
    Keypad9,
    KeypadDot,
    KeypadEnter,
    /// Joystick 0 shares matrix line 9 with no keyboard key.
    Joy0Up,
    Joy0Down,
    Joy0Left,
    Joy0Right,
    Joy0Fire2,
    Joy0Fire1,
    /// Joystick 0's third fire button, on line 9 bit 6.
    Joy0Fire3,
    /// Joystick 1 shares matrix line 6 with the keyboard keys Digit6, Digit5,
    /// R, T, G, F, B (the real CPC wires the second joystick onto line 6).
    Joy1Up,
    Joy1Down,
    Joy1Left,
    Joy1Right,
    Joy1Fire1,
    Joy1Fire2,
    Joy1Fire3,
}

impl CpcKey {
    /// `(line, bit)` of the key in the matrix: `line` is the value selected
    /// on PPI port C bits 0-3, `bit` the bit that goes low in the byte read
    /// back through PSG register 14.
    pub const fn matrix_position(self) -> (u8, u8) {
        use CpcKey::*;
        match self {
            CursorUp => (0, 0),
            CursorRight => (0, 1),
            CursorDown => (0, 2),
            Keypad9 => (0, 3),
            Keypad6 => (0, 4),
            Keypad3 => (0, 5),
            KeypadEnter => (0, 6),
            KeypadDot => (0, 7),

            CursorLeft => (1, 0),
            Copy => (1, 1),
            Keypad7 => (1, 2),
            Keypad8 => (1, 3),
            Keypad5 => (1, 4),
            Keypad1 => (1, 5),
            Keypad2 => (1, 6),
            Keypad0 => (1, 7),

            Clr => (2, 0),
            OpenBracket => (2, 1),
            Return => (2, 2),
            CloseBracket => (2, 3),
            Keypad4 => (2, 4),
            Shift => (2, 5),
            Backslash => (2, 6),
            Control => (2, 7),

            Caret => (3, 0),
            Minus => (3, 1),
            At => (3, 2),
            P => (3, 3),
            Semicolon => (3, 4),
            Colon => (3, 5),
            Slash => (3, 6),
            Dot => (3, 7),

            Digit0 => (4, 0),
            Digit9 => (4, 1),
            O => (4, 2),
            I => (4, 3),
            L => (4, 4),
            K => (4, 5),
            M => (4, 6),
            Comma => (4, 7),

            Digit8 => (5, 0),
            Digit7 => (5, 1),
            U => (5, 2),
            Y => (5, 3),
            H => (5, 4),
            J => (5, 5),
            N => (5, 6),
            Space => (5, 7),

            Digit6 => (6, 0),
            Digit5 => (6, 1),
            R => (6, 2),
            T => (6, 3),
            G => (6, 4),
            F => (6, 5),
            B => (6, 6),
            V => (6, 7),

            Digit4 => (7, 0),
            Digit3 => (7, 1),
            E => (7, 2),
            W => (7, 3),
            S => (7, 4),
            D => (7, 5),
            C => (7, 6),
            X => (7, 7),

            Digit1 => (8, 0),
            Digit2 => (8, 1),
            Esc => (8, 2),
            Q => (8, 3),
            Tab => (8, 4),
            A => (8, 5),
            CapsLock => (8, 6),
            Z => (8, 7),

            Joy0Up => (9, 0),
            Joy0Down => (9, 1),
            Joy0Left => (9, 2),
            Joy0Right => (9, 3),
            Joy0Fire2 => (9, 4),
            Joy0Fire1 => (9, 5),
            Joy0Fire3 => (9, 6),
            Del => (9, 7),

            /// Joystick 1 shares line 6 with the keyboard keys (bits 0-6).
            Joy1Up => (6, 0),
            Joy1Down => (6, 1),
            Joy1Left => (6, 2),
            Joy1Right => (6, 3),
            Joy1Fire1 => (6, 4),
            Joy1Fire2 => (6, 5),
            Joy1Fire3 => (6, 6),
        }
    }
}

/// Number of lines in the matrix.
pub const NB_LINES: usize = 10;

/// The 10x8 keyboard matrix read back through PSG register 14 (I/O port A),
/// one line at a time as selected by PPI port C bits 0-3.
///
/// Matches the format `CslInstruction::KeyboardWrite([u8; 10])` already
/// uses: each line is active-low (bit = 0 means that key **is** pressed).
#[derive(Debug, Clone, Copy)]
pub struct KeyboardMatrix {
    lines: [u8; NB_LINES],
}

impl Default for KeyboardMatrix {
    fn default() -> Self {
        // Active-low: 0xff means nothing on that line is pressed.
        Self {
            lines: [0xff; NB_LINES],
        }
    }
}

impl KeyboardMatrix {
    pub fn set_pressed(&mut self, key: CpcKey, pressed: bool) {
        let (line, bit) = key.matrix_position();
        if pressed {
            self.lines[line as usize] &= !(1 << bit);
        } else {
            self.lines[line as usize] |= 1 << bit;
        }
    }

    pub fn is_pressed(&self, key: CpcKey) -> bool {
        let (line, bit) = key.matrix_position();
        self.lines[line as usize] & (1 << bit) == 0
    }

    /// The raw byte for one matrix line (0-9), as PSG register 14 would
    /// return it once that line is selected via PPI port C. Port C's
    /// line-select nibble can address 16 lines but only 10 are wired
    /// to anything - selecting one of the others floats the bus high
    /// (nothing pressed), exactly like every wired line reads when idle.
    pub fn line(&self, line: u8) -> u8 {
        self.lines.get(line as usize).copied().unwrap_or(0xff)
    }

    /// Directly set every line - the wire format for
    /// `CslInstruction::KeyboardWrite`.
    pub fn set_lines(&mut self, lines: [u8; NB_LINES]) {
        self.lines = lines;
    }

    pub fn lines(&self) -> [u8; NB_LINES] {
        self.lines
    }

    pub fn release_all(&mut self) {
        self.lines = [0xff; NB_LINES];
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn default_matrix_has_nothing_pressed() {
        let matrix = KeyboardMatrix::default();
        assert_eq!(matrix.lines(), [0xff; NB_LINES]);
        assert!(!matrix.is_pressed(CpcKey::A));
    }

    #[test]
    fn press_and_release_a_key() {
        let mut matrix = KeyboardMatrix::default();
        matrix.set_pressed(CpcKey::A, true);
        assert!(matrix.is_pressed(CpcKey::A));
        assert_eq!(matrix.line(8), !(1 << 5));

        matrix.set_pressed(CpcKey::A, false);
        assert!(!matrix.is_pressed(CpcKey::A));
        assert_eq!(matrix.line(8), 0xff);
    }

    #[test]
    fn pressing_one_key_does_not_affect_another_on_the_same_line() {
        let mut matrix = KeyboardMatrix::default();
        matrix.set_pressed(CpcKey::A, true);
        assert!(!matrix.is_pressed(CpcKey::Z));
        assert!(!matrix.is_pressed(CpcKey::Q));
    }

    #[test]
    fn set_lines_matches_csl_keyboard_write_format() {
        let mut matrix = KeyboardMatrix::default();
        let mut lines = [0xffu8; NB_LINES];
        lines[8] = !(1 << 5); // A pressed
        matrix.set_lines(lines);
        assert!(matrix.is_pressed(CpcKey::A));
        assert_eq!(matrix.lines(), lines);
    }

    #[test]
    fn release_all_clears_every_line() {
        let mut matrix = KeyboardMatrix::default();
        matrix.set_pressed(CpcKey::A, true);
        matrix.set_pressed(CpcKey::Return, true);
        matrix.release_all();
        assert_eq!(matrix.lines(), [0xff; NB_LINES]);
    }

    #[test]
    fn unwired_lines_read_as_nothing_pressed() {
        assert_eq!(KeyboardMatrix::default().line(12), 0xff);
    }
}

/// Every key of the matrix, in no particular order.
pub const ALL_KEYS: &[CpcKey] = {
    use CpcKey::*;
    &[
        A,
        B,
        C,
        D,
        E,
        F,
        G,
        H,
        I,
        J,
        K,
        L,
        M,
        N,
        O,
        P,
        Q,
        R,
        S,
        T,
        U,
        V,
        W,
        X,
        Y,
        Z,
        Digit0,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        Space,
        Return,
        Shift,
        Control,
        CapsLock,
        Tab,
        Esc,
        Del,
        Clr,
        Copy,
        CursorUp,
        CursorDown,
        CursorLeft,
        CursorRight,
        Caret,
        Minus,
        At,
        Colon,
        Semicolon,
        Slash,
        Dot,
        Comma,
        OpenBracket,
        CloseBracket,
        Backslash,
        Keypad0,
        Keypad1,
        Keypad2,
        Keypad3,
        Keypad4,
        Keypad5,
        Keypad6,
        Keypad7,
        Keypad8,
        Keypad9,
        KeypadDot,
        KeypadEnter,
        Joy0Up,
        Joy0Down,
        Joy0Left,
        Joy0Right,
        Joy0Fire2,
        Joy0Fire1,
        Joy0Fire3,
        Joy1Up,
        Joy1Down,
        Joy1Left,
        Joy1Right,
        Joy1Fire1,
        Joy1Fire2,
        Joy1Fire3,
    ]
};

/// The key at `line * 8 + bit` of the matrix (the firmware's key number).
pub fn key_at(index: usize) -> Option<CpcKey> {
    ALL_KEYS.iter().copied().find(|key| {
        let (line, bit) = key.matrix_position();
        usize::from(line) * 8 + usize::from(bit) == index
    })
}

/// What the keys type: the key translation tables of the firmware (the lower ROM), so UK
/// (QWERTY, the English ROMs) or French (AZERTY, the French 6128's ROMs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyboardLayout {
    #[default]
    Uk,
    French,
}

/// Where the 6128's (and 664's) firmware keeps its key translation tables: 80 bytes without a
/// modifier, then 80 with Shift, then 80 with Control.
const KEY_TABLES: usize = 0x1eef;

impl KeyboardLayout {
    /// The firmware's tables: normal, then Shift.
    fn tables(self) -> &'static [u8] {
        let rom: &'static [u8] = match self {
            Self::Uk => crate::roms::OS6128,
            Self::French => crate::roms::OS6128_AZERTY,
        };
        &rom[KEY_TABLES..KEY_TABLES + 160]
    }

    /// The code the firmware gives `c`: the French ROMs show ISO 646-FR, where é, è, à, ç, ù, §
    /// and ° take the codes of `{`, `}`, `@`, `\`, `|`, `]` and `[`.
    fn code_of(self, c: char) -> Option<u8> {
        let c = match (self, c) {
            (_, '\n') => '\r',
            (Self::French, 'é') => '{',
            (Self::French, 'è') => '}',
            (Self::French, 'à') => '@',
            (Self::French, 'ç') => '\\',
            (Self::French, 'ù') => '|',
            (Self::French, '§') => ']',
            (Self::French, '°') => '[',
            (_, c) => c,
        };
        u8::try_from(c).ok().filter(|code| code.is_ascii())
    }

    /// The keys to hold down together to type `c` (Shift and a key, or a key alone).
    pub fn keys_of_char(self, c: char) -> Option<Vec<CpcKey>> {
        if self == Self::Uk {
            return crate::csl_runner::keys_of_char(c);
        }
        let code = self.code_of(c)?;
        let (normal, shifted) = self.tables().split_at(80);
        // the keyboard's keys only: the joystick's line has letters too
        let keyboard = |index: &usize| *index < 72 || *index == 79;
        if let Some(index) = (0..80)
            .filter(keyboard)
            .find(|&index| normal[index] == code)
        {
            return key_at(index).map(|key| vec![key]);
        }
        let index = (0..80)
            .filter(keyboard)
            .find(|&index| shifted[index] == code)?;
        key_at(index).map(|key| vec![CpcKey::Shift, key])
    }

    /// The key a host key labelled as the UK `key` (a letter) is here: the one that types the same
    /// letter (`A` is where the UK `Q` is on an AZERTY keyboard). Other keys stay where they are.
    pub fn key_labelled(self, key: CpcKey) -> CpcKey {
        if self == Self::Uk {
            return key;
        }
        let (line, bit) = key.matrix_position();
        let uk = KeyboardLayout::Uk.tables()[usize::from(line) * 8 + usize::from(bit)];
        if !uk.is_ascii_lowercase() {
            return key;
        }
        self.tables()[..80]
            .iter()
            .position(|&k| k == uk)
            .and_then(key_at)
            .unwrap_or(key)
    }
}

#[cfg(test)]
mod layout_test {
    use super::*;

    #[test]
    fn every_matrix_position_has_one_key() {
        for index in 0..80 {
            let keys = ALL_KEYS.iter().filter(|key| {
                let (line, bit) = key.matrix_position();
                usize::from(line) * 8 + usize::from(bit) == index
            });
            // joystick 1 shares line 6 with the keyboard keys (Digit6..B), so
            // those positions hold two keys: a keyboard key and a joystick key.
            assert!(keys.count() <= 2, "{index}");
        }
    }

    #[test]
    fn azerty_types_through_the_french_tables() {
        use CpcKey::*;
        let fr = KeyboardLayout::French;
        assert_eq!(fr.keys_of_char('a'), Some(vec![Q]));
        assert_eq!(fr.keys_of_char('Z'), Some(vec![Shift, W]));
        assert_eq!(fr.keys_of_char('m'), Some(vec![Colon]));
        assert_eq!(fr.keys_of_char('1'), Some(vec![Shift, Digit1]));
        assert_eq!(fr.keys_of_char('é'), Some(vec![Digit2]));
        assert_eq!(fr.keys_of_char('"'), Some(vec![Digit3]));
        assert_eq!(fr.keys_of_char('\n'), Some(vec![Return]));
        assert_eq!(fr.key_labelled(A), Q);
        assert_eq!(fr.key_labelled(M), Colon);
        assert_eq!(fr.key_labelled(Digit1), Digit1);
        assert_eq!(KeyboardLayout::Uk.keys_of_char('a'), Some(vec![A]));
    }

    /// The French ROMs come with the layout, on a 6128 only.
    #[test]
    fn the_french_roms_are_fitted_on_a_6128() {
        use crate::motherboard::{CpcModel, MotherboardBuilder};
        let mut builder = MotherboardBuilder::default();
        builder.set_keyboard_layout(KeyboardLayout::French);
        let mut machine = builder.build();
        assert_eq!(machine.keyboard_layout(), KeyboardLayout::French);
        // the ROMs' own key table, as the machine reads it
        let rom = |machine: &crate::motherboard::Motherboard| machine.read_byte(0x1eef + 67);
        assert_eq!(rom(&machine), b'a', "the French OS: the UK Q key types a");
        assert!(machine.set_keyboard_layout(KeyboardLayout::Uk).is_ok());
        assert_eq!(rom(&machine), b'q');
        machine.set_model(CpcModel::Cpc464);
        assert!(machine.set_keyboard_layout(KeyboardLayout::French).is_err());
        assert_eq!(machine.keyboard_layout(), KeyboardLayout::Uk);
    }
}
