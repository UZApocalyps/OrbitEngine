//! `EditorCore`: the editor without a window or a GPU.
//!
//! It owns the plugin registry, the editor state, the open project session,
//! the dock layout, the palette and the dialogs, and draws one frame of UI
//! into an `egui::Ui` (see `ui.rs`). Because nothing here touches wgpu or
//! winit, the whole editor logic (open / edit / undo / save / recovery /
//! layout persistence) is tested headlessly.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::Context;
use egui_dock::DockState;
use orbit_core::Story;
use orbit_editor_api::{
    EditorContext, EditorPlugin, EditorRegistry, EditorRequest, EditorState, Level, ProjectInfo,
    Selection, ThemeMode,
};
use orbit_plugin_api::ProjectEvent;
use orbit_project::{ChangeKind, load_story};

use crate::builtin;
use crate::config::{ConfigStore, UserConfig};
use crate::diagnostics::{Extras, Live};
use crate::dock;
use crate::palette::Palette;
use crate::session::{AUTOSAVE_INTERVAL, Session};
use crate::storage::{ProjectLocation, RecoveryMeta, create_project};
use crate::theme::{Theme, install_fonts};
use crate::watch::{ProjectWatcher, WatchEvent};

/// How often the project's files are looked at for outside changes.
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

/// Startup options of the editor core.
#[derive(Clone, Debug)]
pub struct CoreOptions {
    /// User config folder; `None` keeps everything in memory (tests).
    pub config_dir: Option<PathBuf>,
    pub autosave_interval: Duration,
    /// How often the project's files are looked at for outside changes.
    pub watch_interval: Duration,
    /// Forces a theme instead of the saved one.
    pub theme: Option<ThemeMode>,
    /// Opens the command palette on the first frame (debug / screenshots).
    pub open_palette: bool,
    /// Text typed in the palette when opened with `open_palette`.
    pub palette_query: String,
}

impl Default for CoreOptions {
    fn default() -> Self {
        Self {
            config_dir: None,
            autosave_interval: AUTOSAVE_INTERVAL,
            watch_interval: WATCH_INTERVAL,
            theme: None,
            open_palette: false,
            palette_query: String::new(),
        }
    }
}

/// An action waiting for the user's answer about unsaved changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    Quit,
    Open(PathBuf),
    Close,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewProjectForm {
    pub name: String,
    /// Folder that will contain the project folder.
    pub parent: String,
    pub error: Option<String>,
    /// The name field took the focus when the dialog opened.
    pub focused: bool,
}

/// A modal question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// Unsaved changes stand in the way of `then`.
    ConfirmUnsaved {
        then: Pending,
    },
    /// The previous session left a snapshot of unsaved work.
    Recovery {
        meta: RecoveryMeta,
    },
    NewProject(NewProjectForm),
    /// The story file changed on disk (another tool, another editor).
    /// `conflict`: the open project also has unsaved edits.
    ExternalChange {
        conflict: bool,
    },
}

pub struct EditorCore {
    pub registry: EditorRegistry,
    pub state: EditorState,
    pub session: Option<Session>,
    pub dock: DockState<String>,
    pub palette: Palette,
    pub dialog: Option<Dialog>,
    pub theme: Theme,
    pub options: CoreOptions,
    store: ConfigStore,
    saved_layout: String,
    quit: bool,
    first_frame_done: bool,
    /// Frame counter overlay (debug).
    pub show_frame_counter: bool,
    /// Bumped each time a project is opened or closed (the preview reloads
    /// from scratch instead of resuming).
    pub project_generation: u64,
    /// Live validation (inline, or debounced on a worker for big stories).
    live: Live,
    /// State shared with the built-in plugins (search index, file moves).
    pub shared: builtin::Shared,
    /// Watches the story file and the asset folder of the open project.
    watcher: Option<ProjectWatcher>,
    /// Folder the manifest's relative paths resolve against.
    assets_root: PathBuf,
    /// The story file changed on disk and no dialog could say so yet.
    story_changed_on_disk: bool,
    /// Bumped when files of the asset folders changed (the preview reloads).
    pub asset_epoch: u64,
}

impl EditorCore {
    /// An editor with the built-in plugins.
    pub fn new(options: CoreOptions, ctx: &Context) -> Self {
        Self::with_plugins(options, ctx, Vec::new())
    }

    /// An editor with the built-in plugins plus `extra` ones.
    pub fn with_plugins(
        options: CoreOptions,
        ctx: &Context,
        extra: Vec<Box<dyn EditorPlugin>>,
    ) -> Self {
        let mut registry = EditorRegistry::new();
        let shared = builtin::Shared::default();
        for plugin in builtin::plugins_with(&shared).iter().chain(extra.iter()) {
            if let Err(e) = registry.install(plugin.as_ref()) {
                registry.errors.push(e.to_string());
            }
        }
        // One "Afficher : <panel>" action per panel, once all panels are known.
        let view = builtin::PanelActions::for_registry(&registry);
        if let Err(e) = registry.install(&view) {
            registry.errors.push(e.to_string());
        }

        let store = ConfigStore::new(options.config_dir.clone());
        let config = store.load();
        let mode = options.theme.unwrap_or(config.theme);
        let theme = Theme::for_mode(mode);
        install_fonts(ctx);
        theme.apply(ctx);

        let mut dock = config
            .dock
            .unwrap_or_else(|| dock::default_layout(&registry));
        dock::sanitize(&mut dock, &registry);

        let mut state = EditorState {
            theme: mode,
            recent: config.recent,
            plugin_prefs: config.plugin_prefs,
            ..EditorState::default()
        };
        for error in &registry.errors {
            state
                .console
                .push(Level::Error, format!("plugin : {error}"));
        }
        let saved_layout = ron::to_string(&dock).unwrap_or_default();
        let live = Live::new(state.waker.clone());
        Self {
            registry,
            state,
            session: None,
            dock,
            palette: Palette::default(),
            dialog: None,
            theme,
            options,
            store,
            saved_layout,
            quit: false,
            first_frame_done: false,
            show_frame_counter: false,
            project_generation: 0,
            live,
            shared,
            watcher: None,
            assets_root: PathBuf::new(),
            story_changed_on_disk: false,
            asset_epoch: 0,
        }
    }

    /// Lets background threads wake the window loop (see `orbit_platform::Waker`).
    pub fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.state.waker.set(wake);
    }

    // ------------------------------------------------------------ projects

    pub fn has_project(&self) -> bool {
        self.session.is_some()
    }

    pub fn story(&self) -> Option<&Story> {
        self.session.as_ref().map(|s| s.project.story())
    }

    /// Selects the `n`-th node of the open story, in reading order (debug
    /// option `--select-node`, tests).
    pub fn select_nth_node(&mut self, n: usize) {
        let id = self
            .story()
            .and_then(|s| s.scenes.iter().flat_map(|sc| &sc.nodes).nth(n))
            .map(|node| node.id);
        if let Some(id) = id {
            self.state.selection = Some(Selection::Node(id));
        }
    }

    /// Selects the `n`-th asset of the manifest in the asset library (debug
    /// option `--select-asset`, tests).
    pub fn select_nth_asset(&mut self, n: usize) {
        let id = self
            .story()
            .and_then(|s| s.assets.assets.get(n))
            .map(|a| a.id);
        if id.is_some() {
            self.state.selected_asset = id;
        }
    }

    /// Replaces the text of the `n`-th line (`Say` node, reading order) of
    /// the open story, as a typed edit would (debug option `--edit-text`).
    pub fn edit_nth_say(&mut self, n: usize, text: &str) -> Result<(), String> {
        let session = self.session.as_mut().ok_or("aucun projet ouvert")?;
        let id = session
            .project
            .story()
            .scenes
            .iter()
            .flat_map(|sc| &sc.nodes)
            .filter_map(|node| match &node.kind {
                orbit_core::NodeKind::Say { text_id, .. } => Some(*text_id),
                _ => None,
            })
            .nth(n)
            .ok_or_else(|| format!("pas de réplique n°{n}"))?;
        session
            .project
            .execute(orbit_project::Command::SetString {
                id,
                text: Some(text.to_owned()),
            })
            .map_err(|e| e.to_string())
    }

    /// Whether the open project has unsaved edits.
    pub fn is_modified(&mut self) -> bool {
        self.session.as_mut().is_some_and(Session::modified)
    }

    /// Opens a project, asking first when the current one has unsaved edits.
    pub fn open_project(&mut self, path: &Path) {
        if self.is_modified() {
            self.dialog = Some(Dialog::ConfirmUnsaved {
                then: Pending::Open(path.to_owned()),
            });
        } else {
            self.open_now(path);
        }
    }

    fn open_now(&mut self, path: &Path) {
        let location = match ProjectLocation::resolve(path) {
            Ok(location) => location,
            Err(e) => {
                self.state.push_toast(Level::Error, e);
                self.forget(path);
                return;
            }
        };
        let story = match load_story(&location.storage(), &location.story_file) {
            Ok(story) => story,
            Err(e) => {
                self.state.push_toast(Level::Error, e.to_string());
                self.forget(path);
                return;
            }
        };
        self.close_now();
        let (layout, layout_warning) =
            orbit_project::graph_layout::load_layout(&location.storage(), &location.story_file);

        let recovery_dir = location.recovery_dir(self.store.dir());
        let remembered = match &location.manifest {
            Some(_) => location.root.clone(),
            None => location.story_path(),
        };
        self.project_generation += 1;
        let session = Session::open(location, story, recovery_dir);
        let pending = session.pending_recovery();
        let title = session.name.clone();
        self.session = Some(session);
        self.state.selection = None;
        self.state.node_selection.clear();
        self.state.current_scene = None;
        self.state.clipboard = None;
        self.state.collapsed_scenes.clear();
        self.state.graph = orbit_editor_api::GraphState {
            persist: !layout.is_empty(),
            layout,
            dirty: false,
            generation: self.project_generation,
            ..Default::default()
        };
        if let Some(warning) = layout_warning {
            self.state.console.push(Level::Warn, warning);
        }
        self.state.selected_asset = None;
        self.state.pending_reveal = None;
        self.reset_project_services();
        dock::show_panel(&mut self.dock, builtin::panels::SCENE);
        self.sync_info();
        self.remember(&title, &remembered);
        self.registry
            .project
            .dispatch(&ProjectEvent::Opened { title: &title });
        self.report_opened();
        if let Some(meta) = pending {
            self.dialog = Some(Dialog::Recovery { meta });
        }
    }

    /// Console summary: counts, validation problems, missing plugins.
    fn report_opened(&mut self) {
        let Some(story) = self.story() else { return };
        let nodes: usize = story.scenes.iter().map(|s| s.nodes.len()).sum();
        let mut lines = vec![(
            Level::Info,
            format!(
                "Projet « {} » ouvert : {} scène(s), {nodes} nœud(s).",
                story.title,
                story.scenes.len()
            ),
        )];
        let report = orbit_core::validate(story);
        for e in report.errors.iter().take(20) {
            lines.push((Level::Error, e.to_string()));
        }
        for w in report.warnings.iter().take(20) {
            lines.push((Level::Warn, w.to_string()));
        }
        for plugin in self.registry.project.missing_plugins(story) {
            lines.push((
                Level::Warn,
                format!("Plugin absent : `{plugin}` ; ses nœuds sont conservés en lecture seule."),
            ));
        }
        for (level, message) in lines {
            self.state.console.push(level, message);
        }
    }

    /// Saves the open project (atomically). `true` on success.
    pub fn save(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        self.registry.project.dispatch(&ProjectEvent::BeforeSave);
        match session.save() {
            Ok(()) => {
                if let Some(watcher) = &self.watcher {
                    watcher.accept_story_now();
                }
                let name = session.name.clone();
                self.state
                    .push_toast(Level::Info, format!("« {name} » enregistré."));
                true
            }
            Err(e) => {
                self.state
                    .push_toast(Level::Error, format!("Échec de l'enregistrement : {e}"));
                false
            }
        }
    }

    /// Closes the project, asking first when it has unsaved edits.
    pub fn close_project(&mut self) {
        if self.is_modified() {
            self.dialog = Some(Dialog::ConfirmUnsaved {
                then: Pending::Close,
            });
        } else {
            self.close_now();
        }
    }

    fn close_now(&mut self) {
        self.flush_layout();
        self.project_generation += 1;
        self.state.graph = orbit_editor_api::GraphState {
            generation: self.project_generation,
            ..Default::default()
        };
        self.watcher = None;
        self.live.reset();
        self.shared.search.0.borrow_mut().clear();
        if let Some(session) = self.session.take() {
            self.registry.project.dispatch(&ProjectEvent::Closed);
            session.close();
        }
        self.state.selection = None;
        self.sync_info();
    }

    /// Creates a project folder from the minimal template and opens it.
    pub fn create_and_open(&mut self, parent: &Path, name: &str) -> Result<(), String> {
        let root = create_project(parent, name)?;
        self.open_project(&root);
        Ok(())
    }

    fn remember(&mut self, name: &str, path: &Path) {
        let mut config = self.user_config();
        config.remember_project(name, path);
        self.state.recent = config.recent.clone();
        self.store.save(&config);
    }

    fn forget(&mut self, path: &Path) {
        let mut config = self.user_config();
        config.forget_project(path);
        self.state.recent = config.recent.clone();
        self.store.save(&config);
    }

    // ------------------------------------------------------------ recovery

    /// Applies the user's choice about a recovery snapshot.
    pub fn resolve_recovery(&mut self, restore: bool) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if restore {
            match session.restore_recovery() {
                Ok(()) => self.state.push_toast(
                    Level::Info,
                    "Version non enregistrée restaurée ; enregistrez pour la conserver.",
                ),
                Err(e) => self
                    .state
                    .push_toast(Level::Error, format!("Restauration impossible : {e}")),
            }
        } else {
            session.discard_recovery();
        }
    }

    // ---------------------------------------------------------------- quit

    /// The window asked to close. `true` when the editor may exit now;
    /// otherwise a confirmation dialog is shown.
    pub fn request_close(&mut self) -> bool {
        if self.is_modified() {
            self.dialog = Some(Dialog::ConfirmUnsaved {
                then: Pending::Quit,
            });
            false
        } else {
            self.quit = true;
            true
        }
    }

    /// An action asked to quit and the user confirmed.
    pub fn take_quit(&mut self) -> bool {
        std::mem::take(&mut self.quit)
    }

    /// Clean shutdown: saves the config, ends the session (a modified
    /// project keeps its recovery snapshot).
    pub fn shutdown(&mut self) {
        self.flush_layout();
        self.persist();
        if let Some(session) = self.session.take() {
            session.close();
        }
    }

    /// The user answered a [`Dialog::ConfirmUnsaved`].
    pub fn resolve_unsaved(&mut self, then: Pending, choice: UnsavedChoice) {
        match choice {
            UnsavedChoice::Cancel => {}
            UnsavedChoice::Save => {
                if self.save() {
                    self.run_pending(then);
                }
            }
            UnsavedChoice::Discard => {
                if let Some(session) = &self.session {
                    session.discard_recovery();
                }
                self.run_pending(then);
            }
        }
    }

    fn run_pending(&mut self, then: Pending) {
        match then {
            Pending::Quit => self.quit = true,
            Pending::Open(path) => {
                // The old session is replaced: whatever it held was saved or
                // discarded on purpose.
                if let Some(session) = &self.session {
                    session.discard_recovery();
                }
                self.open_now(&path);
            }
            Pending::Close => {
                if let Some(session) = &self.session {
                    session.discard_recovery();
                }
                self.close_now();
            }
        }
    }

    // ------------------------------------------------------- requests, frame

    /// Executes what plugins and shortcuts asked for.
    pub fn pump(&mut self, ctx: &Context) {
        for _ in 0..16 {
            let requests = std::mem::take(&mut self.state.requests);
            if requests.is_empty() {
                break;
            }
            for request in requests {
                self.handle_request(request, ctx);
            }
        }
    }

    fn handle_request(&mut self, request: EditorRequest, ctx: &Context) {
        match request {
            EditorRequest::RunAction(id) => self.run_action(&id),
            EditorRequest::OpenProject(path) => self.open_project(&path),
            EditorRequest::PickOpenProject => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Ouvrir un projet")
                    .pick_folder()
                {
                    self.open_project(&path);
                }
            }
            EditorRequest::PickOpenStory => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Ouvrir une histoire")
                    .add_filter("Histoire OrbitEngine", &["ron"])
                    .pick_file()
                {
                    self.open_project(&path);
                }
            }
            EditorRequest::NewProjectDialog => {
                let parent = std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                self.dialog = Some(Dialog::NewProject(NewProjectForm {
                    parent,
                    ..NewProjectForm::default()
                }));
            }
            EditorRequest::Save => {
                self.save();
            }
            EditorRequest::CloseProject => self.close_project(),
            EditorRequest::Quit => {
                self.request_close();
            }
            EditorRequest::OpenPalette => self.palette.toggle(),
            EditorRequest::ResetLayout => {
                self.dock = dock::default_layout(&self.registry);
                self.persist();
            }
            EditorRequest::ShowPanel(id) => {
                self.state.maximized_panel = None;
                dock::show_panel(&mut self.dock, &id);
            }
            EditorRequest::ToggleMaximizePanel(id) => {
                self.state.maximized_panel = if self.state.maximized_panel.as_deref() == Some(&id) {
                    None
                } else {
                    Some(id)
                };
            }
            EditorRequest::SetTheme(mode) => self.set_theme(mode, ctx),
        }
    }

    fn run_action(&mut self, id: &str) {
        let Some(action) = self.registry.find_action(id) else {
            self.state
                .push_toast(Level::Error, format!("Action inconnue : {id}"));
            return;
        };
        let mut ctx = EditorContext::new(
            self.session.as_mut().map(|s| &mut s.project),
            &mut self.state,
            &self.registry,
        );
        if (action.def.enabled)(&ctx) {
            (action.def.run)(&mut ctx);
        }
    }

    /// Runs an action by full id right now (tests, scripts).
    pub fn run_action_now(&mut self, id: &str, ctx: &Context) {
        self.state
            .requests
            .push(EditorRequest::RunAction(id.to_owned()));
        self.pump(ctx);
    }

    pub fn set_theme(&mut self, mode: ThemeMode, ctx: &Context) {
        self.state.theme = mode;
        self.theme = Theme::for_mode(mode);
        self.theme.apply(ctx);
        self.persist();
    }

    /// Runs once on the first frame.
    pub(crate) fn first_frame(&mut self) {
        if self.first_frame_done {
            return;
        }
        self.first_frame_done = true;
        if self.options.open_palette {
            self.palette.open();
            self.palette.query = self.options.palette_query.clone();
        }
    }

    /// Bookkeeping after the UI of a frame: change events, project info,
    /// autosave, layout persistence.
    pub fn after_frame(&mut self, ctx: &Context) {
        self.process_changes();
        self.poll_watcher(ctx);
        if self.refresh_diagnostics() {
            // Findings arrived after the UI of this frame was drawn.
            ctx.request_repaint();
        }
        self.sync_info();
        if let Some(session) = self.session.as_mut() {
            let now = Instant::now();
            if session.autosave_due(now, self.options.autosave_interval)
                && let Err(e) = session.autosave(now)
            {
                log::warn!("autosave failed: {e}");
                self.state
                    .console
                    .push(Level::Warn, format!("Autosave impossible : {e}"));
            }
        }
        // Graph positions: written once a drag has ended.
        if self.state.graph.dirty && !ctx.input(|i| i.pointer.any_down()) {
            self.flush_layout();
        }
        // Layout: written once a drag has ended.
        if !ctx.input(|i| i.pointer.any_down()) {
            let layout = ron::to_string(&self.dock).unwrap_or_default();
            if layout != self.saved_layout {
                self.saved_layout = layout;
                self.persist();
            }
        }
    }

    /// Writes the graph positions (`.layout` sidecar) when they changed.
    pub fn flush_layout(&mut self) {
        if !self.state.graph.dirty {
            return;
        }
        self.state.graph.dirty = false;
        let Some(session) = self.session.as_ref() else {
            return;
        };
        if let Err(e) = orbit_project::graph_layout::save_layout(
            &session.location.storage(),
            &session.location.story_file,
            &self.state.graph.layout,
        ) {
            self.state.console.push(
                Level::Warn,
                format!("Positions du graphe non enregistrées : {e}"),
            );
        }
    }

    /// Keeps the validation findings up to date: inline for small stories,
    /// debounced on a worker thread for big ones. `true` when new findings
    /// were published.
    pub fn refresh_diagnostics(&mut self) -> bool {
        let Some(session) = self.session.as_ref() else {
            if self.state.diagnostics.revision != 0 || !self.state.diagnostics.problems.is_empty() {
                self.state.diagnostics = Default::default();
            }
            return false;
        };
        let key = (self.project_generation, session.project.revision());
        let registry = &self.registry.project;
        let published = self.live.update(
            Instant::now(),
            key,
            session.project.story(),
            &self.assets_root,
            |story| Extras {
                custom: registry.validate_custom(story),
                missing_plugins: registry.missing_plugin_nodes(story),
                asset_issues: Vec::new(),
            },
        );
        match published {
            Some(diagnostics) => {
                if let Some(ms) = self.live.last_run.filter(|d| *d > Duration::from_millis(8)) {
                    log::debug!(
                        target: "orbit_editor::perf",
                        "validation: {} ms",
                        ms.as_millis()
                    );
                }
                self.state.diagnostics = diagnostics;
                true
            }
            None => false,
        }
    }

    /// Validation timings of the last run (tests, perf logs): `(run, edit to
    /// publication)`.
    pub fn validation_timings(&self) -> (Option<Duration>, Option<Duration>) {
        (self.live.last_run, self.live.last_latency)
    }

    /// Starts what depends on the open project: file watching, live
    /// validation, the search index.
    fn reset_project_services(&mut self) {
        self.live.reset();
        self.shared.search.0.borrow_mut().clear();
        self.shared.moves.borrow_mut().0.clear();
        self.story_changed_on_disk = false;
        let Some(session) = &self.session else {
            self.watcher = None;
            return;
        };
        self.assets_root =
            crate::preview_host::find_assets_root(&session.location.root, session.project.story());
        self.watcher = Some(ProjectWatcher::start(
            self.assets_root.clone(),
            session.location.story_path(),
            self.options.watch_interval,
            self.state.waker.clone(),
        ));
    }

    // ----------------------------------------------------- files on disk

    /// Delivers what the file watcher noticed.
    fn poll_watcher(&mut self, ctx: &Context) {
        let events = self
            .watcher
            .as_ref()
            .map(ProjectWatcher::poll)
            .unwrap_or_default();
        for event in events {
            match event {
                WatchEvent::StoryChanged => self.story_changed_on_disk = true,
                WatchEvent::Assets {
                    added,
                    modified,
                    removed,
                } => self.on_assets_changed(ctx, &added, &modified, &removed),
            }
        }
        if self.story_changed_on_disk && self.dialog.is_none() {
            self.story_changed_on_disk = false;
            self.check_external_story();
            ctx.request_repaint();
        }
    }

    fn on_assets_changed(
        &mut self,
        ctx: &Context,
        added: &[String],
        modified: &[String],
        removed: &[String],
    ) {
        self.state.asset_epoch += 1;
        self.asset_epoch += 1;
        self.live.mark_assets_dirty();
        let changed: Vec<String> = added
            .iter()
            .chain(modified)
            .chain(removed)
            .cloned()
            .collect();
        builtin::thumbs::invalidate(ctx, &self.state.waker, &changed);
        for (verb, list) in [
            ("ajouté", added),
            ("modifié", modified),
            ("supprimé", removed),
        ] {
            for path in list.iter().take(5) {
                self.state.console.push(
                    Level::Info,
                    format!("Fichier {verb} sur le disque : {path}"),
                );
            }
            if list.len() > 5 {
                self.state.console.push(
                    Level::Info,
                    format!("… et {} autre(s) fichier(s) {verb}(s)", list.len() - 5),
                );
            }
        }
        ctx.request_repaint();
    }

    /// The story file changed: nothing to do when it holds what the editor
    /// holds, otherwise ask whether to reload.
    fn check_external_story(&mut self) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        match load_story(&session.location.storage(), &session.location.story_file) {
            Ok(story) => {
                if &story == session.saved_story() {
                    return;
                }
                let conflict = session.modified();
                self.dialog = Some(Dialog::ExternalChange { conflict });
            }
            Err(e) => self.state.console.push(
                Level::Warn,
                format!(
                    "Le fichier de l'histoire a changé sur le disque mais n'est pas lisible : {e}"
                ),
            ),
        }
    }

    /// The user answered the "file changed on disk" question.
    pub fn resolve_external_change(&mut self, reload: bool) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if !reload {
            if let Some(watcher) = &self.watcher {
                watcher.accept_story_now();
            }
            self.state.push_toast(
                Level::Info,
                "Fichier modifié ignoré : l'enregistrement le remplacera.",
            );
            return;
        }
        match session.reload_from_disk() {
            Ok(()) => {
                self.project_generation += 1;
                self.reset_project_services();
                if let Some(watcher) = &self.watcher {
                    watcher.accept_story_now();
                }
                self.state.selection = None;
                self.state.node_selection.clear();
                self.state.current_scene = None;
                self.state.clipboard = None;
                self.sync_info();
                let title = self.session.as_ref().map(|s| s.name.clone());
                if let Some(title) = title {
                    self.registry
                        .project
                        .dispatch(&ProjectEvent::Opened { title: &title });
                }
                self.state
                    .push_toast(Level::Info, "Histoire rechargée depuis le disque.");
            }
            Err(e) => self
                .state
                .push_toast(Level::Error, format!("Rechargement impossible : {e}")),
        }
    }

    /// Files dropped on the window: imported as assets of the open project
    /// (images, sounds, folders of them); with no project open, a dropped
    /// project folder or story file opens it.
    pub fn import_dropped(&mut self, paths: Vec<PathBuf>) {
        if self.dialog.is_some() {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            match paths.as_slice() {
                [one] if one.is_dir() || one.extension().is_some_and(|e| e == "ron") => {
                    self.open_project(one);
                }
                _ => self.state.push_toast(
                    Level::Warn,
                    "Ouvrez un projet avant de déposer des ressources.",
                ),
            }
            return;
        };
        let mut ectx =
            EditorContext::new(Some(&mut session.project), &mut self.state, &self.registry);
        let report = builtin::import_paths(&mut ectx, &paths);
        if !report.imported.is_empty() {
            self.state.requests.push(EditorRequest::ShowPanel(
                builtin::panels::LIBRARY.to_owned(),
            ));
        }
    }

    /// Opens the global search panel with `query` typed in (debug option
    /// `--search`, tests).
    pub fn open_search(&mut self, query: &str) {
        self.state
            .plugin_prefs
            .entry(builtin::ids::SEARCH.to_owned())
            .or_default()
            .insert("initial_query".to_owned(), query.to_owned());
        self.state.focus_panel = Some(builtin::panels::SEARCH.to_owned());
        self.state
            .requests
            .push(EditorRequest::ShowPanel(builtin::panels::SEARCH.to_owned()));
    }

    /// Takes the files dropped on the window this frame, if any.
    pub fn handle_dropped_files(&mut self, ctx: &Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            self.import_dropped(dropped);
        }
    }

    /// Delivers project change events: plugin hooks, selection upkeep.
    fn process_changes(&mut self) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        for event in session.drain_changes() {
            self.registry
                .project
                .dispatch(&ProjectEvent::Changed(&event));
            let story = session.project.story();
            self.state
                .node_selection
                .retain(|id| story.locate(*id).is_some());
            match self.state.selection {
                Some(Selection::Node(id)) if story.locate(id).is_none() => {
                    self.state.selection = None;
                }
                Some(Selection::Scene(id)) if story.scene(id).is_none() => {
                    self.state.selection = None;
                }
                _ => {}
            }
            // After undo / redo, files moved by an asset rename follow the manifest.
            if matches!(event.kind, ChangeKind::Undone | ChangeKind::Redone) {
                let moved = crate::assets::reconcile_moves(
                    &self.assets_root,
                    story,
                    &self.shared.moves.borrow(),
                );
                if !moved.is_empty() {
                    self.state.asset_epoch += 1;
                    self.asset_epoch += 1;
                    self.live.mark_assets_dirty();
                }
            }
            // After undo / redo the selection follows the node that changed.
            if matches!(event.kind, ChangeKind::Undone | ChangeKind::Redone)
                && let Some(id) = event.touched.iter().find(|id| story.locate(**id).is_some())
            {
                self.state.selection = Some(Selection::Node(*id));
            }
        }
    }

    fn sync_info(&mut self) {
        self.state.project = self.session.as_mut().map(|s| ProjectInfo {
            name: s.name.clone(),
            root: s.location.root.clone(),
            story_file: s.location.story_file.clone(),
            modified: s.modified(),
            has_manifest: s.location.manifest.is_some(),
        });
    }

    /// When the loop must wake up without any event: toasts expiring, a
    /// pending autosave. `None` means sleep until the next event.
    pub fn next_wake(&mut self) -> Option<Instant> {
        let toasts = self.state.expire_toasts(Instant::now());
        let autosave = self
            .session
            .as_mut()
            .and_then(|s| s.autosave_wake(self.options.autosave_interval));
        let validation = self.live.next_wake();
        [toasts, autosave, validation].into_iter().flatten().min()
    }

    /// Title of the window ("● Mon jeu — OrbitEngine").
    pub fn window_title(&self) -> String {
        match &self.state.project {
            Some(p) if p.modified => format!("● {} — OrbitEngine", p.name),
            Some(p) => format!("{} — OrbitEngine", p.name),
            None => "OrbitEngine".to_owned(),
        }
    }

    // --------------------------------------------------------- persistence

    fn user_config(&self) -> UserConfig {
        UserConfig {
            theme: self.state.theme,
            dock: Some(self.dock.clone()),
            recent: self.state.recent.clone(),
            plugin_prefs: self.state.plugin_prefs.clone(),
        }
    }

    /// Writes the user config (layout, theme, recent projects, preferences).
    pub fn persist(&mut self) {
        self.store.save(&self.user_config());
    }
}

/// Answer to the "unsaved changes" question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}
