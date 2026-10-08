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
    /// Date and time alone (`timestamp` without the chapter); empty when the
    /// host gives none.
    pub date: String,
    /// Time played, formatted by the host (empty when unknown).
    pub play_time: String,
    /// Name of the scene (chapter) the save was made in.
    pub chapter: String,
}

impl SlotView {
    pub fn empty(id: SlotId) -> Self {
        Self {
            id,
            state: SlotState::Empty,
            thumbnail: None,
            timestamp: String::new(),
            preview: String::new(),
            date: String::new(),
            play_time: String::new(),
            chapter: String::new(),
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
    /// Screen reader support (`None` hides the option).
    pub screen_reader: Option<bool>,
    /// Text scale inside [`orbit_ui_data::TEXT_SCALE_RANGE`] (M7-07).
    pub text_scale: f32,
    pub high_contrast: bool,
    pub reduced_motion: bool,
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
            screen_reader: Some(true),
            text_scale: 1.0,
            high_contrast: false,
            reduced_motion: false,
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
    /// The line has a voice (M7-06): show the "Voice" button, which sends
    /// [`UiEvent::ReplayVoiceAt`] with the index of the entry.
    pub has_voice: bool,
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
    /// The line on screen has a voice (M7-06): show the replay button.
    pub can_replay_voice: bool,
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
    /// Close the current screen handing a value to the story (`UiAction::CloseWith`):
    /// a screen the story opened stores it in its result variable, any other
    /// screen just closes (like `Back`).
    BackWith(orbit_core::Value),
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
    /// Screen reader support on / off (ADR 0017).
    SetScreenReader(bool),
    /// Text scale (M7-07), inside `TEXT_SCALE_RANGE`.
    SetTextScale(f32),
    /// High-contrast theme on / off (M7-07).
    SetHighContrast(bool),
    /// Reduced motion on / off (M7-07).
    SetReducedMotion(bool),
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
    /// Play a music file of the project in a loop (`UiAction::PlayMusic`).
    PlayMusic(String),
    /// Fade the music out (`UiAction::StopMusic`).
    StopMusic,
    /// Free-form hook for the host or a plugin.
    Custom { name: String, args: Vec<String> },
    /// Apply a no-code variable operation to the running game.
    VarOp(orbit_core::VarOp),
    /// Jump the running game to a scene (scene id, or name), see
    /// `UiAction::SceneGoto`.
    SceneGoto(String),
    /// Replay a seen scene (scene id, or name) in replay mode, see
    /// `UiAction::ReplayScene` (M7-04).
    ReplayScene(String),
    /// Play the voice of the line on screen again (M7-06).
    ReplayVoice,
    /// Play the voice of the history entry at this index again (M7-06).
    ReplayVoiceAt(usize),
    /// Show a short notification (already localized).
    Toast(String),
    /// Play a UI sound (see `Theme::sounds`).
    Sound(UiSound),
}
