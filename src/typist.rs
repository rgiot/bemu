//! Text typed on the CPC keyboard as if by hand: `--type`, `--basic` and the autorun of the
//! frontends (see [`crate::autorun`]). One character after the other, each held for 2 frames then
//! released for 2 frames, with `\(WAIT n)` pauses.

use std::collections::VecDeque;

use crate::keyboard::CpcKey;
use crate::motherboard::Motherboard;

/// The CPC's clock: T-states (µs) per second.
pub const T_STATES_PER_SECOND: usize = 4_000_000;

/// `--type`: the text typed on the CPC keyboard, one character after the
/// other, each held for 2 frames then released for 2 frames.
pub struct Typist {
    /// `--basic`'s listing, put in memory (then RUN typed) when the typing starts.
    basic: Option<String>,
    keys: VecDeque<Step>,
    /// T-state at which the next press or release is due.
    next_at: usize,
    pressed: Option<Vec<CpcKey>>,
}

/// 40ms: 2 frames, long enough for the firmware's keyboard scan.
pub const KEY_T_STATES: usize = T_STATES_PER_SECOND / 25;

/// What `--type` does next: press keys together, or wait (`\(WAIT n)`, n seconds).
pub enum Step {
    Keys(Vec<CpcKey>),
    Pause(usize),
}

/// `text`'s keys, with its `\(WAIT n)` pauses (n seconds: a program's prompt to wait for).
pub fn steps_of_text(
    text: &str,
    layout: crate::keyboard::KeyboardLayout,
) -> Result<Vec<Step>, String> {
    const WAIT: &str = "\\(WAIT ";
    let mut steps = Vec::new();
    let mut rest = text;
    loop {
        let (keys, after) = match rest.find(WAIT) {
            Some(start) => (&rest[..start], Some(&rest[start + WAIT.len()..])),
            None => (rest, None),
        };
        steps.extend(
            crate::csl_runner::keys_of_text(keys, layout)?
                .into_iter()
                .map(Step::Keys),
        );
        let Some(after) = after else {
            return Ok(steps);
        };
        let end = after
            .find(')')
            .ok_or_else(|| format!("{WAIT}... without its )"))?;
        let seconds: f64 = after[..end]
            .trim()
            .parse()
            .map_err(|_| format!("{WAIT}{}): not a number of seconds", &after[..end]))?;
        steps.push(Step::Pause((seconds * T_STATES_PER_SECOND as f64) as usize));
        rest = &after[end + 1..];
    }
}

impl Typist {
    pub fn new(text: &str, at_seconds: f64, layout: crate::keyboard::KeyboardLayout) -> Self {
        let keys = steps_of_text(text, layout)
            .unwrap_or_else(|e| {
                eprintln!("--type: {e}");
                Vec::new()
            })
            .into();
        Self {
            basic: None,
            keys,
            next_at: (at_seconds * T_STATES_PER_SECOND as f64) as usize,
            pressed: None,
        }
    }

    /// `--basic`: the listing goes in memory at `at_seconds`, then RUN and `text` are typed.
    pub fn with_basic(
        text: &str,
        basic: String,
        at_seconds: f64,
        layout: crate::keyboard::KeyboardLayout,
    ) -> Self {
        let mut typist = Self::new(&format!("RUN\n{text}"), at_seconds, layout);
        typist.basic = Some(basic);
        typist
    }

    /// Press or release the keys that are due. Cheap: call it as often as
    /// wanted (every instruction or every frame).
    pub fn update(&mut self, motherboard: &mut Motherboard) {
        let now = motherboard.t_state_counter().value();
        if now >= self.next_at {
            if let Some(source) = self.basic.take() {
                // as LOAD would (BASIC is up by now: --type-at); what the tokenizer can't read
                // is typed instead (slower: the firmware tokenizes it)
                if let Err(error) = crate::basic_program::load(motherboard, &source) {
                    eprintln!("--basic: {error}\n--basic: typing the listing instead");
                    let layout = motherboard.keyboard_layout();
                    let typed = source
                        .lines()
                        .map(|line| format!("{line}\n"))
                        .collect::<String>();
                    match crate::csl_runner::keys_of_text(&typed, layout) {
                        Ok(keys) => {
                            for key in keys.into_iter().rev() {
                                self.keys.push_front(Step::Keys(key));
                            }
                        }
                        Err(error) => {
                            eprintln!("--basic: {error}");
                            self.keys.clear();
                        }
                    }
                }
            }
        }
        if now < self.next_at {
            // Held keys stay pressed even if something else (the window's
            // host keyboard) released them meanwhile.
            for &key in self.pressed.iter().flatten() {
                motherboard.keyboard_mut().set_pressed(key, true);
            }
            return;
        }
        let (keys, pressed) = match self.pressed.take() {
            Some(keys) => (keys, false),
            None => match self.keys.pop_front() {
                Some(Step::Keys(keys)) => (keys, true),
                Some(Step::Pause(t_states)) => {
                    self.next_at = now + t_states;
                    return;
                }
                None => return,
            },
        };
        for &key in &keys {
            motherboard.keyboard_mut().set_pressed(key, pressed);
        }
        if pressed {
            self.pressed = Some(keys);
        }
        self.next_at = now + KEY_T_STATES;
    }
}

impl Typist {
    /// Whether everything was typed (nothing pressed, nothing left).
    pub fn is_done(&self) -> bool {
        self.basic.is_none() && self.keys.is_empty() && self.pressed.is_none()
    }

    /// The keys held now: a frontend that sets the whole matrix each frame keeps them pressed.
    pub fn held(&self) -> &[CpcKey] {
        self.pressed.as_deref().unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use crate::keyboard::KeyboardLayout;

    use super::{steps_of_text, Step, T_STATES_PER_SECOND};

    #[test]
    fn typed_text_waits_where_asked() {
        let steps = steps_of_text("ab\\(WAIT 1.5)c\\(RET)", KeyboardLayout::Uk).unwrap();
        let shape: Vec<Option<usize>> = steps
            .iter()
            .map(|step| match step {
                Step::Keys(_) => None,
                Step::Pause(t_states) => Some(*t_states),
            })
            .collect();
        assert_eq!(
            shape,
            [None, None, Some(T_STATES_PER_SECOND * 3 / 2), None, None]
        );
        assert!(steps_of_text("\\(WAIT x)a", KeyboardLayout::Uk).is_err());
        assert!(steps_of_text("\\(WAIT 2", KeyboardLayout::Uk).is_err());
    }
}
