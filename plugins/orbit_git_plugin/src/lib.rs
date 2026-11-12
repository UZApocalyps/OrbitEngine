//! "Git": an official editor plugin that puts the basics of version control
//! next to the story.
//!
//! A docked panel for the open project: the current branch, the changed files
//! with their state, a message field and a Commit button (a check box per
//! file), the 20 latest commits, and, for a project that is not in a
//! repository, a button that creates one with a `.gitignore` that leaves out
//! the editor's own files.
//!
//! It drives the system's `git` executable and nothing else is installed. The
//! scope is deliberately small and safe: `status`, `add`, `commit`, `log` and
//! `init`. No push, pull, fetch, reset, checkout, no file is ever deleted and
//! the network is never used.
//!
//! * `parse.rs`: reading what git prints, the `.gitignore` text (pure, unit
//!   tested);
//! * `git.rs`: running git (always in a background task) and the operations,
//!   tested on a real scratch repository;
//! * this file: the panel.

mod git;
mod parse;

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use orbit_editor_sdk::egui;
use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::{ActionDef, DockSlot, EditorContext, EditorRequest, PanelDef, TaskError, TaskHandle, icons};

use git::{GitError, Repo, Runner, Snapshot};
use parse::{Entry, FileState};

/// The plugin id (also in `manifest.ron`).
pub const ID: &str = "orbit.git";
/// Full id of the panel (`plugin id` + local id).
const PANEL: &str = "orbit.git.panel";
const ROW_HEIGHT: f32 = 20.0;

/// What the user asked for.
enum Action {
    Refresh,
    Init,
    Commit { paths: Vec<String>, message: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Did {
    Nothing,
    Init,
    Commit,
}

/// What a task brings back. A failure is data, not an error: the panel shows
/// it.
struct Loaded {
    /// No git executable was found.
    missing_git: bool,
    /// git's message when something failed.
    error: Option<String>,
    /// The repository as it is after the action (`None` when git failed).
    repo: Option<Repo>,
    did: Did,
}

fn work(runner: Runner, action: Action, cancelled: &dyn Fn() -> bool) -> Result<Loaded, TaskError> {
    let mut loaded = Loaded { missing_git: false, error: None, repo: None, did: Did::Nothing };
    let fail = |e: GitError, loaded: &mut Loaded| match e {
        GitError::NotFound => loaded.missing_git = true,
        GitError::Cancelled => {}
        GitError::TimedOut => loaded.error = Some("git did not answer in time".to_owned()),
        GitError::Failed(m) => loaded.error = Some(m),
    };
    let done = match action {
        Action::Refresh => Ok(Did::Nothing),
        Action::Init => git::init(&runner, cancelled).map(|()| Did::Init),
        Action::Commit { paths, message } => git::commit(&runner, &paths, &message, cancelled).map(|()| Did::Commit),
    };
    match done {
        Ok(did) => loaded.did = did,
        Err(GitError::Cancelled) => return Err(TaskError::Cancelled),
        Err(e) => fail(e, &mut loaded),
    }
    // Always show the state after the action (also after a failed one: a
    // half-done commit must be visible).
    if !loaded.missing_git {
        match git::snapshot(&runner, cancelled) {
            Ok(repo) => loaded.repo = Some(repo),
            Err(GitError::Cancelled) => return Err(TaskError::Cancelled),
            Err(e) if loaded.error.is_none() => fail(e, &mut loaded),
            Err(_) => {}
        }
    }
    Ok(loaded)
}

// ------------------------------------------------------------------- state

struct State {
    handle: RefCell<Option<TaskHandle<Loaded>>>,
    /// The latest result, kept on screen while a new task runs.
    loaded: RefCell<Option<Rc<Loaded>>>,
    /// The project the result belongs to.
    root: RefCell<Option<PathBuf>>,
    /// Files the user unchecked (a new file is checked by default).
    unchecked: RefCell<BTreeSet<String>>,
    message: RefCell<String>,
    /// The story had unsaved edits at the last frame (a save refreshes).
    was_modified: Cell<bool>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            handle: RefCell::new(None),
            loaded: RefCell::new(None),
            root: RefCell::new(None),
            unchecked: RefCell::new(BTreeSet::new()),
            message: RefCell::new(String::new()),
            was_modified: Cell::new(false),
        }
    }
}

impl State {
    fn running(&self) -> bool {
        self.handle.borrow().is_some()
    }

    fn start(&self, ctx: &mut EditorContext<'_>, action: Action) {
        if self.running() {
            return;
        }
        let Some(info) = ctx.project_info() else { return };
        let runner = Runner::new(info.root.clone());
        let title = ctx.t("git.task");
        let step = ctx.t(match action {
            Action::Refresh => "git.task_refresh",
            Action::Init => "git.task_init",
            Action::Commit { .. } => "git.task_commit",
        });
        let handle = ctx.state().spawn_task(title, None, move |progress| {
            progress.set(0.0, step);
            work(runner, action, &|| progress.is_cancelled())
        });
        *self.handle.borrow_mut() = Some(handle);
    }

    fn poll(&self, ctx: &mut EditorContext<'_>) {
        let outcome = self.handle.borrow_mut().as_mut().and_then(TaskHandle::poll);
        let Some(result) = outcome else { return };
        *self.handle.borrow_mut() = None;
        match result {
            Ok(loaded) => {
                match loaded.did {
                    Did::Commit if loaded.error.is_none() => {
                        self.message.borrow_mut().clear();
                        let message = ctx.t("git.commit_done");
                        ctx.info(message);
                    }
                    Did::Init if loaded.error.is_none() => {
                        let message = ctx.t("git.init_done");
                        ctx.info(message);
                    }
                    _ => {}
                }
                // Forget deselections of files that are no longer listed.
                if let Some(Repo::Root(snap)) = &loaded.repo {
                    let listed: BTreeSet<&str> = snap.status.entries.iter().map(|e| e.path.as_str()).collect();
                    self.unchecked.borrow_mut().retain(|p| listed.contains(p.as_str()));
                }
                *self.loaded.borrow_mut() = Some(Rc::new(loaded));
            }
            Err(TaskError::Cancelled) => {}
            Err(TaskError::Failed(e)) => {
                *self.loaded.borrow_mut() =
                    Some(Rc::new(Loaded { missing_git: false, error: Some(e), repo: None, did: Did::Nothing }));
            }
            Err(_) => {}
        }
    }
}

// ---------------------------------------------------------------------- ui

fn weak(ui: &egui::Ui) -> egui::Color32 {
    ui.visuals().weak_text_color()
}

fn state_style(ui: &egui::Ui, state: FileState) -> (&'static str, &'static str, egui::Color32) {
    match state {
        FileState::New => ("A", "git.state.new", egui::Color32::from_rgb(90, 190, 110)),
        FileState::Modified => ("M", "git.state.modified", ui.visuals().warn_fg_color),
        FileState::Deleted => ("D", "git.state.deleted", ui.visuals().error_fg_color),
        FileState::Renamed => ("R", "git.state.renamed", egui::Color32::from_rgb(110, 160, 230)),
        FileState::Copied => ("C", "git.state.copied", egui::Color32::from_rgb(110, 160, 230)),
        FileState::TypeChanged => ("T", "git.state.type", ui.visuals().warn_fg_color),
        FileState::Conflict => ("!", "git.state.conflict", ui.visuals().error_fg_color),
    }
}

/// A hint for the failures people actually meet.
fn hint_for(error: &str) -> Option<&'static str> {
    let e = error.to_lowercase();
    if e.contains("tell me who you are") || e.contains("unable to auto-detect") || e.contains("empty ident") {
        Some("git.hint_identity")
    } else if e.contains("dubious ownership") || e.contains("safe.directory") {
        Some("git.hint_safe_directory")
    } else if e.contains("index.lock") {
        Some("git.hint_lock")
    } else {
        None
    }
}

fn panel(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, s: &State) {
    s.poll(ctx);
    let Some(info) = ctx.project_info() else {
        ui.add_space(8.0);
        ui.label(ctx.t("git.no_project"));
        return;
    };
    let (root, modified) = (info.root.clone(), info.modified);

    // Another project: forget everything about the previous one.
    if s.root.borrow().as_ref() != Some(&root) {
        *s.root.borrow_mut() = Some(root);
        *s.loaded.borrow_mut() = None;
        s.unchecked.borrow_mut().clear();
        s.message.borrow_mut().clear();
    }
    // Read git the first time, and after the story was saved.
    let saved = s.was_modified.get() && !modified;
    s.was_modified.set(modified);
    if !s.running() && (s.loaded.borrow().is_none() || saved) {
        s.start(ctx, Action::Refresh);
    }
    let loaded = s.loaded.borrow().clone();

    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!s.running(), egui::Button::new(format!("{} {}", icons::ARROWS_CLOCKWISE, ctx.t("git.refresh"))))
            .clicked()
        {
            s.start(ctx, Action::Refresh);
        }
        if s.running() {
            ui.spinner();
            ui.label(ctx.t("git.working"));
        }
    });
    ui.label(egui::RichText::new(ctx.t("git.scope")).small().color(weak(ui)));
    let Some(loaded) = loaded else { return };
    ui.separator();

    if loaded.missing_git {
        missing_git(ui, ctx);
        return;
    }
    if let Some(error) = &loaded.error {
        error_box(ui, ctx, error);
    }
    match &loaded.repo {
        None => {}
        Some(Repo::None) => no_repository(ui, ctx, s),
        Some(Repo::Nested) => {
            ui.colored_label(ui.visuals().warn_fg_color, ctx.t("git.nested"));
        }
        Some(Repo::Root(snap)) => repository(ui, ctx, s, snap, modified),
    }
}

fn missing_git(ui: &mut egui::Ui, ctx: &EditorContext<'_>) {
    ui.colored_label(ui.visuals().warn_fg_color, egui::RichText::new(ctx.t("git.missing_title")).strong());
    ui.label(ctx.t("git.missing_help"));
}

fn error_box(ui: &mut egui::Ui, ctx: &EditorContext<'_>, error: &str) {
    ui.colored_label(ui.visuals().error_fg_color, ctx.tf("git.failed", &[("error", &error)]));
    if let Some(hint) = hint_for(error) {
        ui.label(egui::RichText::new(ctx.t(hint)).small());
    }
}

fn no_repository(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, s: &State) {
    ui.label(egui::RichText::new(ctx.t("git.none_title")).strong());
    ui.label(ctx.t("git.none_help"));
    ui.add_space(4.0);
    ui.label(egui::RichText::new(ctx.t("git.ignore_title")).small().color(weak(ui)));
    for line in parse::GITIGNORE.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
        ui.label(egui::RichText::new(line).monospace().small());
    }
    ui.add_space(4.0);
    if ui
        .add_enabled(!s.running(), egui::Button::new(format!("{} {}", icons::GIT_BRANCH, ctx.t("git.init"))))
        .clicked()
    {
        s.start(ctx, Action::Init);
    }
}

fn repository(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, s: &State, snap: &Snapshot, modified: bool) {
    let branch = &snap.status.branch;
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{}", icons::GIT_BRANCH));
        match &branch.name {
            Some(name) => ui.label(egui::RichText::new(name).strong()),
            None => ui.label(egui::RichText::new(ctx.t("git.detached")).strong()),
        };
        if let Some(up) = &branch.upstream {
            ui.label(egui::RichText::new(ctx.tf("git.upstream", &[("name", up)])).small().color(weak(ui)));
        }
        if branch.no_commits {
            ui.label(egui::RichText::new(ctx.t("git.no_commits")).small().color(weak(ui)));
        }
    });
    if modified {
        ui.colored_label(ui.visuals().warn_fg_color, ctx.t("git.unsaved"));
    }
    ui.add_space(2.0);

    let entries = &snap.status.entries;
    let conflicts = entries.iter().filter(|e| e.conflicted()).count();
    let chosen: Vec<&Entry> = {
        let unchecked = s.unchecked.borrow();
        entries.iter().filter(|e| !unchecked.contains(&e.path) && !e.conflicted()).collect()
    };

    // ---- changed files
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(ctx.tf("git.changes", &[("count", &entries.len())])).strong());
        if !entries.is_empty() {
            if ui.small_button(ctx.t("git.select_all")).clicked() {
                s.unchecked.borrow_mut().clear();
            }
            if ui.small_button(ctx.t("git.select_none")).clicked() {
                *s.unchecked.borrow_mut() = entries.iter().map(|e| e.path.clone()).collect();
            }
        }
    });
    if entries.is_empty() {
        ui.label(egui::RichText::new(ctx.t("git.clean")).color(weak(ui)));
    } else {
        egui::ScrollArea::vertical()
            .id_salt("git.files")
            .max_height(260.0)
            .auto_shrink([false, true])
            .show_rows(ui, ROW_HEIGHT, entries.len(), |ui, range| {
                for e in &entries[range] {
                    ui.horizontal(|ui| {
                        let mut on = !s.unchecked.borrow().contains(&e.path);
                        let (letter, key, color) = state_style(ui, e.state());
                        let enabled = !e.conflicted();
                        if ui.add_enabled(enabled, egui::Checkbox::without_text(&mut on)).changed() {
                            let mut u = s.unchecked.borrow_mut();
                            if on {
                                u.remove(&e.path);
                            } else {
                                u.insert(e.path.clone());
                            }
                        }
                        ui.label(egui::RichText::new(letter).monospace().strong().color(color))
                            .on_hover_text(ctx.t(key));
                        let mut text = e.path.clone();
                        if let Some(from) = &e.from {
                            text = format!("{from}  →  {}", e.path);
                        }
                        let mut label = egui::RichText::new(text);
                        if e.state() == FileState::Deleted {
                            label = label.strikethrough();
                        }
                        ui.add(egui::Label::new(label).truncate());
                        if e.staged() {
                            ui.label(egui::RichText::new(ctx.t("git.staged")).small().color(weak(ui)));
                        }
                    });
                }
            });
    }
    if conflicts > 0 {
        ui.colored_label(ui.visuals().error_fg_color, ctx.tf("git.conflicts", &[("count", &conflicts)]));
    }

    // ---- commit
    ui.add_space(6.0);
    let mut message = s.message.borrow().clone();
    if ui
        .add(
            egui::TextEdit::multiline(&mut message)
                .hint_text(ctx.t("git.message_hint"))
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        )
        .changed()
    {
        *s.message.borrow_mut() = message.clone();
    }
    let can_commit = !s.running() && !message.trim().is_empty() && !chosen.is_empty() && conflicts == 0;
    let label = format!("{} {}", icons::GIT_COMMIT, ctx.tf("git.commit", &[("count", &chosen.len())]));
    if ui.add_enabled(can_commit, egui::Button::new(label)).clicked() {
        let paths = parse::paths_to_commit(chosen.iter().copied());
        s.start(ctx, Action::Commit { paths, message: message.trim().to_owned() });
    }

    // ---- history
    ui.add_space(6.0);
    egui::CollapsingHeader::new(egui::RichText::new(ctx.tf("git.history", &[("count", &snap.log.len())])).strong())
        .default_open(true)
        .show(ui, |ui| {
            if snap.log.is_empty() {
                ui.label(egui::RichText::new(ctx.t("git.no_history")).color(weak(ui)));
                return;
            }
            egui::ScrollArea::vertical().id_salt("git.log").auto_shrink([false, true]).show(ui, |ui| {
                egui::Grid::new("git.log_grid").num_columns(4).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                    for c in &snap.log {
                        ui.label(egui::RichText::new(&c.date).monospace().small());
                        ui.label(egui::RichText::new(&c.author).small().color(weak(ui)));
                        ui.label(egui::RichText::new(&c.short).monospace().small().color(weak(ui)));
                        ui.add(egui::Label::new(&c.subject).truncate());
                        ui.end_row();
                    }
                });
            });
        });
}

// ------------------------------------------------------------------ plugin

#[derive(Default)]
pub struct GitPlugin {
    state: Rc<State>,
}

impl NativePlugin for GitPlugin {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        let s = self.state.clone();
        host.panel(PanelDef::new("panel", "git.title", icons::GIT_BRANCH, DockSlot::Hidden, move |ui, ctx| {
            panel(ui, ctx, &s);
        }));
        host.action(
            ActionDef::new("open", "git.open", |ctx| ctx.request(EditorRequest::ShowPanel(PANEL.to_owned())))
                .icon(icons::GIT_BRANCH)
                .category("category.tools")
                .menu("tools", 930),
        );
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(
    GitPlugin,
    id = "orbit.git",
    name = "Git",
    description = "Git panel for the open project: branch, changed files, commit, recent history, repository creation. Uses the system's git; no network, nothing destructive.",
    author = "OrbitEngine contributors",
);
