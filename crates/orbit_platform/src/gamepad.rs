//! Gamepad input: device-independent button names, a stick-to-d-pad
//! converter, and (native only) the `gilrs` poller.
//!
//! Web gap: the browser Gamepad API is not wired yet, so web builds receive
//! no gamepad events (keyboard and pointer keep working).

/// A physical gamepad button, named by position (layout independent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GamepadButton {
    /// Bottom face button (A on Xbox, cross on PlayStation).
    South,
    /// Right face button (B / circle).
    East,
    /// Left face button (X / square).
    West,
    /// Top face button (Y / triangle).
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

impl GamepadButton {
    pub const ALL: [GamepadButton; 14] = [
        Self::South,
        Self::East,
        Self::West,
        Self::North,
        Self::LeftBumper,
        Self::RightBumper,
        Self::LeftTrigger,
        Self::RightTrigger,
        Self::Select,
        Self::Start,
        Self::DPadUp,
        Self::DPadDown,
        Self::DPadLeft,
        Self::DPadRight,
    ];

    /// Stable name used in the preferences file.
    pub fn name(self) -> &'static str {
        match self {
            Self::South => "South",
            Self::East => "East",
            Self::West => "West",
            Self::North => "North",
            Self::LeftBumper => "LeftBumper",
            Self::RightBumper => "RightBumper",
            Self::LeftTrigger => "LeftTrigger",
            Self::RightTrigger => "RightTrigger",
            Self::Select => "Select",
            Self::Start => "Start",
            Self::DPadUp => "DPadUp",
            Self::DPadDown => "DPadDown",
            Self::DPadLeft => "DPadLeft",
            Self::DPadRight => "DPadRight",
        }
    }

    /// Inverse of [`GamepadButton::name`]; `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// Turns the left stick into d-pad presses (with hysteresis so a stick
/// resting near the threshold does not flicker).
#[derive(Debug, Default)]
pub struct StickDpad {
    held: [bool; 4], // up, down, left, right
}

const PRESS: f32 = 0.6;
const RELEASE: f32 = 0.35;

impl StickDpad {
    /// Feeds a stick position (`y` positive = up, like gilrs) and returns the
    /// d-pad button changes it causes.
    pub fn update(&mut self, x: f32, y: f32) -> Vec<(GamepadButton, bool)> {
        let dirs = [
            (GamepadButton::DPadUp, y),
            (GamepadButton::DPadDown, -y),
            (GamepadButton::DPadLeft, -x),
            (GamepadButton::DPadRight, x),
        ];
        let mut out = Vec::new();
        for (i, (button, v)) in dirs.into_iter().enumerate() {
            let held = self.held[i];
            if !held && v >= PRESS {
                self.held[i] = true;
                out.push((button, true));
            } else if held && v < RELEASE {
                self.held[i] = false;
                out.push((button, false));
            }
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::Poller;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{GamepadButton, StickDpad};
    use gilrs::{Axis, Button, EventType, Gilrs};

    /// Polls connected gamepads through `gilrs`.
    pub(crate) struct Poller {
        gilrs: Gilrs,
        stick: StickDpad,
        stick_x: f32,
        stick_y: f32,
    }

    fn map_button(b: Button) -> Option<GamepadButton> {
        Some(match b {
            Button::South => GamepadButton::South,
            Button::East => GamepadButton::East,
            Button::West => GamepadButton::West,
            Button::North => GamepadButton::North,
            Button::LeftTrigger => GamepadButton::LeftBumper,
            Button::RightTrigger => GamepadButton::RightBumper,
            Button::LeftTrigger2 => GamepadButton::LeftTrigger,
            Button::RightTrigger2 => GamepadButton::RightTrigger,
            Button::Select => GamepadButton::Select,
            Button::Start => GamepadButton::Start,
            Button::DPadUp => GamepadButton::DPadUp,
            Button::DPadDown => GamepadButton::DPadDown,
            Button::DPadLeft => GamepadButton::DPadLeft,
            Button::DPadRight => GamepadButton::DPadRight,
            _ => return None,
        })
    }

    impl Poller {
        /// `None` when no gamepad backend is available on this system.
        pub(crate) fn new() -> Option<Self> {
            match Gilrs::new() {
                Ok(gilrs) => Some(Self {
                    gilrs,
                    stick: StickDpad::default(),
                    stick_x: 0.0,
                    stick_y: 0.0,
                }),
                Err(err) => {
                    log::warn!("gamepad support unavailable: {err}");
                    None
                }
            }
        }

        /// Drains pending events into button changes.
        pub(crate) fn poll(&mut self) -> Vec<(GamepadButton, bool)> {
            let mut out = Vec::new();
            while let Some(event) = self.gilrs.next_event() {
                match event.event {
                    EventType::ButtonPressed(b, _) => {
                        out.extend(map_button(b).map(|b| (b, true)));
                    }
                    EventType::ButtonReleased(b, _) => {
                        out.extend(map_button(b).map(|b| (b, false)));
                    }
                    EventType::AxisChanged(axis, value, _) => {
                        match axis {
                            Axis::LeftStickX => self.stick_x = value,
                            Axis::LeftStickY => self.stick_y = value,
                            _ => continue,
                        }
                        out.extend(self.stick.update(self.stick_x, self.stick_y));
                    }
                    _ => {}
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for b in GamepadButton::ALL {
            assert_eq!(GamepadButton::from_name(b.name()), Some(b));
        }
        assert_eq!(GamepadButton::from_name("Nope"), None);
    }

    #[test]
    fn stick_presses_with_hysteresis() {
        let mut s = StickDpad::default();
        assert!(s.update(0.4, 0.0).is_empty());
        assert_eq!(s.update(0.9, 0.0), vec![(GamepadButton::DPadRight, true)]);
        // Still held between the two thresholds: no repeat, no release.
        assert!(s.update(0.5, 0.0).is_empty());
        assert_eq!(s.update(0.1, 0.0), vec![(GamepadButton::DPadRight, false)]);
        assert_eq!(s.update(0.0, -0.8), vec![(GamepadButton::DPadDown, true)]);
    }
}
