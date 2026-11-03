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
    /// The ISO keyboards' key left of Z (`<>` or `\|`).
    IntlBackslash,
    /// F1-F3 press the CPC's keypad 1-3 (f1-f3), for hosts without a keypad.
    F1,
    F2,
    F3,
    /// Not CPC keys: the emulator's own (F7 saves a snapshot, F8 pauses and resumes, F9 cycles
    /// the display, F10 toggles full speed).
    F7,
    F8,
    F9,
    F10,
    /// F6 shows or hides the timing overlay.
    F6,
    /// F5 turns the disc of drive A over (Shift+F5: drive B), in a 3" drive.
    F5,
    /// F12 switches between the symbolic and the positional keyboard mappings.
    F12
}

/// How the host's keys press the CPC's, as VICE's keyboard mappings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardMapping {
    /// A key types the character the host types with it, the CPC's Shift as that character
    /// needs it: for typing.
    #[default]
    Symbolic,
    /// A key presses the CPC key at its place, whatever either's labels: for games.
    Positional
}

impl KeyboardMapping {
    pub const NAMES: &'static str = "symbolic|positional";

    pub fn other(self) -> Self {
        match self {
            Self::Symbolic => Self::Positional,
            Self::Positional => Self::Symbolic
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Symbolic => "symbolic",
            Self::Positional => "positional"
        }
    }
}

impl std::str::FromStr for KeyboardMapping {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text {
            "symbolic" => Ok(Self::Symbolic),
            "positional" => Ok(Self::Positional),
            other => Err(format!("unknown keyboard mapping {other:?} (expected {})", Self::NAMES))
        }
    }
}

/// What a frontend reports on each poll.
#[derive(Default, Debug)]
pub struct HostInput {
    /// Host keys currently held down.
    pub down: Vec<HostKey>,
    /// Characters typed since the previous poll, resolved by the OS under
    /// the active layout and modifiers (so independent of `down`).
    pub typed: Vec<char>,
    /// The keys held named by their place on a US keyboard (the physical keys), for the
    /// positional mapping: on an AZERTY host the key labelled A is [`HostKey::Q`] here. Empty
    /// when the frontend can't tell (minifb): `down` stands in.
    pub placed: Vec<HostKey>
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
    frames_left: u8,
    /// A character started at the last advance.
    started: bool
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
                    self.started = true;
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

    /// Whether a character started at the last advance (once: the next call says false).
    pub fn take_started(&mut self) -> bool {
        std::mem::take(&mut self.started)
    }

    /// Hold the character a frame more (its keys come a frame later).
    pub fn hold_one_more(&mut self) {
        self.frames_left += 1;
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
    mapping: KeyboardMapping,
    /// The mapping changed: every key is released first (the other mapping's included).
    switched: bool,
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

    pub fn new(mapping: KeyboardMapping) -> Self {
        Self { mapping, ..Default::default() }
    }

    pub fn mapping(&self) -> KeyboardMapping {
        self.mapping
    }

    /// From the next [`Self::apply`] on; what was learned or typed is forgotten.
    pub fn set_mapping(&mut self, mapping: KeyboardMapping) {
        *self = Self { switched: true, ..Self::new(mapping) };
    }

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
        if std::mem::take(&mut self.switched) {
            motherboard.keyboard_mut().release_all();
        }
        match self.mapping {
            KeyboardMapping::Symbolic => self.apply_symbolic(motherboard, input),
            KeyboardMapping::Positional => apply_positional(motherboard, input)
        }
    }

    fn apply_symbolic(&mut self, motherboard: &mut Motherboard, input: &HostInput) {
        let layout = motherboard.keyboard_layout();
        self.learn(input, layout);
        for &c in &input.typed {
            self.typed.push(c);
        }
        let mut combo = self.combo.advance_on(&mut self.typed, layout).to_vec();
        let started = self.combo.take_started();
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
        // a key held pressing the CPC key it learned (a game's): the host's Shift is its
        let playing = input.down.iter().any(|&host| matches!(self.pressing(host, shift), Pressing::Learned(Some(_))));
        const MODIFIERS: [CpcKey; 3] = [CpcKey::Shift, CpcKey::Control, CpcKey::Copy];
        // The firmware takes a key going down in the scan that releases Shift as shifted (the
        // host's Shift+& types 1 on an AZERTY, Shift+1 on the CPC is !): a modifier the host
        // holds and the character doesn't want goes up a frame before the character's keys.
        let conflict = MODIFIERS.iter().any(|m| !combo.contains(m) && pressed.contains(&(*m, true)));
        let lead = started && !playing && conflict;
        if lead {
            self.combo.hold_one_more();
            combo.retain(|key| MODIFIERS.contains(key));
        }
        let matrix = motherboard.keyboard_mut();
        for (key, down) in pressed {
            matrix.set_pressed(key, down);
        }
        if (!combo.is_empty() || lead) && !playing {
            // the character's modifiers, not the host's: Shift+2 types @ on a US host, the CPC's
            // @ is unshifted (with Shift it is |), even once the 2 is up; AltGr (Alt, or Ctrl+Alt)
            // is no CTRL or COPY for the CPC
            matrix.set_pressed(CpcKey::Shift, false);
            matrix.set_pressed(CpcKey::Control, false);
            matrix.set_pressed(CpcKey::Copy, false);
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
                    let key = keys.iter().copied().rfind(|k| !matches!(k, CpcKey::Shift | CpcKey::Control));
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

/// The host keyboard's layout, for a window that names the keys after it rather than by their
/// place (minifb on Linux: the keys' symbols, X11's and Wayland's): what it takes to find the
/// places back, for the positional keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostLayout {
    /// US or UK: a key's symbol is its place.
    #[default]
    Qwerty,
    /// French.
    Azerty,
    /// German.
    Qwertz
}

impl HostLayout {
    pub const NAMES: &'static str = "qwerty|azerty|qwertz";

    /// The place of the key the window names `key`.
    fn place_of(self, key: HostKey) -> HostKey {
        use HostKey::*;
        match (self, key) {
            (Self::Azerty, A) => Q,
            (Self::Azerty, Q) => A,
            (Self::Azerty, Z) => W,
            (Self::Azerty, W) => Z,
            (Self::Azerty, M) => Semicolon,
            (Self::Azerty, Comma) => M,
            (Self::Azerty, Semicolon) => Comma,
            (Self::Azerty, Apostrophe) => Key4,
            (Self::Azerty, Minus) => Key6,
            (Self::Qwertz, Y) => Z,
            (Self::Qwertz, Z) => Y,
            (Self::Qwertz, Minus) => Slash,
            (_, key) => key
        }
    }

    /// The place of a key the window doesn't name (its symbol is no [`HostKey`]: AZERTY's é,
    /// QWERTZ's ö), from the character it types, without or with Shift.
    fn place_of_char(self, c: char) -> Option<HostKey> {
        use HostKey::*;
        Some(match (self, c) {
            (Self::Azerty, '&' | '1') => Key1,
            (Self::Azerty, 'é' | '2') => Key2,
            (Self::Azerty, '"' | '3') => Key3,
            (Self::Azerty, '(' | '5') => Key5,
            (Self::Azerty, 'è' | '7') => Key7,
            (Self::Azerty, '_' | '8') => Key8,
            (Self::Azerty, 'ç' | '9') => Key9,
            (Self::Azerty, 'à' | '0') => Key0,
            (Self::Azerty, ')' | '°') => Minus,
            (Self::Azerty, '$' | '£') => RightBracket,
            (Self::Azerty, 'ù' | '%') => Apostrophe,
            (Self::Azerty, '*' | 'µ') => Backslash,
            (Self::Azerty, '<' | '>') => IntlBackslash,
            (Self::Azerty, ':' | '/') => Period,
            (Self::Azerty, '!' | '§') => Slash,
            (Self::Azerty, '²') => Backquote,
            (Self::Qwertz, 'ß' | '?') => Minus,
            (Self::Qwertz, 'ü' | 'Ü') => LeftBracket,
            (Self::Qwertz, '+' | '*') => RightBracket,
            (Self::Qwertz, 'ö' | 'Ö') => Semicolon,
            (Self::Qwertz, 'ä' | 'Ä') => Apostrophe,
            (Self::Qwertz, '#' | '\'') => Backslash,
            (Self::Qwertz, '<' | '>') => IntlBackslash,
            (Self::Qwertz, '°') => Backquote,
            _ => return None
        })
    }
}

impl std::str::FromStr for HostLayout {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text.to_ascii_lowercase().as_str() {
            "qwerty" | "us" | "uk" | "gb" => Ok(Self::Qwerty),
            "azerty" | "fr" => Ok(Self::Azerty),
            "qwertz" | "de" => Ok(Self::Qwertz),
            other => Err(format!("unknown host keyboard layout {other:?} (expected {})", Self::NAMES))
        }
    }
}

/// The places of the keys held ([`HostInput::placed`]) for a window that names the keys after
/// the host's layout. A key the window doesn't name is known by the character it types only,
/// which comes as it goes down (and again with the key's auto-repeat), never as it goes up: it
/// is held for [`Self::CHARACTER_POLLS`] polls after each.
#[derive(Debug, Default)]
pub struct HostPlaces {
    layout: HostLayout,
    by_character: Vec<(HostKey, u8)>
}

impl HostPlaces {
    /// Two frames down and one up: what the firmware needs to see a key once.
    pub const CHARACTER_POLLS: u8 = 3;

    pub fn new(layout: HostLayout) -> Self {
        Self { layout, by_character: Vec::new() }
    }

    /// This poll's places, from the keys the window names and the characters typed.
    pub fn places(&mut self, down: &[HostKey], typed: &[char]) -> Vec<HostKey> {
        for (_, polls) in &mut self.by_character {
            *polls -= 1;
        }
        self.by_character.retain(|(_, polls)| *polls > 0);
        // the keypad's characters (/ * + and its digits) are no other key's
        let keypad = down.iter().any(|key| {
            use HostKey::*;
            matches!(key, NumPad0 | NumPad1 | NumPad2 | NumPad3 | NumPad4 | NumPad5 | NumPad6 | NumPad7 | NumPad8 | NumPad9 | NumPadDot | NumPadEnter)
        });
        if !keypad {
            for &c in typed {
                if let Some(place) = self.layout.place_of_char(c) {
                    self.by_character.retain(|(key, _)| *key != place);
                    self.by_character.push((place, Self::CHARACTER_POLLS));
                }
            }
        }
        let mut places: Vec<HostKey> = down.iter().map(|&key| self.layout.place_of(key)).collect();
        for (key, _) in &self.by_character {
            if !places.contains(key) {
                places.push(*key);
            }
        }
        places
    }
}

/// Every key of [`POSITIONAL_MAP`] as its place holds it: the CPC's matrix is the same on every
/// layout (an AZERTY CPC's A is the UK Q), so the CPC's own layout doesn't matter either.
fn apply_positional(motherboard: &mut Motherboard, input: &HostInput) {
    let held = if input.placed.is_empty() { &input.down } else { &input.placed };
    let matrix = motherboard.keyboard_mut();
    for (_, key) in POSITIONAL_MAP {
        matrix.set_pressed(*key, false);
    }
    for (host, key) in POSITIONAL_MAP {
        if held.contains(host) {
            matrix.set_pressed(*key, true);
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
        (Hk::F1, Keypad1), (Hk::F2, Keypad2), (Hk::F3, Keypad3),
    ]
};

/// Host key (by its place on a US keyboard) -> the CPC key at that place. The letters, digits
/// and the keys around them are where a 6128's are; the CPC's punctuation follows its own
/// rows (`- ^`, `@ [`, `: ; ]`, `, . / \`), so a host key presses the CPC key it sits on, not
/// the one with its label.
pub const POSITIONAL_MAP: &[(HostKey, CpcKey)] = {
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
        // the digits' row: ESC left of 1, - and ^ right of 0, CLR and DEL
        (Hk::Backquote, Esc), (Hk::Escape, Esc),
        (Hk::Minus, Minus), (Hk::Equal, Caret),
        (Hk::Delete, Clr), (Hk::Backspace, Del),
        // right of P: @ [; right of L: : ; ]; right of / (the ISO key left of Z): \
        (Hk::LeftBracket, At), (Hk::RightBracket, OpenBracket),
        (Hk::Semicolon, Colon), (Hk::Apostrophe, Semicolon), (Hk::Backslash, CloseBracket),
        (Hk::Comma, Comma), (Hk::Period, Dot), (Hk::Slash, Slash), (Hk::IntlBackslash, Backslash),
        (Hk::Tab, Tab), (Hk::CapsLock, CapsLock), (Hk::Enter, Return),
        (Hk::LeftShift, Shift), (Hk::RightShift, Shift),
        (Hk::LeftCtrl, Control), (Hk::RightCtrl, Control),
        (Hk::LeftAlt, Copy), (Hk::RightAlt, Copy),
        (Hk::Space, Space),
        (Hk::Up, CursorUp), (Hk::Down, CursorDown),
        (Hk::Left, CursorLeft), (Hk::Right, CursorRight),
        (Hk::NumPad0, Keypad0), (Hk::NumPad1, Keypad1),
        (Hk::NumPad2, Keypad2), (Hk::NumPad3, Keypad3),
        (Hk::NumPad4, Keypad4), (Hk::NumPad5, Keypad5),
        (Hk::NumPad6, Keypad6), (Hk::NumPad7, Keypad7),
        (Hk::NumPad8, Keypad8), (Hk::NumPad9, Keypad9),
        (Hk::NumPadDot, KeypadDot),
        (Hk::NumPadEnter, KeypadEnter),
        (Hk::F1, Keypad1), (Hk::F2, Keypad2), (Hk::F3, Keypad3),
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
        keyboard.apply(motherboard, &HostInput { down: down.to_vec(), typed: typed.chars().collect(), ..Default::default() });
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
        // its Shift up first (the firmware would take the 1 shifted), then the 1
        let shifted = poll(&mut keyboard, &mut motherboard, &[HostKey::Key1, HostKey::LeftShift], "1");
        assert!(shifted.is_empty(), "{shifted:?}");
        let shifted = poll(&mut keyboard, &mut motherboard, &[HostKey::Key1, HostKey::LeftShift], "");
        assert_eq!(shifted, vec![CpcKey::Digit1]);
    }

    /// Positional: an AZERTY host's key at Q's place (the frontend's logical A, typing `a`)
    /// presses the CPC's Q: the key at that place, which a French CPC labels A.
    #[test]
    fn positional_keys_press_the_cpc_key_at_their_place() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Positional);
        let input = |down: &[HostKey], placed: &[HostKey], typed: &str| HostInput {
            down: down.to_vec(),
            placed: placed.to_vec(),
            typed: typed.chars().collect()
        };
        keyboard.apply(&mut motherboard, &input(&[HostKey::A], &[HostKey::Q], "a"));
        assert!(motherboard.keyboard().is_pressed(CpcKey::Q));
        assert!(!motherboard.keyboard().is_pressed(CpcKey::A), "neither its label nor its character");
        // the punctuation by place: = is ^'s, [ @'s, ' ;'s, the ISO key \'s
        for (host, cpc) in [(HostKey::Equal, CpcKey::Caret), (HostKey::LeftBracket, CpcKey::At), (HostKey::Apostrophe, CpcKey::Semicolon), (HostKey::IntlBackslash, CpcKey::Backslash), (HostKey::Backquote, CpcKey::Esc)] {
            keyboard.apply(&mut motherboard, &input(&[], &[host, HostKey::LeftShift], ""));
            assert!(motherboard.keyboard().is_pressed(cpc) && motherboard.keyboard().is_pressed(CpcKey::Shift), "{host:?}");
            keyboard.apply(&mut motherboard, &input(&[], &[], ""));
            assert!(!motherboard.keyboard().is_pressed(cpc), "{host:?} released");
        }
        // a frontend that can't tell the places (minifb): the keys it reports
        keyboard.apply(&mut motherboard, &input(&[HostKey::Z], &[], "z"));
        assert!(motherboard.keyboard().is_pressed(CpcKey::Z));
    }

    /// Symbolic: a character the CPC types without Shift, typed with the host's Shift (`@` is
    /// Shift+2 on a US host) comes without the CPC's Shift (with it, the @ key types |).
    #[test]
    fn symbolic_characters_take_their_own_shift() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Symbolic);
        let down = [HostKey::LeftShift, HostKey::Key2];
        keyboard.apply(&mut motherboard, &HostInput { down: down.to_vec(), typed: vec!['@'], ..Default::default() });
        // the host's Shift up a frame first: the firmware takes a key going down as Shift goes
        // up shifted
        assert!(!motherboard.keyboard().is_pressed(CpcKey::Shift) && !motherboard.keyboard().is_pressed(CpcKey::At));
        keyboard.apply(&mut motherboard, &HostInput { down: down.to_vec(), ..Default::default() });
        assert!(motherboard.keyboard().is_pressed(CpcKey::At));
        assert!(!motherboard.keyboard().is_pressed(CpcKey::Shift), "the host's Shift is not @'s");
        assert!(!motherboard.keyboard().is_pressed(CpcKey::Digit2));
        // a German host (QWERTZ): its Y key (at the US Z's place) types `y`, and ß has no CPC
        // key: Y types y, ß nothing, and Shift held with a letter stays (a game's keys)
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Symbolic);
        keyboard.apply(&mut motherboard, &HostInput { down: vec![HostKey::Y], typed: vec!['y'], ..Default::default() });
        assert!(motherboard.keyboard().is_pressed(CpcKey::Y) && !motherboard.keyboard().is_pressed(CpcKey::Z));
        keyboard.apply(&mut motherboard, &HostInput { down: vec![HostKey::Y, HostKey::LeftShift], ..Default::default() });
        assert!(motherboard.keyboard().is_pressed(CpcKey::Y) && motherboard.keyboard().is_pressed(CpcKey::Shift));
        keyboard.apply(&mut motherboard, &HostInput { typed: vec!['ß'], ..Default::default() });
        // a French CPC: the character is found where its layout has it (A at the UK Q)
        let mut french = crate::motherboard::MotherboardBuilder::default().build();
        french.set_keyboard_layout(KeyboardLayout::French).unwrap();
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Symbolic);
        keyboard.apply(&mut french, &HostInput { down: vec![HostKey::A], typed: vec!['a'], ..Default::default() });
        assert!(french.keyboard().is_pressed(CpcKey::Q), "the French CPC's A is the UK Q's key");
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Symbolic);
        let down = [HostKey::LeftShift, HostKey::Key2];
        keyboard.apply(&mut motherboard, &HostInput { down: down.to_vec(), typed: vec!['@'], ..Default::default() });
        // the key up before its character's turn (a quick tap, Shift still held): still no Shift
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Symbolic);
        keyboard.apply(&mut motherboard, &HostInput { down: vec![HostKey::LeftShift], typed: vec!['@'], ..Default::default() });
        keyboard.apply(&mut motherboard, &HostInput { down: vec![HostKey::LeftShift], ..Default::default() });
        assert!(motherboard.keyboard().is_pressed(CpcKey::At) && !motherboard.keyboard().is_pressed(CpcKey::Shift));
        // AltGr reported as Right Alt (COPY) or Ctrl+Alt: the character alone
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Symbolic);
        let altgr = vec![HostKey::LeftCtrl, HostKey::RightAlt];
        keyboard.apply(&mut motherboard, &HostInput { down: altgr.clone(), typed: vec!['@'], ..Default::default() });
        keyboard.apply(&mut motherboard, &HostInput { down: altgr, ..Default::default() });
        for key in [CpcKey::Copy, CpcKey::Control, CpcKey::Shift] {
            assert!(!motherboard.keyboard().is_pressed(key), "{key:?}");
        }
        assert!(motherboard.keyboard().is_pressed(CpcKey::At));
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Symbolic);
        let down = [HostKey::LeftShift, HostKey::Key2];
        keyboard.apply(&mut motherboard, &HostInput { down: down.to_vec(), typed: vec!['@'], ..Default::default() });
        // switching mappings forgets the character
        keyboard.set_mapping(KeyboardMapping::Positional);
        keyboard.apply(&mut motherboard, &HostInput { down: down.to_vec(), ..Default::default() });
        assert!(motherboard.keyboard().is_pressed(CpcKey::Shift) && motherboard.keyboard().is_pressed(CpcKey::Digit2));
        assert!(!motherboard.keyboard().is_pressed(CpcKey::At));
    }

    /// A key the symbolic mapping holds outside the positional map's own (a French CPC's A,
    /// learned) is released when the mapping changes, so nothing stays down.
    #[test]
    fn switching_mappings_releases_every_key() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Positional);
        motherboard.keyboard_mut().set_pressed(CpcKey::Joy0Up, true);
        keyboard.set_mapping(KeyboardMapping::Symbolic);
        keyboard.apply(&mut motherboard, &HostInput::default());
        assert!(!motherboard.keyboard().is_pressed(CpcKey::Joy0Up));
    }

    /// A French host on a window naming the keys after the layout (minifb on Linux): the key
    /// labelled A is Q's place, M the US ;'s, and the digits' row, which it doesn't name, is
    /// found from the characters (é is 2's place), held a few polls.
    #[test]
    fn a_host_layout_gives_the_places_back() {
        let mut places = HostPlaces::new(HostLayout::Azerty);
        assert_eq!(places.places(&[HostKey::A, HostKey::M, HostKey::Comma], &['a']), [HostKey::Q, HostKey::Semicolon, HostKey::M]);
        assert_eq!(places.places(&[], &['é']), [HostKey::Key2]);
        for _ in 1..HostPlaces::CHARACTER_POLLS {
            assert_eq!(places.places(&[], &[]), [HostKey::Key2], "held");
        }
        assert_eq!(places.places(&[], &[]), [], "then up");
        // Shift: the same places
        assert_eq!(places.places(&[HostKey::LeftShift], &['2']), [HostKey::LeftShift, HostKey::Key2]);
        // the keypad's characters are not the main keys'
        let mut places = HostPlaces::new(HostLayout::Azerty);
        assert_eq!(places.places(&[HostKey::NumPad5], &['5']), [HostKey::NumPad5]);
        // German: Z and Y swapped, ö at the US ;
        let mut places = HostPlaces::new(HostLayout::Qwertz);
        assert_eq!(places.places(&[HostKey::Z, HostKey::Key1], &['ö']), [HostKey::Y, HostKey::Key1, HostKey::Semicolon]);
        // QWERTY: unchanged
        let mut places = HostPlaces::new(HostLayout::Qwerty);
        assert_eq!(places.places(&[HostKey::A, HostKey::Minus], &['a', '-']), [HostKey::A, HostKey::Minus]);
        assert_eq!("fr".parse::<HostLayout>(), Ok(HostLayout::Azerty));
    }

    /// The whole chain on a French host: the AZERTY key at Q's place (named A) presses the
    /// CPC's Q, its é key the CPC's 2.
    #[test]
    fn a_french_host_plays_by_place() {
        let mut motherboard = crate::motherboard::MotherboardBuilder::default().build();
        let mut keyboard = HostKeyboard::new(KeyboardMapping::Positional);
        let mut places = HostPlaces::new(HostLayout::Azerty);
        let (down, typed) = (vec![HostKey::A], vec!['a', 'é']);
        let placed = places.places(&down, &typed);
        keyboard.apply(&mut motherboard, &HostInput { down, typed, placed });
        for (key, pressed) in [(CpcKey::Q, true), (CpcKey::Digit2, true), (CpcKey::A, false)] {
            assert_eq!(motherboard.keyboard().is_pressed(key), pressed, "{key:?}");
        }
    }
}
