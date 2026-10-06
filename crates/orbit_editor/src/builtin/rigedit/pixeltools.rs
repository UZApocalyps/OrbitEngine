//! The picture tools of the rig editor: **cut** a part's picture into layers
//! (lasso, polygon, magic wand, then "Cut to new part"), **paint** on it
//! (brush, eraser, clone stamp, fill bucket, eyedropper), a blank **new
//! layer** to paint on (closed eyelids...), and **edit in another
//! application** with a reload when the file changes.
//!
//! Editor only (the game never runs any of it) and idle when unused: the
//! pixels live in the rig document ([`super::model::Canvas`]) so a stroke or
//! a cut is one undo step with the rig; the viewport uploads only the
//! pictures whose pixels changed; the external application is followed
//! through the editor's existing asset watcher (no thread, no polling here).
//!
//! The original file of a picture is kept safe: the first edit of a picture
//! the editor did not make writes a copy (`<name>_edit.png`) and the rig
//! points to it, unless "Paint over the original file" is ticked.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::SystemTime;

use egui::{Color32, Pos2, Rect, RichText, Ui, vec2};
use orbit_anim::rig::{Affine, Attachment, Rig, SlotId};
use orbit_core::AssetKind;
use orbit_editor_api::{EditorContext, icons};
use orbit_project::rigmesh::pixels::{self, Mask};
use orbit_project::rigops;

use super::geom::Tool;
use super::model::{self, DEFAULT_SKIN, OpError};
use super::paint::{self, Brush, Mode};
use super::workflow::{self, Picture};
use super::{State, center_of, pointer_rig, to_pos};
use crate::builtin::ui_util::{accent, faint, weak};
use crate::rig_host::PixelUpload;

/// Preference (plugin `orbit.rig`): program that opens pictures for "Edit in
/// external app" (empty: the system's default application).
pub const PREF_IMAGE_EDITOR: &str = "image_editor";

/// The sub-tool of the cut and paint tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixTool {
    Lasso,
    Polygon,
    Wand,
    Brush,
    Eraser,
    Clone,
    Fill,
    Picker,
}

/// The "New layer…" form.
pub struct NewLayer {
    pub name: String,
    pub w: u32,
    pub h: u32,
}

/// `(length, modification time)` of a file, `None` when missing.
type Stamp = Option<(u64, Option<SystemTime>)>;

fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()))
}

pub struct PixelState {
    pub cut_tool: PixTool,
    pub paint_tool: PixTool,
    pub brush: Brush,
    pub tolerance: u8,
    pub contiguous: bool,
    /// Cut parts go behind the source instead of in front.
    pub below: bool,
    pub fill_hole: bool,
    /// Paint over the original file instead of a copy.
    pub overwrite: bool,
    /// The selection, on the picture of this slot.
    selection: Option<(SlotId, Mask)>,
    mask_rev: u64,
    mask_tex: Option<(u64, egui::TextureHandle)>,
    /// Lasso / polygon points being drawn (picture pixels).
    draft: Vec<[f32; 2]>,
    stroke: Option<(String, paint::Stroke)>,
    clone_source: Option<[f32; 2]>,
    /// Pictures made by the editor (cuts, layers, copies): edited in place.
    owned: HashSet<String>,
    /// Pictures that could not be read (not retried every frame).
    failed: HashSet<String>,
    /// Pictures open in another application, and their file's last state.
    watched: HashMap<String, Stamp>,
    epoch: u64,
    /// Pixel revision of each picture last sent to the viewport host.
    sent: HashMap<String, u64>,
    /// Edited pictures dropped with the previous rig (the host reloads them).
    pub forget: Vec<String>,
    pub new_layer: Option<NewLayer>,
}

impl Default for PixelState {
    fn default() -> Self {
        Self {
            cut_tool: PixTool::Lasso,
            paint_tool: PixTool::Brush,
            brush: Brush::default(),
            tolerance: 24,
            contiguous: true,
            below: false,
            fill_hole: true,
            overwrite: false,
            selection: None,
            mask_rev: 0,
            mask_tex: None,
            draft: Vec::new(),
            stroke: None,
            clone_source: None,
            owned: HashSet::new(),
            failed: HashSet::new(),
            watched: HashMap::new(),
            epoch: 0,
            sent: HashMap::new(),
            forget: Vec::new(),
            new_layer: None,
        }
    }
}

impl PixelState {
    /// Another document: drops the selection and strokes; the pictures the
    /// previous one edited in memory go back to their file in the viewport.
    pub fn reset(&mut self, previous: Vec<String>) {
        let keep = (self.cut_tool, self.paint_tool, self.brush, self.tolerance, self.contiguous, self.below, self.fill_hole);
        let overwrite = self.overwrite;
        let mut forget = std::mem::take(&mut self.forget);
        forget.extend(previous.into_iter().filter(|p| self.sent.contains_key(p)));
        *self = Self::default();
        (self.cut_tool, self.paint_tool, self.brush, self.tolerance, self.contiguous, self.below, self.fill_hole) = keep;
        self.overwrite = overwrite;
        self.forget = forget;
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    fn set_selection(&mut self, s: Option<(SlotId, Mask)>) {
        self.selection = s.filter(|(_, m)| !m.is_empty());
        self.mask_rev += 1;
    }
}

pub fn is_pixel_tool(tool: Tool) -> bool {
    matches!(tool, Tool::Cut | Tool::Paint)
}

fn current(st: &State) -> PixTool {
    if st.tool == Tool::Cut { st.px.cut_tool } else { st.px.paint_tool }
}

// ------------------------------------------------------------------ target

/// The picture the tools work on: the selected slot's, in the shown skin.
pub struct Target {
    pub slot: SlotId,
    pub path: String,
    /// Picture UV (`0..1`) → rig space, setup pose.
    pub map: Affine,
    pub w: u32,
    pub h: u32,
}

impl Target {
    fn to_px(&self, rig_p: [f32; 2]) -> Option<[f32; 2]> {
        let uv = self.map.inverse()?.apply(rig_p);
        Some([uv[0] * self.w as f32, uv[1] * self.h as f32])
    }

    fn to_rig(&self, px: [f32; 2]) -> [f32; 2] {
        self.map.apply([px[0] / self.w.max(1) as f32, px[1] / self.h.max(1) as f32])
    }
}

/// The selected slot, its picture path and its map (no pixels needed).
fn locate(st: &State) -> Option<(SlotId, String, Affine)> {
    let doc = st.doc.as_ref()?;
    let rig = doc.rig();
    let sid = st.selected_slot.clone()?;
    let slot = model::slot(rig, &sid)?;
    let att_id = slot.attachment.as_ref()?;
    let att = rig
        .skins
        .iter()
        .find(|k| k.name == st.skin && k.attachments.contains_key(att_id))
        .or_else(|| rig.skins.iter().find(|k| k.attachments.contains_key(att_id)))
        .and_then(|k| k.attachments.get(att_id))?;
    let map = rigops::picture_map(rig, slot, att)?;
    Some((sid, att.image().to_owned(), map))
}

/// The target, its pixels loaded into the document when needed.
pub fn target(ctx: &mut EditorContext<'_>, st: &mut State) -> Option<Target> {
    let (slot, path, map) = locate(st)?;
    if st.doc.as_ref()?.canvas(&path).is_none() && !st.px.failed.contains(&path) {
        let root = st.project_root(ctx);
        match workflow::load_picture(&root, &path) {
            Ok(p) => st.doc.as_mut()?.set_canvas(&path, p.width, p.height, p.rgba),
            Err(e) => {
                st.px.failed.insert(path.clone());
                ctx.error(ctx.tf("rig.mesh.err.image", &[("path", &path), ("error", &e)]));
            }
        }
    }
    let c = st.doc.as_ref()?.canvas(&path)?;
    Some(Target { slot, path, map, w: c.w, h: c.h })
}

/// The target when its pixels are already loaded (painting the overlay).
fn loaded_target(st: &State) -> Option<Target> {
    let (slot, path, map) = locate(st)?;
    let c = st.doc.as_ref()?.canvas(&path)?;
    Some(Target { slot, path, map, w: c.w, h: c.h })
}

// ------------------------------------------------------------------- files

/// Writes a PNG (folders created).
pub fn write_png(abs: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    use image::ImageEncoder;
    if let Some(dir) = abs.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(rgba, w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    std::fs::write(abs, png).map_err(|e| e.to_string())
}

/// File-name friendly version of `name`.
fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_owned();
    if s.is_empty() { "part".to_owned() } else { s }
}

/// `<folder of beside>/<stem>.png` (or `_2`, `_3`...) not on disk yet.
pub fn free_path(root: &Path, beside: &str, stem: &str) -> String {
    let dir = beside.replace('\\', "/").rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default();
    let base = if dir.is_empty() { stem.to_owned() } else { format!("{dir}/{stem}") };
    (1..)
        .map(|n| if n == 1 { format!("{base}.png") } else { format!("{base}_{n}.png") })
        .find(|rel| !root.join(rel).exists())
        .unwrap_or_default()
}

fn stem_of(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    sanitize(name.rsplit_once('.').map_or(name, |(s, _)| s))
}

/// Declares a picture the editor wrote as an image asset of the project.
fn register(ctx: &mut EditorContext<'_>, rel: &str) {
    if ctx.has_project() && !Path::new(rel).is_absolute() {
        ctx.ops_or_toast(|p| orbit_project::ops::add_asset(p, AssetKind::Image, rel));
    }
}

/// Every attachment showing `from` shows `to` instead.
fn repoint(rig: &mut Rig, from: &str, to: &str) {
    for k in &mut rig.skins {
        for att in k.attachments.values_mut() {
            match att {
                Attachment::Mesh(m) if m.image == from => m.image = to.to_owned(),
                Attachment::Region(r) if r.image == from => r.image = to.to_owned(),
                _ => {}
            }
        }
    }
}

/// The picture to change for an edit of `path`: itself when the editor made
/// it (or "paint over the original" is on), else a copy written now that
/// the rig shows from now on (part of the open undo step).
fn editable(ctx: &mut EditorContext<'_>, st: &mut State, path: &str) -> Option<String> {
    if st.px.overwrite || st.px.owned.contains(path) {
        return Some(path.to_owned());
    }
    let root = st.project_root(ctx);
    let c = st.doc.as_ref()?.canvas(path)?.clone();
    let copy = free_path(&root, path, &format!("{}_edit", stem_of(path)));
    if let Err(e) = write_png(&root.join(&copy), c.w, c.h, &c.rgba) {
        ctx.error(ctx.tf("rig.pixels.write_failed", &[("path", &copy), ("error", &e)]));
        return None;
    }
    register(ctx, &copy);
    let doc = st.doc.as_mut()?;
    doc.set_canvas(&copy, c.w, c.h, c.rgba);
    let _ = doc.edit(|rig| {
        repoint(rig, path, &copy);
        Ok(())
    });
    st.px.owned.insert(copy.clone());
    if let Some(s) = st.image_sizes.get(path).copied() {
        st.image_sizes.insert(copy.clone(), s);
    }
    ctx.info(ctx.tf("rig.paint.copied", &[("path", &copy)]));
    Some(copy)
}

/// Writes the pictures changed in memory (with the rig's save).
pub fn save_canvases(ctx: &mut EditorContext<'_>, st: &mut State) -> bool {
    let root = st.project_root(ctx);
    let Some(doc) = st.doc.as_mut() else { return true };
    let mut ok = true;
    for path in doc.unsaved_canvases() {
        let Some(c) = doc.canvas(&path) else { continue };
        let abs = root.join(&path);
        match write_png(&abs, c.w, c.h, &c.rgba) {
            Ok(()) => {
                doc.mark_canvas_saved(&path);
                if let Some(s) = st.px.watched.get_mut(&path) {
                    *s = stamp(&abs);
                }
            }
            Err(e) => {
                ok = false;
                ctx.error(ctx.tf("rig.pixels.write_failed", &[("path", &path), ("error", &e)]));
            }
        }
    }
    ok
}

/// The edited pixels the viewport host does not have yet.
pub fn uploads(st: &mut State) -> Vec<PixelUpload> {
    let Some(doc) = st.doc.as_ref() else { return Vec::new() };
    let mut out = Vec::new();
    for (path, c) in doc.canvases() {
        let known = st.px.sent.get(path).copied();
        // Fresh from its file and never changed: the host reads the file.
        if known == Some(c.rev) || (known.is_none() && c.rev == 1 && !c.unsaved) {
            continue;
        }
        st.px.sent.insert(path.clone(), c.rev);
        out.push(PixelUpload { name: path.clone(), width: c.w, height: c.h, rgba: c.rgba.clone() });
    }
    out
}

// -------------------------------------------------------------- operations

fn combine(st: &mut State, slot: &SlotId, m: Mask, add: bool, sub: bool) {
    let next = match st.px.selection.take() {
        Some((s, mut cur)) if &s == slot && cur.w == m.w && cur.h == m.h && (add || sub) => {
            if add {
                cur.union(&m);
            } else {
                cur.subtract(&m);
            }
            Some((s, cur))
        }
        _ if sub => None,
        _ => Some((slot.clone(), m)),
    };
    st.px.set_selection(next);
}

fn close_polygon(st: &mut State, t: &Target, add: bool, sub: bool) {
    let draft = std::mem::take(&mut st.px.draft);
    if draft.len() >= 3 {
        combine(st, &t.slot, pixels::polygon_mask(t.w, t.h, &draft), add, sub);
    }
}

/// "Cut to new part": one undo step (the new slot, its picture, the filled
/// hole). The new part follows the selected bone (else the source's bone).
pub fn cut(ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(t) = target(ctx, st) else { return };
    let Some((sid, mask)) = st.px.selection.clone() else {
        ctx.warn(ctx.t("rig.cut.no_selection"));
        return;
    };
    if sid != t.slot || mask.w != t.w || mask.h != t.h {
        ctx.warn(ctx.t("rig.cut.no_selection"));
        return;
    }
    let root = st.project_root(ctx);
    let Some(doc) = st.doc.as_ref() else { return };
    let bone = st.selected.clone().filter(|b| model::bone(doc.rig(), b).is_some());
    let skin = doc.rig().skins.iter().any(|k| k.name == st.skin).then(|| st.skin.clone());
    let slot_name = model::slot(doc.rig(), &sid).map(|s| s.name.clone()).unwrap_or_default();
    let new_path = free_path(&root, &t.path, &format!("{}_cut", sanitize(&slot_name)));
    let Some(c) = doc.canvas(&t.path) else { return };
    let src = Picture { path: t.path.clone(), width: c.w, height: c.h, rgba: c.rgba.clone() };
    let (below, fill) = (st.px.below, st.px.fill_hole);
    let spec = |fill_hole: bool| rigops::CutSpec {
        slot: sid.as_str(),
        skin: skin.as_deref(),
        new_image: &new_path,
        name: None,
        bone: bone.as_ref().map(|b| b.as_str()),
        below,
        fill_hole,
    };
    // Dry run first (no fill): a refused cut changes nothing, not even the copy.
    if let Err(e) = rigops::cut_part(&mut doc.rig().clone(), &src, &mask, &spec(false)) {
        ctx.warn(e.message);
        return;
    }
    let Some(doc) = st.doc.as_mut() else { return };
    doc.begin_gesture();
    let src_path = if fill { editable(ctx, st, &t.path) } else { Some(t.path.clone()) };
    let Some(src_path) = src_path else {
        if let Some(d) = st.doc.as_mut() {
            d.end_gesture();
        }
        return;
    };
    let src = Picture { path: src_path.clone(), ..src };
    let spec = spec(fill);
    let Some(doc) = st.doc.as_mut() else { return };
    let mut failure = None;
    let result = doc.edit(|rig| {
        rigops::cut_part(rig, &src, &mask, &spec).map_err(|e| {
            failure = Some(e.message);
            OpError::Unknown
        })
    });
    let r = match result {
        Ok(r) => r,
        Err(_) => {
            doc.end_gesture();
            ctx.warn(failure.unwrap_or_else(|| ctx.t("rig.err.unknown")));
            return;
        }
    };
    if let Err(e) = write_png(&root.join(&new_path), r.part.width, r.part.height, &r.part.rgba) {
        doc.end_gesture();
        doc.undo();
        ctx.error(ctx.tf("rig.pixels.write_failed", &[("path", &new_path), ("error", &e)]));
        return;
    }
    doc.set_canvas(&new_path, r.part.width, r.part.height, r.part.rgba.clone());
    if let Some(filled) = r.source {
        doc.replace_pixels(&src_path, filled);
    }
    doc.end_gesture();
    register(ctx, &new_path);
    st.px.owned.insert(new_path.clone());
    st.image_sizes.insert(new_path.clone(), [r.part.width as f32, r.part.height as f32]);
    st.selected_slot = Some(r.slot);
    st.px.set_selection(None);
    st.px.draft.clear();
    ctx.info(ctx.tf("rig.cut.done", &[("path", &new_path)]));
    if r.remeshed > 0 {
        ctx.info(ctx.tn("rig.cut.remeshed", r.remeshed, &[]));
    }
}

/// "Re-trace mesh": the meshes of the target's picture follow its alpha again.
pub fn retrace(ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(t) = target(ctx, st) else { return };
    let Some(doc) = st.doc.as_mut() else { return };
    let Some(c) = doc.canvas(&t.path) else { return };
    let pic = Picture { path: t.path.clone(), width: c.w, height: c.h, rgba: c.rgba.clone() };
    if let Ok(n) = doc.edit(|rig| Ok(rigops::retrace_picture(rig, &pic))) {
        ctx.info(ctx.tn("rig.paint.retraced", n, &[]));
    }
}

/// "New layer…": a blank picture of the form's size, as a slot just in front
/// of the selected part and lined up on it when it has the same size.
pub fn create_layer(ctx: &mut EditorContext<'_>, st: &mut State, form: &NewLayer) {
    let (w, h) = (form.w.clamp(1, 4096), form.h.clamp(1, 4096));
    let root = st.project_root(ctx);
    let target = target(ctx, st);
    let beside = target.as_ref().map_or_else(|| "assets/images/x.png".to_owned(), |t| t.path.clone());
    let path = free_path(&root, &beside, &sanitize(&form.name));
    let rgba = vec![0u8; w as usize * h as usize * 4];
    if let Err(e) = write_png(&root.join(&path), w, h, &rgba) {
        ctx.error(ctx.tf("rig.pixels.write_failed", &[("path", &path), ("error", &e)]));
        return;
    }
    register(ctx, &path);
    let view_center = [-st.view.pan[0] / st.view.zoom, -st.view.pan[1] / st.view.zoom];
    let Some(doc) = st.doc.as_mut() else { return };
    let pic = Picture { path: path.clone(), width: w, height: h, rgba: rgba.clone() };
    doc.begin_gesture();
    let bone = workflow::part_bone(doc.rig(), st.selected.as_ref());
    let result = (|| {
        let bone = match bone {
            Some(b) => b,
            None => workflow::add_root(doc, &pic)?,
        };
        let center = target.as_ref().map_or(view_center, |t| t.map.apply([0.5, 0.5]));
        let slot = workflow::add_part(doc, &bone, &pic, center)?;
        if let Some(t) = &target {
            if t.w == w && t.h == h {
                doc.set_mesh(&slot, DEFAULT_SKIN, orbit_project::rigmesh::workflow::mesh_with_map(&pic, &t.map))?;
            }
            if let Some(i) = doc.rig().slots.iter().position(|s| s.id == t.slot) {
                doc.move_slot_to(&slot, i + 1)?;
            }
        }
        Ok::<_, OpError>(slot)
    })();
    doc.end_gesture();
    match result {
        Ok(slot) => {
            doc.set_canvas(&path, w, h, rgba);
            st.px.owned.insert(path.clone());
            st.image_sizes.insert(path.clone(), [w as f32, h as f32]);
            st.selected_slot = Some(slot);
            st.tool = Tool::Paint;
            st.px.paint_tool = PixTool::Brush;
            ctx.info(ctx.tf("rig.layer.created", &[("path", &path)]));
        }
        Err(e) => ctx.warn(ctx.t(e.key())),
    }
}

// ---------------------------------------------------------- external app

#[cfg(target_os = "windows")]
fn open_default(abs: &Path) -> std::io::Result<std::process::Child> {
    // `explorer <file>` opens it with its default application, without a
    // shell parsing the path.
    std::process::Command::new("explorer").arg(abs).spawn()
}

#[cfg(target_os = "macos")]
fn open_default(abs: &Path) -> std::io::Result<std::process::Child> {
    std::process::Command::new("open").arg(abs).spawn()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn open_default(abs: &Path) -> std::io::Result<std::process::Child> {
    std::process::Command::new("xdg-open").arg(abs).spawn()
}

/// "Edit in external app": writes pending pixels, opens the file with the
/// configured program (else the system's default) and follows the file.
pub fn open_external(ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(t) = target(ctx, st) else { return };
    if !save_canvases(ctx, st) {
        return;
    }
    let abs = st.project_root(ctx).join(&t.path);
    let custom = ctx.pref(crate::builtin::ids::RIG, PREF_IMAGE_EDITOR).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let spawned = match &custom {
        Some(exe) => std::process::Command::new(exe).arg(&abs).spawn(),
        None => open_default(&abs),
    };
    match spawned {
        Ok(_) => {
            st.px.watched.insert(t.path.clone(), stamp(&abs));
            ctx.info(ctx.tf("rig.external.opened", &[("path", &t.path)]));
        }
        Err(e) => ctx.error(ctx.tf("rig.external.failed", &[("path", &abs.display()), ("error", &e)])),
    }
}

/// Reloads the pictures open in another application when their file
/// changed: only when the editor's asset watcher reported a change (one
/// integer compare per frame otherwise). The reload is an undo step; a
/// changed silhouette traces the meshes again.
pub fn poll_external(ctx: &mut EditorContext<'_>, st: &mut State) {
    let epoch = ctx.state().asset_epoch;
    if epoch == st.px.epoch {
        return;
    }
    st.px.epoch = epoch;
    if st.px.watched.is_empty() || st.doc.is_none() {
        return;
    }
    let root = st.project_root(ctx);
    let changed: Vec<String> = st
        .px
        .watched
        .iter()
        .filter(|(p, s)| stamp(&root.join(p.as_str())) != **s)
        .map(|(p, _)| p.clone())
        .collect();
    for path in changed {
        let abs = root.join(&path);
        let now = stamp(&abs);
        let pic = match workflow::load_picture(&root, &path) {
            Ok(p) => p,
            // Still being written: next change.
            Err(_) => continue,
        };
        st.px.watched.insert(path.clone(), now);
        let Some(doc) = st.doc.as_mut() else { return };
        let old = doc.canvas(&path).cloned();
        doc.begin_gesture();
        let reshaped = match &old {
            Some(c) if c.w == pic.width && c.h == pic.height => {
                let threshold = orbit_project::rigmesh::meshgen::MeshParams::default().alpha_threshold;
                let reshaped = pixels::alpha_shape_changed(&c.rgba, &pic.rgba, threshold);
                doc.replace_pixels(&path, pic.rgba.clone());
                reshaped
            }
            _ => {
                doc.set_canvas(&path, pic.width, pic.height, pic.rgba.clone());
                doc.paint_live(&path, |_| true);
                true
            }
        };
        if reshaped {
            let _ = doc.edit(|rig| Ok(rigops::retrace_picture(rig, &pic)));
        }
        doc.end_gesture();
        doc.mark_canvas_saved(&path);
        st.image_sizes.insert(path.clone(), [pic.width as f32, pic.height as f32]);
        ctx.info(ctx.tf("rig.external.reloaded", &[("path", &path)]));
    }
}

// ------------------------------------------------------------------- input

/// Ends the stroke in progress (button released): one undo step.
pub fn finish_stroke(st: &mut State) {
    if let Some((path, stroke)) = st.px.stroke.take()
        && let Some(doc) = st.doc.as_mut()
    {
        doc.commit_pixels(&path, &stroke.before);
        doc.end_gesture();
    }
}

fn begin_stroke(ctx: &mut EditorContext<'_>, st: &mut State, t: &Target, tool: PixTool, p: [f32; 2]) {
    let mode = match tool {
        PixTool::Eraser => Mode::Erase,
        PixTool::Clone => match st.px.clone_source {
            Some(src) => Mode::Clone { offset: [src[0] - p[0], src[1] - p[1]] },
            None => {
                ctx.warn(ctx.t("rig.paint.clone_source"));
                return;
            }
        },
        _ => Mode::Paint,
    };
    finish_stroke(st);
    let Some(doc) = st.doc.as_mut() else { return };
    doc.begin_gesture();
    let Some(path) = editable(ctx, st, &t.path) else {
        if let Some(d) = st.doc.as_mut() {
            d.end_gesture();
        }
        return;
    };
    let Some(doc) = st.doc.as_mut() else { return };
    let Some(c) = doc.canvas(&path) else { return };
    let mut stroke = paint::Stroke::new(c.w, c.h, c.rgba.clone(), mode, st.px.brush);
    doc.paint_live(&path, |c| stroke.to(&mut c.rgba, p));
    st.px.stroke = Some((path, stroke));
}

fn continue_stroke(st: &mut State, p: [f32; 2]) {
    let (Some((path, stroke)), Some(doc)) = (st.px.stroke.as_mut(), st.doc.as_mut()) else { return };
    doc.paint_live(path, |c| stroke.to(&mut c.rgba, p));
}

fn fill_at(ctx: &mut EditorContext<'_>, st: &mut State, t: &Target, p: [f32; 2]) {
    if p[0] < 0.0 || p[1] < 0.0 || p[0] >= t.w as f32 || p[1] >= t.h as f32 {
        return;
    }
    let Some(doc) = st.doc.as_mut() else { return };
    doc.begin_gesture();
    if let Some(path) = editable(ctx, st, &t.path)
        && let Some(doc) = st.doc.as_mut()
        && let Some(c) = doc.canvas(&path)
    {
        let mut px = c.rgba.clone();
        let (w, h) = (c.w, c.h);
        let b = st.px.brush;
        if paint::fill(w, h, &mut px, [p[0] as u32, p[1] as u32], st.px.tolerance, b.color, b.opacity) {
            doc.replace_pixels(&path, px);
        }
    }
    if let Some(d) = st.doc.as_mut() {
        d.end_gesture();
    }
}

fn pick(st: &mut State, t: &Target, p: [f32; 2]) {
    if let Some(c) = st.doc.as_ref().and_then(|d| d.canvas(&t.path))
        && let Some(color) = paint::pick(c.w, c.h, &c.rgba, p)
    {
        st.px.brush.color = color;
    }
}

/// Viewport input of the cut and paint tools (rest pose only).
pub fn interact(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State, response: &egui::Response, rect: Rect) {
    if st.pose_mode || ui.input(|i| i.key_down(egui::Key::Space)) {
        return;
    }
    let Some(t) = target(ctx, st) else { return };
    let primary = egui::PointerButton::Primary;
    let (shift, cmd, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
    let pointer = response.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()));
    let Some(pos) = pointer else { return };
    let Some(p) = t.to_px(pointer_rig(st, rect, pos)) else { return };
    let tool = current(st);
    match tool {
        PixTool::Lasso => {
            if response.drag_started_by(primary) {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                st.px.draft = t.to_px(pointer_rig(st, rect, origin)).into_iter().collect();
            }
            if response.dragged_by(primary)
                && let Some(last) = st.px.draft.last()
                && ((last[0] - p[0]).powi(2) + (last[1] - p[1]).powi(2)) >= 0.5
            {
                st.px.draft.push(p);
            }
            if response.drag_stopped_by(primary) {
                close_polygon(st, &t, shift, cmd);
            }
        }
        PixTool::Polygon => {
            if response.double_clicked_by(primary) {
                close_polygon(st, &t, shift, cmd);
            } else if response.clicked_by(primary) {
                let near_first = st.px.draft.first().is_some_and(|f| {
                    let a = to_pos(st.view.to_screen(center_of(rect), t.to_rig(*f)));
                    a.distance(pos) < 8.0
                });
                if near_first && st.px.draft.len() >= 3 {
                    close_polygon(st, &t, shift, cmd);
                } else {
                    st.px.draft.push(p);
                }
            }
        }
        PixTool::Wand => {
            if response.clicked_by(primary)
                && p[0] >= 0.0
                && p[1] >= 0.0
                && let Some(c) = st.doc.as_ref().and_then(|d| d.canvas(&t.path))
            {
                let m = pixels::magic_wand(c.w, c.h, &c.rgba, [p[0] as u32, p[1] as u32], st.px.tolerance, st.px.contiguous);
                combine(st, &t.slot, m, shift, cmd);
            }
        }
        PixTool::Brush | PixTool::Eraser | PixTool::Clone => {
            if alt {
                if response.clicked_by(primary) {
                    if tool == PixTool::Clone {
                        st.px.clone_source = Some(p);
                    } else {
                        pick(st, &t, p);
                    }
                }
                return;
            }
            if response.drag_started_by(primary) {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                let start = t.to_px(pointer_rig(st, rect, origin)).unwrap_or(p);
                begin_stroke(ctx, st, &t, tool, start);
            }
            if response.dragged_by(primary) {
                continue_stroke(st, p);
            }
            if response.clicked_by(primary) {
                begin_stroke(ctx, st, &t, tool, p);
                finish_stroke(st);
            }
        }
        PixTool::Fill => {
            if response.clicked_by(primary) {
                fill_at(ctx, st, &t, p);
            }
        }
        PixTool::Picker => {
            if response.clicked_by(primary) {
                pick(st, &t, p);
            }
        }
    }
    if response.hovered() && matches!(tool, PixTool::Brush | PixTool::Eraser | PixTool::Clone) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    } else if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
}

/// Keys of the picture tools: Enter closes the polygon, Escape drops the
/// outline being drawn then the selection, `[` / `]` change the brush size.
pub fn keys(ui: &Ui, st: &mut State) {
    let (enter, esc, smaller, bigger) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::OpenBracket),
            i.key_pressed(egui::Key::CloseBracket),
        )
    });
    if enter && let Some(t) = loaded_target(st) {
        close_polygon(st, &t, false, false);
    }
    if esc {
        if st.px.draft.is_empty() {
            st.px.set_selection(None);
        } else {
            st.px.draft.clear();
        }
    }
    if smaller {
        st.px.brush.size = (st.px.brush.size / 1.25).max(1.0);
    }
    if bigger {
        st.px.brush.size = (st.px.brush.size * 1.25).min(512.0);
    }
}

// ---------------------------------------------------------------- drawing

/// The selection as a texture: tinted inside, opaque on its edge.
fn mask_image(m: &Mask, tint: Color32) -> egui::ColorImage {
    let mut bytes = vec![0u8; m.w as usize * m.h as usize * 4];
    for y in 0..m.h {
        for x in 0..m.w {
            if !m.get(x, y) {
                continue;
            }
            let edge = x == 0
                || y == 0
                || !m.get(x - 1, y)
                || !m.get(x + 1, y)
                || !m.get(x, y - 1)
                || !m.get(x, y + 1);
            let i = ((y * m.w + x) * 4) as usize;
            bytes[i..i + 4].copy_from_slice(&[tint.r(), tint.g(), tint.b(), if edge { 240 } else { 70 }]);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([m.w as usize, m.h as usize], &bytes)
}

/// Selection, outline being drawn, picture frame and brush cursor.
pub fn paint_overlay(ui: &Ui, st: &mut State, painter: &egui::Painter, rect: Rect) {
    let Some(t) = loaded_target(st) else { return };
    let center = center_of(rect);
    let screen = |px: [f32; 2]| to_pos(st.view.to_screen(center, t.to_rig(px)));
    let (w, h) = (t.w as f32, t.h as f32);
    let corners = [screen([0.0, 0.0]), screen([w, 0.0]), screen([w, h]), screen([0.0, h])];
    painter.add(egui::Shape::closed_line(corners.to_vec(), egui::Stroke::new(1.0, faint(ui))));
    // Selection.
    if let Some((sid, m)) = &st.px.selection
        && sid == &t.slot
        && m.w == t.w
        && m.h == t.h
    {
        let rev = st.px.mask_rev;
        if st.px.mask_tex.as_ref().is_none_or(|(r, _)| *r != rev) {
            let tex = ui.ctx().load_texture("rig-selection", mask_image(m, accent(ui)), egui::TextureOptions::NEAREST);
            st.px.mask_tex = Some((rev, tex));
        }
        if let Some((_, tex)) = &st.px.mask_tex {
            let mut mesh = egui::Mesh::with_texture(tex.id());
            for (p, uv) in corners.iter().zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]) {
                mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv: Pos2::new(uv[0], uv[1]), color: Color32::WHITE });
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
            painter.add(egui::Shape::mesh(mesh));
        }
    }
    // Outline being drawn.
    if !st.px.draft.is_empty() {
        let mut pts: Vec<Pos2> = st.px.draft.iter().map(|p| screen(*p)).collect();
        if st.tool == Tool::Cut
            && st.px.cut_tool == PixTool::Polygon
            && let Some(hover) = ui.input(|i| i.pointer.hover_pos())
        {
            pts.push(hover);
        }
        painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(1.5, accent(ui))));
        for p in st.px.draft.iter().take(1) {
            painter.circle_stroke(screen(*p), 4.0, egui::Stroke::new(1.0, accent(ui)));
        }
    }
    // Clone source.
    if st.tool == Tool::Paint
        && st.px.paint_tool == PixTool::Clone
        && let Some(src) = st.px.clone_source
    {
        let c = screen(src);
        let s = egui::Stroke::new(1.0, accent(ui));
        painter.line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], s);
        painter.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], s);
    }
    // Brush cursor.
    if st.tool == Tool::Paint
        && matches!(st.px.paint_tool, PixTool::Brush | PixTool::Eraser | PixTool::Clone)
        && let Some(hover) = ui.input(|i| i.pointer.hover_pos())
        && rect.contains(hover)
    {
        let a = screen([0.0, 0.0]);
        let b = screen([1.0, 0.0]);
        let r = (st.px.brush.size * 0.5 * a.distance(b)).max(1.0);
        painter.circle_stroke(hover, r, egui::Stroke::new(1.5, Color32::from_black_alpha(160)));
        painter.circle_stroke(hover, r, egui::Stroke::new(0.8, Color32::WHITE));
    }
}

// ---------------------------------------------------------------- options

fn tool_button(ui: &mut Ui, ctx: &EditorContext<'_>, on: bool, icon: &str, key: &str) -> bool {
    let mut text = RichText::new(icon).size(15.0);
    if on {
        text = text.color(accent(ui));
    }
    ui.add(egui::Button::new(text).frame(on)).on_hover_text(ctx.t(key)).clicked()
}

/// The options strip of the cut and paint tools (under the toolbar).
pub fn options(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    if !is_pixel_tool(st.tool) {
        return;
    }
    let target = locate(st).map(|(_, p, _)| p);
    let mut action: Option<fn(&mut EditorContext<'_>, &mut State)> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if st.tool == Tool::Cut {
            for (tool, icon, key) in [
                (PixTool::Lasso, icons::LASSO, "rig.cut.lasso"),
                (PixTool::Polygon, icons::POLYGON, "rig.cut.polygon"),
                (PixTool::Wand, icons::MAGIC_WAND, "rig.cut.wand"),
            ] {
                if tool_button(ui, ctx, st.px.cut_tool == tool, icon, key) {
                    st.px.cut_tool = tool;
                    st.px.draft.clear();
                }
            }
            if st.px.cut_tool == PixTool::Wand {
                ui.add(egui::Slider::new(&mut st.px.tolerance, 0..=255).text(ctx.t("rig.cut.tolerance")));
                ui.checkbox(&mut st.px.contiguous, ctx.t("rig.cut.contiguous"));
            }
            ui.separator();
            ui.selectable_value(&mut st.px.below, false, ctx.t("rig.cut.in_front"));
            ui.selectable_value(&mut st.px.below, true, ctx.t("rig.cut.behind"));
            ui.checkbox(&mut st.px.fill_hole, ctx.t("rig.cut.fill")).on_hover_text(ctx.t("rig.cut.fill_hint"));
            if ui
                .add_enabled(
                    st.px.has_selection(),
                    egui::Button::new(RichText::new(format!("{}  {}", icons::SCISSORS, ctx.t("rig.cut.do"))).strong()),
                )
                .on_hover_text(ctx.t("rig.cut.do_hint"))
                .clicked()
            {
                action = Some(cut);
            }
            if st.px.has_selection() && ui.small_button(ctx.t("rig.cut.clear")).clicked() {
                st.px.set_selection(None);
            }
        } else {
            for (tool, icon, key) in [
                (PixTool::Brush, icons::PAINT_BRUSH_BROAD, "rig.paint.brush"),
                (PixTool::Eraser, icons::ERASER, "rig.paint.eraser"),
                (PixTool::Clone, icons::STAMP, "rig.paint.clone"),
                (PixTool::Fill, icons::PAINT_BUCKET, "rig.paint.fill"),
                (PixTool::Picker, icons::EYEDROPPER, "rig.paint.picker"),
            ] {
                if tool_button(ui, ctx, st.px.paint_tool == tool, icon, key) {
                    st.px.paint_tool = tool;
                }
            }
            let b = &mut st.px.brush;
            let mut color = Color32::from_rgba_unmultiplied(b.color[0], b.color[1], b.color[2], b.color[3]);
            if ui.color_edit_button_srgba(&mut color).changed() {
                b.color = color.to_srgba_unmultiplied();
            }
            ui.label(RichText::new(ctx.t("rig.paint.size")).color(weak(ui)));
            ui.add(egui::DragValue::new(&mut b.size).range(1.0..=512.0).speed(0.5));
            ui.label(RichText::new(ctx.t("rig.paint.hardness")).color(weak(ui)));
            ui.add(egui::Slider::new(&mut b.hardness, 0.0..=1.0).show_value(false));
            ui.label(RichText::new(ctx.t("rig.paint.opacity")).color(weak(ui)));
            ui.add(egui::Slider::new(&mut b.opacity, 0.0..=1.0).show_value(false));
            if st.px.paint_tool == PixTool::Fill {
                ui.add(egui::Slider::new(&mut st.px.tolerance, 0..=255).text(ctx.t("rig.cut.tolerance")));
            }
            ui.separator();
            ui.checkbox(&mut st.px.overwrite, ctx.t("rig.paint.overwrite")).on_hover_text(ctx.t("rig.paint.overwrite_hint"));
        }
        ui.separator();
        if ui.button(format!("{}  {}", icons::STACK, ctx.t("rig.layer.new"))).on_hover_text(ctx.t("rig.layer.new_hint")).clicked() {
            let size = locate(st)
                .and_then(|(_, p, _)| st.doc.as_ref()?.canvas(&p).map(|c| (c.w, c.h)))
                .unwrap_or((512, 512));
            st.px.new_layer = Some(NewLayer { name: ctx.t("rig.layer.default_name"), w: size.0, h: size.1 });
        }
        if ui
            .add_enabled(target.is_some(), egui::Button::new(ctx.t("rig.paint.retrace")))
            .on_hover_text(ctx.t("rig.paint.retrace_hint"))
            .clicked()
        {
            action = Some(retrace);
        }
        if ui
            .add_enabled(target.is_some(), egui::Button::new(format!("{}  {}", icons::ARROW_SQUARE_OUT, ctx.t("rig.external.open"))))
            .on_hover_text(ctx.t("rig.external.open_hint"))
            .clicked()
        {
            action = Some(open_external);
        }
        ui.menu_button(icons::DOTS_THREE, |ui| {
            ui.label(RichText::new(ctx.t("rig.external.program")).color(weak(ui)));
            let mut path = ctx.pref(crate::builtin::ids::RIG, PREF_IMAGE_EDITOR).unwrap_or_default().to_owned();
            if ui.add(egui::TextEdit::singleline(&mut path).hint_text(ctx.t("rig.external.program_hint"))).changed() {
                ctx.set_pref(crate::builtin::ids::RIG, PREF_IMAGE_EDITOR, path);
            }
            if ui.button(ctx.t("rig.external.browse")).clicked()
                && let Some(p) = rfd::FileDialog::new().pick_file()
            {
                ctx.set_pref(crate::builtin::ids::RIG, PREF_IMAGE_EDITOR, p.to_string_lossy().into_owned());
            }
        });
    });
    let hint = match &target {
        Some(p) => RichText::new(format!("{}  {p}", icons::IMAGE)).small().color(weak(ui)),
        None => RichText::new(ctx.t("rig.pixels.pick_part")).small().color(faint(ui)),
    };
    let tip = match (st.tool, current(st)) {
        (Tool::Cut, PixTool::Lasso) => "rig.cut.lasso_tip",
        (Tool::Cut, PixTool::Polygon) => "rig.cut.polygon_tip",
        (Tool::Cut, _) => "rig.cut.wand_tip",
        (_, PixTool::Clone) => "rig.paint.clone_tip",
        _ => "rig.paint.tip",
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(hint);
        ui.label(RichText::new(ctx.t(tip)).small().color(faint(ui)));
    });
    if let Some(f) = action {
        f(ctx, st);
    }
}

/// The "New layer…" window.
pub fn new_layer_dialog(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(mut form) = st.px.new_layer.take() else { return };
    let mut open = true;
    let mut create = false;
    egui::Window::new(ctx.t("rig.layer.title"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .open(&mut open)
        .show(ui.ctx(), |ui| {
            ui.label(RichText::new(ctx.t("rig.layer.explain")).small().color(weak(ui)));
            egui::Grid::new("rig_new_layer").num_columns(2).show(ui, |ui| {
                ui.label(ctx.t("rig.layer.name"));
                ui.text_edit_singleline(&mut form.name);
                ui.end_row();
                ui.label(ctx.t("rig.layer.size"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut form.w).range(1..=4096));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut form.h).range(1..=4096));
                });
                ui.end_row();
            });
            ui.horizontal(|ui| {
                create = ui.button(ctx.t("rig.layer.create")).clicked();
            });
        });
    if create {
        create_layer(ctx, st, &form);
    } else if open {
        st.px.new_layer = Some(form);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::rig_host::RigLink;

    fn state() -> State {
        let mut st = State::new(Rc::new(RefCell::new(RigLink::default())));
        st.new_rig();
        st
    }

    #[test]
    fn only_changed_pixels_are_sent_to_the_viewport() {
        let mut st = state();
        let doc = st.doc.as_mut().unwrap();
        doc.set_canvas("a.png", 2, 2, vec![0; 16]);
        // Fresh from its file: the host reads the file itself.
        assert!(uploads(&mut st).is_empty());
        st.doc.as_mut().unwrap().paint_live("a.png", |c| {
            c.rgba[0] = 9;
            true
        });
        let sent = uploads(&mut st);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].name.as_str(), sent[0].width, sent[0].rgba[0]), ("a.png", 2, 9));
        // Nothing new: nothing sent (idle costs nothing).
        assert!(uploads(&mut st).is_empty());
        // Another rig: the edited picture goes back to its file.
        st.set_doc(super::super::model::RigDoc::new());
        assert_eq!(st.px.forget, vec!["a.png".to_owned()]);
    }

    #[test]
    fn new_pictures_never_overwrite_a_file() {
        let dir = std::env::temp_dir().join(format!("orbit_rig_px_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets/images")).unwrap();
        std::fs::write(dir.join("assets/images/face_cut.png"), b"x").unwrap();
        assert_eq!(free_path(&dir, "assets/images/face.png", "face_cut"), "assets/images/face_cut_2.png");
        assert_eq!(free_path(&dir, "face.png", "eyes"), "eyes.png");
        assert_eq!(stem_of("assets/images/my face.v2.png"), "my_face_v2");
        assert_eq!(sanitize("  "), "part");
        write_png(&dir.join("assets/images/t.png"), 1, 1, &[1, 2, 3, 4]).unwrap();
        let back = workflow::load_picture(&dir, "assets/images/t.png").unwrap();
        assert_eq!(back.rgba, vec![1, 2, 3, 4]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_selection_adds_and_subtracts_on_one_part_only() {
        let mut st = state();
        let s = SlotId::new("s1");
        let m = |x: u32| {
            let mut m = Mask::new(4, 1);
            m.set(x, 0, true);
            m
        };
        combine(&mut st, &s, m(0), false, false);
        combine(&mut st, &s, m(1), true, false);
        assert_eq!(st.px.selection.as_ref().unwrap().1.count(), 2);
        combine(&mut st, &s, m(0), false, true);
        assert_eq!(st.px.selection.as_ref().unwrap().1.count(), 1);
        // Another part: a plain selection replaces it.
        combine(&mut st, &SlotId::new("s2"), m(3), true, false);
        assert_eq!(st.px.selection.as_ref().unwrap().0.as_str(), "s2");
        combine(&mut st, &SlotId::new("s2"), m(3), false, true);
        assert!(!st.px.has_selection());
    }
}
