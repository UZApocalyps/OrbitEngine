//! The plugin manager (Tools > Extensions > Plugins), three tabs:
//!
//! * Installed: the native plugins installed in `<config>/plugins/`, their
//!   state, install from a file (`.orbitplugin` package or a bare library),
//!   enable / disable (next start), uninstall, settings pages, restart, and
//!   the updates the catalogs offer;
//! * Store and Sources: see `builtin::plugin_store`.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use egui::{Color32, RichText, ScrollArea, Ui};
use orbit_editor_api::native::{Fingerprint, FingerprintField};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, Level, PanelDef,
    Plugin, PluginManifest, TaskError, TaskHandle, WindowDef, icons,
};
use orbit_plugin_host::binary::{BinaryError, library_extension};
use orbit_plugin_host::{
    EditorFacts, InstallError, Installed, InstalledPlugin, PACKAGE_EXTENSION, PluginManager, Problem,
    ProcessProber, RangeError,
};

use super::plugin_store::{self, StoreUi};
use super::ui_util::{card, danger, primary_button, secondary_button, success, warning, weak};
use crate::plugins::SharedPlugins;

pub const PLUGIN_ID: &str = "orbit.plugins";
/// Full id of the manager panel.
pub const PANEL: &str = "orbit.plugins.manager";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Installed,
    Store,
    Sources,
}

#[derive(Default)]
struct ManagerUi {
    tab: Tab,
    /// The panel was just shown (cached catalogs, background refresh).
    shown: bool,
    store: StoreUi,
    /// The plugin folders, read again when the panel shows or after a change.
    list: Option<Vec<InstalledPlugin>>,
    /// A file picked, waiting for the security confirmation.
    pending_install: Option<PathBuf>,
    installing: Option<TaskHandle<Result<Installed, InstallError>>>,
    confirm_uninstall: Option<String>,
    /// The plugin whose settings window is open.
    settings_for: Option<String>,
    message: Option<(Level, String)>,
}

pub struct PluginsPlugin {
    shared: SharedPlugins,
    ui: Rc<RefCell<ManagerUi>>,
}

impl PluginsPlugin {
    pub fn new(shared: SharedPlugins) -> Self {
        Self { shared, ui: Rc::default() }
    }
}

impl Plugin for PluginsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(PLUGIN_ID, "Plugins", "0.1.0")
    }
}

impl EditorPlugin for PluginsPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        let (shared, ui) = (self.shared.clone(), self.ui.clone());
        let on_shown = self.ui.clone();
        reg.panel(
            PanelDef::new("manager", "plugins.title", icons::PUZZLE_PIECE, DockSlot::Hidden, move |u, c| {
                panel(u, c, &shared, &mut ui.borrow_mut());
            })
            .on_shown(move |_| {
                let mut m = on_shown.borrow_mut();
                m.list = None;
                m.shown = true;
            }),
        );
        reg.action(
            ActionDef::new("open", "plugins.open", |c| {
                c.request(EditorRequest::ShowPanel(PANEL.to_owned()));
            })
            .icon(icons::PUZZLE_PIECE)
            .category("category.tools")
            .menu("tools", 800)
            .menu_group("menu.extensions")
            .menu_label("plugins.menu"),
        );
        let ui = self.ui.clone();
        reg.window(WindowDef::new("settings", move |ctx, c| settings_window(ctx, c, &ui)));
    }
}

// ------------------------------------------------------------------ texts

fn field_name(ctx: &EditorContext<'_>, field: FingerprintField) -> String {
    ctx.t(&format!("plugins.field.{}", field.key()))
}

fn describe_binary(ctx: &EditorContext<'_>, e: &BinaryError) -> String {
    match e {
        BinaryError::Unreadable(d) => ctx.tf("plugins.binary.unreadable", &[("detail", d)]),
        BinaryError::NotALibrary => ctx.t("plugins.binary.not_library"),
        BinaryError::WrongPlatform { found, expected } => {
            ctx.tf("plugins.binary.wrong_platform", &[("found", found), ("expected", expected)])
        }
        BinaryError::WrongArch { found, expected } => {
            ctx.tf("plugins.binary.wrong_arch", &[("found", found), ("expected", expected)])
        }
        BinaryError::NotDynamic => ctx.t("plugins.binary.not_dynamic"),
    }
}

/// The reason a plugin is not running, in the interface language.
pub fn describe_problem(ctx: &EditorContext<'_>, p: &Problem) -> String {
    match p {
        Problem::Binary(e) => describe_binary(ctx, e),
        Problem::ProbeFailed(d) => ctx.tf("plugins.problem.probe", &[("detail", d)]),
        Problem::BadDescriptor(d) => ctx.tf("plugins.problem.descriptor", &[("detail", d)]),
        Problem::Incompatible(mismatches) => {
            let editor = Fingerprint::current();
            let of = |f: FingerprintField, own: &str| {
                mismatches
                    .iter()
                    .find(|m| m.field == f)
                    .map_or_else(|| own.to_owned(), |m| m.plugin.clone())
            };
            let details: Vec<String> = mismatches
                .iter()
                .map(|m| format!("{} {} ≠ {}", field_name(ctx, m.field), m.plugin, m.editor))
                .collect();
            ctx.tf(
                "plugins.problem.incompatible",
                &[
                    ("plugin_editor", &of(FingerprintField::ApiVersion, &editor.api_version)),
                    ("plugin_rustc", &of(FingerprintField::Rustc, &editor.rustc)),
                    ("editor", &editor.api_version),
                    ("rustc", &editor.rustc),
                    ("details", &details.join(" ; ")),
                ],
            )
        }
        Problem::EditorRange(RangeError::TooOld { min }) => {
            ctx.tf("plugins.problem.too_old", &[("min", min), ("editor", &orbit_editor_api::native::API_VERSION)])
        }
        Problem::EditorRange(RangeError::TooNew { max }) => {
            ctx.tf("plugins.problem.too_new", &[("max", max), ("editor", &orbit_editor_api::native::API_VERSION)])
        }
        Problem::EditorRange(RangeError::Invalid(v)) => ctx.tf("plugins.problem.bad_range", &[("value", v)]),
        Problem::IdMismatch { manifest, library } => {
            ctx.tf("plugins.problem.id_mismatch", &[("manifest", manifest), ("library", library)])
        }
        Problem::InvalidId(id) => ctx.tf("plugins.problem.invalid_id", &[("id", id)]),
        Problem::DuplicateId(id) => ctx.tf("plugins.problem.duplicate", &[("id", id)]),
        Problem::Modified => ctx.t("plugins.problem.modified"),
        Problem::CrashedAtLoad => ctx.t("plugins.problem.crashed_at_load"),
        Problem::LoadFailed(d) => ctx.tf("plugins.problem.load_failed", &[("detail", d)]),
        Problem::InitFailed(d) => ctx.tf("plugins.problem.init_failed", &[("detail", d)]),
        Problem::Panicked(d) => ctx.tf("plugins.problem.panicked", &[("detail", d)]),
        Problem::Missing(d) => ctx.tf("plugins.problem.missing", &[("detail", d)]),
    }
}

pub(super) fn describe_install_error(ctx: &EditorContext<'_>, e: &InstallError) -> String {
    match e {
        InstallError::Unsupported(f) => ctx.tf("plugins.install.unsupported", &[("file", f)]),
        InstallError::Io(d) => ctx.tf("plugins.install.io", &[("detail", d)]),
        InstallError::Package(d) => ctx.tf("plugins.install.package", &[("detail", d)]),
        InstallError::Rejected(p) => ctx.tf("plugins.install.rejected", &[("reason", &describe_problem(ctx, p))]),
        InstallError::Reserved(id) => ctx.tf("plugins.install.reserved", &[("id", id)]),
    }
}

// ------------------------------------------------------------------ state

/// What the manager shows for one plugin.
enum Status {
    Running,
    /// Enabled; runs from the next start (installed or enabled meanwhile).
    NextStart,
    Disabled,
    Removing,
    Problem(Problem),
    /// Disabled after a crash (message kept in the store).
    Crashed(String),
}

fn status_of(plugin: &InstalledPlugin, shared: &crate::plugins::PluginsState, editor: &EditorFacts) -> Status {
    let id = plugin.id();
    if plugin.record.uninstall {
        return Status::Removing;
    }
    if let Some(p) = &plugin.problem {
        return Status::Problem(p.clone());
    }
    if let Some(failure) = shared.failure(id) {
        return Status::Problem(Problem::Panicked(failure));
    }
    if let Some(p) = shared.problems.get(id) {
        return Status::Problem(p.clone());
    }
    if let Some(e) = &plugin.record.error {
        return Status::Crashed(e.clone());
    }
    // A cached probe of this very editor build tells compatibility even for
    // a disabled plugin.
    if let Some(cache) = &plugin.record.probe
        && cache.key.starts_with(&editor.cache_key())
        && let Err(p) = orbit_plugin_host::verify::verify(&cache.info, Some(&plugin.manifest), editor)
    {
        return Status::Problem(p);
    }
    match (plugin.record.enabled, shared.is_loaded(id)) {
        (true, true) => Status::Running,
        (true, false) => Status::NextStart,
        (false, _) => Status::Disabled,
    }
}

// ------------------------------------------------------------------ actions

fn pick_file(ctx: &EditorContext<'_>) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title(ctx.t("plugins.pick_title"))
        .add_filter(ctx.t("plugins.filter"), &[PACKAGE_EXTENSION, library_extension()])
        .pick_file()
}

fn open_folder(path: &std::path::Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn start_install(ctx: &mut EditorContext<'_>, shared: &SharedPlugins, m: &mut ManagerUi, file: PathBuf) {
    let state = shared.borrow();
    let Some(config_dir) = state.config_dir.clone() else { return };
    let reserved: BTreeSet<String> = state.builtin_ids.clone();
    let loaded: BTreeSet<String> = state.loaded.iter().map(|p| p.id.clone()).collect();
    drop(state);
    let title = ctx.t("plugins.installing");
    m.installing = Some(ctx.state().spawn_task(title, None, move |_| {
        let mut manager = PluginManager::for_config_dir(&config_dir, EditorFacts::current());
        let prober = ProcessProber::current_exe()
            .ok_or_else(|| TaskError::Failed("the editor executable was not found".into()))?;
        Ok(manager.install(&file, &prober, &|id| reserved.contains(id), &|id| loaded.contains(id)))
    }));
}

fn poll_install(ctx: &mut EditorContext<'_>, shared: &SharedPlugins, m: &mut ManagerUi) {
    let Some(handle) = m.installing.as_mut() else { return };
    let Some(outcome) = handle.poll() else { return };
    m.installing = None;
    m.list = None;
    m.message = Some(match outcome {
        Ok(Ok(done)) => {
            shared.borrow_mut().restart_needed = true;
            plugin_store::forget_origin(shared, &mut m.store, &done.id);
            let key = if done.staged || done.updated { "plugins.updated" } else { "plugins.installed" };
            (Level::Info, ctx.tf(key, &[("name", &done.name), ("version", &done.version)]))
        }
        Ok(Err(e)) => (Level::Error, describe_install_error(ctx, &e)),
        Err(TaskError::Failed(e)) => (Level::Error, e),
        Err(_) => (Level::Warn, ctx.t("plugins.install_cancelled")),
    });
    if let Some((level, text)) = &m.message {
        ctx.log(*level, text.clone());
    }
}

// ------------------------------------------------------------------ panel

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &SharedPlugins, m: &mut ManagerUi) {
    poll_install(ctx, shared, m);
    if plugin_store::poll(ctx, shared, &mut m.store) {
        m.list = None;
    }
    if std::mem::take(&mut m.shown) {
        plugin_store::on_show(ctx, shared, &mut m.store);
    }
    let Some(mut manager) = shared.borrow().manager() else {
        ui.label(RichText::new(ctx.t("plugins.no_config")).color(weak(ui)));
        return;
    };
    let list = m.list.get_or_insert_with(|| manager.installed()).clone();
    let editor = manager.editor().clone();

    ui.heading(ctx.t("plugins.title"));
    ui.label(RichText::new(ctx.tf("plugins.intro", &[("version", &editor.version)])).color(weak(ui)));
    {
        let state = shared.borrow();
        if state.safe_mode {
            ui.colored_label(warning(ui), ctx.t("plugins.safe_mode"));
        }
        if let Some(id) = &state.crashed_at_load {
            ui.colored_label(danger(ui), ctx.tf("plugins.crashed_at_load", &[("id", id)]));
        }
    }
    if shared.borrow().restart_needed {
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(warning(ui), format!("{} {}", icons::ARROW_CLOCKWISE, ctx.t("plugins.restart_needed")));
                if primary_button(ui, &ctx.t("plugins.restart")).clicked() {
                    ctx.request(EditorRequest::Restart);
                }
            });
        });
    }
    let updates = m.store.updates(&list);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let installed = if updates.is_empty() {
            ctx.tf("store.tab.installed", &[("count", &list.len())])
        } else {
            ctx.tf("store.tab.installed_updates", &[("count", &list.len()), ("updates", &updates.len())])
        };
        ui.selectable_value(&mut m.tab, Tab::Installed, format!("{} {installed}", icons::PUZZLE_PIECE));
        ui.selectable_value(&mut m.tab, Tab::Store, format!("{} {}", icons::STOREFRONT, ctx.t("store.tab.store")));
        ui.selectable_value(&mut m.tab, Tab::Sources, format!("{} {}", icons::LIST, ctx.t("store.tab.sources")));
    });
    ui.separator();
    match m.tab {
        Tab::Installed => {}
        Tab::Store => {
            plugin_store::store_tab(ui, ctx, shared, &mut m.store, &list);
            return;
        }
        Tab::Sources => {
            plugin_store::sources_tab(ui, ctx, shared, &mut m.store);
            return;
        }
    }
    ui.horizontal_wrapped(|ui| {
        let busy = m.installing.is_some() || m.store.is_busy();
        if ui
            .add_enabled(!busy, egui::Button::new(format!("{} {}", icons::PLUS, ctx.t("plugins.install"))))
            .clicked()
            && let Some(file) = pick_file(ctx)
        {
            m.pending_install = Some(file);
        }
        if secondary_button(ui, &format!("{} {}", icons::FOLDER_OPEN, ctx.t("plugins.open_folder"))).clicked() {
            open_folder(manager.root());
        }
        if secondary_button(ui, &ctx.t("plugins.refresh")).clicked() {
            m.list = None;
        }
        if busy {
            ui.spinner();
            ui.label(ctx.t("plugins.installing"));
        }
    });
    ui.label(RichText::new(ctx.t("plugins.trust_note")).small().color(weak(ui)));

    if let Some(file) = m.pending_install.clone() {
        card(ui, |ui| {
            ui.colored_label(warning(ui), RichText::new(format!("{} {}", icons::WARNING, ctx.t("plugins.warning_title"))).strong());
            ui.label(ctx.tf("plugins.warning", &[("file", &file.display())]));
            ui.horizontal(|ui| {
                if primary_button(ui, &ctx.t("plugins.install_confirm")).clicked() {
                    m.pending_install = None;
                    start_install(ctx, shared, m, file.clone());
                }
                if secondary_button(ui, &ctx.t("plugins.cancel")).clicked() {
                    m.pending_install = None;
                }
            });
        });
    }
    if let Some((level, text)) = &m.message {
        let color = match level {
            Level::Info => success(),
            Level::Warn => warning(ui),
            Level::Error => danger(ui),
        };
        ui.colored_label(color, text);
    }
    plugin_store::installed_header(ui, ctx, shared, &mut m.store);
    ui.separator();

    if list.is_empty() {
        ui.label(RichText::new(ctx.t("plugins.empty")).color(weak(ui)));
        return;
    }
    ScrollArea::vertical().id_salt("plugins_list").auto_shrink([false, false]).show(ui, |ui| {
        for plugin in &list {
            plugin_card(ui, ctx, shared, m, &mut manager, plugin, &editor, &updates, &list);
            ui.add_space(6.0);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn plugin_card(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    m: &mut ManagerUi,
    manager: &mut PluginManager,
    plugin: &InstalledPlugin,
    editor: &EditorFacts,
    updates: &std::collections::BTreeMap<String, orbit_plugin_store::installs::Update>,
    list: &[InstalledPlugin],
) {
    let id = plugin.id().to_owned();
    let status = status_of(plugin, &shared.borrow(), editor);
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&plugin.manifest.name).strong().size(15.0));
            ui.label(RichText::new(&plugin.manifest.version).color(weak(ui)));
            if !plugin.manifest.author.is_empty() {
                ui.label(RichText::new(ctx.tf("plugins.by", &[("author", &plugin.manifest.author)])).color(weak(ui)));
            }
        });
        ui.label(RichText::new(&id).small().monospace().color(weak(ui)));
        if !plugin.manifest.description.is_empty() {
            ui.label(&plugin.manifest.description);
        }
        let (color, text): (Color32, String) = match &status {
            Status::Running => (success(), ctx.t("plugins.status.running")),
            Status::NextStart => (warning(ui), ctx.t("plugins.status.next_start")),
            Status::Disabled => (weak(ui), ctx.t("plugins.status.disabled")),
            Status::Removing => (weak(ui), ctx.t("plugins.status.removing")),
            Status::Crashed(e) => (danger(ui), ctx.tf("plugins.status.crashed", &[("error", e)])),
            Status::Problem(p @ (Problem::Incompatible(_) | Problem::EditorRange(_))) => {
                (danger(ui), format!("{} — {}", ctx.t("plugins.status.incompatible"), describe_problem(ctx, p)))
            }
            Status::Problem(p) => (danger(ui), format!("{} — {}", ctx.t("plugins.status.error"), describe_problem(ctx, p))),
        };
        ui.colored_label(color, text);
        if let Some(update) = updates.get(&id)
            && !matches!(status, Status::Removing)
        {
            plugin_store::update_line(ui, ctx, shared, &mut m.store, update, list);
        }
        ui.horizontal_wrapped(|ui| {
            if !matches!(status, Status::Removing) {
                let mut enabled = plugin.record.enabled;
                if ui.checkbox(&mut enabled, ctx.t("plugins.enabled")).changed() {
                    match manager.set_enabled(&id, enabled) {
                        Ok(()) => {
                            shared.borrow_mut().restart_needed = true;
                            m.list = None;
                        }
                        Err(e) => m.message = Some((Level::Error, e.to_string())),
                    }
                }
            }
            let has_settings = shared.borrow().is_loaded(&id) && ctx.registry().settings_for(&id).is_some();
            if has_settings && secondary_button(ui, &format!("{} {}", icons::GEAR, ctx.t("plugins.settings"))).clicked() {
                m.settings_for = Some(id.clone());
            }
            if m.confirm_uninstall.as_deref() == Some(id.as_str()) {
                ui.colored_label(danger(ui), ctx.t("plugins.uninstall_confirm"));
                if ui.button(ctx.t("plugins.uninstall_yes")).clicked() {
                    let loaded = shared.borrow().is_loaded(&id);
                    match manager.uninstall(&id, loaded) {
                        Ok(()) => {
                            if loaded {
                                shared.borrow_mut().restart_needed = true;
                            }
                            m.message = Some((Level::Info, ctx.tf("plugins.uninstalled", &[("name", &plugin.manifest.name)])));
                        }
                        Err(e) => m.message = Some((Level::Error, e.to_string())),
                    }
                    m.confirm_uninstall = None;
                    m.list = None;
                }
                if ui.button(ctx.t("plugins.cancel")).clicked() {
                    m.confirm_uninstall = None;
                }
            } else if !matches!(status, Status::Removing)
                && ui.button(format!("{} {}", icons::TRASH, ctx.t("plugins.uninstall"))).clicked()
            {
                m.confirm_uninstall = Some(id.clone());
            }
        });
    });
}

fn settings_window(ctx: &egui::Context, ectx: &mut EditorContext<'_>, ui_state: &Rc<RefCell<ManagerUi>>) {
    let Some(id) = ui_state.borrow().settings_for.clone() else { return };
    let registry = ectx.registry();
    let Some(page) = registry.settings_for(&id) else {
        ui_state.borrow_mut().settings_for = None;
        return;
    };
    let mut open = true;
    egui::Window::new(format!("{} — {}", ectx.t("plugins.settings"), ectx.t(&page.title)))
        .id(egui::Id::new("orbit.plugins.settings"))
        .open(&mut open)
        .default_width(460.0)
        .vscroll(true)
        .show(ctx, |ui| (page.ui)(ui, ectx));
    if !open {
        ui_state.borrow_mut().settings_for = None;
    }
}
