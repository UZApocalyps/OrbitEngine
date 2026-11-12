//! Export panel: turns the open project into games (Windows, Linux, web).
//!
//! The work is `orbit_export`; this plugin only edits the export profiles of
//! the manifest, runs an export on a worker thread (the window stays alive,
//! the thread wakes it with the editor's `Waker`), shows the progress and the
//! journal, opens the output folder and serves a web export to try it in the
//! browser. The panel is hidden by default: File > "Export game…" or the
//! View menu opens it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use egui::{ComboBox, ProgressBar, RichText, ScrollArea, TextEdit, Ui};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, PanelDef,
    Plugin, PluginManifest, Translator, UiText, Waker, icons,
};
use orbit_export::android::{AndroidOptions, Secret, Signing, toolchain as android_tools};
use orbit_export::{
    AndroidFormat, AndroidOrientation, DesktopQuality,
    Event, ExportOptions, ExportProfile, ExportProject, ExportReport, ExportTarget, LogLevel,
    PakCompression, Server, Templates, export_many,
};
use orbit_project::{MANIFEST_FILE, ProjectManifest};

use super::ids;
use super::panels;
use super::ui_util::{card, danger, faint, section_title, success, warning, weak};
use crate::storage::FsStorage;

pub struct ExportPlugin;

/// Preference key of the folder holding the player templates.
const PREF_TEMPLATES: &str = "templates_dir";
/// Preference keys of the Android keystore file and key alias. The password
/// is never a preference.
const PREF_ANDROID_KEYSTORE: &str = "android_keystore";
const PREF_ANDROID_ALIAS: &str = "android_alias";

impl Plugin for ExportPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::EXPORT, "Export", "0.1.0")
    }
}

impl EditorPlugin for ExportPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "export",
            "export.title",
            icons::EXPORT,
            DockSlot::Hidden,
            panel,
        )
        .opens_in(DockSlot::Right));
        reg.action(
            ActionDef::new("open", "export.open", |c| {
                c.request(EditorRequest::ShowPanel(panels::EXPORT.to_owned()));
            })
            .icon(icons::EXPORT)
            .category("category.file")
            .menu_group("menu.exchange")
            .menu("file", 300)
            .enabled(|c| c.has_project()),
        );
    }
}

// ------------------------------------------------------------------ job

/// One line of the journal.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub level: LogLevel,
    pub text: String,
}

#[derive(Default)]
struct JobShared {
    fraction: f32,
    label: String,
    log: Vec<LogLine>,
    outcome: Option<Result<Vec<ExportReport>, String>>,
}

/// An export running on its own thread.
pub struct ExportJob {
    shared: Arc<Mutex<JobShared>>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// What a job can be asked, between frames.
pub struct JobStatus {
    pub fraction: f32,
    pub label: String,
    /// Lines of the journal not seen yet.
    pub new_log: Vec<LogLine>,
    /// Set once, when the export ended.
    pub outcome: Option<Result<Vec<ExportReport>, String>>,
}

impl ExportJob {
    /// Starts exporting `project` (a project folder or story file) for
    /// `targets`. `waker` is called on every event so the window redraws.
    pub fn start(
        project: PathBuf,
        profile: ExportProfile,
        targets: Vec<ExportTarget>,
        out_dir: PathBuf,
        templates: Templates,
        android: AndroidOptions,
        waker: Waker,
        texts: &Translator,
    ) -> Self {
        let shared = Arc::new(Mutex::new(JobShared::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sink, flag) = (shared.clone(), cancel.clone());
        let thread = std::thread::Builder::new()
            .name("orbit-export".to_owned())
            .spawn(move || {
                let wake = waker.clone();
                let outcome = (|| {
                    let project = ExportProject::open(&project).map_err(|e| e.to_string())?;
                    let options = ExportOptions {
                        templates,
                        cancel: Some(flag),
                        android,
                    };
                    let events = sink.clone();
                    export_many(
                        &project,
                        &profile,
                        &targets,
                        &out_dir,
                        &options,
                        &mut move |event| {
                            if let Ok(mut s) = events.lock() {
                                match event {
                                    Event::Step { fraction, label } => {
                                        s.fraction = *fraction;
                                        s.label.clone_from(label);
                                    }
                                    Event::Log { level, text } => s.log.push(LogLine {
                                        level: *level,
                                        text: text.clone(),
                                    }),
                                }
                            }
                            waker.wake();
                        },
                    )
                    .map_err(|e| e.to_string())
                })();
                if let Ok(mut s) = sink.lock() {
                    s.outcome = Some(outcome);
                }
                wake.wake();
            })
            .ok();
        if thread.is_none()
            && let Ok(mut s) = shared.lock()
        {
            s.outcome = Some(Err(texts.t("export.thread_failed")));
        }
        Self {
            shared,
            cancel,
            thread,
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Progress and new journal lines; takes the outcome once it is there.
    pub fn poll(&mut self) -> JobStatus {
        let mut s = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let status = JobStatus {
            fraction: s.fraction,
            label: s.label.clone(),
            new_log: std::mem::take(&mut s.log),
            outcome: s.outcome.take(),
        };
        drop(s);
        if status.outcome.is_some()
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
        status
    }
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        // Leaving the editor (or closing the project) stops the export.
        self.cancel();
    }
}

// ------------------------------------------------------------------ state

/// Everything the panel remembers.
#[derive(Default)]
struct ExportUi {
    /// Project folder the form was loaded for.
    loaded_root: Option<PathBuf>,
    profiles: Vec<ExportProfile>,
    selected: usize,
    /// The project has a manifest: profiles can be saved.
    has_manifest: bool,
    dirty: bool,
    out_dir: String,
    templates_dir: String,
    /// Android signing: keystore path and alias are preferences, the
    /// password is typed for each export and never stored.
    android_keystore: String,
    android_alias: String,
    android_debug: bool,
    android_password: String,
    /// Android tools found on this machine (refreshed on demand).
    android_tools: Option<android_tools::Detected>,
    job: Option<ExportJob>,
    fraction: f32,
    label: String,
    log: Vec<LogLine>,
    result: Option<Result<Vec<ExportReport>, String>>,
    server: Option<Server>,
    /// The export was asked while the story had unsaved edits: it starts
    /// once the save went through.
    after_save: bool,
    /// Rust plugin crates of the project and the toolchain (M8-04).
    plugins: PluginStatus,
    /// 3D scenes measured against the quality profiles (M6-12); `None`
    /// until checked (on first display, then on demand).
    scenes3d: Option<Vec<orbit_export::scene3d::SceneReport>>,
}

/// What the panel tells about an export "with plugins".
#[derive(Default)]
struct PluginStatus {
    /// Crate names in `plugins/`.
    crates: Vec<String>,
    /// Why `plugins/` could not be read.
    error: Option<String>,
    /// `cargo --version`, or why it is unusable. Only probed when there are
    /// plugin crates.
    toolchain: Option<Result<String, String>>,
}

impl PluginStatus {
    fn detect(root: &Path) -> Self {
        let (crates, error) = match orbit_export::plugins::discover(root) {
            Ok(c) => (c.into_iter().map(|c| c.name).collect::<Vec<_>>(), None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        let toolchain = (!crates.is_empty()).then(|| {
            orbit_export::plugins::cargo_version()
                .and_then(|v| orbit_export::plugins::engine_source().map(|_| v))
                .map_err(|e| e.to_string())
        });
        Self {
            crates,
            error,
            toolchain,
        }
    }
}

/// The "Plugins" note of the panel: shown only for projects with plugins.
fn plugins_section(ui: &mut Ui, state: &ExportUi) {
    let status = &state.plugins;
    if let Some(e) = &status.error {
        ui.label(RichText::new(e).color(danger(ui)));
        return;
    }
    let crates = &status.crates;
    if crates.is_empty() {
        return;
    }
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        section_title(ui, &ui.t("export.plugins.title"));
        ui.label(ui.tf("export.plugins.crates", &[("crates", &crates.join(", "))]));
        match &status.toolchain {
            Some(Ok(version)) => {
                ui.label(
                    RichText::new(ui.tf("export.plugins.toolchain_ok", &[("version", version)]))
                        .color(success()),
                );
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(danger(ui)));
            }
            None => {}
        }
        ui.label(RichText::new(ui.t("export.plugins.note")).small().color(weak(ui)));
    });
}

/// The "3D quality" card (M6-12): each 3D scene of the story measured
/// against the web profile and the desktop profile of the export profile.
/// Going over only warns: the export still works (ADR 0014 section 3).
fn scenes3d_section(ui: &mut Ui, state: &mut ExportUi, root: &Path, story: &orbit_core::Story) {
    use orbit_export::scene3d::{analyze, tier_of};
    let texts = ui.texts();
    let mut recheck = false;
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            section_title(ui, &texts.t("export.s3d.title"));
            recheck = ui
                .small_button(format!("{}  {}", icons::ARROWS_CLOCKWISE, texts.t("export.s3d.check")))
                .on_hover_text(texts.t("export.s3d.check_hint"))
                .clicked();
        });
        let Some(reports) = &state.scenes3d else {
            return;
        };
        let profile = &state.profiles[state.selected.min(state.profiles.len() - 1)];
        let mut tiers = vec![tier_of(ExportTarget::Web, profile)];
        tiers.push(tier_of(ExportTarget::Windows, profile));
        for report in reports {
            let s = &report.stats;
            let largest = s
                .textures
                .values()
                .max_by_key(|(w, h)| w.max(h))
                .map(|(w, h)| format!("{w}x{h}"))
                .unwrap_or_else(|| "-".into());
            ui.label(RichText::new(&report.path).strong());
            ui.label(
                RichText::new(texts.tf(
                    "export.s3d.stats",
                    &[
                        ("lights", &s.lights.to_string()),
                        ("shadows", &s.shadow_lights.to_string()),
                        ("triangles", &s.triangles.to_string()),
                        ("textures", &s.textures.len().to_string()),
                        ("largest", &largest),
                    ],
                ))
                .color(weak(ui)),
            );
            for error in &report.errors {
                ui.label(RichText::new(format!("{}  {error}", icons::WARNING)).color(warning(ui)));
            }
            for &tier in &tiers {
                let name = texts.t(&format!("export.s3d.tier.{}", tier.id()));
                let issues = report.issues(tier);
                if issues.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "{}  {}",
                            icons::CHECK,
                            texts.tf("export.s3d.fits", &[("profile", &name)])
                        ))
                        .color(success()),
                    );
                }
                for issue in issues {
                    let issue = issue_text(&texts, &issue);
                    ui.label(
                        RichText::new(format!("{}  {name}: {issue}", icons::WARNING))
                            .color(warning(ui)),
                    );
                }
            }
            if !s.unknown.is_empty() {
                ui.label(
                    RichText::new(texts.tf(
                        "export.s3d.unknown",
                        &[("refs", &s.unknown.join(", "))],
                    ))
                    .small()
                    .color(faint(ui)),
                );
            }
            ui.add_space(4.0);
        }
        ui.label(RichText::new(texts.t("export.s3d.note")).small().color(weak(ui)));
    });
    if recheck || state.scenes3d.is_none() {
        state.scenes3d = Some(analyze(root, story));
    }
}

/// A quality issue in the language of the editor.
fn issue_text(texts: &Translator, issue: &orbit_scene::quality::QualityIssue) -> String {
    use orbit_scene::quality::QualityIssue as Q;
    match issue {
        Q::TooManyLights { count, max } => texts.tf(
            "export.s3d.issue.lights",
            &[("count", &count.to_string()), ("max", &max.to_string())],
        ),
        Q::TooManyShadows { count, max } => texts.tf(
            "export.s3d.issue.shadows",
            &[("count", &count.to_string()), ("max", &max.to_string())],
        ),
        Q::TooManyTriangles { count, max } => texts.tf(
            "export.s3d.issue.triangles",
            &[("count", &count.to_string()), ("max", &max.to_string())],
        ),
        Q::TextureTooLarge {
            name,
            width,
            height,
            max,
        } => texts.tf(
            "export.s3d.issue.texture",
            &[
                ("name", name),
                ("size", &format!("{width}x{height}")),
                ("max", &max.to_string()),
            ],
        ),
    }
}

type SharedUi = Arc<Mutex<ExportUi>>;

fn shared_state(ctx: &egui::Context) -> SharedUi {
    let id = egui::Id::new("orbit_export_ui");
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with::<SharedUi>(id, || Arc::new(Mutex::new(ExportUi::default())))
            .clone()
    })
}

/// The export folder next to the project, by default.
fn default_out_dir(root: &Path) -> String {
    root.join("export").to_string_lossy().into_owned()
}

impl ExportUi {
    /// (Re)loads the profiles when another project is open.
    fn ensure_loaded(&mut self, root: &Path, story_file: &str, has_manifest: bool, name: &str) {
        if self.loaded_root.as_deref() == Some(root) {
            return;
        }
        *self = Self {
            templates_dir: std::mem::take(&mut self.templates_dir),
            android_keystore: std::mem::take(&mut self.android_keystore),
            android_alias: std::mem::take(&mut self.android_alias),
            android_debug: self.android_debug,
            android_tools: self.android_tools.take(),
            server: self.server.take(),
            ..Self::default()
        };
        let opened = if has_manifest {
            ExportProject::open(root)
        } else {
            ExportProject::open(&root.join(story_file))
        };
        self.profiles = match opened {
            Ok(project) => project.profiles(),
            Err(_) => vec![ExportProfile::new("release", name)],
        };
        if self.profiles.is_empty() {
            self.profiles.push(ExportProfile::new("release", name));
        }
        self.has_manifest = has_manifest;
        self.plugins = PluginStatus::detect(root);
        self.scenes3d = None;
        self.out_dir = default_out_dir(root);
        self.loaded_root = Some(root.to_owned());
    }

    fn profile(&mut self) -> &mut ExportProfile {
        self.selected = self.selected.min(self.profiles.len().saturating_sub(1));
        &mut self.profiles[self.selected]
    }

    /// Writes the profiles into `orbit.project.ron`.
    fn save_profiles(&mut self, root: &Path) -> Result<(), String> {
        let storage = FsStorage::new(root);
        let mut manifest = orbit_project::load_manifest(&storage).map_err(|e| e.to_string())?;
        manifest.export_profiles = self.profiles.clone();
        orbit_project::save_manifest(&storage, &manifest).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    fn templates(&self) -> Templates {
        let mut templates = Templates::standard();
        if !self.templates_dir.trim().is_empty() {
            templates = templates.with_dir_first(self.templates_dir.trim());
        }
        templates
    }

    fn wants_android(&mut self) -> bool {
        self.profile().targets.contains(&ExportTarget::Android)
    }

    /// The signing the Android export will use. Takes the typed password:
    /// it is asked again for the next export.
    fn take_android_options(&mut self) -> AndroidOptions {
        let signing = if self.android_debug {
            Signing::Debug
        } else if self.android_keystore.trim().is_empty() {
            Signing::Unset
        } else {
            Signing::Keystore {
                path: PathBuf::from(self.android_keystore.trim()),
                alias: self.android_alias.clone(),
                password: Secret::new(std::mem::take(&mut self.android_password)),
            }
        };
        self.android_password.clear();
        AndroidOptions {
            signing,
            ..AndroidOptions::default()
        }
    }

    /// Android needs a signing choice and, for a keystore, its password.
    fn android_ready(&mut self) -> bool {
        !self.wants_android()
            || self.android_debug
            || (!self.android_keystore.trim().is_empty()
                && !self.android_alias.trim().is_empty()
                && !self.android_password.is_empty())
    }

    fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Takes what the job did since the last frame.
    fn poll(&mut self) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        let status = job.poll();
        self.fraction = status.fraction;
        self.label = status.label;
        self.log.extend(status.new_log);
        if let Some(outcome) = status.outcome {
            self.job = None;
            self.result = Some(outcome);
        }
    }
}

// ------------------------------------------------------------------ panel

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let Some(info) = ctx.project_info().cloned() else {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ui.t("export.no_project")).color(weak(ui)));
        });
        return;
    };
    let shared = shared_state(ui.ctx());
    let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
    if state.templates_dir.is_empty() {
        state.templates_dir = ctx
            .pref(ids::EXPORT, PREF_TEMPLATES)
            .unwrap_or("")
            .to_owned();
    }
    if state.android_keystore.is_empty() {
        state.android_keystore = ctx
            .pref(ids::EXPORT, PREF_ANDROID_KEYSTORE)
            .unwrap_or("")
            .to_owned();
    }
    if state.android_alias.is_empty() {
        state.android_alias = ctx
            .pref(ids::EXPORT, PREF_ANDROID_ALIAS)
            .unwrap_or("")
            .to_owned();
    }
    state.ensure_loaded(&info.root, &info.story_file, info.has_manifest, &info.name);
    state.poll();
    if state.running() {
        ui.ctx().request_repaint();
    }
    // An export waiting for the save to finish.
    if state.after_save && !info.modified {
        state.after_save = false;
        start_export(&mut state, ctx, &info);
    }

    let mut messages: Vec<(bool, String)> = Vec::new();
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            profile_form(ui, &mut state, &info.root, &mut messages);
            ui.add_space(6.0);
            targets_and_output(ui, &mut state);
            if !state.plugins.crates.is_empty() || state.plugins.error.is_some() {
                ui.add_space(6.0);
                plugins_section(ui, &state);
            }
            if state.wants_android() {
                ui.add_space(6.0);
                android_section(ui, &mut state);
            }
            if let Some(story) = ctx.story()
                && orbit_core::experimental::enabled_3d()
                && orbit_export::scene3d::uses_3d(story)
            {
                ui.add_space(6.0);
                scenes3d_section(ui, &mut state, &info.root, story);
            }
            ui.add_space(8.0);
            run_controls(ui, &mut state, ctx, &info, &mut messages);
            ui.add_space(8.0);
            journal(ui, &mut state, &mut messages);
        });
    let templates_dir = state.templates_dir.trim().to_owned();
    let android_keystore = state.android_keystore.trim().to_owned();
    let android_alias = state.android_alias.trim().to_owned();
    drop(state);
    if templates_dir != ctx.pref(ids::EXPORT, PREF_TEMPLATES).unwrap_or("") {
        ctx.set_pref(ids::EXPORT, PREF_TEMPLATES, templates_dir);
    }
    if android_keystore != ctx.pref(ids::EXPORT, PREF_ANDROID_KEYSTORE).unwrap_or("") {
        ctx.set_pref(ids::EXPORT, PREF_ANDROID_KEYSTORE, android_keystore);
    }
    if android_alias != ctx.pref(ids::EXPORT, PREF_ANDROID_ALIAS).unwrap_or("") {
        ctx.set_pref(ids::EXPORT, PREF_ANDROID_ALIAS, android_alias);
    }
    for (error, message) in messages {
        if error {
            ctx.error(message);
        } else {
            ctx.info(message);
        }
    }
}

/// Path of `file` relative to the project (forward slashes), as the exporter
/// joins it onto the project root; absolute when outside the project.
fn project_relative(root: &Path, file: &Path) -> String {
    let rel = file.strip_prefix(root).unwrap_or(file);
    rel.to_string_lossy().replace('\\', "/")
}

fn profile_form(
    ui: &mut Ui,
    state: &mut ExportUi,
    root: &Path,
    messages: &mut Vec<(bool, String)>,
) {
    let texts = ui.texts();
    section_title(ui, &texts.t("export.profile.title"));
    let names: Vec<String> = state.profiles.iter().map(|p| p.name.clone()).collect();
    let mut changed = false;
    ui.horizontal(|ui| {
        let selected = state.selected.min(names.len().saturating_sub(1));
        ComboBox::from_id_salt("export_profile")
            .selected_text(names[selected].clone())
            .show_ui(ui, |ui| {
                for (i, name) in names.iter().enumerate() {
                    ui.selectable_value(&mut state.selected, i, name);
                }
            });
        if ui
            .button(format!(
                "{}  {}",
                icons::PLUS,
                texts.t("export.profile.new")
            ))
            .clicked()
        {
            let base = state.profile().clone();
            let mut next = base.clone();
            next.name = unique_name(&names, &base.name);
            state.profiles.push(next);
            state.selected = state.profiles.len() - 1;
            changed = true;
        }
        if state.profiles.len() > 1
            && ui
                .button(format!("{}  {}", icons::TRASH, texts.t("common.delete")))
                .on_hover_text(texts.t("export.profile.delete_hint"))
                .clicked()
        {
            let at = state.selected.min(state.profiles.len() - 1);
            state.profiles.remove(at);
            state.selected = 0;
            changed = true;
        }
    });
    ui.add_space(4.0);
    let profile = state.profile();
    egui::Grid::new("export_profile_grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            let field = |ui: &mut Ui, label: &str, hint: &str, text: &mut String| {
                ui.label(label);
                let r = ui.add(TextEdit::singleline(text).desired_width(260.0));
                if !hint.is_empty() {
                    r.clone().on_hover_text(hint);
                }
                ui.end_row();
                r.changed()
            };
            changed |= field(
                ui,
                &texts.t("export.field.profile_name"),
                "",
                &mut profile.name,
            );
            changed |= field(
                ui,
                &texts.t("export.field.game_name"),
                &texts.t("export.field.game_name_hint"),
                &mut profile.game_name,
            );
            changed |= field(
                ui,
                &texts.t("export.field.game_id"),
                &texts.t("export.field.game_id_hint"),
                &mut profile.game_id,
            );
            changed |= field(
                ui,
                &texts.t("export.field.version"),
                "",
                &mut profile.version,
            );
            let mut icon = profile.icon.clone().unwrap_or_default();
            ui.label(texts.t("export.field.icon"));
            ui.horizontal(|ui| {
                let r = ui.add(TextEdit::singleline(&mut icon).desired_width(260.0));
                let mut icon_changed = r.changed();
                r.on_hover_text(texts.t("export.field.icon_hint"));
                if ui
                    .button(texts.t("export.field.icon_browse"))
                    .on_hover_text(texts.t("export.field.icon_browse_hint"))
                    .clicked()
                    && let Some(file) = rfd::FileDialog::new()
                        .add_filter(texts.t("export.field.icon_filter"), &["png"])
                        .set_directory(root)
                        .pick_file()
                {
                    icon = project_relative(root, &file);
                    icon_changed = true;
                }
                if icon_changed {
                    profile.icon = (!icon.trim().is_empty()).then_some(icon.clone());
                    changed = true;
                }
            });
            ui.end_row();
            let mut language = profile.default_language.clone().unwrap_or_default();
            if field(
                ui,
                &texts.t("export.field.language"),
                &texts.t("export.field.language_hint"),
                &mut language,
            ) {
                profile.default_language = (!language.trim().is_empty()).then_some(language);
                changed = true;
            }
            ui.label(texts.t("export.field.pak"));
            ui.horizontal(|ui| {
                changed |= ui
                    .selectable_value(&mut profile.pak_compression, PakCompression::Lz4, "LZ4")
                    .changed();
                changed |= ui
                    .selectable_value(
                        &mut profile.pak_compression,
                        PakCompression::None,
                        texts.t("export.field.pak_none"),
                    )
                    .changed();
            });
            ui.end_row();
            ui.label(texts.t("export.field.web"));
            changed |= ui
                .checkbox(
                    &mut profile.web_precompress,
                    texts.t("export.field.web_precompress"),
                )
                .changed();
            ui.end_row();
            if orbit_core::experimental::enabled_3d() {
            ui.label(texts.t("export.field.quality3d"));
            ui.horizontal(|ui| {
                for (value, key) in [
                    (DesktopQuality::Low, "export.quality.low"),
                    (DesktopQuality::High, "export.quality.high"),
                ] {
                    changed |= ui
                        .selectable_value(&mut profile.desktop_quality, value, texts.t(key))
                        .on_hover_text(texts.t(&format!("{key}_hint")))
                        .changed();
                }
            })
            .response
            .on_hover_text(texts.t("export.field.quality3d_hint"));
            ui.end_row();
            }
        });
    if changed {
        state.dirty = true;
    }
    for problem in state.profile().problems() {
        ui.label(RichText::new(format!("{}  {problem}", icons::WARNING)).color(warning(ui)));
    }
    ui.horizontal(|ui| {
        let can_save = state.has_manifest && state.dirty;
        if ui
            .add_enabled(
                can_save,
                egui::Button::new(format!(
                    "{}  {}",
                    icons::FLOPPY_DISK,
                    texts.t("export.profile.save")
                )),
            )
            .clicked()
        {
            match state.save_profiles(root) {
                Ok(()) => messages.push((
                    false,
                    texts.tf("export.profile.saved", &[("file", &MANIFEST_FILE)]),
                )),
                Err(e) => messages.push((
                    true,
                    texts.tf("export.profile.save_failed", &[("error", &e)]),
                )),
            }
        }
        if !state.has_manifest {
            ui.label(RichText::new(texts.t("export.profile.no_manifest")).color(faint(ui)));
        } else if state.dirty {
            ui.label(RichText::new(texts.t("export.profile.dirty")).color(faint(ui)));
        }
    });
}

fn unique_name(existing: &[String], base: &str) -> String {
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !existing.contains(name))
        .unwrap_or_else(|| base.to_owned())
}

fn targets_and_output(ui: &mut Ui, state: &mut ExportUi) {
    let texts = ui.texts();
    section_title(ui, &texts.t("export.targets"));
    let templates = state.templates();
    let available = templates.available();
    let mut changed = false;
    for target in ExportTarget::ALL {
        let present = available.contains(&target);
        let profile = state.profile();
        let mut on = profile.targets.contains(&target);
        let label = if present {
            texts.t(&format!("export.target.{}", target.id()))
        } else {
            format!("{}  {}", texts.t(&format!("export.target.{}", target.id())), texts.t("export.template_missing"))
        };
        let response = ui.checkbox(&mut on, label);
        if !present {
            response.on_hover_text(texts.t("export.template_missing_hint"));
        }
        if on != profile.targets.contains(&target) {
            if on {
                profile.targets.push(target);
            } else {
                profile.targets.retain(|t| *t != target);
            }
            changed = true;
        }
    }
    if changed {
        state.dirty = true;
    }
    ui.add_space(4.0);
    egui::Grid::new("export_output_grid")
        .num_columns(3)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(texts.t("export.output_dir"));
            ui.add(TextEdit::singleline(&mut state.out_dir).desired_width(260.0));
            if ui
                .button(icons::FOLDER_OPEN)
                .on_hover_text(texts.t("export.pick_folder"))
                .clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                state.out_dir = dir.to_string_lossy().into_owned();
            }
            ui.end_row();
            ui.label(texts.t("export.templates"));
            ui.add(
                TextEdit::singleline(&mut state.templates_dir)
                    .hint_text(texts.t("export.templates_default"))
                    .desired_width(260.0),
            )
            .on_hover_text(texts.t("export.templates_hint"));
            if ui
                .button(icons::FOLDER_OPEN)
                .on_hover_text(texts.t("export.pick_folder"))
                .clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                state.templates_dir = dir.to_string_lossy().into_owned();
            }
            ui.end_row();
        });
}

/// Android options: package, orientation, format, tools found, signing.
fn android_section(ui: &mut Ui, state: &mut ExportUi) {
    let texts = ui.texts();
    section_title(ui, &texts.t("export.android.title"));
    let mut changed = false;
    let templates = state.templates();
    let profile = state.profile();
    egui::Grid::new("export_android_grid")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(texts.t("export.android.package"));
            let mut package = profile.android_package.clone().unwrap_or_default();
            let derived = format!("com.orbit.{}", profile.game_id.replace('-', "_"));
            if ui
                .add(
                    TextEdit::singleline(&mut package)
                        .hint_text(derived)
                        .desired_width(260.0),
                )
                .on_hover_text(texts.t("export.android.package_hint"))
                .changed()
            {
                profile.android_package = (!package.trim().is_empty()).then_some(package);
                changed = true;
            }
            ui.end_row();
            ui.label(texts.t("export.android.version_code"));
            let mut code = profile
                .android_version_code
                .map(|c| c.to_string())
                .unwrap_or_default();
            if ui
                .add(TextEdit::singleline(&mut code).desired_width(80.0))
                .on_hover_text(texts.t("export.android.version_code_hint"))
                .changed()
            {
                profile.android_version_code = code.trim().parse().ok();
                changed = true;
            }
            ui.end_row();
            ui.label(texts.t("export.android.orientation"));
            ui.horizontal(|ui| {
                for (value, key) in [
                    (AndroidOrientation::Landscape, "export.android.landscape"),
                    (AndroidOrientation::Portrait, "export.android.portrait"),
                    (AndroidOrientation::Sensor, "export.android.sensor"),
                ] {
                    changed |= ui
                        .selectable_value(&mut profile.android_orientation, value, texts.t(key))
                        .changed();
                }
            });
            ui.end_row();
            ui.label(texts.t("export.android.format"));
            ui.horizontal(|ui| {
                changed |= ui
                    .selectable_value(&mut profile.android_format, AndroidFormat::Apk, "APK")
                    .on_hover_text(texts.t("export.android.apk_hint"))
                    .changed();
                changed |= ui
                    .selectable_value(&mut profile.android_format, AndroidFormat::Aab, "AAB")
                    .on_hover_text(texts.t("export.android.aab_hint"))
                    .changed();
            });
            ui.end_row();
        });
    let wants_aab = profile.android_format == AndroidFormat::Aab;
    if changed {
        state.dirty = true;
    }

    // Tools found on this machine.
    ui.add_space(4.0);
    let detected = state
        .android_tools
        .get_or_insert_with(|| {
            android_tools::detect(&android_tools::process_env, &templates.android_dirs())
        })
        .clone();
    let mut refresh = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(texts.t("export.android.tools")).strong());
        refresh = ui
            .small_button(icons::ARROWS_CLOCKWISE)
            .on_hover_text(texts.t("export.android.refresh"))
            .clicked();
    });
    let shown = |ui: &mut Ui, label: String, found: Option<String>| match found {
        Some(path) => {
            ui.label(RichText::new(format!("{}  {label}: {path}", icons::CHECK)).color(success()));
        }
        None => {
            ui.label(
                RichText::new(format!(
                    "{}  {label}: {}",
                    icons::WARNING,
                    texts.t("export.android.not_found")
                ))
                .color(warning(ui)),
            );
        }
    };
    let path = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
    shown(ui, texts.t("export.android.sdk_build_tools"), path(&detected.build_tools));
    shown(ui, texts.t("export.android.sdk_platform"), path(&detected.android_jar));
    shown(
        ui,
        "JDK".to_owned(),
        detected.jdk.as_ref().map(|j| {
            j.home
                .as_ref()
                .map_or_else(|| "PATH".to_owned(), |h| h.display().to_string())
        }),
    );
    if wants_aab {
        shown(
            ui,
            "bundletool".to_owned(),
            detected.bundletool.as_ref().map(|b| match b {
                android_tools::Bundletool::Jar(p) | android_tools::Bundletool::Program(p) => {
                    p.display().to_string()
                }
            }),
        );
    }
    if detected.sdk.is_none() || detected.build_tools.is_none() || detected.jdk.is_none() {
        ui.label(RichText::new(texts.t("export.android.install_hint")).color(faint(ui)));
    }
    if refresh {
        state.android_tools = None;
    }

    // Signing.
    ui.add_space(4.0);
    ui.label(RichText::new(texts.t("export.android.signing")).strong());
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.android_debug, false, texts.t("export.android.keystore"));
        ui.selectable_value(&mut state.android_debug, true, texts.t("export.android.debug"))
            .on_hover_text(texts.t("export.android.debug_hint"));
    });
    if state.android_debug {
        ui.label(RichText::new(texts.t("export.android.debug_hint")).color(faint(ui)));
        return;
    }
    egui::Grid::new("export_android_signing_grid")
        .num_columns(3)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(texts.t("export.android.keystore_file"));
            ui.add(TextEdit::singleline(&mut state.android_keystore).desired_width(260.0))
                .on_hover_text(texts.t("export.android.keystore_hint"));
            if ui
                .button(icons::FOLDER_OPEN)
                .on_hover_text(texts.t("export.android.pick_keystore"))
                .clicked()
                && let Some(file) = rfd::FileDialog::new().pick_file()
            {
                state.android_keystore = file.to_string_lossy().into_owned();
            }
            ui.end_row();
            ui.label(texts.t("export.android.alias"));
            ui.add(TextEdit::singleline(&mut state.android_alias).desired_width(260.0));
            ui.end_row();
            ui.label(texts.t("export.android.password"));
            ui.add(
                TextEdit::singleline(&mut state.android_password)
                    .password(true)
                    .desired_width(260.0),
            )
            .on_hover_text(texts.t("export.android.password_hint"));
            ui.end_row();
        });
}

fn start_export(
    state: &mut ExportUi,
    ctx: &mut EditorContext<'_>,
    info: &orbit_editor_api::ProjectInfo,
) {
    let profile = state.profile().clone();
    let problems = profile.problems();
    if !problems.is_empty() {
        state.result = Some(Err(problems.join("\n")));
        return;
    }
    let project = if info.has_manifest {
        info.root.clone()
    } else {
        info.root.join(&info.story_file)
    };
    state.log.clear();
    state.result = None;
    state.fraction = 0.0;
    state.label = ctx.t("export.starting");
    state.server = None;
    let targets = profile.targets.clone();
    let android = state.take_android_options();
    state.job = Some(ExportJob::start(
        project,
        profile,
        targets,
        PathBuf::from(state.out_dir.trim()),
        state.templates(),
        android,
        ctx.state().waker.clone(),
        &ctx.translator(),
    ));
    let message = ctx.t("export.started");
    ctx.info(message);
}

fn run_controls(
    ui: &mut Ui,
    state: &mut ExportUi,
    ctx: &mut EditorContext<'_>,
    info: &orbit_editor_api::ProjectInfo,
    messages: &mut Vec<(bool, String)>,
) {
    let texts = ui.texts();
    if state.running() {
        ui.horizontal(|ui| {
            ui.add(
                ProgressBar::new(state.fraction)
                    .desired_width(ui.available_width() - 110.0)
                    .show_percentage(),
            );
            if ui
                .button(format!("{}  {}", icons::X, texts.t("common.cancel")))
                .clicked()
                && let Some(job) = &state.job
            {
                job.cancel();
                messages.push((false, texts.t("export.cancel_requested")));
            }
        });
        ui.label(RichText::new(&state.label).color(weak(ui)));
        return;
    }
    if info.modified {
        ui.label(
            RichText::new(format!(
                "{}  {}",
                icons::WARNING,
                texts.t("export.unsaved_warning")
            ))
            .color(warning(ui)),
        );
    }
    let ready = !state.profile().targets.is_empty() && state.android_ready();
    if ui
        .add_enabled(
            ready,
            egui::Button::new(
                RichText::new(format!("{}  {}", icons::EXPORT, texts.t("export.run"))).strong(),
            ),
        )
        .clicked()
    {
        if info.modified {
            state.after_save = true;
            ctx.request(EditorRequest::Save);
        } else {
            start_export(state, ctx, info);
        }
    }
    if state.profile().targets.is_empty() {
        ui.label(RichText::new(texts.t("export.pick_target")).color(faint(ui)));
    } else if !ready {
        ui.label(RichText::new(texts.t("export.android.need_signing")).color(faint(ui)));
    }
}

fn journal(ui: &mut Ui, state: &mut ExportUi, messages: &mut Vec<(bool, String)>) {
    let texts = ui.texts();
    if let Some(result) = &state.result {
        match result {
            Err(message) => {
                card(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}  {}",
                            icons::WARNING_CIRCLE,
                            texts.t("export.failed")
                        ))
                        .color(danger(ui))
                        .strong(),
                    );
                    ui.label(RichText::new(message).color(danger(ui)));
                });
            }
            Ok(reports) => {
                let reports = reports.clone();
                card(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}  {}",
                            icons::CHECK_CIRCLE,
                            texts.t("export.done")
                        ))
                        .color(success())
                        .strong(),
                    );
                    for report in &reports {
                        ui.label(texts.tf(
                            "export.report_line",
                            &[
                                ("target", &texts.t(&format!("export.target.{}", report.target.id()))),
                                ("path", &report.output.display()),
                                ("size", &human_size(&texts, report.bytes)),
                            ],
                        ));
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .button(format!(
                                "{}  {}",
                                icons::FOLDER_OPEN,
                                texts.t("export.open_folder")
                            ))
                            .clicked()
                        {
                            let dir = reports
                                .first()
                                .map(|r| {
                                    if r.output.is_dir() {
                                        r.output.clone()
                                    } else {
                                        r.output.parent().map(Path::to_owned).unwrap_or_default()
                                    }
                                })
                                .unwrap_or_default();
                            if let Err(e) = orbit_export::open_path(&dir) {
                                messages.push((
                                    true,
                                    texts.tf(
                                        "export.open_failed",
                                        &[("path", &dir.display()), ("error", &e)],
                                    ),
                                ));
                            }
                        }
                        if let Some(web) = reports.iter().find(|r| r.target == ExportTarget::Web)
                            && ui
                                .button(format!(
                                    "{}  {}",
                                    icons::GLOBE,
                                    texts.t("export.test_browser")
                                ))
                                .clicked()
                        {
                            test_in_browser(state, &web.output, &texts, messages);
                        }
                    });
                    if let Some(server) = &state.server {
                        ui.label(
                            RichText::new(
                                texts.tf("export.local_server", &[("url", &server.url())]),
                            )
                            .color(weak(ui)),
                        );
                        if ui.small_button(texts.t("export.stop_server")).clicked() {
                            state.server = None;
                        }
                    }
                });
            }
        }
        ui.add_space(6.0);
    }
    if state.log.is_empty() {
        return;
    }
    section_title(ui, &texts.t("export.log"));
    ScrollArea::vertical()
        .id_salt("export_log")
        .max_height(220.0)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for line in &state.log {
                let color = match line.level {
                    LogLevel::Warning => warning(ui),
                    LogLevel::Info => weak(ui),
                };
                let text = if line.level == LogLevel::Warning {
                    texts.tf("export.log_warning", &[("text", &line.text)])
                } else {
                    line.text.clone()
                };
                ui.label(RichText::new(text).color(color).small());
            }
        });
}

/// Serves the web export on a free local port and opens it.
fn test_in_browser(
    state: &mut ExportUi,
    dir: &Path,
    texts: &Translator,
    messages: &mut Vec<(bool, String)>,
) {
    state.server = None;
    match Server::start(dir.to_owned(), None, None) {
        Ok(server) => {
            let url = server.url();
            if let Err(e) = orbit_export::open_in_browser(&url) {
                messages.push((
                    true,
                    texts.tf("export.browser_failed", &[("error", &e), ("url", &url)]),
                ));
            } else {
                messages.push((false, texts.tf("export.served", &[("url", &url)])));
            }
            state.server = Some(server);
        }
        Err(e) => messages.push((true, texts.tf("export.server_failed", &[("error", &e)]))),
    }
}

fn human_size(texts: &Translator, bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MIB {
        texts.tf(
            "export.size_mib",
            &[("n", &format!("{:.1}", bytes as f64 / MIB))],
        )
    } else {
        texts.tf(
            "export.size_kib",
            &[("n", &format!("{:.0}", bytes as f64 / 1024.0))],
        )
    }
}

/// Reads the profiles of a manifest (used by tests and the MCP bridge).
#[allow(dead_code)]
pub(crate) fn manifest_profiles(manifest: &ProjectManifest) -> Vec<ExportProfile> {
    manifest.export_profiles.clone()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use orbit_export::testing::{TempDir, demo_project};

    use super::*;

    fn texts() -> Translator {
        crate::i18n::translator(orbit_editor_api::Lang::Fr)
    }

    #[test]
    fn sizes_are_written_in_the_language_of_the_editor() {
        let fr = texts();
        assert_eq!(human_size(&fr, 2048), "2 Kio");
        assert_eq!(human_size(&fr, 3 * 1024 * 1024), "3.0 Mio");
        let en = crate::i18n::translator(orbit_editor_api::Lang::En);
        assert_eq!(human_size(&en, 2048), "2 KiB");
        assert_eq!(human_size(&en, 3 * 1024 * 1024), "3.0 MiB");
    }

    fn wait(job: &mut ExportJob) -> (Vec<LogLine>, Result<Vec<ExportReport>, String>) {
        let started = Instant::now();
        let mut log = Vec::new();
        loop {
            let status = job.poll();
            log.extend(status.new_log);
            if let Some(outcome) = status.outcome {
                return (log, outcome);
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the export hangs"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn fake_templates(dir: &Path) -> Templates {
        std::fs::create_dir_all(dir.join("linux")).unwrap();
        std::fs::write(dir.join("linux/orbit-player"), b"\x7fELF fake player").unwrap();
        Templates::new(vec![dir.to_owned()])
    }

    #[test]
    fn a_job_exports_on_its_own_thread_and_reports_progress() {
        let project_dir = TempDir::new("editor_export_project");
        let templates_dir = TempDir::new("editor_export_templates");
        let out = TempDir::new("editor_export_out");
        demo_project(project_dir.path());
        let woken = Arc::new(AtomicBool::new(false));
        let waker = Waker::default();
        let flag = woken.clone();
        waker.set(move || flag.store(true, Ordering::Relaxed));
        let profile = ExportProfile::new("release", "Demo");
        let mut job = ExportJob::start(
            project_dir.path().to_owned(),
            profile,
            vec![ExportTarget::Linux],
            out.path().to_owned(),
            fake_templates(templates_dir.path()),
            Default::default(),
            waker,
            &texts(),
        );
        let (log, outcome) = wait(&mut job);
        let reports = outcome.expect("export succeeds");
        assert_eq!(reports.len(), 1);
        assert!(reports[0].output.is_file());
        assert!(log.iter().any(|l| l.text.contains("asset(s)")), "{log:?}");
        assert!(woken.load(Ordering::Relaxed), "the window was not woken");
    }

    #[test]
    fn errors_come_back_as_readable_messages_and_jobs_can_be_cancelled() {
        let project_dir = TempDir::new("editor_export_err");
        let out = TempDir::new("editor_export_err_out");
        demo_project(project_dir.path());
        // No template at all.
        let mut job = ExportJob::start(
            project_dir.path().to_owned(),
            ExportProfile::new("release", "Demo"),
            vec![ExportTarget::Windows],
            out.path().to_owned(),
            Templates::new(vec![out.path().join("nowhere")]),
            Default::default(),
            Waker::default(),
            &texts(),
        );
        let (_, outcome) = wait(&mut job);
        assert!(outcome.unwrap_err().contains("template"));
        // A project that does not exist.
        let mut job = ExportJob::start(
            out.path().join("missing"),
            ExportProfile::new("release", "Demo"),
            vec![ExportTarget::Linux],
            out.path().to_owned(),
            Templates::default(),
            Default::default(),
            Waker::default(),
            &texts(),
        );
        assert!(wait(&mut job).1.is_err());
    }

    #[test]
    fn profile_form_state_loads_and_saves_the_manifest() {
        let dir = TempDir::new("editor_export_profiles");
        demo_project(dir.path());
        let mut ui = ExportUi::default();
        ui.ensure_loaded(dir.path(), "story.orbit", true, "Demo");
        assert_eq!(ui.profiles.len(), 1);
        ui.profile().game_name = "Mon jeu".into();
        ui.profile().version = "3.0.0".into();
        ui.dirty = true;
        ui.save_profiles(dir.path()).unwrap();
        assert!(!ui.dirty);
        let again = ExportProject::open(dir.path()).unwrap();
        assert_eq!(again.profiles()[0].game_name, "Mon jeu");
        assert_eq!(again.profiles()[0].version, "3.0.0");
        assert_eq!(unique_name(&["a".into(), "a 2".into()], "a"), "a 3");
    }
}
