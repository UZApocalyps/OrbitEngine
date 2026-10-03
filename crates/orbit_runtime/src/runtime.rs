//! The runtime: owns the renderer, text, audio and the current phase
//! (loading, "click to start", running, error) and implements the platform
//! [`App`] trait. What happens while running (menus, game, saves...) lives in
//! [`crate::shell`]; this module is the GPU and device side: drawing, audio,
//! thumbnails, files, host requests.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use orbit_assets::{
    AssetError, AssetHandle, AssetLoader, AssetPoll, FileWatcher, PakLocation, PakSource, PcmAudio,
    decode_image,
};
use orbit_audio::{AudioEngine, Bus, MusicOptions, Sound};
use orbit_core::{StageFx, Story, TransitionKind};
use orbit_i18n::{Localizer, Translation};
use orbit_pak::{GAME_PACK_ENTRY, GameInfo};
use orbit_platform::{App, HostRequest, InputEvent, PhysicalSize, Visibility};
use orbit_render2d::{
    Frame, Layer, Readback, ReadbackStatus, RenderError, Renderer2d, Scene, Sprite2d, TextureId,
    TextureOptions, Transition,
};
use orbit_save::{MemoryStorage, SaveConfig, Storage};
use orbit_text::{TextEngine, TextRenderer, srgb_to_linear};
use orbit_ui::{
    AudioBus, Banner, DrawCtx, SplashDef, Texts, Theme, UiBundle, UiSound, draw_progress_bar, tint,
};
use thiserror::Error;

use crate::director::AudioCommand;
use crate::format::slug;
use crate::gamepad_bindings::GamepadBindings;
use crate::input::{InputAction, InputMapper};
use crate::loading::{
    AssetStore, LoadedStory, PendingAsset, finish_loading, parse_story, scene_asset_paths,
    start_loading, start_loading_only, successor_scenes,
};
use crate::shell::{AutosaveConfig, Effect, Shell, ShellConfig};
use crate::splash::{AfterLoad, SplashPhase, SplashState};
use crate::splash_draw::SplashAssets;
use crate::stage::{
    AnimClock, CANVAS, StageCharacter, StageState, animate_background, animate_layer, animate_placement,
    character_placement, cover_size, frame_scale, particle_quads, screen_overlay, tiled_centers, LAYER_OVERSCAN,
};
use orbit_core::anim::{FrameRef, sheet_cell_size, sheet_uv};
use orbit_core::AnimFrames;
use orbit_ui::UiInput;

mod embedded;

pub use embedded::{
    EmbeddedConfig, LoadOutcome, Notice, PlayFrom, PlaySpeed, PlayStatus, PreviewSnapshot,
    UiPreviewTarget,
};

/// Largest frame time fed to the simulation (a stall must not skip content).
const MAX_DT: f32 = 0.1;
/// How often the story file is checked for changes (native hot reload).
const WATCH_INTERVAL: Duration = Duration::from_millis(400);
/// Fade applied to the music when the story is reloaded or restarted.
const RESTART_MUSIC_FADE: Duration = Duration::from_millis(300);
/// Lines of an error message shown on screen.
const MAX_ERROR_LINES: usize = 14;
/// Size of the save thumbnails (16:9, like the stage).
const THUMBNAIL_SIZE: [u32; 2] = [320, 180];

/// Where the runtime finds things. Roots are directories natively and URL
/// prefixes on the web.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// Root the story file is resolved against.
    pub story_root: String,
    /// Story file, relative to `story_root`.
    pub story_path: String,
    /// Root the story manifest's asset paths are relative to (the project).
    pub assets_root: String,
    /// Root of the engine's own files (fonts).
    pub engine_root: String,
    /// Font files relative to `engine_root`.
    pub fonts: Vec<String>,
    /// Family name of the default font.
    pub font_family: String,
    /// Native filesystem path of the story to watch for hot reload (`None`
    /// disables it; it is inert on the web anyway).
    pub watch_path: Option<String>,
    pub theme: Theme,
    /// Identifies the game in the save storage (data directory natively,
    /// `localStorage` key prefix on the web). Default: the slug of the story
    /// title.
    pub game_id: Option<String>,
    /// Native: directory holding the saves, preferences and persistent data
    /// instead of the per-user default.
    pub data_dir: Option<String>,
    /// Folder, relative to `story_root`, with the translations: an
    /// `index.txt` listing one file name per line; the files named
    /// `<story file stem>.<lang>.ron` in it are loaded.
    pub translations_dir: String,
    pub save: SaveConfig,
    pub autosave: AutosaveConfig,
    /// Replacements of the built-in interface texts, per language (`"fr"`):
    /// key and text.
    pub ui_text_overrides: HashMap<String, Vec<(String, String)>>,
    /// Skip the main menu and start a new game as soon as everything is
    /// loaded (the "click to start" gesture is still required on the web).
    pub auto_start: bool,
    /// Play an exported game: the pak (file natively, URL on the web) holds
    /// the compiled story, its translations, the assets and the fonts, and
    /// the roots and the story path above are ignored.
    pub pak: Option<PakLocation>,
    /// Load the assets scene by scene while playing (the next scenes ahead of
    /// time) instead of all of them before the start: what the web player
    /// does with its pak, so that a long game starts quickly.
    pub lazy_assets: bool,
    /// Native: folder with the project's data-driven UI (`theme.ron`,
    /// `*.screen.ron`, `ui.ron`, see `UiBundle::load_dir`), read at start-up
    /// and on every hot reload. Without it (and without `ui_bundle`), the
    /// UI of the game pack is used, else the stock Rust screens. Splashes
    /// of the bundle follow the mandatory engine splash.
    pub ui_dir: Option<String>,
    /// A UI bundle given directly (it wins over `ui_dir` and the pack).
    pub ui_bundle: Option<UiBundle>,
    /// Runtime plugins (M8-04): custom nodes and custom UI actions. Set by a
    /// player built "with plugins" (`orbit_player::run_with_plugins`).
    pub plugins: Option<orbit_vm::SharedPluginHost>,
}

/// Where the fonts of the engine are inside a pak (`engine/<path>`).
pub const ENGINE_PAK_PREFIX: &str = orbit_pak::ENGINE_PREFIX;

impl RuntimeConfig {
    pub fn new(
        story_root: impl Into<String>,
        story_path: impl Into<String>,
        assets_root: impl Into<String>,
        engine_root: impl Into<String>,
    ) -> Self {
        Self {
            story_root: story_root.into(),
            story_path: story_path.into(),
            assets_root: assets_root.into(),
            engine_root: engine_root.into(),
            fonts: vec![
                "assets/fonts/Inter-Regular.ttf".into(),
                "assets/fonts/Inter-Bold.ttf".into(),
                "assets/fonts/Inter-Italic.ttf".into(),
            ],
            font_family: "Inter".into(),
            watch_path: None,
            theme: Theme::default(),
            game_id: None,
            data_dir: None,
            translations_dir: "i18n".into(),
            save: SaveConfig::default(),
            autosave: AutosaveConfig::default(),
            ui_text_overrides: HashMap::new(),
            auto_start: false,
            pak: None,
            lazy_assets: false,
            ui_dir: None,
            ui_bundle: None,
            plugins: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("renderer: {0}")]
    Render(#[from] RenderError),
}

/// Translation files being fetched: first the index, then each listed file.
struct TranslationLoad {
    index: AssetHandle<Vec<u8>>,
    files: Option<Vec<AssetHandle<Vec<u8>>>>,
}

impl TranslationLoad {
    fn start(loader: &AssetLoader, dir: &str) -> Self {
        Self {
            index: loader.load_bytes(&format!("{dir}/index.txt")),
            files: None,
        }
    }

    /// `true` once the index and every file it lists are done (a missing
    /// index simply means "no translations").
    fn poll(&mut self, loader: &AssetLoader, dir: &str, stem: &str) -> bool {
        if self.files.is_none() {
            match self.index.poll() {
                AssetPoll::Loading => return false,
                AssetPoll::Failed(_) => self.files = Some(Vec::new()),
                AssetPoll::Ready(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let files = parse_translation_index(&text, stem)
                        .iter()
                        .map(|name| loader.load_bytes(&format!("{dir}/{name}")))
                        .collect();
                    self.files = Some(files);
                }
            }
        }
        self.files
            .iter()
            .flatten()
            .all(|h| !matches!(h.poll(), AssetPoll::Loading))
    }

    /// The texts of the translation files that loaded.
    fn texts(&self) -> Vec<String> {
        self.files
            .iter()
            .flatten()
            .filter_map(|h| match h.poll() {
                AssetPoll::Ready(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                AssetPoll::Failed(err) => {
                    log::warn!("translation: {err}");
                    None
                }
                AssetPoll::Loading => None,
            })
            .collect()
    }
}

/// The translations folder: next to the story file (on the web the story path
/// carries its directories, natively `story_root` already is the directory).
fn translations_path(story_path: &str, dir: &str) -> String {
    match story_path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{dir}"),
        None => dir.to_owned(),
    }
}

/// File names of an `index.txt` that belong to the story named `stem`
/// (`<stem>.<lang>.ron`). Blank lines and `#` comments are ignored.
pub fn parse_translation_index(text: &str, stem: &str) -> Vec<String> {
    let prefix = format!("{stem}.");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| l.starts_with(&prefix) && l.ends_with(".ron"))
        .map(str::to_owned)
        .collect()
}

struct LoadingState {
    story: AssetHandle<LoadedStory>,
    /// Translation files next to a RON story (`None` for a game pack, which
    /// carries its translations).
    translations: Option<TranslationLoad>,
    loaded: Option<Arc<Story>>,
    /// Identity and translations of a game pack.
    info: Option<GameInfo>,
    packed_translations: Vec<Translation>,
    /// UI bundle of a game pack.
    ui: Option<Box<UiBundle>>,
    pending: Vec<PendingAsset>,
    assets_total: usize,
    /// Start playing as soon as loaded (hot reload after a first start).
    autostart: bool,
}

impl LoadingState {
    fn new(
        story: AssetHandle<LoadedStory>,
        translations: Option<TranslationLoad>,
        autostart: bool,
    ) -> Self {
        Self {
            story,
            translations,
            loaded: None,
            info: None,
            packed_translations: Vec::new(),
            ui: None,
            pending: Vec::new(),
            assets_total: 0,
            autostart,
        }
    }
}

/// Scene by scene loading of the assets (see [`RuntimeConfig::lazy_assets`]):
/// the assets of the scene on screen are required, those of the scenes it can
/// lead to are fetched ahead.
struct Streaming {
    /// Assets being fetched, whichever scene asked for them.
    in_flight: Vec<PendingAsset>,
    /// Scenes whose assets were requested.
    requested: HashSet<usize>,
    /// Scene the last round of requests was made for.
    last_scene: Option<usize>,
    /// The scene on screen waits for an asset: the game is held.
    blocked: bool,
}

impl Streaming {
    fn new(first_scene: usize) -> Self {
        Self {
            in_flight: Vec::new(),
            requested: HashSet::from([first_scene]),
            last_scene: None,
            blocked: false,
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum Phase {
    /// Exported game: reading the header and the index of the pak.
    Opening(AssetHandle<PakSource>),
    Loading(LoadingState),
    /// Everything is loaded; waiting for the "click to start" gesture.
    Ready(Box<Shell>),
    /// The splash screens (the engine's, then the project's) play before
    /// the shell takes over. Only an editor-embedded runtime skips them.
    Splash(Box<SplashPhase>),
    Running(Box<Shell>),
    Failed(String),
    /// Hosted by an editor and not playing: a blank stage.
    Stopped,
}

/// Where a frame is drawn: the window surface, or an offscreen render target
/// (the editor preview).
enum Output<'a> {
    Surface(&'a mut Frame),
    Target {
        id: TextureId,
        /// Something was already drawn into the target this frame.
        drawn: bool,
    },
}

impl Output<'_> {
    /// Draws `scene`; over what is there, except for the first one of a
    /// target (which starts from a clean slate).
    fn scene(&mut self, renderer: &mut Renderer2d, scene: &Scene) -> Result<(), RenderError> {
        match self {
            Output::Surface(frame) => renderer.draw_scene(frame, scene),
            Output::Target { id, drawn } => {
                let result = if *drawn {
                    renderer.render_to_target_over(*id, scene)
                } else {
                    renderer.render_to_target(*id, scene)
                };
                *drawn = true;
                result
            }
        }
    }

    fn transition(
        &mut self,
        renderer: &mut Renderer2d,
        from: TextureId,
        to: TextureId,
        transition: &Transition,
        progress: f32,
    ) -> Result<(), RenderError> {
        match self {
            Output::Surface(frame) => {
                renderer.draw_transition(frame, from, to, transition, progress)
            }
            Output::Target { id, drawn } => {
                if !*drawn {
                    renderer.render_to_target(*id, &Scene::new())?;
                    *drawn = true;
                }
                renderer.draw_transition_to_target(*id, from, to, transition, progress)
            }
        }
    }
}

/// GPU-side drawing context.
struct Ctx {
    renderer: Renderer2d,
    text_engine: TextEngine,
    text_renderer: TextRenderer,
    white: TextureId,
    theme: Theme,
}

/// Lazily built full-screen texts (they need the fonts). Dropped when the
/// language changes.
#[derive(Default)]
struct Screens {
    loading: Option<Banner>,
    start: Option<Banner>,
    title: Option<(String, Banner)>,
    error_title: Option<Banner>,
    error_hint: Option<Banner>,
    error_body: Option<(String, Banner)>,
}

/// The optional UI sounds of the theme (none by default).
struct UiSoundBank {
    handles: Vec<(UiSound, AssetHandle<PcmAudio>)>,
    sounds: HashMap<UiSound, Sound>,
}

impl UiSoundBank {
    fn new(theme: &Theme, loader: &AssetLoader) -> Self {
        let handles = [
            UiSound::Hover,
            UiSound::Confirm,
            UiSound::Cancel,
            UiSound::Slide,
        ]
        .into_iter()
        .filter_map(|s| Some((s, loader.load_audio(theme.sounds.get(s)?))))
        .collect();
        Self {
            handles,
            sounds: HashMap::new(),
        }
    }

    fn play(&mut self, sound: UiSound, audio: &mut AudioEngine) {
        if !self.sounds.contains_key(&sound) {
            let Some((_, handle)) = self.handles.iter().find(|(s, _)| *s == sound) else {
                return; // the theme defines no sound for this cue
            };
            match handle.poll() {
                AssetPoll::Ready(pcm) => match Sound::from_pcm(&pcm) {
                    Ok(decoded) => {
                        self.sounds.insert(sound, decoded);
                    }
                    Err(err) => log::warn!("ui sound {sound:?}: {err}"),
                },
                _ => return,
            }
        }
        if let Some(decoded) = self.sounds.get(&sound) {
            audio.play_sfx(decoded);
        }
    }
}

/// The storage of saves, preferences and persistent data. Falls back to memory
/// (nothing survives the session) when the real one is unavailable.
fn open_storage(config: &RuntimeConfig, game_id: &str) -> (Box<dyn Storage>, bool) {
    #[cfg(not(target_arch = "wasm32"))]
    let opened: Result<Box<dyn Storage>, orbit_save::StorageError> = match &config.data_dir {
        Some(dir) => Ok(Box::new(orbit_save::FileStorage::new(dir))),
        None => orbit_save::FileStorage::for_game(game_id).map(|s| Box::new(s) as _),
    };
    #[cfg(target_arch = "wasm32")]
    let opened: Result<Box<dyn Storage>, orbit_save::StorageError> = {
        let _ = config;
        orbit_save::LocalStorage::new(game_id).map(|s| Box::new(s) as _)
    };
    match opened {
        Ok(storage) => (storage, true),
        Err(err) => {
            log::warn!("save storage unavailable, progress will not be kept: {err}");
            (Box::new(MemoryStorage::new()), false)
        }
    }
}

fn bus_of(bus: AudioBus) -> Bus {
    match bus {
        AudioBus::Master => Bus::Master,
        AudioBus::Music => Bus::Music,
        AudioBus::Sfx => Bus::Sfx,
        AudioBus::Voice => Bus::Voice,
    }
}

/// PNG bytes of tightly packed RGBA8 pixels.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(out)
}

pub struct Runtime {
    ctx: Ctx,
    config: RuntimeConfig,
    store: AssetStore,
    audio: AudioEngine,
    story_loader: AssetLoader,
    assets_loader: AssetLoader,
    story_path: String,
    watcher: Option<FileWatcher>,
    font_handles: Vec<AssetHandle<Vec<u8>>>,
    fonts_ready: bool,
    targets: [TextureId; 2],
    /// The 3D background layer (inert without the `3d` feature or before
    /// the first 3D scene).
    layer3d: crate::compose3d::Layer3d,
    thumbnail_target: TextureId,
    captures: Vec<(u64, Readback)>,
    mapper: InputMapper,
    phase: Phase,
    /// The player already pressed "start" once (audio is unlocked).
    started_once: bool,
    time: f32,
    /// Animation times of the characters; runs with the game only.
    anim_clock: AnimClock,
    screens: Screens,
    /// Language the cached [`Screens`] were built for.
    screens_language: String,
    boot_texts: Texts,
    ui_sounds: UiSoundBank,
    host_requests: Vec<HostRequest>,
    /// Set when hosted by an editor (see [`Runtime::new_embedded`]).
    embedded: Option<embedded::Embedded>,
    /// The F3 profiler overlay, when shown (development builds only).
    overlay: Option<crate::overlay::ProfilerOverlay>,
    /// Lazy asset loading in progress (web player, see [`Streaming`]).
    streaming: Option<Streaming>,
    /// The page is hidden: nothing is drawn.
    hidden: bool,
    /// Android: the window insets clear of system bars and cutouts.
    #[allow(dead_code)]
    safe_area: orbit_platform::SafeArea,
    splash_assets: SplashAssets,
    /// Splashes of the UI bundle in use (the engine splash is not here: it
    /// is built into [`SplashState::new`]).
    ui_splashes: Vec<SplashDef>,
    /// The last accessibility tree sent to the screen reader.
    a11y_last: Option<orbit_ui::a11y::A11yTree>,
}

impl Runtime {
    /// Builds the runtime and starts loading the fonts and the story.
    pub fn new(renderer: Renderer2d, config: RuntimeConfig) -> Result<Self, RuntimeError> {
        Self::build(renderer, config, None)
    }

    fn build(
        mut renderer: Renderer2d,
        mut config: RuntimeConfig,
        embedded: Option<EmbeddedConfig>,
    ) -> Result<Self, RuntimeError> {
        if let Some(embedded) = &embedded {
            config.fonts.clear();
            config.font_family = embedded.font_family.clone();
            config.theme = embedded.theme.clone();
            config.theme.textbox.indicator_blink = false;
            config.autosave.enabled = false;
        }
        let text_renderer = TextRenderer::new(&mut renderer)?;
        let targets = [
            renderer.create_canvas_target("stage from")?,
            renderer.create_canvas_target("stage to")?,
        ];
        let thumbnail_target = renderer.create_render_target(
            "save thumbnail",
            THUMBNAIL_SIZE[0],
            THUMBNAIL_SIZE[1],
        )?;
        let pak_open = config
            .pak
            .clone()
            .filter(|_| embedded.is_none())
            .map(PakSource::open);
        let engine_loader = AssetLoader::new(config.engine_root.clone());
        // With a pak the fonts are read from it once it is open.
        let font_handles = if pak_open.is_some() {
            Vec::new()
        } else {
            config
                .fonts
                .iter()
                .map(|path| engine_loader.load_bytes(path))
                .collect()
        };
        let mut text_engine = TextEngine::new();
        text_engine.set_default_family(config.font_family.clone());
        if let Some(embedded) = &embedded {
            let added = embedded
                .fonts
                .iter()
                .filter(|bytes| text_engine.add_font_data(bytes.to_vec()) > 0)
                .count();
            if added == 0 {
                log::error!("no preview font could be loaded: text will not be displayed");
            }
        }

        let story_loader = AssetLoader::new(config.story_root.clone());
        let assets_loader = AssetLoader::new(config.assets_root.clone());
        if embedded.is_none() {
            if config.pak.is_some() {
                log::info!("runtime: playing the game of a pak");
            } else {
                log::info!(
                    "runtime: story `{}` (root `{}`), assets root `{}`",
                    config.story_path,
                    story_loader.root(),
                    assets_loader.root()
                );
            }
        }
        let phase = if embedded.is_some() {
            Phase::Stopped
        } else if let Some(open) = pak_open {
            Phase::Opening(open)
        } else {
            let story = load_story(&story_loader, &config.story_path);
            let translations = TranslationLoad::start(
                &story_loader,
                &translations_path(&config.story_path, &config.translations_dir),
            );
            Phase::Loading(LoadingState::new(story, Some(translations), false))
        };
        let white = renderer.white_texture();
        let boot_language = orbit_platform::system_language().unwrap_or_else(|| "en".into());
        let ui_sounds = UiSoundBank::new(&config.theme, &assets_loader);
        Ok(Self {
            ctx: Ctx {
                renderer,
                text_engine,
                text_renderer,
                white,
                theme: config.theme.clone(),
            },
            store: AssetStore::default(),
            audio: AudioEngine::new_locked(),
            story_path: config.story_path.clone(),
            watcher: config
                .watch_path
                .clone()
                .map(|path| FileWatcher::new(path, WATCH_INTERVAL)),
            story_loader,
            assets_loader,
            font_handles,
            fonts_ready: embedded.is_some(),
            targets,
            layer3d: crate::compose3d::Layer3d::default(),
            thumbnail_target,
            captures: Vec::new(),
            mapper: InputMapper::default(),
            phase,
            started_once: false,
            time: 0.0,
            anim_clock: AnimClock::default(),
            screens: Screens::default(),
            screens_language: boot_language.clone(),
            boot_texts: Texts::builtin(&boot_language),
            ui_sounds,
            host_requests: Vec::new(),
            embedded: embedded.map(embedded::Embedded::new),
            overlay: None,
            streaming: None,
            hidden: false,
            safe_area: orbit_platform::SafeArea::default(),
            splash_assets: SplashAssets::default(),
            ui_splashes: Vec::new(),
            a11y_last: None,
            config,
        })
    }

    /// An exported game: once its pak is open, the story, the translations,
    /// the assets and the fonts are all read from it.
    fn poll_opening(&mut self) {
        let Phase::Opening(handle) = &self.phase else {
            return;
        };
        match handle.poll() {
            AssetPoll::Loading => {}
            AssetPoll::Failed(err) => {
                let message = describe_error(&err);
                log::error!("cannot open the game: {message}");
                self.fonts_ready = true;
                self.phase = Phase::Failed(message);
            }
            AssetPoll::Ready(pak) => {
                let pak: PakSource = (*pak).clone();
                self.layer3d
                    .set_quality(pak.meta(crate::compose3d::QUALITY_META));
                let engine = AssetLoader::from_pak(pak.clone(), ENGINE_PAK_PREFIX);
                self.font_handles = self
                    .config
                    .fonts
                    .iter()
                    .map(|path| engine.load_bytes(path))
                    .collect();
                self.story_loader = AssetLoader::from_pak(pak, "");
                self.assets_loader = self.story_loader.clone();
                self.story_path = GAME_PACK_ENTRY.to_owned();
                self.ui_sounds = UiSoundBank::new(&self.config.theme, &self.assets_loader);
                log::info!("game pack opened");
                let story = load_story(&self.story_loader, &self.story_path);
                self.phase = Phase::Loading(LoadingState::new(story, None, false));
            }
        }
    }

    // ------------------------------------------------------------ updating

    fn poll_fonts(&mut self) {
        if self.fonts_ready
            || self
                .font_handles
                .iter()
                .any(|h| matches!(h.poll(), AssetPoll::Loading))
        {
            return;
        }
        let mut loaded = 0;
        for handle in &self.font_handles {
            match handle.poll() {
                AssetPoll::Ready(bytes) => {
                    if self.ctx.text_engine.add_font_data(bytes.to_vec()) > 0 {
                        loaded += 1;
                    }
                }
                AssetPoll::Failed(err) => log::error!("font: {err}"),
                AssetPoll::Loading => {}
            }
        }
        if loaded == 0 {
            log::error!("no font could be loaded: text will not be displayed");
        }
        self.fonts_ready = true;
    }

    fn poll_loading(&mut self) {
        let Phase::Loading(loading) = &mut self.phase else {
            return;
        };
        if loading.loaded.is_none() {
            match loading.story.poll() {
                AssetPoll::Loading => return,
                AssetPoll::Failed(err) => {
                    let message = describe_error(&err);
                    log::error!("cannot load the story: {message}");
                    self.phase = Phase::Failed(message);
                    return;
                }
                AssetPoll::Ready(loaded) => {
                    loading.pending = if self.config.lazy_assets {
                        let boot = boot_paths(&loaded.story);
                        log::info!("lazy loading: {} asset(s) before the start", boot.len());
                        start_loading_only(
                            &self.assets_loader,
                            &loaded.story,
                            &self.store,
                            Some(&boot),
                        )
                    } else {
                        start_loading(&self.assets_loader, &loaded.story, &self.store)
                    };
                    loading.assets_total = loading.pending.len();
                    loading.info = loaded.info.clone();
                    loading.packed_translations = loaded.translations.clone();
                    loading.ui = loaded.ui.clone().map(Box::new);
                    loading.loaded = Some(loaded.story.clone());
                }
            }
        }
        let stem = std::path::Path::new(&self.story_path)
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let translations_dir = translations_path(&self.story_path, &self.config.translations_dir);
        let translations_done = match loading.translations.as_mut() {
            Some(translations) => translations.poll(&self.story_loader, &translations_dir, &stem),
            None => true,
        };
        if !self.fonts_ready
            || !translations_done
            || !loading.pending.iter().all(PendingAsset::is_done)
        {
            return;
        }
        let Some(story) = loading.loaded.take() else {
            return;
        };
        let autostart = loading.autostart;
        let pending = std::mem::take(&mut loading.pending);
        let translation_texts = loading
            .translations
            .as_ref()
            .map(TranslationLoad::texts)
            .unwrap_or_default();
        let packed = std::mem::take(&mut loading.packed_translations);
        let info = loading.info.take();
        let pack_ui = loading.ui.take().map(|b| *b);
        finish_loading(&mut self.ctx.renderer, &mut self.store, pending);
        log::info!("story `{}` ready", story.title);
        if self.config.lazy_assets {
            let first = story
                .entry()
                .and_then(|node| story.locate(node))
                .map_or(0, |(scene, _)| scene);
            self.streaming = Some(Streaming::new(first));
        }
        let shell = self.build_shell_full(story, &translation_texts, packed, info, pack_ui);
        // Hot reload restarts at once; `auto_start` skips the click natively
        // (nothing blocks audio there) but not on the web (autoplay policy).
        let native_auto = self.config.auto_start && !cfg!(target_arch = "wasm32");
        self.phase = match AfterLoad::decide(autostart, native_auto) {
            AfterLoad::RunNow => {
                self.started_once = true;
                let mut shell = shell;
                shell.start_new_game();
                Phase::Running(shell)
            }
            AfterLoad::Splash => {
                self.started_once = true;
                self.audio.resume_after_user_gesture();
                self.begin_splash(shell, true)
            }
            AfterLoad::Ready => Phase::Ready(shell),
        };
        self.after_shell();
    }

    fn build_shell(&mut self, story: Arc<Story>, translations: &[String]) -> Box<Shell> {
        self.build_shell_full(story, translations, Vec::new(), None, None)
    }

    /// Starts the splash sequence in front of `shell`: always the engine
    /// splash first, then the project's. The only way to reach the
    /// running game from a loaded story other than hot reload and the
    /// editor preview.
    fn begin_splash(&mut self, shell: Box<Shell>, start_game: bool) -> Phase {
        let state = SplashState::new(&self.ui_splashes);
        self.splash_assets
            .request(&self.assets_loader, state.steps());
        Phase::Splash(Box::new(SplashPhase::new(shell, state, start_game)))
    }

    /// Advances the splashes; hands over to the running game when done.
    fn update_splash(&mut self, dt: f32) {
        if !matches!(self.phase, Phase::Splash(_)) {
            return;
        }
        let placeholder = Phase::Failed(String::new());
        if let Phase::Splash(sp) = std::mem::replace(&mut self.phase, placeholder) {
            self.phase = match sp.update(dt) {
                Ok(shell) => Phase::Running(shell),
                Err(sp) => Phase::Splash(Box::new(sp)),
            };
        }
    }

    /// The UI bundle to use: given by the config, else read from `ui_dir`,
    /// else the one of the game pack. A bundle with errors is dropped (the
    /// stock screens stay).
    fn resolve_ui(&self, pack_ui: Option<UiBundle>) -> Option<UiBundle> {
        if let Some(bundle) = &self.config.ui_bundle {
            return crate::loading::check_ui_bundle(bundle.clone());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = &self.config.ui_dir {
            return match UiBundle::load_dir(std::path::Path::new(dir)) {
                Ok(bundle) => crate::loading::check_ui_bundle(bundle),
                Err(err) => {
                    log::warn!("cannot read the UI folder: {err}");
                    None
                }
            };
        }
        pack_ui
    }

    /// The shell of a loaded story: `translations` are RON texts (files next
    /// to a RON story), `packed` the translations of a game pack, `info` its
    /// identity (the id names the saves, the language is the default one).
    fn build_shell_full(
        &mut self,
        story: Arc<Story>,
        translations: &[String],
        packed: Vec<Translation>,
        info: Option<GameInfo>,
        pack_ui: Option<UiBundle>,
    ) -> Box<Shell> {
        let mut loc = Localizer::from_story(&story);
        for text in translations {
            if let Err(err) = loc.add_translation_ron(text) {
                log::warn!("translation ignored: {err}");
            }
        }
        for translation in packed {
            if let Err(err) = loc.add_translation(translation) {
                log::warn!("translation ignored: {err}");
            }
        }
        let game_id = self
            .config
            .game_id
            .clone()
            .or_else(|| info.as_ref().map(|i| i.id.clone()))
            .unwrap_or_else(|| slug(&story.title));
        let embedded = self.embedded.is_some();
        // The preview never touches the real saves of the game.
        let (storage, storage_ok) = if embedded {
            (Box::new(MemoryStorage::new()) as Box<dyn Storage>, true)
        } else {
            open_storage(&self.config, &game_id)
        };
        let cfg = ShellConfig {
            save: self.config.save,
            autosave: self.config.autosave,
            ui_overrides: self.config.ui_text_overrides.clone(),
            system_language: orbit_platform::system_language(),
            utc_offset_minutes: orbit_platform::local_utc_offset_minutes(),
            default_language: info.as_ref().and_then(|i| i.default_language.clone()),
            fullscreen_supported: orbit_platform::FULLSCREEN_SUPPORTED && !embedded,
            quit_supported: orbit_platform::QUIT_SUPPORTED && !embedded,
            capture_thumbnails: !embedded,
            embedded,
        };
        // The editor preview has no pack: it shows the project's `ui_dir`
        // (set by the host), if any.
        let ui = self.resolve_ui(pack_ui);
        self.ui_splashes = ui.as_ref().map_or_else(Vec::new, |b| b.splashes.clone());
        if let Some(theme) = ui.as_ref().and_then(|b| b.theme.clone()) {
            self.ctx.theme = theme.clone();
            self.config.theme = theme;
            self.ui_sounds = UiSoundBank::new(&self.config.theme, &self.assets_loader);
        }
        let mut shell = Box::new(Shell::new(
            story.clone(),
            storage,
            loc,
            self.ctx.theme.clone(),
            cfg,
        ));
        shell.set_plugin_host(self.config.plugins.clone());
        if let Some(bundle) = &ui {
            shell.set_ui_bundle(bundle);
        }
        if !storage_ok {
            shell.warn_storage_unavailable();
        }
        shell.set_menu_background(menu_background(&story, &self.store));
        let mut images = theme_images(&self.ctx.theme, &self.store);
        images.extend(gallery_images(&story, &self.store));
        shell.set_ui_images(images);
        shell
    }

    /// Reloads the story file (hot reload): validates it again and restarts
    /// from the beginning.
    fn reload(&mut self) {
        log::info!("story file changed: reloading");
        self.audio.stop_music(RESTART_MUSIC_FADE);
        let handle = load_story(&self.story_loader, &self.story_path);
        let translations = TranslationLoad::start(
            &self.story_loader,
            &translations_path(&self.story_path, &self.config.translations_dir),
        );
        self.screens.error_body = None;
        self.phase = Phase::Loading(LoadingState::new(
            handle,
            Some(translations),
            self.started_once,
        ));
    }

    fn shell_mut(&mut self) -> Option<&mut Shell> {
        match &mut self.phase {
            Phase::Running(shell) | Phase::Ready(shell) => Some(shell),
            Phase::Splash(sp) => Some(&mut sp.shell),
            _ => None,
        }
    }

    /// Carries out what the shell asked for (audio, volumes, window), and
    /// turns a failed story into the error screen.
    fn after_shell(&mut self) {
        let (effects, story, failure) = match self.shell_mut() {
            Some(shell) => (
                shell.take_effects(),
                shell.story().clone(),
                shell.failure().map(str::to_owned),
            ),
            None => return,
        };
        for effect in effects {
            match effect {
                Effect::Audio(command) => {
                    if let Some(command) = self.filter_audio(command) {
                        play_command(&mut self.audio, &self.store, &story, command);
                    }
                }
                Effect::UiSound(sound) => self.ui_sounds.play(sound, &mut self.audio),
                Effect::SetVolume(bus, volume) => self.set_volume(bus, volume),
                Effect::SetFullscreen(on) => {
                    self.host_requests.push(HostRequest::SetFullscreen(on))
                }
                Effect::PlayAsset(path) => match self.store.sound(&path) {
                    Some(sound) => self.audio.play_sfx(sound),
                    None => log::warn!("sound `{path}` is not available"),
                },
                Effect::Custom { name, args } => {
                    // Plugins already had their chance (shell); this one
                    // is for an embedding host.
                    log::info!("custom UI action `{name}` {args:?} has no handler");
                }
                Effect::Quit => self.host_requests.push(HostRequest::Quit),
            }
        }
        if let Some(message) = failure {
            self.phase = Phase::Failed(message);
        }
    }

    fn set_volume(&mut self, bus: AudioBus, volume: f32) {
        match &mut self.embedded {
            // The host mutes and pauses through the master bus.
            Some(embedded) if bus == AudioBus::Master => {
                embedded.master = volume;
                self.apply_embedded_volume();
            }
            _ => self.audio.set_bus_volume(bus_of(bus), volume),
        }
    }

    /// Lazy loading (web): finishes the assets that arrived, asks for the
    /// assets of the scene on screen and of the scenes it leads to, and
    /// holds the game while the scene on screen is still missing something.
    fn poll_streaming(&mut self) {
        let Some(stream) = self.streaming.as_mut() else {
            return;
        };
        if stream.in_flight.iter().any(PendingAsset::is_done) {
            let (done, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut stream.in_flight)
                .into_iter()
                .partition(PendingAsset::is_done);
            stream.in_flight = waiting;
            finish_loading(&mut self.ctx.renderer, &mut self.store, done);
        }
        let (story, current) = match &self.phase {
            Phase::Running(shell) | Phase::Ready(shell) => {
                (shell.story().clone(), shell.current_scene())
            }
            Phase::Splash(sp) => (sp.shell.story().clone(), sp.shell.current_scene()),
            _ => return,
        };
        let first = story
            .entry()
            .and_then(|node| story.locate(node))
            .map_or(0, |(scene, _)| scene);
        let scene = current.unwrap_or(first);
        if stream.last_scene != Some(scene) {
            stream.last_scene = Some(scene);
            let mut wanted = vec![scene];
            wanted.extend(successor_scenes(&story, scene));
            for next in wanted {
                if !stream.requested.insert(next) {
                    continue;
                }
                let paths: Vec<String> = scene_asset_paths(&story, next)
                    .into_iter()
                    .filter(|p| !stream.in_flight.iter().any(|f| f.path() == p))
                    .collect();
                let started =
                    start_loading_only(&self.assets_loader, &story, &self.store, Some(&paths));
                if !started.is_empty() {
                    log::info!(
                        "lazy loading: scene {next}, {} asset(s) requested",
                        started.len()
                    );
                }
                stream.in_flight.extend(started);
            }
        }
        stream.blocked = current.is_some() && !stream.in_flight.is_empty() && {
            let needed = scene_asset_paths(&story, scene);
            stream
                .in_flight
                .iter()
                .any(|p| needed.iter().any(|n| n == p.path()))
        };
    }

    /// Advances the animation clock, unless the game is held (paused by a
    /// screen, waiting for assets).
    fn advance_anims(&mut self, dt: f32) {
        let held = self.is_blocked();
        if let Phase::Running(shell) = &self.phase
            && !held
            && !shell.game_paused()
        {
            self.anim_clock.advance(dt);
        }
    }

    /// Whether the game is held until the scene on screen has its assets.
    fn is_blocked(&self) -> bool {
        self.streaming.as_ref().is_some_and(|s| s.blocked)
    }

    /// Renders the stages of the pending saves into the thumbnail target and
    /// collects the finished read-backs.
    fn service_thumbnails(&mut self) {
        let Phase::Running(shell) = &mut self.phase else {
            return;
        };
        let renderer = &mut self.ctx.renderer;
        for request in shell.take_capture_requests() {
            let mut scene = Scene::new();
            self.layer3d.begin_frame();
            let layer3d = self
                .layer3d
                .draw(renderer, &self.store, shell.story(), &request.stage);
            stage_scene(
                renderer,
                &self.store,
                shell.story(),
                &request.stage,
                None,
                layer3d,
                None,
                &mut scene,
            );
            let started = renderer
                .render_to_target(self.thumbnail_target, &scene)
                .and_then(|()| renderer.start_readback(self.thumbnail_target));
            match started {
                Ok(readback) => self.captures.push((request.id, readback)),
                Err(err) => {
                    log::warn!("thumbnail capture failed: {err}");
                    shell.complete_capture(request.id, None);
                }
            }
        }
        let mut pending = Vec::new();
        for (id, mut readback) in std::mem::take(&mut self.captures) {
            match renderer.poll_readback(&mut readback) {
                ReadbackStatus::Pending => pending.push((id, readback)),
                ReadbackStatus::Ready {
                    width,
                    height,
                    rgba,
                } => shell.complete_capture(id, encode_png(width, height, &rgba)),
                ReadbackStatus::Failed => shell.complete_capture(id, None),
            }
        }
        self.captures = pending;

        for request in shell.take_thumbnail_requests() {
            let texture = decode_image(&request.png).ok().and_then(|img| {
                let name = format!("save-thumb/{:?}/{}", request.key.0, request.key.1);
                renderer
                    .create_texture(
                        &name,
                        img.width,
                        img.height,
                        &img.rgba,
                        TextureOptions::COLOR,
                    )
                    .ok()
            });
            shell.set_thumbnail(request.key, texture);
        }
        for texture in shell.take_stale_textures() {
            renderer.release_texture(texture);
        }
    }

    // --------------------------------------------------------------- input

    fn on_action(&mut self, action: InputAction) {
        if matches!(
            action,
            InputAction::Ui(UiInput::Confirm | UiInput::PointerDown(_))
                | InputAction::Number(_)
                | InputAction::Hotkey(_)
        ) {
            // Must happen inside the input handler: browsers only unlock
            // audio from a user gesture.
            self.audio.resume_after_user_gesture();
        }
        if let Phase::Splash(sp) = &mut self.phase {
            sp.input(&action);
            return;
        }
        let start = matches!(
            action,
            InputAction::Ui(UiInput::Confirm | UiInput::PointerDown(_))
        );
        if start && matches!(self.phase, Phase::Ready(_)) {
            let placeholder = Phase::Failed(String::new());
            if let Phase::Ready(mut shell) = std::mem::replace(&mut self.phase, placeholder) {
                self.started_once = true;
                let start_game = self.config.auto_start;
                self.phase = if self.embedded.is_some() {
                    if start_game {
                        shell.start_new_game();
                    }
                    Phase::Running(shell)
                } else {
                    self.begin_splash(shell, start_game)
                };
            }
        } else if self.is_blocked() {
            // The scene is still loading: nothing to click on yet.
        } else if let Phase::Running(shell) = &mut self.phase {
            shell.input(action);
        }
        self.after_shell();
    }

    // ------------------------------------------------------------- drawing

    fn draw(&mut self, out: &mut Output<'_>) -> Result<(), RenderError> {
        if self
            .embedded
            .as_ref()
            .is_some_and(|e| e.ui_preview.is_some())
        {
            return self.draw_ui_preview(out);
        }
        let language = match &self.phase {
            Phase::Ready(shell) | Phase::Running(shell) => shell.language().to_owned(),
            _ => self.screens_language.clone(),
        };
        if language != self.screens_language {
            self.screens = Screens::default();
            self.screens_language = language;
            self.boot_texts = Texts::builtin(&self.screens_language);
        }
        // M7-02: the language selects the font chain of every text (dialogue,
        // choices, widgets) and the text direction.
        self.ctx.text_engine.set_language(&self.screens_language);
        self.ctx
            .text_engine
            .set_language_fonts(self.ctx.theme.language_font_families());
        let Ctx {
            renderer,
            text_engine,
            text_renderer,
            white,
            theme,
        } = &mut self.ctx;
        let fonts_ready = self.fonts_ready;
        let time = self.time;
        if let Phase::Splash(sp) = &self.phase {
            let scene = self.splash_assets.scene(
                &sp.state,
                renderer,
                text_engine,
                text_renderer,
                fonts_ready,
            );
            return out.scene(renderer, &scene);
        }
        let texts = match &self.phase {
            Phase::Ready(shell) | Phase::Running(shell) => shell.texts().clone(),
            _ => self.boot_texts.clone(),
        };
        match &mut self.phase {
            Phase::Running(shell) => {
                let mut transitioning = false;
                match shell.game() {
                    Some(game) => {
                        let mut scene = Scene::new();
                        let fx = game.director.stage_fx();
                        self.anim_clock
                            .set_reduced_motion(game.director.reduced_motion());
                        match game.director.transition() {
                            Some(tr) => self.anim_clock.sync([&tr.from, game.director.stage()]),
                            None => self.anim_clock.sync([game.director.stage()]),
                        }
                        self.layer3d.begin_frame();
                        if let Some(tr) = game.director.transition() {
                            transitioning = true;
                            // 2D <-> 3D and camera changes blend like any
                            // stage change: each side has its own 3D view.
                            let from_3d =
                                self.layer3d
                                    .draw(renderer, &self.store, &game.story, &tr.from);
                            let to_3d = self.layer3d.draw(
                                renderer,
                                &self.store,
                                &game.story,
                                game.director.stage(),
                            );
                            let mut from = Scene::new();
                            stage_scene(
                                renderer,
                                &self.store,
                                &game.story,
                                &tr.from,
                                Some(&self.anim_clock),
                                from_3d,
                                Some((&fx, *white)),
                                &mut from,
                            );
                            stage_scene(
                                renderer,
                                &self.store,
                                &game.story,
                                game.director.stage(),
                                Some(&self.anim_clock),
                                to_3d,
                                Some((&fx, *white)),
                                &mut scene,
                            );
                            renderer.render_to_target(self.targets[0], &from)?;
                            renderer.render_to_target(self.targets[1], &scene)?;
                            out.transition(
                                renderer,
                                self.targets[0],
                                self.targets[1],
                                &render_transition(tr.kind),
                                tr.progress,
                            )?;
                        } else {
                            let layer3d = self.layer3d.draw(
                                renderer,
                                &self.store,
                                &game.story,
                                game.director.stage(),
                            );
                            stage_scene(
                                renderer,
                                &self.store,
                                &game.story,
                                game.director.stage(),
                                Some(&self.anim_clock),
                                layer3d,
                                Some((&fx, *white)),
                                &mut scene,
                            );
                            out.scene(renderer, &scene)?;
                        }
                    }
                    None => {
                        let mut scene = Scene::new();
                        scene.clear = Some(tint(theme.screens.background));
                        out.scene(renderer, &scene)?;
                    }
                }
                if fonts_ready {
                    // The UI goes in its own pass over the stage (or over the
                    // transition), so menus and toasts are never hidden by it.
                    let mut ui_scene = Scene::new();
                    let mut ctx = DrawCtx {
                        engine: text_engine,
                        text_renderer,
                        renderer,
                        scene: &mut ui_scene,
                        white: *white,
                    };
                    {
                        orbit_profile::scope!("ui draw");
                        shell.draw_ui(&mut ctx, !transitioning);
                    }
                    out.scene(renderer, &ui_scene)?;
                }
                if self.streaming.as_ref().is_some_and(|s| s.blocked) {
                    // The scene waits for its assets: dim, text and a bar.
                    let s = &theme.screens;
                    let mut wait = Scene::new();
                    wait.push(
                        Sprite2d::new(*white, [0.0, 0.0], CANVAS)
                            .anchor([0.0, 0.0])
                            .layer(Layer::UI)
                            .z(-10.0)
                            .tint(tint([0, 0, 0, 170])),
                    );
                    draw_progress_bar(
                        &mut wait,
                        *white,
                        [CANVAS[0] * 0.5, CANVAS[1] * 0.58],
                        s.bar_size,
                        s.bar_background,
                        s.bar_fill,
                        (time * 0.8).fract(),
                    );
                    if fonts_ready {
                        let banner = self.screens.loading.get_or_insert_with(|| {
                            Banner::centered(&texts.get("ui.loading"), s.body_size, s.text_color)
                        });
                        banner.draw(
                            text_engine,
                            text_renderer,
                            renderer,
                            &mut wait,
                            CANVAS[1] * 0.47,
                            0.9,
                        );
                    }
                    out.scene(renderer, &wait)?;
                }
                Ok(())
            }
            phase => {
                let s = &theme.screens;
                let mut scene = Scene::new();
                scene.clear = Some(tint(s.background));
                match phase {
                    Phase::Opening(_) | Phase::Loading(_) => {
                        let fonts_total = self.font_handles.len();
                        let (total, done) = match phase {
                            Phase::Loading(l) => (
                                fonts_total + 1 + l.assets_total,
                                if self.fonts_ready { fonts_total } else { 0 }
                                    + usize::from(l.loaded.is_some())
                                    + l.assets_total.saturating_sub(
                                        l.pending.iter().filter(|p| !p.is_done()).count(),
                                    ),
                            ),
                            _ => (1, 0),
                        };
                        draw_progress_bar(
                            &mut scene,
                            *white,
                            [CANVAS[0] * 0.5, CANVAS[1] * 0.58],
                            s.bar_size,
                            s.bar_background,
                            s.bar_fill,
                            done as f32 / total as f32,
                        );
                        if fonts_ready {
                            let banner = self.screens.loading.get_or_insert_with(|| {
                                Banner::centered(
                                    &texts.get("ui.loading"),
                                    s.body_size,
                                    s.text_color,
                                )
                            });
                            banner.draw(
                                text_engine,
                                text_renderer,
                                renderer,
                                &mut scene,
                                CANVAS[1] * 0.47,
                                0.9,
                            );
                        }
                    }
                    Phase::Ready(shell) => {
                        if fonts_ready {
                            let story_title = shell.story().title.clone();
                            let title = match &mut self.screens.title {
                                Some((t, b)) if *t == story_title => b,
                                slot => {
                                    &mut slot
                                        .insert((
                                            story_title.clone(),
                                            Banner::centered(
                                                &escape_markup(&story_title),
                                                s.title_size,
                                                s.text_color,
                                            ),
                                        ))
                                        .1
                                }
                            };
                            title.draw(
                                text_engine,
                                text_renderer,
                                renderer,
                                &mut scene,
                                CANVAS[1] * 0.36,
                                1.0,
                            );
                            let start = self.screens.start.get_or_insert_with(|| {
                                Banner::centered(
                                    &texts.get("ui.click_to_start"),
                                    s.body_size * 1.2,
                                    s.text_color,
                                )
                            });
                            let pulse = 0.65 + 0.35 * (time * 3.0).sin();
                            start.draw(
                                text_engine,
                                text_renderer,
                                renderer,
                                &mut scene,
                                CANVAS[1] * 0.58,
                                pulse,
                            );
                        }
                    }
                    Phase::Failed(message) => {
                        if fonts_ready {
                            let title = self.screens.error_title.get_or_insert_with(|| {
                                Banner::left(
                                    &texts.get("ui.error.title"),
                                    s.title_size * 0.65,
                                    s.error_color,
                                    110.0,
                                    1700.0,
                                )
                            });
                            title.draw(text_engine, text_renderer, renderer, &mut scene, 90.0, 1.0);
                            let body = match &mut self.screens.error_body {
                                Some((m, b)) if m == message => b,
                                slot => {
                                    &mut slot
                                        .insert((
                                            message.clone(),
                                            Banner::left(
                                                &error_markup(message, MAX_ERROR_LINES),
                                                s.body_size,
                                                s.text_color,
                                                110.0,
                                                1700.0,
                                            ),
                                        ))
                                        .1
                                }
                            };
                            body.draw(text_engine, text_renderer, renderer, &mut scene, 220.0, 1.0);
                            let hint = self.screens.error_hint.get_or_insert_with(|| {
                                Banner::left(
                                    &texts.get("ui.error.hint"),
                                    s.body_size * 0.85,
                                    [150, 158, 178, 255],
                                    110.0,
                                    1700.0,
                                )
                            });
                            hint.draw(
                                text_engine,
                                text_renderer,
                                renderer,
                                &mut scene,
                                CANVAS[1] - 100.0,
                                1.0,
                            );
                        }
                    }
                    Phase::Running(_) | Phase::Splash(_) | Phase::Stopped => {}
                }
                out.scene(renderer, &scene)
            }
        }
    }
}

/// The picture shown behind the main menu: the first background of the story.
fn menu_background(story: &Story, store: &AssetStore) -> Option<TextureId> {
    menu_background_path(story).and_then(|path| store.texture(path))
}

/// Textures of the nine-slice images the styles of `theme` use (the ones
/// already loaded in `store`), by asset path.
pub(crate) fn theme_images(
    theme: &orbit_ui::Theme,
    store: &AssetStore,
) -> std::collections::HashMap<String, TextureId> {
    theme
        .styles
        .values()
        .filter_map(|style| style.nine_slice().map(|(path, _)| path))
        .filter_map(|path| store.texture(path).map(|t| (path.to_owned(), t)))
        .collect()
}

/// Textures of the gallery images (M7-03) that are loaded, by path.
pub(crate) fn gallery_images(
    story: &Story,
    store: &AssetStore,
) -> std::collections::HashMap<String, TextureId> {
    crate::shell::gallery_paths(story)
        .into_iter()
        .filter_map(|path| store.texture(&path).map(|t| (path, t)))
        .collect()
}

/// Manifest path of the picture behind the main menu.
fn menu_background_path(story: &Story) -> Option<&str> {
    story
        .scenes
        .iter()
        .flat_map(|scene| &scene.nodes)
        .find_map(|node| match &node.kind {
            orbit_core::NodeKind::Background { asset, .. } => Some(*asset),
            orbit_core::NodeKind::BackgroundLayers { layers, .. } => layers.first().map(|l| l.asset),
            _ => None,
        })
        .and_then(|asset| story.assets.get(asset))
        .map(|entry| entry.path.as_str())
}

/// What the lazy mode loads before the start: the assets of the first scene
/// and the picture behind the main menu.
fn boot_paths(story: &Story) -> Vec<String> {
    let first = story
        .entry()
        .and_then(|node| story.locate(node))
        .map_or(0, |(scene, _)| scene);
    let mut paths = scene_asset_paths(story, first);
    if let Some(menu) = menu_background_path(story)
        && !paths.iter().any(|p| p == menu)
    {
        paths.push(menu.to_owned());
    }
    paths
}

fn load_story(loader: &AssetLoader, path: &str) -> AssetHandle<LoadedStory> {
    loader.load_with(path, |_, bytes| parse_story(bytes))
}

fn describe_error(err: &AssetError) -> String {
    match err {
        AssetError::Io { path, message } => format!("Cannot read `{path}`: {message}"),
        AssetError::Decode { path, message } => format!("`{path}`: {message}"),
        AssetError::InvalidPath(path) => format!("Invalid story path `{path}`"),
    }
}

/// Escapes the `orbit_text` tag syntax so `message` is shown literally.
pub fn escape_markup(message: &str) -> String {
    message.replace('{', "{{").replace('}', "}}")
}

/// A message prepared for the error screen: literal text, at most
/// `max_lines` lines (the rest is summarized).
pub fn error_markup(message: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = message.lines().collect();
    let mut shown: Vec<String> = lines
        .iter()
        .take(max_lines)
        .map(|l| escape_markup(l))
        .collect();
    if lines.len() > max_lines {
        shown.push(format!(
            "\u{2026} (+{} more lines)",
            lines.len() - max_lines
        ));
    }
    shown.join("\n")
}

/// Story transition kind to renderer transition (colors are sRGB bytes in
/// the story, linear in the renderer).
fn render_transition(kind: TransitionKind) -> Transition {
    match kind {
        TransitionKind::Fade(c) => Transition::Fade {
            color: [
                srgb_to_linear(c.r),
                srgb_to_linear(c.g),
                srgb_to_linear(c.b),
                1.0,
            ],
        },
        TransitionKind::Dissolve => Transition::Dissolve,
    }
}

/// What to draw for a character: texture, sub-rectangle (sprite sheet cell)
/// and the size in pixels of what is shown (cell or whole image).
struct CharacterFrame {
    texture: TextureId,
    uv: Option<[f32; 4]>,
    size: [u32; 2],
    /// Size of the animation's first frame (`size` for a still): the
    /// character's standard height is that of this frame.
    reference: [u32; 2],
}

/// Picks the frame of `c` at its animation time: the current cell / image of
/// its animation, else (still expression, rigged, frames not loaded) its
/// poster image. `None` when nothing can be drawn yet.
fn character_frame(
    renderer: &Renderer2d,
    store: &AssetStore,
    story: &Story,
    c: &StageCharacter,
    anims: Option<&AnimClock>,
) -> Option<CharacterFrame> {
    let texture = |id| {
        let path = &story.assets.get(id)?.path;
        store.texture(path)
    };
    let animation = story
        .character(c.character)
        .and_then(|ch| ch.expression(&c.expression))
        .and_then(|e| e.animation());
    let elapsed = anims.map_or(0.0, |a| a.elapsed(c.character, &c.expression));
    let load = |a: &orbit_core::Animation, frame: FrameRef| match frame {
        FrameRef::Image(id) => {
            let tex = texture(id)?;
            let size = renderer.texture_size(tex)?;
            Some(CharacterFrame {
                texture: tex,
                uv: None,
                size,
                reference: size,
            })
        }
        FrameRef::Sheet { asset, index } => {
            let tex = texture(asset)?;
            let sheet = renderer.texture_size(tex)?;
            let AnimFrames::SpriteSheet {
                cols,
                rows,
                frame_size,
                ..
            } = &a.frames
            else {
                return None;
            };
            let size = sheet_cell_size(*cols, *rows, *frame_size, sheet);
            Some(CharacterFrame {
                texture: tex,
                uv: Some(sheet_uv(index, *cols, *rows, *frame_size, sheet)?),
                size,
                reference: size,
            })
        }
    };
    let animated = animation
        .and_then(|a| a.frame_at(elapsed).map(|f| (a, f)))
        .and_then(|(a, frame)| {
            let mut shown = load(a, frame)?;
            // Frames of a sequence may differ in size: they are laid out
            // relative to the first frame (whose size sets the character's
            // scale), not each stretched to the standard height.
            if let Some(first) = a.frame_at(0.0).and_then(|f| load(a, f)) {
                shown.reference = first.size;
            }
            Some(shown)
        });
    if animated.is_some() {
        return animated;
    }
    if matches!(
        animation.map(|a| &a.frames),
        Some(AnimFrames::SpriteSheet { .. })
    ) {
        // The poster of a sheet is usually the sheet itself: not drawable.
        let poster = story
            .character(c.character)
            .and_then(|ch| ch.expression(&c.expression))
            .map(|e| e.image);
        let sheet = animation.and_then(|a| a.frames.assets().first().copied());
        if poster == sheet {
            return None;
        }
    }
    let tex = texture(c.image)?;
    let size = renderer.texture_size(tex)?;
    Some(CharacterFrame {
        texture: tex,
        uv: None,
        size,
        reference: size,
    })
}

/// Fills `scene` with the stage: opaque black, background (covering the
/// canvas), then the characters bottom-center anchored at their position.
/// `anims` gives the animation times (`None`: every animation on frame 0).
/// When the stage shows a 3D scene, `layer3d` (its rendered view, see
/// [`crate::compose3d::Layer3d`]) takes the background's place; while it is
/// not available the background layer stays black. Without a 3D scene this
/// is exactly the 2D stage.
/// `fx` is what the running effects and timelines add on top of the staging,
/// with the white texture the screen overlay (flash, tint) is drawn with
/// (`None`: the staging as is).
#[allow(clippy::too_many_arguments)]
pub(crate) fn stage_scene(
    renderer: &Renderer2d,
    store: &AssetStore,
    story: &Story,
    stage: &StageState,
    anims: Option<&AnimClock>,
    layer3d: Option<TextureId>,
    fx: Option<(&StageFx, TextureId)>,
    scene: &mut Scene,
) {
    let texture = |id| {
        let path = &story.assets.get(id)?.path;
        store.texture(path)
    };
    let identity = StageFx::default();
    let stage_fx = fx.map_or(&identity, |(f, _)| f);
    scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
    if stage.scene3d.is_some() {
        if let Some(tex) = layer3d {
            scene.push(
                Sprite2d::new(tex, [CANVAS[0] * 0.5, CANVAS[1] * 0.5], CANVAS)
                    .anchor([0.5, 0.5])
                    .layer(Layer::BACKGROUND),
            );
        }
    } else if !stage.bg_layers.is_empty() {
        for (i, layer) in stage.bg_layers.iter().enumerate() {
            let Some(tex) = texture(layer.asset) else { continue };
            // A little larger than the screen, so a gentle pan never shows
            // the edge of a layer that moves less than the camera.
            let size = renderer
                .texture_size(tex)
                .map_or(CANVAS, cover_size)
                .map(|v| v * LAYER_OVERSCAN);
            let (position, size, tint) = animate_layer(size, layer.depth, stage_fx);
            let centers = if layer.repeat_x {
                tiled_centers(position[0], size[0])
            } else {
                vec![position[0]]
            };
            for x in centers {
                scene.push(
                    Sprite2d::new(tex, [x, position[1]], size)
                        .anchor([0.5, 0.5])
                        .tint(tint)
                        .layer(Layer::BACKGROUND)
                        .z(i as f32),
                );
            }
        }
    } else if let Some(tex) = stage.background.and_then(texture) {
        let size = renderer.texture_size(tex).map_or(CANVAS, cover_size);
        let (position, size, tint) = animate_background(size, stage_fx);
        scene.push(
            Sprite2d::new(tex, position, size)
                .anchor([0.5, 0.5])
                .tint(tint)
                .layer(Layer::BACKGROUND),
        );
    }
    for (i, c) in stage.characters.iter().enumerate() {
        if crate::rigs::push_rigged_character(
            renderer, store, story, c, anims, stage_fx, i as f32, scene,
        ) {
            continue;
        }
        let Some(frame) = character_frame(renderer, store, story, c, anims) else {
            continue;
        };
        let placement = character_placement(c, frame.reference);
        let animated = animate_placement(&placement, c.character, stage_fx);
        let fs = frame_scale(frame.size, frame.reference);
        let size = [placement.size[0] * fs[0], placement.size[1] * fs[1]];
        let mut sprite = Sprite2d::new(frame.texture, animated.anchor, size)
            .anchor([0.5, 1.0])
            .scale(animated.scale)
            .tint(animated.tint);
        sprite.rotation = animated.rotation;
        if let Some(uv) = frame.uv {
            sprite = sprite.uv(uv);
        }
        scene.push(
            sprite
                .flipped(placement.flip_x, placement.flip_y)
                .layer(Layer::CHARACTERS)
                .z(i as f32),
        );
    }
    if let Some((f, white)) = fx {
        // Particles: behind the characters (layer 5) or over them (layer 12),
        // always under the screen overlay and the UI.
        for (sprite, front, quads) in particle_quads(f) {
            let tex = sprite.and_then(texture).unwrap_or(white);
            let layer = if front { Layer(12) } else { Layer(5) };
            for q in quads {
                scene.push(
                    Sprite2d::new(tex, q.position, q.size)
                        .anchor([0.5, 0.5])
                        .tint(q.tint)
                        .rotation(q.rotation)
                        .layer(layer),
                );
            }
        }
    }
    if let Some((f, white)) = fx
        && let Some(color) = screen_overlay(f)
    {
        // Over the characters, under the UI.
        scene.push(
            Sprite2d::new(white, [CANVAS[0] * 0.5, CANVAS[1] * 0.5], CANVAS)
                .anchor([0.5, 0.5])
                .tint(color)
                .layer(Layer(15)),
        );
    }
}

fn secs(seconds: f32) -> Duration {
    Duration::try_from_secs_f32(seconds).unwrap_or(Duration::ZERO)
}

fn play_command(audio: &mut AudioEngine, store: &AssetStore, story: &Story, command: AudioCommand) {
    let sound = |id| {
        let path = &story.assets.get(id)?.path;
        let found = store.sound(path);
        if found.is_none() {
            log::warn!("sound `{path}` is not available");
        }
        found
    };
    match command {
        AudioCommand::PlayMusic {
            asset,
            fade_in,
            looping,
        } => {
            if let Some(music) = sound(asset) {
                let fade = secs(fade_in);
                audio.play_music(
                    music,
                    MusicOptions {
                        looping,
                        fade_in: fade,
                        fade_out_previous: fade,
                    },
                );
            }
        }
        AudioCommand::StopMusic { fade_out } => audio.stop_music(secs(fade_out)),
        AudioCommand::PlaySound { asset } => {
            if let Some(effect) = sound(asset) {
                audio.play_sfx(effect);
            }
        }
    }
}

impl App for Runtime {
    fn update(&mut self, dt: Duration) {
        // Hosted by the editor, frames are the editor's.
        if self.embedded.is_none() {
            orbit_profile::begin_frame();
        }
        orbit_profile::scope!("runtime update");
        let dt_secs = dt.as_secs_f32().min(MAX_DT);
        if let Some(overlay) = &mut self.overlay {
            overlay.update(dt_secs);
        }
        self.time += dt_secs;
        self.audio.update();
        self.poll_opening();
        self.poll_fonts();
        if let Some(watcher) = &mut self.watcher
            && watcher.poll(dt)
        {
            self.reload();
        }
        self.poll_loading();
        for bytes in self.store.take_new_fonts() {
            if self.ctx.text_engine.add_font_data(bytes) == 0 {
                log::warn!("a font asset could not be loaded");
            }
        }
        let blocked = self.is_blocked();
        self.update_splash(dt_secs);
        if let Phase::Running(shell) = &mut self.phase {
            shell.set_now(orbit_platform::unix_time_ms());
            if !blocked {
                orbit_profile::scope!("shell update");
                shell.update(dt_secs);
            }
        }
        self.advance_anims(dt_secs);
        self.poll_streaming();
        self.service_thumbnails();
        self.after_shell();
    }

    fn render(&mut self) {
        if self.hidden {
            return;
        }
        let mut frame = match self.ctx.renderer.begin_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(err) => {
                log::error!("begin_frame: {err}");
                return;
            }
        };
        {
            orbit_profile::scope!("runtime draw");
            if let Err(err) = self.draw(&mut Output::Surface(&mut frame)) {
                log::error!("render error: {err}");
            }
        }
        if let Some(overlay) = &mut self.overlay
            && self.fonts_ready
        {
            let Ctx {
                renderer,
                text_engine,
                text_renderer,
                white,
                ..
            } = &mut self.ctx;
            if let Some(scene) = overlay.scene(text_engine, text_renderer, renderer, *white)
                && let Err(err) = renderer.draw_scene(&mut frame, &scene)
            {
                log::error!("profiler overlay: {err}");
            }
        }
        self.ctx.renderer.end_frame(frame);
        if self.embedded.is_none() {
            orbit_profile::finish_frame();
        }
    }

    fn resize(&mut self, size: PhysicalSize<u32>, _scale_factor: f64) {
        self.ctx.renderer.resize(size.width, size.height);
    }

    fn input(&mut self, event: &InputEvent) {
        // F3: profiler overlay (development builds; never in the editor's
        // preview, which has its own profiler panel).
        if crate::overlay::ENABLED
            && self.embedded.is_none()
            && matches!(
                event,
                InputEvent::Key {
                    code: orbit_platform::KeyCode::F3,
                    pressed: true
                }
            )
        {
            self.overlay = match self.overlay.take() {
                Some(_) => None,
                None => Some(crate::overlay::ProfilerOverlay::default()),
            };
            return;
        }
        if let Phase::Running(shell) = &self.phase {
            let bindings = GamepadBindings::from_prefs(&shell.prefs().gamepad);
            if &bindings != self.mapper.bindings() {
                self.mapper.set_bindings(bindings);
            }
        }
        let renderer = &self.ctx.renderer;
        if let Some(action) = self.mapper.map(event, |p| renderer.screen_to_world(p)) {
            self.on_action(action);
        }
    }

    fn close_requested(&mut self) -> bool {
        // Last autosave, preferences and persistent data.
        if let Phase::Running(shell) = &mut self.phase {
            shell.set_now(orbit_platform::unix_time_ms());
            shell.on_exit();
        }
        true
    }

    fn host_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.host_requests)
    }

    fn accessibility_tree(&mut self, force: bool) -> Option<orbit_platform::accesskit::TreeUpdate> {
        use orbit_ui::a11y::{A11yNode, A11yRole, A11yTree, LINE_ID, ROOT_ID};
        let tree = match &mut self.phase {
            Phase::Running(shell) => {
                let title = shell.story().title.clone();
                shell.a11y_tree(&title)
            }
            Phase::Failed(message) => {
                let mut tree = A11yTree::root("OrbitEngine");
                let text = orbit_ui::a11y::plain(message);
                tree.push(ROOT_ID, A11yNode::new(LINE_ID, A11yRole::Live, text));
                tree
            }
            Phase::Ready(shell) => {
                let mut tree = A11yTree::root(shell.story().title.clone());
                if shell.prefs().screen_reader {
                    let mut start = A11yNode::new(
                        orbit_ui::a11y::START_ID,
                        A11yRole::Button,
                        shell.texts().get("ui.click_to_start"),
                    );
                    start.bounds = [0.0, 0.0, orbit_ui::CANVAS[0], orbit_ui::CANVAS[1]];
                    tree.push(ROOT_ID, start);
                    tree.focus = orbit_ui::a11y::START_ID;
                }
                tree
            }
            _ => A11yTree::root("OrbitEngine"),
        };
        if !force && self.a11y_last.as_ref() == Some(&tree) {
            return None;
        }
        // Canvas → window pixels: the inverse of the letterboxed mapping the
        // pointer goes through.
        let renderer = &self.ctx.renderer;
        let o = renderer.screen_to_world([0.0, 0.0]);
        let u = renderer.screen_to_world([1.0, 1.0]);
        let scale = [u[0] - o[0], u[1] - o[1]];
        let to_window = |[x, y, w, h]: [f32; 4]| {
            let px = |v: f32, axis: usize| {
                if scale[axis].abs() > f32::EPSILON {
                    f64::from((v - o[axis]) / scale[axis])
                } else {
                    f64::from(v)
                }
            };
            [px(x, 0), px(y, 1), px(x + w, 0), px(y + h, 1)]
        };
        let update = tree.to_accesskit(to_window);
        self.a11y_last = Some(tree);
        Some(update)
    }

    fn accessibility_action(&mut self, request: orbit_platform::accesskit::ActionRequest) {
        let Some(action) = orbit_ui::a11y::A11yAction::from_accesskit(request.action) else {
            return;
        };
        let id = request.target_node.0;
        match &mut self.phase {
            Phase::Running(shell) => shell.a11y_action(id, action),
            Phase::Ready(_) if id == orbit_ui::a11y::START_ID && action == orbit_ui::a11y::A11yAction::Click => {
                self.on_action(InputAction::Ui(UiInput::Confirm));
            }
            _ => {}
        }
        self.after_shell();
    }

    fn visibility_changed(&mut self, state: Visibility) {
        // Silence and pause the music unless the player is looking at it.
        self.audio.set_paused(state != Visibility::Active);
        let hidden = state == Visibility::Hidden;
        if hidden
            && !self.hidden
            && let Phase::Running(shell) = &mut self.phase
        {
            // A hidden tab can be discarded at any moment: save now.
            shell.set_now(orbit_platform::unix_time_ms());
            shell.on_exit();
        }
        self.hidden = hidden;
    }

    fn suspended(&mut self) {
        // Android may kill a background app without warning: save now
        // (autosave, preferences, persistent data), then let the native
        // window go. Textures and pipelines stay on the device.
        self.visibility_changed(Visibility::Hidden);
        self.ctx.renderer.gfx_mut().suspend();
    }

    fn resumed(&mut self, size: PhysicalSize<u32>, _scale_factor: f64) {
        if let Err(err) = self
            .ctx
            .renderer
            .gfx_mut()
            .resume((size.width, size.height))
        {
            log::error!("cannot recreate the surface: {err}");
            return;
        }
        self.ctx.renderer.resize(size.width, size.height);
        self.visibility_changed(Visibility::Active);
    }

    fn safe_area_changed(&mut self, area: orbit_platform::SafeArea) {
        // The scene is letterboxed into the whole window for now; the insets
        // are recorded for UI layout (touch targets) to use.
        log::info!("safe area: {area:?}");
        self.safe_area = area;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translations_live_next_to_the_story_file() {
        assert_eq!(
            translations_path("examples/stories/demo.ron", "i18n"),
            "examples/stories/i18n"
        );
        assert_eq!(translations_path("demo.ron", "i18n"), "i18n");
    }

    #[test]
    fn translation_index_keeps_the_files_of_the_story() {
        let index = "# comment

demo.fr.ron
  demo.de.ron  
other.fr.ron
demo.txt
index.txt
";
        assert_eq!(
            parse_translation_index(index, "demo"),
            ["demo.fr.ron", "demo.de.ron"]
        );
        assert!(parse_translation_index("", "demo").is_empty());
    }

    #[test]
    fn markup_is_escaped_so_errors_show_literally() {
        assert_eq!(escape_markup("a {b} c"), "a {{b}} c");
        assert_eq!(escape_markup("plain"), "plain");
    }

    #[test]
    fn long_errors_are_truncated_with_a_count() {
        let message: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let shown = error_markup(&message, 5);
        assert_eq!(shown.lines().count(), 6);
        assert!(shown.ends_with("(+25 more lines)"));
        let short = error_markup("one\ntwo", 5);
        assert_eq!(short, "one\ntwo");
    }

    #[test]
    fn fade_color_is_converted_to_linear() {
        let t = render_transition(TransitionKind::Fade(orbit_core::Color::rgb(255, 0, 0)));
        assert_eq!(
            t,
            Transition::Fade {
                color: [1.0, 0.0, 0.0, 1.0]
            }
        );
        assert_eq!(
            render_transition(TransitionKind::Dissolve),
            Transition::Dissolve
        );
    }

    #[test]
    fn negative_or_nan_durations_become_zero() {
        assert_eq!(secs(-1.0), Duration::ZERO);
        assert_eq!(secs(f32::NAN), Duration::ZERO);
        assert_eq!(secs(1.5), Duration::from_millis(1500));
    }

    #[test]
    fn asset_errors_are_described_for_the_player() {
        let e = AssetError::Io {
            path: "story.ron".into(),
            message: "not found".into(),
        };
        assert!(describe_error(&e).contains("story.ron"));
    }
}
