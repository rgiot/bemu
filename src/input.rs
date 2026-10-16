//! Host keyboard input, independent of the windowing backend.
//!
//! A frontend (minifb today, anything else tomorrow) only has to translate
//! its native events into a [`HostInput`]: the [`HostKey`]s currently held
//! down and the characters typed since the last poll. Everything else - the
//! host key to CPC key table, the OR-combining of host keys mapped to the
//! same CPC key, the layout-aware typed-character handling - lives here and
//! is testable without any window.

use crate::csl_runner::keys_of_char;
use crate::keyboard::CpcKey;
use crate::motherboard::Motherboard;
use std::collections::VecDeque;

/// A physical host key, named as in the usual windowing libraries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostKey {
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9,
    Space, Enter, LeftShift, RightShift, LeftCtrl, RightCtrl, LeftAlt, RightAlt,
    CapsLock, Tab, Escape, Backspace, Delete, Up, Down, Left, Right,
    Minus, LeftBracket, RightBracket, Backslash, Semicolon, Apostrophe, Comma,
    Period, Slash, Backquote, Equal,
    NumPad0, NumPad1, NumPad2, NumPad3, NumPad4, NumPad5, NumPad6, NumPad7, NumPad8, NumPad9,
    NumPadDot, NumPadEnter,
    /// Not CPC keys: the emulator's own (F9 cycles the display, F10 toggles full speed).
    F9,
    F10
}

/// What a frontend reports on each poll.
#[derive(Default, Debug)]
pub struct HostInput {
    /// Host keys currently held down.
    pub down: Vec<HostKey>,
    /// Characters typed since the previous poll, resolved by the OS under
    /// the active layout and modifiers (so independent of `down`).
    pub typed: Vec<char>
}

/// Queue of characters typed on the host keyboard.
#[derive(Default)]
pub struct TypedChars(VecDeque<char>);

impl TypedChars {
    pub fn push(&mut self, c: char) {
        self.0.push_back(c);
    }

    /// The next queued character, if any.
    pub fn pop(&mut self) -> Option<char> {
        self.0.pop_front()
    }
}

/// Holds a [`keys_of_char`]-resolved key combo "pressed" for a few frames
/// whenever a character arrives through the host's layout-aware character
/// input, on top of `HOST_KEY_MAP`'s direct physical-key polling.
///
/// Needed because a physical-key enum resolves a key's *base* (unshifted)
/// character under a fixed, US/UK-centric table: on a French AZERTY host the
/// digit row's unshifted glyphs are `& é " ' ( - è _ ç à` - digits only exist
/// at the *shifted* level there - so physical polling reports nothing for
/// those keys, regardless of Shift ("nothing happens" when pressing the key
/// labelled `"`).
///
/// Typed characters are routed through `keys_of_char` (the same table CSL
/// scripts use to turn text into CPC key combos) instead, and the resulting
/// combo is forced on for a few frames: long enough for the CPC firmware's
/// own keyboard scan to notice, short enough to not stall held-key gameplay
/// using the same keys once the physical polling takes back over.
#[derive(Default)]
pub struct TypedCombo {
    keys: Vec<CpcKey>,
    frames_left: u8
}

impl TypedCombo {
    /// How many frames (at the CPC's own ~50Hz redraw rate) to hold a
    /// typed character's key combo down for.
    pub const HOLD_FRAMES: u8 = 4;
    /// Frames with nothing pressed between two typed characters: without them the next
    /// character's keys come in the scan that releases the last one's, and the firmware takes
    /// them together (`2+2` pasted gave `2+"`: the Shift of `+` on the last `2`; `ll` one `l`).
    pub const RELEASE_FRAMES: u8 = 2;

    /// The combo to force on this frame: continues holding whatever's
    /// already in progress, else starts holding the next queued
    /// character's combo (skipping any character `keys_of_char` doesn't
    /// know how to type), else nothing.
    pub fn advance(&mut self, typed: &mut TypedChars) -> &[CpcKey] {
        if self.frames_left == 0 {
            self.keys.clear();
            while let Some(c) = typed.pop() {
                if let Some(keys) = keys_of_char(c) {
                    self.keys = keys;
                    self.frames_left = Self::HOLD_FRAMES + Self::RELEASE_FRAMES;
                    break;
                }
            }
        }
        if self.frames_left > 0 {
            self.frames_left -= 1;
        }
        // held, then released for RELEASE_FRAMES before the next character
        if self.frames_left >= Self::RELEASE_FRAMES { &self.keys } else { &[] }
    }
}

/// The host keyboard's state as seen by the emulator: feed it a
/// [`HostInput`] once per redraw.
#[derive(Default)]
pub struct HostKeyboard {
    typed: TypedChars,
    combo: TypedCombo
}

impl HostKeyboard {
    /// Set every mapped [`CpcKey`]'s pressed state from `input` - polled at
    /// the CPC's own ~50Hz rather than per-event.
    ///
    /// Several host keys map to the same [`CpcKey`] (both Shift keys, both
    /// Ctrl keys, both Alt keys), so each `CpcKey` is OR-combined across
    /// every host key mapped to it: a naive "one `set_pressed` per table
    /// entry" lets the last entry for a key cancel an earlier one that was
    /// held (holding only the left Shift got cancelled by the unheld right
    /// one, breaking every Shift-combo character).
    pub fn apply(&mut self, motherboard: &mut Motherboard, input: &HostInput) {
        for &c in &input.typed {
            self.typed.push(c);
        }
        let combo = self.combo.advance(&mut self.typed);
        let matrix = motherboard.keyboard_mut();
        for key in HOST_KEY_MAP {
            matrix.set_pressed(key.1, any_host_key_pressed(HOST_KEY_MAP, key.1, |k| input.down.contains(&k)));
        }
        for key in combo {
            matrix.set_pressed(*key, true);
        }
    }
}

/// Whether `target` should be considered pressed: any host key `map`s to it
/// that `is_down` reports held.
pub fn any_host_key_pressed(map: &[(HostKey, CpcKey)], target: CpcKey, is_down: impl Fn(HostKey) -> bool) -> bool {
    map.iter().filter(|(_, cpc)| *cpc == target).any(|(host, _)| is_down(*host))
}

/// Host key -> CPC key, per [`crate::keyboard`]'s own contract. A first pass
/// covering the obvious keys; a real CPC keyboard's punctuation doesn't line
/// up 1:1 with a modern one, so a few of these (`Colon`, `At`, `Caret`) are
/// best-effort placements rather than a faithful physical-layout match.
pub const HOST_KEY_MAP: &[(HostKey, CpcKey)] = {
    use CpcKey::*;
    use HostKey as Hk;
    &[
        (Hk::A, A), (Hk::B, B), (Hk::C, C), (Hk::D, D), (Hk::E, E),
        (Hk::F, F), (Hk::G, G), (Hk::H, CpcKey::H), (Hk::I, I), (Hk::J, J),
        (Hk::K, K), (Hk::L, L), (Hk::M, M), (Hk::N, N), (Hk::O, O),
        (Hk::P, P), (Hk::Q, Q), (Hk::R, R), (Hk::S, S), (Hk::T, T),
        (Hk::U, U), (Hk::V, V), (Hk::W, W), (Hk::X, X), (Hk::Y, Y),
        (Hk::Z, Z),
        (Hk::Key0, Digit0), (Hk::Key1, Digit1), (Hk::Key2, Digit2),
        (Hk::Key3, Digit3), (Hk::Key4, Digit4), (Hk::Key5, Digit5),
        (Hk::Key6, Digit6), (Hk::Key7, Digit7), (Hk::Key8, Digit8),
        (Hk::Key9, Digit9),
        (Hk::Space, Space),
        (Hk::Enter, Return),
        (Hk::LeftShift, Shift), (Hk::RightShift, Shift),
        (Hk::LeftCtrl, Control), (Hk::RightCtrl, Control),
        (Hk::LeftAlt, Copy), (Hk::RightAlt, Copy),
        (Hk::CapsLock, CapsLock),
        (Hk::Tab, Tab),
        (Hk::Escape, Esc),
        (Hk::Backspace, Del),
        (Hk::Delete, Clr),
        (Hk::Up, CursorUp), (Hk::Down, CursorDown),
        (Hk::Left, CursorLeft), (Hk::Right, CursorRight),
        (Hk::Minus, Minus),
        (Hk::LeftBracket, OpenBracket), (Hk::RightBracket, CloseBracket),
        (Hk::Backslash, Backslash),
        (Hk::Semicolon, Semicolon),
        (Hk::Apostrophe, Colon),
        (Hk::Comma, Comma),
        (Hk::Period, Dot),
        (Hk::Slash, Slash),
        (Hk::Backquote, Caret),
        (Hk::Equal, At),
        (Hk::NumPad0, Keypad0), (Hk::NumPad1, Keypad1),
        (Hk::NumPad2, Keypad2), (Hk::NumPad3, Keypad3),
        (Hk::NumPad4, Keypad4), (Hk::NumPad5, Keypad5),
        (Hk::NumPad6, Keypad6), (Hk::NumPad7, Keypad7),
        (Hk::NumPad8, Keypad8), (Hk::NumPad9, Keypad9),
        (Hk::NumPadDot, KeypadDot),
        (Hk::NumPadEnter, KeypadEnter),
    ]
};

#[cfg(test)]
mod test {
    use super::*;

    /// Holding only ONE of two host keys mapped to the same `CpcKey` must
    /// still register as pressed, not get cancelled by the other, unheld,
    /// one (see `HostKeyboard::apply`).
    #[test]
    fn holding_just_one_mapped_host_key_is_not_cancelled_by_the_other_unheld_one() {
        let map: &[(HostKey, CpcKey)] = &[(HostKey::LeftShift, CpcKey::Shift), (HostKey::RightShift, CpcKey::Shift)];
        assert!(any_host_key_pressed(map, CpcKey::Shift, |k| k == HostKey::LeftShift), "left alone should register");
        assert!(any_host_key_pressed(map, CpcKey::Shift, |k| k == HostKey::RightShift), "right alone should register");
        assert!(any_host_key_pressed(map, CpcKey::Shift, |_| true), "both together should register");
        assert!(!any_host_key_pressed(map, CpcKey::Shift, |_| false), "neither held should not register");
    }

    /// Both Alt keys act as the CPC's own COPY key.
    #[test]
    fn alt_is_mapped_to_copy() {
        assert!(HOST_KEY_MAP.contains(&(HostKey::LeftAlt, CpcKey::Copy)));
        assert!(HOST_KEY_MAP.contains(&(HostKey::RightAlt, CpcKey::Copy)));
    }

    /// `"` is Shift+2 on a real CPC keyboard: the host keys that should
    /// combine to reach it are both mapped and distinct.
    #[test]
    fn double_quote_is_reachable_via_shift_plus_2() {
        assert!(HOST_KEY_MAP.iter().any(|(h, c)| *h == HostKey::Key2 && *c == CpcKey::Digit2));
        assert!(HOST_KEY_MAP
            .iter()
            .any(|(h, c)| (*h == HostKey::LeftShift || *h == HostKey::RightShift) && *c == CpcKey::Shift));
    }

    /// On a French AZERTY host, physical-key polling alone can never produce
    /// `"` - but the OS's real character resolution still reports `"` was
    /// typed. Feed it straight through the typed characters, with no
    /// `HostKey` involved, and check it resolves to the CPC's own
    /// Shift+Digit2 and stays asserted for more than one frame.
    #[test]
    fn typed_double_quote_resolves_to_shift_plus_digit2_and_holds_for_several_frames() {
        let mut typed = TypedChars::default();
        typed.push('"');
        let mut combo = TypedCombo::default();

        let first = combo.advance(&mut typed).to_vec();
        assert_eq!(first, vec![CpcKey::Shift, CpcKey::Digit2]);

        // Still held on a later frame, with nothing new typed meanwhile.
        let second = combo.advance(&mut typed).to_vec();
        assert_eq!(second, vec![CpcKey::Shift, CpcKey::Digit2]);

        // Eventually releases rather than holding forever.
        let mut released = false;
        for _ in 0..TypedCombo::HOLD_FRAMES {
            if combo.advance(&mut typed).is_empty() {
                released = true;
                break;
            }
        }
        assert!(released, "a typed combo must eventually release, not stick forever");
    }

    /// Two typed characters are apart: their keys are never pressed in the same frame, and all
    /// keys are up for a while between them (`+` then `2`: the Shift of `+` is up before `2`).
    #[test]
    fn typed_characters_are_released_before_the_next_one() {
        let mut typed = TypedChars::default();
        typed.push('+');
        typed.push('2');
        let mut combo = TypedCombo::default();
        let frames: Vec<Vec<CpcKey>> = (0..20).map(|_| combo.advance(&mut typed).to_vec()).collect();
        let plus = keys_of_char('+').unwrap();
        let two = keys_of_char('2').unwrap();
        let held_plus = frames.iter().position(|keys| *keys != plus).unwrap();
        assert_eq!(held_plus, usize::from(TypedCombo::HOLD_FRAMES));
        let gap = frames[held_plus..].iter().take_while(|keys| keys.is_empty()).count();
        assert_eq!(gap, usize::from(TypedCombo::RELEASE_FRAMES));
        assert_eq!(frames[held_plus + gap], two, "then the 2, without the Shift");
    }

    /// A character `keys_of_char` doesn't know how to type is just skipped,
    /// not mistaken for an empty-but-held combo.
    #[test]
    fn untypeable_character_is_skipped_without_holding_anything() {
        let mut typed = TypedChars::default();
        typed.push('\u{1}'); // unmapped control code
        let mut combo = TypedCombo::default();
        assert!(combo.advance(&mut typed).is_empty());
    }
}
