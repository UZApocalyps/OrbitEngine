//! Actions a button runs and the values a control edits.

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

/// What a widget reads and writes. Only [`BindPath::Setting`] works at
/// runtime yet; [`BindPath::Var`] is reserved for story variables.
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
    /// Reserved: set a story variable. Not supported at runtime yet.
    VarSet { name: String, value: String },
}

impl UiAction {
    /// `false` for the actions that are declared but do nothing yet.
    pub fn is_supported(&self) -> bool {
        !matches!(self, UiAction::SceneGoto(_) | UiAction::VarSet { .. })
    }
}
