//! View-models (what a screen displays) and UI events (what it reports).
//!
//! The screens know nothing about saves, the VM or the audio engine: the
//! integration fills these plain structs and reacts to [`UiEvent`]s.

use orbit_render2d::TextureId;

use crate::input::UiSound;

/// Range the text-speed preference slider covers, in characters per second.
pub const TEXT_SPEED_RANGE: [f32; 2] = [10.0, 120.0];
/// Range the auto-mode delay slider covers, in seconds.
pub const AUTO_DELAY_RANGE: [f32; 2] = [0.5, 6.0];
/// Slots per page of the save/load grid (3 columns x 2 rows).
pub const SLOTS_PER_PAGE: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioBus {
    Master,
    Music,
    Sfx,
    Voice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SkipMode {
    /// Skip only text already read.
    #[default]
    ReadOnly,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotCategory {
    Manual,
    Auto,
    Quick,
}

/// A save slot: category and index inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotId {
    pub category: SlotCategory,
    pub index: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Empty,
    Used,
    /// A save the current game version cannot load.
    Incompatible,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlotView {
    pub id: SlotId,
    pub state: SlotState,
    pub thumbnail: Option<TextureId>,
    /// Display string (the host formats it: date and time).
    pub timestamp: String,
    /// Last line spoken, as plain text.
    pub preview: String,
}

impl SlotView {
    pub fn empty(id: SlotId) -> Self {
        Self {
            id,
            state: SlotState::Empty,
            thumbnail: None,
            timestamp: String::new(),
            preview: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CategoryView {
    pub category: SlotCategory,
    pub slots: Vec<SlotView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveLoadMode {
    Save,
    Load,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaveLoadView {
    pub mode: SaveLoadMode,
    /// Save mode only shows the manual category; load mode shows a tab per
    /// category given here.
    pub categories: Vec<CategoryView>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MainMenuView {
    pub title: String,
    pub subtitle: Option<String>,
    pub background: Option<TextureId>,
    /// Show "Continue" (an autosave / last save exists).
    pub can_continue: bool,
    /// Hide "Quit" on the web.
    pub show_quit: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PauseView {
    pub can_save: bool,
    pub show_quit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageView {
    pub code: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrefsView {
    /// Volumes in `0..=1`.
    pub master: f32,
    pub music: f32,
    pub sfx: f32,
    pub voice: f32,
    /// Characters per second, inside [`TEXT_SPEED_RANGE`].
    pub text_speed: f32,
    /// Seconds, inside [`AUTO_DELAY_RANGE`].
    pub auto_delay: f32,
    pub skip_mode: SkipMode,
    /// `None` hides the option (unsupported, e.g. on the web).
    pub fullscreen: Option<bool>,
    pub languages: Vec<LanguageView>,
    /// Code of the current language.
    pub language: String,
}

impl Default for PrefsView {
    fn default() -> Self {
        Self {
            master: 1.0,
            music: 0.8,
            sfx: 0.8,
            voice: 1.0,
            text_speed: 40.0,
            auto_delay: 2.0,
            skip_mode: SkipMode::ReadOnly,
            fullscreen: Some(false),
            languages: Vec::new(),
            language: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntryView {
    /// Opaque id handed back in [`UiEvent::RollbackTo`].
    pub id: usize,
    /// Speaker display name and sRGB color; `None` for narration.
    pub speaker: Option<(String, [u8; 3])>,
    /// Plain text (braces are shown literally).
    pub text: String,
    /// Show the "Rewind here" button.
    pub can_rollback: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct HistoryView {
    /// Oldest first; the list opens scrolled to the latest line.
    pub entries: Vec<HistoryEntryView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct QuickMenuView {
    pub can_rollback: bool,
    pub can_quick_load: bool,
    pub skip_active: bool,
    pub auto_active: bool,
}

/// What a screen asks the runtime to do.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    StartGame,
    Continue,
    OpenLoad,
    OpenSave,
    OpenPreferences,
    OpenHistory,
    OpenPause,
    Resume,
    ReturnToMainMenu,
    Quit,
    /// Close the current screen and return to the previous one.
    Back,
    /// Rewind one step ("Back" of the quick menu).
    Rollback,
    /// Rewind to a history entry.
    RollbackTo(usize),
    ToggleSkip,
    ToggleAuto,
    QuickSave,
    QuickLoad,
    LoadSlot(SlotId),
    SaveSlot(SlotId),
    DeleteSlot(SlotId),
    SetVolume(AudioBus, f32),
    /// Characters per second.
    SetTextSpeed(f32),
    /// Seconds.
    SetAutoDelay(f32),
    SetSkipMode(SkipMode),
    SetFullscreen(bool),
    /// Language code.
    SetLanguage(String),
    ResetPreferences,
    /// Open a screen by id above the current one (data-driven screens).
    OpenScreen(String),
    /// Replace the top screen by another one.
    ReplaceScreen(String),
    /// Close every screen (resume the game).
    CloseAll,
    /// Play a sound asset by path.
    PlaySound(String),
    /// Free-form hook for the host or a plugin.
    Custom { name: String, args: Vec<String> },
    /// Apply a no-code variable operation to the running game.
    VarOp(orbit_core::VarOp),
    /// Show a short notification (already localized).
    Toast(String),
    /// Play a UI sound (see `Theme::sounds`).
    Sound(UiSound),
}
