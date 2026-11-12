//! Game plugins in the plugin manager (see `docs/plugin-store.md`, "Game
//! plugins"):
//!
//! * the Store tab gets an "Editor plugins / Game plugins" switch; the game
//!   view lists the `game_plugins` of the enabled catalogs (the cached
//!   `index.json` the store already keeps), with a "Game" badge, the
//!   compatibility (plugin API, engine range) and the state in the open
//!   project (installed, update, changed locally);
//! * the Installed tab gets a "Game plugins of this project" section: the
//!   crates of `plugins/`, where they come from, local changes, whether the
//!   editor knows their node types, and whether an export can build them
//!   (Rust toolchain, engine sources).
//!
//! The work is `orbit_plugin_store::game` (no egui); installs run as editor
//! tasks. `orbit.project.ron` (`plugins`: id and version) is updated with
//! the same load / save as the export profiles: installing files is not an
//! undoable story edit.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Color32, RichText, ScrollArea, Ui};
use orbit_editor_api::{EditorContext, Level, TaskError, TaskHandle, icons};
use orbit_plugin_store::download::DownloadError;
use orbit_plugin_store::game::PackageSource;
use orbit_plugin_store::game::catalog::{
    self as game_catalog, GameAvailability, GamePlugin, find_game_updates,
};
use orbit_plugin_store::game::download::{GameDownloadPlan, download};
use orbit_plugin_store::game::project::{
    self, Change, Conflict, Expected, InstallError, InstallOptions, InstallOutcome, InstalledGame,
    OnConflict, UninstallOutcome,
};
use orbit_plugin_store::signing::{Ed25519, SignatureError};
use orbit_plugin_store::sources::Sources;
use orbit_plugin_store::{SourceEntry, StorePaths, Trust, fetch};
use orbit_project::{PluginRequirement, ProjectManifest};

use super::ui_util::{
    accent, card, danger, primary_button, secondary_button, success, warning, weak,
};
use crate::plugins::SharedPlugins;
use crate::storage::FsStorage;

/// The engine version game plugins are matched against.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often the catalogs and the project's `plugins/` are read again while
/// the panel shows (the editor store refreshes the cache in its own task).
const RELOAD: Duration = Duration::from_secs(3);

/// Which plugins the Store tab lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StoreKind {
    #[default]
    Editor,
    Game,
}

/// Whether an export of the project can build its plugins.
#[derive(Clone, Debug)]
struct Toolchain {
    cargo: Result<String, String>,
    engine: Result<PathBuf, String>,
}

enum Done {
    Installed(InstallOutcome, Result<bool, String>),
    Uninstalled(UninstallOutcome, Result<bool, String>),
}

enum Failure {
    Download(DownloadError),
    Install(InstallError),
    /// The verified package is kept to install it again with a backup.
    Conflict(Box<(Conflict, Retry)>),
}

/// A verified package waiting for "replace (backup)".
#[derive(Clone)]
struct Retry {
    file: PathBuf,
    source: PackageSource,
    expected: Expected,
    name: String,
}

type Outcome = Result<Done, Failure>;

/// An install waiting for the user's go-ahead.
struct Confirm {
    plan: GameDownloadPlan,
    name: String,
    /// Installed from another source (its name).
    replaces: Option<String>,
    /// What is in the way (known before downloading).
    conflict: Option<Conflict>,
}

#[derive(Default)]
pub struct GameUi {
    pub kind: StoreKind,
    loaded_at: Option<Instant>,
    root: Option<PathBuf>,
    games: Vec<(SourceEntry, Vec<GamePlugin>)>,
    warnings: Vec<String>,
    /// Ids of the editor plugins the catalogs list (the editor half of a
    /// game plugin can be one).
    editor_ids: BTreeSet<String>,
    installed: Vec<InstalledGame>,
    changes: BTreeMap<PathBuf, Vec<Change>>,
    toolchain: Option<Toolchain>,
    toolchain_task: Option<TaskHandle<Toolchain>>,
    refresh: Option<TaskHandle<()>>,
    search: String,
    task: Option<(String, TaskHandle<Outcome>)>,
    confirm: Option<Confirm>,
    conflict: Option<(Conflict, Retry)>,
    confirm_uninstall: Option<String>,
    message: Option<(Level, String)>,
}

fn paths_of(shared: &SharedPlugins) -> Option<StorePaths> {
    shared
        .borrow()
        .config_dir
        .as_deref()
        .map(StorePaths::for_config_dir)
}

fn project_root(ctx: &EditorContext<'_>) -> Option<PathBuf> {
    ctx.project_info().map(|i| i.root.clone())
}

// ------------------------------------------------------------------ manifest

/// Sets the requirements `set` (id, version) and removes `remove` in the
/// manifest's `plugins` list (pure, tested).
pub fn apply_requirements(
    manifest: &mut ProjectManifest,
    set: &[(String, String)],
    remove: &[String],
) {
    manifest.plugins.retain(|r| !remove.contains(&r.id));
    for (id, version) in set {
        match manifest.plugins.iter_mut().find(|r| &r.id == id) {
            Some(r) => r.version = version.clone(),
            None => manifest.plugins.push(PluginRequirement {
                id: id.clone(),
                version: version.clone(),
            }),
        }
    }
    manifest.plugins.sort_by(|a, b| a.id.cmp(&b.id));
}

/// Updates `orbit.project.ron` of `root`; `Ok(false)`: the project has no
/// manifest (a bare story folder), nothing written.
pub fn update_requirements(
    root: &Path,
    set: &[(String, String)],
    remove: &[String],
) -> Result<bool, String> {
    if !root.join(orbit_project::MANIFEST_FILE).is_file() {
        return Ok(false);
    }
    let storage = FsStorage::new(root);
    let mut manifest = orbit_project::load_manifest(&storage).map_err(|e| e.to_string())?;
    let before = manifest.plugins.clone();
    apply_requirements(&mut manifest, set, remove);
    if manifest.plugins != before {
        orbit_project::save_manifest(&storage, &manifest).map_err(|e| e.to_string())?;
    }
    Ok(true)
}

// ------------------------------------------------------------------ state

impl GameUi {
    /// Reads the cached catalogs and the project's `plugins/` again when
    /// they are older than [`RELOAD`] or the project changed.
    fn ensure_loaded(&mut self, ctx: &EditorContext<'_>, shared: &SharedPlugins, force: bool) {
        let root = project_root(ctx);
        let stale = self.loaded_at.is_none_or(|t| t.elapsed() > RELOAD);
        if !force && !stale && root == self.root {
            return;
        }
        if root != self.root {
            self.toolchain = None;
            self.confirm = None;
            self.conflict = None;
            self.confirm_uninstall = None;
        }
        self.loaded_at = Some(Instant::now());
        self.root = root.clone();
        self.games.clear();
        self.warnings.clear();
        self.editor_ids.clear();
        if let Some(paths) = paths_of(shared) {
            let sources = Sources::load(&paths);
            for s in sources.enabled() {
                if let Some((games, warnings)) = game_catalog::cached(&paths, s) {
                    self.warnings
                        .extend(warnings.into_iter().map(|w| format!("{}: {w}", s.name)));
                    if !games.is_empty() {
                        self.games.push((s.clone(), games));
                    }
                }
                if let Some(c) = fetch::cached(&paths, s) {
                    self.editor_ids
                        .extend(c.parsed.catalog.plugins.iter().map(|p| p.id.clone()));
                }
            }
        }
        self.installed = root.as_deref().map(project::scan).unwrap_or_default();
        self.changes = self
            .installed
            .iter()
            .filter_map(|g| {
                let record = g.record.as_ref()?;
                let changes = project::local_changes(&g.dir, record).ok()?;
                (!changes.is_empty()).then(|| (g.dir.clone(), changes))
            })
            .collect();
        if self.toolchain.is_none() && self.toolchain_task.is_none() && root.is_some() {
            self.toolchain_task = Some(ctx.state().spawn_task(
                ctx.t("gplug.checking_toolchain"),
                None,
                |_| {
                    Ok(Toolchain {
                        cargo: orbit_export::plugins::cargo_version().map_err(|e| e.to_string()),
                        engine: orbit_export::plugins::engine_source().map_err(|e| e.to_string()),
                    })
                },
            ));
        }
    }

    fn installed_of(&self, id: &str) -> Option<&InstalledGame> {
        self.installed.iter().find(|g| g.store_id() == Some(id))
    }

    fn updates(&self) -> BTreeMap<String, game_catalog::GameUpdate> {
        let list: Vec<(String, String, String, String)> = self
            .installed
            .iter()
            .filter_map(|g| g.record.as_ref())
            .map(|r| {
                (
                    r.id.clone(),
                    r.version.clone(),
                    r.source.id.clone(),
                    r.source.url.clone(),
                )
            })
            .collect();
        let catalogs: Vec<(&SourceEntry, &[GamePlugin])> =
            self.games.iter().map(|(s, g)| (s, g.as_slice())).collect();
        find_game_updates(
            &list,
            &catalogs,
            ENGINE_VERSION,
            orbit_plugin_api::API_VERSION,
        )
        .into_iter()
        .map(|u| (u.id.clone(), u))
        .collect()
    }
}

/// Polls the tasks of the game view; call every frame the manager shows.
pub fn poll(ctx: &mut EditorContext<'_>, _shared: &SharedPlugins, g: &mut GameUi) {
    if let Some(h) = g.toolchain_task.as_mut()
        && let Some(out) = h.poll()
    {
        g.toolchain_task = None;
        g.toolchain = out.ok();
    }
    if let Some(h) = g.refresh.as_mut()
        && h.poll().is_some()
    {
        g.refresh = None;
        g.loaded_at = None;
    }
    let Some((_, handle)) = g.task.as_mut() else {
        return;
    };
    let Some(outcome) = handle.poll() else { return };
    g.task = None;
    g.loaded_at = None;
    let manifest_note = |ctx: &EditorContext<'_>, r: &Result<bool, String>| match r {
        Ok(true) => String::new(),
        Ok(false) => format!(" {}", ctx.t("gplug.no_manifest")),
        Err(e) => format!(" {}", ctx.tf("gplug.manifest_failed", &[("error", e)])),
    };
    g.message = Some(match outcome {
        Ok(Ok(Done::Installed(out, manifest))) => {
            let mut text = match &out.previous {
                Some(prev) => ctx.tf(
                    "gplug.updated",
                    &[
                        ("name", &out.name),
                        ("from", prev),
                        ("version", &out.version),
                    ],
                ),
                None => ctx.tf(
                    "gplug.installed",
                    &[("name", &out.name), ("version", &out.version)],
                ),
            };
            text.push_str(&manifest_note(ctx, &manifest));
            for b in &out.backups {
                text.push(' ');
                text.push_str(&ctx.tf("gplug.backup_kept", &[("path", &b.display())]));
            }
            if out.engine_path.is_none() {
                text.push(' ');
                text.push_str(&ctx.t("gplug.engine_not_rewritten"));
            }
            (
                if out.engine_path.is_none() {
                    Level::Warn
                } else {
                    Level::Info
                },
                text,
            )
        }
        Ok(Ok(Done::Uninstalled(out, manifest))) => {
            let mut text = ctx.tf("gplug.uninstalled", &[("name", &out.name)]);
            text.push_str(&manifest_note(ctx, &manifest));
            if let Some(b) = &out.backup {
                text.push(' ');
                text.push_str(&ctx.tf("gplug.backup_kept", &[("path", &b.display())]));
            }
            (Level::Info, text)
        }
        Ok(Err(Failure::Download(e))) => (
            Level::Error,
            ctx.tf("gplug.failed", &[("reason", &describe_download(ctx, &e))]),
        ),
        Ok(Err(Failure::Install(e))) => (Level::Error, ctx.tf("gplug.failed", &[("reason", &e)])),
        Ok(Err(Failure::Conflict(boxed))) => {
            let (conflict, retry) = *boxed;
            let text = describe_conflict(ctx, &conflict);
            g.conflict = Some((conflict, retry));
            (Level::Warn, text)
        }
        Err(TaskError::Failed(e)) => (Level::Error, e),
        Err(_) => (Level::Warn, ctx.t("gplug.cancelled")),
    });
    if let Some((level, text)) = &g.message {
        ctx.log(*level, text.clone());
    }
}

// ------------------------------------------------------------------ texts

fn describe_download(ctx: &EditorContext<'_>, e: &DownloadError) -> String {
    match e {
        DownloadError::Signature(SignatureError::Missing) => ctx.t("store.error.unsigned"),
        DownloadError::Signature(SignatureError::Invalid) => ctx.t("store.error.bad_signature"),
        DownloadError::Signature(SignatureError::NoKey) => ctx.t("store.error.no_key"),
        DownloadError::Network(d) => ctx.tf("store.error.network", &[("detail", d)]),
        DownloadError::Http(s) => ctx.tf("store.error.http", &[("status", s)]),
        DownloadError::Size { expected, got } => {
            ctx.tf("store.error.size", &[("expected", expected), ("got", got)])
        }
        DownloadError::Checksum { .. } => ctx.t("store.error.checksum"),
        DownloadError::Cancelled => ctx.t("store.cancelled"),
        DownloadError::Io(d) => ctx.tf("store.error.io", &[("detail", d)]),
    }
}

fn describe_conflict(ctx: &EditorContext<'_>, c: &Conflict) -> String {
    match c {
        Conflict::Unmanaged(dir) => ctx.tf("gplug.conflict.unmanaged", &[("path", &dir.display())]),
        Conflict::Modified { dir, changes } => ctx.tf(
            "gplug.conflict.modified",
            &[("path", &dir.display()), ("count", &changes.len())],
        ),
        Conflict::SameId { dir, id } => ctx.tf(
            "gplug.conflict.same_id",
            &[("path", &dir.display()), ("id", id)],
        ),
    }
}

fn trust_badge(ui: &mut Ui, ctx: &EditorContext<'_>, trust: Trust) {
    let color = match trust {
        Trust::Official => success(),
        Trust::Trusted => Color32::from_rgb(90, 150, 230),
        Trust::Unsigned => Color32::from_rgb(230, 150, 40),
    };
    badge(ui, &ctx.t(&format!("store.trust.{}", trust.key())), color);
}

fn badge(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(color)));
}

fn game_badge(ui: &mut Ui, ctx: &EditorContext<'_>) {
    badge(
        ui,
        &format!("{} {}", icons::GAME_CONTROLLER, ctx.t("gplug.badge")),
        accent(ui),
    );
}

fn engine_range(ctx: &EditorContext<'_>, min: &str, max: Option<&str>) -> String {
    match max {
        Some(max) => ctx.tf("gplug.range", &[("min", &min), ("max", &max)]),
        None => ctx.tf("gplug.range_open", &[("min", &min)]),
    }
}

fn open_folder(path: &Path) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// The toolchain line: an install always works, an export needs both.
fn toolchain_line(ui: &mut Ui, ctx: &EditorContext<'_>, g: &GameUi) {
    match &g.toolchain {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(ctx.t("gplug.checking_toolchain"))
                        .small()
                        .color(weak(ui)),
                );
            });
        }
        Some(t) => match (&t.cargo, &t.engine) {
            (Ok(cargo), Ok(engine)) => {
                ui.label(
                    RichText::new(ctx.tf(
                        "gplug.toolchain_ok",
                        &[("cargo", cargo), ("engine", &engine.display())],
                    ))
                    .small()
                    .color(success()),
                );
            }
            (cargo, engine) => {
                ui.colored_label(warning(ui), ctx.t("gplug.export_will_fail"));
                if let Err(e) = cargo {
                    ui.label(RichText::new(e).small().color(danger(ui)));
                }
                if let Err(e) = engine {
                    ui.label(RichText::new(e).small().color(danger(ui)));
                }
            }
        },
    }
}

fn show_message(ui: &mut Ui, g: &GameUi) {
    if let Some((level, text)) = &g.message {
        let color = match level {
            Level::Info => success(),
            Level::Warn => warning(ui),
            Level::Error => danger(ui),
        };
        ui.colored_label(color, text);
    }
}

fn task_progress(ui: &mut Ui, ctx: &EditorContext<'_>, g: &GameUi) {
    if let Some((id, handle)) = &g.task {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.add(
                egui::ProgressBar::new(handle.fraction().unwrap_or(0.0))
                    .desired_width(160.0)
                    .show_percentage(),
            );
            ui.label(RichText::new(id).monospace().color(weak(ui)));
            if ui.button(ctx.t("plugins.cancel")).clicked() {
                handle.cancel();
            }
        });
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}

// ------------------------------------------------------------------ install

fn request_install(
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    g: &mut GameUi,
    source_id: &str,
    plugin_id: &str,
) {
    let Some(root) = g.root.clone() else { return };
    let Some((source, plugin)) =
        g.games
            .iter()
            .find(|(s, _)| s.id == source_id)
            .and_then(|(s, games)| {
                games
                    .iter()
                    .find(|p| p.id == plugin_id)
                    .map(|p| (s.clone(), p.clone()))
            })
    else {
        return;
    };
    let Some(version) =
        game_catalog::select_game(&plugin, ENGINE_VERSION, orbit_plugin_api::API_VERSION)
            .version()
            .cloned()
    else {
        return;
    };
    let replaces = g
        .installed_of(plugin_id)
        .and_then(|i| i.record.as_ref())
        .filter(|r| r.source.id != source.id && r.source.url != source.url)
        .map(|r| r.source.name.clone());
    let conflict = project::precheck(&root, plugin_id);
    let plan = GameDownloadPlan {
        source: PackageSource::of(&source),
        plugin_id: plugin.id.clone(),
        version,
    };
    if source.trust() == Trust::Official && replaces.is_none() && conflict.is_none() {
        start_install(ctx, shared, g, plan, &plugin.name, OnConflict::Refuse);
    } else {
        g.confirm = Some(Confirm {
            plan,
            name: plugin.name.clone(),
            replaces,
            conflict,
        });
    }
}

fn install_options(on_conflict: OnConflict, engine: Option<&Path>) -> InstallOptions<'_> {
    InstallOptions {
        engine_version: ENGINE_VERSION,
        api_version: orbit_plugin_api::API_VERSION,
        engine_root: engine,
        on_conflict,
    }
}

/// Installs a verified package file in `root` and updates the manifest.
fn install_file(root: &Path, retry: &Retry, on_conflict: OnConflict) -> Outcome {
    let bytes = std::fs::read(&retry.file)
        .map_err(|e| Failure::Install(InstallError::Io(e.to_string())))?;
    let engine = orbit_export::plugins::engine_source().ok();
    match project::install(
        root,
        &bytes,
        &retry.source,
        Some(&retry.expected),
        &install_options(on_conflict, engine.as_deref()),
    ) {
        Ok(out) => {
            let _ = std::fs::remove_file(&retry.file);
            let set: Vec<(String, String)> = out
                .plugin_ids
                .iter()
                .map(|id| (id.clone(), out.version.clone()))
                .collect();
            let manifest = update_requirements(root, &set, &out.removed_ids);
            Ok(Done::Installed(out, manifest))
        }
        Err(InstallError::Conflict(conflict)) => {
            Err(Failure::Conflict(Box::new((conflict, retry.clone()))))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&retry.file);
            Err(Failure::Install(e))
        }
    }
}

fn start_install(
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    g: &mut GameUi,
    plan: GameDownloadPlan,
    name: &str,
    on_conflict: OnConflict,
) {
    if g.task.is_some() {
        return;
    }
    let (Some(paths), Some(root)) = (paths_of(shared), g.root.clone()) else {
        return;
    };
    let id = plan.plugin_id.clone();
    let name = name.to_owned();
    let title = ctx.tf(
        "gplug.installing",
        &[("name", &name), ("version", &plan.version.version)],
    );
    let handle = ctx.state().spawn_task(title, None, move |progress| {
        let verified = download(&paths, &plan, &Ed25519, &|got, total| {
            progress.set_fraction(got as f32 / total.max(1) as f32 * 0.9);
            !progress.is_cancelled()
        });
        let verified = match verified {
            Ok(v) => v,
            Err(DownloadError::Cancelled) => return Err(TaskError::Cancelled),
            Err(e) => return Ok(Err(Failure::Download(e))),
        };
        progress.check()?;
        let retry = Retry {
            file: verified.file,
            source: plan.source.clone(),
            expected: Expected {
                id: plan.plugin_id.clone(),
                version: plan.version.version.clone(),
                sha256: plan.version.sha256.clone(),
            },
            name,
        };
        let out = install_file(&root, &retry, on_conflict);
        progress.set_fraction(1.0);
        Ok(out)
    });
    g.task = Some((id, handle));
    g.message = None;
}

fn start_retry(ctx: &mut EditorContext<'_>, g: &mut GameUi, retry: Retry) {
    let Some(root) = g.root.clone() else { return };
    let id = retry.expected.id.clone();
    let title = ctx.tf(
        "gplug.installing",
        &[("name", &retry.name), ("version", &retry.expected.version)],
    );
    g.task = Some((
        id,
        ctx.state().spawn_task(title, None, move |_| {
            Ok(install_file(&root, &retry, OnConflict::Backup))
        }),
    ));
    g.message = None;
}

fn uninstall(ctx: &mut EditorContext<'_>, g: &mut GameUi, id: &str, on_conflict: OnConflict) {
    let Some(root) = g.root.clone() else { return };
    let id = id.to_owned();
    let title = ctx.tf("gplug.uninstalling", &[("id", &id)]);
    g.task = Some((
        id.clone(),
        ctx.state().spawn_task(title, None, move |_| {
            Ok(match project::uninstall(&root, &id, on_conflict) {
                Ok(out) => {
                    let manifest = update_requirements(&root, &[], &out.plugin_ids);
                    Ok(Done::Uninstalled(out, manifest))
                }
                Err(e) => Err(Failure::Install(e)),
            })
        }),
    ));
    g.message = None;
}

/// The confirmation (third-party source, replaced install, conflict) and
/// the "replace with a backup" card after a conflict.
fn confirm_cards(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &SharedPlugins, g: &mut GameUi) {
    if let Some(c) = &g.confirm {
        let mut decision = None;
        let trust = c.plan.source.trust;
        card(ui, |ui| {
            ui.colored_label(
                warning(ui),
                RichText::new(format!(
                    "{} {}",
                    icons::WARNING,
                    ctx.t("plugins.warning_title")
                ))
                .strong(),
            );
            ui.label(ctx.tf(
                "gplug.confirm.text",
                &[
                    ("name", &c.name),
                    ("version", &c.plan.version.version),
                    ("source", &c.plan.source.name),
                ],
            ));
            ui.label(ctx.t("gplug.confirm.code"));
            match trust {
                Trust::Unsigned => {
                    ui.colored_label(danger(ui), ctx.t("store.confirm.unsigned"));
                }
                Trust::Trusted => {
                    ui.colored_label(warning(ui), ctx.t("store.confirm.third_party"));
                }
                Trust::Official => {}
            }
            if let Some(from) = &c.replaces {
                ui.colored_label(
                    warning(ui),
                    ctx.tf("store.confirm.replaces", &[("source", from)]),
                );
            }
            if let Some(conflict) = &c.conflict {
                ui.colored_label(danger(ui), describe_conflict(ctx, conflict));
                if let Conflict::Modified { changes, .. } = conflict {
                    changes_list(ui, changes);
                }
            }
            ui.horizontal(|ui| {
                let label = if c.conflict.is_some() {
                    "gplug.replace_backup"
                } else {
                    "plugins.install_confirm"
                };
                if primary_button(ui, &ctx.t(label)).clicked() {
                    decision = Some(true);
                }
                if secondary_button(ui, &ctx.t("plugins.cancel")).clicked() {
                    decision = Some(false);
                }
            });
        });
        match decision {
            Some(true) => {
                if let Some(c) = g.confirm.take() {
                    let on_conflict = if c.conflict.is_some() {
                        OnConflict::Backup
                    } else {
                        OnConflict::Refuse
                    };
                    start_install(ctx, shared, g, c.plan, &c.name, on_conflict);
                }
            }
            Some(false) => g.confirm = None,
            None => {}
        }
    }
    if let Some((conflict, retry)) = &g.conflict {
        let mut decision = None;
        card(ui, |ui| {
            ui.colored_label(
                danger(ui),
                RichText::new(describe_conflict(ctx, conflict)).strong(),
            );
            if let Conflict::Modified { changes, .. } = conflict {
                changes_list(ui, changes);
            }
            ui.label(ctx.t("gplug.conflict.explain"));
            ui.horizontal(|ui| {
                if primary_button(ui, &ctx.t("gplug.replace_backup")).clicked() {
                    decision = Some(true);
                }
                if secondary_button(ui, &ctx.t("plugins.cancel")).clicked() {
                    decision = Some(false);
                }
            });
        });
        match decision {
            Some(true) => {
                let retry = retry.clone();
                g.conflict = None;
                start_retry(ctx, g, retry);
            }
            Some(false) => {
                let _ = std::fs::remove_file(&retry.file);
                g.conflict = None;
                g.message = None;
            }
            None => {}
        }
    }
}

fn changes_list(ui: &mut Ui, changes: &[Change]) {
    let shown: Vec<String> = changes.iter().take(8).map(ToString::to_string).collect();
    let mut text = shown.join("\n");
    if changes.len() > shown.len() {
        text.push_str(&format!("\n… (+{})", changes.len() - shown.len()));
    }
    ui.label(RichText::new(text).small().monospace().color(weak(ui)));
}

// ------------------------------------------------------------------ store tab

/// The "Editor plugins / Game plugins" switch of the Store tab; `true` when
/// the game plugins are shown.
pub fn store_switch(ui: &mut Ui, ctx: &EditorContext<'_>, g: &mut GameUi) -> bool {
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut g.kind,
            StoreKind::Editor,
            format!("{} {}", icons::PUZZLE_PIECE, ctx.t("gplug.kind.editor")),
        );
        ui.selectable_value(
            &mut g.kind,
            StoreKind::Game,
            format!("{} {}", icons::GAME_CONTROLLER, ctx.t("gplug.kind.game")),
        );
    });
    g.kind == StoreKind::Game
}

/// The game plugins of the enabled catalogs.
pub fn store_view(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    g: &mut GameUi,
) {
    g.ensure_loaded(ctx, shared, false);
    let Some(info) = ctx.project_info().cloned() else {
        ui.label(RichText::new(ctx.t("gplug.no_project")).color(weak(ui)));
        return;
    };
    ui.label(RichText::new(ctx.tf("gplug.intro", &[("project", &info.name)])).color(weak(ui)));
    ui.label(
        RichText::new(ctx.tf(
            "gplug.engine",
            &[
                ("version", &ENGINE_VERSION),
                ("api", &orbit_plugin_api::API_VERSION),
            ],
        ))
        .small()
        .color(weak(ui)),
    );
    toolchain_line(ui, ctx, g);
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut g.search)
                .hint_text(format!(
                    "{} {}",
                    icons::MAGNIFYING_GLASS,
                    ctx.t("store.search")
                ))
                .desired_width(220.0),
        );
        let refreshing = g.refresh.is_some();
        if ui
            .add_enabled(
                !refreshing,
                egui::Button::new(format!(
                    "{} {}",
                    icons::ARROW_CLOCKWISE,
                    ctx.t("plugins.refresh")
                )),
            )
            .clicked()
            && let Some(paths) = paths_of(shared)
        {
            let sources: Vec<SourceEntry> = Sources::load(&paths).enabled().cloned().collect();
            g.refresh = Some(ctx.state().spawn_task(
                ctx.t("store.refreshing"),
                None,
                move |progress| {
                    for s in &sources {
                        progress.check()?;
                        let _ = fetch::fetch(&paths, s, true, fetch::MAX_AGE);
                    }
                    Ok(())
                },
            ));
        }
        if refreshing {
            ui.spinner();
        }
    });
    task_progress(ui, ctx, g);
    confirm_cards(ui, ctx, shared, g);
    show_message(ui, g);
    if !g.warnings.is_empty() {
        ui.collapsing(
            ctx.tf("gplug.warnings", &[("count", &g.warnings.len())]),
            |ui| {
                for w in &g.warnings {
                    ui.label(RichText::new(w).small().color(weak(ui)));
                }
            },
        );
    }
    ui.separator();
    let rows: Vec<(SourceEntry, GamePlugin)> = {
        let mut rows: Vec<(SourceEntry, GamePlugin)> = g
            .games
            .iter()
            .flat_map(|(s, games)| {
                games
                    .iter()
                    .filter(|p| p.matches(&g.search))
                    .map(move |p| (s.clone(), p.clone()))
            })
            .collect();
        rows.sort_by(|a, b| {
            (a.0.trust(), a.1.name.to_lowercase()).cmp(&(b.0.trust(), b.1.name.to_lowercase()))
        });
        rows
    };
    if rows.is_empty() {
        let key = if g.games.is_empty() {
            "gplug.none_in_catalogs"
        } else {
            "store.no_match"
        };
        ui.label(RichText::new(ctx.t(key)).color(weak(ui)));
        return;
    }
    let updates = g.updates();
    let lang = ctx.lang().code();
    let busy = g.task.is_some();
    let mut install: Option<(String, String)> = None;
    ScrollArea::vertical()
        .id_salt("game_store_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (source, plugin) in &rows {
                let installed = g.installed_of(&plugin.id);
                let modified = installed.is_some_and(|i| g.changes.contains_key(&i.dir));
                let manual = g
                    .installed
                    .iter()
                    .any(|i| i.record.is_none() && i.provides(&plugin.id));
                card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&plugin.name).strong().size(15.0));
                        game_badge(ui, ctx);
                        trust_badge(ui, ctx, source.trust());
                        ui.label(
                            RichText::new(ctx.tf("gplug.from_source", &[("source", &source.name)]))
                                .small()
                                .color(weak(ui)),
                        );
                    });
                    ui.label(
                        RichText::new(&plugin.id)
                            .small()
                            .monospace()
                            .color(weak(ui)),
                    );
                    let description = plugin.description.get(lang);
                    if !description.is_empty() {
                        ui.label(description);
                    }
                    let availability = game_catalog::select_game(
                        plugin,
                        ENGINE_VERSION,
                        orbit_plugin_api::API_VERSION,
                    );
                    match availability {
                        GameAvailability::Available {
                            version,
                            newer_elsewhere,
                        } => {
                            ui.label(
                                RichText::new(ctx.tf(
                                    "gplug.version_line",
                                    &[
                                        ("version", &version.version),
                                        ("api", &version.api_version),
                                        (
                                            "range",
                                            &engine_range(
                                                ctx,
                                                &version.min_engine,
                                                version.max_engine.as_deref(),
                                            ),
                                        ),
                                    ],
                                ))
                                .small()
                                .color(weak(ui)),
                            );
                            if let Some(n) = newer_elsewhere {
                                ui.label(
                                    RichText::new(ctx.tf(
                                        "gplug.newer_elsewhere",
                                        &[
                                            ("version", &n.version),
                                            (
                                                "range",
                                                &engine_range(
                                                    ctx,
                                                    &n.min_engine,
                                                    n.max_engine.as_deref(),
                                                ),
                                            ),
                                        ],
                                    ))
                                    .small()
                                    .color(weak(ui)),
                                );
                            }
                            let changelog = version.changelog.get(lang);
                            if !changelog.is_empty() {
                                ui.label(RichText::new(changelog).small());
                            }
                            ui.horizontal_wrapped(|ui| {
                                let state = installed.and_then(|i| i.record.as_ref());
                                let can = !busy;
                                match state {
                                    Some(r)
                                        if updates
                                            .get(&plugin.id)
                                            .is_some_and(|u| u.source == source.id) =>
                                    {
                                        badge(ui, &ctx.t("store.update_badge"), accent(ui));
                                        if ui
                                            .add_enabled(
                                                can,
                                                egui::Button::new(format!(
                                                    "{} {}",
                                                    icons::ARROW_CLOCKWISE,
                                                    ctx.tf(
                                                        "gplug.update_to",
                                                        &[("version", &version.version)]
                                                    )
                                                )),
                                            )
                                            .on_hover_text(ctx.tf(
                                                "gplug.installed_version",
                                                &[("version", &r.version)],
                                            ))
                                            .clicked()
                                        {
                                            install = Some((source.id.clone(), plugin.id.clone()));
                                        }
                                    }
                                    Some(r) => {
                                        ui.colored_label(
                                            success(),
                                            format!(
                                                "{} {}",
                                                icons::CHECK,
                                                ctx.tf(
                                                    "gplug.installed_version",
                                                    &[("version", &r.version)]
                                                )
                                            ),
                                        );
                                    }
                                    None => {
                                        if ui
                                            .add_enabled(
                                                can,
                                                egui::Button::new(format!(
                                                    "{} {}",
                                                    icons::DOWNLOAD_SIMPLE,
                                                    ctx.t("gplug.install")
                                                )),
                                            )
                                            .clicked()
                                        {
                                            install = Some((source.id.clone(), plugin.id.clone()));
                                        }
                                    }
                                }
                                if modified {
                                    ui.colored_label(warning(ui), ctx.t("gplug.modified_badge"));
                                }
                                if manual {
                                    ui.colored_label(warning(ui), ctx.t("gplug.manual_same_id"));
                                }
                            });
                        }
                        GameAvailability::Incompatible { latest } => {
                            let text = match latest {
                                Some(l) => ctx.tf(
                                    "gplug.incompatible",
                                    &[
                                        ("version", &l.version),
                                        ("api", &l.api_version),
                                        (
                                            "range",
                                            &engine_range(
                                                ctx,
                                                &l.min_engine,
                                                l.max_engine.as_deref(),
                                            ),
                                        ),
                                    ],
                                ),
                                None => ctx.t("gplug.no_version"),
                            };
                            ui.colored_label(warning(ui), text);
                        }
                    }
                });
                ui.add_space(6.0);
            }
        });
    if let Some((source, plugin)) = install {
        request_install(ctx, shared, g, &source, &plugin);
    }
}

// ------------------------------------------------------------------ installed tab

/// "Game plugins of this project" in the Installed tab.
pub fn installed_section(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    g: &mut GameUi,
) {
    g.ensure_loaded(ctx, shared, false);
    let Some(info) = ctx.project_info().cloned() else {
        return;
    };
    let title = format!(
        "{} {}",
        icons::GAME_CONTROLLER,
        ctx.tf(
            "gplug.section",
            &[("project", &info.name), ("count", &g.installed.len())]
        )
    );
    egui::CollapsingHeader::new(title)
        .id_salt("game_plugins_section")
        .default_open(!g.installed.is_empty())
        .show(ui, |ui| {
            toolchain_line(ui, ctx, g);
            task_progress(ui, ctx, g);
            confirm_cards(ui, ctx, shared, g);
            show_message(ui, g);
            if g.installed.is_empty() {
                ui.label(RichText::new(ctx.t("gplug.none_installed")).color(weak(ui)));
                return;
            }
            let updates = g.updates();
            let loaded: BTreeSet<String> = ctx
                .registry()
                .project
                .plugins()
                .iter()
                .map(|m| m.id.clone())
                .collect();
            let used = used_plugins(ctx);
            let mut act: Option<Act> = None;
            ScrollArea::vertical()
                .id_salt("game_plugins_list")
                .max_height(320.0)
                .show(ui, |ui| {
                    for game in &g.installed {
                        game_card(ui, ctx, g, game, &updates, &loaded, &used, &mut act);
                        ui.add_space(4.0);
                    }
                });
            match act {
                Some(Act::Update(source, id)) => request_install(ctx, shared, g, &source, &id),
                Some(Act::AskUninstall(id)) => g.confirm_uninstall = Some(id),
                Some(Act::Uninstall(id, on_conflict)) => {
                    g.confirm_uninstall = None;
                    uninstall(ctx, g, &id, on_conflict);
                }
                Some(Act::CancelUninstall) => g.confirm_uninstall = None,
                Some(Act::Open(dir)) => open_folder(&dir),
                None => {}
            }
        });
}

enum Act {
    Update(String, String),
    AskUninstall(String),
    Uninstall(String, OnConflict),
    CancelUninstall,
    Open(PathBuf),
}

/// Plugin ids the open story's nodes use, with their node count.
fn used_plugins(ctx: &EditorContext<'_>) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    if let Some(story) = ctx.story() {
        for node in story.scenes.iter().flat_map(|s| &s.nodes) {
            if let orbit_core::NodeKind::Custom { plugin_id, .. } = &node.kind {
                *out.entry(plugin_id.clone()).or_insert(0) += 1;
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn game_card(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    g: &GameUi,
    game: &InstalledGame,
    updates: &BTreeMap<String, game_catalog::GameUpdate>,
    loaded: &BTreeSet<String>,
    used: &BTreeMap<String, usize>,
    act: &mut Option<Act>,
) {
    let record = game.record.as_ref();
    let name = record
        .map(|r| r.name.clone())
        .or_else(|| game.crate_name.clone())
        .unwrap_or_else(|| game.dir_name.clone());
    let version = record
        .map(|r| r.version.clone())
        .or_else(|| game.version.clone())
        .unwrap_or_default();
    let ids: Vec<String> = record
        .map(|r| r.plugin_ids.clone())
        .unwrap_or_else(|| game.plugin_ids.clone());
    let changes = g.changes.get(&game.dir);
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&name).strong());
            ui.label(RichText::new(&version).color(weak(ui)));
            game_badge(ui, ctx);
            match record {
                Some(r) => {
                    trust_badge(ui, ctx, r.source.trust);
                    ui.label(
                        RichText::new(ctx.tf("gplug.from_source", &[("source", &r.source.name)]))
                            .small()
                            .color(weak(ui)),
                    );
                }
                None => {
                    ui.label(
                        RichText::new(ctx.t("gplug.by_hand"))
                            .small()
                            .color(weak(ui)),
                    );
                }
            }
        });
        ui.label(
            RichText::new(format!("plugins/{}  ·  {}", game.dir_name, ids.join(", ")))
                .small()
                .monospace()
                .color(weak(ui)),
        );
        if let Some(e) = &game.error {
            ui.colored_label(danger(ui), e);
        }
        if let Some(changes) = changes {
            ui.colored_label(
                warning(ui),
                ctx.tf("gplug.modified", &[("count", &changes.len())]),
            )
            .on_hover_text(
                changes
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        // The editor half.
        for id in &ids {
            let (color, text) = if loaded.contains(id) {
                (success(), ctx.tf("gplug.editor_part.loaded", &[("id", id)]))
            } else if g.editor_ids.contains(id) {
                (
                    warning(ui),
                    ctx.tf("gplug.editor_part.in_store", &[("id", id)]),
                )
            } else {
                (weak(ui), ctx.tf("gplug.editor_part.none", &[("id", id)]))
            };
            ui.label(RichText::new(text).small().color(color));
        }
        if let Some(r) = record {
            if r.engine_path.is_none() {
                ui.colored_label(
                    warning(ui),
                    RichText::new(ctx.t("gplug.engine_not_rewritten")).small(),
                );
            } else if let (Some(Ok(engine)), Some(path)) =
                (g.toolchain.as_ref().map(|t| &t.engine), &r.engine_path)
                && !same_path(engine, Path::new(path))
            {
                ui.colored_label(
                    warning(ui),
                    RichText::new(ctx.tf(
                        "gplug.engine_moved",
                        &[("path", path), ("engine", &engine.display())],
                    ))
                    .small(),
                );
            }
            if r.build_script || r.proc_macro {
                ui.label(
                    RichText::new(ctx.t("gplug.build_code"))
                        .small()
                        .color(weak(ui)),
                );
            }
        }
        ui.horizontal_wrapped(|ui| {
            if let (Some(r), Some(u)) = (
                record,
                updates.get(&game.store_id().unwrap_or_default().to_owned()),
            ) && ui
                .add_enabled(
                    g.task.is_none(),
                    egui::Button::new(format!(
                        "{} {}",
                        icons::ARROW_CLOCKWISE,
                        ctx.tf("gplug.update_to", &[("version", &u.available)])
                    )),
                )
                .clicked()
            {
                *act = Some(Act::Update(u.source.clone(), r.id.clone()));
            }
            if ui
                .button(format!(
                    "{} {}",
                    icons::FOLDER_OPEN,
                    ctx.t("gplug.open_folder")
                ))
                .clicked()
            {
                *act = Some(Act::Open(game.dir.clone()));
            }
            if let Some(r) = record {
                if g.confirm_uninstall.as_deref() == Some(r.id.as_str()) {
                    let nodes: usize = ids.iter().filter_map(|id| used.get(id)).sum();
                    if nodes > 0 {
                        ui.colored_label(
                            danger(ui),
                            ctx.tf("gplug.uninstall_used", &[("count", &nodes)]),
                        );
                    }
                    if changes.is_some() {
                        ui.colored_label(warning(ui), ctx.t("gplug.uninstall_backup"));
                    }
                    let on_conflict = if changes.is_some() {
                        OnConflict::Backup
                    } else {
                        OnConflict::Refuse
                    };
                    if ui.button(ctx.t("plugins.uninstall_yes")).clicked() {
                        *act = Some(Act::Uninstall(r.id.clone(), on_conflict));
                    }
                    if ui.button(ctx.t("plugins.cancel")).clicked() {
                        *act = Some(Act::CancelUninstall);
                    }
                } else if ui
                    .add_enabled(
                        g.task.is_none(),
                        egui::Button::new(format!(
                            "{} {}",
                            icons::TRASH,
                            ctx.t("plugins.uninstall")
                        )),
                    )
                    .clicked()
                {
                    *act = Some(Act::AskUninstall(r.id.clone()));
                }
            }
        });
    });
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.display().to_string();
        s.strip_prefix(r"\\?\")
            .unwrap_or(&s)
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_are_set_and_removed() {
        let mut m = ProjectManifest::new("Game");
        m.plugins.push(PluginRequirement {
            id: "acme.old".into(),
            version: "0.1.0".into(),
        });
        m.plugins.push(PluginRequirement {
            id: "acme.dice".into(),
            version: "1.0.0".into(),
        });
        apply_requirements(
            &mut m,
            &[
                ("acme.dice".into(), "1.1.0".into()),
                ("acme.cards".into(), "1.1.0".into()),
            ],
            &["acme.old".into()],
        );
        let got: Vec<(&str, &str)> = m
            .plugins
            .iter()
            .map(|r| (r.id.as_str(), r.version.as_str()))
            .collect();
        assert_eq!(got, [("acme.cards", "1.1.0"), ("acme.dice", "1.1.0")]);
    }

    #[test]
    fn the_manifest_file_is_updated_and_old_projects_still_load() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp/orbit_editor_game_plugins_manifest");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // No manifest (bare story folder): nothing written.
        assert_eq!(
            update_requirements(&root, &[("a.b".into(), "1.0.0".into())], &[]),
            Ok(false)
        );
        assert!(!root.join(orbit_project::MANIFEST_FILE).exists());
        // A manifest written before plugins were listed.
        std::fs::write(
            root.join(orbit_project::MANIFEST_FILE),
            "(format: 1, name: \"x\", story: \"story.orbit\")",
        )
        .unwrap();
        assert_eq!(
            update_requirements(&root, &[("a.b".into(), "1.0.0".into())], &[]),
            Ok(true)
        );
        let m = orbit_project::load_manifest(&FsStorage::new(&root)).unwrap();
        assert_eq!(
            m.plugins,
            [PluginRequirement {
                id: "a.b".into(),
                version: "1.0.0".into()
            }]
        );
        assert_eq!(update_requirements(&root, &[], &["a.b".into()]), Ok(true));
        assert!(
            orbit_project::load_manifest(&FsStorage::new(&root))
                .unwrap()
                .plugins
                .is_empty()
        );
        assert!(same_path(Path::new(r"\\?\E:\A\b\"), Path::new("e:/a/B")));
    }
}
