//! Translations panel: the languages of the project (`i18n/<story>.<lang>.ron`),
//! their coverage, the stale translations (source text edited since), and the
//! PO / CSV export and import for translation tools.

use std::path::{Path, PathBuf};

use egui::Ui;
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, PanelDef, Plugin,
    PluginManifest, icons,
};
use orbit_i18n::{ExchangeFormat, StaleEntry, Translation, files, stale_entries};

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
        reg.panel(PanelDef::new(
            "translations",
            "transl.title",
            icons::TRANSLATE,
            DockSlot::Bottom,
            panel,
        ));
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

/// What the panel knows about the selected language, refreshed about once a
/// second (the files are edited outside the editor too).
#[derive(Clone, Default)]
struct View {
    stamp: f64,
    langs: Vec<String>,
    lang: String,
    translation: Option<Translation>,
    new_lang: String,
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

fn refresh(view: &mut View, path: &Path, now: f64, force: bool) {
    if !force && now - view.stamp < 1.0 {
        return;
    }
    view.stamp = now;
    view.langs = files::list_languages(path);
    if !view.langs.contains(&view.lang) {
        view.lang = view.langs.first().cloned().unwrap_or_default();
    }
    view.translation = if view.lang.is_empty() {
        None
    } else {
        files::load(&files::translation_path(path, &view.lang))
            .ok()
            .flatten()
    };
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let Some(path) = story_path(ctx) else {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(ctx.t("transl.no_project")).color(weak(ui)));
        });
        return;
    };
    let id = egui::Id::new("orbit_translations_view");
    let mut view: View = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let now = ui.input(|i| i.time);
    let first = view.stamp == 0.0;
    refresh(&mut view, &path, now, first);
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
    ui.separator();

    if view.lang.is_empty() {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(ctx.t("transl.none")).color(weak(ui)));
        ui.data_mut(|d| d.insert_temp(id, view));
        return;
    }

    let lang = view.lang.clone();
    let Some(story) = ctx.story().cloned() else {
        return;
    };
    let translation = view.translation.clone().unwrap_or_default();
    let report = orbit_i18n::coverage(&story, &translation);
    let stale: Vec<StaleEntry> = stale_entries(&story.strings, &translation);
    let legacy = translation
        .entries
        .iter()
        .filter(|(k, v)| !v.trim().is_empty() && !translation.source_hashes.contains_key(k))
        .count();

    ui.horizontal_wrapped(|ui| {
        ui.label(ctx.tf(
            "transl.coverage",
            &[
                ("translated", &report.translated),
                ("total", &report.total),
                ("missing", &report.missing.len()),
                ("orphans", &report.orphans.len()),
            ],
        ));
        ui.add(egui::ProgressBar::new(report.ratio()).desired_width(120.0));
    });
    ui.horizontal_wrapped(|ui| {
        if secondary_button(ui, &ctx.t("transl.export_po")).clicked() {
            export(ctx, &story.strings, &path, &lang, ExchangeFormat::Po);
        }
        if secondary_button(ui, &ctx.t("transl.export_csv")).clicked() {
            export(ctx, &story.strings, &path, &lang, ExchangeFormat::Csv);
        }
        if secondary_button(ui, &ctx.t("transl.import")).clicked() {
            import_dialog(ctx);
            force = true;
        }
        if legacy > 0
            && secondary_button(ui, &ctx.t("transl.adopt"))
                .on_hover_text(ctx.tf("transl.adopt_hint", &[("count", &legacy)]))
                .clicked()
        {
            update(ctx, &path, &lang, |t| {
                t.stamp_missing(&story.strings);
            });
            force = true;
        }
    });
    ui.add_space(4.0);
    section_title(ui, &ctx.tf("transl.stale_title", &[("count", &stale.len())]));
    if stale.is_empty() {
        ui.label(egui::RichText::new(ctx.t("transl.stale_none")).color(weak(ui)));
    } else {
        if secondary_button(ui, &ctx.t("transl.accept_all")).clicked() {
            update(ctx, &path, &lang, |t| {
                t.accept_stale(&story.strings);
            });
            force = true;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for s in &stale {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(icons::WARNING).color(warning(ui)));
                        ui.label(format!("{} -> {}", s.source, s.translation));
                    });
                    ui.label(egui::RichText::new(s.id.to_string()).small().color(faint(ui)));
                }
            });
    }
    if force {
        refresh(&mut view, &path, now, true);
    }
    ui.data_mut(|d| d.insert_temp(id, view));
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
    let (Some(path), Some(lang), Some(story)) = (story_path(ctx), selected(ctx), ctx.story().cloned())
    else {
        ctx.warn(ctx.t("transl.none"));
        return;
    };
    export(ctx, &story.strings, &path, &lang, format);
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
