//! The plugin store in the plugin manager: the "Store" and "Sources" tabs,
//! and the update badges of the "Installed" tab (`builtin::plugins`).
//!
//! Catalogs come from `orbit_plugin_store` (cached, refreshed in a task);
//! an install downloads the build matching this editor's build id, checks its
//! size, SHA-256 and signature, then hands it to the usual installer
//! (`PluginManager::install`: format, descriptor probe, fingerprint), all in
//! one cancellable editor task. See `docs/plugin-store.md`.

use std::collections::BTreeMap;

use egui::{Color32, RichText, ScrollArea, Ui};
use orbit_editor_api::{EditorContext, Level, TaskError, TaskHandle, icons};
use orbit_plugin_host::{
    EditorFacts, InstallError, Installed, InstalledPlugin, PluginManager, ProcessProber,
};
use orbit_plugin_store::catalog::{self, Availability, CatalogPlugin};
use orbit_plugin_store::download::{self, DownloadError, DownloadPlan};
use orbit_plugin_store::fetch::{self, MAX_AGE, SourceCatalog};
use orbit_plugin_store::installs::{Origins, Update, find_updates};
use orbit_plugin_store::signing::{Ed25519, SignatureError};
use orbit_plugin_store::sources::{SourceError, Sources};
use orbit_plugin_store::{SourceEntry, StorePaths, Trust, now_secs};

use super::ui_util::{
    accent, card, danger, primary_button, secondary_button, success, warning, weak,
};
use crate::plugins::SharedPlugins;

/// This editor build's id, as catalogs list it.
pub fn editor_build_id() -> String {
    orbit_plugin_store::fingerprint_id(&EditorFacts::current().cache_key())
}

/// Why a store install failed.
#[derive(Debug)]
pub enum StoreInstallError {
    Download(DownloadError),
    Install(InstallError),
    NoProber,
}

type Refresh = Vec<(String, Result<SourceCatalog, String>)>;

/// An install waiting for the user's confirmation (third-party source).
struct Confirm {
    plan: DownloadPlan,
    name: String,
    /// The plugin is installed from another source (or from a file).
    replaces: Option<String>,
}

#[derive(Default)]
pub struct StoreUi {
    sources: Option<Sources>,
    origins: Option<Origins>,
    catalogs: BTreeMap<String, Result<SourceCatalog, String>>,
    refreshing: Option<TaskHandle<Refresh>>,
    /// The automatic refresh (catalogs older than an hour) ran this session.
    auto_refreshed: bool,
    search: String,
    source_filter: Option<String>,
    tag_filter: Option<String>,
    /// (source id, plugin id) whose details are open.
    details: Option<(String, String)>,
    installing: Option<(String, TaskHandle<Result<Installed, StoreInstallError>>)>,
    confirm: Option<Confirm>,
    add_url: String,
    add_name: String,
    add_key: String,
    confirm_remove: Option<String>,
    message: Option<(Level, String)>,
    build_id: String,
}

fn paths_of(shared: &SharedPlugins) -> Option<StorePaths> {
    shared
        .borrow()
        .config_dir
        .as_deref()
        .map(StorePaths::for_config_dir)
}

impl StoreUi {
    /// Sources, origins and the cached catalogs, read once (no network).
    fn ensure_loaded(&mut self, paths: &StorePaths) {
        if self.build_id.is_empty() {
            self.build_id = editor_build_id();
        }
        if self.sources.is_none() {
            let sources = Sources::load(paths);
            for s in sources.enabled() {
                if let Some(c) = fetch::cached(paths, s) {
                    self.catalogs.insert(s.id.clone(), Ok(c));
                }
            }
            self.sources = Some(sources);
        }
        if self.origins.is_none() {
            self.origins = Some(Origins::load(paths));
        }
    }

    fn sources(&self) -> &Sources {
        static EMPTY: std::sync::OnceLock<Sources> = std::sync::OnceLock::new();
        self.sources
            .as_ref()
            .unwrap_or_else(|| EMPTY.get_or_init(Sources::default))
    }

    fn save_sources(&mut self, paths: &StorePaths) {
        if let Some(s) = &self.sources
            && let Err(e) = s.save(paths)
        {
            self.message = Some((Level::Error, e.to_string()));
        }
    }

    /// The enabled sources with a catalog.
    fn catalogs(&self) -> Vec<(&SourceEntry, &SourceCatalog)> {
        self.sources()
            .enabled()
            .filter_map(|s| match self.catalogs.get(&s.id) {
                Some(Ok(c)) => Some((s, c)),
                _ => None,
            })
            .collect()
    }

    /// Newer builds for this editor of the installed plugins.
    pub fn updates(&self, installed: &[InstalledPlugin]) -> BTreeMap<String, Update> {
        let Some(origins) = &self.origins else {
            return BTreeMap::new();
        };
        let list: Vec<(String, String)> = installed
            .iter()
            .map(|p| (p.id().to_owned(), p.manifest.version.clone()))
            .collect();
        let catalogs: Vec<(&SourceEntry, &catalog::Catalog)> = self
            .catalogs()
            .into_iter()
            .map(|(s, c)| (s, &c.parsed.catalog))
            .collect();
        find_updates(&list, origins, &catalogs, &self.build_id)
            .into_iter()
            .map(|u| (u.id.clone(), u))
            .collect()
    }

    fn start_refresh(&mut self, ctx: &EditorContext<'_>, paths: &StorePaths, force: bool) {
        if self.refreshing.is_some() {
            return;
        }
        let sources: Vec<SourceEntry> = self.sources().enabled().cloned().collect();
        let paths = paths.clone();
        self.refreshing = Some(ctx.state().spawn_task(
            ctx.t("store.refreshing"),
            None,
            move |progress| {
                let mut out = Vec::new();
                for (i, s) in sources.iter().enumerate() {
                    progress.check()?;
                    progress.set(i as f32 / sources.len().max(1) as f32, s.name.clone());
                    out.push((s.id.clone(), fetch::fetch(&paths, s, force, MAX_AGE)));
                }
                Ok(out)
            },
        ));
    }

    pub fn is_busy(&self) -> bool {
        self.installing.is_some()
    }
}

// ------------------------------------------------------------------ texts

fn trust_label(ctx: &EditorContext<'_>, trust: Trust) -> (String, Color32) {
    let color = match trust {
        Trust::Official => success(),
        Trust::Trusted => Color32::from_rgb(90, 150, 230),
        Trust::Unsigned => Color32::from_rgb(230, 150, 40),
    };
    (ctx.t(&format!("store.trust.{}", trust.key())), color)
}

fn badge(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(color)));
}

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

fn describe_source_error(ctx: &EditorContext<'_>, e: &SourceError) -> String {
    match e {
        SourceError::BadUrl(u) => ctx.tf("store.source.bad_url", &[("url", u)]),
        SourceError::BadKey => ctx.t("store.source.bad_key"),
        SourceError::Duplicate(n) => ctx.tf("store.source.duplicate", &[("name", n)]),
        SourceError::Builtin => ctx.t("store.source.builtin"),
        SourceError::Unknown(id) => id.clone(),
    }
}

fn ago(ctx: &EditorContext<'_>, secs: u64) -> String {
    let d = now_secs().saturating_sub(secs);
    if d < 60 {
        ctx.t("store.ago.now")
    } else if d < 3600 {
        ctx.tf("store.ago.minutes", &[("n", &(d / 60))])
    } else if d < 86_400 {
        ctx.tf("store.ago.hours", &[("n", &(d / 3600))])
    } else {
        ctx.tf("store.ago.days", &[("n", &(d / 86_400))])
    }
}

fn size_text(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// Opens an `https://` link in the system browser (no shell involved).
fn open_link(url: &str) {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_control() || c == '"' || c == ' ') {
        return;
    }
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("rundll32")
        .arg("url.dll,FileProtocolHandler")
        .arg(url)
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

// ------------------------------------------------------------------ install

/// The warnings shown before installing `plugin` from `source` (empty: the
/// official source, nothing replaced).
fn needs_confirmation(
    source: &SourceEntry,
    origins: &Origins,
    installed: bool,
    id: &str,
) -> (bool, Option<String>) {
    let replaces = installed
        .then(|| origins.update_source(id).to_owned())
        .filter(|from| *from != source.id);
    (
        source.trust() != Trust::Official || replaces.is_some(),
        replaces,
    )
}

/// Asks to install the build of `plugin_id` from `source_id` made for this
/// editor: at once from the official source, after a confirmation otherwise.
pub fn request_install(
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    m: &mut StoreUi,
    source_id: &str,
    plugin_id: &str,
    installed: &[InstalledPlugin],
) {
    let Some(paths) = paths_of(shared) else {
        return;
    };
    m.ensure_loaded(&paths);
    let Some(source) = m.sources().get(source_id).cloned() else {
        return;
    };
    let Some(Ok(cat)) = m.catalogs.get(source_id) else {
        return;
    };
    let Some(plugin) = cat.parsed.catalog.plugin(plugin_id) else {
        return;
    };
    let Some((version, build)) = catalog::select(plugin, &m.build_id).build() else {
        return;
    };
    let plan = DownloadPlan {
        source: source.clone(),
        plugin_id: plugin.id.clone(),
        version: version.version.clone(),
        build: build.clone(),
    };
    let name = plugin.name.clone();
    let is_installed = installed.iter().any(|p| p.id() == plugin_id);
    let origins = m.origins.clone().unwrap_or_default();
    match needs_confirmation(&source, &origins, is_installed, plugin_id) {
        (false, _) => start_install(ctx, shared, m, plan, &name),
        (true, replaces) => {
            m.confirm = Some(Confirm {
                plan,
                name,
                replaces,
            })
        }
    }
}

fn start_install(
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    m: &mut StoreUi,
    plan: DownloadPlan,
    name: &str,
) {
    if m.installing.is_some() {
        return;
    }
    let state = shared.borrow();
    let Some(config_dir) = state.config_dir.clone() else {
        return;
    };
    let reserved = state.builtin_ids.clone();
    let loaded: std::collections::BTreeSet<String> =
        state.loaded.iter().map(|p| p.id.clone()).collect();
    drop(state);
    let paths = StorePaths::for_config_dir(&config_dir);
    let id = plan.plugin_id.clone();
    let title = ctx.tf(
        "store.installing",
        &[("name", &name), ("version", &plan.version)],
    );
    let handle = ctx.state().spawn_task(title, None, move |progress| {
        let verified = download::download(&paths, &plan, &Ed25519, &|got, total| {
            progress.set_fraction(got as f32 / total.max(1) as f32 * 0.9);
            !progress.is_cancelled()
        });
        let verified = match verified {
            Ok(v) => v,
            Err(DownloadError::Cancelled) => return Err(TaskError::Cancelled),
            Err(e) => return Ok(Err(StoreInstallError::Download(e))),
        };
        progress.check()?;
        let Some(prober) = ProcessProber::current_exe() else {
            return Ok(Err(StoreInstallError::NoProber));
        };
        let mut manager = PluginManager::for_config_dir(&config_dir, EditorFacts::current());
        let result = manager.install(
            &verified.file,
            &prober,
            &|id| reserved.contains(id),
            &|id| loaded.contains(id),
        );
        let _ = std::fs::remove_file(&verified.file);
        let installed = match result {
            Ok(i) => i,
            Err(e) => return Ok(Err(StoreInstallError::Install(e))),
        };
        let mut origins = Origins::load(&paths);
        origins.record(
            &installed.id,
            &plan.source.id,
            &installed.version,
            &plan.build.fingerprint,
        );
        let _ = origins.save(&paths);
        progress.set_fraction(1.0);
        Ok(Ok(installed))
    });
    m.installing = Some((id, handle));
    m.message = None;
}

/// Polls the refresh and install tasks; `true` when the installed plugins
/// changed (the caller reads them again).
pub fn poll(ctx: &mut EditorContext<'_>, shared: &SharedPlugins, m: &mut StoreUi) -> bool {
    if let Some(h) = m.refreshing.as_mut()
        && let Some(outcome) = h.poll()
    {
        m.refreshing = None;
        if let Ok(list) = outcome {
            for (id, result) in list {
                // A failed refresh keeps what was shown.
                if result.is_ok() || !matches!(m.catalogs.get(&id), Some(Ok(_))) {
                    m.catalogs.insert(id, result);
                }
            }
        }
    }
    let Some((_, handle)) = m.installing.as_mut() else {
        return false;
    };
    let Some(outcome) = handle.poll() else {
        return false;
    };
    m.installing = None;
    m.origins = None;
    m.message = Some(match outcome {
        Ok(Ok(done)) => {
            shared.borrow_mut().restart_needed = true;
            let key = if done.staged || done.updated {
                "plugins.updated"
            } else {
                "plugins.installed"
            };
            (
                Level::Info,
                ctx.tf(key, &[("name", &done.name), ("version", &done.version)]),
            )
        }
        Ok(Err(StoreInstallError::Download(e))) => (
            Level::Error,
            ctx.tf(
                "store.install_failed",
                &[("reason", &describe_download(ctx, &e))],
            ),
        ),
        Ok(Err(StoreInstallError::Install(e))) => (
            Level::Error,
            super::plugins::describe_install_error(ctx, &e),
        ),
        Ok(Err(StoreInstallError::NoProber)) => (
            Level::Error,
            "the editor executable was not found".to_owned(),
        ),
        Err(TaskError::Failed(e)) => (Level::Error, e),
        Err(_) => (Level::Warn, ctx.t("store.cancelled")),
    });
    if let Some((level, text)) = &m.message {
        ctx.log(*level, text.clone());
    }
    true
}

/// A plugin was installed from a file: its updates come from the official
/// source again.
pub fn forget_origin(shared: &SharedPlugins, m: &mut StoreUi, id: &str) {
    let Some(paths) = paths_of(shared) else {
        return;
    };
    let mut origins = Origins::load(&paths);
    if origins.forget(id) {
        let _ = origins.save(&paths);
    }
    m.origins = Some(origins);
}

/// When the manager panel shows: cached catalogs at once, then a refresh of
/// those older than an hour (once per session).
pub fn on_show(ctx: &EditorContext<'_>, shared: &SharedPlugins, m: &mut StoreUi) {
    let Some(paths) = paths_of(shared) else {
        return;
    };
    m.origins = None;
    m.ensure_loaded(&paths);
    // Forget the origin of plugins uninstalled since.
    let manager = shared.borrow().manager();
    if let (Some(manager), Some(origins)) = (manager, m.origins.as_mut()) {
        let installed: Vec<String> = manager
            .installed()
            .iter()
            .map(|p| p.id().to_owned())
            .collect();
        if origins.retain_installed(&installed) {
            let _ = origins.save(&paths);
        }
    }
    if !m.auto_refreshed {
        m.auto_refreshed = true;
        m.start_refresh(ctx, &paths, false);
    }
}

fn show_message(ui: &mut Ui, m: &StoreUi) {
    if let Some((level, text)) = &m.message {
        let color = match level {
            Level::Info => success(),
            Level::Warn => warning(ui),
            Level::Error => danger(ui),
        };
        ui.colored_label(color, text);
    }
}

fn install_progress(ui: &mut Ui, ctx: &EditorContext<'_>, m: &StoreUi) {
    if let Some((id, handle)) = &m.installing {
        ui.horizontal(|ui| {
            ui.spinner();
            let fraction = handle.fraction().unwrap_or(0.0);
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(160.0)
                    .show_percentage(),
            );
            ui.label(RichText::new(id).monospace().color(weak(ui)));
            if ui.button(ctx.t("plugins.cancel")).clicked() {
                handle.cancel();
            }
        });
    }
}

/// The confirmation card of a third-party install.
fn confirm_card(ui: &mut Ui, ctx: &mut EditorContext<'_>, shared: &SharedPlugins, m: &mut StoreUi) {
    let Some(confirm) = &m.confirm else { return };
    let source = confirm.plan.source.clone();
    let trust = source.trust();
    let mut decision = None;
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
            "store.confirm.text",
            &[
                ("name", &confirm.name),
                ("version", &confirm.plan.version),
                ("source", &source.name),
            ],
        ));
        match trust {
            Trust::Unsigned => {
                ui.colored_label(danger(ui), ctx.t("store.confirm.unsigned"));
            }
            Trust::Trusted => {
                ui.colored_label(warning(ui), ctx.t("store.confirm.third_party"));
            }
            Trust::Official => {}
        }
        if let Some(from) = &confirm.replaces {
            let from_name = m
                .sources()
                .get(from)
                .map_or_else(|| ctx.t("store.from_file"), |s| s.name.clone());
            ui.colored_label(
                warning(ui),
                ctx.tf("store.confirm.replaces", &[("source", &from_name)]),
            );
        }
        ui.horizontal(|ui| {
            if primary_button(ui, &ctx.t("plugins.install_confirm")).clicked() {
                decision = Some(true);
            }
            if secondary_button(ui, &ctx.t("plugins.cancel")).clicked() {
                decision = Some(false);
            }
        });
    });
    match decision {
        Some(true) => {
            if let Some(c) = m.confirm.take() {
                start_install(ctx, shared, m, c.plan, &c.name);
            }
        }
        Some(false) => m.confirm = None,
        None => {}
    }
}

// ------------------------------------------------------------------ installed tab

/// The update line of an installed plugin's card.
pub fn update_line(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    m: &mut StoreUi,
    update: &Update,
    installed: &[InstalledPlugin],
) {
    ui.horizontal_wrapped(|ui| {
        let source = m
            .sources()
            .get(&update.source)
            .map_or_else(|| update.source.clone(), |s| s.name.clone());
        badge(ui, &ctx.t("store.update_badge"), accent(ui));
        ui.label(ctx.tf(
            "store.update_available",
            &[("version", &update.available), ("source", &source)],
        ));
        let busy = m.installing.is_some();
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(format!(
                    "{} {}",
                    icons::ARROW_CLOCKWISE,
                    ctx.t("store.update")
                )),
            )
            .clicked()
        {
            request_install(ctx, shared, m, &update.source, &update.id, installed);
        }
    });
}

/// Above the installed list: install progress, confirmation, result.
pub fn installed_header(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    m: &mut StoreUi,
) {
    install_progress(ui, ctx, m);
    confirm_card(ui, ctx, shared, m);
    show_message(ui, m);
}

// ------------------------------------------------------------------ store tab

/// One plugin of one source, as listed by the store.
struct Row<'a> {
    source: &'a SourceEntry,
    plugin: &'a CatalogPlugin,
}

fn rows<'a>(
    catalogs: &[(&'a SourceEntry, &'a SourceCatalog)],
    search: &str,
    source: Option<&str>,
    tag: Option<&str>,
) -> Vec<Row<'a>> {
    let mut out: Vec<Row<'a>> = catalogs
        .iter()
        .filter(|(s, _)| source.is_none_or(|id| s.id == id))
        .flat_map(|(s, c)| {
            c.parsed.catalog.plugins.iter().map(move |p| Row {
                source: s,
                plugin: p,
            })
        })
        .filter(|r| tag.is_none_or(|t| r.plugin.tags.iter().any(|x| x == t)))
        .filter(|r| r.plugin.matches(search))
        .collect();
    // Official first, then by name.
    out.sort_by(|a, b| {
        (a.source.trust(), a.plugin.name.to_lowercase())
            .cmp(&(b.source.trust(), b.plugin.name.to_lowercase()))
    });
    out
}

pub fn store_tab(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    m: &mut StoreUi,
    installed: &[InstalledPlugin],
) {
    let Some(paths) = paths_of(shared) else {
        ui.label(RichText::new(ctx.t("plugins.no_config")).color(weak(ui)));
        return;
    };
    m.ensure_loaded(&paths);
    ui.horizontal_wrapped(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut m.search)
                .hint_text(format!(
                    "{} {}",
                    icons::MAGNIFYING_GLASS,
                    ctx.t("store.search")
                ))
                .desired_width(220.0),
        );
        let source_name = m
            .source_filter
            .as_deref()
            .and_then(|id| m.sources().get(id))
            .map_or_else(|| ctx.t("store.all_sources"), |s| s.name.clone());
        let enabled: Vec<(String, String)> = m
            .sources()
            .enabled()
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect();
        egui::ComboBox::from_id_salt("store_source_filter")
            .selected_text(source_name)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut m.source_filter, None, ctx.t("store.all_sources"));
                for (id, name) in enabled {
                    ui.selectable_value(&mut m.source_filter, Some(id), name);
                }
            });
        let mut tags = std::collections::BTreeSet::new();
        for (_, c) in m.catalogs() {
            tags.extend(c.parsed.catalog.tags());
        }
        let tag_text = m
            .tag_filter
            .clone()
            .unwrap_or_else(|| ctx.t("store.all_tags"));
        egui::ComboBox::from_id_salt("store_tag_filter")
            .selected_text(tag_text)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut m.tag_filter, None, ctx.t("store.all_tags"));
                for t in tags {
                    ui.selectable_value(&mut m.tag_filter, Some(t.clone()), t);
                }
            });
        let refreshing = m.refreshing.is_some();
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
        {
            m.start_refresh(ctx, &paths, true);
        }
        if refreshing {
            ui.spinner();
        }
    });
    ui.label(
        RichText::new(ctx.tf(
            "store.editor_build",
            &[
                ("version", &orbit_editor_api::native::API_VERSION),
                ("id", &m.build_id),
            ],
        ))
        .small()
        .color(weak(ui)),
    );
    install_progress(ui, ctx, m);
    confirm_card(ui, ctx, shared, m);
    show_message(ui, m);
    // Offline / failing sources.
    for s in m.sources().enabled() {
        match m.catalogs.get(&s.id) {
            Some(Ok(c)) if c.offline.is_some() => {
                ui.colored_label(
                    warning(ui),
                    ctx.tf(
                        "store.offline",
                        &[("source", &s.name), ("when", &ago(ctx, c.fetched_at))],
                    ),
                );
            }
            Some(Err(e)) => {
                ui.colored_label(
                    danger(ui),
                    ctx.tf("store.source_failed", &[("source", &s.name), ("error", e)]),
                );
            }
            _ => {}
        }
    }
    ui.separator();

    let catalogs = m.catalogs();
    if catalogs.is_empty() {
        let key = if m.refreshing.is_some() {
            "store.loading"
        } else {
            "store.no_catalog"
        };
        ui.label(RichText::new(ctx.t(key)).color(weak(ui)));
        return;
    }
    let list = rows(
        &catalogs,
        &m.search,
        m.source_filter.as_deref(),
        m.tag_filter.as_deref(),
    );
    if list.is_empty() {
        ui.label(RichText::new(ctx.t("store.no_match")).color(weak(ui)));
        return;
    }
    // Collected while drawing, applied after (the rows borrow `m`).
    let mut install: Option<(String, String)> = None;
    let mut toggle: Option<(String, String)> = None;
    let lang = ctx.lang().code();
    let busy = m.installing.is_some();
    ScrollArea::vertical()
        .id_salt("store_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in &list {
                let open = m
                    .details
                    .as_ref()
                    .is_some_and(|(s, p)| *s == row.source.id && *p == row.plugin.id);
                let action = plugin_card(ui, ctx, row, installed, &m.build_id, lang, open, busy);
                match action {
                    CardAction::Install => {
                        install = Some((row.source.id.clone(), row.plugin.id.clone()))
                    }
                    CardAction::Toggle => {
                        toggle = Some((row.source.id.clone(), row.plugin.id.clone()))
                    }
                    CardAction::None => {}
                }
                ui.add_space(6.0);
            }
        });
    if let Some(t) = toggle {
        m.details = if m.details.as_ref() == Some(&t) {
            None
        } else {
            Some(t)
        };
    }
    if let Some((source, plugin)) = install {
        request_install(ctx, shared, m, &source, &plugin, installed);
    }
}

enum CardAction {
    None,
    Install,
    Toggle,
}

#[allow(clippy::too_many_arguments)]
fn plugin_card(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    row: &Row<'_>,
    installed: &[InstalledPlugin],
    build_id: &str,
    lang: &str,
    open: bool,
    busy: bool,
) -> CardAction {
    let p = row.plugin;
    let mut action = CardAction::None;
    let availability = catalog::select(p, build_id);
    let current = installed
        .iter()
        .find(|i| i.id() == p.id)
        .map(|i| i.manifest.version.clone());
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&p.name).strong().size(15.0));
            if let Some((v, _)) = availability.build() {
                ui.label(RichText::new(&v.version).color(weak(ui)));
            }
            if !p.author.is_empty() {
                ui.label(
                    RichText::new(ctx.tf("plugins.by", &[("author", &p.author)])).color(weak(ui)),
                );
            }
            let (text, color) = trust_label(ctx, row.source.trust());
            badge(ui, &format!("{} · {}", row.source.name, text), color);
        });
        ui.label(RichText::new(&p.id).small().monospace().color(weak(ui)));
        let description = p.description.get(lang);
        if !description.is_empty() {
            ui.label(description);
        }
        if !p.tags.is_empty() {
            ui.label(
                RichText::new(
                    p.tags
                        .iter()
                        .map(|t| format!("#{t}"))
                        .collect::<Vec<_>>()
                        .join("  "),
                )
                .small()
                .color(weak(ui)),
            );
        }
        ui.horizontal_wrapped(|ui| {
            match (&availability, &current) {
                (
                    Availability::Available {
                        version,
                        newer_elsewhere,
                        ..
                    },
                    current,
                ) => {
                    let newer = current
                        .as_deref()
                        .is_none_or(|c| orbit_plugin_store::version::is_newer(&version.version, c));
                    let label = match current {
                        None => Some(ctx.t("store.install")),
                        Some(_) if newer => {
                            Some(ctx.tf("store.update_to", &[("version", &version.version)]))
                        }
                        Some(c) => {
                            ui.colored_label(
                                success(),
                                format!(
                                    "{} {}",
                                    icons::CHECK,
                                    ctx.tf("store.installed", &[("version", c)])
                                ),
                            );
                            None
                        }
                    };
                    if let Some(label) = label
                        && ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(format!("{} {label}", icons::DOWNLOAD_SIMPLE)),
                            )
                            .clicked()
                    {
                        action = CardAction::Install;
                    }
                    if let Some(n) = newer_elsewhere {
                        ui.label(
                            RichText::new(
                                ctx.tf("store.newer_elsewhere", &[("version", &n.version)]),
                            )
                            .small()
                            .color(weak(ui)),
                        );
                    }
                }
                (Availability::NoBuild { .. }, _) => {
                    let editors = catalog::editor_versions(p);
                    let text = if editors.is_empty() {
                        ctx.tf(
                            "store.no_build",
                            &[("editor", &orbit_editor_api::native::API_VERSION)],
                        )
                    } else {
                        ctx.tf(
                            "store.no_build_for",
                            &[
                                ("editor", &orbit_editor_api::native::API_VERSION),
                                ("versions", &editors.join(", ")),
                            ],
                        )
                    };
                    ui.colored_label(warning(ui), text);
                    if let Some(c) = &current {
                        ui.label(
                            RichText::new(ctx.tf("store.installed", &[("version", c)]))
                                .color(weak(ui)),
                        );
                    }
                }
            }
            let details = if open {
                ctx.t("store.hide_details")
            } else {
                ctx.t("store.details")
            };
            if ui.button(details).clicked() {
                action = CardAction::Toggle;
            }
        });
        if open {
            details(ui, ctx, row, build_id, lang);
        }
    });
    action
}

fn details(ui: &mut Ui, ctx: &EditorContext<'_>, row: &Row<'_>, build_id: &str, lang: &str) {
    let p = row.plugin;
    ui.separator();
    egui::Grid::new(("store_details", &row.source.id, &p.id))
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new(ctx.t("store.license")).color(weak(ui)));
            ui.label(p.license.clone().unwrap_or_else(|| ctx.t("store.unknown")));
            ui.end_row();
            ui.label(RichText::new(ctx.t("store.source")).color(weak(ui)));
            ui.label(format!("{} ({})", row.source.name, row.source.url));
            ui.end_row();
            if let Some(home) = &p.homepage {
                ui.label(RichText::new(ctx.t("store.homepage")).color(weak(ui)));
                if home.starts_with("https://") {
                    if ui.link(home).clicked() {
                        open_link(home);
                    }
                } else {
                    ui.label(home);
                }
                ui.end_row();
            }
        });
    if row.source.trust() == Trust::Unsigned {
        ui.colored_label(danger(ui), ctx.t("store.unsigned_note"));
    }
    ui.add_space(4.0);
    ui.label(RichText::new(ctx.t("store.versions")).strong());
    for v in &p.versions {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&v.version).strong());
            if let Some(date) = &v.released {
                ui.label(RichText::new(date).small().color(weak(ui)));
            }
            let mine = v.builds.iter().find(|b| b.fingerprint == build_id);
            match mine {
                Some(b) => {
                    let signed = if b.signature.is_some() {
                        ctx.t("store.signed")
                    } else {
                        ctx.t("store.not_signed")
                    };
                    ui.label(
                        RichText::new(format!(
                            "{} · {} · {signed}",
                            ctx.t("store.for_this_editor"),
                            size_text(b.size)
                        ))
                        .small()
                        .color(success()),
                    );
                }
                None => {
                    let editors: Vec<String> = v
                        .builds
                        .iter()
                        .map(|b| b.editor_version.clone())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    ui.label(
                        RichText::new(
                            ctx.tf("store.built_for", &[("versions", &editors.join(", "))]),
                        )
                        .small()
                        .color(weak(ui)),
                    );
                }
            }
        });
        let changelog = v.changelog.get(lang);
        if !changelog.is_empty() {
            ui.label(RichText::new(changelog).color(weak(ui)));
        }
    }
}

// ------------------------------------------------------------------ sources tab

pub fn sources_tab(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    shared: &SharedPlugins,
    m: &mut StoreUi,
) {
    let Some(paths) = paths_of(shared) else {
        ui.label(RichText::new(ctx.t("plugins.no_config")).color(weak(ui)));
        return;
    };
    m.ensure_loaded(&paths);
    ui.label(RichText::new(ctx.t("store.sources_intro")).color(weak(ui)));
    ui.horizontal(|ui| {
        let refreshing = m.refreshing.is_some();
        if ui
            .add_enabled(
                !refreshing,
                egui::Button::new(format!(
                    "{} {}",
                    icons::ARROW_CLOCKWISE,
                    ctx.t("store.refresh_all")
                )),
            )
            .clicked()
        {
            m.start_refresh(ctx, &paths, true);
        }
        if refreshing {
            ui.spinner();
        }
    });
    show_message(ui, m);
    ui.add_space(4.0);

    let mut toggle: Option<(String, bool)> = None;
    let mut remove: Option<String> = None;
    let mut ask_remove: Option<Option<String>> = None;
    let list: Vec<SourceEntry> = m.sources().sources.clone();
    ScrollArea::vertical()
        .id_salt("store_sources")
        .auto_shrink([false, true])
        .max_height((ui.available_height() - 170.0).max(120.0))
        .show(ui, |ui| {
            for s in &list {
                card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let mut enabled = s.enabled;
                        if ui
                            .checkbox(&mut enabled, RichText::new(&s.name).strong())
                            .changed()
                        {
                            toggle = Some((s.id.clone(), enabled));
                        }
                        let (text, color) = trust_label(ctx, s.trust());
                        badge(ui, &text, color);
                    });
                    ui.label(RichText::new(&s.url).small().monospace().color(weak(ui)));
                    match s.trust() {
                        Trust::Official => {
                            ui.label(
                                RichText::new(ctx.t("store.source.official_note"))
                                    .small()
                                    .color(weak(ui)),
                            );
                        }
                        Trust::Trusted => {
                            let key = s.public_key.clone().unwrap_or_default();
                            ui.label(
                                RichText::new(ctx.tf("store.source.key", &[("key", &key)]))
                                    .small()
                                    .monospace()
                                    .color(weak(ui)),
                            );
                            ui.label(
                                RichText::new(ctx.t("store.source.trusted_note"))
                                    .small()
                                    .color(warning(ui)),
                            );
                        }
                        Trust::Unsigned => {
                            ui.colored_label(
                                danger(ui),
                                RichText::new(ctx.t("store.source.unsigned_note")).small(),
                            );
                        }
                    }
                    match m.catalogs.get(&s.id) {
                        Some(Ok(c)) => {
                            let n = c.parsed.catalog.plugins.len();
                            let mut line = ctx.tf(
                                "store.source.status",
                                &[("count", &n), ("when", &ago(ctx, c.fetched_at))],
                            );
                            if let Some(e) = &c.offline {
                                line = format!(
                                    "{line} — {}",
                                    ctx.tf("store.source.offline", &[("error", e)])
                                );
                            }
                            ui.label(RichText::new(line).small().color(weak(ui)));
                            if !c.parsed.warnings.is_empty() {
                                egui::CollapsingHeader::new(
                                    RichText::new(ctx.tf(
                                        "store.source.warnings",
                                        &[("count", &c.parsed.warnings.len())],
                                    ))
                                    .small()
                                    .color(warning(ui)),
                                )
                                .id_salt(("source_warnings", &s.id))
                                .show(ui, |ui| {
                                    for w in &c.parsed.warnings {
                                        ui.label(RichText::new(w).small());
                                    }
                                });
                            }
                        }
                        Some(Err(e)) => {
                            ui.colored_label(
                                danger(ui),
                                RichText::new(ctx.tf("store.source.failed", &[("error", e)]))
                                    .small(),
                            );
                        }
                        None if s.enabled => {
                            ui.label(
                                RichText::new(ctx.t("store.source.not_fetched"))
                                    .small()
                                    .color(weak(ui)),
                            );
                        }
                        None => {}
                    }
                    if !s.builtin {
                        ui.horizontal(|ui| {
                            if m.confirm_remove.as_deref() == Some(s.id.as_str()) {
                                ui.colored_label(danger(ui), ctx.t("store.source.remove_confirm"));
                                if ui.button(ctx.t("plugins.uninstall_yes")).clicked() {
                                    remove = Some(s.id.clone());
                                }
                                if ui.button(ctx.t("plugins.cancel")).clicked() {
                                    ask_remove = Some(None);
                                }
                            } else if ui
                                .button(format!(
                                    "{} {}",
                                    icons::TRASH,
                                    ctx.t("store.source.remove")
                                ))
                                .clicked()
                            {
                                ask_remove = Some(Some(s.id.clone()));
                            }
                        });
                    }
                });
                ui.add_space(4.0);
            }
        });
    if let Some(a) = ask_remove {
        m.confirm_remove = a;
    }
    if let Some((id, enabled)) = toggle
        && let Some(sources) = m.sources.as_mut()
    {
        let _ = sources.set_enabled(&id, enabled);
        m.save_sources(&paths);
        if enabled {
            m.start_refresh(ctx, &paths, false);
        }
    }
    if let Some(id) = remove
        && let Some(sources) = m.sources.as_mut()
    {
        match sources.remove(&id) {
            Ok(removed) => {
                fetch::forget(&paths, &removed);
                m.catalogs.remove(&id);
                m.message = Some((
                    Level::Info,
                    ctx.tf("store.source.removed", &[("name", &removed.name)]),
                ));
                m.save_sources(&paths);
            }
            Err(e) => m.message = Some((Level::Error, describe_source_error(ctx, &e))),
        }
        m.confirm_remove = None;
    }

    ui.separator();
    ui.label(RichText::new(ctx.t("store.add_title")).strong());
    egui::Grid::new("store_add_source")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label(ctx.t("store.add_url"));
            ui.add(
                egui::TextEdit::singleline(&mut m.add_url)
                    .hint_text("https://github.com/owner/repo")
                    .desired_width(320.0),
            );
            ui.end_row();
            ui.label(ctx.t("store.add_name"));
            ui.add(
                egui::TextEdit::singleline(&mut m.add_name)
                    .hint_text(ctx.t("store.optional"))
                    .desired_width(320.0),
            );
            ui.end_row();
            ui.label(ctx.t("store.add_key"));
            ui.add(
                egui::TextEdit::singleline(&mut m.add_key)
                    .hint_text(ctx.t("store.add_key_hint"))
                    .desired_width(320.0)
                    .font(egui::TextStyle::Monospace),
            );
            ui.end_row();
        });
    ui.label(
        RichText::new(ctx.t("store.add_warning"))
            .small()
            .color(warning(ui)),
    );
    if primary_button(ui, &format!("{} {}", icons::PLUS, ctx.t("store.add"))).clicked() {
        let added = m
            .sources
            .as_mut()
            .map(|s| s.add(&m.add_url, &m.add_name, &m.add_key));
        match added {
            Some(Ok(entry)) => {
                m.message = Some((
                    Level::Info,
                    ctx.tf(
                        "store.source.added",
                        &[("name", &entry.name), ("url", &entry.url)],
                    ),
                ));
                m.add_url.clear();
                m.add_name.clear();
                m.add_key.clear();
                m.save_sources(&paths);
                m.start_refresh(ctx, &paths, true);
            }
            Some(Err(e)) => m.message = Some((Level::Error, describe_source_error(ctx, &e))),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_plugin_store::catalog::{ParsedCatalog, parse};

    fn catalog(json: serde_json::Value, official: bool) -> SourceCatalog {
        let parsed: ParsedCatalog = parse(&json.to_string(), official).unwrap();
        SourceCatalog {
            parsed,
            fetched_at: 0,
            offline: None,
        }
    }

    #[test]
    fn store_rows_filter_and_put_official_first() {
        let mut sources = Sources::default();
        let third = sources
            .add("https://example.org/index.json", "Third", "")
            .unwrap();
        let official = sources.sources[0].clone();
        let a = catalog(
            serde_json::json!({"schema_version":1,"source":{"name":"O"},"plugins":[
                {"id":"orbit.ai_studio","name":"Zeta studio","tags":["ai"]}]}),
            true,
        );
        let b = catalog(
            serde_json::json!({"schema_version":1,"source":{"name":"T"},"plugins":[
                {"id":"acme.alpha","name":"Alpha","tags":["tools"],"description":{"fr":"Outil"}},
                {"id":"acme.beta","name":"Beta","tags":["ai","tools"]}]}),
            false,
        );
        let catalogs = [(&third, &b), (&official, &a)];
        let names = |r: Vec<Row<'_>>| r.iter().map(|r| r.plugin.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(rows(&catalogs, "", None, None)),
            ["Zeta studio", "Alpha", "Beta"]
        );
        assert_eq!(
            names(rows(&catalogs, "", None, Some("ai"))),
            ["Zeta studio", "Beta"]
        );
        assert_eq!(names(rows(&catalogs, "outil", None, None)), ["Alpha"]);
        assert_eq!(
            names(rows(&catalogs, "", Some(&third.id), Some("tools"))),
            ["Alpha", "Beta"]
        );
        assert!(rows(&catalogs, "nothing", None, None).is_empty());
    }

    #[test]
    fn third_party_and_replacing_installs_ask_first() {
        let mut sources = Sources::default();
        let third = sources
            .add("https://example.org/index.json", "Third", "")
            .unwrap();
        let official = sources.sources[0].clone();
        let mut origins = Origins::default();
        // Official, not installed: no question.
        assert_eq!(
            needs_confirmation(&official, &origins, false, "orbit.a"),
            (false, None)
        );
        // Official, installed from a file (updates come from official): none.
        assert_eq!(
            needs_confirmation(&official, &origins, true, "orbit.a"),
            (false, None)
        );
        // Third party: always asked; replacing the official one says so.
        assert_eq!(
            needs_confirmation(&third, &origins, false, "acme.a"),
            (true, None)
        );
        assert_eq!(
            needs_confirmation(&third, &origins, true, "acme.a"),
            (true, Some("official".into()))
        );
        // Installed from the third party, updated from it: asked, no "replaces".
        origins.record("acme.a", &third.id, "1.0.0", "x");
        assert_eq!(
            needs_confirmation(&third, &origins, true, "acme.a"),
            (true, None)
        );
        // Official replacing a third-party install: asked.
        assert_eq!(
            needs_confirmation(&official, &origins, true, "acme.a"),
            (true, Some(third.id.clone()))
        );
        assert_eq!(editor_build_id().len(), 32);
    }
}
