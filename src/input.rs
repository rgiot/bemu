//! Host keyboard input, independent of the windowing backend.
//!
//! A frontend (minifb today, anything else tomorrow) only has to translate
//! its native events into a [`HostInput`]: the [`HostKey`]s currently held
//! down and the characters typed since the last poll. Everything else - the
//! host key to CPC key table, the OR-combining of host keys mapped to the
//! same CPC key, the layout-aware typed-character handling - lives here and
//! is testable without any window.

use crate::keyboard::{CpcKey, KeyboardLayout};
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
    /// Not CPC keys: the emulator's own (F7 saves a snapshot, F8 pauses and resumes, F9 cycles
    /// the display, F10 toggles full speed).
    F7,
    F8,
    F9,
    F10,
    /// F6 shows or hides the timing overlay.
    F6
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
        self.advance_on(typed, KeyboardLayout::Uk)
    }

    /// [`Self::advance`] for a machine whose keys type `layout`.
    pub fn advance_on(&mut self, typed: &mut TypedChars, layout: KeyboardLayout) -> &[CpcKey] {
        if self.frames_left == 0 {
            self.keys.clear();
            while let Some(c) = typed.pop() {
                if let Some(keys) = layout.keys_of_char(c) {
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
///
/// A host key is named by its place (a QWERTY one): on another layout the key at Q's place
/// types `a` (AZERTY), and pressing it reported both Q, from its place, and A, from the
/// character typed ("qa"). So the printable keys learn what they type: when one goes down and
/// the host types a character with it, that character's CPC key is what the key presses while
/// held (a game's keys included), with and without Shift; when the CPC needs the other Shift
/// state for it (`&` is Shift+6 on a UK CPC, unshifted on an AZERTY host), the key presses
/// nothing itself and the typed character does it all. A printable key not learned yet waits a
/// little for its character before it presses its place's key.
#[derive(Default)]
pub struct HostKeyboard {
    typed: TypedChars,
    combo: TypedCombo,
    /// The host keys down at the previous poll.
    held: Vec<HostKey>,
    /// Printable keys just pressed and not learned yet: the polls left to wait for the
    /// character the host types with them.
    waiting: Vec<(HostKey, u8)>,
    /// What a printable key types here, with (true) or without Shift: its CPC key, or `None`
    /// when the typed character does it alone.
    learned: Vec<((HostKey, bool), Option<CpcKey>)>
}

/// What a host key down presses.
enum Pressing {
    /// The key of its place (`HOST_KEY_MAP`).
    Place,
    /// The key it was learned to type (none: the typed character's alone).
    Learned(Option<CpcKey>),
    /// Nothing yet: its character may still come.
    Waiting
}

impl HostKeyboard {
    /// How many polls a printable key not learned yet waits for its character.
    pub const WAIT_POLLS: u8 = 2;

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
        let layout = motherboard.keyboard_layout();
        self.learn(input, layout);
        for &c in &input.typed {
            self.typed.push(c);
        }
        let combo = self.combo.advance_on(&mut self.typed, layout).to_vec();
        let shift = is_shift_down(&input.down);

        // every key a host key can press starts up, then the ones held go down
        let mut pressed: Vec<(CpcKey, bool)> = Vec::new();
        let mut press = |key: CpcKey, down: bool| match pressed.iter_mut().find(|(k, _)| *k == key) {
            Some((_, held)) => *held |= down,
            None => pressed.push((key, down))
        };
        for (_, key) in HOST_KEY_MAP {
            press(*key, false);
            if layout != KeyboardLayout::Uk {
                press(layout.key_labelled(*key), false);
            }
        }
        for (_, key) in &self.learned {
            if let Some(key) = key {
                press(*key, false);
            }
        }
        for &host in &input.down {
            match self.pressing(host, shift) {
                Pressing::Waiting | Pressing::Learned(None) => {},
                Pressing::Learned(Some(key)) => press(key, true),
                Pressing::Place => {
                    for (_, key) in HOST_KEY_MAP.iter().filter(|(h, _)| *h == host) {
                        if layout == KeyboardLayout::Uk {
                            press(*key, true);
                        }
                        // A letter goes where the layout has it (an AZERTY CPC's A is the UK Q).
                        // The UK punctuation keys are left to the typed characters: their places
                        // hold letters there.
                        else if !is_uk_punctuation(*key) {
                            press(layout.key_labelled(*key), true);
                        }
                    }
                }
            }
        }
        let matrix = motherboard.keyboard_mut();
        for (key, down) in pressed {
            matrix.set_pressed(key, down);
        }
        for key in combo {
            matrix.set_pressed(key, true);
        }
    }

    /// The printable keys just pressed wait for their characters; the characters typed teach
    /// them, one each, in order.
    fn learn(&mut self, input: &HostInput, layout: KeyboardLayout) {
        let shift = is_shift_down(&input.down);
        let control = input.down.iter().any(|k| matches!(k, HostKey::LeftCtrl | HostKey::RightCtrl));
        // released keys stop waiting; the others wait a poll less
        self.waiting.retain(|(key, _)| input.down.contains(key));
        for &key in &input.down {
            if is_printable(key) && !self.held.contains(&key) && self.learned_as(key, shift).is_none() {
                self.waiting.push((key, Self::WAIT_POLLS + 1));
            }
        }
        // Ctrl with a key types a control code, if anything: nothing to learn from
        if !control && !input.typed.is_empty() && input.typed.len() == self.waiting.len() {
            for ((key, _), &c) in self.waiting.drain(..).zip(&input.typed) {
                let pressing = layout.keys_of_char(c).and_then(|keys| {
                    let needs_shift = keys.contains(&CpcKey::Shift);
                    let key = keys.iter().copied().filter(|k| !matches!(k, CpcKey::Shift | CpcKey::Control)).last();
                    if needs_shift == shift { key } else { None }
                });
                self.learned.retain(|(learned, _)| *learned != (key, shift));
                self.learned.push(((key, shift), pressing));
            }
        }
        for (_, polls) in &mut self.waiting {
            *polls -= 1;
        }
        // the time is up: the place's key, as without a character
        self.waiting.retain(|(_, polls)| *polls > 0);
        self.held = input.down.clone();
    }

    fn learned_as(&self, key: HostKey, shift: bool) -> Option<Option<CpcKey>> {
        self.learned.iter().find(|(learned, _)| *learned == (key, shift)).map(|(_, pressing)| *pressing)
    }

    fn pressing(&self, key: HostKey, shift: bool) -> Pressing {
        if self.waiting.iter().any(|(waiting, _)| *waiting == key) {
            Pressing::Waiting
        }
        // the other Shift state's, not learned yet in this one (Shift pressed with the key held)
        else if let Some(pressing) = self.learned_as(key, shift).or_else(|| self.learned_as(key, !shift)) {
            Pressing::Learned(pressing)
        }
        else {
            Pressing::Place
        }
    }
}

fn is_shift_down(down: &[HostKey]) -> bool {
    down.iter().any(|k| matches!(k, HostKey::LeftShift | HostKey::RightShift))
}

/// The host keys that type a character, so whose CPC key the host's layout decides.
fn is_printable(key: HostKey) -> bool {
    use HostKey::*;
    matches!(
        key,
        A | B | C | D | E | F | G | H | I | J | K | L | M | N | O | P | Q | R | S | T | U | V | W | X | Y | Z
            | Key0 | Key1 | Key2 | Key3 | Key4 | Key5 | Key6 | Key7 | Key8 | Key9
            | Minus | LeftBracket | RightBracket | Backslash | Semicolon | Apostrophe | Comma | Period | Slash
            | Backquote | Equal
    )
}

/// The keys of the UK punctuation, which hold letters on other layouts.
fn is_uk_punctuation(key: CpcKey) -> bool {
    use CpcKey::*;
    matches!(key, Caret | Minus | At | Colon | Semicolon | Slash | Dot | Comma | OpenBracket | CloseBracket | Backslash)
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
    use crate::csl_runner::keys_of_char;

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

    /// On the French layout, a typed character's key is released afterwards even where no host
    /// key maps any more (`,` is the UK M's place).
    #[test]
    fn french_typed_keys_are_released() {
        use crate::keyboard::KeyboardLayout;
        let mut builder = crate::motherboard::MotherboardBuilder::default();
        builder.set_keyboard_layout(KeyboardLayout::French);
        let mut machine = builder.build();
        let mut keyboard = HostKeyboard::default();
        let typed = HostInput { typed: vec![','], ..Default::default() };
        keyboard.apply(&mut machine, &typed);
        assert!(machine.keyboard_mut().is_pressed(CpcKey::M), "the French , key");
        for _ in 0..TypedCombo::HOLD_FRAMES + TypedCombo::RELEASE_FRAMES {
            keyboard.apply(&mut machine, &HostInput::default());
        }
        assert!(!machine.keyboard_mut().is_pressed(CpcKey::M));
    }

    /// Polls a keyboard as a frontend does: the host keys down and the characters typed.
    fn poll(keyboard: &mut HostKeyboard, motherboard: &mut Motherboard, down: &[HostKey], typed: &str) -> Vec<CpcKey> {
        keyboard.apply(motherboard, &HostInput { down: down.to_vec(), typed: typed.chars().collect() });
        use CpcKey::*;
        [A, Q, Z, W, Digit1, Digit6, Shift, Control]
            .into_iter()
            .filter(|key| motherboard.keyboard().is_pressed(*key))
            .collect()
    }

    /// An AZERTY host: the key at Q's place types `a`. The CPC sees A alone, at once and while
    /// held (a game's key), never Q ("qa" before).
    #[test]
    fn a_key_the_layout_moves_presses_what_it_types() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::default();
        assert_eq!(poll(&mut keyboard, &mut motherboard, &[HostKey::Q], "a"), vec![CpcKey::A]);
        for _ in 0..10 {
            assert_eq!(poll(&mut keyboard, &mut motherboard, &[HostKey::Q], ""), vec![CpcKey::A], "held");
        }
        // the character typed late (a poll after the key): nothing meanwhile, then A
        let mut keyboard = HostKeyboard::default();
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        assert!(!poll(&mut keyboard, &mut motherboard, &[HostKey::Q], "").contains(&CpcKey::Q), "waits");
        assert_eq!(poll(&mut keyboard, &mut motherboard, &[HostKey::Q], "a"), vec![CpcKey::A]);
        // Shift pressed with it held: still A
        assert_eq!(poll(&mut keyboard, &mut motherboard, &[HostKey::Q, HostKey::LeftShift], ""), vec![CpcKey::A, CpcKey::Shift]);
    }

    /// A QWERTY host: Q types `q`, the same as its place.
    #[test]
    fn a_qwerty_host_is_unchanged() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::default();
        assert_eq!(poll(&mut keyboard, &mut motherboard, &[HostKey::Q], "q"), vec![CpcKey::Q]);
        assert_eq!(poll(&mut keyboard, &mut motherboard, &[HostKey::Q], ""), vec![CpcKey::Q]);
    }

    /// No character comes (Ctrl+key): after the wait, the place's key.
    #[test]
    fn a_key_typing_nothing_presses_its_place_after_a_while() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::default();
        let down = [HostKey::LeftCtrl, HostKey::Z];
        let polls: Vec<Vec<CpcKey>> = (0..4).map(|_| poll(&mut keyboard, &mut motherboard, &down, "")).collect();
        assert_eq!(polls[0], vec![CpcKey::Control], "waiting for a character");
        assert_eq!(polls[3], vec![CpcKey::Z, CpcKey::Control]);
    }

    /// AZERTY's digit row types `&` unshifted, Shift+6 on a UK CPC: the key presses nothing
    /// itself, the typed character does it (no 1 with it).
    #[test]
    fn a_key_needing_the_other_shift_is_left_to_the_typed_character() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::default();
        let first = poll(&mut keyboard, &mut motherboard, &[HostKey::Key1], "&");
        assert!(!first.contains(&CpcKey::Digit1), "{first:?}");
        assert!(first.contains(&CpcKey::Digit6) && first.contains(&CpcKey::Shift), "the typed &: {first:?}");
        // with Shift it types 1: learned apart
        for _ in 0..8 {
            poll(&mut keyboard, &mut motherboard, &[], "");
        }
        let shifted = poll(&mut keyboard, &mut motherboard, &[HostKey::Key1, HostKey::LeftShift], "1");
        assert!(shifted.contains(&CpcKey::Digit1), "{shifted:?}");
    }

}
