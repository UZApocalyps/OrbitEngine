//! "Voice script": an official, read-only editor plugin.
//!
//! A window (Tools > Voice script) listing, for one character or all of
//! them, the lines to record in scene order: scene, stable line id, text,
//! expected or assigned voice file and state. Counters per character, the
//! language of the project's translations to choose from, a CSV export
//! (UTF-8 with a byte order mark, for Excel) and a plain-text export for the
//! actors. It never changes the project.
//!
//! * `script.rs`: the lines, counters and exports (pure, unit tested);
//! * `scan.rs`: what is read from the project folder (background task);
//! * this file: the window.

mod scan;
mod script;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use orbit_editor_sdk::egui;
use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::orbit_core::AssetKind;
use orbit_editor_sdk::orbit_project::find_assets_root;
use orbit_editor_sdk::{ActionDef, DocKey, EditorContext, Location, TaskError, TaskHandle, WindowDef, icons};

use script::{
    CSV_COLUMNS, Line, Speaker, SpeakerStats, StateLabels, Stats, TextLabels, VoiceState, build_lines, per_speaker,
    select, to_actor_text, to_csv,
};

/// The plugin id (also in `manifest.ron`).
pub const ID: &str = "orbit.voice_script";
const ROW_HEIGHT: f32 = 20.0;

/// The lines for one language and one state of the project.
struct Cache {
    /// What the lines were built from: the story revision, the disk
    /// reading, the language and the interface language (narrator label).
    key: (u64, u64, u64, Option<String>, String),
    lines: Rc<Vec<Line>>,
    speakers: Rc<Vec<SpeakerStats>>,
}

/// The lines and the counters per speaker.
type Computed = (Rc<Vec<Line>>, Rc<Vec<SpeakerStats>>);

struct State {
    open: Cell<bool>,
    handle: RefCell<Option<TaskHandle<scan::Gathered>>>,
    gathered: RefCell<Option<Rc<scan::Gathered>>>,
    /// Bumped each time a disk reading is taken.
    serial: Cell<u64>,
    /// The `(project, asset folder)` state the disk was read for.
    disk_key: Cell<Option<(u64, u64)>>,
    error: RefCell<Option<String>>,
    /// `None`: the source language.
    lang: RefCell<Option<String>>,
    who: Cell<Option<Speaker>>,
    only_todo: Cell<bool>,
    cache: RefCell<Option<Cache>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: Cell::new(false),
            handle: RefCell::new(None),
            gathered: RefCell::new(None),
            serial: Cell::new(0),
            disk_key: Cell::new(None),
            error: RefCell::new(None),
            lang: RefCell::new(None),
            who: Cell::new(None),
            only_todo: Cell::new(false),
            cache: RefCell::new(None),
        }
    }
}

fn disk_key(key: DocKey) -> (u64, u64) {
    (key.generation, key.asset_epoch)
}

impl State {
    fn running(&self) -> bool {
        self.handle.borrow().is_some()
    }

    /// Starts reading the voice files and translations in the background.
    fn start(&self, ctx: &mut EditorContext<'_>) {
        if self.running() {
            return;
        }
        let (Some(info), Some(story)) = (ctx.project_info(), ctx.story()) else { return };
        let root = info.root.clone();
        let job = scan::Job {
            story_file: root.join(&info.story_file),
            assets_root: find_assets_root(&root, story),
            sounds: story
                .assets
                .assets
                .iter()
                .filter(|a| a.kind == AssetKind::Sound)
                .map(|a| a.path.clone())
                .collect(),
        };
        let title = ctx.t("voice_script.task");
        let step = ctx.t("voice_script.task_step");
        let handle = ctx.state().spawn_task(title, None, move |progress| {
            progress.set(0.0, step);
            scan::gather(job, &|| progress.is_cancelled()).ok_or(TaskError::Cancelled)
        });
        self.disk_key.set(Some(disk_key(ctx.doc_key())));
        *self.error.borrow_mut() = None;
        *self.handle.borrow_mut() = Some(handle);
    }

    fn poll(&self, ctx: &EditorContext<'_>) {
        let outcome = self.handle.borrow_mut().as_mut().and_then(TaskHandle::poll);
        let Some(result) = outcome else { return };
        *self.handle.borrow_mut() = None;
        match result {
            Ok(gathered) => {
                // A language that no longer exists falls back to the source.
                let gone = self
                    .lang
                    .borrow()
                    .as_ref()
                    .is_some_and(|l| !gathered.languages.iter().any(|x| x == l));
                if gone {
                    *self.lang.borrow_mut() = None;
                }
                *self.gathered.borrow_mut() = Some(Rc::new(gathered));
                self.serial.set(self.serial.get() + 1);
            }
            Err(TaskError::Cancelled) => {}
            Err(TaskError::Failed(e)) => *self.error.borrow_mut() = Some(e),
            Err(_) => *self.error.borrow_mut() = Some(ctx.t("voice_script.task")),
        }
    }

    /// The lines of the chosen language, rebuilt only when the story, the
    /// disk reading, the language or the interface language changed.
    fn lines(&self, ctx: &EditorContext<'_>) -> Option<Computed> {
        let story = ctx.story()?;
        let gathered = self.gathered.borrow().clone()?;
        let lang = self.lang.borrow().clone();
        let key = (
            ctx.doc_key().generation,
            ctx.doc_key().revision,
            self.serial.get(),
            lang.clone(),
            ctx.lang().code().to_owned(),
        );
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|c| c.key != key) {
            let translation = lang.as_deref().and_then(|l| gathered.translations.get(l));
            let lines = build_lines(story, lang.as_deref(), translation, &ctx.t("voice_script.narrator"), &gathered.disk);
            let speakers = per_speaker(&lines);
            *cache = Some(Cache { key, lines: Rc::new(lines), speakers: Rc::new(speakers) });
        }
        let c = cache.as_ref()?;
        Some((c.lines.clone(), c.speakers.clone()))
    }
}

// ------------------------------------------------------------------ export

fn csv_separator(ctx: &EditorContext<'_>) -> char {
    match ctx.pref(ID, "csv_separator") {
        Some(",") => ',',
        _ => ';',
    }
}

/// A file name part made of letters and digits only.
fn slug(text: &str) -> String {
    let s: String = text.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    s.trim_matches('-').to_lowercase()
}

/// Everything an export needs, taken before the dialog.
struct Export<'a> {
    lines: &'a [Line],
    who: Option<Speaker>,
    who_name: Option<String>,
    only_todo: bool,
    lang: Option<String>,
}

fn write_export(ctx: &mut EditorContext<'_>, title_key: &str, ext: &str, filter: &str, name: String, content: String) {
    let mut dialog = rfd::FileDialog::new()
        .set_title(ctx.t(title_key))
        .set_file_name(format!("{name}.{ext}"))
        .add_filter(filter, &[ext]);
    if let Some(info) = ctx.project_info() {
        dialog = dialog.set_directory(&info.root);
    }
    let Some(path) = dialog.save_file() else { return };
    let shown = path.display().to_string();
    match std::fs::write(&path, content.as_bytes()) {
        Ok(()) => {
            let message = ctx.tf("voice_script.export_done", &[("path", &shown)]);
            ctx.info(message);
        }
        Err(e) => {
            let message = ctx.tf("voice_script.export_failed", &[("path", &shown), ("error", &e)]);
            ctx.error(message);
        }
    }
}

fn file_name(ctx: &EditorContext<'_>, e: &Export<'_>) -> String {
    let mut name = ctx.t("voice_script.file_name");
    if let Some(l) = &e.lang {
        name = format!("{name}-{}", slug(l));
    }
    if let Some(n) = &e.who_name {
        name = format!("{name}-{}", slug(n));
    }
    name
}

fn export_csv(ctx: &mut EditorContext<'_>, e: &Export<'_>) {
    let header: Vec<String> = ["number", "scene", "id", "character", "text", "words", "file", "state"]
        .iter()
        .map(|h| ctx.t(&format!("voice_script.csv.h_{h}")))
        .collect();
    debug_assert_eq!(header.len(), CSV_COLUMNS);
    let states = StateLabels {
        recorded: ctx.t("voice_script.state.recorded"),
        pending: ctx.t("voice_script.state.pending"),
        missing: ctx.t("voice_script.state.missing"),
    };
    let chosen = select(e.lines, e.who, e.only_todo);
    let text = to_csv(&chosen, &header, &states, csv_separator(ctx));
    let name = file_name(ctx, e);
    write_export(ctx, "voice_script.export_title_csv", "csv", "CSV", name, text);
}

fn export_text(ctx: &mut EditorContext<'_>, e: &Export<'_>, language: &str) {
    let project = ctx.project_info().map(|i| i.name.clone()).unwrap_or_default();
    let labels = TextLabels {
        title: ctx
            .t("voice_script.text.title")
            .replace("{project}", &project)
            .replace("{language}", language),
        heading: ctx.t("voice_script.text.heading"),
        line: ctx.t("voice_script.text.line"),
        nothing: ctx.t("voice_script.text.nothing"),
    };
    let chosen = select(e.lines, e.who, e.only_todo);
    let text = to_actor_text(&chosen, &labels);
    let name = file_name(ctx, e);
    write_export(ctx, "voice_script.export_title_text", "txt", "Text", name, text);
}

// ---------------------------------------------------------------------- ui

fn state_text(ctx: &EditorContext<'_>, state: VoiceState) -> String {
    ctx.t(match state {
        VoiceState::Recorded => "voice_script.state.recorded",
        VoiceState::PendingImport => "voice_script.state.pending",
        VoiceState::Missing => "voice_script.state.missing",
    })
}

fn state_color(ui: &egui::Ui, state: VoiceState) -> egui::Color32 {
    match state {
        VoiceState::Recorded => egui::Color32::from_rgb(90, 190, 110),
        VoiceState::PendingImport => ui.visuals().warn_fg_color,
        VoiceState::Missing => ui.visuals().error_fg_color,
    }
}

fn window(egui_ctx: &egui::Context, ctx: &mut EditorContext<'_>, s: &State) {
    s.poll(ctx);
    if !s.open.get() {
        return;
    }
    let mut open = true;
    egui::Window::new(ctx.t("voice_script.title"))
        .id(egui::Id::new("orbit.voice_script.window"))
        .open(&mut open)
        .default_size([980.0, 560.0])
        .resizable(true)
        .show(egui_ctx, |ui| body(ui, ctx, s));
    if !open {
        s.open.set(false);
    }
}

fn body(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, s: &State) {
    if !ctx.has_project() {
        ui.label(ctx.t("voice_script.no_project"));
        return;
    }
    // Read the disk the first time, and again when the project's files
    // changed (never on a plain edit of the story).
    let current = disk_key(ctx.doc_key());
    if !s.running() && s.error.borrow().is_none() && (s.gathered.borrow().is_none() || s.disk_key.get() != Some(current)) {
        s.start(ctx);
    }
    let gathered = s.gathered.borrow().clone();
    let source_lang = ctx.story().map(|st| st.strings.source_lang.clone()).unwrap_or_default();

    // ---- toolbar
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                !s.running(),
                egui::Button::new(format!("{} {}", icons::ARROWS_CLOCKWISE, ctx.t("voice_script.refresh"))),
            )
            .clicked()
        {
            *s.error.borrow_mut() = None;
            s.start(ctx);
        }
        if s.running() {
            ui.spinner();
            ui.label(ctx.t("voice_script.scanning"));
        }
    });
    if let Some(error) = s.error.borrow().as_ref() {
        ui.colored_label(ui.visuals().error_fg_color, ctx.tf("voice_script.failed", &[("error", error)]));
    }
    ui.label(egui::RichText::new(ctx.t("voice_script.read_only")).small().color(ui.visuals().weak_text_color()));
    let Some(gathered) = gathered else { return };
    let Some((lines, speakers)) = s.lines(ctx) else { return };
    for (lang, error) in &gathered.unreadable {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            ctx.tf("voice_script.unreadable", &[("lang", lang), ("error", error)]),
        );
    }

    // ---- choices
    let mut lang = s.lang.borrow().clone();
    let mut who = s.who.get();
    let mut only_todo = s.only_todo.get();
    // A speaker without lines in this language selection falls back to everybody.
    if who.is_some_and(|w| !speakers.iter().any(|sp| sp.who == w)) {
        who = None;
    }
    let lang_name = |code: &Option<String>| -> String {
        match code {
            None => ctx.tf("voice_script.lang_source", &[("code", &source_lang)]),
            Some(c) => match gathered.translations.get(c).map(|t| t.name.as_str()).filter(|n| !n.is_empty()) {
                Some(name) => format!("{c} - {name}"),
                None => c.clone(),
            },
        }
    };
    let who_name = |w: Option<Speaker>| -> String {
        match w {
            None => ctx.t("voice_script.everyone"),
            Some(w) => speakers.iter().find(|sp| sp.who == w).map(|sp| sp.name.clone()).unwrap_or_default(),
        }
    };
    ui.horizontal_wrapped(|ui| {
        if !gathered.languages.is_empty() {
            ui.label(ctx.t("voice_script.language"));
            egui::ComboBox::from_id_salt("voice_script.lang").selected_text(lang_name(&lang)).show_ui(ui, |ui| {
                ui.selectable_value(&mut lang, None, lang_name(&None));
                for code in gathered.languages.iter().filter(|c| !c.eq_ignore_ascii_case(&source_lang)) {
                    let option = Some(code.clone());
                    let label = lang_name(&option);
                    ui.selectable_value(&mut lang, option, label);
                }
            });
        }
        ui.label(ctx.t("voice_script.character"));
        egui::ComboBox::from_id_salt("voice_script.who").selected_text(who_name(who)).show_ui(ui, |ui| {
            ui.selectable_value(&mut who, None, who_name(None));
            for sp in speakers.iter() {
                ui.selectable_value(&mut who, Some(sp.who), &sp.name);
            }
        });
        ui.checkbox(&mut only_todo, ctx.t("voice_script.only_todo"));
    });
    *s.lang.borrow_mut() = lang.clone();
    s.who.set(who);
    s.only_todo.set(only_todo);

    // ---- exports
    let export = Export {
        lines: &lines,
        who,
        who_name: who.map(|w| who_name(Some(w))),
        only_todo,
        lang: lang.clone(),
    };
    let language_label = lang_name(&lang);
    ui.horizontal_wrapped(|ui| {
        if ui.button(format!("{} {}", icons::FILE_CSV, ctx.t("voice_script.export_csv"))).clicked() {
            export_csv(ctx, &export);
        }
        let mut sep = csv_separator(ctx);
        let before = sep;
        egui::ComboBox::from_id_salt("voice_script.sep")
            .selected_text(if sep == ',' { "," } else { ";" })
            .width(60.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut sep, ';', ctx.t("voice_script.csv_semicolon"));
                ui.selectable_value(&mut sep, ',', ctx.t("voice_script.csv_comma"));
            })
            .response
            .on_hover_text(ctx.t("voice_script.csv_separator"));
        if sep != before {
            ctx.set_pref(ID, "csv_separator", sep.to_string());
        }
        if ui.button(format!("{} {}", icons::FILE_TEXT, ctx.t("voice_script.export_text"))).clicked() {
            export_text(ctx, &export, &language_label);
        }
    });

    // ---- counters
    egui::CollapsingHeader::new(ctx.t("voice_script.counters")).default_open(true).show(ui, |ui| {
        egui::Grid::new("voice_script.counters").striped(true).num_columns(6).show(ui, |ui| {
            for key in ["character", "lines", "words", "recorded", "pending", "missing"] {
                ui.strong(ctx.t(&format!("voice_script.col.{key}")));
            }
            ui.end_row();
            let row = |ui: &mut egui::Ui, name: &str, st: &Stats, bold: bool| {
                let name = egui::RichText::new(name);
                ui.label(if bold { name.strong() } else { name });
                ui.label(st.lines.to_string());
                ui.label(st.words.to_string());
                ui.label(st.recorded.to_string());
                ui.label(st.pending.to_string());
                if st.missing > 0 {
                    ui.colored_label(ui.visuals().error_fg_color, st.missing.to_string());
                } else {
                    ui.label("0");
                }
                ui.end_row();
            };
            for sp in speakers.iter() {
                row(ui, &sp.name, &sp.stats, false);
            }
            row(ui, &ctx.t("voice_script.total"), &Stats::of(lines.iter()), true);
        });
    });
    let untranslated = lines.iter().filter(|l| !l.translated).count();
    if untranslated > 0 {
        ui.label(
            egui::RichText::new(ctx.tf("voice_script.untranslated", &[("count", &untranslated)]))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    }
    ui.separator();

    // ---- lines
    let shown = select(&lines, who, only_todo);
    if shown.is_empty() {
        ui.label(ctx.t("voice_script.nothing"));
        return;
    }
    list(ui, ctx, &shown, who.is_none());
}

fn list(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, shown: &[&Line], show_speaker: bool) {
    let (w_num, w_scene, w_id, w_who, w_file, w_state) = (34.0, 110.0, 200.0, 100.0, 230.0, 90.0);
    let fixed = w_num + w_scene + w_id + w_file + w_state + if show_speaker { w_who } else { 0.0 };
    let w_text = (ui.available_width() - fixed - 14.0).max(160.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let head = |ui: &mut egui::Ui, w: f32, key: &str| {
            ui.add_sized([w, ROW_HEIGHT], egui::Label::new(egui::RichText::new(ctx.t(key)).strong()));
        };
        head(ui, w_num, "voice_script.col.number");
        head(ui, w_scene, "voice_script.col.scene");
        head(ui, w_id, "voice_script.col.id");
        if show_speaker {
            head(ui, w_who, "voice_script.col.character");
        }
        head(ui, w_text, "voice_script.col.text");
        head(ui, w_file, "voice_script.col.file");
        head(ui, w_state, "voice_script.col.state");
    });

    let reveal_hint = ctx.t("voice_script.reveal");
    let pending_hint = ctx.t("voice_script.state.pending_hint");
    let broken_hint = ctx.t("voice_script.state.broken_hint");
    let mut reveal = None;
    egui::ScrollArea::vertical()
        .id_salt("voice_script.list")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, shown.len(), |ui, range| {
            for i in range {
                let line = shown[i];
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_sized([w_num, ROW_HEIGHT], egui::Label::new((i + 1).to_string()));
                    let scene = ui.add_sized(
                        [w_scene, ROW_HEIGHT],
                        egui::Label::new(&line.scene).truncate().sense(egui::Sense::click()),
                    );
                    ui.add_sized(
                        [w_id, ROW_HEIGHT],
                        egui::Label::new(egui::RichText::new(line.id()).monospace().small()).truncate(),
                    );
                    if show_speaker {
                        ui.add_sized([w_who, ROW_HEIGHT], egui::Label::new(&line.speaker).truncate());
                    }
                    let mut text = egui::RichText::new(line.text.replace('\n', " / "));
                    if !line.translated {
                        text = text.color(ui.visuals().weak_text_color()).italics();
                    }
                    let text_cell = ui.add_sized(
                        [w_text, ROW_HEIGHT],
                        egui::Label::new(text).truncate().sense(egui::Sense::click()),
                    );
                    let file = ui.add_sized(
                        [w_file, ROW_HEIGHT],
                        egui::Label::new(egui::RichText::new(&line.expected).small()).truncate(),
                    );
                    file.on_hover_text(&line.expected);
                    let color = state_color(ui, line.state);
                    let state = ui.add_sized(
                        [w_state, ROW_HEIGHT],
                        egui::Label::new(egui::RichText::new(state_text(ctx, line.state)).color(color)),
                    );
                    match (line.state, &line.assigned) {
                        (VoiceState::PendingImport, _) => {
                            state.on_hover_text(&pending_hint);
                        }
                        (VoiceState::Missing, Some(_)) => {
                            state.on_hover_text(&broken_hint);
                        }
                        _ => {}
                    }
                    let clicked = scene.on_hover_text(&reveal_hint).clicked()
                        | text_cell.on_hover_text(format!("{}\n\n{}", line.text, reveal_hint)).clicked();
                    if clicked {
                        reveal = Some(line.node);
                    }
                });
            }
        });
    if let Some(node) = reveal {
        ctx.reveal(Location::Node(node));
    }
}

// ------------------------------------------------------------------ plugin

#[derive(Default)]
pub struct VoiceScript {
    state: Rc<State>,
}

impl NativePlugin for VoiceScript {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        let s = self.state.clone();
        host.window(WindowDef::new("window", move |egui_ctx, ctx| window(egui_ctx, ctx, &s)));
        let s = self.state.clone();
        host.action(
            ActionDef::new("open", "voice_script.open", move |_| s.open.set(!s.open.get()))
                .icon(icons::MICROPHONE_STAGE)
                .category("category.tools")
                .menu("tools", 911),
        );
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(
    VoiceScript,
    id = "orbit.voice_script",
    name = "Voice script",
    description = "Read-only voice-acting script: the lines to record per character, in scene order, with the expected voice file and its status; CSV and actor-friendly text export.",
    author = "OrbitEngine contributors",
);
