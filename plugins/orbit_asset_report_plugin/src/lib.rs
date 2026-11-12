//! "Asset report": an official, read-only editor plugin.
//!
//! A docked panel that lists the declared assets nothing references, the
//! files of the asset folder the project does not declare, the size of every
//! asset and of every type, and the biggest files, with a CSV export. It
//! never deletes or changes a file.
//!
//! * `report.rs`: the numbers and the CSV (pure, unit tested);
//! * `scan.rs`: what is read from the project folder (runs in a background
//!   task, tested on a scratch folder);
//! * this file: the panel.
//!
//! What the editor already shows is not repeated: a declared file that is
//! missing is a finding of the Problems panel (only counted here), and the
//! asset library shows the users of the selected asset (this panel lists all
//! the unused ones at once).

mod report;
mod scan;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use orbit_editor_sdk::egui;
use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::orbit_core::AssetKind;
use orbit_editor_sdk::orbit_project::{asset_users_index, find_assets_root};
use orbit_editor_sdk::{
    ActionDef, DocKey, DockSlot, EditorContext, EditorRequest, Location, PanelDef, TaskError, TaskHandle, icons,
};

use report::{CSV_COLUMNS, DeclaredAsset, Report, Row, SortKey, format_bytes, sort_rows, to_csv};

/// The plugin id (also in `manifest.ron`).
pub const ID: &str = "orbit.asset_report";
/// Full id of the panel (`plugin id` + local id).
const PANEL: &str = "orbit.asset_report.report";
const ROW_HEIGHT: f32 = 20.0;
const TOP_FILES: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Unused,
    Undeclared,
    All,
}

/// What a list shows, kept until the report, the tab, the sort or the filter
/// change.
#[derive(Default)]
struct View {
    valid: bool,
    /// Indices into `Report::rows` (tabs Unused, All) or `Report::undeclared`.
    order: Vec<usize>,
}

struct State {
    handle: RefCell<Option<TaskHandle<Report>>>,
    report: RefCell<Option<Rc<Report>>>,
    /// The project state the report was computed for.
    key: Cell<Option<DocKey>>,
    /// The state the running analysis started from.
    running_key: Cell<Option<DocKey>>,
    error: RefCell<Option<String>>,
    tab: Cell<Tab>,
    sort: Cell<(SortKey, bool)>,
    filter: RefCell<String>,
    view: RefCell<View>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            handle: RefCell::new(None),
            report: RefCell::new(None),
            key: Cell::new(None),
            running_key: Cell::new(None),
            error: RefCell::new(None),
            tab: Cell::new(Tab::Unused),
            sort: Cell::new((SortKey::Size, true)),
            filter: RefCell::new(String::new()),
            view: RefCell::new(View::default()),
        }
    }
}

impl State {
    fn running(&self) -> bool {
        self.handle.borrow().is_some()
    }

    fn invalidate_view(&self) {
        self.view.borrow_mut().valid = false;
    }

    /// Reads the project (the story part is cheap: one pass over it) and
    /// starts the disk part in a background task.
    fn start(&self, ctx: &mut EditorContext<'_>) {
        if self.running() {
            return;
        }
        let (Some(info), Some(story)) = (ctx.project_info(), ctx.story()) else { return };
        let root = info.root.clone();
        let job = scan::Job {
            story_file: root.join(&info.story_file),
            assets_root: find_assets_root(&root, story),
            root,
            declared: story
                .assets
                .assets
                .iter()
                .map(|a| DeclaredAsset { id: a.id, kind: a.kind, path: a.path.clone() })
                .collect(),
            story_uses: asset_users_index(story).into_iter().map(|(id, users)| (id, users.len())).collect(),
        };
        let title = ctx.t("asset_report.task");
        let step = ctx.t("asset_report.task_step");
        let handle = ctx.state().spawn_task(title, None, move |progress| {
            progress.set(0.0, step);
            let inputs = scan::gather(job, &|| progress.is_cancelled()).ok_or(TaskError::Cancelled)?;
            Ok(Report::build(inputs))
        });
        self.running_key.set(Some(ctx.doc_key()));
        *self.error.borrow_mut() = None;
        *self.handle.borrow_mut() = Some(handle);
    }

    /// Takes the result of the analysis once it is done.
    fn poll(&self, ctx: &EditorContext<'_>) {
        let outcome = self.handle.borrow_mut().as_mut().and_then(TaskHandle::poll);
        match outcome {
            None => {}
            Some(result) => {
                *self.handle.borrow_mut() = None;
                match result {
                    Ok(report) => {
                        *self.report.borrow_mut() = Some(Rc::new(report));
                        self.key.set(self.running_key.get());
                        self.invalidate_view();
                    }
                    Err(TaskError::Cancelled) => {}
                    Err(TaskError::Failed(e)) => *self.error.borrow_mut() = Some(e),
                    Err(_) => *self.error.borrow_mut() = Some(ctx.t("asset_report.failed")),
                }
            }
        }
    }
}

fn kind_label(ctx: &EditorContext<'_>, kind: Option<AssetKind>) -> String {
    ctx.t(match kind {
        Some(AssetKind::Image) => "asset_report.kind.image",
        Some(AssetKind::Music) => "asset_report.kind.music",
        Some(AssetKind::Sound) => "asset_report.kind.sound",
        Some(AssetKind::Font) => "asset_report.kind.font",
        Some(AssetKind::Rig) => "asset_report.kind.rig",
        Some(AssetKind::Scene3d) => "asset_report.kind.scene3d",
        None => "asset_report.kind.undeclared",
    })
}

// ------------------------------------------------------------------ export

fn csv_separator(ctx: &EditorContext<'_>) -> char {
    match ctx.pref(ID, "csv_separator") {
        Some(",") => ',',
        _ => ';',
    }
}

fn export_csv(ctx: &mut EditorContext<'_>, report: &Report) {
    let header: Vec<String> = ["status", "type", "path", "size", "file", "uses", "usage"]
        .iter()
        .map(|h| ctx.t(&format!("asset_report.csv.h_{h}")))
        .collect();
    debug_assert_eq!(header.len(), CSV_COLUMNS);
    let text = to_csv(report, &header, csv_separator(ctx));
    let mut dialog = rfd::FileDialog::new()
        .set_title(ctx.t("asset_report.csv_title"))
        .set_file_name(ctx.t("asset_report.csv_default_name"))
        .add_filter("CSV", &["csv"]);
    if let Some(info) = ctx.project_info() {
        dialog = dialog.set_directory(&info.root);
    }
    let Some(path) = dialog.save_file() else { return };
    let shown = path.display().to_string();
    match std::fs::write(&path, text.as_bytes()) {
        Ok(()) => {
            let message = ctx.tf("asset_report.csv_done", &[("path", &shown)]);
            ctx.info(message);
        }
        Err(e) => {
            let message = ctx.tf("asset_report.csv_failed", &[("path", &shown), ("error", &e)]);
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
        ui.label(ctx.t("asset_report.no_project"));
        return;
    }
    // First display: analyse at once. Later changes of the project only mark
    // the report as out of date (a disk walk per edit would be wasteful).
    if s.report.borrow().is_none() && !s.running() && s.error.borrow().is_none() {
        s.start(ctx);
    }
    let report = s.report.borrow().clone();
    let stale = s.key.get().is_some_and(|k| k != ctx.doc_key());

    ui.horizontal_wrapped(|ui| {
        let can_refresh = !s.running();
        if ui
            .add_enabled(can_refresh, egui::Button::new(format!("{} {}", icons::ARROWS_CLOCKWISE, ctx.t("asset_report.refresh"))))
            .clicked()
        {
            s.start(ctx);
        }
        if ui
            .add_enabled(
                report.is_some(),
                egui::Button::new(format!("{} {}", icons::FILE_CSV, ctx.t("asset_report.export_csv"))),
            )
            .clicked()
            && let Some(report) = &report
        {
            export_csv(ctx, report);
        }
        let mut sep = csv_separator(ctx);
        let before = sep;
        egui::ComboBox::from_id_salt("asset_report.sep")
            .selected_text(if sep == ',' { "," } else { ";" })
            .width(60.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut sep, ';', ctx.t("asset_report.csv_semicolon"));
                ui.selectable_value(&mut sep, ',', ctx.t("asset_report.csv_comma"));
            })
            .response
            .on_hover_text(ctx.t("asset_report.csv_separator"));
        if sep != before {
            ctx.set_pref(ID, "csv_separator", sep.to_string());
        }
        if s.running() {
            ui.spinner();
            ui.label(ctx.t("asset_report.scanning"));
        } else if stale {
            ui.colored_label(ui.visuals().warn_fg_color, ctx.t("asset_report.stale"));
        }
    });
    if let Some(error) = s.error.borrow().as_ref() {
        ui.colored_label(ui.visuals().error_fg_color, ctx.tf("asset_report.failed", &[("error", error)]));
    }
    ui.label(egui::RichText::new(ctx.t("asset_report.read_only")).small().color(weak(ui)));
    let Some(report) = report else { return };
    ui.separator();

    egui::ScrollArea::vertical().id_salt("asset_report.page").auto_shrink([false; 2]).show(ui, |ui| {
        summary(ui, ctx, &report);
        ui.add_space(4.0);
        egui::CollapsingHeader::new(ctx.t("asset_report.biggest")).default_open(true).show(ui, |ui| biggest(ui, &report));
        egui::CollapsingHeader::new(ctx.t("asset_report.by_kind")).default_open(true).show(ui, |ui| {
            by_kind(ui, ctx, &report);
        });
        ui.add_space(4.0);
        lists(ui, ctx, s, &report);
    });
}

fn summary(ui: &mut egui::Ui, ctx: &EditorContext<'_>, report: &Report) {
    let size = |b: u64| format_bytes(b);
    ui.label(ctx.tf(
        "asset_report.summary.declared",
        &[("count", &report.rows.len()), ("size", &size(report.declared_bytes()))],
    ));
    let unused = report.unused().count();
    ui.label(ctx.tf("asset_report.summary.unused", &[("count", &unused), ("size", &size(report.unused_bytes()))]));
    ui.label(ctx.tf(
        "asset_report.summary.undeclared",
        &[("count", &report.undeclared.len()), ("size", &size(report.undeclared_bytes()))],
    ));
    ui.label(egui::RichText::new(ctx.tf("asset_report.summary.total", &[("size", &size(report.total_bytes()))])).strong());
    let missing = report.missing_count();
    if missing > 0 {
        ui.colored_label(ui.visuals().warn_fg_color, ctx.tf("asset_report.summary.missing", &[("count", &missing)]));
    }
}

/// The biggest files, each with a bar relative to the biggest.
fn biggest(ui: &mut egui::Ui, report: &Report) {
    let top = report.biggest(TOP_FILES);
    let max = top.first().map_or(1, |f| f.bytes.max(1)) as f32;
    for file in &top {
        ui.horizontal(|ui| {
            ui.add_sized(
                [70.0, ROW_HEIGHT],
                egui::Label::new(egui::RichText::new(format_bytes(file.bytes)).monospace()),
            );
            let bar = egui::ProgressBar::new(file.bytes as f32 / max).desired_width(120.0).desired_height(8.0);
            ui.add(bar);
            let text = egui::RichText::new(&file.path);
            ui.add(egui::Label::new(if file.declared { text } else { text.italics() }).truncate());
        });
    }
}

fn by_kind(ui: &mut egui::Ui, ctx: &EditorContext<'_>, report: &Report) {
    let total = report.total_bytes().max(1) as f32;
    egui::Grid::new("asset_report.kinds").num_columns(4).striped(true).show(ui, |ui| {
        ui.strong(ctx.t("asset_report.col.kind"));
        ui.strong(ctx.t("asset_report.col.count"));
        ui.strong(ctx.t("asset_report.col.size"));
        ui.strong(ctx.t("asset_report.col.share"));
        ui.end_row();
        for t in report.totals_by_kind() {
            ui.label(kind_label(ctx, t.kind));
            ui.label(t.count.to_string());
            ui.label(format_bytes(t.bytes));
            ui.add(egui::ProgressBar::new(t.bytes as f32 / total).desired_width(100.0).text(format!(
                "{:.0}%",
                100.0 * t.bytes as f32 / total
            )));
            ui.end_row();
        }
    });
}

fn lists(ui: &mut egui::Ui, ctx: &mut EditorContext<'_>, s: &State, report: &Report) {
    let unused = report.unused().count();
    ui.horizontal_wrapped(|ui| {
        for (tab, key, count) in [
            (Tab::Unused, "asset_report.tab.unused", unused),
            (Tab::Undeclared, "asset_report.tab.undeclared", report.undeclared.len()),
            (Tab::All, "asset_report.tab.all", report.rows.len()),
        ] {
            if ui.selectable_label(s.tab.get() == tab, ctx.tf(key, &[("count", &count)])).clicked() {
                s.tab.set(tab);
                s.invalidate_view();
            }
        }
    });
    let hint = match s.tab.get() {
        Tab::Unused => Some("asset_report.tab.unused_hint"),
        Tab::Undeclared => Some("asset_report.tab.undeclared_hint"),
        Tab::All => None,
    };
    if let Some(hint) = hint {
        ui.label(egui::RichText::new(ctx.t(hint)).small().color(weak(ui)));
    }
    let mut filter = s.filter.borrow().clone();
    if ui
        .add(egui::TextEdit::singleline(&mut filter).hint_text(ctx.t("asset_report.filter_hint")).desired_width(f32::INFINITY))
        .changed()
    {
        *s.filter.borrow_mut() = filter;
        s.invalidate_view();
    }

    refresh_view(s, report);
    let view = s.view.borrow();
    if view.order.is_empty() {
        let key = match (s.tab.get(), s.filter.borrow().trim().is_empty()) {
            (Tab::Unused, true) => "asset_report.nothing_unused",
            (Tab::Undeclared, true) => "asset_report.nothing_undeclared",
            _ => "asset_report.nothing",
        };
        ui.label(ctx.t(key));
        return;
    }

    let tab = s.tab.get();
    let (sort_key, descending) = s.sort.get();
    let width = ui.available_width();
    let (w_size, w_kind, w_uses) = (80.0, 110.0, 60.0);
    let show_kind = tab != Tab::Undeclared;
    let show_uses = tab == Tab::All;
    let w_path = (width - w_size - 12.0 - if show_kind { w_kind } else { 0.0 } - if show_uses { w_uses } else { 0.0 }).max(120.0);

    // Header: a click sorts by the column, a second click reverses.
    let mut new_sort = None;
    ui.horizontal(|ui| {
        let mut head = |ui: &mut egui::Ui, w: f32, key: SortKey, label: String| {
            let arrow = if sort_key == key { if descending { " ▼" } else { " ▲" } } else { "" };
            let button = egui::Button::new(egui::RichText::new(format!("{label}{arrow}")).strong()).frame(false);
            if ui.add_sized([w, ROW_HEIGHT], button).clicked() {
                new_sort = Some(if sort_key == key { (key, !descending) } else { (key, key == SortKey::Size || key == SortKey::Uses) });
            }
        };
        head(ui, w_path, SortKey::Path, ctx.t("asset_report.col.path"));
        if show_kind {
            head(ui, w_kind, SortKey::Kind, ctx.t("asset_report.col.kind"));
        }
        if show_uses {
            head(ui, w_uses, SortKey::Uses, ctx.t("asset_report.col.uses"));
        }
        head(ui, w_size, SortKey::Size, ctx.t("asset_report.col.size"));
    });

    let missing_text = ctx.t("asset_report.missing_file");
    let reveal_hint = ctx.t("asset_report.reveal");
    let mut reveal = None;
    egui::ScrollArea::vertical()
        .id_salt("asset_report.list")
        .max_height(ui.available_height().max(240.0))
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, view.order.len(), |ui, range| {
            for i in range {
                let index = view.order[i];
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if tab == Tab::Undeclared {
                        let file = &report.undeclared[index];
                        ui.add_sized([w_path, ROW_HEIGHT], egui::Label::new(&file.path).truncate());
                        ui.add_sized([w_size, ROW_HEIGHT], egui::Label::new(format_bytes(file.len)));
                    } else {
                        let row: &Row = &report.rows[index];
                        let path = ui.add_sized(
                            [w_path, ROW_HEIGHT],
                            egui::Label::new(&row.path).truncate().sense(egui::Sense::click()),
                        );
                        if path.on_hover_text(&reveal_hint).clicked() {
                            reveal = Some(row.id);
                        }
                        ui.add_sized([w_kind, ROW_HEIGHT], egui::Label::new(kind_label(ctx, Some(row.kind))).truncate());
                        if show_uses {
                            ui.add_sized([w_uses, ROW_HEIGHT], egui::Label::new(row.story_uses.to_string()));
                        }
                        match row.size {
                            Some(size) => ui.add_sized([w_size, ROW_HEIGHT], egui::Label::new(format_bytes(size))),
                            None => ui.add_sized(
                                [w_size, ROW_HEIGHT],
                                egui::Label::new(egui::RichText::new(&missing_text).color(ui.visuals().warn_fg_color)),
                            ),
                        };
                    }
                });
            }
        });
    drop(view);
    if let Some(sort) = new_sort {
        s.sort.set(sort);
        s.invalidate_view();
    }
    if let Some(id) = reveal {
        ctx.reveal(Location::Asset(id));
    }
}

/// Rebuilds the displayed order when something changed.
fn refresh_view(s: &State, report: &Report) {
    if s.view.borrow().valid {
        return;
    }
    let (key, descending) = s.sort.get();
    let filter = s.filter.borrow().trim().to_lowercase();
    let matches = |path: &str| filter.is_empty() || path.to_lowercase().contains(&filter);
    let order = match s.tab.get() {
        Tab::Undeclared => {
            let mut idx: Vec<usize> =
                (0..report.undeclared.len()).filter(|&i| matches(&report.undeclared[i].path)).collect();
            idx.sort_by(|&a, &b| {
                let (x, y) = (&report.undeclared[a], &report.undeclared[b]);
                let ord = if key == SortKey::Size { x.len.cmp(&y.len) } else { x.path.cmp(&y.path) };
                (if descending { ord.reverse() } else { ord }).then_with(|| x.path.cmp(&y.path))
            });
            idx
        }
        tab => {
            let candidates: Vec<usize> = (0..report.rows.len())
                .filter(|&i| (tab == Tab::All || report.rows[i].is_unused()) && matches(&report.rows[i].path))
                .collect();
            let rows: Vec<&Row> = candidates.iter().map(|&i| &report.rows[i]).collect();
            sort_rows(&rows, key, descending).into_iter().map(|i| candidates[i]).collect()
        }
    };
    *s.view.borrow_mut() = View { valid: true, order };
}

// ------------------------------------------------------------------ plugin

#[derive(Default)]
pub struct AssetReport {
    state: Rc<State>,
}

impl NativePlugin for AssetReport {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        let s = self.state.clone();
        host.panel(PanelDef::new("report", "asset_report.title", icons::HARD_DRIVES, DockSlot::Hidden, move |ui, ctx| {
            panel(ui, ctx, &s);
        }));
        host.action(
            ActionDef::new("open", "asset_report.open", |ctx| ctx.request(EditorRequest::ShowPanel(PANEL.to_owned())))
                .icon(icons::HARD_DRIVES)
                .category("category.tools")
                .menu("tools", 910),
        );
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(
    AssetReport,
    id = "orbit.asset_report",
    name = "Asset report",
    description = "Read-only report on the project's assets: unreferenced and undeclared files, size per asset and per type, biggest files, CSV export.",
    author = "OrbitEngine contributors",
);
