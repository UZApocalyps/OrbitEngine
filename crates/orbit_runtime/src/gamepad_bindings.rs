//! Gamepad bindings: which button triggers which game action, resolved from
//! the player's preferences (missing or invalid entries use the defaults).

use std::collections::BTreeMap;

use orbit_platform::GamepadButton;
use orbit_save::GAMEPAD_ACTIONS;
use orbit_ui::UiInput;

use crate::input::{Hotkey, InputAction};

/// A remappable gamepad action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PadAction {
    Advance,
    Back,
    Menu,
    Skip,
    Auto,
    Rollback,
    History,
    ShowHotspots,
}

impl PadAction {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "advance" => Self::Advance,
            "back" => Self::Back,
            "menu" => Self::Menu,
            "skip" => Self::Skip,
            "auto" => Self::Auto,
            "rollback" => Self::Rollback,
            "history" => Self::History,
            "show_hotspots" => Self::ShowHotspots,
            _ => return None,
        })
    }

    fn input(self) -> InputAction {
        match self {
            Self::Advance => InputAction::Ui(UiInput::Confirm),
            // "Menu" opens the pause menu, which Cancel does in the game view.
            Self::Back | Self::Menu => InputAction::Ui(UiInput::Cancel),
            // Rolls back in the game view; pages a grid elsewhere.
            Self::Rollback => InputAction::Ui(UiInput::PageUp),
            Self::Skip => InputAction::Hotkey(Hotkey::ToggleSkip),
            Self::Auto => InputAction::Hotkey(Hotkey::ToggleAuto),
            Self::History => InputAction::Hotkey(Hotkey::History),
            Self::ShowHotspots => InputAction::Hotkey(Hotkey::ShowHotspots),
        }
    }
}

/// Resolved button-to-action table.
#[derive(Clone, Debug, PartialEq)]
pub struct GamepadBindings {
    /// Priority order: the first matching entry wins.
    table: Vec<(GamepadButton, PadAction)>,
}

impl Default for GamepadBindings {
    fn default() -> Self {
        Self::from_prefs(&BTreeMap::new())
    }
}

impl GamepadBindings {
    /// Builds the table from the `gamepad` map of the preferences. A button
    /// already taken by an earlier action keeps that action; an unknown
    /// button name falls back to the action's default.
    pub fn from_prefs(map: &BTreeMap<String, String>) -> Self {
        let mut table: Vec<(GamepadButton, PadAction)> = Vec::new();
        for (name, default) in GAMEPAD_ACTIONS {
            let Some(action) = PadAction::from_name(name) else {
                continue;
            };
            let button = map
                .get(name)
                .and_then(|b| GamepadButton::from_name(b))
                .or_else(|| GamepadButton::from_name(default));
            if let Some(button) = button
                && !table.iter().any(|(b, _)| *b == button)
            {
                table.push((button, action));
            }
        }
        Self { table }
    }

    /// The button bound to the remappable action `name` (see
    /// `orbit_save::GAMEPAD_ACTIONS`), if any.
    pub fn button_for(&self, name: &str) -> Option<GamepadButton> {
        let action = PadAction::from_name(name)?;
        self.table.iter().find(|(_, a)| *a == action).map(|(b, _)| *b)
    }

    /// The action of a pressed button: user bindings first, then the fixed
    /// d-pad navigation.
    pub fn action(&self, button: GamepadButton) -> Option<InputAction> {
        if let Some((_, action)) = self.table.iter().find(|(b, _)| *b == button) {
            return Some(action.input());
        }
        let nav = match button {
            GamepadButton::DPadUp => UiInput::Up,
            GamepadButton::DPadDown => UiInput::Down,
            GamepadButton::DPadLeft => UiInput::Left,
            GamepadButton::DPadRight => UiInput::Right,
            GamepadButton::RightTrigger => UiInput::PageDown,
            _ => return None,
        };
        Some(InputAction::Ui(nav))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui(i: UiInput) -> Option<InputAction> {
        Some(InputAction::Ui(i))
    }

    #[test]
    fn defaults() {
        let b = GamepadBindings::default();
        assert_eq!(b.action(GamepadButton::South), ui(UiInput::Confirm));
        assert_eq!(b.action(GamepadButton::East), ui(UiInput::Cancel));
        assert_eq!(b.action(GamepadButton::Start), ui(UiInput::Cancel));
        assert_eq!(b.action(GamepadButton::LeftBumper), ui(UiInput::PageUp));
        assert_eq!(
            b.action(GamepadButton::RightBumper),
            Some(InputAction::Hotkey(Hotkey::ToggleSkip))
        );
        assert_eq!(
            b.action(GamepadButton::North),
            Some(InputAction::Hotkey(Hotkey::ToggleAuto))
        );
        assert_eq!(
            b.action(GamepadButton::West),
            Some(InputAction::Hotkey(Hotkey::History))
        );
        assert_eq!(b.action(GamepadButton::DPadUp), ui(UiInput::Up));
        assert_eq!(b.action(GamepadButton::DPadRight), ui(UiInput::Right));
        assert_eq!(b.action(GamepadButton::Select), Some(InputAction::Hotkey(Hotkey::ShowHotspots)));
        assert_eq!(b.action(GamepadButton::LeftTrigger), None);
    }

    #[test]
    fn show_hotspots_has_a_gamepad_button_and_can_take_y() {
        let b = GamepadBindings::default();
        let show = Some(InputAction::Hotkey(Hotkey::ShowHotspots));
        assert_eq!(b.action(GamepadButton::Select), show);
        assert_eq!(b.button_for("show_hotspots"), Some(GamepadButton::Select));
        // Y / Triangle for it, Auto elsewhere.
        let map: BTreeMap<String, String> = [("show_hotspots", "North"), ("auto", "Select")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let b = GamepadBindings::from_prefs(&map);
        assert_eq!(b.action(GamepadButton::North), show);
        assert_eq!(b.action(GamepadButton::Select), Some(InputAction::Hotkey(Hotkey::ToggleAuto)));
        assert_eq!(b.button_for("show_hotspots"), Some(GamepadButton::North));
        assert_eq!(b.button_for("nothing"), None);
    }

    #[test]
    fn remapping_overrides_defaults_and_ignores_garbage() {
        let map: BTreeMap<String, String> = [
            ("advance", "East"),
            ("back", "South"),
            ("skip", "Banana"),
            ("unknown_action", "North"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let b = GamepadBindings::from_prefs(&map);
        assert_eq!(b.action(GamepadButton::East), ui(UiInput::Confirm));
        assert_eq!(b.action(GamepadButton::South), ui(UiInput::Cancel));
        // Invalid button name: default kept.
        assert_eq!(
            b.action(GamepadButton::RightBumper),
            Some(InputAction::Hotkey(Hotkey::ToggleSkip))
        );
    }

    #[test]
    fn a_button_bound_twice_keeps_the_first_action() {
        let map: BTreeMap<String, String> = [("back".to_owned(), "South".to_owned())].into();
        let b = GamepadBindings::from_prefs(&map);
        // "advance" comes first and keeps South.
        assert_eq!(b.action(GamepadButton::South), ui(UiInput::Confirm));
        // "back" lost its button: East does nothing now.
        assert_eq!(b.action(GamepadButton::East), None);
    }
}
