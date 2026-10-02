//! Central decoding of raw platform input (keyboard, mouse, wheel) into
//! logical actions. Nothing outside this module knows about key codes: the
//! runtime reacts to [`InputAction`]s only.

use orbit_platform::{InputEvent, KeyCode, MouseButton};
use orbit_ui::UiInput;

/// A keyboard shortcut of the game view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    QuickSave,
    QuickLoad,
    History,
    ToggleSkip,
    ToggleAuto,
    ToggleFullscreen,
}

/// A logical input, independent of the device that produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputAction {
    /// An abstract UI input: directions, confirm/cancel, pointer (virtual
    /// canvas coordinates), wheel (positive = up).
    Ui(UiInput),
    /// A digit key, 1-based (`Digit1`..`Digit9`, numpad too).
    Number(usize),
    Hotkey(Hotkey),
    /// The "temporary skip" key (Ctrl) was pressed or released.
    SkipHeld(bool),
}

/// Remembers the pointer position and the modifier keys.
#[derive(Debug)]
pub struct InputMapper {
    pointer: [f32; 2],
    shift: bool,
    ctrl: bool,
    alt: bool,
}

impl Default for InputMapper {
    fn default() -> Self {
        Self {
            pointer: [-1.0, -1.0],
            shift: false,
            ctrl: false,
            alt: false,
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
    ) -> Option<InputAction> {
        match event {
            InputEvent::CursorMoved { x, y } => {
                self.pointer = to_world([*x as f32, *y as f32]);
                Some(InputAction::Ui(UiInput::PointerMove(self.pointer)))
            }
            InputEvent::MouseButton { button, pressed } => match (button, pressed) {
                (MouseButton::Left, true) => {
                    Some(InputAction::Ui(UiInput::PointerDown(self.pointer)))
                }
                (MouseButton::Left, false) => {
                    Some(InputAction::Ui(UiInput::PointerUp(self.pointer)))
                }
                (MouseButton::Right, true) => Some(InputAction::Ui(UiInput::Cancel)),
                _ => None,
            },
            InputEvent::Scroll { dy, .. } if *dy != 0.0 => {
                Some(InputAction::Ui(UiInput::Scroll(*dy)))
            }
            InputEvent::Scroll { .. } => None,
            InputEvent::Key { code, pressed } => self.key(*code, *pressed),
        }
    }

    fn key(&mut self, code: KeyCode, pressed: bool) -> Option<InputAction> {
        use KeyCode::*;
        match code {
            ShiftLeft | ShiftRight => {
                self.shift = pressed;
                return None;
            }
            AltLeft | AltRight => {
                self.alt = pressed;
                return None;
            }
            ControlLeft | ControlRight => {
                // Key repeat sends further presses: report changes only.
                let changed = self.ctrl != pressed;
                self.ctrl = pressed;
                return changed.then_some(InputAction::SkipHeld(pressed));
            }
            _ if !pressed => return None,
            _ => {}
        }
        let ui = |input| Some(InputAction::Ui(input));
        let hotkey = |key| Some(InputAction::Hotkey(key));
        let number = |n| Some(InputAction::Number(n));
        match code {
            Enter | NumpadEnter if self.alt => hotkey(Hotkey::ToggleFullscreen),
            Space | Enter | NumpadEnter => ui(UiInput::Confirm),
            Escape => ui(UiInput::Cancel),
            ArrowUp => ui(UiInput::Up),
            ArrowDown => ui(UiInput::Down),
            ArrowLeft => ui(UiInput::Left),
            ArrowRight => ui(UiInput::Right),
            Tab if self.shift => ui(UiInput::Prev),
            Tab => ui(UiInput::Next),
            PageUp => ui(UiInput::PageUp),
            PageDown => ui(UiInput::PageDown),
            F5 => hotkey(Hotkey::QuickSave),
            F9 => hotkey(Hotkey::QuickLoad),
            F11 => hotkey(Hotkey::ToggleFullscreen),
            KeyH => hotkey(Hotkey::History),
            KeyS => hotkey(Hotkey::ToggleSkip),
            KeyA => hotkey(Hotkey::ToggleAuto),
            Digit1 | Numpad1 => number(1),
            Digit2 | Numpad2 => number(2),
            Digit3 | Numpad3 => number(3),
            Digit4 | Numpad4 => number(4),
            Digit5 | Numpad5 => number(5),
            Digit6 | Numpad6 => number(6),
            Digit7 | Numpad7 => number(7),
            Digit8 | Numpad8 => number(8),
            Digit9 | Numpad9 => number(9),
            _ => None,
        }
    }
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

    fn ui(input: UiInput) -> Option<InputAction> {
        Some(InputAction::Ui(input))
    }

    #[test]
    fn keys_map_to_actions() {
        let mut m = InputMapper::default();
        assert_eq!(
            m.map(&key(KeyCode::Space, true), halve),
            ui(UiInput::Confirm)
        );
        assert_eq!(
            m.map(&key(KeyCode::Enter, true), halve),
            ui(UiInput::Confirm)
        );
        assert_eq!(
            m.map(&key(KeyCode::Escape, true), halve),
            ui(UiInput::Cancel)
        );
        assert_eq!(m.map(&key(KeyCode::ArrowUp, true), halve), ui(UiInput::Up));
        assert_eq!(
            m.map(&key(KeyCode::ArrowLeft, true), halve),
            ui(UiInput::Left)
        );
        assert_eq!(
            m.map(&key(KeyCode::PageUp, true), halve),
            ui(UiInput::PageUp)
        );
        assert_eq!(
            m.map(&key(KeyCode::Digit3, true), halve),
            Some(InputAction::Number(3))
        );
        assert_eq!(
            m.map(&key(KeyCode::Numpad9, true), halve),
            Some(InputAction::Number(9))
        );
        assert_eq!(
            m.map(&key(KeyCode::F5, true), halve),
            Some(InputAction::Hotkey(Hotkey::QuickSave))
        );
        assert_eq!(
            m.map(&key(KeyCode::F9, true), halve),
            Some(InputAction::Hotkey(Hotkey::QuickLoad))
        );
        assert_eq!(m.map(&key(KeyCode::KeyZ, true), halve), None);
    }

    #[test]
    fn tab_and_shift_tab_cycle_focus() {
        let mut m = InputMapper::default();
        assert_eq!(m.map(&key(KeyCode::Tab, true), halve), ui(UiInput::Next));
        assert_eq!(m.map(&key(KeyCode::ShiftLeft, true), halve), None);
        assert_eq!(m.map(&key(KeyCode::Tab, true), halve), ui(UiInput::Prev));
        m.map(&key(KeyCode::ShiftLeft, false), halve);
        assert_eq!(m.map(&key(KeyCode::Tab, true), halve), ui(UiInput::Next));
    }

    #[test]
    fn ctrl_reports_press_and_release_once() {
        let mut m = InputMapper::default();
        assert_eq!(
            m.map(&key(KeyCode::ControlLeft, true), halve),
            Some(InputAction::SkipHeld(true))
        );
        // Key repeat: no new action.
        assert_eq!(m.map(&key(KeyCode::ControlLeft, true), halve), None);
        assert_eq!(
            m.map(&key(KeyCode::ControlLeft, false), halve),
            Some(InputAction::SkipHeld(false))
        );
    }

    #[test]
    fn alt_enter_toggles_fullscreen() {
        let mut m = InputMapper::default();
        m.map(&key(KeyCode::AltLeft, true), halve);
        assert_eq!(
            m.map(&key(KeyCode::Enter, true), halve),
            Some(InputAction::Hotkey(Hotkey::ToggleFullscreen))
        );
        m.map(&key(KeyCode::AltLeft, false), halve);
        assert_eq!(
            m.map(&key(KeyCode::Enter, true), halve),
            ui(UiInput::Confirm)
        );
    }

    #[test]
    fn releases_are_ignored() {
        let mut m = InputMapper::default();
        assert_eq!(m.map(&key(KeyCode::Space, false), halve), None);
    }

    #[test]
    fn pointer_events_use_world_space() {
        let mut m = InputMapper::default();
        let moved = m.map(&InputEvent::CursorMoved { x: 200.0, y: 100.0 }, halve);
        assert_eq!(moved, ui(UiInput::PointerMove([100.0, 50.0])));
        let press = |button, pressed| InputEvent::MouseButton { button, pressed };
        assert_eq!(
            m.map(&press(MouseButton::Left, true), halve),
            ui(UiInput::PointerDown([100.0, 50.0]))
        );
        assert_eq!(
            m.map(&press(MouseButton::Left, false), halve),
            ui(UiInput::PointerUp([100.0, 50.0]))
        );
        assert_eq!(
            m.map(&press(MouseButton::Right, true), halve),
            ui(UiInput::Cancel)
        );
        assert_eq!(m.map(&press(MouseButton::Right, false), halve), None);
    }

    #[test]
    fn wheel_maps_to_scroll_and_ignores_zero() {
        let mut m = InputMapper::default();
        assert_eq!(
            m.map(&InputEvent::Scroll { dx: 0.0, dy: 1.0 }, halve),
            ui(UiInput::Scroll(1.0))
        );
        assert_eq!(
            m.map(&InputEvent::Scroll { dx: 0.0, dy: -2.0 }, halve),
            ui(UiInput::Scroll(-2.0))
        );
        assert_eq!(m.map(&InputEvent::Scroll { dx: 1.0, dy: 0.0 }, halve), None);
    }
}
