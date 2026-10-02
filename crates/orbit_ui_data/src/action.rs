//! Actions a button runs and the values a control edits.

use orbit_core::VarOp;
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
}

/// What a widget reads and writes: a player setting, or a story variable
/// (a label shows it; a slider edits a number, a toggle a boolean).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BindPath {
    Setting(SettingKey),
    Var(String),
}

impl BindPath {
    /// The setting this path edits, when it is a supported one.
    pub fn setting(&self) -> Option<SettingKey> {
        match self {
            BindPath::Setting(key) => Some(*key),
            BindPath::Var(_) => None,
        }
    }

    /// The story variable this path reads, when it is one.
    pub fn var(&self) -> Option<&str> {
        match self {
            BindPath::Var(name) => Some(name),
            BindPath::Setting(_) => None,
        }
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
    /// Reserved: jump to a scene. Not supported at runtime yet.
    SceneGoto(String),
    /// A no-code story variable operation (set, add, toggle...). The shell
    /// applies it to the running game.
    VarSet(VarOp),
}

impl UiAction {
    /// `false` for the actions that are declared but do nothing yet.
    pub fn is_supported(&self) -> bool {
        !matches!(self, UiAction::SceneGoto(_))
    }
}
