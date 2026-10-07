//! Translations panel: the languages of the project (`i18n/<story>.<lang>.ron`),
//! their coverage, the stale translations (source text edited since), and the
//! PO / CSV export and import for translation tools.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use egui::Ui;
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, LazyAsync, PanelDef, Plugin,
    PluginManifest, TaskError, icons,
};
use orbit_i18n::{CoverageReport, ExchangeFormat, StaleEntry, Translation, files, stale_entries};

use super::ids;
use super::ui_util::{faint, section_title, secondary_button, warning, weak};

pub struct TranslationsPlugin;

impl Plugin for TranslationsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::TRANSLATIONS, "Translations", "0.1.0")
    }
}

impl EditorPlugin for TranslationsPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        let view = Rc::new(RefCell::new(View::default()));
        let shown = view.clone();
        reg.panel(
            PanelDef::new(
                "translations",
                "transl.title",
                icons::TRANSLATE,
                DockSlot::Bottom,
                move |ui, ctx| panel(ui, ctx, &mut view.borrow_mut()),
            )
            // Back on screen: look at the disk now (the files may have been
            // edited meanwhile); the coverage follows if anything changed.
            .on_shown(move |_| shown.borrow_mut().polled = None),
        );
        reg.action(
            ActionDef::new("export_po", "transl.menu_export_po", |c| {
                export_selected(c, ExchangeFormat::Po);
            })
            .icon(icons::EXPORT)
            .category("category.file")
            .menu("file", 330)
            .enabled(|c| c.has_project()),
        );
        reg.action(
            ActionDef::new("export_csv", "transl.menu_export_csv", |c| {
                export_selected(c, ExchangeFormat::Csv);
            })
            .icon(icons::EXPORT)
            .category("category.file")
            .menu("file", 331)
            .enabled(|c| c.has_project()),
        );
        reg.action(
            ActionDef::new("import", "transl.menu_import", |c| import_dialog(c))
                .icon(icons::DOWNLOAD_SIMPLE)
                .category("category.file")
                .menu("file", 332)
                .enabled(|c| c.has_project()),
        );
    }
}

/// How often the visible panel looks at the `i18n/` folder (the files are
/// edited outside the editor too): a directory listing and one `stat`.
const DISK_POLL: f64 = 1.0;

/// What the coverage of a language was computed for: the story's strings
/// (project generation and revision), the language and its file on disk.
#[derive(Clone, Debug, PartialEq)]
struct ReportKey {
    generation: u64,
    revision: u64,
    lang: String,
    file: Option<(u64, Option<std::time::SystemTime>)>,
}

/// The coverage of the selected language, computed on the worker pool.
struct Report {
    coverage: CoverageReport,
    stale: Vec<StaleEntry>,
    legacy: usize,
}

/// The panel's state. Nothing is read or computed while it is hidden; when
/// shown (or when the story or the file changed) the language file is read
/// and the coverage computed off the UI thread.
#[derive(Default)]
struct View {
    /// Time of the last look at the disk (`i18n/` listing, file stamp).
    polled: Option<f64>,
    langs: Vec<String>,
    lang: String,
    file: Option<(u64, Option<std::time::SystemTime>)>,
    new_lang: String,
    report: LazyAsync<ReportKey, Report>,
}

fn file_stamp(path: &Path) -> Option<(u64, Option<std::time::SystemTime>)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()))
}

fn story_path(ctx: &EditorContext<'_>) -> Option<PathBuf> {
    let info = ctx.project_info()?;
    Some(info.root.join(&info.story_file))
}

fn selected(ctx: &EditorContext<'_>) -> Option<String> {
    // Actions run outside the panel: the first language on disk.
    let path = story_path(ctx)?;
    files::list_languages(&path).into_iter().next()
}

/// Looks at the disk (at most every [`DISK_POLL`], or now with `force`):
/// the languages and the stamp of the selected one's file.
fn poll_disk(view: &mut View, path: &Path, now: f64, force: bool) {
    if !force && view.polled.is_some_and(|t| now - t < DISK_POLL) {
        return;
    }
    view.polled = Some(now);
    view.langs = files::list_languages(path);
    if !view.langs.contains(&view.lang) {
        view.lang = view.langs.first().cloned().unwrap_or_default();
    }
    view.file = if view.lang.is_empty() {
        None
    } else {
        file_stamp(&files::translation_path(path, &view.lang))
    };
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, view: &mut View) {
    let Some(path) = story_path(ctx) else {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(ctx.t("transl.no_project")).color(weak(ui)));
        });
        return;
    };
    let now = ui.input(|i| i.time);
    poll_disk(view, &path, now, false);
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(DISK_POLL));
    let mut force = false;

    ui.horizontal_wrapped(|ui| {
        for lang in view.langs.clone() {
            if ui.selectable_label(view.lang == lang, &lang).clicked() {
                view.lang = lang;
                force = true;
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut view.new_lang)
                .hint_text(ctx.t("transl.new_hint"))
                .desired_width(70.0),
        );
        let tag = view.new_lang.trim().to_owned();
        if ui
            .add_enabled(!tag.is_empty(), egui::Button::new(ctx.t("transl.create")))
            .clicked()
            && let Some(story) = ctx.story()
        {
            let target = files::translation_path(&path, &tag);
            if target.exists() {
                ctx.warn(ctx.tf("transl.exists", &[("lang", &tag)]));
            } else {
                let t = Translation::skeleton(&tag, "", story);
                match files::save(&target, &t) {
                    Ok(()) => {
                        view.lang = tag;
                        view.new_lang.clear();
                        force = true;
                    }
                    Err(e) => ctx.error(e),
                }
            }
        }
    });
    if force {
        poll_disk(view, &path, now, true);
        force = false;
    }
    ui.separator();

    if view.lang.is_empty() {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(ctx.t("transl.none")).color(weak(ui)));
        return;
    }

    let lang = view.lang.clone();
    let doc = ctx.doc_key();
    let key = ReportKey {
        generation: doc.generation,
        revision: doc.revision,
        lang: lang.clone(),
        file: view.file,
    };
    // Only the string table is copied (not the story), once per change
    // while the panel is visible, and at most every 250 ms while typing.
    let strings = || ctx.story().map(|s| s.strings.clone()).unwrap_or_default();
    let file = files::translation_path(&path, &lang);
    view.report.poll(
        true,
        key,
        &ctx.state().tasks,
        &ctx.state().waker,
        || {
            let strings = strings();
            orbit_editor_api::lazy::job(move |_| {
                let translation = files::load(&file)
                    .map_err(TaskError::Failed)?
                    .unwrap_or_default();
                let coverage = orbit_i18n::coverage_of_strings(&strings, &translation);
                let stale = stale_entries(&strings, &translation);
                let legacy = translation
                    .entries
                    .iter()
                    .filter(|(k, v)| !v.trim().is_empty() && !translation.source_hashes.contains_key(k))
                    .count();
                Ok(Report {
                    coverage,
                    stale,
                    legacy,
                })
            })
        },
    );
    if let Some(after) = view.report.retry_after() {
        ui.ctx().request_repaint_after(after);
    }
    let Some(report) = view.report.value() else {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(egui::RichText::new(ctx.t("transl.computing")).color(weak(ui)));
        });
        return;
    };
    let coverage = &report.coverage;

    ui.horizontal_wrapped(|ui| {
        ui.label(ctx.tf(
            "transl.coverage",
            &[
                ("translated", &coverage.translated),
                ("total", &coverage.total),
                ("missing", &coverage.missing.len()),
                ("orphans", &coverage.orphans.len()),
            ],
        ));
        ui.add(egui::ProgressBar::new(coverage.ratio()).desired_width(120.0));
    });
    ui.horizontal_wrapped(|ui| {
        if secondary_button(ui, &ctx.t("transl.export_po")).clicked() {
            export_current(ctx, &path, &lang, ExchangeFormat::Po);
        }
        if secondary_button(ui, &ctx.t("transl.export_csv")).clicked() {
            export_current(ctx, &path, &lang, ExchangeFormat::Csv);
        }
        if secondary_button(ui, &ctx.t("transl.import")).clicked() {
            import_dialog(ctx);
            force = true;
        }
        if report.legacy > 0
            && secondary_button(ui, &ctx.t("transl.adopt"))
                .on_hover_text(ctx.tf("transl.adopt_hint", &[("count", &report.legacy)]))
                .clicked()
            && let Some(strings) = ctx.story().map(|s| s.strings.clone())
        {
            update(ctx, &path, &lang, |t| {
                t.stamp_missing(&strings);
            });
            force = true;
        }
    });
    ui.add_space(4.0);
    section_title(ui, &ctx.tf("transl.stale_title", &[("count", &report.stale.len())]));
    if report.stale.is_empty() {
        ui.label(egui::RichText::new(ctx.t("transl.stale_none")).color(weak(ui)));
    } else {
        if secondary_button(ui, &ctx.t("transl.accept_all")).clicked()
            && let Some(strings) = ctx.story().map(|s| s.strings.clone())
        {
            update(ctx, &path, &lang, |t| {
                t.accept_stale(&strings);
            });
            force = true;
        }
        // Virtualized: one line per entry (the id on hover), only the rows
        // on screen are laid out.
        let row = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row, report.stale.len(), |ui, range| {
                for s in &report.stale[range] {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icons::WARNING).color(warning(ui)));
                        ui.add(
                            egui::Label::new(format!("{} -> {}", s.source, s.translation))
                                .truncate(),
                        )
                        .on_hover_text(egui::RichText::new(s.id.to_string()).color(faint(ui)));
                    });
                }
            });
    }
    if force {
        poll_disk(view, &path, now, true);
        view.report.invalidate();
    }
}

fn update(
    ctx: &mut EditorContext<'_>,
    path: &Path,
    lang: &str,
    edit: impl FnOnce(&mut Translation),
) {
    let target = files::translation_path(path, lang);
    match files::load(&target) {
        Ok(Some(mut t)) => {
            edit(&mut t);
            if let Err(e) = files::save(&target, &t) {
                ctx.error(e);
            }
        }
        Ok(None) => {}
        Err(e) => ctx.error(e),
    }
}

fn export(
    ctx: &mut EditorContext<'_>,
    strings: &orbit_core::StringTable,
    path: &Path,
    lang: &str,
    format: ExchangeFormat,
) {
    let t = match files::load(&files::translation_path(path, lang)) {
        Ok(t) => t.unwrap_or_else(|| Translation::new(lang, "")),
        Err(e) => return ctx.error(e),
    };
    let default_name = format!(
        "{}.{lang}.{}",
        path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        format.extension()
    );
    let Some(target) = rfd::FileDialog::new()
        .set_file_name(default_name)
        .add_filter(format.extension(), &[format.extension()])
        .save_file()
    else {
        return;
    };
    match files::write_text(&target, &t.export(strings, format)) {
        Ok(()) => ctx.info(ctx.tf(
            "transl.exported",
            &[("file", &target.display()), ("count", &strings.entries.len())],
        )),
        Err(e) => ctx.error(e),
    }
}

fn export_selected(ctx: &mut EditorContext<'_>, format: ExchangeFormat) {
    let (Some(path), Some(lang)) = (story_path(ctx), selected(ctx)) else {
        ctx.warn(ctx.t("transl.none"));
        return;
    };
    export_current(ctx, &path, &lang, format);
}

/// Exports `lang` against the current source strings.
fn export_current(ctx: &mut EditorContext<'_>, path: &Path, lang: &str, format: ExchangeFormat) {
    let Some(strings) = ctx.story().map(|s| s.strings.clone()) else {
        ctx.warn(ctx.t("transl.none"));
        return;
    };
    export(ctx, &strings, path, lang, format);
}

fn import_dialog(ctx: &mut EditorContext<'_>) {
    let Some(path) = story_path(ctx) else {
        return;
    };
    let Some(file) = rfd::FileDialog::new()
        .add_filter("PO / CSV", &["po", "pot", "csv"])
        .pick_file()
    else {
        return;
    };
    match import_file(ctx, &path, &file) {
        Ok((lang, changed, stale)) => ctx.info(ctx.tf(
            "transl.imported",
            &[("lang", &lang), ("count", &changed), ("stale", &stale)],
        )),
        Err(e) => ctx.error(e),
    }
}

/// Merges a PO/CSV file into the language file of the project. Returns the
/// language, the strings updated and the stale ones after the merge.
fn import_file(
    ctx: &EditorContext<'_>,
    story_path: &Path,
    file: &Path,
) -> Result<(String, usize, usize), String> {
    let format = file
        .extension()
        .and_then(|e| e.to_str())
        .and_then(ExchangeFormat::from_name)
        .ok_or_else(|| ctx.t("transl.bad_format"))?;
    let text = std::fs::read_to_string(file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    // CSV carries no language: default to the one the panel shows.
    let hint = selected(ctx).unwrap_or_default();
    let imported = Translation::import(&text, format, "", "").or_else(|e| match format {
        ExchangeFormat::Csv if !hint.is_empty() => Translation::import(&text, format, &hint, ""),
        _ => Err(e),
    })
    .map_err(|e| format!("{}: {e}", file.display()))?;
    let story = ctx.story().ok_or_else(|| ctx.t("transl.no_project"))?;
    if imported
        .lang
        .trim()
        .eq_ignore_ascii_case(&story.strings.source_lang)
    {
        return Err(ctx.tf("transl.is_source", &[("lang", &imported.lang)]));
    }
    let target = files::translation_path(story_path, &imported.lang);
    let lang = imported.lang.clone();
    let mut t = files::load(&target)?.unwrap_or_else(|| Translation::new(&lang, ""));
    let changed = t.merge(imported);
    files::save(&target, &t)?;
    Ok((lang, changed, stale_entries(&story.strings, &t).len()))
}
