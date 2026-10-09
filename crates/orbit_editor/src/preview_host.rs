//! The GPU side of the preview: owns the runtime (and through it the shared
//! `Gfx`), draws the game into a render target, hands that target to egui as
//! a texture on the same device (no CPU copy), hot reloads the story when the
//! project changes and routes the panel's commands and input.
//!
//! Everything that can be decided without a GPU is a small pure piece with
//! tests: [`Debounce`] (when to reload), [`choose_target_size`], the key
//! mapping and the translation of panel input to runtime input.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use orbit_core::Story;
use orbit_editor_api::SceneViewActor;
use orbit_editor_api::{
    Level, PreviewCommand, PreviewInput, PreviewSpeed, PreviewStatus, Selection, UiPreviewRequest,
};
use orbit_gfx::{Gfx, wgpu};
use orbit_platform::{InputEvent, KeyCode, MouseButton};
use orbit_render2d::{Readback, ReadbackStatus, Renderer2d, TextureId};
use orbit_runtime::preview::ResumeSource;
use orbit_runtime::scene_view;
use orbit_runtime::stage::character_size;
use orbit_runtime::{
    EmbeddedConfig, LoadOutcome, PlayFrom, PlaySpeed, PlayStatus, Runtime, RuntimeError,
    UiPreviewTarget,
};

use crate::core::EditorCore;
use crate::live::{PreviewBackend, PreviewOutput};
use orbit_mcp::ToolError;
use orbit_mcp::live_tools::{ChoiceSelector, LiveCall};

/// A reload waits for this long without a new edit (typing is a stream of
/// edits)...
pub const RELOAD_QUIET: Duration = Duration::from_millis(100);
/// How often the `ui/` folder is looked at while the game plays.
pub const UI_POLL: Duration = Duration::from_millis(400);
/// ...but never longer than this since the first one.
pub const RELOAD_MAX_WAIT: Duration = Duration::from_millis(250);
/// Interval between two frames while the game animates.
const FRAME_INTERVAL: Duration = Duration::from_millis(12);
/// Size of the pictures the live MCP's screenshot tool returns (16:9).
const MCP_CAPTURE_SIZE: [u32; 2] = [1280, 720];
/// `PreviewHost::settle`: game time per step, steps and real time at most.
const SETTLE_TICK: Duration = Duration::from_millis(100);
const SETTLE_MAX_TICKS: usize = 80;
const SETTLE_WALL_LIMIT: Duration = Duration::from_secs(6);
/// Smallest and largest preview targets (16:9).
const MIN_TARGET: [u32; 2] = [320, 180];
const MAX_TARGET: [u32; 2] = [1920, 1080];

// ------------------------------------------------------------ pure pieces

/// Collapses a burst of edits into one reload.
#[derive(Debug)]
pub struct Debounce {
    quiet: Duration,
    max_wait: Duration,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Debounce {
    pub fn new(quiet: Duration, max_wait: Duration) -> Self {
        Self {
            quiet,
            max_wait,
            first: None,
            last: None,
        }
    }

    /// An edit happened at `now`.
    pub fn changed(&mut self, now: Instant) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    pub fn pending(&self) -> bool {
        self.first.is_some()
    }

    /// When the reload is due (`None`: nothing pending).
    pub fn deadline(&self) -> Option<Instant> {
        let (first, last) = (self.first?, self.last?);
        Some((last + self.quiet).min(first + self.max_wait))
    }

    pub fn due(&self, now: Instant) -> bool {
        self.deadline().is_some_and(|d| now >= d)
    }

    /// The reload runs: returns when the first edit of the burst happened.
    pub fn take(&mut self) -> Option<Instant> {
        self.last = None;
        self.first.take()
    }
}

/// The size of the render target for a panel that would like `wanted`
/// physical pixels: 16:9, a multiple of 64 wide, within limits. The current
/// size is kept while it is close enough (a window being resized must not
/// recreate the target at every pixel).
pub fn choose_target_size(current: Option<[u32; 2]>, wanted: [u32; 2]) -> [u32; 2] {
    let width = wanted[0].clamp(MIN_TARGET[0], MAX_TARGET[0]);
    if let Some(current) = current
        && current[0].abs_diff(width) < 96
    {
        return current;
    }
    let snapped = ((width + 32) / 64 * 64).clamp(MIN_TARGET[0], MAX_TARGET[0]);
    [snapped, (snapped * 9 / 16).max(MIN_TARGET[1])]
}

/// The game key for an egui key (the ones `orbit_runtime::InputMapper` knows).
pub fn key_code(key: egui::Key) -> Option<KeyCode> {
    use egui::Key as K;
    Some(match key {
        K::Enter => KeyCode::Enter,
        K::Space => KeyCode::Space,
        K::Escape => KeyCode::Escape,
        K::ArrowUp => KeyCode::ArrowUp,
        K::ArrowDown => KeyCode::ArrowDown,
        K::ArrowLeft => KeyCode::ArrowLeft,
        K::ArrowRight => KeyCode::ArrowRight,
        K::Tab => KeyCode::Tab,
        K::PageUp => KeyCode::PageUp,
        K::PageDown => KeyCode::PageDown,
        K::F5 => KeyCode::F5,
        K::F9 => KeyCode::F9,
        K::F11 => KeyCode::F11,
        K::H => KeyCode::KeyH,
        K::S => KeyCode::KeyS,
        K::A => KeyCode::KeyA,
        K::Num1 => KeyCode::Digit1,
        K::Num2 => KeyCode::Digit2,
        K::Num3 => KeyCode::Digit3,
        K::Num4 => KeyCode::Digit4,
        K::Num5 => KeyCode::Digit5,
        K::Num6 => KeyCode::Digit6,
        K::Num7 => KeyCode::Digit7,
        K::Num8 => KeyCode::Digit8,
        K::Num9 => KeyCode::Digit9,
        _ => return None,
    })
}

/// What the runtime sees for one panel input (pointer positions are already
/// virtual pixels). Shift and Alt are separate key events for the runtime's
/// input mapper, so they come first.
pub fn translate(input: &PreviewInput) -> Vec<InputEvent> {
    match *input {
        PreviewInput::PointerMoved([x, y]) => vec![InputEvent::CursorMoved {
            x: f64::from(x),
            y: f64::from(y),
        }],
        PreviewInput::PointerLeft => vec![InputEvent::CursorMoved {
            x: -1000.0,
            y: -1000.0,
        }],
        PreviewInput::Primary { pressed } => vec![InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed,
        }],
        PreviewInput::Secondary => vec![InputEvent::MouseButton {
            button: MouseButton::Right,
            pressed: true,
        }],
        PreviewInput::Scroll(dy) => vec![InputEvent::Scroll { dx: 0.0, dy }],
        PreviewInput::Ctrl(down) => vec![InputEvent::Key {
            code: KeyCode::ControlLeft,
            pressed: down,
        }],
        PreviewInput::Key {
            key,
            pressed,
            shift,
            alt,
        } => match key_code(key) {
            Some(code) => vec![
                InputEvent::Key {
                    code: KeyCode::ShiftLeft,
                    pressed: shift,
                },
                InputEvent::Key {
                    code: KeyCode::AltLeft,
                    pressed: alt,
                },
                InputEvent::Key { code, pressed },
            ],
            None => Vec::new(),
        },
    }
}

/// The folder the manifest's asset paths are relative to (shared with the
/// MCP server, see `orbit_project::find_assets_root`).
pub use orbit_project::find_assets_root;

/// Assets of `story` whose file changed since the last call (by modification
/// time), so the runtime fetches them again instead of showing a stale
/// texture. The first sighting of a file only records it.
pub fn changed_assets(
    stamps: &mut HashMap<String, SystemTime>,
    root: &Path,
    story: &Story,
) -> Vec<String> {
    changed_assets_since(stamps, root, story, false)
}

/// [`changed_assets`]; with `new_changed`, a file seen for the first time
/// counts as changed too (the stamps are only taken when the asset folder
/// changed: a file declared since may have been loaded in its old version).
pub fn changed_assets_since(
    stamps: &mut HashMap<String, SystemTime>,
    root: &Path,
    story: &Story,
    new_changed: bool,
) -> Vec<String> {
    let mut changed = Vec::new();
    for asset in &story.assets.assets {
        let Ok(modified) = std::fs::metadata(root.join(&asset.path)).and_then(|m| m.modified())
        else {
            stamps.remove(&asset.path);
            continue;
        };
        match stamps.insert(asset.path.clone(), modified) {
            Some(previous) if previous != modified => changed.push(asset.path.clone()),
            None if new_changed => changed.push(asset.path.clone()),
            _ => {}
        }
    }
    changed
}

/// `(file name, length, modification time)` of the files of a folder, sorted.
pub type UiStamps = Vec<(String, u64, Option<SystemTime>)>;

/// The top-level files of the `ui/` folder (empty when it does not exist).
pub fn ui_stamps(dir: &Path) -> UiStamps {
    let mut stamps: UiStamps = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let meta = e.metadata().ok().filter(|m| m.is_file())?;
                    Some((e.file_name().to_string_lossy().into_owned(), meta.len(), meta.modified().ok()))
                })
                .collect()
        })
        .unwrap_or_default();
    stamps.sort();
    stamps
}

fn flow_text(flow: Option<&orbit_runtime::director::Flow>) -> String {
    use orbit_runtime::director::Flow;
    match flow {
        None => String::new(),
        Some(Flow::Say(_)) => "preview.flow.say".into(),
        Some(Flow::Choice(_)) => "preview.flow.choice".into(),
        Some(Flow::Waiting { .. }) => "preview.flow.waiting".into(),
        Some(Flow::Transition { .. }) => "preview.flow.transition".into(),
        Some(Flow::Ended) => "preview.flow.ended".into(),
        Some(Flow::Screen(_)) => "preview.flow.screen".into(),
        Some(Flow::Failed(_)) => "preview.flow.failed".into(),
    }
}

// ------------------------------------------------------------------- host

struct Target {
    id: TextureId,
    size: [u32; 2],
}

/// What a call to [`PreviewHost::update`] asks of the window loop.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostWake {
    /// A frame is due at this instant even without any event.
    pub at: Option<Instant>,
    /// Draw one more frame at once (the UI must show new state).
    pub redraw: bool,
}

pub struct PreviewHost {
    runtime: Runtime,
    target: Option<Target>,
    egui_texture: Option<egui::TextureId>,
    debounce: Debounce,
    /// (project generation, revision) of the story the runtime has / last saw.
    seen: Option<(u64, u64)>,
    last_tick: Option<Instant>,
    window_focused: bool,
    user_muted: bool,
    applied_mute: Option<bool>,
    /// Frames drawn into the target.
    pub frames: u64,
    last_node: Option<orbit_core::NodeId>,
    last_error: Option<String>,
    /// Measure of the last hot reload: from the first edit to the picture.
    reload_started: Option<(Instant, Duration)>,
    /// Modification times of the project's assets (see [`changed_assets`]).
    asset_stamps: HashMap<String, SystemTime>,
    /// Files of the project's `ui/` folder as last seen (`None`: not yet),
    /// for the hot reload during Play.
    ui_stamps: Option<UiStamps>,
    ui_next_poll: Option<Instant>,
    ui_error: Option<String>,
    /// Last reload latency in milliseconds, for the debug overlay and tests.
    pub last_reload_ms: Option<u128>,
    /// Pictures drawn into the scene view's target.
    pub scene_frames: u64,
    /// `EditorCore::asset_epoch` as last seen: files changed on disk.
    seen_asset_epoch: u64,
    /// `EditorCore::asset_epoch` when the asset files were last stamped.
    asset_stamps_epoch: Option<u64>,
    /// The `models/` folders last given to the runtime (3D scenes run from
    /// the project folder) and the asset epoch they were read at.
    models3d: Option<(Vec<PathBuf>, u64)>,
    /// The scene view's own target and what it was last drawn for.
    scene_target: Option<Target>,
    scene_egui_texture: Option<egui::TextureId>,
    scene_cache: Option<SceneCache>,
    scene_drawn: Option<SceneKey>,
    /// Seconds the scene view has been playing its characters animations,
    /// and when it last advanced (`None`: the stage shown is still).
    scene_time: f32,
    scene_tick: Option<Instant>,
    scene_stage: Option<orbit_runtime::StageState>,
    scene_animated: bool,
}

/// What a scene view picture was drawn for: when none of it changed, the
/// picture is still right and nothing is drawn (zero frames at rest).
#[derive(Clone, Debug, PartialEq)]
struct SceneKey {
    generation: u64,
    revision: u64,
    node: Option<orbit_core::NodeId>,
    size: [u32; 2],
    epoch: u64,
}

/// The path to the node shown by the scene view: found once, reused while a
/// character is dragged (the story changes at every frame, the path does not).
struct SceneCache {
    generation: u64,
    node: orbit_core::NodeId,
    structure: u64,
    source: ResumeSource,
    notice: Option<String>,
}

impl PreviewHost {
    pub fn new(gfx: Gfx) -> Result<Self, RuntimeError> {
        let mut renderer = Renderer2d::new(gfx).map_err(RuntimeError::Render)?;
        renderer.enable_target_gamma_views();
        let config = EmbeddedConfig {
            fonts: vec![
                include_bytes!("../../../assets/fonts/Inter-Regular.ttf").to_vec(),
                include_bytes!("../../../assets/fonts/Inter-Bold.ttf").to_vec(),
                include_bytes!("../../../assets/fonts/Inter-Italic.ttf").to_vec(),
            ],
            font_family: "Inter".into(),
            theme: orbit_ui::Theme::default(),
        };
        let runtime = Runtime::new_embedded(renderer, config)?;
        Ok(Self {
            runtime,
            target: None,
            egui_texture: None,
            debounce: Debounce::new(RELOAD_QUIET, RELOAD_MAX_WAIT),
            seen: None,
            last_tick: None,
            window_focused: true,
            user_muted: false,
            applied_mute: None,
            frames: 0,
            last_node: None,
            last_error: None,
            reload_started: None,
            asset_stamps: HashMap::new(),
            ui_stamps: None,
            ui_next_poll: None,
            ui_error: None,
            last_reload_ms: None,
            scene_frames: 0,
            seen_asset_epoch: 0,
            asset_stamps_epoch: None,
            models3d: None,
            scene_target: None,
            scene_egui_texture: None,
            scene_cache: None,
            scene_drawn: None,
            scene_time: 0.0,
            scene_tick: None,
            scene_stage: None,
            scene_animated: false,
        })
    }

    pub fn gfx(&self) -> &Gfx {
        self.runtime.renderer().gfx()
    }

    pub fn gfx_mut(&mut self) -> &mut Gfx {
        self.runtime.renderer_mut().gfx_mut()
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    pub fn set_window_focused(&mut self, focused: bool) {
        self.window_focused = focused;
    }

    /// Whether anything shows the runtime's story: the preview or the scene
    /// view on screen, or a playthrough (it keeps its place, and its sound,
    /// while hidden).
    fn wanted(&self, core: &EditorCore) -> bool {
        use crate::builtin::panels;
        core.state.panels.is_visible(panels::PREVIEW)
            || core.state.panels.is_visible(panels::SCENEVIEW)
            || core.state.maximized_panel.as_deref() == Some(panels::PREVIEW)
            || self.runtime.status() != PlayStatus::Stopped
    }

    /// Loads the project's current story into the runtime.
    fn load(&mut self, core: &mut EditorCore, now: Instant) {
        let Some(session) = core.session.as_ref() else {
            return;
        };
        orbit_profile::scope!("preview reload");
        let started = Instant::now();
        let story = {
            orbit_profile::scope!("preview: story copy");
            Arc::new(session.project.story().clone())
        };
        let root = find_assets_root(&session.location.root, &story);
        if root != Path::new(self.runtime.assets_root()) {
            self.asset_stamps.clear();
            self.asset_stamps_epoch = None;
        }
        // The files are looked at only when the watcher saw the asset folder
        // change (not at every reload of the story): one `stat` per asset.
        if self.asset_stamps_epoch != Some(core.asset_epoch) {
            orbit_profile::scope!("preview: asset stamps");
            let first = self.asset_stamps_epoch.is_none();
            self.asset_stamps_epoch = Some(core.asset_epoch);
            for path in changed_assets_since(&mut self.asset_stamps, &root, &story, !first) {
                if self.runtime.invalidate_asset(&path) {
                    log::info!("preview: asset `{path}` changed on disk, reloaded");
                }
            }
        }
        self.sync_models3d(&root, &story, core.asset_epoch);
        // The project's own screens (`ui/`) are part of what Play shows.
        let ui_dir = session.location.root.join("ui");
        // The shell built below reads `ui/` now: the hot reload compares
        // against what it will see next. Read again only when it changed.
        let stamps = ui_stamps(&ui_dir);
        if self.ui_stamps.as_ref() != Some(&stamps) {
            self.runtime.invalidate_ui();
        }
        self.ui_stamps = Some(stamps);
        self.ui_error = None;
        self.runtime
            .set_ui_dir(ui_dir.is_dir().then(|| ui_dir.to_string_lossy().into_owned()));
        let outcome = {
            orbit_profile::scope!("preview: runtime load_story");
            self.runtime.load_story(story, &root.to_string_lossy())
        };
        let elapsed = started.elapsed();
        let first_edit = self.debounce.take();
        if let Some(first) = first_edit {
            self.reload_started = Some((first, elapsed));
            log::info!(
                target: "orbit_editor::perf",
                "preview: story loaded in {} ms ({} ms after the first edit of the burst)",
                elapsed.as_millis(),
                now.saturating_duration_since(first).as_millis()
            );
        }
        if let LoadOutcome::Rejected(message) = &outcome
            && self.last_error.as_deref() != Some(message)
        {
            let head = message.lines().next().unwrap_or("").to_owned();
            let line = core.state.tf("preview.console", &[("text", &head)]);
            core.state.console.push(Level::Warn, line);
        }
    }

    /// Gives the runtime the models of the story's 3D scenes. A project run
    /// from its folder has no packed `3d/index.ron`: each scene finds its
    /// models in the `models/` folder next to it, like in the 3D editor and
    /// at export. Read again only when the folders or the asset files
    /// changed; nothing at all for a story without 3D scene.
    fn sync_models3d(&mut self, root: &Path, story: &orbit_core::Story, epoch: u64) {
        let mut dirs: Vec<PathBuf> = story
            .assets
            .assets
            .iter()
            .filter(|a| a.kind == orbit_core::AssetKind::Scene3d)
            .filter_map(|a| Some(root.join(&a.path).parent()?.join("models")))
            .collect();
        dirs.sort();
        dirs.dedup();
        if dirs.is_empty() {
            if self.models3d.take().is_some() {
                self.runtime.set_library3d_fallback(None);
            }
            return;
        }
        if self
            .models3d
            .as_ref()
            .is_some_and(|(d, e)| *d == dirs && *e == epoch)
        {
            return;
        }
        orbit_profile::scope!("preview: 3D models");
        let lib = models3d_library(&dirs);
        self.runtime.set_library3d_fallback(Some(lib));
        self.models3d = Some((dirs, epoch));
    }

    /// One step of the preview, between the UI pass and the paint of a
    /// frame: syncs the story, applies commands and input, advances the
    /// game, draws it into the target and publishes the state for the panel.
    pub fn update(
        &mut self,
        core: &mut EditorCore,
        egui_renderer: &mut egui_wgpu::Renderer,
        now: Instant,
    ) -> HostWake {
        let before = Published::of(&core.state.preview);
        let mut wake = HostWake::default();
        let mut force_draw = false;

        // ---- the story follows the project
        let source = core
            .session
            .as_ref()
            .map(|s| (core.project_generation, s.project.revision()));
        match (source, self.seen) {
            (None, Some(_)) => {
                self.runtime.stop();
                self.seen = None;
                self.debounce.take();
                force_draw = true;
            }
            (Some(key), seen) if seen != Some(key) => {
                let new_project = seen.is_none_or(|(g, _)| g != key.0);
                self.seen = Some(key);
                if new_project {
                    self.runtime.stop();
                    self.asset_stamps_epoch = None;
                    if self.wanted(core) {
                        self.load(core, now);
                        force_draw = true;
                    } else {
                        // Loaded when the preview (or the scene view) shows.
                        self.debounce.changed(now);
                    }
                } else {
                    self.debounce.changed(now);
                }
            }
            _ => {}
        }
        // A file of the project changed on disk (watcher): reload so the
        // runtime fetches the new version of an image or a sound.
        if self.seen_asset_epoch != core.asset_epoch {
            self.seen_asset_epoch = core.asset_epoch;
            if core.session.is_some() && self.seen.is_some() {
                self.debounce.changed(now);
            }
        }
        // Nothing shows the runtime's story (preview and scene view hidden,
        // nothing playing): the reload waits until something does.
        let shown = self.wanted(core);
        if self.debounce.due(now) && shown {
            self.load(core, now);
            force_draw = true;
        }

        // ---- `ui/` changed on disk while the game plays: swap the screens
        let playing = self.runtime.status() != PlayStatus::Stopped && self.seen.is_some();
        if playing && self.poll_ui(core, now) {
            force_draw = true;
            wake.redraw = true;
        }

        // ---- the UI editor asks for a screen or the splashes
        self.runtime
            .set_ui_highlight(core.state.preview.ui_highlight.clone());
        if let Some(request) = core.state.preview.ui_request.take() {
            self.apply_ui_request(core, request);
            force_draw = true;
            wake.redraw = true;
        }

        // ---- commands
        let commands = std::mem::take(&mut core.state.preview.commands);
        if !commands.is_empty() {
            // Playing uses the newest story, not one still waiting to reload,
            // and the newest files: a replaced image shows up on a new run.
            let restarts = commands.iter().any(|c| {
                matches!(
                    c,
                    PreviewCommand::Play | PreviewCommand::Restart | PreviewCommand::PlayFrom(_)
                )
            });
            if self.debounce.pending() || restarts {
                self.load(core, now);
            }
            for command in commands {
                self.apply(command);
            }
            force_draw = true;
            wake.redraw = true;
        }

        // ---- input, only for a visible panel that has the focus
        let inputs = std::mem::take(&mut core.state.preview.input);
        let visible = core.state.preview.visible;
        if visible && core.state.preview.focused {
            for input in &inputs {
                for event in translate(input) {
                    self.runtime.inject_input(&event);
                }
            }
        }

        // ---- time
        let active = visible && self.window_focused;
        let mute = self.user_muted || !active;
        if self.applied_mute != Some(mute) {
            self.applied_mute = Some(mute);
            self.runtime.set_muted(mute);
        }
        let dt = match (active, self.last_tick) {
            (true, Some(t)) => now.saturating_duration_since(t),
            _ => Duration::ZERO,
        };
        self.runtime.tick(dt);
        let animating = active && self.runtime.is_animating();
        self.last_tick = animating.then_some(now);

        // ---- the target
        let wanted = core.state.preview.wanted_size;
        if visible && wanted[0] > 0 && wanted[1] > 0 {
            let size = choose_target_size(self.target.as_ref().map(|t| t.size), wanted);
            if self.target.as_ref().map(|t| t.size) != Some(size) {
                self.set_target(egui_renderer, size);
                force_draw = true;
            }
        }
        if visible
            && let Some(target) = &self.target
            && (animating || force_draw || self.runtime.take_dirty())
        {
            let id = target.id;
            match self.runtime.render_to_target(id) {
                Ok(()) => {
                    self.frames += 1;
                    if let Some((first, loaded)) = self.reload_started.take() {
                        let total = now.saturating_duration_since(first) + loaded;
                        self.last_reload_ms = Some(total.as_millis());
                        log::info!(
                            target: "orbit_editor::perf",
                            "preview: up to date {} ms after the first edit (story load {} ms)",
                            total.as_millis(),
                            loaded.as_millis()
                        );
                    }
                }
                Err(e) => log::error!("preview render failed: {e}"),
            }
        }

        // ---- the scene view
        if self.update_scene_view(core, egui_renderer, now) {
            wake.redraw = true;
        }

        // ---- notices, publication
        for notice in self.runtime.take_notices() {
            let level = if notice.warning {
                Level::Warn
            } else {
                Level::Info
            };
            let line = core.state.tf("preview.console", &[("text", &notice.text)]);
            core.state.console.push(level, line);
        }
        self.publish(core);
        let after = Published::of(&core.state.preview);
        if after != before {
            log::debug!(target: "orbit_editor::perf", "preview state changed: {before:?} -> {after:?}");
            wake.redraw = true;
        }

        // ---- when to come back
        let mut at = self.debounce.deadline().filter(|_| shown);
        // Assets on their way (on demand): come back to move them in.
        if animating || self.runtime.is_loading() {
            let next = now + FRAME_INTERVAL;
            at = Some(at.map_or(next, |d| d.min(next)));
        }
        if playing && let Some(next) = self.ui_next_poll {
            at = Some(at.map_or(next, |d| d.min(next)));
        }
        wake.at = at;
        wake
    }

    /// Looks at the `ui/` folder (at most every [`UI_POLL`]) and, when it
    /// changed, hands the new bundle to the running shell. An invalid one is
    /// refused: the previous screens stay and the problem is shown once.
    /// Returns whether the screens were replaced.
    fn poll_ui(&mut self, core: &mut EditorCore, now: Instant) -> bool {
        if self.ui_next_poll.is_some_and(|t| now < t) {
            return false;
        }
        self.ui_next_poll = Some(now + UI_POLL);
        let Some(root) = core.session.as_ref().map(|s| s.location.root.clone()) else {
            return false;
        };
        let dir = root.join("ui");
        let stamps = ui_stamps(&dir);
        if self.ui_stamps.as_ref() == Some(&stamps) {
            return false;
        }
        let first_sight = self.ui_stamps.is_none();
        self.ui_stamps = Some(stamps.clone());
        if first_sight {
            return false;
        }
        self.runtime
            .set_ui_dir(dir.is_dir().then(|| dir.to_string_lossy().into_owned()));
        match self.runtime.reload_ui() {
            Ok(()) => {
                self.ui_error = None;
                log::info!("preview: ui/ changed on disk, screens reloaded");
                true
            }
            Err(error) => {
                if self.ui_error.as_deref() != Some(error.as_str()) {
                    let text = core.state.tf("preview.ui_reload_failed", &[("error", &error)]);
                    core.state.console.push(Level::Error, text.clone());
                    core.state.push_toast(Level::Error, text);
                    self.ui_error = Some(error);
                }
                false
            }
        }
    }

    /// Shows (or stops showing) a UI screen / the splashes instead of the game.
    fn apply_ui_request(&mut self, core: &mut EditorCore, request: UiPreviewRequest) {
        // The game's interface language, not the editor's: the one chosen
        // for the preview, else the story's own.
        let language = core
            .state
            .preview
            .ui_language
            .clone()
            .or_else(|| {
                core.session
                    .as_ref()
                    .map(|s| s.project.story().strings.source_lang.clone())
            })
            .unwrap_or_else(|| "en".to_owned());
        let (bundle_ron, target) = match &request {
            UiPreviewRequest::Off => {
                self.runtime.clear_ui_preview();
                return;
            }
            UiPreviewRequest::Screen { bundle_ron, id } => {
                (bundle_ron, UiPreviewTarget::Screen(id.as_str()))
            }
            UiPreviewRequest::Splashes { bundle_ron } => (bundle_ron, UiPreviewTarget::Splashes),
            UiPreviewRequest::Dialogue {
                bundle_ron,
                speaker,
                line,
                choices,
            } => (
                bundle_ron,
                UiPreviewTarget::Dialogue {
                    speaker,
                    line,
                    choices,
                },
            ),
        };
        let shown = orbit_ui::UiBundle::from_ron(bundle_ron)
            .and_then(|bundle| self.runtime.set_ui_preview(&bundle, target, &language));
        if let Err(error) = shown {
            log::warn!("ui preview: {error}");
            self.runtime.clear_ui_preview();
        }
    }

    fn apply(&mut self, command: PreviewCommand) {
        if matches!(
            command,
            PreviewCommand::Play | PreviewCommand::Restart | PreviewCommand::PlayFrom(_)
        ) {
            self.runtime.clear_ui_preview();
        }
        match command {
            PreviewCommand::Play => match self.runtime.status() {
                PlayStatus::Playing => {}
                PlayStatus::Paused => self.runtime.resume(),
                PlayStatus::Stopped => self.runtime.play(PlayFrom::Beginning),
            },
            PreviewCommand::Pause => self.runtime.pause(),
            PreviewCommand::Stop => self.runtime.stop(),
            PreviewCommand::Restart => self.runtime.restart(),
            PreviewCommand::PlayFrom(node) => {
                let started = Instant::now();
                self.runtime.play(PlayFrom::Node(node));
                log::info!(
                    target: "orbit_editor::perf",
                    "preview: play from here (path search included) took {} ms",
                    started.elapsed().as_millis()
                );
            }
            PreviewCommand::SetSpeed(speed) => self.runtime.set_speed(match speed {
                PreviewSpeed::Normal => PlaySpeed::Normal,
                PreviewSpeed::Double => PlaySpeed::Double,
            }),
            PreviewCommand::SetSkip(on) => self.runtime.set_skip(on),
            PreviewCommand::SetMuted(muted) => self.user_muted = muted,
            PreviewCommand::ScrubAnimation(node, ms) => {
                self.runtime.clear_ui_preview();
                self.runtime.scrub_animation(node, ms as f32 / 1000.0);
            }
        }
    }

    fn set_target(&mut self, egui_renderer: &mut egui_wgpu::Renderer, size: [u32; 2]) {
        let created = create_shared_target(
            &mut self.runtime,
            egui_renderer,
            "preview",
            size,
            &mut self.egui_texture,
        );
        if let Some(new) = created
            && let Some(old) = self.target.replace(new)
        {
            self.runtime.renderer_mut().release_texture(old.id);
        }
    }

    /// The scene view: simulates the story up to the node the panel asked
    /// for, draws that stage with the real renderer into the panel's own
    /// target and publishes the placement of its characters. Draws only when
    /// something it depends on changed. Returns whether the UI must run once
    /// more to show what was published.
    fn update_scene_view(
        &mut self,
        core: &mut EditorCore,
        egui_renderer: &mut egui_wgpu::Renderer,
        now: Instant,
    ) -> bool {
        let visible = std::mem::take(&mut core.state.scene_view.visible);
        core.state.scene_view.available = true;
        let wanted = core.state.scene_view.wanted_size;
        if !visible || wanted[0] == 0 || wanted[1] == 0 {
            self.scene_tick = None;
            return false;
        }
        let size = choose_target_size(self.scene_target.as_ref().map(|t| t.size), wanted);
        if self.scene_target.as_ref().map(|t| t.size) != Some(size) {
            let created = create_shared_target(
                &mut self.runtime,
                egui_renderer,
                "scene view",
                size,
                &mut self.scene_egui_texture,
            );
            if let Some(new) = created
                && let Some(old) = self.scene_target.replace(new)
            {
                self.runtime.renderer_mut().release_texture(old.id);
            }
        }
        let Some((target, size)) = self.scene_target.as_ref().map(|t| (t.id, t.size)) else {
            return false;
        };
        let node = core.state.scene_view.wanted_node;
        let Some(session) = core.session.as_ref() else {
            return Self::publish_scene_view(
                core,
                None,
                Vec::new(),
                None,
                size,
                self.scene_egui_texture,
                self.scene_frames,
            );
        };
        let key = SceneKey {
            generation: core.project_generation,
            revision: session.project.revision(),
            node,
            size,
            epoch: self.runtime.assets_epoch(),
        };
        if self.scene_drawn.as_ref() == Some(&key) {
            // Nothing changed: still, unless a character plays an animation,
            // which is redrawn at the elapsed time (the story is not
            // replayed).
            let Some(stage) = self.scene_stage.as_ref().filter(|_| self.scene_animated) else {
                self.scene_tick = None;
                return false;
            };
            let dt = self
                .scene_tick
                .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32().min(0.1));
            self.scene_tick = Some(now);
            self.scene_time += dt;
            let story = session.project.story();
            if let Err(e) =
                self.runtime
                    .render_stage_to_target_at(story, stage, target, Some(self.scene_time))
            {
                log::error!("scene view render failed: {e}");
            }
            self.scene_frames += 1;
            core.state.scene_view.frames = self.scene_frames;
            return true;
        }
        self.scene_time = 0.0;
        self.scene_tick = Some(now);
        self.scene_animated = false;
        self.scene_stage = None;
        let interacting = core.state.scene_view.interacting;
        let story = Arc::new(session.project.story().clone());
        let (shown, actors, notice) = match node.filter(|n| story.locate(*n).is_some()) {
            Some(node) => match self.draw_scene_view(&story, &key, node, target, interacting) {
                Ok((actors, notice)) => (Some(node), actors, notice),
                Err(e) => {
                    log::warn!("scene view: {e}");
                    (None, Vec::new(), Some(e))
                }
            },
            None => {
                self.scene_cache = None;
                (None, Vec::new(), None)
            }
        };
        self.scene_drawn = Some(key);
        Self::publish_scene_view(
            core,
            shown,
            actors,
            notice,
            size,
            self.scene_egui_texture,
            self.scene_frames,
        )
    }

    /// Finds (or reuses) the path to `node`, draws the stage after it into
    /// `target` and returns the characters on it with the search's notice.
    fn draw_scene_view(
        &mut self,
        story: &Arc<Story>,
        key: &SceneKey,
        node: orbit_core::NodeId,
        target: TextureId,
        interacting: bool,
    ) -> Result<(Vec<SceneViewActor>, Option<String>), String> {
        let arc = story;
        let structure = orbit_save::story_hash(story);
        let fresh = self.scene_cache.as_ref().is_some_and(|c| {
            c.generation == key.generation
                && c.node == node
                && (interacting || c.structure == structure)
        });
        if !fresh {
            let started = Instant::now();
            self.scene_cache = None;
            let (source, notice) =
                scene_view::seek_before(arc, node, orbit_vm::SeekLimits::default())?;
            log::debug!(
                target: "orbit_editor::perf",
                "scene view: path to the node found in {} ms",
                started.elapsed().as_millis()
            );
            self.scene_cache = Some(SceneCache {
                generation: key.generation,
                node,
                structure,
                source,
                notice,
            });
        }
        let cache = self.scene_cache.as_ref().ok_or("no path")?;
        let notice = cache.notice.clone();
        let at = scene_view::scene_at(arc, &cache.source, node)?;
        let stage = scene_view::stage_state(story, &at.after);
        self.scene_animated = orbit_runtime::Runtime::stage_is_animated(story, &stage);
        let time = self.scene_animated.then_some(0.0);
        // Pictures load on demand: what this stage shows is fetched now and
        // drawn again when it arrives (the assets epoch is in the key).
        self.runtime.request_stage_assets(story, &stage);
        if let Err(e) = self
            .runtime
            .render_stage_to_target_at(story, &stage, target, time)
        {
            log::error!("scene view render failed: {e}");
        }
        self.scene_stage = Some(stage);
        self.scene_frames += 1;
        let actors = scene_view::actors(story, &at.after)
            .into_iter()
            .map(|a| SceneViewActor {
                character: a.character,
                source: a.source,
                x: a.x,
                y: a.y,
                scale: a.scale,
                flip_x: a.flip_x,
                flip_y: a.flip_y,
                base_size: self
                    .runtime
                    .expression_size(story, a.character, &a.expression, a.image)
                    .map_or([0.0, 0.0], character_size),
            })
            .collect();
        Ok((actors, notice))
    }

    /// Copies the scene view's result into the shared state; `true` when it
    /// differs from what the panel last saw.
    fn publish_scene_view(
        core: &mut EditorCore,
        node: Option<orbit_core::NodeId>,
        actors: Vec<SceneViewActor>,
        notice: Option<String>,
        size: [u32; 2],
        texture: Option<egui::TextureId>,
        frames: u64,
    ) -> bool {
        let sv = &mut core.state.scene_view;
        let changed = sv.node != node
            || sv.actors != actors
            || sv.notice != notice
            || sv.texture_size != size
            || sv.texture != texture
            || sv.frames != frames;
        sv.node = node;
        sv.actors = actors;
        sv.notice = notice;
        sv.texture_size = size;
        sv.texture = texture;
        sv.frames = frames;
        changed
    }

    /// Copies what the runtime knows into `EditorState::preview`.
    fn publish(&mut self, core: &mut EditorCore) {
        let node = self.runtime.current_node();
        let status = match self.runtime.status() {
            PlayStatus::Stopped => PreviewStatus::Stopped,
            PlayStatus::Playing => PreviewStatus::Playing,
            PlayStatus::Paused => PreviewStatus::Paused,
        };
        let error = self.runtime.last_error().map(str::to_owned);
        self.last_error.clone_from(&error);
        let preview = &mut core.state.preview;
        preview.available = true;
        preview.ui_active = self.runtime.has_ui_preview();
        preview.ui_shown = self.runtime.ui_preview_shown().map(str::to_owned);
        if let Some(pick) = self.runtime.take_ui_pick() {
            preview.ui_pick = Some(pick);
        }
        preview.status = status;
        preview.speed = match self.runtime.speed() {
            PlaySpeed::Normal => PreviewSpeed::Normal,
            PlaySpeed::Double => PreviewSpeed::Double,
        };
        preview.skipping = self.runtime.is_skipping();
        preview.muted = self.user_muted;
        preview.texture = self.egui_texture;
        preview.texture_size = self.target.as_ref().map_or([0, 0], |t| t.size);
        preview.current_node = node;
        preview.flow = flow_text(self.runtime.flow());
        preview.error = error;
        let follow = preview.follow_playback;
        // Selection follows the line being played, when asked to.
        if follow
            && status != PreviewStatus::Stopped
            && node.is_some()
            && node != self.last_node
            && let Some(id) = node
            && core.story().is_some_and(|s| s.locate(id).is_some())
        {
            core.state.selection = Some(Selection::Node(id));
        }
        self.last_node = node;
        // The panel sets this again if it is drawn next frame.
        core.state.preview.visible = false;
    }

    /// Starts copying the preview target to the CPU (non blocking); `None`
    /// before the first frame. The MCP screenshot of M4 uses the same path.
    pub fn start_capture(&self) -> Option<Readback> {
        let target = self.target.as_ref()?;
        self.runtime.renderer().start_readback(target.id).ok()
    }

    /// Waits for a capture: `(width, height, RGBA8 pixels)`.
    pub fn read_capture(&self, readback: &mut Readback) -> Result<(u32, u32, Vec<u8>), String> {
        loop {
            match self.runtime.renderer().poll_readback(readback) {
                ReadbackStatus::Pending => {
                    orbit_profile::scope!("gpu wait: buffer map");
                    self.gfx()
                        .device()
                        .poll(wgpu::PollType::wait_indefinitely())
                        .map_err(|e| format!("GPU poll failed: {e}"))?;
                }
                ReadbackStatus::Ready {
                    width,
                    height,
                    rgba,
                } => return Ok((width, height, rgba)),
                ReadbackStatus::Failed => return Err("readback failed".to_owned()),
            }
        }
    }

    /// Waits for a capture and writes it as a PNG (debug option, tests).
    pub fn write_capture(&self, readback: &mut Readback, path: &Path) -> Result<(), String> {
        let (width, height, rgba) = self.read_capture(readback)?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8)
            .map_err(|e| e.to_string())
    }

    // ------------------------------------------------- scripted (live MCP)

    /// Lets what animates by itself finish (typewriter, transition, timed
    /// pause, assets loading) so that a state read or a screenshot shows the
    /// settled picture. Bounded: a few seconds of game time and of waiting.
    fn settle(&mut self) {
        let give_up = Instant::now() + SETTLE_WALL_LIMIT;
        for _ in 0..SETTLE_MAX_TICKS {
            if !self.runtime.is_animating() || Instant::now() > give_up {
                break;
            }
            self.runtime.tick(SETTLE_TICK);
            // Assets load on other threads: give them real time.
            std::thread::sleep(Duration::from_millis(4));
        }
    }

    /// The game drawn into a fresh 1280x720 target of its own (independent
    /// of the panel's size and visibility), as PNG bytes.
    fn capture_png(&mut self) -> Result<Vec<u8>, String> {
        let [width, height] = MCP_CAPTURE_SIZE;
        let id = self
            .runtime
            .renderer_mut()
            .create_render_target("mcp capture", width, height)
            .map_err(|e| format!("cannot create the capture target: {e}"))?;
        let result = (|| {
            self.runtime
                .render_to_target(id)
                .map_err(|e| format!("cannot draw the preview: {e}"))?;
            let mut readback = self
                .runtime
                .renderer()
                .start_readback(id)
                .map_err(|e| format!("cannot read the picture back: {e}"))?;
            let (w, h, rgba) = self.read_capture(&mut readback)?;
            let mut png = Vec::new();
            image::ImageEncoder::write_image(
                image::codecs::png::PngEncoder::new(&mut png),
                &rgba,
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| format!("cannot encode the PNG: {e}"))?;
            Ok(png)
        })();
        self.runtime.renderer_mut().release_texture(id);
        result
    }

    /// The preview as the AI reads it.
    fn state_text(&self, core: &EditorCore) -> String {
        let status = match self.runtime.status() {
            PlayStatus::Stopped => "stopped",
            PlayStatus::Playing => "playing",
            PlayStatus::Paused => "paused",
        };
        let snapshot = self.runtime.snapshot();
        let value = match core.story() {
            Some(story) => crate::live::preview_state_json(story, status, snapshot.as_ref()),
            None => serde_json::json!({"status": status}),
        };
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }

    /// Runs one live-only preview tool (see `crate::live`).
    fn mcp_run(
        &mut self,
        core: &mut EditorCore,
        egui_renderer: &mut egui_wgpu::Renderer,
        now: Instant,
        call: &LiveCall,
    ) -> Result<PreviewOutput, ToolError> {
        // A pending edit reaches the runtime before anything else.
        if self.debounce.pending() {
            self.load(core, now);
        }
        match call {
            LiveCall::PreviewPlay { from_node } => {
                let command = match from_node {
                    Some(text) => {
                        let id: orbit_core::NodeId = orbit_mcp::convert::parse_id(text, "node")?;
                        if core.story().is_none_or(|s| s.locate(id).is_none()) {
                            return Err(ToolError::not_found(format!("no node {id}")));
                        }
                        PreviewCommand::PlayFrom(id)
                    }
                    None => PreviewCommand::Restart,
                };
                core.state.preview.command(command);
                self.update(core, egui_renderer, now);
                if self.runtime.status() == PlayStatus::Stopped {
                    let why = self
                        .runtime
                        .last_error()
                        .map(|e| e.lines().next().unwrap_or("").to_owned())
                        .or_else(|| {
                            core.state
                                .console
                                .entries()
                                .back()
                                .map(|e| e.message.clone())
                        })
                        .unwrap_or_default();
                    return Err(ToolError::refused(format!(
                        "the preview could not start: {why}. Fix the story (validate) and retry."
                    )));
                }
                self.settle();
            }
            LiveCall::PreviewStep {
                steps,
                until_choice,
            } => {
                self.require_running()?;
                let mut advanced = 0;
                for _ in 0..*steps {
                    if *until_choice && self.at_a_stop() {
                        break;
                    }
                    if !self.runtime.advance_one() {
                        break;
                    }
                    advanced += 1;
                }
                self.settle();
                self.update(core, egui_renderer, now);
                return Ok(PreviewOutput {
                    text: format!("advanced {advanced} step(s)\n{}", self.state_text(core)),
                    png: None,
                });
            }
            LiveCall::PreviewChoose(selector) => {
                self.require_running()?;
                let options = match self.runtime.flow() {
                    Some(orbit_runtime::Flow::Choice(options)) => options.clone(),
                    _ => {
                        return Err(ToolError::refused(
                            "the preview is not showing a choice right now (see preview_state)",
                        ));
                    }
                };
                let index = resolve_option(&options, selector)?;
                if !self.runtime.choose_option(index) {
                    return Err(ToolError::refused("the option could not be picked"));
                }
                self.settle();
            }
            LiveCall::PreviewState => {}
            LiveCall::PreviewStop => {
                core.state.preview.command(PreviewCommand::Stop);
            }
            LiveCall::PreviewScreenshot => {
                self.require_running()?;
                self.settle();
                let png = self.capture_png().map_err(|e| {
                    ToolError::new(
                        orbit_mcp::ErrorKind::Internal,
                        format!("screenshot failed: {e}"),
                    )
                })?;
                self.update(core, egui_renderer, now);
                return Ok(PreviewOutput {
                    text: self.state_text(core),
                    png: Some(png),
                });
            }
            LiveCall::CaptureScene3d {
                scene,
                camera,
                width,
                height,
            } => {
                let png = self.capture_scene3d(core, scene, camera.as_deref(), [*width, *height])?;
                let text = serde_json::json!({
                    "scene": scene,
                    "camera": camera,
                    "size": [width, height],
                })
                .to_string();
                return Ok(PreviewOutput {
                    text,
                    png: Some(png),
                });
            }
            LiveCall::SelectNode { .. } | LiveCall::Export(_) => {
                return Err(ToolError::new(
                    orbit_mcp::ErrorKind::Internal,
                    "not a preview tool",
                ));
            }
        }
        self.update(core, egui_renderer, now);
        Ok(PreviewOutput {
            text: self.state_text(core),
            png: None,
        })
    }

    /// A 3D scene of the project drawn from one of its cameras (`capture_scene3d`):
    /// the file as saved on disk, with the models and images next to it.
    fn capture_scene3d(
        &self,
        core: &EditorCore,
        scene: &str,
        camera: Option<&str>,
        size: [u32; 2],
    ) -> Result<Vec<u8>, ToolError> {
        use orbit_core::AssetKind;
        let internal = |m: String| ToolError::new(orbit_mcp::ErrorKind::Internal, m);
        let story = core
            .story()
            .ok_or_else(|| ToolError::unavailable("no project is open in the editor"))?;
        let root = core
            .session
            .as_ref()
            .map(|s| s.location.root.clone())
            .ok_or_else(|| ToolError::unavailable("no project is open in the editor"))?;
        let id = orbit_mcp::convert::find_asset(story, scene, Some(AssetKind::Scene3d)).or_else(
            |first| {
                orbit_mcp::convert::find_asset(
                    story,
                    &format!("{scene}.scene3d"),
                    Some(AssetKind::Scene3d),
                )
                .map_err(|_| first)
            },
        )?;
        let rel = story
            .assets
            .get(id)
            .map(|a| a.path.clone())
            .ok_or_else(|| ToolError::not_found(format!("no 3D scene `{scene}`")))?;
        let abs = root.join(&rel);
        let text = std::fs::read_to_string(&abs)
            .map_err(|e| internal(format!("cannot read `{rel}`: {e}")))?;
        let parsed = orbit_scene::from_ron(&text)
            .map_err(|e| ToolError::invalid(format!("`{rel}` is not a valid 3D scene: {e}")))?;
        if let Some(wanted) = camera
            && !parsed.cameras().any(|(_, c)| c.name == wanted)
        {
            let names: Vec<&str> = parsed.cameras().map(|(_, c)| c.name.as_str()).collect();
            return Err(ToolError::not_found(format!(
                "the scene has no camera `{wanted}` (cameras: {})",
                names.join(", ")
            )));
        }
        let models = abs.parent().map(|d| d.join("models"));
        let (lib, _skipped) = models.map_or_else(
            Default::default,
            |d| crate::builtin::scene3d::model::ModelLib::load_dir(&d),
        );
        let assets = crate::scene3d_host::ModelAssets {
            prefabs: lib.prefabs.values().cloned().collect(),
            meshes: lib.meshes.values().cloned().collect(),
            textures: lib.textures.clone(),
        };
        crate::scene3d_host::capture_png(
            self.gfx().device(),
            self.gfx().queue(),
            &parsed,
            &assets,
            camera,
            size,
        )
        .map_err(|e| internal(format!("3D capture failed: {e}")))
    }

    fn require_running(&self) -> Result<(), ToolError> {
        if self.runtime.status() == PlayStatus::Stopped {
            Err(ToolError::refused(
                "the preview is not running: call preview_play first",
            ))
        } else {
            Ok(())
        }
    }

    /// The story waits for a pick, is over or failed.
    fn at_a_stop(&self) -> bool {
        !matches!(
            self.runtime.flow(),
            Some(
                orbit_runtime::Flow::Say(_)
                    | orbit_runtime::Flow::Transition { .. }
                    | orbit_runtime::Flow::Waiting { .. }
            )
        )
    }
}

/// The models, meshes and textures of `dirs` (the first folder wins on a
/// name clash, like at export).
fn models3d_library(dirs: &[PathBuf]) -> orbit_runtime::library3d::Library3d {
    let mut out = orbit_runtime::library3d::Library3d::default();
    let mut prefabs = std::collections::HashSet::new();
    let mut meshes = std::collections::HashSet::new();
    let mut textures = std::collections::HashSet::new();
    for dir in dirs {
        let (lib, errors) = crate::builtin::scene3d::model::ModelLib::load_dir(dir);
        for e in errors {
            log::warn!("preview: 3D model {e}");
        }
        for (name, p) in lib.prefabs {
            if prefabs.insert(name) {
                out.prefabs.push(p);
            }
        }
        for (name, m) in lib.meshes {
            if meshes.insert(name) {
                out.meshes.push(m);
            }
        }
        for t in lib.textures {
            if textures.insert(t.name.clone()) {
                out.textures.push(t);
            }
        }
    }
    out
}

/// Which option a selector designates: an index, an exact text (case
/// insensitive) or the beginning of exactly one option.
fn resolve_option(options: &[String], selector: &ChoiceSelector) -> Result<usize, ToolError> {
    let list = || {
        options
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{i}: {t}"))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    match selector {
        ChoiceSelector::Index(i) if *i < options.len() => Ok(*i),
        ChoiceSelector::Index(i) => Err(ToolError::invalid(format!(
            "option {i} does not exist; options are {}",
            list()
        ))),
        ChoiceSelector::Text(text) => {
            let wanted = text.trim().to_lowercase();
            if let Some(i) = options.iter().position(|o| o.to_lowercase() == wanted) {
                return Ok(i);
            }
            let starts: Vec<usize> = options
                .iter()
                .enumerate()
                .filter(|(_, o)| o.to_lowercase().starts_with(&wanted))
                .map(|(i, _)| i)
                .collect();
            match starts.as_slice() {
                [one] if !wanted.is_empty() => Ok(*one),
                _ => Err(ToolError::invalid(format!(
                    "no unique option matches `{text}`; options are {}",
                    list()
                ))),
            }
        }
    }
}

/// The preview as the live MCP's backend: borrows the host and what it needs
/// for one pump of the queue.
pub struct PreviewBridge<'a> {
    pub host: &'a mut PreviewHost,
    pub renderer: &'a mut egui_wgpu::Renderer,
    pub now: Instant,
}

impl PreviewBackend for PreviewBridge<'_> {
    fn run(&mut self, core: &mut EditorCore, call: &LiveCall) -> Result<PreviewOutput, ToolError> {
        self.host.mcp_run(core, self.renderer, self.now, call)
    }
}

/// Creates a render target of `size` and shows it to egui as a texture (the
/// same egui id on a resize, so the shapes already built stay valid).
fn create_shared_target(
    runtime: &mut Runtime,
    egui_renderer: &mut egui_wgpu::Renderer,
    name: &str,
    size: [u32; 2],
    egui_texture: &mut Option<egui::TextureId>,
) -> Option<Target> {
    let id = match runtime
        .renderer_mut()
        .create_render_target(name, size[0], size[1])
    {
        Ok(id) => id,
        Err(e) => {
            log::error!("cannot create the {name} target: {e}");
            return None;
        }
    };
    let Some(view) = runtime.renderer().target_gamma_view(id) else {
        log::error!("the {name} target has no gamma view");
        return None;
    };
    let device = runtime.renderer().gfx().device().clone();
    match *egui_texture {
        Some(texture) => egui_renderer.update_egui_texture_from_wgpu_texture(
            &device,
            &view,
            wgpu::FilterMode::Linear,
            texture,
        ),
        None => {
            *egui_texture = Some(egui_renderer.register_native_texture(
                &device,
                &view,
                wgpu::FilterMode::Linear,
            ));
        }
    }
    log::info!("{name} target {}x{}", size[0], size[1]);
    Some(Target { id, size })
}

/// The part of the published state a change of which needs a UI frame.
#[derive(PartialEq, Debug)]
struct Published {
    status: PreviewStatus,
    speed: PreviewSpeed,
    skipping: bool,
    muted: bool,
    texture: Option<egui::TextureId>,
    texture_size: [u32; 2],
    node: Option<orbit_core::NodeId>,
    flow: String,
    error: Option<String>,
    available: bool,
}

impl Published {
    fn of(p: &orbit_editor_api::PreviewState) -> Self {
        Self {
            status: p.status,
            speed: p.speed,
            skipping: p.skipping,
            muted: p.muted,
            texture: p.texture,
            texture_size: p.texture_size,
            node: p.current_node,
            flow: p.flow.clone(),
            error: p.error.clone(),
            available: p.available,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ms: u64) -> Instant {
        // A fixed origin per test run.
        static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *ORIGIN.get_or_init(Instant::now) + Duration::from_millis(ms)
    }

    #[test]
    fn a_choice_is_picked_by_index_exact_text_or_a_unique_beginning() {
        let options: Vec<String> = ["Go left", "Go right", "Stay"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let pick = |s: ChoiceSelector| resolve_option(&options, &s);
        assert_eq!(pick(ChoiceSelector::Index(2)).unwrap(), 2);
        assert_eq!(pick(ChoiceSelector::Text("stay".into())).unwrap(), 2);
        assert_eq!(pick(ChoiceSelector::Text("Go r".into())).unwrap(), 1);
        // Ambiguous, unknown and out of range: the error lists the options.
        for bad in [
            ChoiceSelector::Text("Go".into()),
            ChoiceSelector::Text("nope".into()),
            ChoiceSelector::Text(String::new()),
            ChoiceSelector::Index(3),
        ] {
            let err = pick(bad).unwrap_err();
            assert!(err.message.contains("0: Go left"), "{}", err.message);
        }
    }

    #[test]
    fn a_burst_of_edits_reloads_once_after_a_quiet_moment() {
        let mut d = Debounce::new(Duration::from_millis(100), Duration::from_millis(250));
        assert!(!d.pending());
        assert_eq!(d.deadline(), None);
        d.changed(t(0));
        d.changed(t(60));
        d.changed(t(120));
        assert!(!d.due(t(200)), "still typing");
        assert!(d.due(t(220)), "100 ms after the last edit");
        assert_eq!(d.take(), Some(t(0)), "the burst began at the first edit");
        assert!(!d.pending());
        assert_eq!(d.take(), None);
    }

    #[test]
    fn a_long_burst_is_cut_at_the_maximum_wait() {
        let mut d = Debounce::new(Duration::from_millis(100), Duration::from_millis(250));
        for ms in (0..=400).step_by(50) {
            d.changed(t(ms));
        }
        // Edits every 50 ms would postpone forever; the deadline is capped.
        assert_eq!(d.deadline(), Some(t(250)));
        assert!(d.due(t(260)));
    }

    #[test]
    fn the_target_follows_the_panel_without_thrashing() {
        assert_eq!(choose_target_size(None, [500, 281]), [512, 288]);
        assert_eq!(choose_target_size(None, [10, 10]), [320, 180]);
        assert_eq!(choose_target_size(None, [4000, 2000]), [1920, 1080]);
        // A few pixels of resize keep the current target.
        assert_eq!(choose_target_size(Some([512, 288]), [530, 298]), [512, 288]);
        assert_eq!(choose_target_size(Some([512, 288]), [470, 264]), [512, 288]);
        // A real change makes a new one, still 16:9.
        let [w, h] = choose_target_size(Some([512, 288]), [900, 506]);
        assert_eq!([w, h], [896, 504]);
        assert_eq!(w * 9, h * 16);
    }

    #[test]
    fn game_keys_map_to_the_keys_the_runtime_knows() {
        assert_eq!(key_code(egui::Key::Enter), Some(KeyCode::Enter));
        assert_eq!(key_code(egui::Key::Num3), Some(KeyCode::Digit3));
        assert_eq!(key_code(egui::Key::H), Some(KeyCode::KeyH));
        assert_eq!(key_code(egui::Key::Z), None);
    }

    #[test]
    fn panel_input_becomes_runtime_input() {
        assert_eq!(
            translate(&PreviewInput::PointerMoved([960.0, 540.0])),
            vec![InputEvent::CursorMoved { x: 960.0, y: 540.0 }]
        );
        assert_eq!(
            translate(&PreviewInput::Primary { pressed: true }),
            vec![InputEvent::MouseButton {
                button: MouseButton::Left,
                pressed: true
            }]
        );
        let keys = translate(&PreviewInput::Key {
            key: egui::Key::Space,
            pressed: true,
            shift: false,
            alt: false,
        });
        assert_eq!(
            keys.last(),
            Some(&InputEvent::Key {
                code: KeyCode::Space,
                pressed: true
            })
        );
        // A key the game ignores sends nothing.
        assert!(
            translate(&PreviewInput::Key {
                key: egui::Key::Z,
                pressed: true,
                shift: false,
                alt: false
            })
            .is_empty()
        );
    }

    #[test]
    fn a_replaced_asset_file_is_reported_once() {
        let dir = std::env::temp_dir().join(format!("orbit-asset-stamps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bg.png");
        std::fs::write(&file, b"one").unwrap();
        let mut story = Story::new("t");
        story.assets.assets.push(orbit_core::AssetEntry {
            id: orbit_core::AssetId::new(),
            kind: orbit_core::AssetKind::Image,
            path: "bg.png".into(),
        });
        let mut stamps = HashMap::new();
        assert!(
            changed_assets(&mut stamps, &dir, &story).is_empty(),
            "first sighting"
        );
        assert!(
            changed_assets(&mut stamps, &dir, &story).is_empty(),
            "unchanged"
        );
        let later = SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(changed_assets(&mut stamps, &dir, &story), ["bg.png"]);
        assert!(
            changed_assets(&mut stamps, &dir, &story).is_empty(),
            "reported once"
        );
        // A missing file is simply forgotten.
        std::fs::remove_file(&file).unwrap();
        assert!(changed_assets(&mut stamps, &dir, &story).is_empty());
        assert!(stamps.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relative_story_path_never_gives_an_empty_assets_root() {
        // Tests run in the crate folder, where `Cargo.toml` exists: the
        // last ancestor of `src/builtin` is the empty path.
        let mut story = Story::new("t");
        story.assets.assets.push(orbit_core::AssetEntry {
            id: orbit_core::AssetId::new(),
            kind: orbit_core::AssetKind::Image,
            path: "Cargo.toml".into(),
        });
        assert_eq!(
            find_assets_root(Path::new("src/builtin"), &story),
            Path::new(".")
        );
        assert_eq!(find_assets_root(Path::new(""), &story), Path::new("."));
    }

    #[test]
    fn the_assets_root_is_found_above_a_story_file() {
        let dir = std::env::temp_dir().join(format!("orbit-assets-root-{}", std::process::id()));
        let stories = dir.join("examples").join("stories");
        let assets = dir.join("assets").join("demo");
        std::fs::create_dir_all(&stories).unwrap();
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("bg.png"), b"x").unwrap();
        let mut story = Story::new("t");
        story.assets.assets.push(orbit_core::AssetEntry {
            id: orbit_core::AssetId::new(),
            kind: orbit_core::AssetKind::Image,
            path: "assets/demo/bg.png".into(),
        });
        assert_eq!(find_assets_root(&stories, &story), dir);
        // A project folder that holds its assets is its own root.
        assert_eq!(find_assets_root(&dir, &story), dir);
        // Nothing found: the given folder.
        story.assets.assets[0].path = "missing.png".into();
        assert_eq!(find_assets_root(&stories, &story), stories);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ui_stamps_see_added_changed_and_removed_files() {
        let dir = std::env::temp_dir().join(format!("orbit_ui_stamps_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ui_stamps(&dir).is_empty(), "a missing folder is empty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.screen.ron"), "1").unwrap();
        let first = ui_stamps(&dir);
        assert_eq!(first.len(), 1);
        assert_eq!(ui_stamps(&dir), first, "stable while nothing changes");
        std::fs::write(dir.join("a.screen.ron"), "12").unwrap();
        assert_ne!(ui_stamps(&dir), first, "length change is seen");
        std::fs::write(dir.join("b.screen.ron"), "1").unwrap();
        assert_eq!(ui_stamps(&dir).len(), 2);
        std::fs::remove_file(dir.join("a.screen.ron")).unwrap();
        assert_eq!(ui_stamps(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The preview's hot reload after the first edit of a big story runs on
    /// the UI thread: its frame must stay under the budget. Prints the
    /// breakdown. Skipped (with a message) without a GPU adapter.
    #[test]
    fn the_first_edit_hot_reload_of_a_big_story_stays_within_budget() {
        let Ok(mut gfx) = pollster::block_on(Gfx::headless((64, 64))) else {
            eprintln!("no GPU adapter: skipping the preview reload test");
            return;
        };
        gfx.enable_gamma_view();
        let dir = std::env::temp_dir().join(format!("orbit-preview-reload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let story = crate::bench::generate_story(10_000);
        std::fs::write(dir.join("story.orbit"), story.to_ron().unwrap()).unwrap();
        std::fs::write(
            dir.join(orbit_project::MANIFEST_FILE),
            "(format: 1, name: \"Reload\", story: \"story.orbit\", plugins: [])",
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut core = EditorCore::new(crate::core::CoreOptions::with_language(orbit_editor_api::Lang::En), &ctx);
        core.open_project(&dir);
        let mut preview = PreviewHost::new(gfx).unwrap();
        let mut er = egui_wgpu::Renderer::new(
            preview.gfx().device(),
            wgpu::TextureFormat::Rgba8Unorm,
            egui_wgpu::RendererOptions::default(),
        );
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let timed = |preview: &mut PreviewHost, core: &mut EditorCore, er: &mut egui_wgpu::Renderer, now: Instant| {
            orbit_profile::begin_frame();
            let t = Instant::now();
            // The preview panel is on screen (the reload waits for it otherwise).
            core.state.panels.begin_frame();
            core.state.panels.mark_drawn(crate::builtin::panels::PREVIEW);
            preview.update(core, er, now);
            let took = t.elapsed();
            orbit_profile::finish_frame();
            let f = orbit_profile::frames(1).pop().unwrap();
            (took, orbit_profile::Hitch::of(&f, 0).summary())
        };
        let start = Instant::now();
        let (open, open_spans) = timed(&mut preview, &mut core, &mut er, start);

        // An edit, then the frame where the debounced reload happens.
        let line = core.story().unwrap().scenes[0]
            .nodes
            .iter()
            .find_map(|n| match &n.kind {
                orbit_core::NodeKind::Say { text_id, .. } => Some(*text_id),
                _ => None,
            })
            .unwrap();
        {
            let session = core.session.as_mut().unwrap();
            let mut ectx = orbit_editor_api::EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
            ectx.exec(orbit_project::Command::SetString { id: line, text: Some("edited".into()) }).unwrap();
        }
        let _ = timed(&mut preview, &mut core, &mut er, start + Duration::from_millis(10));
        let (reload, reload_spans) = timed(&mut preview, &mut core, &mut er, start + Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!(
            "preview ({}, 10k nodes): first load {:.1} ms [{open_spans}]\n  first edit hot reload {:.1} ms [{reload_spans}]",
            if cfg!(debug_assertions) { "debug" } else { "release" },
            ms(open),
            ms(reload)
        );
        let budget = Duration::from_millis(if cfg!(debug_assertions) { 150 } else { 50 });
        assert!(reload <= budget, "the hot reload took {:.1} ms", ms(reload));
    }

    /// The scene view's picture of the stage at `node` (what the preview
    /// shows at that line), read back as 1920×1080 RGBA.
    fn stage_pixels(preview: &mut PreviewHost, core: &mut EditorCore, node: orbit_core::NodeId) -> Vec<u8> {
        preview.load(core, Instant::now());
        let story = Arc::new(core.story().unwrap().clone());
        let (source, _) =
            scene_view::seek_before(&story, node, orbit_vm::SeekLimits::default()).unwrap();
        let at = scene_view::scene_at(&story, &source, node).unwrap();
        let stage = scene_view::stage_state(&story, &at.after);
        assert!(stage.scene3d.is_some(), "the 3D scene is on the stage at that line");
        let rt = &mut preview.runtime;
        for _ in 0..2000 {
            rt.request_stage_assets(&story, &stage);
            rt.tick(Duration::from_millis(5));
            if !rt.is_loading() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let target = rt.renderer_mut().create_canvas_target("test stage").unwrap();
        rt.render_stage_to_target_at(&story, &stage, target, None).unwrap();
        let r = rt.renderer();
        let mut readback = r.start_readback(target).unwrap();
        for _ in 0..10_000 {
            if let ReadbackStatus::Ready { rgba, .. } = r.poll_readback(&mut readback) {
                return rgba;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("read-back timed out");
    }

    fn px(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
        let i = (y * 1920 + x) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    /// Regression ("the scene still does not show my character"): a 3D
    /// scene edited at the project root (`scene.scene3d.ron` + `models/`)
    /// was imported into the library as `assets/3d/scene_scene3d.ron`, the
    /// scene file alone: the story showed that copy, whose `models/` folder
    /// did not exist, so the preview drew the scene's background without
    /// the (skinned) character. The import now keeps the `.scene3d.ron`
    /// name and brings the models; the character is drawn at its place.
    #[test]
    fn a_story_showing_a_3d_scene_draws_its_skinned_model_in_the_preview() {
        use orbit_core::NodeKind;
        use orbit_project::ops;
        let Ok(mut gfx) = pollster::block_on(Gfx::headless((64, 64))) else {
            eprintln!("no GPU adapter: skipping the 3D preview test");
            return;
        };
        gfx.enable_gamma_view();
        let dir = std::env::temp_dir().join(format!("orbit-preview-s3d-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = crate::storage::create_project(
            &dir,
            "Hero",
            &crate::i18n::translator(orbit_editor_api::Lang::En),
        )
        .unwrap();
        // Edited in the 3D editor, saved at the project root.
        let edited = root.join("scene.scene3d.ron");
        let scene = crate::testing3d::scene_with_model("hero");
        crate::testing3d::write_scene_with_models(&edited, &scene, "hero");
        // What the library import used to make of it: the file alone.
        std::fs::create_dir_all(root.join("assets/3d")).unwrap();
        std::fs::copy(&edited, root.join("assets/3d/scene_scene3d.ron")).unwrap();

        let ctx = egui::Context::default();
        let mut core = EditorCore::new(
            crate::core::CoreOptions::with_language(orbit_editor_api::Lang::En),
            &ctx,
        );
        core.open_project(&root);
        let with_ctx = |core: &mut EditorCore, f: &mut dyn FnMut(&mut orbit_editor_api::EditorContext<'_>)| {
            let session = core.session.as_mut().unwrap();
            let mut ectx = orbit_editor_api::EditorContext::new(
                Some(&mut session.project),
                &mut core.state,
                &core.registry,
            );
            f(&mut ectx);
        };
        let show_then_say = |core: &mut EditorCore, asset: orbit_core::AssetId| -> orbit_core::NodeId {
            let mut ids = Vec::new();
            with_ctx(core, &mut |c| {
                let scene = c.story().unwrap().scenes[0].id;
                ids = c
                    .ops(|p| {
                        ops::insert_drafts(
                            p,
                            scene,
                            0,
                            vec![
                                ops::NodeDraft::plain(NodeKind::ShowScene3d {
                                    scene: asset,
                                    camera: None,
                                    transition: None,
                                }),
                                ops::NodeDraft::say(None, "Mira! I saved you a seat."),
                            ],
                            "test",
                        )
                    })
                    .unwrap();
            });
            ids[1]
        };
        let mut stale = None;
        with_ctx(&mut core, &mut |c| {
            stale = Some(
                c.ops(|p| {
                    ops::add_asset(p, orbit_core::AssetKind::Scene3d, "assets/3d/scene_scene3d.ron")
                })
                .unwrap(),
            );
        });
        let line = show_then_say(&mut core, stale.unwrap());
        let mut preview = PreviewHost::new(gfx).unwrap();

        // The stale copy: background only, and the Problems panel says why.
        let out = stage_pixels(&mut preview, &mut core, line);
        let (centre, corner) = (px(&out, 960, 540), px(&out, 40, 40));
        assert!(
            centre[2] > centre[0] && corner[2] > corner[0],
            "no model in the copy: blue background only ({centre:?}, {corner:?})"
        );
        let issues = crate::assets::lint_scene3d(&root, core.story().unwrap());
        assert!(
            issues.iter().any(|i| matches!(
                i,
                crate::assets::AssetIssue::Scene3dModelMissing { model, folder, .. }
                    if model == "hero" && folder == "assets/3d/models/"
            )),
            "{issues:?}"
        );

        // Imported as it should: name kept, models brought along.
        let report = {
            let session = core.session.as_mut().unwrap();
            crate::assets::import_files(
                &mut session.project,
                &root,
                &core.registry.project,
                std::slice::from_ref(&edited),
                &crate::i18n::translator(orbit_editor_api::Lang::En),
            )
        };
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        let imported = &report.imported[0];
        assert_eq!(imported.path, "assets/3d/scene.scene3d.ron");
        assert!(root.join("assets/3d/models/hero.gltf").is_file());
        assert!(root.join("assets/3d/models/hero.bin").is_file());
        let line = show_then_say(&mut core, imported.id);
        core.asset_epoch += 1;
        let out = stage_pixels(&mut preview, &mut core, line);
        let (centre, corner) = (px(&out, 960, 540), px(&out, 40, 40));
        assert_ne!(centre, corner, "the character is drawn over the background");
        assert!(
            centre[0] > 120 && centre[0] > centre[1] + 60 && centre[0] > centre[2] + 60,
            "red model at the centre: {centre:?}"
        );
        drop(preview);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
