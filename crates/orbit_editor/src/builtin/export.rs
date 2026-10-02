//! Export panel: turns the open project into games (Windows, Linux, web).
//!
//! The work is `orbit_export`; this plugin only edits the export profiles of
//! the manifest, runs an export on a worker thread (the window stays alive,
//! the thread wakes it with the editor's `Waker`), shows the progress and the
//! journal, opens the output folder and serves a web export to try it in the
//! browser. The panel is hidden by default: File > "Exporter le jeu…" or the
//! View menu opens it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use egui::{ComboBox, ProgressBar, RichText, ScrollArea, TextEdit, Ui};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, PanelDef,
    Plugin, PluginManifest, Waker, icons,
};
use orbit_export::{
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

impl Plugin for ExportPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::EXPORT, "Export", "0.1.0")
    }
}

impl EditorPlugin for ExportPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "export",
            "Export",
            icons::EXPORT,
            DockSlot::Hidden,
            panel,
        ));
        reg.action(
            ActionDef::new("open", "Exporter le jeu…", |c| {
                c.request(EditorRequest::ShowPanel(panels::EXPORT.to_owned()));
            })
            .icon(icons::EXPORT)
            .category("Fichier")
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
        waker: Waker,
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
            s.outcome = Some(Err("impossible de démarrer le fil d'export".to_owned()));
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
    job: Option<ExportJob>,
    fraction: f32,
    label: String,
    log: Vec<LogLine>,
    result: Option<Result<Vec<ExportReport>, String>>,
    server: Option<Server>,
    /// The export was asked while the story had unsaved edits: it starts
    /// once the save went through.
    after_save: bool,
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
        self.has_manifest = has_manifest;
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
            ui.label(RichText::new("Aucun projet ouvert").color(weak(ui)));
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
            ui.add_space(8.0);
            run_controls(ui, &mut state, ctx, &info, &mut messages);
            ui.add_space(8.0);
            journal(ui, &mut state, &mut messages);
        });
    let templates_dir = state.templates_dir.trim().to_owned();
    drop(state);
    if templates_dir != ctx.pref(ids::EXPORT, PREF_TEMPLATES).unwrap_or("") {
        ctx.set_pref(ids::EXPORT, PREF_TEMPLATES, templates_dir);
    }
    for (error, message) in messages {
        if error {
            ctx.error(message);
        } else {
            ctx.info(message);
        }
    }
}

fn profile_form(
    ui: &mut Ui,
    state: &mut ExportUi,
    root: &Path,
    messages: &mut Vec<(bool, String)>,
) {
    section_title(ui, "Profil d'export");
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
        if ui.button(format!("{}  Nouveau", icons::PLUS)).clicked() {
            let base = state.profile().clone();
            let mut next = base.clone();
            next.name = unique_name(&names, &base.name);
            state.profiles.push(next);
            state.selected = state.profiles.len() - 1;
            changed = true;
        }
        if state.profiles.len() > 1
            && ui
                .button(format!("{}  Supprimer", icons::TRASH))
                .on_hover_text("Supprime ce profil")
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
            changed |= field(ui, "Nom du profil", "", &mut profile.name);
            changed |= field(ui, "Nom du jeu", "Titre de la fenêtre, nom des fichiers.", &mut profile.game_name);
            changed |= field(
                ui,
                "Identifiant",
                "Dossier des sauvegardes des joueurs : lettres minuscules, chiffres, - et _. Ne le changez plus une fois le jeu publié.",
                &mut profile.game_id,
            );
            changed |= field(ui, "Version", "", &mut profile.version);
            let mut icon = profile.icon.clone().unwrap_or_default();
            if field(ui, "Icône (PNG)", "Chemin relatif au projet, carré, 256x256 recommandé.", &mut icon) {
                profile.icon = (!icon.trim().is_empty()).then_some(icon);
                changed = true;
            }
            let mut language = profile.default_language.clone().unwrap_or_default();
            if field(
                ui,
                "Langue par défaut",
                "Code de langue (fr, en...) utilisé quand le joueur n'a pas choisi ; vide : celle du système.",
                &mut language,
            ) {
                profile.default_language = (!language.trim().is_empty()).then_some(language);
                changed = true;
            }
            ui.label("Compression du pak");
            ui.horizontal(|ui| {
                changed |= ui
                    .selectable_value(&mut profile.pak_compression, PakCompression::Lz4, "LZ4")
                    .changed();
                changed |= ui
                    .selectable_value(&mut profile.pak_compression, PakCompression::None, "Aucune")
                    .changed();
            });
            ui.end_row();
            ui.label("Web");
            changed |= ui
                .checkbox(&mut profile.web_precompress, "Fichiers .br et .gz précompressés")
                .changed();
            ui.end_row();
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
            .add_enabled(can_save, egui::Button::new(format!("{}  Enregistrer les profils", icons::FLOPPY_DISK)))
            .clicked()
        {
            match state.save_profiles(root) {
                Ok(()) => messages.push((false, format!("Profils d'export enregistrés dans {MANIFEST_FILE}."))),
                Err(e) => messages.push((true, format!("Profils non enregistrés : {e}"))),
            }
        }
        if !state.has_manifest {
            ui.label(
                RichText::new("Ce projet n'a pas de manifeste : le profil sert à cet export mais n'est pas enregistré.")
                    .color(faint(ui)),
            );
        } else if state.dirty {
            ui.label(RichText::new("modifications non enregistrées").color(faint(ui)));
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
    section_title(ui, "Cibles");
    let templates = state.templates();
    let available = templates.available();
    let mut changed = false;
    for target in ExportTarget::ALL {
        let present = available.contains(&target);
        let profile = state.profile();
        let mut on = profile.targets.contains(&target);
        let label = if present {
            target.label().to_owned()
        } else {
            format!("{}  (template introuvable)", target.label())
        };
        let response = ui.checkbox(&mut on, label);
        if !present {
            response.on_hover_text(
                "Le template du player de cette cible n'est pas installé : voir `cargo xtask templates` ou le dossier des templates ci-dessous.",
            );
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
            ui.label("Dossier de sortie");
            ui.add(TextEdit::singleline(&mut state.out_dir).desired_width(260.0));
            if ui.button(icons::FOLDER_OPEN).on_hover_text("Choisir un dossier").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                state.out_dir = dir.to_string_lossy().into_owned();
            }
            ui.end_row();
            ui.label("Templates");
            ui.add(
                TextEdit::singleline(&mut state.templates_dir)
                    .hint_text("à côté de l'éditeur (par défaut)")
                    .desired_width(260.0),
            )
            .on_hover_text("Dossier des templates de player (windows/, linux/, web/). Vide : le dossier `templates` à côté de l'éditeur.");
            if ui.button(icons::FOLDER_OPEN).on_hover_text("Choisir un dossier").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                state.templates_dir = dir.to_string_lossy().into_owned();
            }
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
    state.label = "Démarrage".to_owned();
    state.server = None;
    let targets = profile.targets.clone();
    state.job = Some(ExportJob::start(
        project,
        profile,
        targets,
        PathBuf::from(state.out_dir.trim()),
        state.templates(),
        ctx.state().waker.clone(),
    ));
    ctx.info("Export du jeu démarré.");
}

fn run_controls(
    ui: &mut Ui,
    state: &mut ExportUi,
    ctx: &mut EditorContext<'_>,
    info: &orbit_editor_api::ProjectInfo,
    messages: &mut Vec<(bool, String)>,
) {
    if state.running() {
        ui.horizontal(|ui| {
            ui.add(
                ProgressBar::new(state.fraction)
                    .desired_width(ui.available_width() - 110.0)
                    .show_percentage(),
            );
            if ui.button(format!("{}  Annuler", icons::X)).clicked()
                && let Some(job) = &state.job
            {
                job.cancel();
                messages.push((false, "Annulation de l'export demandée.".to_owned()));
            }
        });
        ui.label(RichText::new(&state.label).color(weak(ui)));
        return;
    }
    if info.modified {
        ui.label(
            RichText::new(format!(
                "{}  L'histoire a des modifications non enregistrées : elles seront enregistrées avant l'export.",
                icons::WARNING
            ))
            .color(warning(ui)),
        );
    }
    let ready = !state.profile().targets.is_empty();
    if ui
        .add_enabled(
            ready,
            egui::Button::new(RichText::new(format!("{}  Exporter", icons::EXPORT)).strong()),
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
    if !ready {
        ui.label(RichText::new("Choisissez au moins une cible.").color(faint(ui)));
    }
}

fn journal(ui: &mut Ui, state: &mut ExportUi, messages: &mut Vec<(bool, String)>) {
    if let Some(result) = &state.result {
        match result {
            Err(message) => {
                card(ui, |ui| {
                    ui.label(
                        RichText::new(format!("{}  Export échoué", icons::WARNING_CIRCLE))
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
                        RichText::new(format!("{}  Export terminé", icons::CHECK_CIRCLE))
                            .color(success())
                            .strong(),
                    );
                    for report in &reports {
                        ui.label(format!(
                            "{} : {} ({})",
                            report.target.label(),
                            report.output.display(),
                            human_size(report.bytes)
                        ));
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .button(format!("{}  Ouvrir le dossier", icons::FOLDER_OPEN))
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
                                    format!("Impossible d'ouvrir {} : {e}", dir.display()),
                                ));
                            }
                        }
                        if let Some(web) = reports.iter().find(|r| r.target == ExportTarget::Web)
                            && ui
                                .button(format!("{}  Tester dans le navigateur", icons::GLOBE))
                                .clicked()
                        {
                            test_in_browser(state, &web.output, messages);
                        }
                    });
                    if let Some(server) = &state.server {
                        ui.label(
                            RichText::new(format!("Serveur local : {}", server.url()))
                                .color(weak(ui)),
                        );
                        if ui.small_button("Arrêter le serveur").clicked() {
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
    section_title(ui, "Journal");
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
                let prefix = if line.level == LogLevel::Warning {
                    "avertissement : "
                } else {
                    ""
                };
                ui.label(
                    RichText::new(format!("{prefix}{}", line.text))
                        .color(color)
                        .small(),
                );
            }
        });
}

/// Serves the web export on a free local port and opens it.
fn test_in_browser(state: &mut ExportUi, dir: &Path, messages: &mut Vec<(bool, String)>) {
    state.server = None;
    match Server::start(dir.to_owned(), None, None) {
        Ok(server) => {
            let url = server.url();
            if let Err(e) = orbit_export::open_in_browser(&url) {
                messages.push((
                    true,
                    format!("Le navigateur ne s'ouvre pas ({e}) : ouvrez {url} à la main."),
                ));
            } else {
                messages.push((false, format!("Jeu servi sur {url}.")));
            }
            state.server = Some(server);
        }
        Err(e) => messages.push((
            true,
            format!("Impossible de démarrer le serveur local : {e}"),
        )),
    }
}

fn human_size(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MIB {
        format!("{:.1} Mio", bytes as f64 / MIB)
    } else {
        format!("{:.0} Kio", bytes as f64 / 1024.0)
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
            waker,
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
            Waker::default(),
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
            Waker::default(),
        );
        assert!(wait(&mut job).1.is_err());
    }

    #[test]
    fn profile_form_state_loads_and_saves_the_manifest() {
        let dir = TempDir::new("editor_export_profiles");
        demo_project(dir.path());
        let mut ui = ExportUi::default();
        ui.ensure_loaded(dir.path(), "story.ron", true, "Demo");
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
