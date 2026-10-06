//! Rig level tools of the rig editor: "Duplicate rig…", "Copy clips from
//! another rig…" and "Recenter on origin…". The work itself is done by pure
//! functions of `orbit_project::rigops` (shared with the MCP tools
//! `rig_duplicate`, `rig_copy_clips`, `rig_recenter`, and runnable off the UI
//! thread); this module only asks the questions, applies the result as one
//! undo step and reports it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{Align2, Color32, FontId, RichText, Stroke, Ui, vec2};
use orbit_anim::rig::{Rig, origin_offset};
use orbit_core::AssetKind;
use orbit_editor_api::{EditorContext, icons};
use orbit_project::rigops::{self, CopyClipsReport, CopyClipsSpec};

use super::model::RigDoc;
use super::{Pending, State};
use crate::builtin::ui_util::{faint, warning, weak};

/// A rig the clips come from, with the clips ticked for copying.
pub(super) struct Source {
    label: String,
    rig: Rig,
    picked: BTreeMap<String, bool>,
}

pub(super) enum Dialog {
    Duplicate { name: String, copy_pictures: bool },
    CopyClips { source: Option<Source>, by_hierarchy: bool, rescale: bool },
    Report(CopyClipsReport),
    Recenter { all_skins: bool },
}

/// Cached "the origin is far from the drawing" check of the viewport.
#[derive(Default)]
pub(super) struct OriginCheck {
    key: Option<(u64, String)>,
    pub far: bool,
}

impl State {
    pub fn request_duplicate(&mut self, ctx: &mut EditorContext<'_>) {
        if self.doc.is_none() {
            return;
        }
        if !ctx.has_project() {
            ctx.warn(ctx.t("rig.dup.need_project"));
            return;
        }
        let stem = self
            .doc
            .as_ref()
            .and_then(|d| d.path.as_ref())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .map(|n| n.trim_end_matches(".ron").trim_end_matches(".rig").to_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "character".to_owned());
        let root = self.project_root(ctx);
        let taken = |n: &str| root.join(format!("{}/{n}.rig.ron", orbit_project::rigs::RIG_DIR)).exists();
        let mut name = format!("{stem}_copy");
        let mut i = 2;
        while taken(&name) {
            name = format!("{stem}_copy{i}");
            i += 1;
        }
        self.dialog = Some(Dialog::Duplicate { name, copy_pictures: true });
    }

    pub fn request_copy_clips(&mut self) {
        if self.doc.is_some() {
            self.dialog = Some(Dialog::CopyClips { source: None, by_hierarchy: false, rescale: false });
        }
    }

    pub fn request_recenter(&mut self) {
        if self.doc.is_some() {
            self.dialog = Some(Dialog::Recenter { all_skins: false });
        }
    }

    /// Whether the origin is far from the visible drawing (rest pose, shown
    /// skin); recomputed only when the rig or the skin changes.
    pub fn origin_is_far(&mut self) -> bool {
        let Some(doc) = self.doc.as_ref() else { return false };
        let key = (doc.revision, self.skin.clone());
        if self.origin.key.as_ref() != Some(&key) {
            self.origin.far = doc
                .rig()
                .rest_bounds(Some(&self.skin), false)
                .is_some_and(|b| origin_offset(&b).1);
            self.origin.key = Some(key);
        }
        self.origin.far
    }

    fn open_new_rig(&mut self, ctx: &mut EditorContext<'_>, abs: PathBuf) {
        if self.doc.as_ref().is_some_and(RigDoc::dirty) {
            self.pending = Some(Pending::Open(abs));
        } else {
            self.open_path(&abs, ctx);
        }
    }

    fn duplicate(&mut self, ctx: &mut EditorContext<'_>, name: &str, copy_pictures: bool) -> bool {
        // Painted pictures first: the copy is made from files.
        if !super::pixeltools::save_canvases(ctx, self) {
            return false;
        }
        if super::jobs::busy(ctx, self) {
            return false;
        }
        let root = self.project_root(ctx);
        let Some(rig) = self.doc.as_ref().map(|d| d.rig().clone()) else { return false };
        // The files are copied in the background (pictures can be big).
        let name = name.to_owned();
        super::jobs::start(ctx, self, "rig.task.duplicate", super::jobs::files(Vec::new()), move |_| {
            rigops::duplicate_files(&root, &rig, &name, copy_pictures)
                .map(|done| super::jobs::Out::Duplicated(Box::new(done)))
                .map_err(|e| orbit_editor_api::TaskError::Failed(e.message))
        });
        true
    }

    /// A finished "Duplicate rig": declares the copies and opens the new rig.
    pub(super) fn finish_duplicate(&mut self, ctx: &mut EditorContext<'_>, done: rigops::Duplicated) {
        let root = self.project_root(ctx);
        {
            {
                let rel = done.rig.clone();
                ctx.ops_or_toast(|p| orbit_project::ops::add_asset(p, AssetKind::Rig, &rel));
                for (_, new) in &done.pictures {
                    ctx.ops_or_toast(|p| orbit_project::ops::add_asset(p, AssetKind::Image, new));
                }
                ctx.info(ctx.tf("rig.dup.done", &[("path", &done.rig)]));
                self.open_new_rig(ctx, root.join(&done.rig));
            }
        }
    }

    fn copy_clips(&mut self, ctx: &mut EditorContext<'_>, source: &Source, by_hierarchy: bool, rescale: bool) -> Option<CopyClipsReport> {
        let doc = self.doc.as_mut()?;
        let spec = CopyClipsSpec {
            clips: source.picked.iter().filter(|(_, on)| **on).map(|(n, _)| n.clone()).collect(),
            by_hierarchy,
            rescale,
        };
        let mut rig = doc.rig().clone();
        match rigops::copy_clips(&mut rig, &source.rig, &spec) {
            Ok(report) => {
                doc.replace(rig);
                self.sync();
                Some(report)
            }
            Err(e) => {
                ctx.warn(ctx.tf("rig.copy.err", &[("error", &e.message)]));
                None
            }
        }
    }

    fn recenter(&mut self, ctx: &mut EditorContext<'_>, all_skins: bool) -> bool {
        let skin = self.skin.clone();
        let Some(doc) = self.doc.as_mut() else { return false };
        let mut rig = doc.rig().clone();
        match rigops::recenter(&mut rig, Some(&skin), all_skins) {
            Ok(r) => {
                doc.replace(rig);
                self.sync();
                self.framed = false;
                ctx.info(ctx.tf("rig.center.done", &[("dx", &format!("{:.0}", r.shift[0])), ("dy", &format!("{:.0}", r.shift[1]))]));
                true
            }
            Err(e) => {
                ctx.warn(ctx.tf("rig.center.err", &[("error", &e.message)]));
                false
            }
        }
    }

    /// Reads a source rig of the project (`rel`, relative to the assets
    /// root) or a file picked by the user.
    fn load_source(&self, ctx: &mut EditorContext<'_>, label: &str, rig: Result<Rig, String>) -> Option<Source> {
        match rig {
            Ok(rig) => {
                let picked = rig.clips.keys().map(|n| (n.clone(), true)).collect();
                Some(Source { label: label.to_owned(), rig, picked })
            }
            Err(e) => {
                ctx.warn(ctx.tf("rig.copy.err_read", &[("path", &label), ("error", &e)]));
                None
            }
        }
    }
}

// ----------------------------------------------------------------- dialogs

fn window(ui: &Ui, title: String, add: impl FnOnce(&mut Ui)) {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ui.ctx(), add);
}

pub(super) fn dialogs(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(mut dialog) = st.dialog.take() else { return };
    let mut keep = true;
    let mut next: Option<Dialog> = None;
    match &mut dialog {
        Dialog::Duplicate { name, copy_pictures } => {
            let mut go = false;
            window(ui, ctx.t("rig.dup.title"), |ui| {
                ui.label(ctx.t("rig.dup.name"));
                let r = ui.text_edit_singleline(name);
                ui.checkbox(copy_pictures, ctx.t("rig.dup.copy_pictures")).on_hover_text(ctx.t("rig.dup.copy_pictures_hint"));
                ui.horizontal(|ui| {
                    let ok = !name.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new(ctx.t("rig.dup.ok"))).clicked()
                        || (ok && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        go = true;
                    }
                    if ui.button(ctx.t("rig.cancel")).clicked() {
                        keep = false;
                    }
                });
            });
            if go {
                let (n, c) = (name.clone(), *copy_pictures);
                keep = !st.duplicate(ctx, &n, c);
            }
        }
        Dialog::CopyClips { source, by_hierarchy, rescale } => {
            let mut apply = false;
            let mut pick: Option<(String, Result<Rig, String>)> = None;
            let own = st.project_rel(ctx);
            let root = st.project_root(ctx);
            let project_rigs: Vec<(String, String)> = ctx
                .story()
                .map(|s| {
                    s.assets
                        .assets
                        .iter()
                        .filter(|a| a.kind == AssetKind::Rig && Some(&a.path) != own.as_ref())
                        .map(|a| (a.path.clone(), a.path.rsplit('/').next().unwrap_or(&a.path).to_owned()))
                        .collect()
                })
                .unwrap_or_default();
            window(ui, ctx.t("rig.copy.title"), |ui| {
                ui.label(RichText::new(ctx.t("rig.copy.intro")).small().color(weak(ui)));
                ui.horizontal(|ui| {
                    ui.label(ctx.t("rig.copy.source"));
                    egui::ComboBox::from_id_salt("rig_copy_source")
                        .selected_text(source.as_ref().map_or_else(|| ctx.t("rig.copy.pick"), |s| s.label.clone()))
                        .show_ui(ui, |ui| {
                            if project_rigs.is_empty() {
                                ui.label(RichText::new(ctx.t("rig.copy.no_rigs")).color(weak(ui)));
                            }
                            for (path, name) in &project_rigs {
                                if ui.selectable_label(source.as_ref().is_some_and(|s| &s.label == name), name).clicked() {
                                    pick = Some((name.clone(), orbit_project::rigs::read_rig(&root, path)));
                                }
                            }
                        });
                    if ui.button(ctx.t("rig.copy.browse")).clicked()
                        && let Some(file) = rfd::FileDialog::new().add_filter("Rig", &["ron"]).pick_file()
                    {
                        let label = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        let loaded = std::fs::read_to_string(&file)
                            .map_err(|e| e.to_string())
                            .and_then(|t| orbit_anim::rig::load_rig(&t).map(|l| l.rig).map_err(|e| e.to_string()));
                        pick = Some((label, loaded));
                    }
                });
                if let Some(s) = source.as_mut() {
                    ui.separator();
                    ui.label(RichText::new(ctx.t("rig.copy.clips")).strong());
                    if s.picked.is_empty() {
                        ui.label(RichText::new(ctx.t("rig.copy.no_clips")).color(weak(ui)));
                    }
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for (name, on) in &mut s.picked {
                            let (tracks, dur) = s.rig.clips.get(name).map_or((0, 0.0), |c| (c.tracks.len(), c.duration));
                            ui.checkbox(on, format!("{name}  ({tracks} tracks, {dur:.2}s)"));
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.small_button(ctx.t("rig.copy.all")).clicked() {
                            s.picked.values_mut().for_each(|v| *v = true);
                        }
                        if ui.small_button(ctx.t("rig.copy.none")).clicked() {
                            s.picked.values_mut().for_each(|v| *v = false);
                        }
                    });
                    ui.checkbox(by_hierarchy, ctx.t("rig.copy.by_hierarchy")).on_hover_text(ctx.t("rig.copy.by_hierarchy_hint"));
                    ui.checkbox(rescale, ctx.t("rig.copy.rescale")).on_hover_text(ctx.t("rig.copy.rescale_hint"));
                }
                ui.horizontal(|ui| {
                    let ok = source.as_ref().is_some_and(|s| s.picked.values().any(|v| *v));
                    if ui.add_enabled(ok, egui::Button::new(ctx.t("rig.copy.ok"))).clicked() {
                        apply = true;
                    }
                    if ui.button(ctx.t("rig.cancel")).clicked() {
                        keep = false;
                    }
                });
            });
            if let Some((label, loaded)) = pick {
                *source = st.load_source(ctx, &label, loaded);
            }
            if apply && let Some(s) = source.as_ref() {
                keep = false;
                if let Some(r) = st.copy_clips(ctx, s, *by_hierarchy, *rescale) {
                    let clean = r.unmatched_bones.is_empty()
                        && r.unmatched_slots.is_empty()
                        && r.unmatched_attachments.is_empty()
                        && r.dropped_deform.is_empty()
                        && r.tracks_dropped == 0;
                    if clean {
                        ctx.info(ctx.tf("rig.copy.done", &[("n", &r.copied.len()), ("tracks", &r.tracks_copied)]));
                    } else {
                        next = Some(Dialog::Report(r));
                    }
                }
            }
        }
        Dialog::Report(r) => {
            window(ui, ctx.t("rig.copy.report"), |ui| {
                ui.label(ctx.tf("rig.copy.done", &[("n", &r.copied.len()), ("tracks", &r.tracks_copied)]));
                for (from, to) in &r.copied {
                    ui.label(RichText::new(if from == to { from.clone() } else { format!("{from}  →  {to}") }).small().color(faint(ui)));
                }
                if r.tracks_dropped > 0 {
                    ui.add_space(4.0);
                    ui.label(RichText::new(ctx.tf("rig.copy.dropped", &[("n", &r.tracks_dropped)])).color(warning(ui)));
                }
                for (key, list) in [
                    ("rig.copy.unmatched_bones", &r.unmatched_bones),
                    ("rig.copy.unmatched_slots", &r.unmatched_slots),
                    ("rig.copy.unmatched_attachments", &r.unmatched_attachments),
                    ("rig.copy.dropped_deform", &r.dropped_deform),
                ] {
                    if !list.is_empty() {
                        ui.label(RichText::new(ctx.tf(key, &[("list", &list.join(", "))])).color(warning(ui)));
                    }
                }
                if ui.button(ctx.t("rig.copy.close")).clicked() {
                    keep = false;
                }
            });
        }
        Dialog::Recenter { all_skins } => {
            let measured = st.doc.as_ref().and_then(|d| d.rig().rest_bounds(Some(&st.skin), *all_skins));
            let mut go = false;
            window(ui, ctx.t("rig.center.title"), |ui| {
                ui.label(RichText::new(ctx.t("rig.center.intro")).small().color(weak(ui)));
                match measured {
                    Some(b) => {
                        let ([cx, bottom], _) = origin_offset(&b);
                        ui.label(ctx.tf(
                            "rig.center.bounds",
                            &[
                                ("x0", &format!("{:.0}", b[0])),
                                ("x1", &format!("{:.0}", b[2])),
                                ("y0", &format!("{:.0}", b[1])),
                                ("y1", &format!("{:.0}", b[3])),
                            ],
                        ));
                        ui.label(ctx.tf("rig.center.shift", &[("dx", &format!("{:.0}", -cx)), ("dy", &format!("{:.0}", -bottom))]));
                    }
                    None => {
                        ui.label(RichText::new(ctx.t("rig.center.none")).color(warning(ui)));
                    }
                }
                ui.checkbox(all_skins, ctx.t("rig.center.all_skins"));
                ui.horizontal(|ui| {
                    if ui.add_enabled(measured.is_some(), egui::Button::new(ctx.t("rig.center.ok"))).clicked() {
                        go = true;
                    }
                    if ui.button(ctx.t("rig.cancel")).clicked() {
                        keep = false;
                    }
                });
            });
            if go {
                let all = *all_skins;
                st.recenter(ctx, all);
                keep = false;
            }
        }
    }
    if st.dialog.is_none() {
        st.dialog = next.or(keep.then_some(dialog));
    }
}

// ---------------------------------------------------------- origin marker

/// The rig origin `(0, 0)`: where the character is placed on the stage,
/// drawn as a target; amber when it is far from the drawing.
pub(super) fn paint_origin(ui: &Ui, st: &mut State, painter: &egui::Painter, rect: egui::Rect) {
    let far = st.origin_is_far();
    let center = super::center_of(rect);
    let o = st.view.to_screen(center, [0.0, 0.0]);
    let c = egui::pos2(o[0], o[1]);
    if !rect.expand(12.0).contains(c) {
        return;
    }
    let color = if far { warning(ui) } else { Color32::from_rgb(240, 200, 80) };
    let stroke = Stroke::new(1.5, color);
    painter.circle_stroke(c, 7.0, stroke);
    painter.line_segment([c - vec2(11.0, 0.0), c + vec2(11.0, 0.0)], stroke);
    painter.line_segment([c - vec2(0.0, 11.0), c + vec2(0.0, 11.0)], stroke);
    painter.text(c + vec2(10.0, 8.0), Align2::LEFT_TOP, icons::CROSSHAIR.to_owned() + " 0,0", FontId::proportional(11.0), color);
}
