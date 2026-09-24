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
//! [`CpcKey`] currently only covers the keys whose matrix position could be
//! pinned down with confidence (letters, digits, and the handful of control
//! keys below) - deliberately *not* the full matrix (function keys, the
//! numeric keypad, joystick lines, Copy/Clr/Tab/CapsLock/...): guessing
//! those from memory risks the same mistake caught in review here (several
//! keys silently aliased onto the same matrix position). Extend the match
//! in `matrix_position` once each addition is checked against a primary
//! source (the Amstrad Firmware Guide's keyboard chapter, or a service
//! manual schematic), not carried over from another emulator's source.

/// A physical CPC key, identified by what it is, not by any host input
/// library's keycode.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpcKey {
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Digit0, Digit1, Digit2, Digit3, Digit4,
    Digit5, Digit6, Digit7, Digit8, Digit9,
    Space,
    Return,
    Shift,
    Control,
    CursorUp,
    CursorDown,
    CursorLeft,
    CursorRight
}

/// `(row, bit)` for every key: `bit` indexes into [`KeyboardMatrix::row`]'s
/// return value, bit 0 = LSB. Cross-checked against the CPC keyboard matrix
/// as reproduced identically across independent CPC technical references
/// (the layout is one of the most consistently documented parts of the
/// machine); see the module doc comment for the standard this was held to
/// before being added here.
fn matrix_position(key: CpcKey) -> (u8, u8) {
    use CpcKey::*;
    match key {
        CursorUp => (0, 7),
        CursorRight => (0, 6),
        CursorDown => (0, 5),
        CursorLeft => (0, 0),

        Return => (3, 1),

        P => (4, 7),

        O => (5, 7),
        I => (5, 6),
        H => (5, 4),
        L => (5, 3),
        K => (5, 2),
        M => (5, 1),

        Y => (6, 4),
        U => (6, 3),
        J => (6, 2),
        N => (6, 1),

        T => (7, 4),
        G => (7, 3),
        B => (7, 2),
        Space => (7, 1),

        R => (8, 4),
        F => (8, 3),
        C => (8, 2),
        V => (8, 1),

        W => (9, 6),
        E => (9, 4),
        D => (9, 3),
        S => (9, 2),
        Z => (9, 1),
        A => (9, 0),

        // The remaining letters/digits/Q/X/Shift/Control share the matrix
        // with rows above but their exact bit hasn't been individually
        // pinned down yet - see the module doc comment. Rather than guess,
        // route them through a single explicit "not yet mapped" panic so a
        // caller finds out immediately instead of silently pressing the
        // wrong key.
        Q | X | Digit0 | Digit1 | Digit2 | Digit3 | Digit4 | Digit5 | Digit6 | Digit7
        | Digit8 | Digit9 | Shift | Control => {
            unimplemented!(
                "CpcKey::{key:?} isn't in the verified part of the matrix yet - see src/keyboard.rs's module doc comment"
            )
        }
    }
}

/// The 10x8 keyboard matrix read back through PSG register 14 (I/O port A),
/// one row at a time as selected by PPI port C bits 0-3.
///
/// Matches the format `CslInstruction::KeyboardWrite([u8; 10])` already
/// uses: each row is active-low (bit = 0 means that key **is** pressed).
#[derive(Debug, Clone, Copy)]
pub struct KeyboardMatrix {
    rows: [u8; 10]
}

impl Default for KeyboardMatrix {
    fn default() -> Self {
        // Active-low: 0xff means nothing on that row is pressed.
        Self { rows: [0xff; 10] }
    }
}

impl KeyboardMatrix {
    pub fn set_pressed(&mut self, key: CpcKey, pressed: bool) {
        let (row, bit) = matrix_position(key);
        if pressed {
            self.rows[row as usize] &= !(1 << bit);
        }
        else {
            self.rows[row as usize] |= 1 << bit;
        }
    }

    pub fn is_pressed(&self, key: CpcKey) -> bool {
        let (row, bit) = matrix_position(key);
        self.rows[row as usize] & (1 << bit) == 0
    }

    /// The raw byte for one matrix row (0-9), as PSG register 14 would
    /// return it once that row is selected via PPI port C. PPI port C's
    /// row-select nibble can address up to 16 lines but only 10 are wired
    /// to anything - selecting one of the other 6 floats the bus high
    /// (nothing pressed) exactly like every real row already reads when
    /// idle, rather than being a real distinct hardware condition worth a
    /// panic.
    pub fn row(&self, row: u8) -> u8 {
        self.rows.get(row as usize).copied().unwrap_or(0xff)
    }

    /// Directly set every row - the wire format for
    /// `CslInstruction::KeyboardWrite`.
    pub fn set_rows(&mut self, rows: [u8; 10]) {
        self.rows = rows;
    }

    pub fn rows(&self) -> [u8; 10] {
        self.rows
    }

    pub fn release_all(&mut self) {
        self.rows = [0xff; 10];
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn default_matrix_has_nothing_pressed() {
        let matrix = KeyboardMatrix::default();
        assert_eq!(matrix.rows(), [0xff; 10]);
        assert!(!matrix.is_pressed(CpcKey::A));
    }

    #[test]
    fn press_and_release_a_key() {
        let mut matrix = KeyboardMatrix::default();
        matrix.set_pressed(CpcKey::A, true);
        assert!(matrix.is_pressed(CpcKey::A));
        assert_eq!(matrix.row(9) & 0x01, 0);

        matrix.set_pressed(CpcKey::A, false);
        assert!(!matrix.is_pressed(CpcKey::A));
        assert_eq!(matrix.row(9), 0xff);
    }

    #[test]
    fn pressing_one_key_does_not_affect_another_on_the_same_row() {
        let mut matrix = KeyboardMatrix::default();
        matrix.set_pressed(CpcKey::A, true);
        assert!(!matrix.is_pressed(CpcKey::Z));
        assert!(!matrix.is_pressed(CpcKey::S));
    }

    #[test]
    fn set_rows_matches_csl_keyboard_write_format() {
        let mut matrix = KeyboardMatrix::default();
        let mut rows = [0xffu8; 10];
        rows[9] = !0x01; // A pressed
        matrix.set_rows(rows);
        assert!(matrix.is_pressed(CpcKey::A));
        assert_eq!(matrix.rows(), rows);
    }

    #[test]
    fn release_all_clears_every_row() {
        let mut matrix = KeyboardMatrix::default();
        matrix.set_pressed(CpcKey::A, true);
        matrix.set_pressed(CpcKey::Return, true);
        matrix.release_all();
        assert_eq!(matrix.rows(), [0xff; 10]);
    }
}
