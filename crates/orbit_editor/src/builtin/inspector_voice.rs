//! Voice acting and achievements in the inspector (M7-05, M7-06): the voice
//! of a line (source language) with its batch import, and the `Unlock` node.

use std::path::{Path, PathBuf};

use egui::{ComboBox, RichText, Ui};
use orbit_core::{AchievementId, AssetId, AssetKind, NodeId, NodeKind, Story};
use orbit_editor_api::fields::asset_picker;
use orbit_editor_api::{EditorContext, UiText, icons};
use orbit_project::voices::{
    VOICE_EXTENSIONS, VoicePlan, apply_voice_plan, plan_voice_import,
};

use super::edit;
use super::inspector::{commit, tfield};
use super::ui_util::{danger, faint, weak};

/// The voice row of a `Say`: the sound, "add a file", "remove", and the
/// batch import.
pub(super) fn voice_row(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    kind: &NodeKind,
) {
    let NodeKind::Say {
        speaker,
        text_id,
        voice,
    } = kind
    else {
        return;
    };
    let Some(story) = ctx.story() else { return };
    let mut chosen: Option<Option<AssetId>> = None;
    let mut add_file = false;
    let mut import: Option<bool> = None;
    tfield(ui, "inspector.say.voice", |ui| {
        match voice {
            Some(current) => {
                if let Some(a) = asset_picker(ui, ("say_voice", id), story, AssetKind::Sound, *current) {
                    chosen = Some(Some(a));
                }
            }
            None => {
                ui.label(RichText::new(ui.t("inspector.say.no_voice")).color(weak(ui)));
                let sounds: Vec<_> = story
                    .assets
                    .assets
                    .iter()
                    .filter(|a| a.kind == AssetKind::Sound)
                    .collect();
                if !sounds.is_empty() {
                    ComboBox::from_id_salt(("say_voice_new", id))
                        .selected_text(ui.t("inspector.say.pick_voice"))
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for a in &sounds {
                                if ui.selectable_label(false, &a.path).clicked() {
                                    chosen = Some(Some(a.id));
                                }
                            }
                        });
                }
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .small_button(format!(
                    "{}  {}",
                    icons::FILE_PLUS,
                    ui.t("inspector.asset.add_file")
                ))
                .on_hover_text(ui.t("inspector.say.add_voice.tip"))
                .clicked()
            {
                add_file = true;
            }
            if voice.is_some()
                && ui
                    .small_button(format!("{}  {}", icons::X, ui.t("inspector.say.remove_voice")))
                    .clicked()
            {
                chosen = Some(None);
            }
            if ui
                .small_button(format!(
                    "{}  {}",
                    icons::FOLDER_OPEN,
                    ui.t("inspector.say.import_voices")
                ))
                .on_hover_text(ui.t("inspector.say.import_voices.tip"))
                .clicked()
            {
                import = Some(false);
            }
            if ui
                .small_button(format!(
                    "{}  {}",
                    icons::TABLE,
                    ui.t("inspector.say.import_voices_csv")
                ))
                .on_hover_text(ui.t("inspector.say.import_voices_csv.tip"))
                .clicked()
            {
                import = Some(true);
            }
        });
    });
    if add_file && let Some(new) = edit::import_asset_dialog(ctx, AssetKind::Sound) {
        chosen = Some(Some(new));
    }
    if let Some(csv) = import {
        import_voices_dialog(ctx, csv);
    }
    if let Some(v) = chosen {
        commit(
            ctx,
            id,
            NodeKind::Say {
                speaker: *speaker,
                text_id: *text_id,
                voice: v,
            },
            "voice",
        );
    }
}

/// All the audio files under `dir`, as (absolute path, path relative to
/// `dir` with forward slashes).
fn audio_files(dir: &Path) -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(PathBuf, String)>) {
        let Ok(read) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| VOICE_EXTENSIONS.contains(&x.to_ascii_lowercase().as_str()))
                && let Ok(rel) = p.strip_prefix(base)
            {
                out.push((p.clone(), rel.to_string_lossy().replace('\\', "/")));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

/// "Import voices": asks for a folder of voice files (named after the lines,
/// see `orbit_project::voices`) or a CSV mapping, copies what is outside the
/// project under `assets/voice/` and sets the voices in one undo step.
pub fn import_voices_dialog(ctx: &mut EditorContext<'_>, csv: bool) {
    let Some(root) = ctx.project_info().map(|i| i.root.clone()) else {
        return;
    };
    let (files, csv_text): (Vec<(PathBuf, String)>, Option<String>) = if csv {
        let Some(file) = rfd::FileDialog::new()
            .set_title(ctx.t("voices.pick_csv"))
            .set_directory(&root)
            .add_filter("CSV", &["csv", "txt"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read_to_string(&file) {
            // Paths of a CSV are relative to the project root.
            Ok(text) => (Vec::new(), Some(text)),
            Err(e) => {
                ctx.error(e.to_string());
                return;
            }
        }
    } else {
        let Some(dir) = rfd::FileDialog::new()
            .set_title(ctx.t("voices.pick_folder"))
            .set_directory(&root)
            .pick_folder()
        else {
            return;
        };
        (audio_files(&dir), None)
    };
    import_voices(ctx, &root, &files, csv_text.as_deref());
}

/// The non-interactive half of [`import_voices_dialog`] (also what the tests
/// drive): `files` are (absolute path, path relative to `base`).
pub fn import_voices(
    ctx: &mut EditorContext<'_>,
    root: &Path,
    files: &[(PathBuf, String)],
    csv: Option<&str>,
) {
    let Some(story) = ctx.story() else { return };
    // Where each file lives in the project: in place when already inside it,
    // else copied under assets/voice/ keeping the sub-folders.
    let project_path = |abs: &Path, rel: &str| -> (String, Option<PathBuf>) {
        match abs.strip_prefix(root) {
            Ok(inside) => (inside.to_string_lossy().replace('\\', "/"), None),
            Err(_) => {
                let target = format!("assets/voice/{rel}");
                (target.clone(), Some(root.join(target)))
            }
        }
    };
    let listed: Vec<(String, Option<(PathBuf, PathBuf)>)> = files
        .iter()
        .map(|(abs, rel)| {
            let (path, copy) = project_path(abs, rel);
            (path, copy.map(|to| (abs.clone(), to)))
        })
        .collect();
    let names: Vec<String> = listed.iter().map(|(p, _)| p.clone()).collect();
    let plan: VoicePlan = plan_voice_import(story, &names, csv);
    // Copy only what the plan uses.
    for m in &plan.matches {
        if let Some((_, Some((from, to)))) = listed.iter().find(|(p, _)| *p == m.path) {
            if let Some(dir) = to.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::copy(from, to) {
                ctx.error(e.to_string());
                return;
            }
        }
    }
    let Some(report) = ctx.ops_or_toast(|p| apply_voice_plan(p, &plan)) else {
        return;
    };
    let translated: usize = report.translated.values().map(|m| m.len()).sum();
    let texts = ctx.translator();
    ctx.info(texts.tf(
        "voices.imported",
        &[
            ("lines", &report.source_voices),
            ("translated", &translated),
            ("unmatched", &plan.unmatched.len()),
        ],
    ));
    // The voices of a translation live in its language file.
    if let Some(info) = ctx.project_info() {
        let story_path = info.root.join(&info.story_file);
        for (lang, voices) in &report.translated {
            match orbit_i18n::files::merge_voices(&story_path, lang, voices) {
                Ok(true) => {}
                Ok(false) => ctx.warn(texts.tf("voices.no_language", &[("lang", lang)])),
                Err(e) => ctx.error(e),
            }
        }
    }
    if !plan.unmatched.is_empty() {
        let shown: Vec<_> = plan.unmatched.iter().take(5).cloned().collect();
        ctx.warn(texts.tf("voices.unmatched", &[("files", &shown.join(", "))]));
    }
}

/// The inspector of an `Unlock` node: the achievement it unlocks.
pub(super) fn unlock_node(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    achievement: AchievementId,
) {
    let Some(story) = ctx.story() else { return };
    let title = |story: &Story, a: AchievementId| {
        story
            .achievements
            .iter()
            .find(|x| x.id == a)
            .and_then(|x| story.text(x.title_id))
            .map(str::to_owned)
    };
    let mut chosen = None;
    tfield(ui, "inspector.unlock.achievement", |ui| {
        let known = title(story, achievement);
        let shown = match &known {
            Some(t) => RichText::new(t.clone()),
            None => RichText::new(ui.t("inspector.unlock.unknown")).color(danger(ui)),
        };
        ComboBox::from_id_salt(("unlock_achievement", id))
            .selected_text(shown)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for a in &story.achievements {
                    let name = story.text(a.title_id).unwrap_or("?").to_owned();
                    if ui.selectable_label(a.id == achievement, name).clicked() {
                        chosen = Some(a.id);
                    }
                }
                if story.achievements.is_empty() {
                    ui.label(RichText::new(ui.t("inspector.unlock.none")).color(weak(ui)));
                }
            });
    });
    ui.label(
        RichText::new(ui.t("inspector.unlock.info"))
            .small()
            .color(faint(ui)),
    );
    if ui
        .small_button(format!(
            "{}  {}",
            icons::TROPHY,
            ui.t("inspector.unlock.open_panel")
        ))
        .clicked()
    {
        ctx.request(orbit_editor_api::EditorRequest::ShowPanel(
            super::panels::ACHIEVEMENTS.to_owned(),
        ));
    }
    if let Some(a) = chosen {
        commit(ctx, id, NodeKind::Unlock { achievement: a }, "achievement");
    }
}
