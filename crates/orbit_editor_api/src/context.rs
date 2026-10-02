//! The editor context handed to plugins, and the editor state behind it.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use orbit_core::{AssetId, CharacterId, NodeId, SceneId, Story};
use orbit_project::ops::NodeClip;
use orbit_project::{Command, Project, ProjectError, Transaction};
use serde::{Deserialize, Serialize};

use crate::preview::{PreviewCommand, PreviewState};
use crate::registry::EditorRegistry;

/// Light or dark UI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

/// A transient message shown over the editor.
#[derive(Clone, Debug)]
pub struct Toast {
    pub message: String,
    pub level: Level,
    pub expires: Instant,
}

/// A line of the console panel.
#[derive(Clone, Debug)]
pub struct ConsoleEntry {
    pub level: Level,
    pub message: String,
    /// Seconds since the editor started.
    pub time: f32,
}

/// Bounded journal shown by the console panel.
#[derive(Debug)]
pub struct ConsoleLog {
    started: Instant,
    entries: VecDeque<ConsoleEntry>,
    /// Bumped on every push, so panels can tell something new arrived.
    pub generation: u64,
}

impl ConsoleLog {
    const CAPACITY: usize = 1000;

    pub fn push(&mut self, level: Level, message: impl Into<String>) {
        if self.entries.len() == Self::CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(ConsoleEntry {
            level,
            message: message.into(),
            time: self.started.elapsed().as_secs_f32(),
        });
        self.generation += 1;
    }

    pub fn entries(&self) -> &VecDeque<ConsoleEntry> {
        &self.entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.generation += 1;
    }
}

impl Default for ConsoleLog {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            entries: VecDeque::new(),
            generation: 0,
        }
    }
}

/// A project in the "recent projects" list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProject {
    pub name: String,
    pub path: PathBuf,
}

/// What is selected in the editor (editor state, never undoable).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Node(NodeId),
    Scene(SceneId),
}

/// Facts about the open project, kept up to date by the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectInfo {
    pub name: String,
    pub root: PathBuf,
    pub story_file: String,
    /// The story has unsaved edits.
    pub modified: bool,
    /// The folder has an `orbit.project.ron` (false for a bare story file).
    pub has_manifest: bool,
}

/// Things a plugin cannot do itself and asks the host to do between frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorRequest {
    OpenProject(PathBuf),
    /// File dialog to pick a project folder.
    PickOpenProject,
    /// File dialog to pick a story `.ron` file.
    PickOpenStory,
    /// Opens the "new project" dialog.
    NewProjectDialog,
    Save,
    CloseProject,
    Quit,
    OpenPalette,
    ResetLayout,
    /// Brings a panel (full id) to the front, adding it to the layout if needed.
    ShowPanel(String),
    SetTheme(ThemeMode),
    /// Runs an action by full id.
    RunAction(String),
    /// Shows a panel (full id) alone, filling the window, or brings the
    /// layout back when it already is.
    ToggleMaximizePanel(String),
}

/// Something of the project the editor can show: a node, a scene, a
/// definition. The target of a validation problem or a search hit; see
/// [`EditorContext::reveal`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Location {
    Node(NodeId),
    Scene(SceneId),
    Character(CharacterId),
    Variable(String),
    Asset(AssetId),
}

/// One finding of the live validation, in French, with where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub level: Level,
    pub message: String,
    pub location: Option<Location>,
}

/// What validation found about nodes: errors and warnings, in French.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    /// Project revision these findings were computed for.
    pub revision: u64,
    pub by_node: std::collections::HashMap<NodeId, Vec<(Level, String)>>,
    /// Findings not tied to a node (definitions...).
    pub general: Vec<(Level, String)>,
    /// Every finding (nodes, definitions, asset files), errors first, in
    /// reading order: what the Problems panel lists.
    pub problems: Vec<Problem>,
}

impl Diagnostics {
    /// Number of errors.
    pub fn error_count(&self) -> usize {
        self.problems
            .iter()
            .filter(|p| p.level == Level::Error)
            .count()
    }

    /// Number of warnings.
    pub fn warning_count(&self) -> usize {
        self.problems
            .iter()
            .filter(|p| p.level == Level::Warn)
            .count()
    }

    pub fn of(&self, node: NodeId) -> &[(Level, String)] {
        self.by_node.get(&node).map_or(&[], Vec::as_slice)
    }

    /// The worst level among the findings of a node.
    pub fn worst(&self, node: NodeId) -> Option<Level> {
        let items = self.of(node);
        if items.iter().any(|(l, _)| *l == Level::Error) {
            Some(Level::Error)
        } else if items.is_empty() {
            None
        } else {
            Some(Level::Warn)
        }
    }
}

/// Lets background threads ask the host for a frame (the editor draws on
/// demand). Clones share one target, set by the host once the window runs;
/// until then (and in tests) waking does nothing.
#[derive(Clone, Default)]
pub struct Waker(Arc<Mutex<Option<WakeFn>>>);

type WakeFn = Arc<dyn Fn() + Send + Sync>;

impl Waker {
    /// Installs what [`Waker::wake`] does, for every clone.
    pub fn set(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(Arc::new(wake));
        }
    }

    pub fn wake(&self) {
        let wake = self.0.lock().ok().and_then(|slot| slot.clone());
        if let Some(wake) = wake {
            wake();
        }
    }
}

impl std::fmt::Debug for Waker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Waker")
    }
}

/// State owned by the editor, separate from the project: never undoable.
#[derive(Default)]
pub struct EditorState {
    /// Wakes the event loop from background threads (thumbnails, file
    /// watching, validation).
    pub waker: Waker,
    /// A thing to show that no shared panel can: the panel that owns it
    /// takes it (characters panel, asset library...). See `reveal`.
    pub pending_reveal: Option<Location>,
    /// Bumped by every `reveal`, so lists (explorer) scroll to the selection.
    pub reveal_serial: u64,
    /// The asset selected in the asset library.
    pub selected_asset: Option<AssetId>,
    /// Bumped by the host when files of the project's asset folders changed
    /// (watcher, import): lists of files look again.
    pub asset_epoch: u64,
    /// A panel (full id) whose text input should take the focus at its next
    /// frame (global search).
    pub focus_panel: Option<String>,
    pub selection: Option<Selection>,
    /// Every selected node (multi-selection); the primary one, if any, is in
    /// `selection`. Empty when no node is selected.
    pub node_selection: Vec<NodeId>,
    /// The scene shown by the scene editor.
    pub current_scene: Option<SceneId>,
    /// Internal clipboard of the scene editor (copy / paste of nodes).
    pub clipboard: Option<NodeClip>,
    /// A node whose text the scene editor should start editing in place.
    pub inline_edit: Option<NodeId>,
    /// Validation findings per node, kept up to date by the host.
    pub diagnostics: Diagnostics,
    /// Debug (`--autoscroll`): pixels the scene editor scrolls by each frame.
    pub debug_autoscroll: Option<f32>,
    pub toasts: Vec<Toast>,
    pub console: ConsoleLog,
    pub requests: Vec<EditorRequest>,
    pub project: Option<ProjectInfo>,
    pub recent: Vec<RecentProject>,
    /// Scenes collapsed in the explorer.
    pub collapsed_scenes: HashSet<SceneId>,
    /// Per-plugin preferences (a sandbox keyed by plugin id), persisted by the host.
    pub plugin_prefs: BTreeMap<String, BTreeMap<String, String>>,
    pub theme: ThemeMode,
    /// Frames drawn so far, when the debug counter is on.
    pub debug_frames: Option<u64>,
    /// The panel (full id) shown alone over the whole window, if any.
    pub maximized_panel: Option<String>,
    /// The preview: what the host published and what the panel posted.
    pub preview: PreviewState,
    /// Positions of the graph blocks (`.layout` sidecar).
    pub graph: crate::GraphState,
}

impl EditorState {
    const TOAST_LIFETIME: Duration = Duration::from_secs(4);

    pub fn push_toast(&mut self, level: Level, message: impl Into<String>) {
        let message = message.into();
        self.console.push(level, message.clone());
        self.toasts.push(Toast {
            message,
            level,
            expires: Instant::now() + Self::TOAST_LIFETIME,
        });
    }

    /// Drops expired toasts and returns when the next one expires.
    pub fn expire_toasts(&mut self, now: Instant) -> Option<Instant> {
        self.toasts.retain(|t| t.expires > now);
        self.toasts.iter().map(|t| t.expires).min()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error("no project is open")]
    NoProject,
    #[error(transparent)]
    Project(#[from] ProjectError),
}

/// What a plugin sees of the editor. Reads the project and emits commands;
/// there is no mutable access to the story.
pub struct EditorContext<'a> {
    project: Option<&'a mut Project>,
    state: &'a mut EditorState,
    registry: &'a EditorRegistry,
}

impl<'a> EditorContext<'a> {
    pub fn new(
        project: Option<&'a mut Project>,
        state: &'a mut EditorState,
        registry: &'a EditorRegistry,
    ) -> Self {
        Self {
            project,
            state,
            registry,
        }
    }

    /// The registry, with a lifetime independent of `&self` so that an
    /// inspector found in it can be called with `&mut self`.
    pub fn registry(&self) -> &'a EditorRegistry {
        self.registry
    }

    // ---- reading

    pub fn has_project(&self) -> bool {
        self.project.is_some()
    }

    pub fn project(&self) -> Option<&Project> {
        self.project.as_deref()
    }

    pub fn story(&self) -> Option<&Story> {
        self.project.as_deref().map(Project::story)
    }

    /// Current version of a node, to pass back as `expected_version`.
    pub fn node_version(&self, id: NodeId) -> Option<u64> {
        self.project.as_deref().and_then(|p| p.node_version(id))
    }

    pub fn project_info(&self) -> Option<&ProjectInfo> {
        self.state.project.as_ref()
    }

    pub fn state(&self) -> &EditorState {
        self.state
    }

    pub fn state_mut(&mut self) -> &mut EditorState {
        self.state
    }

    // ---- commands

    /// Executes one command as one undo entry.
    pub fn exec(&mut self, command: Command) -> Result<(), EditorError> {
        let project = self.project.as_deref_mut().ok_or(EditorError::NoProject)?;
        Ok(project.execute(command)?)
    }

    /// Like [`EditorContext::exec`], but consecutive calls with the same `key`
    /// share one undo entry (live text editing). The editor ends the session
    /// when no text field has focus any more.
    pub fn exec_merged(&mut self, command: Command, key: &str) -> Result<(), EditorError> {
        let project = self.project.as_deref_mut().ok_or(EditorError::NoProject)?;
        Ok(project.execute_merged(command, key)?)
    }

    /// Ends the current merge session of [`EditorContext::exec_merged`].
    pub fn end_merge(&mut self) {
        if let Some(project) = self.project.as_deref_mut() {
            project.end_merge();
        }
    }

    /// Runs several commands as one undo entry labeled `label`.
    pub fn transaction<R>(
        &mut self,
        label: &str,
        f: impl FnOnce(&mut Transaction<'_>) -> Result<R, ProjectError>,
    ) -> Result<R, EditorError> {
        let project = self.project.as_deref_mut().ok_or(EditorError::NoProject)?;
        Ok(project.transaction_labeled(label, f)?)
    }

    /// Runs a compound operation of `orbit_project::ops` (insert, delete,
    /// move, copy / paste, renames...): they are built from commands and
    /// each one is a single undo entry.
    pub fn ops<R>(
        &mut self,
        f: impl FnOnce(&mut Project) -> Result<R, ProjectError>,
    ) -> Result<R, EditorError> {
        let project = self.project.as_deref_mut().ok_or(EditorError::NoProject)?;
        Ok(f(project)?)
    }

    /// Like [`EditorContext::ops`], reporting a failure as an error toast.
    pub fn ops_or_toast<R>(
        &mut self,
        f: impl FnOnce(&mut Project) -> Result<R, ProjectError>,
    ) -> Option<R> {
        match self.ops(f) {
            Ok(value) => Some(value),
            Err(e) => {
                self.error(e.to_string());
                None
            }
        }
    }

    /// Like [`EditorContext::exec`], reporting a failure as an error toast.
    pub fn exec_or_toast(&mut self, command: Command) -> bool {
        match self.exec(command) {
            Ok(()) => true,
            Err(e) => {
                self.error(e.to_string());
                false
            }
        }
    }

    pub fn undo(&mut self) -> Result<bool, EditorError> {
        let project = self.project.as_deref_mut().ok_or(EditorError::NoProject)?;
        Ok(project.undo()?)
    }

    pub fn redo(&mut self) -> Result<bool, EditorError> {
        let project = self.project.as_deref_mut().ok_or(EditorError::NoProject)?;
        Ok(project.redo()?)
    }

    pub fn can_undo(&self) -> bool {
        self.project().is_some_and(Project::can_undo)
    }

    pub fn can_redo(&self) -> bool {
        self.project().is_some_and(Project::can_redo)
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.project().and_then(Project::undo_label)
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.project().and_then(Project::redo_label)
    }

    // ---- editor state

    pub fn selection(&self) -> Option<Selection> {
        self.state.selection
    }

    /// Shows `location` to the user: selects the node or scene (the scene
    /// editor, explorer and inspector follow), or leaves a pending reveal for
    /// the panel that owns characters, variables and assets. Panels are
    /// brought forward by the caller (see the built-in `reveal`).
    pub fn reveal(&mut self, location: Location) {
        self.state.reveal_serial += 1;
        match location {
            Location::Node(id) => {
                let scene = self
                    .story()
                    .and_then(|s| s.locate(id).map(|(si, _)| s.scenes[si].id));
                if let Some(scene) = scene {
                    self.state.collapsed_scenes.remove(&scene);
                    self.state.current_scene = Some(scene);
                }
                self.select(Selection::Node(id));
            }
            Location::Scene(id) => {
                self.state.collapsed_scenes.remove(&id);
                self.select(Selection::Scene(id));
            }
            other => {
                if let Location::Asset(id) = other {
                    self.state.selected_asset = Some(id);
                }
                self.state.pending_reveal = Some(other);
            }
        }
    }

    /// Selects one element (replaces any multi-selection).
    pub fn select(&mut self, selection: Selection) {
        self.state.selection = Some(selection);
        self.state.node_selection = match selection {
            Selection::Node(id) => vec![id],
            Selection::Scene(id) => {
                self.state.current_scene = Some(id);
                Vec::new()
            }
        };
    }

    /// Selects several nodes; `primary` (which should be one of them) is
    /// the one the inspector shows.
    pub fn select_nodes(&mut self, nodes: Vec<NodeId>, primary: NodeId) {
        self.state.selection = Some(Selection::Node(primary));
        self.state.node_selection = nodes;
    }

    /// The selected nodes (the multi-selection, or the single selected node).
    pub fn selected_nodes(&self) -> Vec<NodeId> {
        match self.state.selection {
            Some(Selection::Node(id)) if self.state.node_selection.contains(&id) => {
                self.state.node_selection.clone()
            }
            Some(Selection::Node(id)) => vec![id],
            _ => Vec::new(),
        }
    }

    pub fn clear_selection(&mut self) {
        self.state.selection = None;
        self.state.node_selection.clear();
    }

    pub fn info(&mut self, message: impl Into<String>) {
        self.state.push_toast(Level::Info, message);
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.state.push_toast(Level::Warn, message);
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.state.push_toast(Level::Error, message);
    }

    /// Writes to the console only (no toast).
    pub fn log(&mut self, level: Level, message: impl Into<String>) {
        self.state.console.push(level, message);
    }

    /// Posts a command to the preview (applied by the host between frames).
    pub fn preview_command(&mut self, command: PreviewCommand) {
        self.state.preview.command(command);
    }

    /// "Play from here": a new playthrough with the state a player would
    /// have at `node`.
    pub fn play_from(&mut self, node: NodeId) {
        self.state.preview.command(PreviewCommand::PlayFrom(node));
    }

    /// Asks the host to do something between frames.
    pub fn request(&mut self, request: EditorRequest) {
        self.state.requests.push(request);
    }

    /// Runs an action by full id (after the current frame).
    pub fn run_action(&mut self, id: impl Into<String>) {
        self.request(EditorRequest::RunAction(id.into()));
    }

    // ---- plugin preferences

    pub fn pref(&self, plugin_id: &str, key: &str) -> Option<&str> {
        self.state
            .plugin_prefs
            .get(plugin_id)
            .and_then(|m| m.get(key))
            .map(String::as_str)
    }

    pub fn set_pref(&mut self, plugin_id: &str, key: &str, value: impl Into<String>) {
        self.state
            .plugin_prefs
            .entry(plugin_id.to_owned())
            .or_default()
            .insert(key.to_owned(), value.into());
    }
}
