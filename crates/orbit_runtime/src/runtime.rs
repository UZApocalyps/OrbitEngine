//! The runtime: owns the renderer, text, audio and the current phase
//! (loading, "click to start", playing, error) and implements the platform
//! [`App`] trait.

use std::sync::Arc;
use std::time::Duration;

use orbit_assets::{AssetError, AssetHandle, AssetLoader, AssetPoll, FileWatcher};
use orbit_audio::{AudioEngine, MusicOptions};
use orbit_core::{Story, TransitionKind};
use orbit_platform::{App, InputEvent, PhysicalSize};
use orbit_render2d::{
    Frame, Layer, RenderError, Renderer2d, Scene, Sprite2d, TextureId, Transition,
};
use orbit_text::{TextEngine, TextRenderer, srgb_to_linear};
use orbit_ui::{Banner, Theme, draw_progress_bar, tint};
use orbit_vm::Vm;
use thiserror::Error;

use crate::director::AudioCommand;
use crate::game::{FlowKind, Game, InputOutcome};
use crate::input::{InputMapper, UiAction};
use crate::loading::{
    AssetStore, LoadedStory, PendingAsset, finish_loading, parse_story, start_loading,
};
use crate::stage::{CANVAS, StageState, character_size, cover_size, to_virtual};

/// Largest frame time fed to the simulation (a stall must not skip content).
const MAX_DT: f32 = 0.1;
/// How often the story file is checked for changes (native hot reload).
const WATCH_INTERVAL: Duration = Duration::from_millis(400);
/// Fade applied to the music when the story is reloaded or restarted.
const RESTART_MUSIC_FADE: Duration = Duration::from_millis(300);
/// Lines of an error message shown on screen.
const MAX_ERROR_LINES: usize = 14;

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
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("renderer: {0}")]
    Render(#[from] RenderError),
}

struct LoadingState {
    story: AssetHandle<LoadedStory>,
    loaded: Option<Arc<Story>>,
    pending: Vec<PendingAsset>,
    assets_total: usize,
    /// Start playing as soon as loaded (hot reload after a first start).
    autostart: bool,
}

impl LoadingState {
    fn new(story: AssetHandle<LoadedStory>, autostart: bool) -> Self {
        Self {
            story,
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
    Ready(Arc<Story>),
    Playing(Box<Game>),
    Failed(String),
}

/// GPU-side drawing context.
struct Ctx {
    renderer: Renderer2d,
    text_engine: TextEngine,
    text_renderer: TextRenderer,
    white: TextureId,
    theme: Theme,
}

/// Lazily built full-screen texts (they need the fonts).
#[derive(Default)]
struct Screens {
    loading: Option<Banner>,
    start: Option<Banner>,
    title: Option<(String, Banner)>,
    end: Option<Banner>,
    error_title: Option<Banner>,
    error_hint: Option<Banner>,
    error_body: Option<(String, Banner)>,
}

pub struct Runtime {
    ctx: Ctx,
    store: AssetStore,
    audio: AudioEngine,
    story_loader: AssetLoader,
    assets_loader: AssetLoader,
    story_path: String,
    watcher: Option<FileWatcher>,
    font_handles: Vec<AssetHandle<Vec<u8>>>,
    fonts_ready: bool,
    targets: [TextureId; 2],
    mapper: InputMapper,
    phase: Phase,
    /// The player already pressed "start" once (audio is unlocked).
    started_once: bool,
    time: f32,
    screens: Screens,
}

impl Runtime {
    /// Builds the runtime and starts loading the fonts and the story.
    pub fn new(mut renderer: Renderer2d, config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let text_renderer = TextRenderer::new(&mut renderer)?;
        let targets = [
            renderer.create_canvas_target("stage from")?,
            renderer.create_canvas_target("stage to")?,
        ];
        let engine_loader = AssetLoader::new(config.engine_root.clone());
        let font_handles = config
            .fonts
            .iter()
            .map(|path| engine_loader.load_bytes(path))
            .collect();
        let mut text_engine = TextEngine::new();
        text_engine.set_default_family(config.font_family.clone());

        let story_loader = AssetLoader::new(config.story_root.clone());
        let assets_loader = AssetLoader::new(config.assets_root.clone());
        log::info!(
            "runtime: story `{}` (root `{}`), assets root `{}`",
            config.story_path,
            story_loader.root(),
            assets_loader.root()
        );
        let story = load_story(&story_loader, &config.story_path);
        let white = renderer.white_texture();
        Ok(Self {
            ctx: Ctx {
                renderer,
                text_engine,
                text_renderer,
                white,
                theme: config.theme,
            },
            store: AssetStore::default(),
            audio: AudioEngine::new_locked(),
            story_loader,
            assets_loader,
            story_path: config.story_path,
            watcher: config
                .watch_path
                .map(|path| FileWatcher::new(path, WATCH_INTERVAL)),
            font_handles,
            fonts_ready: false,
            targets,
            mapper: InputMapper::default(),
            phase: Phase::Loading(LoadingState::new(story, false)),
            started_once: false,
            time: 0.0,
            screens: Screens::default(),
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
        if !self.fonts_ready || !loading.pending.iter().all(PendingAsset::is_done) {
            return;
        }
        let Some(story) = loading.loaded.take() else {
            return;
        };
        let autostart = loading.autostart;
        let pending = std::mem::take(&mut loading.pending);
        finish_loading(&mut self.ctx.renderer, &mut self.store, pending);
        log::info!("story `{}` ready", story.title);
        self.phase = if autostart {
            self.begin_story(story)
        } else {
            Phase::Ready(story)
        };
    }

    fn begin_story(&mut self, story: Arc<Story>) -> Phase {
        match Vm::from_start(story.clone()) {
            Ok(vm) => {
                self.started_once = true;
                log::info!("story started");
                Phase::Playing(Box::new(Game::new(story, vm, &self.ctx.theme)))
            }
            Err(err) => Phase::Failed(err.to_string()),
        }
    }

    /// Reloads the story file (hot reload): validates it again and restarts
    /// from the beginning.
    fn reload(&mut self) {
        log::info!("story file changed: reloading");
        self.audio.stop_music(RESTART_MUSIC_FADE);
        let handle = load_story(&self.story_loader, &self.story_path);
        self.screens.error_body = None;
        self.phase = Phase::Loading(LoadingState::new(handle, self.started_once));
    }

    fn flush_audio(&mut self) {
        let Phase::Playing(game) = &mut self.phase else {
            return;
        };
        for command in game.director.take_audio() {
            play_command(&mut self.audio, &self.store, &game.story, command);
        }
    }

    fn check_failure(&mut self) {
        let Phase::Playing(game) = &self.phase else {
            return;
        };
        if let crate::director::Flow::Failed(message) = game.director.flow() {
            self.phase = Phase::Failed(format!("The story stopped: {message}"));
        }
    }

    // --------------------------------------------------------------- input

    fn on_action(&mut self, action: UiAction) {
        if matches!(action, UiAction::Confirm | UiAction::Click(_)) {
            // Must happen inside the input handler: browsers only unlock
            // audio from a user gesture.
            self.audio.resume_after_user_gesture();
        }
        let confirm = matches!(action, UiAction::Confirm | UiAction::Click(_));
        match &mut self.phase {
            Phase::Ready(story) if confirm => {
                let story = story.clone();
                self.phase = self.begin_story(story);
            }
            Phase::Playing(game) => {
                if game.on_action(action, &self.ctx.theme) == InputOutcome::Restarted {
                    self.audio.stop_music(RESTART_MUSIC_FADE);
                }
            }
            _ => {}
        }
        self.flush_audio();
        self.check_failure();
    }

    // ------------------------------------------------------------- drawing

    fn draw(&mut self, frame: &mut Frame) -> Result<(), RenderError> {
        let Ctx {
            renderer,
            text_engine,
            text_renderer,
            white,
            theme,
        } = &mut self.ctx;
        let fonts_ready = self.fonts_ready;
        let time = self.time;
        match &mut self.phase {
            Phase::Playing(game) => {
                let mut scene = Scene::new();
                if let Some(tr) = game.director.transition() {
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
                    return renderer.draw_transition(
                        frame,
                        self.targets[0],
                        self.targets[1],
                        &render_transition(tr.kind),
                        tr.progress,
                    );
                }
                stage_scene(
                    renderer,
                    &self.store,
                    &game.story,
                    game.director.stage(),
                    &mut scene,
                );
                if fonts_ready {
                    match game.kind() {
                        FlowKind::Say => game.dialogue.draw(
                            text_engine,
                            text_renderer,
                            renderer,
                            &mut scene,
                            *white,
                        ),
                        FlowKind::Choice => {
                            if let Some(menu) = &mut game.menu {
                                menu.draw(text_engine, text_renderer, renderer, &mut scene, *white)
                            }
                        }
                        FlowKind::Ended => {
                            scene.push(
                                Sprite2d::new(*white, [0.0, 0.0], CANVAS)
                                    .anchor([0.0, 0.0])
                                    .layer(Layer::UI)
                                    .z(-1.0)
                                    .tint(tint(theme.choice.overlay)),
                            );
                            let size = theme.screens.title_size * 0.7;
                            let text = &theme.screens.texts.the_end;
                            let banner = self.screens.end.get_or_insert_with(|| {
                                Banner::centered(text, size, theme.screens.text_color)
                            });
                            banner.draw(
                                text_engine,
                                text_renderer,
                                renderer,
                                &mut scene,
                                CANVAS[1] * 0.5 - size * 0.675,
                                1.0,
                            );
                        }
                        _ => {}
                    }
                }
                renderer.draw_scene(frame, &scene)
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
                                Banner::centered(&s.texts.loading, s.body_size, s.text_color)
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
                    Phase::Ready(story) => {
                        if fonts_ready {
                            let title = match &mut self.screens.title {
                                Some((t, b)) if *t == story.title => b,
                                slot => {
                                    &mut slot
                                        .insert((
                                            story.title.clone(),
                                            Banner::centered(
                                                &escape_markup(&story.title),
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
                                    &s.texts.click_to_start,
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
                                    &s.texts.error_title,
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
                                    &s.texts.error_hint,
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
                    Phase::Playing(_) => {}
                }
                renderer.draw_scene(frame, &scene)
            }
        }
    }
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
        let size = renderer
            .texture_size(tex)
            .map_or([0.0, 0.0], character_size);
        scene.push(
            Sprite2d::new(tex, to_virtual(c.x, c.y), size)
                .anchor([0.5, 1.0])
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
        if let Phase::Playing(game) = &mut self.phase {
            game.update(dt_secs, &self.ctx.theme);
        }
        self.flush_audio();
        self.check_failure();
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
        if let Err(err) = self.draw(&mut frame) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
