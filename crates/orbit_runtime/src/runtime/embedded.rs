//! The runtime hosted by an editor ("embedded"): the real game, drawn into a
//! render target the host shows in a panel.
//!
//! What differs from the player:
//! * the story comes from memory ([`Runtime::load_story`]), not from a file,
//!   and a new version of it **hot reloads** under a running game: the
//!   stage and the position are carried over ([`crate::preview`]);
//! * the host drives time ([`Runtime::tick`], only while it plays), feeds the
//!   input it routes to the panel ([`Runtime::inject_input`], coordinates
//!   already in virtual pixels) and asks for a frame
//!   ([`Runtime::render_to_target`]) when [`Runtime::is_animating`] or
//!   [`Runtime::take_dirty`] says something changed. A game waiting for the
//!   player with nothing moving is a still image: zero frames;
//! * no "click to start" screen, no main menu (leaving the game starts it
//!   again), saves and preferences in memory, never the real ones.

use orbit_core::NodeId;
use orbit_platform::InputEvent;
use orbit_vm::SeekLimits;

use super::*;
use crate::director::Flow;
use crate::preview::{
    PlannedStart, ResumeSource, StartKind, plan_from_beginning, plan_from_node, plan_resume,
};

/// What an embedded runtime needs from its host.
#[derive(Clone)]
pub struct EmbeddedConfig {
    /// Font files (bytes) for the game text: the host embeds them, there is
    /// no engine folder to read them from.
    pub fonts: Vec<Vec<u8>>,
    pub font_family: String,
    pub theme: Theme,
}

/// Where the preview is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayStatus {
    Stopped,
    Playing,
    Paused,
}

/// What the preview shows, as plain data (the MCP's `preview_state`).
#[derive(Clone, Debug)]
pub struct PreviewSnapshot {
    pub flow: Flow,
    /// The node of the line or choice on screen.
    pub node: Option<NodeId>,
    pub variables: std::collections::BTreeMap<String, orbit_core::Value>,
    /// Background, characters on stage and music (declarative state).
    pub scene: orbit_vm::SceneState,
}

/// Time multiplier of the preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaySpeed {
    Normal,
    Double,
}

impl PlaySpeed {
    fn factor(self) -> f32 {
        match self {
            Self::Normal => 1.0,
            Self::Double => 2.0,
        }
    }
}

/// Where a new playthrough begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayFrom {
    Beginning,
    /// "Play from here": with the state a player would have at that node.
    Node(NodeId),
}

/// What [`Runtime::load_story`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// The story is in use.
    Applied,
    /// New assets are being fetched; the story is applied once they are in.
    Loading,
    /// The story has errors: the previous one keeps running. Readable
    /// message (also kept in [`Runtime::last_error`]).
    Rejected(String),
}

/// Something to tell the author (the host puts it in its console).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub warning: bool,
    pub text: String,
}

struct Awaiting {
    story: Arc<Story>,
    pending: Vec<PendingAsset>,
    /// A playthrough asked for while the assets were loading.
    play: Option<PlayFrom>,
}

/// What the editor asks the preview to show instead of the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiPreviewTarget<'a> {
    /// The data screen with this id of the bundle.
    Screen(&'a str),
    /// The splash sequence (engine splash, then the bundle's), in a loop.
    Splashes,
}

/// A UI screen or the splashes standing in for the game (editor UI preview).
pub(super) enum UiPreview {
    Screen(Box<ScreenPreview>),
    Splashes(SplashState, Vec<orbit_ui::SplashDef>),
}

/// A screen of the project shown as the player would see it: the screen on
/// top, the screens below it, mock settings and saves, and the feedback of
/// the actions that would act on the game.
pub(super) struct ScreenPreview {
    screen: orbit_ui::DataScreen,
    nav: orbit_ui::PreviewNav,
    bundle: orbit_ui::UiBundle,
    /// Default screens (save, load...) for ids the project does not define.
    stock: orbit_ui::UiBundle,
    theme: Theme,
    texts: orbit_ui::Texts,
    toasts: orbit_ui::ToastStack,
    /// Widget the pointer went down on, taken by the editor.
    pick: Option<(String, Vec<usize>)>,
    /// Widget the editor has selected (outlined).
    highlight: Option<Vec<usize>>,
}

/// Seconds a toast of the preview needs to show and fade.
const TOAST_SETTLE: f32 = 4.0;

impl ScreenPreview {
    fn find(
        bundle: &orbit_ui::UiBundle,
        stock: &orbit_ui::UiBundle,
        id: &str,
    ) -> Option<orbit_ui::ScreenAsset> {
        bundle.screen(id).or_else(|| stock.screen(id)).cloned()
    }

    fn new(
        bundle: &orbit_ui::UiBundle,
        id: &str,
        theme: Theme,
        texts: orbit_ui::Texts,
    ) -> Result<Self, String> {
        let stock = orbit_ui::UiBundle::builtin();
        let asset =
            Self::find(bundle, &stock, id).ok_or_else(|| format!("no screen called `{id}`"))?;
        let nav = orbit_ui::PreviewNav::new(id);
        let screen = orbit_ui::DataScreen::new(asset, &theme, &texts, nav.context("Preview"));
        Ok(Self {
            screen,
            nav,
            bundle: bundle.clone(),
            stock,
            toasts: orbit_ui::ToastStack::new(&theme),
            theme,
            texts,
            pick: None,
            highlight: None,
        })
    }

    /// The editor sent a new version of the bundle: the stack stays, the
    /// screen on top is built again from it.
    fn update_bundle(&mut self, bundle: &orbit_ui::UiBundle, theme: Theme, texts: orbit_ui::Texts) {
        self.bundle = bundle.clone();
        self.theme = theme;
        self.texts = texts;
        self.show();
    }

    /// Builds the screen on top of the stack. When it no longer exists the
    /// current one stays.
    fn show(&mut self) {
        let id = self.nav.top().to_owned();
        if let Some(asset) = Self::find(&self.bundle, &self.stock, &id) {
            self.screen = orbit_ui::DataScreen::new(
                asset,
                &self.theme,
                &self.texts,
                self.nav.context("Preview"),
            );
        }
    }

    /// Applies one input and feeds the navigation with what the screen
    /// reported.
    fn handle(&mut self, input: orbit_ui::UiInput) {
        use orbit_ui::Screen as _;
        if let orbit_ui::UiInput::PointerDown(p) = input
            && let Some(path) = self.screen.widget_at(p)
        {
            self.pick = Some((self.nav.top().to_owned(), path));
        }
        let events = self.screen.handle(input);
        let mut changed = false;
        for event in events {
            let (bundle, stock) = (&self.bundle, &self.stock);
            let step = self
                .nav
                .apply(event, &|id| bundle.screen(id).is_some() || stock.screen(id).is_some());
            for text in step.feedback {
                self.toasts.push(text);
            }
            changed |= step.shown_changed;
        }
        for action in self.screen.take_unsupported() {
            self.toasts.push(orbit_ui::PreviewNav::describe_action(&action));
        }
        if changed {
            self.show();
        } else {
            self.screen.set_context(self.nav.context("Preview"));
        }
    }
}

/// Seconds a screen preview keeps animating after a change or an input
/// (focus and hover transitions), then rests.
const UI_SETTLE: f32 = 0.6;

pub(super) struct Embedded {
    /// The UI being previewed, if any (it hides the game).
    pub(super) ui_preview: Option<UiPreview>,
    ui_settle: f32,
    /// The last accepted story.
    story: Option<Arc<Story>>,
    status: PlayStatus,
    speed: PlaySpeed,
    muted: bool,
    /// Master volume asked by the game's preferences.
    pub(super) master: f32,
    awaiting: Option<Awaiting>,
    notices: Vec<Notice>,
    last_error: Option<String>,
    /// The music the game started (not to restart it at every hot reload).
    music: Option<orbit_core::AssetId>,
    dirty: bool,
    /// Bumped when the story or an asset file was swapped: what a still
    /// image of the stage (see [`Runtime::render_stage_to_target`]) shows may
    /// have changed.
    epoch: u64,
    limits: SeekLimits,
    /// Skip was asked before a playthrough existed: applied when one starts.
    pending_skip: bool,
}

impl Embedded {
    pub(super) fn new(_config: EmbeddedConfig) -> Self {
        Self {
            ui_preview: None,
            ui_settle: 0.0,
            story: None,
            status: PlayStatus::Stopped,
            speed: PlaySpeed::Normal,
            muted: false,
            master: 1.0,
            awaiting: None,
            notices: Vec::new(),
            last_error: None,
            music: None,
            dirty: true,
            epoch: 0,
            limits: SeekLimits::default(),
            pending_skip: false,
        }
    }
}

impl Runtime {
    /// A runtime for an editor: starts stopped and blank. `renderer` is the
    /// host's (it can still reach it, see [`Runtime::renderer_mut`]).
    pub fn new_embedded(
        renderer: Renderer2d,
        config: EmbeddedConfig,
    ) -> Result<Self, RuntimeError> {
        Self::build(renderer, RuntimeConfig::new("", "", "", ""), Some(config))
    }

    pub fn renderer(&self) -> &Renderer2d {
        &self.ctx.renderer
    }

    pub fn renderer_mut(&mut self) -> &mut Renderer2d {
        &mut self.ctx.renderer
    }

    fn emb(&self) -> &Embedded {
        self.embedded
            .as_ref()
            .expect("embedded runtime API used on a player runtime")
    }

    fn emb_mut(&mut self) -> &mut Embedded {
        self.embedded
            .as_mut()
            .expect("embedded runtime API used on a player runtime")
    }

    fn notify(&mut self, warning: bool, text: impl Into<String>) {
        let text = text.into();
        if warning {
            log::warn!("preview: {text}");
        } else {
            log::info!("preview: {text}");
        }
        self.emb_mut().notices.push(Notice { warning, text });
    }

    // ------------------------------------------------------------- story

    /// Gives the runtime the story to play, from memory (nothing is read from
    /// disk but the assets, under `assets_root`). The first call prepares a
    /// stopped preview; later ones are **hot reloads**: a running playthrough
    /// continues from where it was when that node still exists (else from the
    /// closest label before it, else from the beginning). A story that does
    /// not validate is refused and the previous one keeps playing.
    pub fn load_story(&mut self, story: Arc<Story>, assets_root: &str) -> LoadOutcome {
        let report = orbit_core::validate(&story);
        if !report.errors.is_empty() {
            let mut message = format!("{} erreur(s) de validation :", report.errors.len());
            for error in report.errors.iter().take(8) {
                message.push_str(&format!("\n- {error}"));
            }
            if report.errors.len() > 8 {
                message.push_str(&format!("\n… (+{} autres)", report.errors.len() - 8));
            }
            self.emb_mut().last_error = Some(message.clone());
            self.emb_mut().dirty = true;
            return LoadOutcome::Rejected(message);
        }
        self.emb_mut().last_error = None;
        if assets_root != self.assets_loader.root() {
            self.assets_loader = AssetLoader::new(assets_root);
            self.store.clear(&mut self.ctx.renderer);
        }
        let pending = start_loading(&self.assets_loader, &story, &self.store);
        let play = self.emb_mut().awaiting.take().and_then(|a| a.play);
        let done = pending.iter().all(PendingAsset::is_done);
        self.emb_mut().awaiting = Some(Awaiting {
            story,
            pending,
            play,
        });
        if done {
            self.finish_awaiting();
            LoadOutcome::Applied
        } else {
            LoadOutcome::Loading
        }
    }

    /// The project folder holding the data-driven UI (`UiBundle::load_dir`)
    /// the next shells are built with; `None` keeps the stock screens.
    pub fn set_ui_dir(&mut self, dir: Option<String>) {
        self.config.ui_dir = dir;
    }

    /// The text of the last story the runtime refused, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.embedded.as_ref()?.last_error.as_deref()
    }

    /// The folder the story's asset paths are resolved against.
    pub fn assets_root(&self) -> &str {
        self.assets_loader.root()
    }

    /// A file of the project changed: its texture or sound is fetched again
    /// the next time a story is loaded. Returns whether the runtime had it.
    pub fn invalidate_asset(&mut self, path: &str) -> bool {
        let found = self.store.forget(&mut self.ctx.renderer, path);
        if found {
            self.emb_mut().dirty = true;
            self.emb_mut().epoch += 1;
        }
        found
    }

    fn poll_embedded(&mut self) {
        let done = self
            .embedded
            .as_ref()
            .and_then(|e| e.awaiting.as_ref())
            .is_some_and(|a| a.pending.iter().all(PendingAsset::is_done));
        if done {
            self.finish_awaiting();
        }
    }

    /// The assets of the awaited story are in: swap it in.
    fn finish_awaiting(&mut self) {
        let Some(awaiting) = self.emb_mut().awaiting.take() else {
            return;
        };
        let failures = finish_loading(&mut self.ctx.renderer, &mut self.store, awaiting.pending);
        for failure in failures {
            self.notify(true, format!("Asset : {failure}"));
        }
        let story = awaiting.story;
        let resume = match &self.phase {
            Phase::Running(shell) => shell.game().map(|g| ResumeSource::of(g.director.vm())),
            _ => None,
        };
        let failed_while_playing =
            matches!(self.phase, Phase::Failed(_)) && self.emb().status != PlayStatus::Stopped;
        self.emb_mut().story = Some(story.clone());
        self.emb_mut().dirty = true;
        self.emb_mut().epoch += 1;
        if let Some(from) = awaiting.play {
            self.play(from);
        } else if let Some(source) = resume {
            let planned = plan_resume(&story, &source, self.emb().limits);
            self.install(&story, planned, true);
        } else if failed_while_playing {
            self.play(PlayFrom::Beginning);
        }
    }

    /// Replaces the shell by one running `planned` (or reports why not).
    fn install(
        &mut self,
        story: &Arc<Story>,
        planned: Result<PlannedStart, String>,
        instant: bool,
    ) {
        match planned {
            Ok(planned) => {
                if let Some(text) = planned.notice {
                    self.notify(true, text);
                }
                if let StartKind::Path { choices } = planned.kind {
                    log::info!("preview: played from a path of {choices} choice(s)");
                }
                let mut shell = self.build_shell(story.clone(), &[]);
                shell.install_vm(planned.vm, instant);
                if std::mem::take(&mut self.emb_mut().pending_skip) {
                    shell.set_skip(true);
                }
                self.phase = Phase::Running(shell);
                self.after_shell();
            }
            Err(message) => {
                self.notify(true, format!("Impossible de lancer la preview : {message}"));
                self.phase = Phase::Stopped;
                self.emb_mut().status = PlayStatus::Stopped;
            }
        }
        self.apply_embedded_volume();
    }

    // ----------------------------------------------------------- controls

    pub fn status(&self) -> PlayStatus {
        self.emb().status
    }

    /// Whether a story was loaded.
    pub fn has_story(&self) -> bool {
        self.emb().story.is_some()
    }

    /// Starts a new playthrough (from the beginning, or "from here") and
    /// plays it. Asked while assets are loading, it starts when they are in.
    pub fn play(&mut self, from: PlayFrom) {
        if let Some(awaiting) = &mut self.emb_mut().awaiting {
            awaiting.play = Some(from);
            return;
        }
        let Some(story) = self.emb().story.clone() else {
            return;
        };
        self.audio.resume_after_user_gesture();
        // Whatever played before stops: the new run starts its own music.
        self.audio.stop_music(RESTART_MUSIC_FADE);
        let limits = self.emb().limits;
        let planned = match from {
            PlayFrom::Beginning => plan_from_beginning(&story),
            PlayFrom::Node(node) => plan_from_node(&story, node, limits),
        };
        self.emb_mut().music = None;
        self.emb_mut().status = PlayStatus::Playing;
        self.emb_mut().dirty = true;
        self.install(&story, planned, false);
    }

    /// Starts again from the beginning.
    pub fn restart(&mut self) {
        self.play(PlayFrom::Beginning);
    }

    /// Freezes the game and silences it; the picture stays.
    pub fn pause(&mut self) {
        if self.emb().status == PlayStatus::Playing {
            self.emb_mut().status = PlayStatus::Paused;
            self.apply_embedded_volume();
        }
    }

    /// Unfreezes a paused game.
    pub fn resume(&mut self) {
        if self.emb().status == PlayStatus::Paused {
            self.emb_mut().status = PlayStatus::Playing;
            self.audio.resume_after_user_gesture();
            self.apply_embedded_volume();
        }
    }

    /// Ends the playthrough: a blank stage, no sound.
    pub fn stop(&mut self) {
        if self.emb().status == PlayStatus::Stopped {
            return;
        }
        self.audio.stop_music(RESTART_MUSIC_FADE);
        let emb = self.emb_mut();
        emb.status = PlayStatus::Stopped;
        emb.music = None;
        emb.dirty = true;
        self.phase = Phase::Stopped;
        self.apply_embedded_volume();
    }

    pub fn speed(&self) -> PlaySpeed {
        self.emb().speed
    }

    pub fn set_speed(&mut self, speed: PlaySpeed) {
        self.emb_mut().speed = speed;
    }

    /// Skip mode on or off (it stops by itself at a choice).
    pub fn set_skip(&mut self, on: bool) {
        match self.shell_mut() {
            Some(shell) => shell.set_skip(on),
            None => self.emb_mut().pending_skip = on,
        }
        self.emb_mut().dirty = true;
    }

    pub fn is_skipping(&self) -> bool {
        match &self.phase {
            Phase::Running(shell) => shell.game().is_some_and(|g| g.skipping()),
            _ => false,
        }
    }

    pub fn is_muted(&self) -> bool {
        self.emb().muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.emb_mut().muted = muted;
        self.apply_embedded_volume();
    }

    pub(super) fn apply_embedded_volume(&mut self) {
        let emb = self.emb();
        let volume = if emb.muted || emb.status != PlayStatus::Playing {
            0.0
        } else {
            emb.master
        };
        self.audio.set_bus_volume(Bus::Master, volume);
    }

    /// Lets the game's music request through, except restarting the track
    /// that already plays (a hot reload rebuilds the game around it).
    pub(super) fn filter_audio(&mut self, command: AudioCommand) -> Option<AudioCommand> {
        let Some(emb) = self.embedded.as_mut() else {
            return Some(command);
        };
        match &command {
            AudioCommand::PlayMusic { asset, .. } => {
                if emb.music == Some(*asset) && self.audio.is_music_playing() {
                    return None;
                }
                emb.music = Some(*asset);
            }
            AudioCommand::StopMusic { .. } => emb.music = None,
            AudioCommand::PlaySound { .. } => {}
        }
        Some(command)
    }

    // ------------------------------------------------------------ running

    /// Advances the game by `dt` (scaled by the speed), only while playing.
    /// Also finishes the assets being loaded. Call it before drawing.
    pub fn tick(&mut self, dt: Duration) {
        self.audio.update();
        self.poll_embedded();
        if self.tick_ui_preview(dt) {
            return;
        }
        if self.emb().status != PlayStatus::Playing {
            return;
        }
        // What moved during this tick must be drawn once more even when it
        // came to rest in it (a typewriter finishing, skip landing on a choice).
        let was_animating = self.is_animating();
        let dt_secs = dt.as_secs_f32().min(MAX_DT) * self.emb().speed.factor();
        self.time += dt_secs;
        if let Phase::Running(shell) = &mut self.phase {
            shell.set_now(orbit_platform::unix_time_ms());
            shell.update(dt_secs);
        }
        self.service_thumbnails();
        self.after_shell();
        if let Phase::Failed(message) = &self.phase {
            let message = message.clone();
            self.emb_mut().status = PlayStatus::Stopped;
            self.emb_mut().dirty = true;
            self.notify(true, format!("L'histoire s'est arrêtée : {message}"));
        }
        if was_animating || self.is_animating() {
            self.emb_mut().dirty = true;
        }
    }

    /// Routes an input event to the game (only while playing). Pointer
    /// coordinates are in virtual pixels (1920x1080 canvas).
    pub fn inject_input(&mut self, event: &InputEvent) {
        if self.emb().ui_preview.is_some() {
            if let Some(action) = self.mapper.map(event, |p| p) {
                self.ui_preview_input(action);
            }
            return;
        }
        if self.emb().status != PlayStatus::Playing {
            return;
        }
        if let Some(action) = self.mapper.map(event, |p| p) {
            self.on_action(action);
            self.emb_mut().dirty = true;
        }
    }

    /// Whether time alone changes the picture (typewriter, pause timer,
    /// transition, skip / auto, toast, assets loading): the host keeps
    /// ticking and drawing. `false` for a game waiting for input.
    pub fn is_animating(&self) -> bool {
        let Some(emb) = &self.embedded else {
            return true;
        };
        match &emb.ui_preview {
            Some(UiPreview::Splashes(..)) => return true,
            Some(UiPreview::Screen(_)) => return emb.ui_settle > 0.0,
            None => {}
        }
        if emb.awaiting.is_some() {
            return true;
        }
        match (&self.phase, emb.status) {
            (Phase::Running(shell), PlayStatus::Playing) => shell.is_animating(),
            _ => false,
        }
    }

    /// `true` once after something changed that the picture must show (an
    /// input, a story swap, a state change); the host then draws one frame.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.emb_mut().dirty)
    }

    /// Draws the game into `target` (a render target of the renderer, 16:9).
    pub fn render_to_target(&mut self, target: TextureId) -> Result<(), RenderError> {
        let Some([_, height]) = self.ctx.renderer.texture_size(target) else {
            return Err(RenderError::UnknownTexture(target));
        };
        self.ctx
            .renderer
            .set_pixel_ratio_override(Some(height as f32 / CANVAS[1]));
        let result = self.draw(&mut Output::Target {
            id: target,
            drawn: false,
        });
        self.ctx.renderer.set_pixel_ratio_override(None);
        self.emb_mut().dirty = false;
        result
    }

    // -------------------------------------------- scripted driving (MCP)

    /// One logical step forward, as the player would take it: on a line the
    /// typewriter completes and then the next step runs; a transition or a
    /// timed pause is cut short. `false` when nothing could advance (stopped,
    /// at a choice, at the end, after a failure).
    pub fn advance_one(&mut self) -> bool {
        if self.emb().status == PlayStatus::Stopped {
            return false;
        }
        let progress = |rt: &Self| match &rt.phase {
            Phase::Running(shell) => shell.game().map(|g| g.director.progress()),
            _ => None,
        };
        let Some(before) = progress(self) else {
            return false;
        };
        match self.flow() {
            Some(Flow::Say(_)) => {
                // The first confirmation may only complete the typewriter.
                for _ in 0..2 {
                    self.on_action(InputAction::Ui(UiInput::Confirm));
                    if progress(self) != Some(before) {
                        break;
                    }
                }
            }
            Some(Flow::Transition { .. }) => self.on_action(InputAction::Ui(UiInput::Confirm)),
            Some(Flow::Waiting { remaining }) => {
                let seconds = *remaining + 0.01;
                if let Phase::Running(shell) = &mut self.phase {
                    shell.update(seconds);
                }
                self.after_shell();
            }
            _ => return false,
        }
        self.emb_mut().dirty = true;
        progress(self) != Some(before)
    }

    /// Picks option `index` of the choice on screen. `false` when none is
    /// shown or the index is out of range.
    pub fn choose_option(&mut self, index: usize) -> bool {
        let done = match &mut self.phase {
            Phase::Running(shell) => shell.choose_option(index),
            _ => false,
        };
        if done {
            self.after_shell();
            self.emb_mut().dirty = true;
        }
        done
    }

    /// What the preview shows, as plain data; `None` without a playthrough.
    pub fn snapshot(&self) -> Option<PreviewSnapshot> {
        let Phase::Running(shell) = &self.phase else {
            return None;
        };
        let game = shell.game()?;
        let vm = game.director.vm();
        Some(PreviewSnapshot {
            flow: game.director.flow().clone(),
            node: game.director.shown().map(orbit_vm::Interaction::node),
            variables: vm.variables().clone(),
            scene: vm.scene().clone(),
        })
    }

    /// Draws a still `stage` (background and characters: no game, no UI)
    /// into `target`, with the very same code as the game. Textures come from
    /// the runtime's asset store, paths from `story` (the host passes the
    /// project's current story): an image the store does not have yet is
    /// simply not drawn, see [`Runtime::assets_epoch`].
    pub fn render_stage_to_target(
        &mut self,
        story: &Story,
        stage: &StageState,
        target: TextureId,
    ) -> Result<(), RenderError> {
        let mut scene = Scene::new();
        stage_scene(&self.ctx.renderer, &self.store, story, stage, &mut scene);
        self.ctx.renderer.render_to_target(target, &scene)
    }

    /// Size in pixels of the image of `asset`, when the store has its texture.
    pub fn image_size(&self, story: &Story, asset: orbit_core::AssetId) -> Option<[u32; 2]> {
        let path = &story.assets.get(asset)?.path;
        let texture = self.store.texture(path)?;
        self.ctx.renderer.texture_size(texture)
    }

    /// Changes whenever the story or an image file was swapped in the
    /// runtime: a still stage drawn before is out of date.
    pub fn assets_epoch(&self) -> u64 {
        self.emb().epoch
    }

    // ------------------------------------------------------- observation

    /// The node of the line or choice on screen.
    pub fn current_node(&self) -> Option<NodeId> {
        match &self.phase {
            Phase::Running(shell) => shell.shown_node(),
            _ => None,
        }
    }

    /// What the story is doing, while a playthrough exists.
    pub fn flow(&self) -> Option<&Flow> {
        match &self.phase {
            Phase::Running(shell) => shell.game().map(|g| g.director.flow()),
            _ => None,
        }
    }

    /// Messages for the author since the last call.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.emb_mut().notices)
    }
}

// --------------------------------------------------------------- UI preview

impl Runtime {
    /// Shows a screen of `bundle` (or its splashes) instead of the game, as
    /// the player would see it (with sample game data: a save to continue
    /// from, saving allowed...). Replaces any earlier UI preview; the story
    /// playthrough is left untouched. `language` is the table the
    /// interface keys are read from.
    pub fn set_ui_preview(
        &mut self,
        bundle: &orbit_ui::UiBundle,
        target: UiPreviewTarget<'_>,
        language: &str,
    ) -> Result<(), String> {
        let preview = match target {
            UiPreviewTarget::Screen(id) => {
                let theme = bundle.theme.clone().unwrap_or_else(|| self.ctx.theme.clone());
                let texts = orbit_ui::Texts::builtin(language);
                // The editor re-sends the bundle after every edit, and the
                // screen the preview navigated to after a click: the stack
                // is kept while the screen asked for is the one shown.
                if let Some(UiPreview::Screen(p)) =
                    self.embedded.as_mut().and_then(|e| e.ui_preview.as_mut())
                    && p.nav.top() == id
                {
                    p.update_bundle(bundle, theme, texts);
                    let emb = self.emb_mut();
                    emb.ui_settle = UI_SETTLE;
                    emb.dirty = true;
                    return Ok(());
                }
                UiPreview::Screen(Box::new(ScreenPreview::new(bundle, id, theme, texts)?))
            }
            UiPreviewTarget::Splashes => {
                let state = SplashState::new(&bundle.splashes);
                self.splash_assets.request(&self.assets_loader, state.steps());
                UiPreview::Splashes(state, bundle.splashes.clone())
            }
        };
        let emb = self.emb_mut();
        emb.ui_preview = Some(preview);
        emb.ui_settle = UI_SETTLE;
        emb.dirty = true;
        Ok(())
    }

    /// Id of the screen the UI preview shows (it follows the navigation).
    pub fn ui_preview_shown(&self) -> Option<&str> {
        match self.embedded.as_ref()?.ui_preview.as_ref()? {
            UiPreview::Screen(p) => Some(p.nav.top()),
            UiPreview::Splashes(..) => None,
        }
    }

    /// The widget (screen id, path in its tree) the pointer last went down
    /// on in the UI preview; once.
    pub fn take_ui_pick(&mut self) -> Option<(String, Vec<usize>)> {
        match self.embedded.as_mut()?.ui_preview.as_mut()? {
            UiPreview::Screen(p) => p.pick.take(),
            UiPreview::Splashes(..) => None,
        }
    }

    /// Outlines the widget at `path` of the screen shown (`None`: no outline).
    pub fn set_ui_highlight(&mut self, path: Option<Vec<usize>>) {
        let emb = self.emb_mut();
        if let Some(UiPreview::Screen(p)) = emb.ui_preview.as_mut()
            && p.highlight != path
        {
            p.highlight = path;
            emb.dirty = true;
        }
    }

    /// Back to the game.
    pub fn clear_ui_preview(&mut self) {
        let emb = self.emb_mut();
        if emb.ui_preview.take().is_some() {
            emb.dirty = true;
        }
    }

    pub fn has_ui_preview(&self) -> bool {
        self.embedded.as_ref().is_some_and(|e| e.ui_preview.is_some())
    }

    /// Advances a UI preview; `true` when there is one (the game is not
    /// ticked then).
    fn tick_ui_preview(&mut self, dt: Duration) -> bool {
        let secs = dt.as_secs_f32().min(MAX_DT);
        let Some(emb) = self.embedded.as_mut() else {
            return false;
        };
        let Some(preview) = emb.ui_preview.as_mut() else {
            return false;
        };
        match preview {
            UiPreview::Screen(p) => {
                use orbit_ui::Screen as _;
                if emb.ui_settle > 0.0 {
                    p.screen.update(secs);
                    p.toasts.update(secs);
                    emb.ui_settle -= secs;
                    emb.dirty = true;
                }
            }
            UiPreview::Splashes(state, defs) => {
                state.update(secs);
                if state.finished() {
                    *state = SplashState::new(defs);
                }
                emb.dirty = true;
            }
        }
        true
    }

    fn ui_preview_input(&mut self, action: InputAction) {
        let Some(emb) = self.embedded.as_mut() else {
            return;
        };
        match (emb.ui_preview.as_mut(), action) {
            (Some(UiPreview::Screen(p)), InputAction::Ui(input)) => {
                // Navigation actions are followed inside the preview, the
                // others flash a notice (see `ScreenPreview::handle`).
                p.handle(input);
                emb.ui_settle = if p.toasts.is_empty() { UI_SETTLE } else { TOAST_SETTLE };
                emb.dirty = true;
            }
            (
                Some(UiPreview::Splashes(state, _)),
                InputAction::Ui(UiInput::Confirm | UiInput::PointerDown(_) | UiInput::Cancel),
            ) => {
                state.skip();
                emb.dirty = true;
            }
            _ => {}
        }
    }

    /// Draws the UI preview into `out`.
    pub(super) fn draw_ui_preview(&mut self, out: &mut Output<'_>) -> Result<(), RenderError> {
        let Ctx {
            renderer,
            text_engine,
            text_renderer,
            white,
            theme,
        } = &mut self.ctx;
        let Some(preview) = self.embedded.as_mut().and_then(|e| e.ui_preview.as_mut()) else {
            return Ok(());
        };
        match preview {
            UiPreview::Splashes(state, _) => {
                let scene =
                    self.splash_assets
                        .scene(state, renderer, text_engine, text_renderer, true);
                out.scene(renderer, &scene)
            }
            UiPreview::Screen(p) => {
                use orbit_ui::Screen as _;
                let mut back = Scene::new();
                back.clear = Some(tint(theme.screens.background));
                out.scene(renderer, &back)?;
                let mut ui_scene = Scene::new();
                {
                    let mut ctx = DrawCtx {
                        engine: text_engine,
                        text_renderer,
                        renderer,
                        scene: &mut ui_scene,
                        white: *white,
                    };
                    p.screen.draw(&mut ctx);
                    if let Some(path) = p.highlight.clone() {
                        p.screen.draw_outline(&mut ctx, &path);
                    }
                    p.toasts.draw(&mut ctx);
                }
                out.scene(renderer, &ui_scene)
            }
        }
    }
}
