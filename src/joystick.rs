//! Joysticks: host-connected and simulated, wired onto the CPC keyboard matrix.
//!
//! The CPC has two joystick ports. Joystick 1 is read from matrix line 9
//! (PPI port C selects line 9, PSG register 14 reads it back), joystick 2
//! from matrix line 6 - which it shares with the keyboard keys Digit6, Digit5,
//! R, T, G, F, B. Each joystick reports a direction (up/down/left/right/none)
//! and three fire buttons.
//!
//! A joystick's state is a direction plus three fire buttons. The direction is
//! expressed as an axis pair (-1..=1 each) for the simulated joystick's stick,
//! and snapped to the four cardinal directions (plus centre) for the matrix.

use crate::keyboard::CpcKey;
use crate::motherboard::Motherboard;

/// One joystick's state: a direction and three fire buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoystickState {
    /// The stick's X axis, -1 = left, 0 = centre, 1 = right.
    pub x: i8,
    /// The stick's Y axis, -1 = up, 0 = centre, 1 = down.
    pub y: i8,
    /// Fire buttons 1-3.
    pub fire: [bool; 3],
}

impl JoystickState {
    /// The direction the stick is pointing: `None` when centred.
    pub fn direction(&self) -> Option<(i8, i8)> {
        if self.x == 0 && self.y == 0 {
            None
        } else {
            Some((self.x, self.y))
        }
    }

    /// Press the CPC matrix keys this joystick state implies onto `motherboard`,
    /// releasing the ones that are no longer pressed. `line` is the matrix
    /// line the joystick is wired to (9 for joystick 1, 6 for joystick 2).
    pub fn apply_to_matrix(self, mb: &mut Motherboard, line: u8) {
        let matrix = mb.keyboard_mut();
        let up = self.y < 0;
        let down = self.y > 0;
        let left = self.x < 0;
        let right = self.x > 0;
        let fire1 = self.fire[0];
        let fire2 = self.fire[1];
        let fire3 = self.fire[2];

        match line {
            9 => {
                matrix.set_pressed(CpcKey::Joy0Up, up);
                matrix.set_pressed(CpcKey::Joy0Down, down);
                matrix.set_pressed(CpcKey::Joy0Left, left);
                matrix.set_pressed(CpcKey::Joy0Right, right);
                matrix.set_pressed(CpcKey::Joy0Fire1, fire1);
                matrix.set_pressed(CpcKey::Joy0Fire2, fire2);
                matrix.set_pressed(CpcKey::Joy0Fire3, fire3);
            }
            6 => {
                matrix.set_pressed(CpcKey::Joy1Up, up);
                matrix.set_pressed(CpcKey::Joy1Down, down);
                matrix.set_pressed(CpcKey::Joy1Left, left);
                matrix.set_pressed(CpcKey::Joy1Right, right);
                matrix.set_pressed(CpcKey::Joy1Fire1, fire1);
                matrix.set_pressed(CpcKey::Joy1Fire2, fire2);
                matrix.set_pressed(CpcKey::Joy1Fire3, fire3);
            }
            _ => {}
        }
    }
}

/// Both joysticks: joystick 0 is the CPC's joystick 1 (line 9), joystick 1 is
/// the CPC's joystick 2 (line 6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Joysticks {
    pub joy0: JoystickState,
    pub joy1: JoystickState,
}

impl Joysticks {
    /// Apply both joysticks' state onto the keyboard matrix.
    pub fn apply_to_matrix(self, mb: &mut Motherboard) {
        self.joy0.apply_to_matrix(mb, 9);
        self.joy1.apply_to_matrix(mb, 6);
    }
}

/// Where a joystick's state comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JoystickSource {
    /// No joystick: all buttons up, stick centred.
    #[default]
    None,
    /// A host-connected joystick, polled by the frontend.
    Host,
    /// The simulated joystick window, driven by the mouse.
    Simulated,
}

impl JoystickSource {
    pub const NAMES: &'static str = "none|host|simulated";

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Host => "host",
            Self::Simulated => "simulated",
        }
    }
}

impl std::str::FromStr for JoystickSource {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text {
            "none" => Ok(Self::None),
            "host" => Ok(Self::Host),
            "simulated" => Ok(Self::Simulated),
            other => Err(format!(
                "unknown joystick source {other:?} (expected {})",
                Self::NAMES
            )),
        }
    }
}
