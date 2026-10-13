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
    /// `Repeat` source (`ui_version` 6, M7-03): the gallery images, in
    /// order, as a list of texts: the asset path of an unlocked image (one
    /// shown in a played game), an empty text for a locked one. The list
    /// names the eligible images (asset paths); empty = every background of
    /// the story, in story order.
    Gallery(Vec<String>),
    /// `Repeat` source (`ui_version` 6, M7-04): the scenes the player can
    /// replay (the eligible ones already seen in a played game) as a list of
    /// scene names in story order; `ReplayScene(Item)` replays the row. The
    /// list names the eligible scenes (ids or names); empty = every scene.
    ReplayScenes(Vec<String>),
    /// `Repeat` source (`ui_version` 7, M7-05): the achievements of the story
    /// in order, as a list of texts (their ids). Inside the row, read
    /// the achievement with [`BindPath::Achievement`] and tell the unlocked
    /// ones with `Cond::Unlocked`.
    Achievements,
    /// A field of the achievement of the row of an `Achievements` list
    /// (`ui_version` 7), or the unlock progress of the whole story.
    Achievement(AchievementField),
    /// A field of the slot of the card of a `SaveSlots` template
    /// (`ui_version` 9).
    SlotField(SlotField),
}

/// What [`BindPath::SlotField`] reads (`ui_version` 9): one slot of a
/// `SaveSlots` card template. Texts are empty when the slot has no save
/// (or the host does not know the value).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotField {
    /// `Slot 3`, `Auto 1`, `Quick save`.
    Title,
    /// The number of the slot alone (`3`).
    Number,
    /// Date and time, then the chapter: what the stock card shows.
    Timestamp,
    /// Date and time of the save.
    Date,
    /// Time played (`1h 05m`), when the save records it.
    PlayTime,
    /// Name of the scene (chapter) the save was made in.
    Chapter,
    /// The last line spoken, cut to what the stock card shows.
    Preview,
    /// `Empty` or `Incompatible` (localized); empty for a usable save.
    Status,
    /// Thumbnail of the save (bind it to an `Image`).
    Thumbnail,
}

/// What [`BindPath::Achievement`] reads (`ui_version` 7, M7-05).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AchievementField {
    /// Title in the language of the player (a placeholder while a hidden
    /// achievement is locked).
    Title,
    /// Description, likewise.
    Description,
    /// Asset path of the icon, empty when it has none or is hidden and
    /// locked (bind it to an `Image`).
    Icon,
    /// `3 / 10`: unlocked achievements out of all (not a row field).
    Progress,
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
        matches!(
            self,
            BindPath::Item
                | BindPath::ItemIndex
                | BindPath::ItemKey
                | BindPath::Achievement(
                    AchievementField::Title | AchievementField::Description | AchievementField::Icon
                )
                | BindPath::SlotField(_)
        )
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
    /// Replays a scene (id or name; `Item` in a `ReplayScenes` row) in
    /// replay mode (`ui_version` 6, M7-04): the scene runs on its own copy of
    /// the variables, nothing is saved, and its end (or the menu) comes back
    /// to the title with the `replay` screen open. Only seen scenes replay.
    ReplayScene(Operand),
    /// Plays a music file of the project (path) in a loop, replacing the
    /// music playing (`ui_version` 9). Lasts until another screen with music
    /// is shown or `StopMusic`.
    PlayMusic(String),
    /// Fades the music out (`ui_version` 9).
    StopMusic,
    /// Shows an in-game notification (toast, `ui_version` 11).
    Notify(crate::notify::UiNotify),
}

impl UiAction {
    /// `false` for the actions that are declared but do nothing yet (none
    /// since `SceneGoto` is supported; kept for future reserved actions).
    pub fn is_supported(&self) -> bool {
        true
    }
}
