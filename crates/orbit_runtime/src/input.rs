//! Decoding of raw platform input into logical UI actions.

use orbit_platform::{InputEvent, KeyCode, MouseButton};

/// A logical input, independent of the device that produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiAction {
    /// Advance / confirm (Space, Enter).
    Confirm,
    /// Primary pointer click at a virtual position.
    Click([f32; 2]),
    /// Pointer moved to a virtual position.
    Hover([f32; 2]),
    Up,
    Down,
    /// A digit key, 1-based (`Digit1`..`Digit9`, numpad too).
    Number(usize),
}

/// Stateless apart from the last pointer position.
#[derive(Debug)]
pub struct InputMapper {
    pointer: [f32; 2],
}

impl Default for InputMapper {
    fn default() -> Self {
        Self {
            pointer: [-1.0, -1.0],
        }
    }
}

impl InputMapper {
    /// Last known pointer position in virtual pixels.
    pub fn pointer(&self) -> [f32; 2] {
        self.pointer
    }

    /// `to_world` converts a surface pixel position to virtual coordinates
    /// (letterbox aware).
    pub fn map(
        &mut self,
        event: &InputEvent,
        to_world: impl Fn([f32; 2]) -> [f32; 2],
    ) -> Option<UiAction> {
        match event {
            InputEvent::CursorMoved { x, y } => {
                self.pointer = to_world([*x as f32, *y as f32]);
                Some(UiAction::Hover(self.pointer))
            }
            InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: true,
            } => Some(UiAction::Click(self.pointer)),
            InputEvent::Key {
                code,
                pressed: true,
            } => key_action(*code),
            _ => None,
        }
    }
}

fn key_action(code: KeyCode) -> Option<UiAction> {
    use KeyCode::*;
    Some(match code {
        Space | Enter | NumpadEnter => UiAction::Confirm,
        ArrowUp => UiAction::Up,
        ArrowDown => UiAction::Down,
        Digit1 | Numpad1 => UiAction::Number(1),
        Digit2 | Numpad2 => UiAction::Number(2),
        Digit3 | Numpad3 => UiAction::Number(3),
        Digit4 | Numpad4 => UiAction::Number(4),
        Digit5 | Numpad5 => UiAction::Number(5),
        Digit6 | Numpad6 => UiAction::Number(6),
        Digit7 | Numpad7 => UiAction::Number(7),
        Digit8 | Numpad8 => UiAction::Number(8),
        Digit9 | Numpad9 => UiAction::Number(9),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, pressed: bool) -> InputEvent {
        InputEvent::Key { code, pressed }
    }

    fn halve(p: [f32; 2]) -> [f32; 2] {
        [p[0] / 2.0, p[1] / 2.0]
    }

    #[test]
    fn keys_map_to_actions() {
        let mut m = InputMapper::default();
        assert_eq!(
            m.map(&key(KeyCode::Space, true), halve),
            Some(UiAction::Confirm)
        );
        assert_eq!(
            m.map(&key(KeyCode::Enter, true), halve),
            Some(UiAction::Confirm)
        );
        assert_eq!(
            m.map(&key(KeyCode::ArrowUp, true), halve),
            Some(UiAction::Up)
        );
        assert_eq!(
            m.map(&key(KeyCode::ArrowDown, true), halve),
            Some(UiAction::Down)
        );
        assert_eq!(
            m.map(&key(KeyCode::Digit3, true), halve),
            Some(UiAction::Number(3))
        );
        assert_eq!(
            m.map(&key(KeyCode::Numpad9, true), halve),
            Some(UiAction::Number(9))
        );
        assert_eq!(m.map(&key(KeyCode::KeyA, true), halve), None);
    }

    #[test]
    fn releases_are_ignored() {
        let mut m = InputMapper::default();
        assert_eq!(m.map(&key(KeyCode::Space, false), halve), None);
        let up = InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: false,
        };
        assert_eq!(m.map(&up, halve), None);
    }

    #[test]
    fn click_uses_the_last_pointer_position_in_world_space() {
        let mut m = InputMapper::default();
        let moved = m.map(&InputEvent::CursorMoved { x: 200.0, y: 100.0 }, halve);
        assert_eq!(moved, Some(UiAction::Hover([100.0, 50.0])));
        let click = InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        };
        assert_eq!(m.map(&click, halve), Some(UiAction::Click([100.0, 50.0])));
        let right = InputEvent::MouseButton {
            button: MouseButton::Right,
            pressed: true,
        };
        assert_eq!(m.map(&right, halve), None);
    }

    #[test]
    fn scroll_is_ignored() {
        let mut m = InputMapper::default();
        assert_eq!(m.map(&InputEvent::Scroll { dx: 0.0, dy: 1.0 }, halve), None);
    }
}
