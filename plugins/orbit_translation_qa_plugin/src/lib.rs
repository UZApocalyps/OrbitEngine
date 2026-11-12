//! "Translation check": an official, read-only editor plugin.
//!
//! A docked panel that compares every translation with the source text and
//! lists what the Translations panel does not report: variables (`[name]`) and
//! text tags (`{b}`, `{color=..}`...) that differ from the source, identical
//! or blank translations, spaces at the ends, final punctuation, texts much
//! longer than the source, and entries of texts that no longer exist. It
//! never changes a translation file.
//!
//! The Translations panel already reports the missing and the stale
//! translations: they are not repeated here.
//!
//! * `check.rs`: the checks and the CSV (pure, unit tested);
//! * this file: reading the project, the panel and the settings page.

mod check;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashSet};
use std::rc::Rc;

use orbit_editor_sdk::egui;
use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::orbit_core::{NodeKind, Story};
use orbit_editor_sdk::{
    ActionDef, DockSlot, DocKey, EditorContext, EditorRequest, Location, PanelDef, SettingsDef, TaskError, TaskHandle,
    icons,
};

use check::{CSV_COLUMNS, CsvRow, Detail, Issue, Item, Kind, Origin, Settings};

/// The plugin id (also in `manifest.ron`).
pub const ID: &str = "orbit.translation_qa";
/// Full id of the panel (`plugin id` + local id).
const PANEL: &str = "orbit.translation_qa.report";
const ROW_HEIGHT: f32 = 40.0;

// ---------------------------------------------------------------- settings

fn flag(ctx: &EditorContext<'_>, key: &str, default: bool) -> bool {
    match ctx.pref(ID, key) {
        Some("0") => false,
        Some("1") => true,
        _ => default,
    }
}

fn settings(ctx: &EditorContext<'_>) -> Settings {
    let d = Settings::default();
    let percent = ctx
        .pref(ID, "length_percent")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(d.length_percent)
        .clamp(check::MIN_LENGTH_PERCENT, check::MAX_LENGTH_PERCENT);
    Settings {
        identical: flag(ctx, "identical", d.identical),
        punctuation: flag(ctx, "punctuation", d.punctuation),
        length: flag(ctx, "length", d.length),
        length_percent: percent,
    }
}

fn settings_page(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>) {
    let mut current = settings(ctx);
    let before = current;
    ui.checkbox(&mut current.identical, ctx.t("tqa.set.identical"));
    ui.checkbox(&mut current.punctuation, ctx.t("tqa.set.punctuation"));
    ui.checkbox(&mut current.length, ctx.t("tqa.set.length"));
    ui.add_enabled_ui(current.length, |ui| {
        ui.horizontal(|ui| {
            ui.label(ctx.t("tqa.set.length_percent"));
            ui.add(
                egui::DragValue::new(&mut current.length_percent)
                    .range(check::MIN_LENGTH_PERCENT..=check::MAX_LENGTH_PERCENT)
                    .suffix(" %"),
            );
        });
    });
    ui.small(ctx.t("tqa.set.help"));
    if current != before {
        let bit = |b: bool| if b { "1" } else { "0" };
        ctx.set_pref(ID, "identical", bit(current.identical));
        ctx.set_pref(ID, "punctuation", bit(current.punctuation));
        ctx.set_pref(ID, "length", bit(current.length));
        ctx.set_pref(ID, "length_percent", current.length_percent.to_string());
    }
}

// ------------------------------------------------------------------ report

/// The checks of one language.
struct LangReport {
    lang: String,
    name: String,
    issues: Vec<Issue>,
}

/// What the background task produces.
struct Report {
    items: Vec<Item>,
    langs: Vec<LangReport>,
    /// Language files that could not be read: tag and reason.
    unreadable: Vec<(String, String)>,
}

/// What the task needs, all owned.
struct Job {
    story_file: std::path::PathBuf,
    source_lang: String,
    items: Vec<Item>,
    settings: Settings,
}

/// Every string of the story with the node that owns it: lines, then choice
/// labels and notifications in story order, then the strings nothing owns.
fn collect_items(story: &Story) -> Vec<Item> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for scene in &story.scenes {
        for node in &scene.nodes {
            let mut add = |id, origin| {
                if let Some(source) = story.strings.get(id)
                    && seen.insert(id)
                {
                    items.push(Item {
                        id,
                        source: source.to_owned(),
                        origin,
                        node: Some(node.id),
                        scene: scene.name.clone(),
                    });
                }
            };
            match &node.kind {
                NodeKind::Say { text_id, .. } => add(*text_id, Origin::Line),
                NodeKind::Choice { options } => {
                    for o in options {
                        add(o.text_id, Origin::Choice);
                    }
                }
                NodeKind::Notify(n) => add(n.text_id, Origin::Other),
                _ => {}
            }
        }
    }
    for (id, source) in &story.strings.entries {
        if seen.insert(*id) {
            items.push(Item {
                id: *id,
                source: source.clone(),
                origin: Origin::Other,
                node: None,
                scene: String::new(),
            });
        }
    }
    items
}

/// Reads the translation files and runs the checks. `None` when cancelled.
fn run(job: Job, cancelled: &dyn Fn() -> bool) -> Option<Report> {
    let mut langs = Vec::new();
    let mut unreadable = Vec::new();
    for lang in orbit_i18n::files::list_languages(&job.story_file) {
        if cancelled() {
            return None;
        }
        let path = orbit_i18n::files::translation_path(&job.story_file, &lang);
        match orbit_i18n::files::load(&path) {
            Ok(Some(t)) => {
                let issues = check::analyse(&job.items, &t.entries, &job.source_lang, &t.lang, &job.settings);
                let name = if t.name.is_empty() { t.lang.clone() } else { t.name.clone() };
                langs.push(LangReport { lang: t.lang, name, issues });
            }
            Ok(None) => {}
            Err(e) => unreadable.push((lang, e)),
        }
    }
    Some(Report { items: job.items, langs, unreadable })
}

// ------------------------------------------------------------------- state

struct State {
    handle: RefCell<Option<TaskHandle<Report>>>,
    report: RefCell<Option<Rc<Report>>>,
    /// The project state and settings the report was computed for.
    key: Cell<Option<(DocKey, Settings)>>,
    running_key: Cell<Option<(DocKey, Settings)>>,
    error: RefCell<Option<String>>,
    /// The language shown (`None`: all).
    lang: RefCell<Option<String>>,
    /// Kinds the user switched off.
    hidden: RefCell<BTreeSet<Kind>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            handle: RefCell::new(None),
            report: RefCell::new(None),
            key: Cell::new(None),
            running_key: Cell::new(None),
            error: RefCell::new(None),
            lang: RefCell::new(None),
            hidden: RefCell::new(BTreeSet::new()),
        }
    }
}

impl State {
    fn running(&self) -> bool {
        self.handle.borrow().is_some()
    }

    fn start(&self, ctx: &mut EditorContext<'_>) {
        if self.running() {
            return;
        }
        let (Some(info), Some(story)) = (ctx.project_info(), ctx.story()) else { return };
        let job = Job {
            story_file: info.root.join(&info.story_file),
            source_lang: story.strings.source_lang.clone(),
            items: collect_items(story),
            settings: settings(ctx),
        };
        let title = ctx.t("tqa.task");
        let step = ctx.t("tqa.task_step");
        let handle = ctx.state().spawn_task(title, None, move |progress| {
            progress.set(0.0, step);
            run(job, &|| progress.is_cancelled()).ok_or(TaskError::Cancelled)
        });
        self.running_key.set(Some((ctx.doc_key(), settings(ctx))));
        *self.error.borrow_mut() = None;
        *self.handle.borrow_mut() = Some(handle);
    }

    fn poll(&self, ctx: &EditorContext<'_>) {
        let outcome = self.handle.borrow_mut().as_mut().and_then(TaskHandle::poll);
        let Some(result) = outcome else { return };
        *self.handle.borrow_mut() = None;
        match result {
            Ok(report) => {
                let gone = self
                    .lang
                    .borrow()
                    .as_ref()
                    .is_some_and(|l| !report.langs.iter().any(|r| &r.lang == l));
                if gone {
                    *self.lang.borrow_mut() = None;
                }
                *self.report.borrow_mut() = Some(Rc::new(report));
                self.key.set(self.running_key.get());
            }
            Err(TaskError::Cancelled) => {}
            Err(TaskError::Failed(e)) => *self.error.borrow_mut() = Some(e),
            Err(_) => *self.error.borrow_mut() = Some(ctx.t("tqa.task")),
        }
    }
}

// -------------------------------------------------------------------- text

fn kind_label(ctx: &EditorContext<'_>, kind: Kind) -> String {
    ctx.t(&format!("tqa.kind.{}", kind.key()))
}

fn origin_label(ctx: &EditorContext<'_>, origin: Origin) -> String {
    ctx.t(match origin {
        Origin::Line => "tqa.origin.line",
        Origin::Choice => "tqa.origin.choice",
        Origin::Other => "tqa.origin.other",
    })
}

fn join(list: &[String]) -> String {
    list.join(" ")
}

/// The sentence that explains an issue.
fn describe(ctx: &EditorContext<'_>, issue: &Issue) -> String {
    match &issue.detail {
        Detail::Diff { missing, extra } => {
            let mut parts = Vec::new();
            if !missing.is_empty() {
                parts.push(ctx.tf("tqa.d.missing", &[("list", &join(missing))]));
            }
            if !extra.is_empty() {
                parts.push(ctx.tf("tqa.d.extra", &[("list", &join(extra))]));
            }
            parts.join(" ")
        }
        Detail::Unpaired(list) => ctx.tf("tqa.d.unpaired", &[("list", &join(list))]),
        Detail::Ends { leading, trailing } => {
            let mut parts = Vec::new();
            if *leading {
                parts.push(ctx.t("tqa.d.leading"));
            }
            if *trailing {
                parts.push(ctx.t("tqa.d.trailing"));
            }
            parts.join(" ")
        }
        Detail::Punctuation { source, found } => {
            let none = ctx.t("tqa.d.none");
            let show = |s: &String| if s.is_empty() { none.clone() } else { s.clone() };
            ctx.tf("tqa.d.punctuation", &[("source", &show(source)), ("found", &show(found))])
        }
        Detail::Length { source, found, percent } => {
            ctx.tf("tqa.d.length", &[("percent", percent), ("found", found), ("source", source)])
        }
        Detail::None => ctx.t(match issue.kind {
            Kind::Identical => "tqa.d.identical",
            Kind::Empty => "tqa.d.empty",
            Kind::Orphan => "tqa.d.orphan",
            _ => "tqa.d.none",
        }),
    }
}

fn kind_color(ui: &egui::Ui, kind: Kind) -> egui::Color32 {
    match kind {
        Kind::Placeholders | Kind::Tags | Kind::Unpaired | Kind::Empty => ui.visuals().error_fg_color,
        Kind::Orphan => ui.visuals().weak_text_color(),
        _ => ui.visuals().warn_fg_color,
    }
}

// ------------------------------------------------------------------ export

fn csv_separator(ctx: &EditorContext<'_>) -> char {
    match ctx.pref(ID, "csv_separator") {
        Some(",") => ',',
        _ => ';',
    }
}

/// One row shown: language index, issue index.
type Shown = (usize, usize);

fn export_csv(ctx: &mut EditorContext<'_>, report: &Report, shown: &[Shown]) {
    let header: Vec<String> = ["language", "type", "scene", "id", "origin", "source", "translation", "problem"]
        .iter()
        .map(|h| ctx.t(&format!("tqa.csv.h_{h}")))
        .collect();
    debug_assert_eq!(header.len(), CSV_COLUMNS);
    let rows: Vec<CsvRow> = shown
        .iter()
        .map(|&(l, i)| {
            let lang = &report.langs[l];
            let issue = &lang.issues[i];
            let item = issue.item.map(|n| &report.items[n]);
            CsvRow {
                lang: lang.lang.clone(),
                kind: kind_label(ctx, issue.kind),
                scene: item.map(|it| it.scene.clone()).unwrap_or_default(),
                id: issue.id.to_string(),
                origin: item.map(|it| origin_label(ctx, it.origin)).unwrap_or_default(),
                source: item.map(|it| it.source.clone()).unwrap_or_default(),
                translation: issue.text.clone(),
                problem: describe(ctx, issue),
            }
        })
        .collect();
    let text = check::to_csv(&header, &rows, csv_separator(ctx));
    let title = ctx.t("tqa.csv_title");
    let name = ctx.t("tqa.csv_default_name");
    let Some(path) = ctx.save_file(&title, &name, &[("CSV", &["csv"])]) else { return };
    let shown_path = path.display().to_string();
    match std::fs::write(&path, text.as_bytes()) {
        Ok(()) => {
            let message = ctx.tf("tqa.csv_done", &[("path", &shown_path)]);
            ctx.info(message);
        }
        Err(e) => {
            let message = ctx.tf("tqa.csv_failed", &[("path", &shown_path), ("error", &e)]);
            ctx.error(message);
        }
    }
}

// ---------------------------------------------------------------------- ui

fn weak(ui: &egui::Ui) -> egui::Color32 {
    ui.visuals().weak_text_color()
}

fn panel(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, s: &State) {
    s.poll(ctx);
    if !ctx.has_project() {
        ui.add_space(8.0);
        ui.label(ctx.t("tqa.no_project"));
        return;
    }
    let current_settings = settings(ctx);
    // First display, or the settings changed: check at once. A story edit
    // only marks the report as out of date.
    let needs = match s.key.get() {
        None => s.report.borrow().is_none(),
        Some((_, used)) => used != current_settings,
    };
    if needs && !s.running() && s.error.borrow().is_none() {
        s.start(ctx);
    }
    let report = s.report.borrow().clone();
    let stale = s.key.get().is_some_and(|(k, _)| k.generation != ctx.doc_key().generation || k.revision != ctx.doc_key().revision);

    // The rows to show, for the CSV too.
    let lang_filter = s.lang.borrow().clone();
    let shown: Vec<Shown> = report
        .as_ref()
        .map(|r| {
            let hidden = s.hidden.borrow();
            let mut out = Vec::new();
            for (l, lang) in r.langs.iter().enumerate() {
                if lang_filter.as_ref().is_some_and(|f| f != &lang.lang) {
                    continue;
                }
                for (i, issue) in lang.issues.iter().enumerate() {
                    if !hidden.contains(&issue.kind) {
                        out.push((l, i));
                    }
                }
            }
            out
        })
        .unwrap_or_default();

    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!s.running(), egui::Button::new(format!("{} {}", icons::ARROWS_CLOCKWISE, ctx.t("tqa.refresh"))))
            .clicked()
        {
            s.start(ctx);
        }
        if ui
            .add_enabled(
                !shown.is_empty(),
                egui::Button::new(format!("{} {}", icons::FILE_CSV, ctx.t("tqa.export_csv"))),
            )
            .clicked()
            && let Some(report) = &report
        {
            export_csv(ctx, report, &shown);
        }
        let mut sep = csv_separator(ctx);
        let before = sep;
        egui::ComboBox::from_id_salt("tqa.sep")
            .selected_text(if sep == ',' { "," } else { ";" })
            .width(60.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut sep, ';', ctx.t("tqa.csv_semicolon"));
                ui.selectable_value(&mut sep, ',', ctx.t("tqa.csv_comma"));
            })
            .response
            .on_hover_text(ctx.t("tqa.csv_separator"));
        if sep != before {
            ctx.set_pref(ID, "csv_separator", sep.to_string());
        }
        if s.running() {
            ui.spinner();
            ui.label(ctx.t("tqa.scanning"));
        } else if stale {
            ui.colored_label(ui.visuals().warn_fg_color, ctx.t("tqa.stale"));
        }
    });
    if let Some(error) = s.error.borrow().as_ref() {
        ui.colored_label(ui.visuals().error_fg_color, ctx.tf("tqa.failed", &[("error", error)]));
    }
    ui.label(egui::RichText::new(ctx.t("tqa.read_only")).small().color(weak(ui)));
    let Some(report) = report else { return };
    for (lang, error) in &report.unreadable {
        ui.colored_label(ui.visuals().warn_fg_color, ctx.tf("tqa.unreadable", &[("lang", lang), ("error", error)]));
    }
    if report.langs.is_empty() {
        ui.add_space(6.0);
        ui.label(ctx.t("tqa.no_languages"));
        return;
    }
    ui.separator();

    // ---- language filter, with a counter per language
    ui.horizontal_wrapped(|ui| {
        ui.label(ctx.t("tqa.language"));
        let total: usize = report.langs.iter().map(|l| l.issues.len()).sum();
        if ui.selectable_label(lang_filter.is_none(), ctx.tf("tqa.all_languages", &[("count", &total)])).clicked() {
            *s.lang.borrow_mut() = None;
        }
        for l in &report.langs {
            let text = ctx.tf("tqa.lang_count", &[("name", &l.name), ("code", &l.lang), ("count", &l.issues.len())]);
            if ui.selectable_label(lang_filter.as_deref() == Some(l.lang.as_str()), text).clicked() {
                *s.lang.borrow_mut() = Some(l.lang.clone());
            }
        }
    });

    // ---- kind filter, with the counts of the chosen language(s)
    let mut counts = std::collections::BTreeMap::<Kind, usize>::new();
    for lang in &report.langs {
        if lang_filter.as_ref().is_some_and(|f| f != &lang.lang) {
            continue;
        }
        for (k, n) in check::counts(&lang.issues) {
            *counts.entry(k).or_default() += n;
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(ctx.t("tqa.kinds"));
        let mut hidden = s.hidden.borrow_mut();
        for kind in Kind::ALL {
            let n = counts.get(&kind).copied().unwrap_or(0);
            let mut on = !hidden.contains(&kind);
            let text = ctx.tf("tqa.kind_count", &[("kind", &kind_label(ctx, kind)), ("count", &n)]);
            if ui.toggle_value(&mut on, text).on_hover_text(ctx.t(&format!("tqa.kind_help.{}", kind.key()))).changed() {
                if on {
                    hidden.remove(&kind);
                } else {
                    hidden.insert(kind);
                }
            }
        }
    });
    ui.add_space(2.0);

    if shown.is_empty() {
        ui.label(ctx.t(if counts.is_empty() { "tqa.nothing" } else { "tqa.nothing_shown" }));
        return;
    }
    ui.label(egui::RichText::new(ctx.tf("tqa.shown", &[("count", &shown.len())])).small().color(weak(ui)));

    let show_lang = lang_filter.is_none() && report.langs.len() > 1;
    let reveal_hint = ctx.t("tqa.reveal");
    let mut reveal = None;
    egui::ScrollArea::vertical()
        .id_salt("tqa.list")
        .auto_shrink([false; 2])
        .show_rows(ui, ROW_HEIGHT, shown.len(), |ui, range| {
            for &(l, i) in &shown[range] {
                let lang = &report.langs[l];
                let issue = &lang.issues[i];
                let item = issue.item.map(|n| &report.items[n]);
                let node = item.and_then(|it| it.node);
                let sense = if node.is_some() { egui::Sense::click() } else { egui::Sense::hover() };
                let mut clicked = false;
                let mut hover = |r: egui::Response| {
                    let r = if node.is_some() { r.on_hover_text(&reveal_hint) } else { r };
                    clicked |= r.clicked();
                };
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        ui.colored_label(kind_color(ui, issue.kind), egui::RichText::new(kind_label(ctx, issue.kind)).strong());
                        if show_lang {
                            ui.label(egui::RichText::new(&lang.lang).monospace());
                        }
                        if let Some(it) = item {
                            let place = if it.scene.is_empty() {
                                origin_label(ctx, it.origin)
                            } else {
                                format!("{} · {}", it.scene, origin_label(ctx, it.origin))
                            };
                            ui.label(egui::RichText::new(place).small().color(weak(ui)));
                        }
                        hover(ui.add(egui::Label::new(describe(ctx, issue)).truncate().sense(sense)));
                    });
                    let line = match item {
                        Some(it) => format!("{}  →  {}", it.source.replace('\n', " "), issue.text.replace('\n', " ")),
                        None => issue.text.replace('\n', " "),
                    };
                    hover(ui.add(egui::Label::new(egui::RichText::new(line).small().color(weak(ui))).truncate().sense(sense)));
                    ui.add_space(2.0);
                });
                if clicked {
                    reveal = node;
                }
            }
        });
    if let Some(node) = reveal {
        ctx.reveal(Location::Node(node));
    }
}

// ------------------------------------------------------------------ plugin

#[derive(Default)]
pub struct TranslationQa {
    state: Rc<State>,
}

impl NativePlugin for TranslationQa {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        let s = self.state.clone();
        host.panel(PanelDef::new("report", "tqa.title", icons::TRANSLATE, DockSlot::Hidden, move |ui, ctx| {
            panel(ui, ctx, &s);
        }));
        host.action(
            ActionDef::new("open", "tqa.open", |ctx| ctx.request(EditorRequest::ShowPanel(PANEL.to_owned())))
                .icon(icons::TRANSLATE)
                .category("category.tools")
                .menu("tools", 920),
        );
        host.settings_page(SettingsDef::new("tqa.settings_title", settings_page));
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(
    TranslationQa,
    id = "orbit.translation_qa",
    name = "Translation check",
    description = "Read-only check of the translations: variables and text tags that differ from the source, identical or empty translations, spaces, final punctuation, too long texts, orphaned entries, CSV export.",
    author = "OrbitEngine contributors",
);
