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
use orbit_editor_api::{
    Level, PreviewCommand, PreviewInput, PreviewSpeed, PreviewStatus, Selection,
};
use orbit_gfx::{Gfx, wgpu};
use orbit_platform::{InputEvent, KeyCode, MouseButton};
use orbit_render2d::{Readback, ReadbackStatus, Renderer2d, TextureId};
use orbit_runtime::{
    EmbeddedConfig, LoadOutcome, PlayFrom, PlaySpeed, PlayStatus, Runtime, RuntimeError,
};

use crate::core::EditorCore;
use crate::live::{PreviewBackend, PreviewOutput};
use orbit_mcp::ToolError;
use orbit_mcp::live_tools::{ChoiceSelector, LiveCall};

/// A reload waits for this long without a new edit (typing is a stream of
/// edits)...
pub const RELOAD_QUIET: Duration = Duration::from_millis(100);
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

/// The folder the manifest's asset paths are relative to. A project folder
/// is its own root; a story file opened alone (the demo) may keep its assets
/// in a parent folder (`assets/demo/...` next to `examples/`): the first
/// ancestor where the first declared asset exists wins.
pub fn find_assets_root(root: &Path, story: &Story) -> PathBuf {
    let found = story.assets.assets.first().and_then(|first| {
        root.ancestors()
            .take(5)
            .find(|dir| dir.join(&first.path).exists())
    });
    let dir = found.unwrap_or(root);
    // The last ancestor of a relative path is `""`, which `read_dir` refuses.
    if dir.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        dir.to_owned()
    }
}

/// Assets of `story` whose file changed since the last call (by modification
/// time), so the runtime fetches them again instead of showing a stale
/// texture. The first sighting of a file only records it.
pub fn changed_assets(
    stamps: &mut HashMap<String, SystemTime>,
    root: &Path,
    story: &Story,
) -> Vec<String> {
    let mut changed = Vec::new();
    for asset in &story.assets.assets {
        let Ok(modified) = std::fs::metadata(root.join(&asset.path)).and_then(|m| m.modified())
        else {
            stamps.remove(&asset.path);
            continue;
        };
        if let Some(previous) = stamps.insert(asset.path.clone(), modified)
            && previous != modified
        {
            changed.push(asset.path.clone());
        }
    }
    changed
}

fn flow_text(flow: Option<&orbit_runtime::director::Flow>) -> String {
    use orbit_runtime::director::Flow;
    match flow {
        None => String::new(),
        Some(Flow::Say(_)) => "réplique".into(),
        Some(Flow::Choice(_)) => "choix".into(),
        Some(Flow::Waiting { .. }) => "pause".into(),
        Some(Flow::Transition { .. }) => "transition".into(),
        Some(Flow::Ended) => "fin de l'histoire".into(),
        Some(Flow::Failed(_)) => "erreur".into(),
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
    /// Last reload latency in milliseconds, for the debug overlay and tests.
    pub last_reload_ms: Option<u128>,
    /// `EditorCore::asset_epoch` as last seen: files changed on disk.
    seen_asset_epoch: u64,
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
            last_reload_ms: None,
            seen_asset_epoch: 0,
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

    /// Loads the project's current story into the runtime.
    fn load(&mut self, core: &mut EditorCore, now: Instant) {
        let Some(session) = core.session.as_ref() else {
            return;
        };
        let started = Instant::now();
        let story = Arc::new(session.project.story().clone());
        let root = find_assets_root(&session.location.root, &story);
        if root != Path::new(self.runtime.assets_root()) {
            self.asset_stamps.clear();
        }
        for path in changed_assets(&mut self.asset_stamps, &root, &story) {
            if self.runtime.invalidate_asset(&path) {
                log::info!("preview: asset `{path}` changed on disk, reloaded");
            }
        }
        let outcome = self.runtime.load_story(story, &root.to_string_lossy());
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
            core.state
                .console
                .push(Level::Warn, format!("Preview : {head}"));
        }
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
                    self.load(core, now);
                    force_draw = true;
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
        if self.debounce.due(now) {
            self.load(core, now);
            force_draw = true;
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

        // ---- notices, publication
        for notice in self.runtime.take_notices() {
            let level = if notice.warning {
                Level::Warn
            } else {
                Level::Info
            };
            core.state
                .console
                .push(level, format!("Preview : {}", notice.text));
        }
        self.publish(core);
        let after = Published::of(&core.state.preview);
        if after != before {
            log::debug!(target: "orbit_editor::perf", "preview state changed: {before:?} -> {after:?}");
            wake.redraw = true;
        }

        // ---- when to come back
        let mut at = self.debounce.deadline();
        if animating {
            let next = now + FRAME_INTERVAL;
            at = Some(at.map_or(next, |d| d.min(next)));
        }
        wake.at = at;
        wake
    }

    fn apply(&mut self, command: PreviewCommand) {
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
        }
    }

    fn set_target(&mut self, egui_renderer: &mut egui_wgpu::Renderer, size: [u32; 2]) {
        let created = self
            .runtime
            .renderer_mut()
            .create_render_target("preview", size[0], size[1]);
        let id = match created {
            Ok(id) => id,
            Err(e) => {
                log::error!("cannot create the preview target: {e}");
                return;
            }
        };
        let Some(view) = self.runtime.renderer().target_gamma_view(id) else {
            log::error!("the preview target has no gamma view");
            return;
        };
        let device = self.runtime.renderer().gfx().device().clone();
        match self.egui_texture {
            // Same egui id: the shapes of this frame stay valid.
            Some(texture) => egui_renderer.update_egui_texture_from_wgpu_texture(
                &device,
                &view,
                wgpu::FilterMode::Linear,
                texture,
            ),
            None => {
                self.egui_texture = Some(egui_renderer.register_native_texture(
                    &device,
                    &view,
                    wgpu::FilterMode::Linear,
                ));
            }
        }
        if let Some(old) = self.target.replace(Target { id, size }) {
            self.runtime.renderer_mut().release_texture(old.id);
        }
        log::info!("preview target {}x{}", size[0], size[1]);
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
}
