//! The runtime: owns the renderer, text, audio and the current phase
//! (loading, "click to start", running, error) and implements the platform
//! [`App`] trait. What happens while running (menus, game, saves...) lives in
//! [`crate::shell`]; this module is the GPU and device side: drawing, audio,
//! thumbnails, files, host requests.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use orbit_assets::{
    AssetError, AssetHandle, AssetLoader, AssetPoll, FileWatcher, PcmAudio, decode_image,
};
use orbit_audio::{AudioEngine, Bus, MusicOptions, Sound};
use orbit_core::{Story, TransitionKind};
use orbit_i18n::Localizer;
use orbit_platform::{App, HostRequest, InputEvent, PhysicalSize};
use orbit_render2d::{
    Frame, Layer, Readback, ReadbackStatus, RenderError, Renderer2d, Scene, Sprite2d, TextureId,
    TextureOptions, Transition,
};
use orbit_save::{MemoryStorage, SaveConfig, Storage};
use orbit_text::{TextEngine, TextRenderer, srgb_to_linear};
use orbit_ui::{AudioBus, Banner, DrawCtx, Texts, Theme, UiSound, draw_progress_bar, tint};
use thiserror::Error;

use crate::director::AudioCommand;
use crate::format::slug;
use crate::input::{InputAction, InputMapper};
use crate::loading::{
    AssetStore, LoadedStory, PendingAsset, finish_loading, parse_story, start_loading,
};
use crate::shell::{AutosaveConfig, Effect, Shell, ShellConfig};
use crate::stage::{CANVAS, StageState, character_placement, cover_size};
use orbit_ui::UiInput;

mod embedded;

pub use embedded::{EmbeddedConfig, LoadOutcome, Notice, PlayFrom, PlaySpeed, PlayStatus};

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
}

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
    translations: TranslationLoad,
    loaded: Option<Arc<Story>>,
    pending: Vec<PendingAsset>,
    assets_total: usize,
    /// Start playing as soon as loaded (hot reload after a first start).
    autostart: bool,
}

impl LoadingState {
    fn new(
        story: AssetHandle<LoadedStory>,
        translations: TranslationLoad,
        autostart: bool,
    ) -> Self {
        Self {
            story,
            translations,
            loaded: None,
            pending: Vec::new(),
            assets_total: 0,
            autostart,
        }
    }
}

enum Phase {
    Loading(LoadingState),
    /// Everything is loaded; waiting for the "click to start" gesture.
    Ready(Box<Shell>),
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
    thumbnail_target: TextureId,
    captures: Vec<(u64, Readback)>,
    mapper: InputMapper,
    phase: Phase,
    /// The player already pressed "start" once (audio is unlocked).
    started_once: bool,
    time: f32,
    screens: Screens,
    /// Language the cached [`Screens`] were built for.
    screens_language: String,
    boot_texts: Texts,
    ui_sounds: UiSoundBank,
    host_requests: Vec<HostRequest>,
    /// Set when hosted by an editor (see [`Runtime::new_embedded`]).
    embedded: Option<embedded::Embedded>,
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
        let engine_loader = AssetLoader::new(config.engine_root.clone());
        let font_handles = config
            .fonts
            .iter()
            .map(|path| engine_loader.load_bytes(path))
            .collect();
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
            log::info!(
                "runtime: story `{}` (root `{}`), assets root `{}`",
                config.story_path,
                story_loader.root(),
                assets_loader.root()
            );
        }
        let phase = if embedded.is_some() {
            Phase::Stopped
        } else {
            let story = load_story(&story_loader, &config.story_path);
            let translations = TranslationLoad::start(
                &story_loader,
                &translations_path(&config.story_path, &config.translations_dir),
            );
            Phase::Loading(LoadingState::new(story, translations, false))
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
            thumbnail_target,
            captures: Vec::new(),
            mapper: InputMapper::default(),
            phase,
            started_once: false,
            time: 0.0,
            screens: Screens::default(),
            screens_language: boot_language.clone(),
            boot_texts: Texts::builtin(&boot_language),
            ui_sounds,
            host_requests: Vec::new(),
            embedded: embedded.map(embedded::Embedded::new),
            config,
        })
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
                    loading.pending =
                        start_loading(&self.assets_loader, &loaded.story, &self.store);
                    loading.assets_total = loading.pending.len();
                    loading.loaded = Some(loaded.story.clone());
                }
            }
        }
        let stem = std::path::Path::new(&self.story_path)
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let translations_dir = translations_path(&self.story_path, &self.config.translations_dir);
        let translations_done =
            loading
                .translations
                .poll(&self.story_loader, &translations_dir, &stem);
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
        let translation_texts = loading.translations.texts();
        finish_loading(&mut self.ctx.renderer, &mut self.store, pending);
        log::info!("story `{}` ready", story.title);
        let shell = self.build_shell(story, &translation_texts);
        // Hot reload restarts at once; `auto_start` skips the click natively
        // (nothing blocks audio there) but not on the web (autoplay policy).
        let native_auto = self.config.auto_start && !cfg!(target_arch = "wasm32");
        self.phase = if autostart || native_auto {
            self.started_once = true;
            if native_auto {
                self.audio.resume_after_user_gesture();
            }
            let mut shell = shell;
            shell.start_new_game();
            Phase::Running(shell)
        } else {
            Phase::Ready(shell)
        };
        self.after_shell();
    }

    fn build_shell(&mut self, story: Arc<Story>, translations: &[String]) -> Box<Shell> {
        let mut loc = Localizer::from_story(&story);
        for text in translations {
            if let Err(err) = loc.add_translation_ron(text) {
                log::warn!("translation ignored: {err}");
            }
        }
        let game_id = self
            .config
            .game_id
            .clone()
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
            fullscreen_supported: orbit_platform::FULLSCREEN_SUPPORTED && !embedded,
            quit_supported: orbit_platform::FULLSCREEN_SUPPORTED && !embedded,
            capture_thumbnails: !embedded,
            embedded,
        };
        let mut shell = Box::new(Shell::new(
            story.clone(),
            storage,
            loc,
            self.ctx.theme.clone(),
            cfg,
        ));
        if !storage_ok {
            shell.warn_storage_unavailable();
        }
        shell.set_menu_background(menu_background(&story, &self.store));
        shell
    }

    /// Reloads the story file (hot reload): validates it again and restarts
    /// from the beginning.
    fn reload(&mut self) {
        log::info!("story file changed: reloading");
        self.audio.stop_music(RESTART_MUSIC_FADE);
        let handle = load_story(&self.story_loader, &self.story_path);
        let translations =
            TranslationLoad::start(&self.story_loader, &self.config.translations_dir);
        self.screens.error_body = None;
        self.phase = Phase::Loading(LoadingState::new(handle, translations, self.started_once));
    }

    fn shell_mut(&mut self) -> Option<&mut Shell> {
        match &mut self.phase {
            Phase::Running(shell) | Phase::Ready(shell) => Some(shell),
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

    /// Renders the stages of the pending saves into the thumbnail target and
    /// collects the finished read-backs.
    fn service_thumbnails(&mut self) {
        let Phase::Running(shell) = &mut self.phase else {
            return;
        };
        let renderer = &mut self.ctx.renderer;
        for request in shell.take_capture_requests() {
            let mut scene = Scene::new();
            stage_scene(
                renderer,
                &self.store,
                shell.story(),
                &request.stage,
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
        let start = matches!(
            action,
            InputAction::Ui(UiInput::Confirm | UiInput::PointerDown(_))
        );
        if start && matches!(self.phase, Phase::Ready(_)) {
            let placeholder = Phase::Failed(String::new());
            if let Phase::Ready(mut shell) = std::mem::replace(&mut self.phase, placeholder) {
                self.started_once = true;
                if self.config.auto_start {
                    shell.start_new_game();
                }
                self.phase = Phase::Running(shell);
            }
        } else if let Phase::Running(shell) = &mut self.phase {
            shell.input(action);
        }
        self.after_shell();
    }

    // ------------------------------------------------------------- drawing

    fn draw(&mut self, out: &mut Output<'_>) -> Result<(), RenderError> {
        let language = match &self.phase {
            Phase::Ready(shell) | Phase::Running(shell) => shell.language().to_owned(),
            _ => self.screens_language.clone(),
        };
        if language != self.screens_language {
            self.screens = Screens::default();
            self.screens_language = language;
            self.boot_texts = Texts::builtin(&self.screens_language);
        }
        let Ctx {
            renderer,
            text_engine,
            text_renderer,
            white,
            theme,
        } = &mut self.ctx;
        let fonts_ready = self.fonts_ready;
        let time = self.time;
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
                        if let Some(tr) = game.director.transition() {
                            transitioning = true;
                            let mut from = Scene::new();
                            stage_scene(renderer, &self.store, &game.story, &tr.from, &mut from);
                            stage_scene(
                                renderer,
                                &self.store,
                                &game.story,
                                game.director.stage(),
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
                            stage_scene(
                                renderer,
                                &self.store,
                                &game.story,
                                game.director.stage(),
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
                    shell.draw_ui(&mut ctx, !transitioning);
                    out.scene(renderer, &ui_scene)?;
                }
                Ok(())
            }
            phase => {
                let s = &theme.screens;
                let mut scene = Scene::new();
                scene.clear = Some(tint(s.background));
                match phase {
                    Phase::Loading(l) => {
                        let fonts_total = self.font_handles.len();
                        let total = fonts_total + 1 + l.assets_total;
                        let done = if self.fonts_ready { fonts_total } else { 0 }
                            + usize::from(l.loaded.is_some())
                            + l.assets_total
                                .saturating_sub(l.pending.iter().filter(|p| !p.is_done()).count());
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
                    Phase::Running(_) | Phase::Stopped => {}
                }
                out.scene(renderer, &scene)
            }
        }
    }
}

/// The picture shown behind the main menu: the first background of the story.
fn menu_background(story: &Story, store: &AssetStore) -> Option<TextureId> {
    story
        .scenes
        .iter()
        .flat_map(|scene| &scene.nodes)
        .find_map(|node| match &node.kind {
            orbit_core::NodeKind::Background { asset, .. } => Some(*asset),
            _ => None,
        })
        .and_then(|asset| story.assets.get(asset))
        .and_then(|entry| store.texture(&entry.path))
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

/// Fills `scene` with the stage: opaque black, background (covering the
/// canvas), then the characters bottom-center anchored at their position.
fn stage_scene(
    renderer: &Renderer2d,
    store: &AssetStore,
    story: &Story,
    stage: &StageState,
    scene: &mut Scene,
) {
    let texture = |id| {
        let path = &story.assets.get(id)?.path;
        store.texture(path)
    };
    scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
    if let Some(tex) = stage.background.and_then(texture) {
        let size = renderer.texture_size(tex).map_or(CANVAS, cover_size);
        scene.push(
            Sprite2d::new(tex, [CANVAS[0] * 0.5, CANVAS[1] * 0.5], size)
                .anchor([0.5, 0.5])
                .layer(Layer::BACKGROUND),
        );
    }
    for (i, c) in stage.characters.iter().enumerate() {
        let Some(tex) = texture(c.image) else {
            continue;
        };
        let texture_size = renderer.texture_size(tex).unwrap_or([0, 0]);
        let placement = character_placement(c, texture_size);
        scene.push(
            Sprite2d::new(tex, placement.anchor, placement.size)
                .anchor([0.5, 1.0])
                .scale(placement.scale)
                .flipped(placement.flip_x, placement.flip_y)
                .layer(Layer::CHARACTERS)
                .z(i as f32),
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
        let dt_secs = dt.as_secs_f32().min(MAX_DT);
        self.time += dt_secs;
        self.audio.update();
        self.poll_fonts();
        if let Some(watcher) = &mut self.watcher
            && watcher.poll(dt)
        {
            self.reload();
        }
        self.poll_loading();
        if let Phase::Running(shell) = &mut self.phase {
            shell.set_now(orbit_platform::unix_time_ms());
            shell.update(dt_secs);
        }
        self.service_thumbnails();
        self.after_shell();
    }

    fn render(&mut self) {
        let mut frame = match self.ctx.renderer.begin_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(err) => {
                log::error!("begin_frame: {err}");
                return;
            }
        };
        if let Err(err) = self.draw(&mut Output::Surface(&mut frame)) {
            log::error!("render error: {err}");
        }
        self.ctx.renderer.end_frame(frame);
    }

    fn resize(&mut self, size: PhysicalSize<u32>, _scale_factor: f64) {
        self.ctx.renderer.resize(size.width, size.height);
    }

    fn input(&mut self, event: &InputEvent) {
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
