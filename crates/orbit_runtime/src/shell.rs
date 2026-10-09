//! The shell: the state machine above a playthrough.
//!
//! Main menu, game, and the stack of screens over them (pause, save, load,
//! preferences, history), plus everything that persists: saves and autosave,
//! preferences, persistent variables, language. It owns the in-game UI
//! screens and speaks to the outside world only through plain data:
//! [`InputAction`]s in, [`Effect`]s out. No GPU is touched here (drawing only
//! appends sprites to a scene), so the whole flow is unit tested.
//!
//! Screens (`stack` over the base):
//!
//! ```text
//! no game:  MainMenu            [+ Load | Preferences]
//! game:     gameplay + quick menu  [+ Pause -> Save | Load | Preferences | History]
//! ```
//!
//! Escape / right click goes back one screen; on the bare game view it opens
//! the pause menu.

use std::collections::HashMap;
use std::sync::Arc;

use orbit_core::{Story, Value, VarOp};
use orbit_i18n::Localizer;
use orbit_render2d::{Layer, Sprite2d, TextureId};
use orbit_save::{
    CaptureInfo, PersistentData, Preferences, SaveData, SaveError, SaveManager, SlotId as SaveSlot,
    SlotState as SaveSlotState, Storage,
};
use orbit_ui::{
    AUTO_DELAY_RANGE, AudioBus, Banner, CategoryView, DataContext, DataScreen, DrawCtx, HistoryEntryView, HistoryScreen,
    HistoryView, LanguageView, MainMenu, MainMenuView, MenuInput, PauseMenu, PauseView,
    PreferencesScreen, PrefsView, QuickMenu, QuickMenuView, SaveLoadMode, SaveLoadScreen,
    SaveLoadView, Screen, SkipMode as UiSkipMode, SlotCategory, SlotId as UiSlot,
    SlotState as UiSlotState, SlotView, TEXT_SPEED_RANGE, TextTable, Texts, Theme, ToastStack,
    UiBundle, UiEvent, UiInput, UiSound, apply_screen_fx, tint, transition_shown,
};
use orbit_ui::a11y::{
    A11yAction, A11yNode, A11yRole, A11yTarget, A11yTree, Announcer, CHOICE_BASE, CHOICES_ID,
    LINE_ID, ROOT_ID, TOAST_ID,
};
use orbit_vm::{OpenScreenState, Vm};

use crate::director::{AudioCommand, Director, Flow};
use crate::format::{format_timestamp, language_name, plain_text};
use crate::game::{FlowKind, Game, InputOutcome, PlaySettings};
use crate::input::{Hotkey, InputAction};
use crate::stage::{CANVAS, StageState};

/// Autosave rules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutosaveConfig {
    pub enabled: bool,
    /// Autosave when the story enters a new scene.
    pub on_scene_start: bool,
    /// Also autosave every this many lines or choices (`None` = never).
    pub every_lines: Option<u32>,
    /// Minimum seconds between two autosaves that are not scene starts (so
    /// that skipping does not write a save every second).
    pub min_interval: f32,
}

impl Default for AutosaveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_scene_start: true,
            every_lines: Some(20),
            min_interval: 5.0,
        }
    }
}

/// Fade of the music when leaving the game or replacing it by a loaded one.
const LEAVE_MUSIC_FADE: f32 = 0.4;
/// Preferences are written this long after the last change (a slider drag
/// changes them every frame).
const PREFS_SAVE_DELAY: f32 = 0.6;
/// A save waits at most this long for its thumbnail.
const CAPTURE_TIMEOUT: f32 = 4.0;

/// Static settings of a shell.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShellConfig {
    pub save: orbit_save::SaveConfig,
    pub autosave: AutosaveConfig,
    /// Per language (primary subtag), UI text keys replacing the built-in
    /// table.
    pub ui_overrides: HashMap<String, Vec<(String, String)>>,
    /// BCP 47 tag of the system language, if known.
    pub system_language: Option<String>,
    /// Language of the game for a player without a saved preference (it
    /// takes precedence over the system language).
    pub default_language: Option<String>,
    /// Minutes east of UTC.
    pub utc_offset_minutes: i32,
    pub fullscreen_supported: bool,
    pub quit_supported: bool,
    /// Take a thumbnail with each save (needs a GPU: the runtime answers the
    /// capture requests).
    pub capture_thumbnails: bool,
    /// Hosted by the editor preview: no main menu (leaving the game starts
    /// it again), skip goes through everything, nothing persists between
    /// playthroughs.
    pub embedded: bool,
}

/// A screen over the base view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Overlay {
    Pause,
    Save,
    Load,
    Prefs,
    History,
    /// A data-driven screen (index into the shell's screens).
    Custom(usize),
}

/// A screen the story opened (`OpenScreen`, UI-14), built from the VM state.
pub(crate) struct StoryScreen {
    pub state: OpenScreenState,
    pub screen: DataScreen,
    /// Seconds since it opened (its opening transition).
    pub elapsed: f32,
}

/// A screen that closed and plays its closing transition (UI-16).
struct Ghost {
    screen: GhostScreen,
    elapsed: f32,
}

enum GhostScreen {
    /// A story screen (its own instance).
    Story(Box<DataScreen>),
    /// A data screen of the stack (index into `Shell::data`).
    Data(usize),
}

/// What the shell asks the runtime to do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Effect {
    Audio(AudioCommand),
    UiSound(UiSound),
    SetVolume(AudioBus, f32),
    SetFullscreen(bool),
    /// Play a sound asset by path (a `PlaySound` action of a data screen).
    PlayAsset(String),
    /// A `Custom` action of a data screen, for the host or a plugin.
    Custom { name: String, args: Vec<String> },
    Quit,
}

/// A thumbnail the runtime must render (the stage as it was when the player
/// saved) and answer with [`Shell::complete_capture`].
#[derive(Clone, Debug)]
pub(crate) struct CaptureRequest {
    pub id: u64,
    pub stage: StageState,
}

/// A PNG found in a slot, to turn into a texture.
#[derive(Clone, Debug)]
pub(crate) struct ThumbnailRequest {
    pub key: ThumbKey,
    pub png: Arc<Vec<u8>>,
}

/// Identifies a thumbnail: the slot and the save's creation time.
pub(crate) type ThumbKey = (UiSlot, u64);

/// Screen reader slots (see [`orbit_ui::a11y::widget_id`]): the quick menu,
/// the main menu, then the screen stack from `SLOT_STACK` up.
const SLOT_QUICK: u32 = 1;
const SLOT_MAIN: u32 = 2;
const SLOT_STACK: u32 = 3;

enum JobStage {
    NeedsCapture,
    Capturing,
    Done(Option<Vec<u8>>),
}

/// A save whose state is frozen but whose thumbnail is still on its way.
struct SaveJob {
    id: u64,
    slot: SaveSlot,
    data: Option<SaveData>,
    stage: StageState,
    notify: bool,
    age: f32,
    state: JobStage,
}

enum Thumb {
    Requested,
    Ready(TextureId),
    Failed,
}

/// One slot of the lists, as read from the storage.
struct SlotEntry {
    id: UiSlot,
    state: EntryState,
}

enum EntryState {
    Empty,
    Occupied {
        loadable: bool,
        created_at_ms: u64,
        label: String,
        preview: String,
        png: Option<Arc<Vec<u8>>>,
    },
    Corrupt,
}

/// Id of the data screen a scene replay comes back to (M7-04).
pub const REPLAY_SCREEN: &str = "replay";

/// The images of the gallery when a `Gallery` source lists none (M7-03):
/// every image the story shows as a background, in story order, by path.
pub fn gallery_paths(story: &Story) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for node in story.scenes.iter().flat_map(|s| &s.nodes) {
        if let orbit_core::NodeKind::Background { asset, .. } = &node.kind
            && let Some(entry) = story.assets.get(*asset)
            && !out.contains(&entry.path)
        {
            out.push(entry.path.clone());
        }
    }
    out
}

fn to_save_slot(id: UiSlot) -> SaveSlot {
    match id.category {
        SlotCategory::Manual => SaveSlot::Manual(id.index + 1),
        SlotCategory::Auto => SaveSlot::Auto(id.index),
        SlotCategory::Quick => SaveSlot::Quick,
    }
}

fn to_ui_slot(slot: SaveSlot) -> UiSlot {
    match slot {
        SaveSlot::Manual(n) => UiSlot {
            category: SlotCategory::Manual,
            index: n.saturating_sub(1),
        },
        SaveSlot::Auto(n) => UiSlot {
            category: SlotCategory::Auto,
            index: n,
        },
        SaveSlot::Quick => UiSlot {
            category: SlotCategory::Quick,
            index: 0,
        },
    }
}

fn to_ui_skip(mode: orbit_save::SkipMode) -> UiSkipMode {
    match mode {
        orbit_save::SkipMode::ReadOnly => UiSkipMode::ReadOnly,
        orbit_save::SkipMode::All => UiSkipMode::All,
    }
}

fn from_ui_skip(mode: UiSkipMode) -> orbit_save::SkipMode {
    match mode {
        UiSkipMode::ReadOnly => orbit_save::SkipMode::ReadOnly,
        UiSkipMode::All => orbit_save::SkipMode::All,
    }
}

pub(crate) struct Shell {
    story: Arc<Story>,
    /// Runtime plugins (M8-04), attached to every VM the shell creates.
    plugins: Option<orbit_vm::SharedPluginHost>,
    /// Script engine (M8-01), attached to every VM the shell creates.
    scripts: Option<orbit_vm::SharedScriptHost>,
    /// Platform hook told of the achievements (M7-05).
    achievement_hook: Option<crate::achievements::SharedAchievementHook>,
    /// The theme as the project wrote it.
    base_theme: Theme,
    /// What the shell draws with: `base_theme` with the text scale and the
    /// high-contrast preferences applied (M7-07).
    theme: Theme,
    cfg: ShellConfig,
    saves: SaveManager,
    loc: Arc<Localizer>,
    texts: Texts,
    prefs: Preferences,
    prefs_timer: Option<f32>,
    persistent: PersistentData,

    main: MainMenu,
    pause: PauseMenu,
    save_screen: SaveLoadScreen,
    load_screen: SaveLoadScreen,
    prefs_screen: PreferencesScreen,
    history: HistoryScreen,
    quick: QuickMenu,
    toasts: ToastStack,
    /// Data-driven screens (see [`Shell::set_ui_bundle`]).
    data: Vec<DataScreen>,
    /// Data screens standing in for the stock ones.
    sys_main: Option<usize>,
    sys_pause: Option<usize>,
    sys_prefs: Option<usize>,
    sys_save: Option<usize>,
    sys_load: Option<usize>,
    /// A data screen has a save slot list: the slots go into its context.
    slots_wanted: bool,
    end_banner: Option<Banner>,
    /// The tooltip of a hotspot map, laid out (with its text).
    hotspot_tip: Option<(String, Banner)>,
    menu_background: Option<TextureId>,
    /// Textures of the images the theme refers to (nine-slice fills).
    ui_images: std::collections::HashMap<String, TextureId>,

    stack: Vec<Overlay>,
    game: Option<Game>,
    failure: Option<String>,
    play_time: f32,
    now_ms: u64,
    can_continue: bool,
    quick_exists: bool,
    end_handled: bool,

    last_progress: u64,
    last_scene: Option<usize>,
    lines_since_autosave: u32,
    autosave_cooldown: f32,

    jobs: Vec<SaveJob>,
    next_job: u64,
    slots: Vec<SlotEntry>,
    thumbs: HashMap<ThumbKey, Thumb>,
    stale_textures: Vec<TextureId>,
    views_dirty: bool,

    effects: Vec<Effect>,

    /// Screen reader: the live region texts of the dialogue line and of the
    /// last toast (see [`orbit_ui::a11y`]).
    announcer: Announcer,
    last_toast: Option<(u64, String)>,
    toast_announcer: Announcer,

    /// Screens the story opened, in opening order (rebuilt from the VM).
    story_screens: Vec<StoryScreen>,
    /// The HUD that took the current press, to give it the release.
    hud_pointer: Option<usize>,
    /// Closing transitions still playing.
    ghosts: Vec<Ghost>,
    /// Data screens of the stack and the seconds since they opened.
    stack_opened: HashMap<usize, f32>,
    /// The next screens built from the VM appear without transition (a
    /// rollback, a load).
    cut_screen_fx: bool,
}

pub(crate) fn make_texts(language: &str, overrides: &HashMap<String, Vec<(String, String)>>) -> Texts {
    let mut table = TextTable::builtin(language);
    let primary = language
        .split(['-', '_'])
        .next()
        .unwrap_or(language)
        .to_ascii_lowercase();
    if let Some(pairs) = overrides.get(&primary) {
        for (key, text) in pairs {
            table.insert(key.clone(), text.clone());
        }
    }
    Texts::new(Arc::new(table))
}

impl Shell {
    /// Builds the shell for `story`: loads preferences and persistent data
    /// from `storage` (damaged files fall back to defaults), picks the
    /// language and creates the screens, the main menu on top.
    pub fn new(
        story: Arc<Story>,
        storage: Box<dyn Storage>,
        mut loc: Localizer,
        theme: Theme,
        cfg: ShellConfig,
    ) -> Self {
        let saves = SaveManager::new(storage, story.clone(), cfg.save);
        let mut prefs = saves.load_prefs().unwrap_or_else(|err| {
            log::warn!("preferences unreadable, using defaults: {err}");
            Preferences::default()
        });
        if cfg.embedded {
            // Nothing is "already read" in a fresh preview: skip everything.
            prefs.skip_mode = orbit_save::SkipMode::All;
        }
        let persistent = saves.load_persistent().unwrap_or_else(|err| {
            log::warn!("persistent data unreadable, starting empty: {err}");
            PersistentData::default()
        });
        Self::pick_language(&mut loc, &prefs, &cfg);
        let texts = make_texts(loc.language(), &cfg.ui_overrides);
        let loc = Arc::new(loc);
        let base_theme = theme;
        let theme = base_theme.accessible(prefs.text_scale, prefs.high_contrast);

        let mut shell = Self {
            main: MainMenu::new(
                &theme,
                &texts,
                MainMenuView {
                    title: story.title.clone(),
                    subtitle: None,
                    background: None,
                    can_continue: false,
                    show_quit: cfg.quit_supported,
                },
            ),
            pause: PauseMenu::new(
                &theme,
                &texts,
                PauseView {
                    can_save: false,
                    show_quit: cfg.quit_supported,
                },
            ),
            save_screen: SaveLoadScreen::new(
                &theme,
                &texts,
                SaveLoadView {
                    mode: SaveLoadMode::Save,
                    categories: Vec::new(),
                },
            ),
            load_screen: SaveLoadScreen::new(
                &theme,
                &texts,
                SaveLoadView {
                    mode: SaveLoadMode::Load,
                    categories: Vec::new(),
                },
            ),
            prefs_screen: PreferencesScreen::new(&theme, &texts, PrefsView::default()),
            history: HistoryScreen::new(&theme, &texts, HistoryView::default()),
            quick: QuickMenu::new(&theme, &texts, QuickMenuView::default()),
            toasts: ToastStack::new(&theme),
            data: Vec::new(),
            sys_main: None,
            sys_pause: None,
            sys_prefs: None,
            sys_save: None,
            sys_load: None,
            slots_wanted: false,
            story,
            base_theme,
            theme,
            cfg,
            saves,
            loc,
            texts,
            prefs,
            prefs_timer: None,
            persistent,
            end_banner: None,
            hotspot_tip: None,
            menu_background: None,
            ui_images: std::collections::HashMap::new(),
            stack: Vec::new(),
            game: None,
            failure: None,
            play_time: 0.0,
            now_ms: 0,
            can_continue: false,
            quick_exists: false,
            end_handled: false,
            last_progress: 0,
            last_scene: None,
            lines_since_autosave: 0,
            autosave_cooldown: 0.0,
            jobs: Vec::new(),
            next_job: 0,
            slots: Vec::new(),
            thumbs: HashMap::new(),
            stale_textures: Vec::new(),
            views_dirty: false,
            effects: Vec::new(),
            announcer: Announcer::default(),
            last_toast: None,
            toast_announcer: Announcer::default(),
            plugins: None,
            scripts: None,
            achievement_hook: None,
            story_screens: Vec::new(),
            hud_pointer: None,
            ghosts: Vec::new(),
            stack_opened: HashMap::new(),
            cut_screen_fx: false,
        };
        shell.apply_prefs_effects();
        shell.refresh_slots();
        shell.refresh_main_menu();
        shell
    }

    /// Initial language: the saved preference, else the game's default
    /// language, else the system language, else the story's source language.
    fn pick_language(loc: &mut Localizer, prefs: &Preferences, cfg: &ShellConfig) {
        let chosen = prefs
            .language
            .as_deref()
            .and_then(|l| loc.resolve_language(l))
            .or_else(|| {
                cfg.default_language
                    .as_deref()
                    .and_then(|l| loc.resolve_language(l))
            })
            .or_else(|| {
                cfg.system_language
                    .as_deref()
                    .and_then(|l| loc.resolve_language(l))
            });
        if let Some(tag) = chosen {
            loc.set_language(&tag);
        }
    }

    // ------------------------------------------------------------ accessors

    pub fn texts(&self) -> &Texts {
        &self.texts
    }

    pub fn story(&self) -> &Arc<Story> {
        &self.story
    }

    pub fn language(&self) -> &str {
        self.loc.language()
    }

    pub fn prefs(&self) -> &Preferences {
        &self.prefs
    }

    #[cfg(test)]
    pub fn saves(&self) -> &SaveManager {
        &self.saves
    }

    #[cfg(test)]
    pub fn has_game(&self) -> bool {
        self.game.is_some()
    }

    pub fn game(&self) -> Option<&Game> {
        self.game.as_ref()
    }

    pub fn game_mut(&mut self) -> Option<&mut Game> {
        self.game.as_mut()
    }

    pub fn top(&self) -> Option<Overlay> {
        self.stack.last().copied()
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    #[cfg(test)]
    pub fn can_continue(&self) -> bool {
        self.can_continue
    }

    /// Wall clock, set by the runtime before each update (the shell never
    /// reads a clock).
    pub fn set_now(&mut self, unix_ms: u64) {
        self.now_ms = unix_ms;
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Image behind the main menu.
    pub fn set_menu_background(&mut self, texture: Option<TextureId>) {
        self.menu_background = texture;
        self.refresh_main_menu();
    }

    /// Textures (by asset path) of the images the theme's nine-slice fills
    /// use; the data-driven screens draw with them.
    pub fn set_ui_images(&mut self, images: std::collections::HashMap<String, TextureId>) {
        if images != self.ui_images {
            self.ui_images = images;
            self.quick.set_images(self.ui_images.clone());
            self.sync_data();
        }
    }

    fn toast(&mut self, text: String) {
        let seq = self.last_toast.as_ref().map_or(0, |(s, _)| s + 1);
        self.last_toast = Some((seq, orbit_ui::a11y::plain(&text)));
        self.toasts.push(text);
    }

    fn toast_key(&mut self, key: &str) {
        let text = self.texts.get(key);
        self.toast(text);
    }

    /// The storage fell back to memory: saves will not survive the session.
    pub fn warn_storage_unavailable(&mut self) {
        self.toast_key("ui.toast.storage_unavailable");
    }

    // ------------------------------------------------------------ game flow

    fn play_settings(&self) -> PlaySettings {
        PlaySettings {
            text_speed: self.prefs.text_speed,
            auto_delay: self.prefs.auto_delay,
            skip_all: self.prefs.skip_mode == orbit_save::SkipMode::All,
            reduced_motion: self.prefs.reduced_motion,
        }
    }

    /// A VM at the start of the story with the persistent data applied.
    fn fresh_vm(&self) -> Result<Vm, String> {
        let mut vm = Vm::from_start(self.story.clone()).map_err(|e| e.to_string())?;
        vm.set_plugin_host(self.plugins.clone());
        vm.set_script_host(self.scripts.clone());
        self.persistent.apply_to(&mut vm);
        Ok(vm)
    }

    fn install_game(&mut self, director: Director) {
        let settings = self.play_settings();
        self.game = Some(Game::new(
            self.story.clone(),
            director,
            &self.theme,
            settings,
        ));
        self.stack.clear();
        self.end_handled = false;
        self.autosave_cooldown = 0.0;
        self.lines_since_autosave = 0;
        self.pump_audio();
        self.sync_pause_view();
    }

    /// Starts the story from its beginning.
    pub fn start_new_game(&mut self) {
        self.leave_music();
        let vm = match self.fresh_vm() {
            Ok(vm) => vm,
            Err(message) => {
                self.failure = Some(message);
                return;
            }
        };
        let director = Director::with_localizer(vm, self.loc.clone());
        self.install_game(director);
        self.play_time = 0.0;
        // The first line starts the first scene: autosave there.
        self.last_progress = 0;
        self.last_scene = None;
    }

    /// Continues from `vm` (a hot reload, or "play from here"): the stage and
    /// the music are rebuilt from its state without any transition, then the
    /// story runs on. `instant` shows the line already on screen whole
    /// instead of typing it out again.
    pub fn install_vm(&mut self, vm: Vm, instant: bool) {
        self.leave_music();
        self.cut_screen_fx = true;
        let director = Director::resumed(vm, self.loc.clone());
        self.install_game(director);
        self.play_time = 0.0;
        self.last_progress = self.game.as_ref().map_or(0, |g| g.director.progress());
        self.last_scene = self.current_scene();
        if instant && let Some(game) = &mut self.game {
            game.dialogue.advance();
        }
    }

    /// Turns the skip mode on or off from the outside (the preview toolbar).
    pub fn set_skip(&mut self, on: bool) {
        if let Some(game) = &mut self.game {
            game.set_skip(on);
        }
    }

    /// Picks option `index` of the choice on screen, as a click would.
    pub fn choose_option(&mut self, index: usize) -> bool {
        let theme = self.theme.clone();
        let done = self
            .game
            .as_mut()
            .is_some_and(|game| game.choose_index(index, &theme));
        self.after_input();
        done
    }

    /// The node of the line or choice on screen.
    pub fn shown_node(&self) -> Option<orbit_core::NodeId> {
        self.game
            .as_ref()
            .and_then(|g| g.director.shown())
            .map(orbit_vm::Interaction::node)
    }

    /// Whether the picture changes with time alone (typewriter, pauses,
    /// transitions, skip / auto, toasts): the host must keep drawing frames.
    /// `false` means a still image until the next input.
    pub fn is_animating(&self) -> bool {
        if !self.toasts.is_empty() || !self.ghosts.is_empty() {
            return true;
        }
        // An opening transition still playing.
        let opening = |asset: &orbit_ui::ScreenAsset, elapsed: f32| {
            asset
                .transition_in
                .as_ref()
                .is_some_and(|t| !self.prefs.reduced_motion && elapsed < t.seconds())
        };
        if self
            .story_screens
            .iter()
            .any(|s| opening(s.screen.asset(), s.elapsed))
            || self
                .stack_opened
                .iter()
                .any(|(i, e)| self.data.get(*i).is_some_and(|d| opening(d.asset(), *e)))
        {
            return true;
        }
        if self.top().is_some() {
            return false;
        }
        self.game.as_ref().is_some_and(Game::is_animating)
    }

    fn leave_music(&mut self) {
        if self.game.is_some() {
            self.effects.push(Effect::Audio(AudioCommand::StopMusic {
                fade_out: LEAVE_MUSIC_FADE,
            }));
        }
    }

    /// Loads a slot into a fresh playthrough. Failures become toasts and
    /// leave the current state alone.
    fn load_slot(&mut self, slot: SaveSlot) -> bool {
        let loaded = match self.saves.load(slot) {
            Ok(loaded) => loaded,
            Err(err) => {
                self.report_load_error(&err);
                return false;
            }
        };
        let mut vm = match self.fresh_vm() {
            Ok(vm) => vm,
            Err(message) => {
                log::error!("cannot start a playthrough to load {slot}: {message}");
                self.toast_key("ui.toast.load_failed");
                return false;
            }
        };
        // Persistent data first: the save only holds playthrough variables.
        if let Err(err) = loaded.data.apply_to(&mut vm) {
            log::warn!("{slot} cannot be applied: {err}");
            self.toast_key("ui.toast.incompatible");
            return false;
        }
        self.leave_music();
        self.cut_screen_fx = true;
        let director = Director::resumed(vm, self.loc.clone());
        self.install_game(director);
        self.play_time = loaded.data.meta.play_time_secs as f32;
        // Loading must not autosave the line it lands on.
        self.last_progress = self.game.as_ref().map_or(0, |g| g.director.progress());
        self.last_scene = self.current_scene();
        self.toast_key("ui.toast.loaded");
        true
    }

    fn report_load_error(&mut self, err: &SaveError) {
        log::warn!("load failed: {err}");
        match err {
            SaveError::Corrupt(_) => self.toast_key("ui.toast.corrupt"),
            SaveError::Incompatible(_) => self.toast_key("ui.toast.incompatible"),
            SaveError::NotFound(_) => self.toast_key("ui.toast.nothing_to_load"),
            _ => self.toast_key("ui.toast.load_failed"),
        }
    }

    /// "Continue": the most recent loadable save.
    fn continue_game(&mut self) {
        match self.saves.latest() {
            Ok(Some((slot, _))) => {
                self.load_slot(slot);
            }
            Ok(None) => self.toast_key("ui.toast.nothing_to_load"),
            Err(err) => self.report_load_error(&err),
        }
    }

    /// Back to the main menu (game over, or the player's choice). The editor
    /// preview has no main menu: the story starts again.
    fn leave_game(&mut self) {
        let replay = self.replaying();
        if self.cfg.embedded {
            self.start_new_game();
            return;
        }
        self.flush_persistent();
        self.leave_music();
        self.game = None;
        self.cut_screen_fx = true;
        self.sync_story_screens();
        self.stack.clear();
        self.refresh_slots();
        self.refresh_main_menu();
        if replay {
            self.open_screen(REPLAY_SCREEN);
        }
    }

    /// Saves what must survive the process: persistent data, preferences, and
    /// a last autosave of a running game. Call when the window closes.
    pub fn on_exit(&mut self) {
        if self.game.is_some()
            && self.can_save()
            && let Some(data) = self.capture(SaveSlot::Auto(0), false).and_then(|j| j.data)
        {
            self.write_job(data, SaveSlot::Auto(0), false, None);
        }
        self.flush_persistent();
        self.flush_prefs();
    }

    /// Index (in `story.scenes`) of the scene of the line or choice on
    /// screen.
    pub(crate) fn current_scene(&self) -> Option<usize> {
        let node = self.game.as_ref()?.director.shown()?.node();
        self.story.locate(node).map(|(scene, _)| scene)
    }

    // ---------------------------------------------------- gallery and replay

    /// Whether the running game replays a scene (M7-04): nothing is saved,
    /// autosaved or written to the persistent data.
    pub fn replaying(&self) -> bool {
        self.game.as_ref().is_some_and(|g| g.director.vm().is_replay())
    }

    /// Asset paths of the images unlocked so far (M7-03): the saved ones and,
    /// during a playthrough, the ones it showed.
    fn unlocked_images(&self) -> std::collections::BTreeSet<String> {
        let mut out = self.persistent.unlocked_images.clone();
        if let Some(game) = &self.game
            && !game.director.vm().is_replay()
        {
            let vm = game.director.vm();
            out.extend(
                vm.shown_images()
                    .iter()
                    .filter_map(|id| self.story.assets.get(*id).map(|a| a.path.clone())),
            );
        }
        out
    }

    /// The achievements unlocked so far (M7-05): the saved ones and, during
    /// a playthrough, the ones it unlocked.
    fn unlocked_achievements(&self) -> std::collections::BTreeSet<orbit_core::AchievementId> {
        let mut out = self.persistent.unlocked_achievements.clone();
        if let Some(game) = &self.game
            && !game.director.vm().is_replay()
        {
            out.extend(game.director.vm().unlocked_achievements().iter().copied());
        }
        out
    }

    fn achievement_rows(&self) -> Vec<orbit_ui::AchievementRow> {
        crate::achievements::rows(&self.story, &self.loc, &self.texts, &self.unlocked_achievements())
    }

    /// Whether the scene was entered in a playthrough (saved, or the
    /// running one).
    fn scene_seen(&self, id: orbit_core::SceneId) -> bool {
        self.persistent.has_seen(id)
            || self.game.as_ref().is_some_and(|g| {
                let vm = g.director.vm();
                !vm.is_replay() && vm.scene_visits().contains_key(&id)
            })
    }

    /// Every scene in story order as (id, name, seen), for the replay menu.
    fn replay_scenes(&self) -> Vec<(String, String, bool)> {
        self.story
            .scenes
            .iter()
            .map(|s| (s.id.to_string(), s.name.clone(), self.scene_seen(s.id)))
            .collect()
    }

    /// `UiAction::ReplayScene` (M7-04): replays a seen scene (id or name)
    /// from the title, on the variables it was first entered with (declared
    /// initial values for those not recorded) and a copy of the persistent
    /// ones. Nothing of the replay is saved; its end or the menu comes back
    /// to the title with the `replay` screen. Refused (with a log line)
    /// during a playthrough, which it would end, and for unseen scenes.
    pub fn start_replay(&mut self, scene: &str) -> bool {
        if self.game.is_some() && !self.replaying() {
            log::warn!("scene replay of `{scene}` ignored: a game is running");
            return false;
        }
        let key = scene.trim();
        let found = self
            .story
            .scenes
            .iter()
            .find(|s| s.id.to_string() == key)
            .or_else(|| self.story.scenes.iter().find(|s| s.name == key))
            .map(|s| s.id);
        let Some(id) = found.filter(|id| self.scene_seen(*id)) else {
            log::warn!("scene replay of `{scene}` ignored: not a seen scene");
            return false;
        };
        let vars = self.persistent.seen_scenes.get(&id);
        let mut vm = match Vm::replay(self.story.clone(), id, vars, &self.persistent.vars) {
            Ok(vm) => vm,
            Err(err) => {
                log::warn!("scene replay of `{scene}` failed: {err}");
                return false;
            }
        };
        vm.set_plugin_host(self.plugins.clone());
        vm.set_script_host(self.scripts.clone());
        // Lines read in a playthrough can be skipped in the replay.
        vm.set_read_nodes(self.persistent.read_nodes.clone());
        self.leave_music();
        self.cut_screen_fx = true;
        let director = Director::with_localizer(vm, self.loc.clone());
        self.install_game(director);
        self.play_time = 0.0;
        self.last_progress = 0;
        self.last_scene = None;
        true
    }

    /// Persistent variables and read lines of the running game, to disk.
    fn flush_persistent(&mut self) {
        if self.cfg.embedded || self.replaying() {
            return;
        }
        if let Some(game) = &self.game {
            self.persistent = PersistentData::from_vm(game.director.vm());
        }
        if let Err(err) = self.saves.save_persistent(&self.persistent) {
            log::warn!("cannot save persistent data: {err}");
        }
    }

    fn flush_prefs(&mut self) {
        self.prefs_timer = None;
        if let Err(err) = self.saves.save_prefs(&self.prefs) {
            log::warn!("cannot save preferences: {err}");
        }
    }

    fn mark_prefs_dirty(&mut self) {
        self.prefs_timer = Some(PREFS_SAVE_DELAY);
    }

    // ----------------------------------------------------------------- saves

    fn can_save(&self) -> bool {
        self.game
            .as_ref()
            .is_some_and(|g| g.director.shown().is_some())
            && !self.replaying()
    }

    /// Freezes the current progress into a save job.
    fn capture(&self, slot: SaveSlot, notify: bool) -> Option<SaveJob> {
        let game = self.game.as_ref()?;
        let vm = game.director.vm();
        let preview = vm
            .backlog()
            .last()
            .map(|item| plain_text(self.loc.text(item.string_id)));
        let info = CaptureInfo {
            preview,
            ..CaptureInfo::new(self.now_ms, self.play_time as u64)
        };
        Some(SaveJob {
            id: 0,
            slot,
            data: Some(SaveData::capture(vm, info)),
            stage: game.director.stage().clone(),
            notify,
            age: 0.0,
            state: JobStage::Done(None),
        })
    }

    /// Saves to `slot`: the state is frozen now, written once the thumbnail
    /// (if any) is ready.
    fn request_save(&mut self, slot: SaveSlot, notify: bool) {
        if !self.can_save() {
            return;
        }
        let Some(mut job) = self.capture(slot, notify) else {
            return;
        };
        job.id = self.next_job;
        self.next_job += 1;
        job.state = if self.cfg.capture_thumbnails {
            JobStage::NeedsCapture
        } else {
            JobStage::Done(None)
        };
        self.jobs.push(job);
        self.finish_jobs();
    }

    /// Thumbnails the runtime has to render now.
    pub fn take_capture_requests(&mut self) -> Vec<CaptureRequest> {
        self.jobs
            .iter_mut()
            .filter(|j| matches!(j.state, JobStage::NeedsCapture))
            .map(|job| {
                job.state = JobStage::Capturing;
                CaptureRequest {
                    id: job.id,
                    stage: job.stage.clone(),
                }
            })
            .collect()
    }

    /// The runtime's answer to a [`CaptureRequest`] (`None` = no thumbnail).
    pub fn complete_capture(&mut self, id: u64, png: Option<Vec<u8>>) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.state = JobStage::Done(png);
        }
        self.finish_jobs();
    }

    /// Writes the jobs that are done (or that waited too long).
    fn finish_jobs(&mut self) {
        let mut jobs = std::mem::take(&mut self.jobs);
        jobs.retain_mut(|job| {
            let thumb = match &mut job.state {
                JobStage::Done(png) => png.take(),
                _ if job.age >= CAPTURE_TIMEOUT => {
                    log::warn!("thumbnail timed out: saving without it");
                    None
                }
                _ => return true,
            };
            if let Some(data) = job.data.take() {
                self.write_job(data, job.slot, job.notify, thumb);
            }
            false
        });
        // `write_job` never adds jobs, but keep anything queued meanwhile.
        jobs.append(&mut self.jobs);
        self.jobs = jobs;
    }

    fn write_job(
        &mut self,
        mut data: SaveData,
        slot: SaveSlot,
        notify: bool,
        thumbnail: Option<Vec<u8>>,
    ) {
        data.meta.thumbnail = thumbnail;
        let result = match slot {
            SaveSlot::Auto(_) => self.saves.autosave(&data),
            SaveSlot::Quick => self.saves.quicksave(&data),
            SaveSlot::Manual(_) => self.saves.save(slot, &data),
        };
        match result {
            Ok(()) => {
                if notify {
                    self.toast_key(if slot == SaveSlot::Quick {
                        "ui.toast.quick_saved"
                    } else {
                        "ui.toast.saved"
                    });
                }
                self.flush_persistent();
                self.refresh_slots();
                self.refresh_main_menu();
            }
            Err(err) => {
                log::error!("saving {slot} failed: {err}");
                self.toast_key("ui.toast.save_failed");
            }
        }
    }

    // ----------------------------------------------------------- slot lists

    /// Re-reads the slots from the storage.
    fn refresh_slots(&mut self) {
        let list = self.saves.list().unwrap_or_else(|err| {
            log::warn!("cannot list the saves: {err}");
            Vec::new()
        });
        self.can_continue = false;
        self.quick_exists = false;
        let mut newest = 0;
        let mut slots = Vec::with_capacity(list.len());
        for info in list {
            let id = to_ui_slot(info.slot);
            let state = match info.state {
                SaveSlotState::Empty => EntryState::Empty,
                SaveSlotState::Corrupt(_) => EntryState::Corrupt,
                SaveSlotState::Occupied { meta, compat } => {
                    let loadable = compat.is_loadable();
                    if loadable {
                        if info.slot == SaveSlot::Quick {
                            self.quick_exists = true;
                        }
                        if !self.can_continue || meta.created_at_ms > newest {
                            newest = meta.created_at_ms;
                        }
                        self.can_continue = true;
                    }
                    EntryState::Occupied {
                        loadable,
                        created_at_ms: meta.created_at_ms,
                        label: meta.label.clone(),
                        preview: plain_text(&meta.preview),
                        png: meta.thumbnail.map(Arc::new),
                    }
                }
            };
            slots.push(SlotEntry { id, state });
        }
        self.slots = slots;
        // Forget the thumbnails of saves that no longer exist.
        let alive: Vec<ThumbKey> = self
            .slots
            .iter()
            .filter_map(|s| match &s.state {
                EntryState::Occupied {
                    created_at_ms,
                    png: Some(_),
                    ..
                } => Some((s.id, *created_at_ms)),
                _ => None,
            })
            .collect();
        let stale: Vec<ThumbKey> = self
            .thumbs
            .keys()
            .filter(|k| !alive.contains(k))
            .copied()
            .collect();
        for key in stale {
            if let Some(Thumb::Ready(texture)) = self.thumbs.remove(&key) {
                self.stale_textures.push(texture);
            }
        }
        self.views_dirty = true;
    }

    /// PNGs that still have to become textures.
    pub fn take_thumbnail_requests(&mut self) -> Vec<ThumbnailRequest> {
        let mut out = Vec::new();
        for slot in &self.slots {
            let EntryState::Occupied {
                created_at_ms,
                png: Some(png),
                ..
            } = &slot.state
            else {
                continue;
            };
            let key = (slot.id, *created_at_ms);
            if let std::collections::hash_map::Entry::Vacant(v) = self.thumbs.entry(key) {
                v.insert(Thumb::Requested);
                out.push(ThumbnailRequest {
                    key,
                    png: png.clone(),
                });
            }
        }
        out
    }

    /// The runtime's answer to a [`ThumbnailRequest`].
    pub fn set_thumbnail(&mut self, key: ThumbKey, texture: Option<TextureId>) {
        if !self.thumbs.contains_key(&key) {
            // The slot changed meanwhile: the texture is already useless.
            self.stale_textures.extend(texture);
            return;
        }
        self.thumbs
            .insert(key, texture.map_or(Thumb::Failed, Thumb::Ready));
        self.views_dirty = true;
    }

    /// Textures of vanished saves, to release.
    pub fn take_stale_textures(&mut self) -> Vec<TextureId> {
        std::mem::take(&mut self.stale_textures)
    }

    fn slot_view(&self, entry: &SlotEntry) -> SlotView {
        match &entry.state {
            EntryState::Empty => SlotView::empty(entry.id),
            EntryState::Corrupt => SlotView {
                id: entry.id,
                state: UiSlotState::Used,
                thumbnail: None,
                timestamp: "\u{2014}".into(),
                preview: self.texts.get("ui.toast.corrupt"),
            },
            EntryState::Occupied {
                loadable,
                created_at_ms,
                label,
                preview,
                ..
            } => {
                let mut timestamp = format_timestamp(*created_at_ms, self.cfg.utc_offset_minutes);
                if !label.is_empty() {
                    timestamp.push_str("  \u{b7}  ");
                    timestamp.push_str(label);
                }
                let thumbnail = match self.thumbs.get(&(entry.id, *created_at_ms)) {
                    Some(Thumb::Ready(texture)) => Some(*texture),
                    _ => None,
                };
                SlotView {
                    id: entry.id,
                    state: if *loadable {
                        UiSlotState::Used
                    } else {
                        UiSlotState::Incompatible
                    },
                    thumbnail,
                    timestamp,
                    preview: preview.clone(),
                }
            }
        }
    }

    fn category_views(&self) -> Vec<CategoryView> {
        [
            SlotCategory::Manual,
            SlotCategory::Auto,
            SlotCategory::Quick,
        ]
        .into_iter()
        .map(|category| CategoryView {
            category,
            slots: self
                .slots
                .iter()
                .filter(|s| s.id.category == category)
                .map(|s| self.slot_view(s))
                .collect(),
        })
        .collect()
    }

    fn rebuild_save_views(&mut self) {
        let categories = self.category_views();
        self.save_screen.set_view(SaveLoadView {
            mode: SaveLoadMode::Save,
            categories: categories.clone(),
        });
        self.load_screen.set_view(SaveLoadView {
            mode: SaveLoadMode::Load,
            categories,
        });
        self.views_dirty = false;
        if self.slots_wanted {
            self.sync_data();
        }
    }

    fn refresh_main_menu(&mut self) {
        self.main.set_view(MainMenuView {
            title: self.story.title.clone(),
            subtitle: None,
            background: self.menu_background,
            can_continue: self.can_continue,
            show_quit: self.cfg.quit_supported,
        });
        self.sync_data();
    }

    fn sync_pause_view(&mut self) {
        self.pause.set_view(PauseView {
            can_save: self.can_save(),
            show_quit: self.cfg.quit_supported,
        });
    }

    // ------------------------------------------------------------- history

    fn history_view(&self) -> HistoryView {
        let Some(game) = &self.game else {
            return HistoryView::default();
        };
        let director = &game.director;
        let entries = director
            .vm()
            .backlog()
            .into_iter()
            .map(|item| HistoryEntryView {
                id: item.checkpoint.unwrap_or(0) as usize,
                speaker: item.speaker.and_then(|id| director.speaker(id)),
                text: plain_text(self.loc.text(item.string_id)),
                can_rollback: item.checkpoint.is_some(),
                has_voice: self.line_voice(item.node, item.string_id).is_some(),
            })
            .collect();
        HistoryView { entries }
    }

    // ---------------------------------------------------------- preferences

    fn prefs_view(&self) -> PrefsView {
        let v = &self.prefs.volumes;
        PrefsView {
            master: v.master,
            music: v.music,
            sfx: v.sfx,
            voice: v.voice,
            text_speed: self
                .prefs
                .text_speed
                .clamp(TEXT_SPEED_RANGE[0], TEXT_SPEED_RANGE[1]),
            auto_delay: self
                .prefs
                .auto_delay
                .clamp(AUTO_DELAY_RANGE[0], AUTO_DELAY_RANGE[1]),
            skip_mode: to_ui_skip(self.prefs.skip_mode),
            fullscreen: self
                .cfg
                .fullscreen_supported
                .then_some(self.prefs.fullscreen),
            languages: self
                .loc
                .languages()
                .into_iter()
                .map(|l| LanguageView {
                    name: language_name(&l.code, &l.name),
                    code: l.code,
                })
                .collect(),
            language: self.loc.language().to_owned(),
            screen_reader: Some(self.prefs.screen_reader),
            text_scale: self.prefs.text_scale,
            high_contrast: self.prefs.high_contrast,
            reduced_motion: self.prefs.reduced_motion,
        }
    }

    /// Pushes every preference to the places that apply it.
    fn apply_prefs_effects(&mut self) {
        let v = self.prefs.volumes;
        self.effects.extend([
            Effect::SetVolume(AudioBus::Master, v.master),
            Effect::SetVolume(AudioBus::Music, v.music),
            Effect::SetVolume(AudioBus::Sfx, v.sfx),
            Effect::SetVolume(AudioBus::Voice, v.voice),
        ]);
        if self.cfg.fullscreen_supported {
            self.effects
                .push(Effect::SetFullscreen(self.prefs.fullscreen));
        }
        self.apply_play_settings();
    }

    fn apply_play_settings(&mut self) {
        let settings = self.play_settings();
        if let Some(game) = &mut self.game {
            game.set_settings(settings, &self.theme);
        }
    }

    fn set_language(&mut self, code: &str) {
        let mut loc = (*self.loc).clone();
        if !loc.set_language(code) {
            return;
        }
        self.prefs.language = Some(loc.language().to_owned());
        self.mark_prefs_dirty();
        self.install_language(Arc::new(loc));
    }

    /// Re-translates every text after the language changed.
    fn install_language(&mut self, loc: Arc<Localizer>) {
        self.loc = loc;
        self.texts = make_texts(self.loc.language(), &self.cfg.ui_overrides);
        let texts = self.texts.clone();
        self.main.set_texts(&texts);
        self.pause.set_texts(&texts);
        self.save_screen.set_texts(&texts);
        self.load_screen.set_texts(&texts);
        self.prefs_screen.set_texts(&texts);
        self.history.set_texts(&texts);
        self.quick.set_texts(&texts);
        for screen in &mut self.data {
            screen.set_texts(&texts);
        }
        for s in &mut self.story_screens {
            s.screen.set_texts(&texts);
        }
        self.end_banner = None;
        if let Some(game) = &mut self.game {
            game.set_localizer(self.loc.clone(), &self.theme);
        }
        self.prefs_screen.set_view(self.prefs_view());
        self.history.set_view(self.history_view());
        self.views_dirty = true;
    }

    fn reset_preferences(&mut self) {
        self.prefs = Preferences::default();
        let mut loc = (*self.loc).clone();
        let source = loc.source_language().to_owned();
        loc.set_language(&source);
        Self::pick_language(&mut loc, &self.prefs, &self.cfg);
        self.install_language(Arc::new(loc));
        self.apply_prefs_effects();
        self.refresh_theme();
        self.prefs_screen.set_view(self.prefs_view());
        self.mark_prefs_dirty();
    }

    /// Derives the drawing theme from the project theme and the text scale
    /// and high-contrast preferences, and re-skins every screen with it.
    fn refresh_theme(&mut self) {
        let theme = self
            .base_theme
            .accessible(self.prefs.text_scale, self.prefs.high_contrast);
        if theme == self.theme {
            return;
        }
        self.theme = theme;
        let theme = self.theme.clone();
        self.main.set_theme(&theme);
        self.pause.set_theme(&theme);
        self.save_screen.set_theme(&theme);
        self.load_screen.set_theme(&theme);
        self.prefs_screen.set_theme(&theme);
        self.history.set_theme(&theme);
        self.quick.set_theme(&theme);
        self.toasts.set_theme(&theme);
        for screen in &mut self.data {
            screen.set_theme(&theme);
        }
        for s in &mut self.story_screens {
            s.screen.set_theme(&theme);
        }
        if let Some(game) = &mut self.game {
            game.set_theme(&theme);
        }
        self.views_dirty = true;
    }

    // -------------------------------------------------------------- screens

    /// Registers the data-driven screens of a UI bundle: they open by id
    /// (`Open("id")`) and the `main_menu`, `pause` and `preferences` ones
    /// replace the stock Rust screens (which stay the fallback for every
    /// system screen the bundle does not provide). The theme and the
    /// splashes of the bundle are the runtime's business.
    pub fn set_ui_bundle(&mut self, bundle: &UiBundle) {
        self.data.clear();
        self.stack.clear();
        // Story screens are rebuilt from the new assets at the next sync.
        self.story_screens.clear();
        self.ghosts.clear();
        self.stack_opened.clear();
        self.hud_pointer = None;
        self.cut_screen_fx = true;
        let ctx = self.data_context();
        let texts = self.texts.clone();
        if let Some(project_theme) = &bundle.theme {
            self.base_theme = project_theme.clone();
            self.refresh_theme();
        }
        let theme = self.theme.clone();
        for asset in &bundle.screens {
            self.data
                .push(DataScreen::new(asset.clone(), &theme, &texts, ctx.clone()));
        }
        let find = |name: &str| {
            bundle
                .system_screen(name)
                .and_then(|a| self.data.iter().position(|s| s.id() == a.id))
        };
        self.sys_main = find("main_menu");
        self.sys_pause = find("pause");
        self.sys_prefs = find("preferences");
        self.sys_save = find("save");
        self.sys_load = find("load");
        self.slots_wanted = bundle.screens.iter().any(|a| {
            let mut has = false;
            a.root.walk(&mut |w| has |= matches!(w.kind, orbit_ui::WidgetKind::SaveSlots { .. }));
            has
        });
        self.refresh_main_menu();
    }

    fn data_context(&self) -> DataContext {
        DataContext {
            prefs: self.prefs_view(),
            can_continue: self.can_continue,
            can_save: self.can_save(),
            show_quit: self.cfg.quit_supported,
            in_game: self.game.is_some(),
            background: self.menu_background,
            title: self.story.title.clone(),
            saves: if self.slots_wanted {
                self.category_views()
            } else {
                Vec::new()
            },
            vars: self.story_vars(),
            images: self.ui_images.clone(),
            gallery: gallery_paths(&self.story),
            unlocked: self.unlocked_images(),
            scenes: self.replay_scenes(),
            achievements: self.achievement_rows(),
        }
    }

    /// The story variables the data screens read: the running playthrough's
    /// (both scopes), or at the title the declared initial values with the
    /// saved persistent ones.
    fn story_vars(&self) -> std::collections::BTreeMap<String, Value> {
        if let Some(game) = &self.game {
            let vm = game.director.vm();
            let mut vars = vm.variables().clone();
            vars.extend(vm.persistent_vars().clone());
            return vars;
        }
        let mut vars: std::collections::BTreeMap<String, Value> = self
            .story
            .variables
            .iter()
            .map(|v| (v.name.clone(), v.initial.clone()))
            .collect();
        for decl in self.story.persistent_variables() {
            if let Some(v) = self
                .persistent
                .vars
                .get(&decl.name)
                .filter(|v| v.value_type() == decl.initial.value_type())
            {
                vars.insert(decl.name.clone(), v.clone());
            }
        }
        vars
    }

    /// Applies a no-code variable operation from a screen. In a game it goes
    /// through the VM (rollback history and saves stay consistent); at the
    /// title only persistent variables can change (there is no playthrough).
    /// Persistent changes are written to disk at once.
    fn apply_var_op(&mut self, op: &VarOp) {
        let persistent = self
            .story
            .persistent_variables()
            .any(|v| v.name == op.var);
        if let Some(game) = &mut self.game {
            if let Err(err) = game.director.vm_mut().apply_var_op(op) {
                log::warn!("variable operation ignored: {err}");
                return;
            }
        } else if persistent {
            let mut vm = match self.fresh_vm() {
                Ok(vm) => vm,
                Err(message) => {
                    log::warn!("variable operation ignored: {message}");
                    return;
                }
            };
            if let Err(err) = vm.apply_var_op(op) {
                log::warn!("variable operation ignored: {err}");
                return;
            }
            self.persistent = PersistentData::from_vm(&vm);
        } else {
            log::warn!("variable `{}` only exists during a game", op.var);
            return;
        }
        if persistent {
            self.flush_persistent();
        }
    }

    /// Attaches the runtime plugins (M8-04): every VM created from now on
    /// (new game, load) runs custom nodes and UI actions through them.
    pub fn set_plugin_host(&mut self, host: Option<orbit_vm::SharedPluginHost>) {
        if let Some(game) = &mut self.game {
            game.director.vm_mut().set_plugin_host(host.clone());
        }
        self.plugins = host;
    }

    /// Attaches the platform hook of the achievements (M7-05) and catches it
    /// up with the ones the player already has.
    pub fn set_achievement_hook(&mut self, hook: Option<crate::achievements::SharedAchievementHook>) {
        if let Some(h) = &hook {
            for a in &self.story.achievements {
                if self.persistent.unlocked_achievements.contains(&a.id) {
                    h.unlocked(&crate::achievements::AchievementEvent {
                        id: a.id.to_string(),
                        title: self.story.text(a.title_id).unwrap_or_default().to_owned(),
                        fresh: false,
                    });
                }
            }
        }
        self.achievement_hook = hook;
    }

    /// Attaches the script engine (M8-01): every VM created from now on
    /// runs `Script` nodes and Rhai conditions with it.
    pub fn set_script_host(&mut self, host: Option<orbit_vm::SharedScriptHost>) {
        if let Some(game) = &mut self.game {
            game.director.vm_mut().set_script_host(host.clone());
        }
        self.scripts = host;
    }

    /// A `UiAction::Custom` (UI-18): run by the plugin that owns `name`
    /// (`<plugin id>.<action>`) through the VM, so its variable writes are
    /// saved and rolled back like any other; at the title only persistent
    /// variables exist. Without a handler, it goes to the host as an
    /// [`Effect::Custom`].
    fn custom_action(&mut self, name: String, args: Vec<String>) {
        let mut handled = false;
        let mut persistent: Option<PersistentData> = None;
        if self.plugins.is_some() {
            if let Some(game) = &mut self.game {
                let vm = game.director.vm_mut();
                let before = vm.persistent_vars().clone();
                handled = vm.run_ui_action(&name, &args);
                for w in vm.take_warnings() {
                    log::warn!("{w}");
                }
                if handled && !vm.is_replay() && &before != vm.persistent_vars() {
                    persistent = Some(PersistentData::from_vm(vm));
                }
            } else {
                match self.fresh_vm() {
                    Ok(mut vm) => {
                        handled = vm.run_ui_action(&name, &args);
                        for w in vm.take_warnings() {
                            log::warn!("{w}");
                        }
                        if handled {
                            persistent = Some(PersistentData::from_vm(&vm));
                        }
                    }
                    Err(message) => log::warn!("custom UI action `{name}` ignored: {message}"),
                }
            }
        }
        if let Some(data) = persistent {
            self.persistent = data;
            self.flush_persistent();
        }
        if !handled {
            self.effects.push(Effect::Custom { name, args });
        }
    }

    /// Hands the current game / settings state to the data screens that are
    /// showing (they rebuild only when it changed).
    fn sync_data(&mut self) {
        if self.data.is_empty() {
            return;
        }
        let ctx = self.data_context();
        for s in &mut self.story_screens {
            s.screen.set_context(ctx.clone());
        }
        let mut showing: Vec<usize> = self
            .stack
            .iter()
            .filter_map(|o| match o {
                Overlay::Custom(i) => Some(*i),
                _ => None,
            })
            .collect();
        if self.game.is_none() {
            showing.extend(self.sys_main);
        }
        for i in showing {
            self.data[i].set_context(ctx.clone());
        }
    }

    /// The screen shown on the title: the data main menu when the project
    /// provides one.
    fn main_screen(&mut self) -> &mut dyn Screen {
        match self.sys_main {
            Some(i) => &mut self.data[i],
            None => &mut self.main,
        }
    }

    /// `true` while the game must not advance: a stock screen is open, or a
    /// data screen with `pauses_game`.
    pub fn game_paused(&self) -> bool {
        match self.top() {
            // A story screen pauses the game when it asks to (a HUD never).
            None => self
                .story_modal()
                .is_some_and(|i| self.story_screens[i].screen.asset().effective_pauses_game()),
            Some(Overlay::Custom(i)) => self.data[i].asset().effective_pauses_game(),
            Some(_) => true,
        }
    }

    /// `Open("id")`: a data screen of that id, else a stock one.
    fn open_screen(&mut self, id: &str) {
        if let Some(i) = self.data.iter().position(|s| s.id() == id) {
            if Some(i) == self.sys_pause && self.game.is_none() {
                return;
            }
            if Some(i) == self.sys_save && !self.can_save() {
                return;
            }
            self.open(Overlay::Custom(i));
            return;
        }
        match id {
            "preferences" => self.apply(UiEvent::OpenPreferences),
            "save" => self.apply(UiEvent::OpenSave),
            "load" => self.apply(UiEvent::OpenLoad),
            "history" => self.apply(UiEvent::OpenHistory),
            "pause" => self.apply(UiEvent::OpenPause),
            other => log::warn!("no screen called `{other}`"),
        }
    }

    fn open(&mut self, overlay: Overlay) {
        // A data screen standing in for a stock one takes its place.
        let stand_in = match overlay {
            Overlay::Pause => self.sys_pause,
            Overlay::Prefs => self.sys_prefs,
            Overlay::Save => self.sys_save,
            Overlay::Load => self.sys_load,
            _ => None,
        };
        let overlay = stand_in.map_or(overlay, Overlay::Custom);
        if self.top() == Some(overlay) {
            return;
        }
        match overlay {
            Overlay::Pause => self.sync_pause_view(),
            Overlay::Save | Overlay::Load => {
                self.refresh_slots();
                self.rebuild_save_views();
            }
            Overlay::Prefs => self.prefs_screen.set_view(self.prefs_view()),
            Overlay::History => self.history.set_view(self.history_view()),
            Overlay::Custom(i) => {
                if self.slots_wanted {
                    self.refresh_slots();
                }
                self.sync_data();
                self.stack_opened.insert(i, 0.0);
                self.ghosts.retain(|g| !matches!(g.screen, GhostScreen::Data(d) if d == i));
            }
        }
        self.stack.push(overlay);
    }

    fn close_top(&mut self) {
        let popped = self.stack.pop();
        if let Some(Overlay::Custom(i)) = popped {
            self.stack_opened.remove(&i);
            if self.data[i].asset().transition_out.is_some()
                && !self.prefs.reduced_motion
                && !self.stack.contains(&Overlay::Custom(i))
            {
                self.ghosts.push(Ghost {
                    screen: GhostScreen::Data(i),
                    elapsed: 0.0,
                });
            }
        }
        if (popped == Some(Overlay::Prefs) || matches!(popped, Some(Overlay::Custom(_))))
            && self.prefs_timer.is_some()
        {
            self.flush_prefs();
        }
        if self.stack.is_empty() {
            self.refresh_main_menu();
        }
    }

    fn screen(&mut self, overlay: Overlay) -> &mut dyn Screen {
        match overlay {
            Overlay::Custom(i) => &mut self.data[i],
            Overlay::Pause => &mut self.pause,
            Overlay::Save => &mut self.save_screen,
            Overlay::Load => &mut self.load_screen,
            Overlay::Prefs => &mut self.prefs_screen,
            Overlay::History => &mut self.history,
        }
    }

    // ------------------------------------------------------- story screens

    /// Makes the story screens match the screens open in the VM (UI-14):
    /// new ones are built with their parameters, gone ones play their
    /// closing transition. A screen id the project does not have is closed
    /// at once (with a warning) so the story never waits for it forever.
    fn sync_story_screens(&mut self) {
        for _ in 0..4 {
            let wanted: Vec<OpenScreenState> = match &self.game {
                Some(game) => game.director.vm().open_screens().to_vec(),
                None => Vec::new(),
            };
            let same = wanted.len() == self.story_screens.len()
                && wanted.iter().zip(&self.story_screens).all(|(w, s)| *w == s.state);
            if same {
                self.cut_screen_fx = false;
                return;
            }
            let cut = std::mem::take(&mut self.cut_screen_fx) || self.prefs.reduced_motion;
            if cut {
                self.ghosts.clear();
            }
            let ctx = self.data_context();
            let mut old = std::mem::take(&mut self.story_screens);
            let mut unknown = Vec::new();
            for state in wanted {
                if let Some(at) = old.iter().position(|o| o.state == state) {
                    self.story_screens.push(old.remove(at));
                    continue;
                }
                let Some(asset) = self.data.iter().find(|d| d.id() == state.screen).map(|d| d.asset().clone())
                else {
                    unknown.push(state.screen.clone());
                    continue;
                };
                let screen =
                    DataScreen::with_params(asset, &self.theme, &self.texts, ctx.clone(), state.params.clone());
                self.story_screens.push(StoryScreen {
                    state,
                    screen,
                    elapsed: if cut { f32::INFINITY } else { 0.0 },
                });
            }
            self.hud_pointer = None;
            if !cut {
                for o in old {
                    if o.screen.asset().transition_out.is_some() {
                        self.ghosts.push(Ghost {
                            screen: GhostScreen::Story(Box::new(o.screen)),
                            elapsed: 0.0,
                        });
                    }
                }
            }
            if unknown.is_empty() {
                return;
            }
            let theme = self.theme.clone();
            for id in unknown {
                log::warn!("the story opens the screen `{id}`, which the project does not have: closed");
                if let Some(game) = &mut self.game {
                    game.close_screen(&id, None, &theme);
                }
            }
        }
    }

    /// The topmost story screen that is not a HUD: it takes the input.
    fn story_modal(&self) -> Option<usize> {
        self.story_screens.iter().rposition(|s| !s.screen.is_hud())
    }

    /// Runs the events of the story screen `screen`: closing it goes back
    /// to the story (with the close value), anything else is a normal event.
    fn apply_story_events(&mut self, screen: &str, events: Vec<UiEvent>) {
        let mut closed = false;
        for event in events {
            match event {
                UiEvent::Back | UiEvent::CloseAll | UiEvent::BackWith(_) if closed => {}
                UiEvent::Back | UiEvent::CloseAll => {
                    closed = true;
                    self.close_story_screen(screen, None);
                }
                UiEvent::BackWith(value) => {
                    closed = true;
                    self.close_story_screen(screen, Some(value));
                }
                other => self.apply(other),
            }
        }
        self.sync_story_screens();
    }

    fn close_story_screen(&mut self, screen: &str, value: Option<Value>) {
        let theme = self.theme.clone();
        if let Some(game) = &mut self.game {
            game.close_screen(screen, value, &theme);
        }
        self.sync_story_screens();
        self.pump_audio();
        self.check_game_state();
    }

    /// The look of a screen `elapsed` seconds after it opened (`opening`) or
    /// closed; `None` once a closing transition is over.
    fn screen_fx(
        &self,
        asset: &orbit_ui::ScreenAsset,
        elapsed: f32,
        opening: bool,
    ) -> Option<orbit_ui::ScreenFx> {
        let t = if opening { &asset.transition_in } else { &asset.transition_out };
        let Some(t) = t.as_ref().filter(|_| !self.prefs.reduced_motion) else {
            return opening.then_some(orbit_ui::ScreenFx::NONE);
        };
        let story = &self.story;
        match transition_shown(t, elapsed, opening, &|n| story.curves.get(n).cloned()) {
            Some(shown) => Some(t.fx(shown, CANVAS)),
            None => opening.then_some(orbit_ui::ScreenFx::NONE),
        }
    }

    /// Draws `screen` through its transition state.
    fn draw_with_fx(ctx: &mut DrawCtx, screen: &mut DataScreen, fx: orbit_ui::ScreenFx) {
        let start = ctx.scene.len();
        screen.draw(ctx);
        apply_screen_fx(ctx.scene, start, fx, CANVAS);
    }

    /// Story screens, read only (tests, preview).
    #[cfg(test)]
    pub(crate) fn story_screens(&self) -> &[StoryScreen] {
        &self.story_screens
    }

    // ---------------------------------------------------------------- events

    /// Applies the events a screen reported.
    pub fn apply_all(&mut self, events: Vec<UiEvent>) {
        for event in events {
            self.apply(event);
        }
    }

    pub fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::Sound(sound) => self.effects.push(Effect::UiSound(sound)),
            UiEvent::Toast(text) => self.toast(text),
            UiEvent::StartGame => self.start_new_game(),
            UiEvent::Continue => self.continue_game(),
            UiEvent::OpenLoad => self.open(Overlay::Load),
            UiEvent::OpenSave => {
                if self.can_save() {
                    self.open(Overlay::Save);
                }
            }
            UiEvent::OpenPreferences => self.open(Overlay::Prefs),
            UiEvent::OpenHistory => {
                if self.game.is_some() {
                    self.open(Overlay::History);
                }
            }
            UiEvent::OpenPause => {
                if self.game.is_some() {
                    self.open(Overlay::Pause);
                }
            }
            UiEvent::Resume | UiEvent::CloseAll => {
                self.stack.clear();
                if self.prefs_timer.is_some() {
                    self.flush_prefs();
                }
                if self.game.is_none() {
                    self.refresh_main_menu();
                }
            }
            UiEvent::OpenScreen(id) => self.open_screen(&id),
            UiEvent::ReplaceScreen(id) => {
                self.close_top();
                self.open_screen(&id);
            }
            UiEvent::PlaySound(path) => self.effects.push(Effect::PlayAsset(path)),
            UiEvent::Custom { name, args } => self.custom_action(name, args),
            UiEvent::VarOp(op) => self.apply_var_op(&op),
            UiEvent::SceneGoto(scene) => self.goto_scene(&scene),
            UiEvent::ReplayVoice => {
                if let Some(game) = &mut self.game {
                    game.director.replay_voice();
                }
                self.pump_audio();
            }
            UiEvent::ReplayVoiceAt(index) => self.replay_history_voice(index),
            UiEvent::ReplayScene(scene) => {
                self.start_replay(&scene);
            }
            UiEvent::Back | UiEvent::BackWith(_) => self.close_top(),
            UiEvent::ReturnToMainMenu => self.leave_game(),
            UiEvent::Quit => {
                self.on_exit();
                self.effects.push(Effect::Quit);
            }
            UiEvent::Rollback => self.rollback(),
            UiEvent::RollbackTo(id) => {
                self.cut_screen_fx = true;
                let theme = self.theme.clone();
                if let Some(game) = &mut self.game
                    && game.rollback_to(id as u64, &theme)
                {
                    self.stack.clear();
                }
            }
            UiEvent::ToggleSkip => {
                if let Some(game) = &mut self.game {
                    game.toggle_skip();
                }
            }
            UiEvent::ToggleAuto => {
                if let Some(game) = &mut self.game {
                    game.toggle_auto();
                }
            }
            UiEvent::QuickSave => self.request_save(SaveSlot::Quick, true),
            UiEvent::QuickLoad => self.quick_load(),
            UiEvent::LoadSlot(id) => {
                self.load_slot(to_save_slot(id));
                self.refresh_slots();
            }
            UiEvent::SaveSlot(id) => self.request_save(to_save_slot(id), true),
            UiEvent::DeleteSlot(id) => {
                match self.saves.delete(to_save_slot(id)) {
                    Ok(()) => self.toast_key("ui.toast.deleted"),
                    Err(err) => {
                        log::warn!("cannot delete {id:?}: {err}");
                        self.toast_key("ui.toast.save_failed");
                    }
                }
                self.refresh_slots();
                self.refresh_main_menu();
                self.rebuild_save_views();
            }
            UiEvent::SetVolume(bus, value) => {
                let v = &mut self.prefs.volumes;
                match bus {
                    AudioBus::Master => v.master = value,
                    AudioBus::Music => v.music = value,
                    AudioBus::Sfx => v.sfx = value,
                    AudioBus::Voice => v.voice = value,
                }
                self.effects.push(Effect::SetVolume(bus, value));
                self.mark_prefs_dirty();
            }
            UiEvent::SetTextSpeed(v) => {
                self.prefs.text_speed = v;
                self.apply_play_settings();
                self.mark_prefs_dirty();
            }
            UiEvent::SetAutoDelay(v) => {
                self.prefs.auto_delay = v;
                self.apply_play_settings();
                self.mark_prefs_dirty();
            }
            UiEvent::SetSkipMode(mode) => {
                self.prefs.skip_mode = from_ui_skip(mode);
                self.apply_play_settings();
                self.mark_prefs_dirty();
            }
            UiEvent::SetFullscreen(on) => {
                self.prefs.fullscreen = on;
                if self.cfg.fullscreen_supported {
                    self.effects.push(Effect::SetFullscreen(on));
                }
                self.mark_prefs_dirty();
            }
            UiEvent::SetScreenReader(on) => {
                self.prefs.screen_reader = on;
                self.mark_prefs_dirty();
            }
            UiEvent::SetTextScale(v) => {
                self.prefs.text_scale = orbit_ui::clamp_text_scale(v);
                self.refresh_theme();
                self.mark_prefs_dirty();
            }
            UiEvent::SetHighContrast(on) => {
                self.prefs.high_contrast = on;
                self.refresh_theme();
                self.mark_prefs_dirty();
            }
            UiEvent::SetReducedMotion(on) => {
                self.prefs.reduced_motion = on;
                self.apply_play_settings();
                self.mark_prefs_dirty();
            }
            UiEvent::SetLanguage(code) => self.set_language(&code),
            UiEvent::ResetPreferences => self.reset_preferences(),
        }
        self.sync_data();
        self.sync_story_screens();
        self.pump_audio();
    }

    fn quick_load(&mut self) {
        if self.quick_exists {
            self.load_slot(SaveSlot::Quick);
        } else {
            self.toast_key("ui.toast.nothing_to_load");
        }
    }

    fn rollback(&mut self) {
        self.cut_screen_fx = true;
        let theme = self.theme.clone();
        if let Some(game) = &mut self.game {
            game.rollback(&theme);
        }
        self.pump_audio();
    }

    /// `UiAction::SceneGoto`: in a game, jumps to the scene and closes the
    /// screens (the jump goes through the VM, so rollback, history and saves
    /// stay consistent). At the title there is no game: ignored.
    fn goto_scene(&mut self, scene: &str) {
        self.cut_screen_fx = true;
        let theme = self.theme.clone();
        let Some(game) = &mut self.game else {
            log::warn!("scene jump to `{scene}` ignored: no game is running");
            return;
        };
        if game.goto_scene(scene, &theme) {
            self.stack.clear();
            if self.prefs_timer.is_some() {
                self.flush_prefs();
            }
        }
        self.pump_audio();
    }

    fn roll_forward(&mut self) {
        self.cut_screen_fx = true;
        let theme = self.theme.clone();
        if let Some(game) = &mut self.game {
            game.roll_forward(&theme);
        }
        self.pump_audio();
    }

    fn pump_audio(&mut self) {
        let mut unlocks = Vec::new();
        if let Some(game) = &mut self.game {
            // A voice is not started while skipping (it would be cut at once).
            let skipping = game.skipping();
            for command in game.director.take_audio() {
                if skipping && matches!(command, AudioCommand::PlayVoice { .. }) {
                    game.director.voice_finished();
                    continue;
                }
                self.effects.push(Effect::Audio(command));
            }
            unlocks = game.director.take_unlocks();
            if let Some(asset) = game.take_hotspot_sound() {
                self.effects.push(Effect::Audio(AudioCommand::PlaySound { asset }));
            }
        }
        for id in unlocks {
            self.announce_unlock(id);
        }
    }

    /// An `Unlock` node unlocked `id` for the first time (M7-05): a toast, the
    /// persistent data written at once (an achievement must not be lost to a
    /// crash) and the platform hook.
    fn announce_unlock(&mut self, id: orbit_core::AchievementId) {
        let Some(def) = self.story.achievements.iter().find(|a| a.id == id) else {
            return;
        };
        let title = plain_text(self.loc.text(def.title_id));
        let source_title = self.story.text(def.title_id).unwrap_or_default().to_owned();
        let text = format!("{}: {title}", self.texts.get("ui.achievements.toast"));
        let icon = def
            .icon
            .and_then(|icon| self.story.assets.get(icon))
            .and_then(|entry| self.ui_images.get(&entry.path).copied());
        let seq = self.last_toast.as_ref().map_or(0, |(s, _)| s + 1);
        self.last_toast = Some((seq, orbit_ui::a11y::plain(&text)));
        self.toasts.push_with_icon(text, icon);
        self.persistent.unlocked_achievements.insert(id);
        self.flush_persistent();
        if let Some(hook) = &self.achievement_hook {
            hook.unlocked(&crate::achievements::AchievementEvent {
                id: id.to_string(),
                title: source_title,
                fresh: true,
            });
        }
        self.sync_data();
    }

    /// The voice of a line (M7-06): the one of the selected language, else the
    /// one of the `Say` node.
    fn line_voice(&self, node: orbit_core::NodeId, string_id: orbit_core::StringId) -> Option<orbit_core::AssetId> {
        let vm = self.game.as_ref()?.director.vm();
        self.loc.voice(string_id).or_else(|| vm.say_voice(node))
    }

    /// Plays again the voice of the history entry `index` (M7-06).
    fn replay_history_voice(&mut self, index: usize) {
        let asset = self
            .game
            .as_ref()
            .and_then(|g| g.director.vm().backlog().into_iter().nth(index))
            .and_then(|item| self.line_voice(item.node, item.string_id));
        if let Some(asset) = asset {
            self.effects.push(Effect::Audio(AudioCommand::PlayVoice { asset }));
        }
    }

    /// The host tells whether a voice line is still playing: when it is not,
    /// auto mode stops waiting for it (M7-06).
    pub fn sync_voice(&mut self, playing: bool) {
        if !playing && let Some(game) = &mut self.game {
            game.director.voice_finished();
        }
    }

    // ----------------------------------------------------------------- input

    pub fn input(&mut self, action: InputAction) {
        match action {
            InputAction::SkipHeld(held) => {
                if let Some(game) = &mut self.game {
                    game.set_skip_held(held);
                }
            }
            InputAction::Hotkey(key) => self.hotkey(key),
            InputAction::Number(n) => {
                if self.top().is_none() {
                    let theme = self.theme.clone();
                    if let Some(game) = &mut self.game {
                        game.menu_input(MenuInput::Number(n), &theme);
                    }
                }
            }
            InputAction::Ui(input) => self.ui_input(input),
        }
        self.after_input();
    }

    fn after_input(&mut self) {
        self.pump_audio();
        self.check_game_state();
        self.sync_story_screens();
    }

    fn hotkey(&mut self, key: Hotkey) {
        if key == Hotkey::ToggleFullscreen {
            if self.cfg.fullscreen_supported {
                let on = !self.prefs.fullscreen;
                self.apply(UiEvent::SetFullscreen(on));
                self.prefs_screen.set_view(self.prefs_view());
            }
            return;
        }
        if self.game.is_none() || self.top().is_some() {
            return;
        }
        match key {
            Hotkey::QuickSave => self.apply(UiEvent::QuickSave),
            Hotkey::QuickLoad => self.apply(UiEvent::QuickLoad),
            Hotkey::History => self.apply(UiEvent::OpenHistory),
            Hotkey::ToggleSkip => self.apply(UiEvent::ToggleSkip),
            Hotkey::ToggleAuto => self.apply(UiEvent::ToggleAuto),
            Hotkey::ShowHotspots => {
                if let Some(game) = &mut self.game {
                    game.toggle_show_hotspots();
                }
            }
            Hotkey::ToggleFullscreen => {}
        }
    }

    fn ui_input(&mut self, input: UiInput) {
        if let Some(top) = self.top() {
            let events = self.screen(top).handle(input);
            self.apply_all(events);
            return;
        }
        if self.game.is_none() {
            let events = self.main_screen().handle(input);
            self.apply_all(events);
            return;
        }
        if self.story_input(input) {
            return;
        }
        self.game_input(input);
    }

    /// Input for the screens the story opened (UI-14): the topmost one that
    /// is not a HUD takes everything (except clicks on the quick menu, so
    /// rollback stays at hand); otherwise a HUD takes a press on one of its
    /// controls and the release that follows, and the rest goes to the game.
    /// Returns whether the input was used.
    fn story_input(&mut self, input: UiInput) -> bool {
        self.sync_story_screens();
        let on_quick = |sh: &mut Self, p: [f32; 2]| sh.quick.wants_pointer(p);
        if let Some(i) = self.story_modal() {
            match input {
                UiInput::PointerDown(p) | UiInput::PointerUp(p) if on_quick(self, p) => return false,
                _ => {}
            }
            let id = self.story_screens[i].state.screen.clone();
            let has_cancel = !self.story_screens[i].screen.asset().on_cancel.is_empty();
            let events = self.story_screens[i].screen.handle(input);
            if input == UiInput::Cancel && events.is_empty() && !has_cancel {
                // A screen that cannot be cancelled: the pause menu still opens.
                self.apply(UiEvent::OpenPause);
                return true;
            }
            self.apply_story_events(&id, events);
            return true;
        }
        match input {
            UiInput::PointerMove(_) => {
                for s in &mut self.story_screens {
                    s.screen.handle(input);
                }
                false
            }
            UiInput::PointerDown(p) => {
                let hit = self
                    .story_screens
                    .iter_mut()
                    .rposition(|s| s.screen.ui().wants_pointer(p));
                let Some(i) = hit else { return false };
                self.hud_pointer = Some(i);
                let id = self.story_screens[i].state.screen.clone();
                let events = self.story_screens[i].screen.handle(input);
                self.apply_story_events(&id, events);
                true
            }
            UiInput::PointerUp(_) => {
                let Some(i) = self.hud_pointer.take() else { return false };
                let Some(s) = self.story_screens.get_mut(i) else { return false };
                let id = s.state.screen.clone();
                let events = s.screen.handle(input);
                self.apply_story_events(&id, events);
                true
            }
            _ => false,
        }
    }

    /// Input on the bare game view (no screen on top).
    fn game_input(&mut self, input: UiInput) {
        let theme = self.theme.clone();
        let mut outcome = InputOutcome::Nothing;
        match input {
            UiInput::PointerMove(p) => {
                let events = self.quick.handle(input);
                self.apply_all(events);
                if let Some(game) = &mut self.game {
                    game.hover(p);
                }
            }
            UiInput::PointerDown(p) => {
                // Clicks on the quick menu never advance the dialogue.
                if self.quick.wants_pointer(p) {
                    let events = self.quick.handle(input);
                    self.apply_all(events);
                } else if let Some(game) = &mut self.game {
                    outcome = game.click(p, &theme);
                }
            }
            UiInput::PointerUp(_) => {
                let events = self.quick.handle(input);
                self.apply_all(events);
            }
            UiInput::Confirm => {
                if let Some(game) = &mut self.game {
                    outcome = game.confirm(&theme);
                }
            }
            UiInput::Cancel => self.apply(UiEvent::OpenPause),
            UiInput::Up => {
                if let Some(game) = &mut self.game {
                    game.menu_input(MenuInput::Up, &theme);
                }
            }
            UiInput::Down => {
                if let Some(game) = &mut self.game {
                    game.menu_input(MenuInput::Down, &theme);
                }
            }
            // On a hotspot map the arrows and Tab move between the zones.
            UiInput::Left | UiInput::Right | UiInput::Next | UiInput::Prev
                if self.game.as_ref().is_some_and(Game::on_hotspots) =>
            {
                if let Some(game) = &mut self.game {
                    match input {
                        UiInput::Left => game.hotspot_focus(None, (-1.0, 0.0)),
                        UiInput::Right => game.hotspot_focus(None, (1.0, 0.0)),
                        UiInput::Next => game.hotspot_focus(Some(true), (0.0, 0.0)),
                        _ => game.hotspot_focus(Some(false), (0.0, 0.0)),
                    }
                }
            }
            UiInput::Left | UiInput::PageUp => self.rollback(),
            UiInput::Right | UiInput::PageDown => self.roll_forward(),
            UiInput::Scroll(dy) if dy > 0.0 => self.rollback(),
            UiInput::Scroll(dy) if dy < 0.0 => self.roll_forward(),
            UiInput::Scroll(_) | UiInput::Next | UiInput::Prev => {}
        }
        if outcome == InputOutcome::Finished {
            self.leave_game();
        }
    }

    // ---------------------------------------------------------------- update

    /// Advances the game, the screens and the pending work by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        self.toasts.update(dt);
        let top = self.top();
        match (&self.game, top) {
            (None, _) => self.main_screen().update(dt),
            (Some(_), _) => self.quick.update(dt),
        }
        if let Some(top) = top {
            self.screen(top).update(dt);
        }
        // Screen transitions (UI-16) run on real time, paused game or not.
        for s in &mut self.story_screens {
            s.elapsed += dt;
            s.screen.update(dt);
        }
        for t in self.stack_opened.values_mut() {
            *t += dt;
        }
        for g in &mut self.ghosts {
            g.elapsed += dt;
        }
        let ghosts = std::mem::take(&mut self.ghosts);
        self.ghosts = ghosts
            .into_iter()
            .filter(|g| {
                let asset = match &g.screen {
                    GhostScreen::Story(s) => s.asset(),
                    GhostScreen::Data(i) => self.data[*i].asset(),
                };
                self.screen_fx(asset, g.elapsed, false).is_some()
            })
            .collect();

        if !self.game_paused() && self.game.is_some() {
            self.play_time += dt;
            let theme = self.theme.clone();
            if let Some(game) = &mut self.game {
                game.update(dt, &theme);
            }
            self.pump_audio();
            self.check_game_state();
            self.check_autosave(dt);
        }
        self.sync_story_screens();
        self.autosave_cooldown = (self.autosave_cooldown - dt).max(0.0);
        self.sync_quick_view();

        for job in &mut self.jobs {
            job.age += dt;
        }
        self.finish_jobs();

        if let Some(timer) = &mut self.prefs_timer {
            *timer -= dt;
            if *timer <= 0.0 {
                self.flush_prefs();
            }
        }
        if self.views_dirty {
            self.rebuild_save_views();
        }
    }

    fn sync_quick_view(&mut self) {
        let Some(game) = &self.game else { return };
        self.quick.set_view(QuickMenuView {
            can_rollback: game.director.can_rollback(),
            can_quick_load: self.quick_exists,
            skip_active: game.skipping(),
            auto_active: game.auto_on(),
            can_replay_voice: game.director.line_voice().is_some(),
        });
    }

    /// Reacts to the story failing or ending.
    fn check_game_state(&mut self) {
        let Some(game) = &self.game else { return };
        match game.director.flow() {
            Flow::Failed(message) => {
                self.failure = Some(format!("The story stopped: {message}"));
            }
            Flow::Ended if self.replaying() => {
                // The replayed scene is over: back to the replay menu.
                self.leave_game();
            }
            Flow::Ended if !self.end_handled => {
                self.end_handled = true;
                // The playthrough is over: its persistent progress (counters,
                // read lines) is written now.
                self.flush_persistent();
            }
            _ => {}
        }
    }

    fn check_autosave(&mut self, _dt: f32) {
        let cfg = self.cfg.autosave;
        let Some(game) = &self.game else { return };
        if game.director.vm().is_replay() {
            return;
        }
        let progress = game.director.progress();
        if progress == self.last_progress {
            return;
        }
        self.last_progress = progress;
        if !matches!(game.kind(), FlowKind::Say | FlowKind::Choice) {
            return;
        }
        let scene = self.current_scene();
        let new_scene = scene != self.last_scene;
        self.last_scene = scene;
        self.lines_since_autosave += 1;
        if !cfg.enabled {
            return;
        }
        let periodic = cfg
            .every_lines
            .is_some_and(|n| self.lines_since_autosave >= n.max(1))
            && self.autosave_cooldown <= 0.0;
        if (new_scene && cfg.on_scene_start) || periodic {
            self.lines_since_autosave = 0;
            self.autosave_cooldown = cfg.min_interval;
            self.request_save(SaveSlot::Auto(0), false);
        }
    }

    // ---------------------------------------------------------- screen reader

    /// The accessibility tree of what is on screen (ADR 0017, M7-09): the
    /// screen on top, or the main menu, or the game view (dialogue line as a
    /// live region, choices, quick menu), plus the last toast. Only the
    /// window node when the player turned the screen reader support off.
    pub fn a11y_tree(&mut self, name: &str) -> A11yTree {
        let mut tree = A11yTree::root(name);
        if !self.prefs.screen_reader {
            return tree;
        }
        let top = self.top();
        let covered = match top {
            Some(Overlay::Save | Overlay::Load | Overlay::Prefs | Overlay::History) => true,
            Some(Overlay::Custom(i)) => !self.data[i].is_modal() && !self.data[i].pauses_game(),
            _ => false,
        };
        let mut focus = None;
        if let Some(game) = &self.game {
            match game.kind() {
                FlowKind::Say => {}
                _ => self.announcer.clear(),
            }
            if !covered {
                match game.kind() {
                    FlowKind::Say => {
                        if let Some((key, text)) = &game.spoken {
                            let mut node =
                                A11yNode::new(LINE_ID, A11yRole::Live, self.announcer.text(*key, text));
                            node.bounds = self.theme.textbox.rect;
                            tree.push(ROOT_ID, node);
                        }
                    }
                    FlowKind::Choice => {
                        let mut list =
                            A11yNode::new(CHOICES_ID, A11yRole::List, self.texts.get("ui.a11y.choices"));
                        list.bounds = [0.0, 0.0, CANVAS[0], CANVAS[1]];
                        tree.push(ROOT_ID, list);
                        let (selected, rects) = game
                            .menu
                            .as_ref()
                            .map_or((0, &[][..]), |m| (m.selected(), m.rects()));
                        for (i, label) in game.choice_labels.iter().enumerate() {
                            let mut b = A11yNode::new(CHOICE_BASE + i as u64, A11yRole::Button, label.clone());
                            b.selected = i == selected;
                            b.bounds = rects.get(i).copied().unwrap_or_default();
                            tree.push(CHOICES_ID, b);
                        }
                        if top.is_none() && !game.choice_labels.is_empty() {
                            focus = Some(CHOICE_BASE + selected as u64);
                        }
                    }
                    FlowKind::Ended => {
                        tree.push(ROOT_ID, A11yNode::new(LINE_ID, A11yRole::Live, self.texts.get("ui.the_end")));
                    }
                    _ => {}
                }
            }
            if top.is_none() {
                let quick = self.quick.widgets().expect("the quick menu has widgets");
                tree.push_ui(ROOT_ID, SLOT_QUICK, "", quick);
            }
        } else if top.is_none() {
            let name = self.story.title.clone();
            if let Some(ui) = self.main_screen().widgets() {
                focus = tree.push_ui(ROOT_ID, SLOT_MAIN, &name, ui);
            }
        }
        if let Some(top) = top {
            let slot = SLOT_STACK + (self.stack.len() - 1) as u32;
            let name = self.overlay_name(top);
            if let Some(ui) = self.screen(top).widgets() {
                focus = tree.push_ui(ROOT_ID, slot, &name, ui);
            }
        }
        if !self.toasts.is_empty()
            && let Some((seq, text)) = &self.last_toast
        {
            let text = self.toast_announcer.text(*seq, text);
            tree.push(ROOT_ID, A11yNode::new(TOAST_ID, A11yRole::Live, text));
        }
        tree.focus = focus.unwrap_or(ROOT_ID);
        tree
    }

    /// The accessible name of a screen.
    fn overlay_name(&self, overlay: Overlay) -> String {
        match overlay {
            Overlay::Pause => self.texts.get("ui.pause.title"),
            Overlay::Save => self.texts.get("ui.save.title"),
            Overlay::Load => self.texts.get("ui.load.title"),
            Overlay::Prefs => self.texts.get("ui.prefs.title"),
            Overlay::History => self.texts.get("ui.history.title"),
            Overlay::Custom(i) => self.data[i].id().to_owned(),
        }
    }

    /// Runs what the screen reader asked of the node `id` (an id of the
    /// last [`Shell::a11y_tree`]): a click activates a widget or picks a
    /// choice, a focus moves the widget focus, increment / decrement move a
    /// slider or a selector. Unknown or stale ids are ignored.
    pub fn a11y_action(&mut self, id: u64, action: A11yAction) {
        if !self.prefs.screen_reader {
            return;
        }
        match orbit_ui::a11y::target_of(id) {
            A11yTarget::Choice(i) => {
                if action == A11yAction::Click && self.top().is_none() {
                    self.choose_option(i);
                }
            }
            A11yTarget::Widget(slot, index) => {
                let screen: &mut dyn Screen = match slot {
                    SLOT_QUICK if self.game.is_some() && self.top().is_none() => &mut self.quick,
                    SLOT_MAIN if self.game.is_none() && self.top().is_none() => self.main_screen(),
                    s if s >= SLOT_STACK && (s - SLOT_STACK) as usize + 1 == self.stack.len() => {
                        let top = self.stack[self.stack.len() - 1];
                        self.screen(top)
                    }
                    _ => return,
                };
                let Some(ui) = screen.widgets() else { return };
                if !ui.focus_node(index) {
                    return;
                }
                let input = match action {
                    A11yAction::Focus => return,
                    A11yAction::Click => UiInput::Confirm,
                    A11yAction::Increment => UiInput::Right,
                    A11yAction::Decrement => UiInput::Left,
                };
                let events = screen.handle(input);
                self.apply_all(events);
            }
            A11yTarget::Fixed(_) | A11yTarget::Screen(_) => return,
        }
        self.after_input();
    }

    // --------------------------------------------------------------- drawing

    /// Appends the UI of the current state to `ctx.scene`: the textbox or the
    /// choices, the quick menu, the main menu, the screen on top and the
    /// toasts. The stage itself is drawn by the runtime.
    pub fn draw_ui(&mut self, ctx: &mut DrawCtx, show_dialogue: bool) {
        // Full-screen screens are translucent: what they cover is not drawn,
        // so the textbox or the menu never shows through.
        let covered = match self.top() {
            Some(Overlay::Save | Overlay::Load | Overlay::Prefs | Overlay::History) => true,
            Some(Overlay::Custom(i)) => !self.data[i].is_modal() && !self.data[i].pauses_game(),
            _ => false,
        };
        match &mut self.game {
            _ if covered => {}
            Some(game) => {
                if show_dialogue {
                    match game.kind() {
                        FlowKind::Say => game.dialogue.draw(
                            ctx.engine,
                            ctx.text_renderer,
                            ctx.renderer,
                            ctx.scene,
                            ctx.white,
                            &self.ui_images,
                        ),
                        FlowKind::Choice if game.hotspots.is_some() => {
                            let tip = game
                                .hotspots
                                .as_ref()
                                .and_then(|h| h.tooltip().map(str::to_owned));
                            Self::draw_hotspot_tooltip(
                                ctx,
                                &self.theme,
                                &mut self.hotspot_tip,
                                tip.as_deref(),
                            );
                        }
                        FlowKind::Choice => {
                            if let Some(menu) = &mut game.menu {
                                menu.draw(
                                    ctx.engine,
                                    ctx.text_renderer,
                                    ctx.renderer,
                                    ctx.scene,
                                    ctx.white,
                                    &self.ui_images,
                                );
                            }
                        }
                        FlowKind::Ended => self.draw_end(ctx),
                        _ => {}
                    }
                }
                self.quick.draw(ctx);
            }
            None => self.main_screen().draw(ctx),
        }
        if self.game.is_some() && !covered {
            // Story screens (UI-14): HUDs sit on their own layer under the
            // textbox and hide with the interface or behind a menu; the
            // others are drawn over the game, below the menus.
            let menus = self.top().is_some();
            let fxs: Vec<Option<orbit_ui::ScreenFx>> = self
                .story_screens
                .iter()
                .map(|s| self.screen_fx(s.screen.asset(), s.elapsed, true))
                .collect();
            for (s, fx) in self.story_screens.iter_mut().zip(fxs) {
                if s.screen.is_hud() && (menus || !show_dialogue) {
                    continue;
                }
                Self::draw_with_fx(ctx, &mut s.screen, fx.unwrap_or(orbit_ui::ScreenFx::NONE));
            }
        }
        // Closing transitions.
        let ghost_fx: Vec<Option<orbit_ui::ScreenFx>> = self
            .ghosts
            .iter()
            .map(|g| {
                let asset = match &g.screen {
                    GhostScreen::Story(s) => s.asset(),
                    GhostScreen::Data(i) => self.data[*i].asset(),
                };
                self.screen_fx(asset, g.elapsed, false)
            })
            .collect();
        let mut ghosts = std::mem::take(&mut self.ghosts);
        for (g, fx) in ghosts.iter_mut().zip(ghost_fx) {
            let Some(fx) = fx else { continue };
            match &mut g.screen {
                GhostScreen::Story(s) => Self::draw_with_fx(ctx, s, fx),
                GhostScreen::Data(i) => Self::draw_with_fx(ctx, &mut self.data[*i], fx),
            }
        }
        self.ghosts = ghosts;
        // A modal data screen is drawn over the screen below it.
        if let Some(Overlay::Custom(i)) = self.top()
            && self.data[i].is_modal()
            && self.stack.len() >= 2
        {
            let below = self.stack[self.stack.len() - 2];
            self.screen(below).draw(ctx);
        }
        if let Some(top) = self.top() {
            match top {
                Overlay::Custom(i) => {
                    let elapsed = self.stack_opened.get(&i).copied().unwrap_or(f32::INFINITY);
                    let fx = self
                        .screen_fx(self.data[i].asset(), elapsed, true)
                        .unwrap_or(orbit_ui::ScreenFx::NONE);
                    Self::draw_with_fx(ctx, &mut self.data[i], fx);
                }
                other => self.screen(other).draw(ctx),
            }
        }
        self.toasts.draw(ctx);
    }

    /// The tooltip of the highlighted zone of a hotspot map: a caption on a
    /// dark band at the bottom of the screen. The text block is kept while
    /// the tooltip does not change.
    fn draw_hotspot_tooltip(
        ctx: &mut DrawCtx,
        theme: &Theme,
        cache: &mut Option<(String, Banner)>,
        tip: Option<&str>,
    ) {
        let Some(tip) = tip else { return };
        let size = theme.choice.text_size;
        if cache.as_ref().is_none_or(|(t, _)| t != tip) {
            *cache = Some((tip.to_owned(), Banner::centered(tip, size, theme.screens.text_color)));
        }
        let Some((_, banner)) = cache else { return };
        let y = CANVAS[1] * 0.86;
        ctx.scene.push(
            Sprite2d::new(ctx.white, [0.0, y - size * 0.45], [CANVAS[0], size * 2.3])
                .anchor([0.0, 0.0])
                .layer(Layer::UI)
                .z(-1.0)
                .tint(tint([0, 0, 0, 120])),
        );
        banner.draw(ctx.engine, ctx.text_renderer, ctx.renderer, ctx.scene, y, 1.0);
    }

    fn draw_end(&mut self, ctx: &mut DrawCtx) {
        let screens = &self.theme.screens;
        ctx.scene.push(
            Sprite2d::new(ctx.white, [0.0, 0.0], CANVAS)
                .anchor([0.0, 0.0])
                .layer(Layer::UI)
                .z(-1.0)
                .tint(tint(self.theme.choice.overlay)),
        );
        let size = screens.title_size * 0.7;
        let text = self.texts.get("ui.the_end");
        let color = screens.text_color;
        let banner = self
            .end_banner
            .get_or_insert_with(|| Banner::centered(&text, size, color));
        banner.draw(
            ctx.engine,
            ctx.text_renderer,
            ctx.renderer,
            ctx.scene,
            CANVAS[1] * 0.5 - size * 0.675,
            1.0,
        );
    }
}

#[cfg(test)]
mod tests;
