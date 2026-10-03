//! Actions a button runs and the values a control edits.

use orbit_core::{Operand, VarOp};
use serde::{Deserialize, Serialize};

/// A player setting a control can edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SettingKey {
    MasterVolume,
    MusicVolume,
    SfxVolume,
    VoiceVolume,
    /// Characters per second.
    TextSpeed,
    /// Seconds.
    AutoDelay,
    /// Dropdown: option 0 = read text only, 1 = all text.
    SkipMode,
    Fullscreen,
    Language,
    /// Screen reader support (toggle; ADR 0017, M7-09).
    ScreenReader,
    /// Dialogue, choice and UI text scale, `1.0..=1.6` (slider; M7-07).
    TextScale,
    /// High-contrast theme override (toggle; M7-07).
    HighContrast,
    /// Reduced motion: animations, shakes and transitions become cuts
    /// (toggle; M7-07).
    ReducedMotion,
}

/// What a widget reads and writes: a player setting, or a story variable
/// (a label shows it; a slider edits a number, a toggle a boolean).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BindPath {
    Setting(SettingKey),
    Var(String),
    /// Inside a `Repeat` template: the element of the row (`ui_version` 4).
    Item,
    /// Inside a `Repeat` template: the position of the row, from 0.
    ItemIndex,
    /// Inside a `Repeat` template over a map: the key of the row.
    ItemKey,
    /// A parameter the story gave the screen (`OpenScreen::params`).
    Param(String),
}

impl BindPath {
    /// The setting this path edits, when it is a supported one.
    pub fn setting(&self) -> Option<SettingKey> {
        match self {
            BindPath::Setting(key) => Some(*key),
            _ => None,
        }
    }

    /// The story variable this path reads, when it is one.
    pub fn var(&self) -> Option<&str> {
        match self {
            BindPath::Var(name) => Some(name),
            _ => None,
        }
    }

    /// Whether the path reads the row of a `Repeat` template.
    pub fn is_item(&self) -> bool {
        matches!(self, BindPath::Item | BindPath::ItemIndex | BindPath::ItemKey)
    }
}

/// What activating a button does. Several actions run in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UiAction {
    GameNew,
    GameContinue,
    GameQuit,
    ReturnToTitle,
    /// Opens a screen above the current one (by screen id; the built-in
    /// ids are `preferences`, `save`, `load`, `history`, `pause`).
    Open(String),
    /// Replaces the top screen.
    Replace(String),
    /// Closes the top screen.
    Close,
    /// Closes every screen (resumes the game).
    CloseAll,
    QuickSave,
    QuickLoad,
    OpenSave,
    OpenLoad,
    ResetPreferences,
    /// Plays a sound asset (by path).
    PlaySound(String),
    /// Free-form hook for the host or a plugin.
    Custom {
        name: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Jumps the running game to the start of a scene (by scene id; a scene
    /// name is accepted too). Closes the screens; rollback returns to the
    /// line shown before the jump (ADR 0011, addendum UI-09). Ignored at the
    /// title (there is no game to jump in).
    SceneGoto(String),
    /// A no-code story variable operation (set, add, toggle...). The shell
    /// applies it to the running game.
    VarSet(VarOp),
    /// Closes the top screen like `Close`, handing a value back to the story
    /// (`ui_version` 4, UI-14): a screen opened by an `OpenScreen` node puts
    /// it in the node's `result` variable. The value is a literal, a story
    /// variable, the element of a generated row (`Operand::Item`...) or a
    /// screen parameter. On a screen the story did not open it is a `Close`.
    /// (A separate variant keeps the plain `Close` unchanged in older
    /// documents and packs.)
    CloseWith(Operand),
}

impl UiAction {
    /// `false` for the actions that are declared but do nothing yet (none
    /// since `SceneGoto` is supported; kept for future reserved actions).
    pub fn is_supported(&self) -> bool {
        true
    }
}
