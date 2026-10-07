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
use std::rc::Rc;
use std::time::SystemTime;

use egui::{Color32, Pos2, Rect, RichText, Ui, vec2};
use orbit_anim::rig::{Affine, Attachment, CpuSkinner, Rig, RigDrawItem, SlotId};
use orbit_core::AssetKind;
use orbit_editor_api::{EditorContext, TaskError, icons};
use orbit_project::rigmesh::meshgen;
use orbit_project::rigmesh::pixels::{self, Mask};
use orbit_project::rigops;

use super::geom::Tool;
use super::model::{self, DEFAULT_SKIN, OpError};
use super::paint::{self, Brush, Mode};
use super::posemap::PicMap;
use super::shared::Decision;
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
    /// Lasso / polygon points being drawn (rig space, as shown: mapped to
    /// the picture when the outline closes).
    draft: Vec<[f32; 2]>,
    stroke: Option<(String, paint::Stroke)>,
    /// Last brush point of the stroke (rig space, as shown).
    stroke_last: Option<[f32; 2]>,
    /// The posed part of the selected slot (pose mode only).
    posed: Option<(PoseKey, Option<Posed>)>,
    clone_source: Option<[f32; 2]>,
    /// Pictures made by the editor (cuts, layers, copies): edited in place.
    pub(super) owned: HashSet<String>,
    /// Pictures that could not be read (not retried every frame).
    failed: HashSet<String>,
    /// Pictures open in another application, and their file's last state.
    watched: HashMap<String, Stamp>,
    /// Watched pictures being written by the background writer (their new
    /// stamp is taken once written).
    own_writes: HashSet<String>,
    epoch: u64,
    /// Pixel revision of each picture last sent to the viewport host.
    sent: HashMap<String, u64>,
    /// Edited pictures dropped with the previous rig (the host reloads them).
    pub forget: Vec<String>,
    pub new_layer: Option<NewLayer>,
    /// Shared pictures: users index, remembered choices, the dialog.
    pub shared: super::shared::SharedState,
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
            stroke_last: None,
            posed: None,
            clone_source: None,
            owned: HashSet::new(),
            failed: HashSet::new(),
            watched: HashMap::new(),
            own_writes: HashSet::new(),
            epoch: 0,
            sent: HashMap::new(),
            forget: Vec::new(),
            new_layer: None,
            shared: Default::default(),
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

/// The picture the tools work on: the selected slot's, in the shown skin
/// (in pose mode: the picture the pose shows, as deformed).
pub struct Target {
    pub slot: SlotId,
    pub path: String,
    /// Picture UV (`0..1`) → rig space, setup pose.
    pub map: Affine,
    /// Rig space ↔ picture pixels as shown (the rest pose, or the pose).
    pub shown: Rc<PicMap>,
    pub w: u32,
    pub h: u32,
}

impl Target {
    fn to_px(&self, rig_p: [f32; 2]) -> Option<[f32; 2]> {
        self.shown.to_px(rig_p)
    }

    fn to_rig(&self, px: [f32; 2]) -> Option<[f32; 2]> {
        self.shown.to_rig(px)
    }

    /// Picture pixels per rig unit in the setup pose: the brush keeps its
    /// on-screen size when the part is posed, stretched or squashed.
    fn rest_scale(&self) -> f32 {
        PicMap::affine(&self.map, self.w, self.h).px_per_rig([0.0, 0.0]).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0)
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

/// What identifies a displayed pose for the cache of the posed geometry:
/// rig revision, pose revision, prepared rig, slot, skin.
type PoseKey = (u64, u64, usize, SlotId, Option<String>);

/// A posed part: rigid (UV → rig) or skinned triangles.
enum Geom {
    Region(Affine),
    Mesh { pos: Vec<[f32; 2]>, uvs: Vec<[f32; 2]>, tris: Vec<[u16; 3]> },
}

/// The picture a slot shows in the displayed pose, and its map once the
/// picture size is known (rebuilt only when the pose or the rig changed,
/// and only while a picture tool is in use).
pub struct Posed {
    path: String,
    geom: Geom,
    map: Option<(u32, u32, Rc<PicMap>)>,
}

fn pose_key(st: &State, sid: &SlotId) -> Option<PoseKey> {
    let doc = st.doc.as_ref()?;
    let prepared = st.prepared.as_ref()?;
    Some((doc.revision, st.pose_rev, Rc::as_ptr(prepared) as usize, sid.clone(), st.shown_skin()))
}

/// Skins the displayed pose (pose overrides, clip at the playhead,
/// cross-fade, IK) and keeps the slot's part.
fn build_posed(st: &State, sid: &SlotId) -> Option<Posed> {
    let prepared = st.prepared.clone()?;
    let pose = st.rig_pose()?;
    let si = st.doc.as_ref()?.rig().slots.iter().position(|s| &s.id == sid)?;
    let world = prepared.world(&pose);
    let skin = st.shown_skin();
    let list = prepared.draw_list(&pose, &world, skin.as_deref(), &CpuSkinner);
    list.items.iter().find_map(|item| match item {
        RigDrawItem::Region { slot, image, size, transform, .. } if *slot == si => Some(Posed {
            path: (*image).to_owned(),
            geom: Geom::Region(meshgen::region_uv_to_rig(transform, *size)),
            map: None,
        }),
        RigDrawItem::Mesh { slot, image, range, uvs, triangles, .. } if *slot == si => Some(Posed {
            path: (*image).to_owned(),
            geom: Geom::Mesh { pos: list.positions.get(range.clone())?.to_vec(), uvs: uvs.to_vec(), tris: triangles.to_vec() },
            map: None,
        }),
        _ => None,
    })
}

/// The selected slot, the picture shown (as posed in pose mode) and its
/// setup map.
fn resolve(st: &mut State) -> Option<(SlotId, String, Affine)> {
    let (sid, path, map) = locate(st)?;
    if !st.pose_mode {
        return Some((sid, path, map));
    }
    let key = pose_key(st, &sid)?;
    if st.px.posed.as_ref().is_none_or(|(k, _)| k != &key) {
        let posed = build_posed(st, &sid);
        st.px.posed = Some((key, posed));
    }
    let path = st.px.posed.as_ref()?.1.as_ref()?.path.clone();
    Some((sid, path, map))
}

/// The target once its pixels are loaded.
fn finish(st: &mut State, slot: SlotId, path: String, map: Affine) -> Option<Target> {
    let c = st.doc.as_ref()?.canvas(&path)?;
    let (w, h) = (c.w, c.h);
    let shown = if st.pose_mode {
        let p = st.px.posed.as_mut()?.1.as_mut()?;
        match &p.map {
            Some((pw, ph, m)) if (*pw, *ph) == (w, h) => m.clone(),
            _ => {
                let m = Rc::new(match &p.geom {
                    Geom::Region(a) => PicMap::affine(a, w, h),
                    Geom::Mesh { pos, uvs, tris } => PicMap::mesh(pos, uvs, tris, w, h),
                });
                p.map = Some((w, h, m.clone()));
                m
            }
        }
    } else {
        Rc::new(PicMap::affine(&map, w, h))
    };
    Some(Target { slot, path, map, shown, w, h })
}

/// The target, its pixels loaded into the document when needed.
pub fn target(ctx: &mut EditorContext<'_>, st: &mut State) -> Option<Target> {
    let (slot, path, map) = resolve(st)?;
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
    finish(st, slot, path, map)
}

/// The target when its pixels are already loaded (painting the overlay).
fn loaded_target(st: &mut State) -> Option<Target> {
    let (slot, path, map) = resolve(st)?;
    finish(st, slot, path, map)
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
pub(super) fn sanitize(name: &str) -> String {
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
pub(super) fn register(ctx: &mut EditorContext<'_>, rel: &str) {
    if ctx.has_project() && !Path::new(rel).is_absolute() {
        ctx.ops_or_toast(|p| orbit_project::ops::add_asset(p, AssetKind::Image, rel));
    }
}

/// Every attachment showing `from` shows `to` instead.
pub(super) fn repoint(rig: &mut Rig, from: &str, to: &str) {
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
pub(super) fn editable(ctx: &mut EditorContext<'_>, st: &mut State, path: &str, slot: &SlotId) -> Option<String> {
    // A picture other things show: a copy for this slot, the shared file or
    // a question first (copy-on-write, see `shared`).
    match super::shared::decide(ctx, st, path, slot) {
        Decision::Ask => return None,
        Decision::Copy => return super::shared::make_copy(ctx, st, path, slot),
        Decision::InPlace => return Some(path.to_owned()),
        Decision::Edit => {}
    }
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

/// Writes the pictures changed in memory (with the rig's save), on the
/// background writer: encoding a 2048 px PNG takes over 100 ms in debug
/// builds, which froze the editor on every save of a painted rig. Callers
/// that need the files at once (external editor, duplicate) call
/// [`crate::writer::flush`]. A failure is logged (console).
pub fn save_canvases(ctx: &mut EditorContext<'_>, st: &mut State) -> bool {
    let root = st.project_root(ctx);
    let Some(doc) = st.doc.as_mut() else { return true };
    for path in doc.unsaved_canvases() {
        let Some(c) = doc.canvas(&path) else { continue };
        let abs = root.join(&path);
        let (w, h, rgba) = (c.w, c.h, c.rgba.clone());
        let failed = ctx.tf("rig.pixels.write_failed", &[("path", &path), ("error", &"{error}")]);
        crate::writer::submit(Some(abs.clone()), move || {
            orbit_profile::scope!("rig picture encode + write");
            if let Err(e) = write_png(&abs, w, h, &rgba) {
                log::error!("{}", failed.replace("{error}", &e));
            }
        });
        doc.mark_canvas_saved(&path);
        if st.px.watched.contains_key(&path) {
            // Our own write must not look like an external edit.
            st.px.own_writes.insert(path);
        }
    }
    true
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
        // Drawn on the part as shown: each point (edges densified to about
        // a picture pixel on a mesh) lands on the picture.
        let poly = t.shown.outline_to_px(&draft, 1.0 / t.rest_scale());
        if poly.len() >= 3 {
            combine(st, &t.slot, pixels::polygon_mask(t.w, t.h, &poly), add, sub);
        }
    }
}

/// "Cut to new part": one undo step (the new slot, its picture, the filled
/// hole). The new part follows the selected bone (else the source's bone).
pub fn cut(ctx: &mut EditorContext<'_>, st: &mut State) {
    if super::jobs::busy(ctx, st) {
        return;
    }
    let Some(t) = target(ctx, st) else { return };
    let Some((sid, mask)) = st.px.selection.clone() else {
        ctx.warn(ctx.t("rig.cut.no_selection"));
        return;
    };
    if sid != t.slot || mask.w != t.w || mask.h != t.h {
        ctx.warn(ctx.t("rig.cut.no_selection"));
        return;
    }
    // The pose may show another picture than the setup pose (a clip swaps
    // it): the cut works on the setup picture only.
    if locate(st).is_none_or(|(_, p, _)| p != t.path) {
        ctx.warn(ctx.t("rig.cut.pose_other_picture"));
        return;
    }
    let root = st.project_root(ctx);
    let Some(doc) = st.doc.as_ref() else { return };
    let slot_name = model::slot(doc.rig(), &sid).map(|s| s.name.clone()).unwrap_or_default();
    let new_path = free_path(&root, &t.path, &format!("{}_cut", sanitize(&slot_name)));
    let (below, fill) = (st.px.below, st.px.fill_hole);
    // Filling the hole edits the source picture: the first edit of a picture
    // the editor did not make goes to a copy (see `editable`), written by
    // the job too. On a picture other things show, the copy is for this slot
    // only (see `shared`); when the user chose to edit the shared file it is
    // filled in place.
    let mut slot_only = None;
    let copy = if fill {
        match super::shared::decide(ctx, st, &t.path, &t.slot) {
            Decision::Ask => return,
            Decision::Copy => {
                slot_only = Some(t.slot.clone());
                Some(super::shared::copy_path(ctx, st, &t.path, &slot_name))
            }
            Decision::InPlace => None,
            Decision::Edit => (!(st.px.overwrite || st.px.owned.contains(&t.path)))
                .then(|| free_path(&root, &t.path, &format!("{}_edit", stem_of(&t.path)))),
        }
        .filter(|copy| *copy != new_path)
    } else {
        None
    };
    let Some(doc) = st.doc.as_ref() else { return };
    let bone = st.selected.as_ref().filter(|b| model::bone(doc.rig(), b).is_some()).map(|b| b.as_str().to_owned());
    let skin = doc.rig().skins.iter().any(|k| k.name == st.skin).then(|| st.skin.clone());
    let Some(c) = doc.canvas(&t.path) else { return };
    let src = Picture { path: t.path.clone(), width: c.w, height: c.h, rgba: c.rgba.clone() };
    let mut rig = doc.rig().clone();
    let sid = sid.as_str().to_owned();
    let mut files = vec![root.join(&new_path)];
    files.extend(copy.iter().map(|c| root.join(c)));
    super::jobs::start(ctx, st, "rig.task.cut", super::jobs::files(files), move |p| {
        let spec = rigops::CutSpec {
            slot: sid.as_str(),
            skin: skin.as_deref(),
            new_image: &new_path,
            name: None,
            bone: bone.as_deref(),
            below,
            fill_hole: fill,
        };
        // Works on a copy of the rig: a refused cut changes nothing.
        let r = rigops::cut_part(&mut rig, &src, &mask, &spec).map_err(|e| TaskError::Failed(e.message))?;
        p.check()?;
        p.set_fraction(0.8);
        write_png(&root.join(&new_path), r.part.width, r.part.height, &r.part.rgba)
            .map_err(|e| TaskError::Failed(format!("{new_path}: {e}")))?;
        if let Some(copy) = &copy {
            write_png(&root.join(copy), src.width, src.height, &src.rgba)
                .map_err(|e| TaskError::Failed(format!("{copy}: {e}")))?;
        }
        Ok(super::jobs::Out::Cut(Box::new(CutOut { rig, src, copy, slot_only, new_path, r })))
    });
}

/// A cut computed (and its pictures written) by a job.
pub struct CutOut {
    /// The rig after the cut (its source still shows the original picture).
    rig: Rig,
    /// The source picture before the cut.
    src: Picture,
    /// The copy of the source written for the filled hole, if any.
    copy: Option<String>,
    /// The copy is for this slot only (the picture is shared).
    slot_only: Option<SlotId>,
    new_path: String,
    r: rigops::CutResult,
}

/// Applies a finished cut as one undo step (the copy of the source, the new
/// slot, its picture, the filled hole).
pub fn apply_cut(ctx: &mut EditorContext<'_>, st: &mut State, out: CutOut) {
    let CutOut { mut rig, src, copy, slot_only, new_path, r } = out;
    let src_path = match &copy {
        Some(copy) => {
            register(ctx, copy);
            st.px.owned.insert(copy.clone());
            if let Some(s) = st.image_sizes.get(&src.path).copied() {
                st.image_sizes.insert(copy.clone(), s);
            }
            match &slot_only {
                Some(slot) => {
                    orbit_project::picture_users::repoint_slot(&mut rig, slot.as_str(), &src.path, copy);
                }
                None => repoint(&mut rig, &src.path, copy),
            }
            ctx.info(ctx.tf("rig.paint.copied", &[("path", copy)]));
            copy.clone()
        }
        None => src.path.clone(),
    };
    let Some(doc) = st.doc.as_mut() else { return };
    doc.begin_gesture();
    if let Some(copy) = &copy {
        doc.set_canvas(copy, src.width, src.height, src.rgba);
    }
    let _ = doc.edit(|current| {
        *current = rig;
        Ok(())
    });
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

/// "Re-trace mesh": the meshes of the target's picture follow its alpha
/// again (in the background).
pub fn retrace(ctx: &mut EditorContext<'_>, st: &mut State) {
    if super::jobs::busy(ctx, st) {
        return;
    }
    let Some(t) = target(ctx, st) else { return };
    let Some(doc) = st.doc.as_ref() else { return };
    let Some(c) = doc.canvas(&t.path) else { return };
    let pic = Picture { path: t.path.clone(), width: c.w, height: c.h, rgba: c.rgba.clone() };
    let mut rig = doc.rig().clone();
    super::jobs::start(ctx, st, "rig.task.retrace", super::jobs::files(Vec::new()), move |_| {
        let n = rigops::retrace_picture(&mut rig, &pic);
        Ok(super::jobs::Out::Retraced { rig, n })
    });
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
    let Some(mut t) = target(ctx, st) else { return };
    // A shared picture is edited as a copy for this slot, unless the user
    // chose to edit the shared file.
    match super::shared::decide(ctx, st, &t.path, &t.slot) {
        Decision::Ask => return,
        Decision::Copy => match super::shared::make_unique(ctx, st, &t.path, &t.slot) {
            Some(copy) => t.path = copy,
            None => return,
        },
        Decision::InPlace | Decision::Edit => {}
    }
    if !save_canvases(ctx, st) {
        return;
    }
    // The other application opens the file: it must be on disk.
    crate::writer::flush();
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
    if !st.px.own_writes.is_empty() {
        // Our pictures are still being written: look again next frame,
        // then take their new stamps as ours.
        if crate::writer::pending() {
            return;
        }
        let root = st.project_root(ctx);
        for path in std::mem::take(&mut st.px.own_writes) {
            if let Some(s) = st.px.watched.get_mut(&path) {
                *s = stamp(&root.join(&path));
            }
        }
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
    st.px.stroke_last = None;
    if let Some((path, stroke)) = st.px.stroke.take()
        && let Some(doc) = st.doc.as_mut()
    {
        doc.commit_pixels(&path, &stroke.before);
        doc.end_gesture();
    }
}

/// Starts a stroke at `rig` (rig space, as shown).
fn begin_stroke(ctx: &mut EditorContext<'_>, st: &mut State, t: &Target, tool: PixTool, rig: [f32; 2]) {
    let mode = match tool {
        PixTool::Eraser => Mode::Erase,
        PixTool::Clone => match (st.px.clone_source, t.shown.to_px_extrapolated(rig)) {
            (Some(src), Some(p)) => Mode::Clone { offset: [src[0] - p[0], src[1] - p[1]] },
            (None, _) => {
                ctx.warn(ctx.t("rig.paint.clone_source"));
                return;
            }
            (Some(_), None) => return,
        },
        _ => Mode::Paint,
    };
    finish_stroke(st);
    let Some(doc) = st.doc.as_mut() else { return };
    doc.begin_gesture();
    let Some(path) = editable(ctx, st, &t.path, &t.slot) else {
        if let Some(d) = st.doc.as_mut() {
            d.end_gesture();
        }
        return;
    };
    let Some(doc) = st.doc.as_mut() else { return };
    let Some(c) = doc.canvas(&path) else { return };
    let stroke = paint::Stroke::new(c.w, c.h, c.rgba.clone(), mode, st.px.brush);
    st.px.stroke = Some((path, stroke));
    continue_stroke(st, t, rig);
}

/// Paints from the last point of the stroke to `rig` (rig space, as
/// shown): the segment is sampled on screen, each dab lands where the part
/// shows it, its size follows the local stretch of the part (the brush
/// keeps its size on screen). Off a posed mesh, nothing is painted.
fn continue_stroke(st: &mut State, t: &Target, rig: [f32; 2]) {
    let from = st.px.stroke_last.unwrap_or(rig);
    st.px.stroke_last = Some(rig);
    let (Some((path, stroke)), Some(doc)) = (st.px.stroke.as_mut(), st.doc.as_mut()) else { return };
    let rest = t.rest_scale();
    let size = stroke.brush.size;
    let spacing = ((size / rest) * 0.15).max(0.5 / rest);
    let len = ((rig[0] - from[0]).powi(2) + (rig[1] - from[1]).powi(2)).sqrt();
    let steps = ((len / spacing).ceil().max(1.0) as usize).min(100_000);
    let shown = &t.shown;
    doc.paint_live(path, |c| {
        let mut changed = false;
        for s in 1..=steps {
            let k = s as f32 / steps as f32;
            let q = [from[0] + (rig[0] - from[0]) * k, from[1] + (rig[1] - from[1]) * k];
            if let (Some(p), Some(scale)) = (shown.to_px(q), shown.px_per_rig(q)) {
                changed |= stroke.dab_sized(&mut c.rgba, p, size * scale / rest);
            }
        }
        changed
    });
}

fn fill_at(ctx: &mut EditorContext<'_>, st: &mut State, t: &Target, p: [f32; 2]) {
    if p[0] < 0.0 || p[1] < 0.0 || p[0] >= t.w as f32 || p[1] >= t.h as f32 {
        return;
    }
    let Some(doc) = st.doc.as_mut() else { return };
    doc.begin_gesture();
    if let Some(path) = editable(ctx, st, &t.path, &t.slot)
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

/// Viewport input of the cut and paint tools, in the rest pose or on the
/// posed rig (pose mode, clip preview): the pointer is mapped through the
/// shown deformation to the picture. The pose itself is never changed.
pub fn interact(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State, response: &egui::Response, rect: Rect) {
    if ui.input(|i| i.key_down(egui::Key::Space)) {
        return;
    }
    let Some(t) = target(ctx, st) else { return };
    let primary = egui::PointerButton::Primary;
    let (shift, cmd, alt) = ui.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
    let pointer = response.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()));
    let Some(pos) = pointer else { return };
    let rig = pointer_rig(st, rect, pos);
    let p = t.to_px(rig);
    let center = center_of(rect);
    let tool = current(st);
    match tool {
        PixTool::Lasso => {
            if response.drag_started_by(primary) {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                st.px.draft = vec![pointer_rig(st, rect, origin)];
            }
            if response.dragged_by(primary)
                && let Some(last) = st.px.draft.last()
                && to_pos(st.view.to_screen(center, *last)).distance(pos) >= 1.0
            {
                st.px.draft.push(rig);
            }
            if response.drag_stopped_by(primary) {
                close_polygon(st, &t, shift, cmd);
            }
        }
        PixTool::Polygon => {
            if response.double_clicked_by(primary) {
                close_polygon(st, &t, shift, cmd);
            } else if response.clicked_by(primary) {
                let near_first =
                    st.px.draft.first().is_some_and(|f| to_pos(st.view.to_screen(center, *f)).distance(pos) < 8.0);
                if near_first && st.px.draft.len() >= 3 {
                    close_polygon(st, &t, shift, cmd);
                } else {
                    st.px.draft.push(rig);
                }
            }
        }
        PixTool::Wand => {
            if response.clicked_by(primary)
                && let Some(p) = p
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
                        if let Some(src) = t.shown.to_px_extrapolated(rig) {
                            st.px.clone_source = Some(src);
                        }
                    } else if let Some(p) = p {
                        pick(st, &t, p);
                    }
                }
                return;
            }
            if response.drag_started_by(primary) {
                let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                begin_stroke(ctx, st, &t, tool, pointer_rig(st, rect, origin));
            }
            if response.dragged_by(primary) {
                continue_stroke(st, &t, rig);
            }
            if response.clicked_by(primary) {
                begin_stroke(ctx, st, &t, tool, rig);
                finish_stroke(st);
            }
        }
        PixTool::Fill => {
            if response.clicked_by(primary)
                && let Some(p) = p
            {
                fill_at(ctx, st, &t, p);
            }
        }
        PixTool::Picker => {
            if response.clicked_by(primary)
                && let Some(p) = p
            {
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

/// Selection, outline being drawn, picture frame and brush cursor, drawn on
/// the part as shown (posed in pose mode).
pub fn paint_overlay(ui: &Ui, st: &mut State, painter: &egui::Painter, rect: Rect) {
    let Some(t) = loaded_target(st) else { return };
    let center = center_of(rect);
    let view = st.view;
    let on_screen = |rig: [f32; 2]| to_pos(view.to_screen(center, rig));
    let screen = |px: [f32; 2]| t.to_rig(px).map(on_screen);
    let (w, h) = (t.w as f32, t.h as f32);
    let frame = egui::Stroke::new(1.0, faint(ui));
    match &*t.shown {
        PicMap::Affine(m) => {
            let corners: Vec<Pos2> = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].iter().map(|p| on_screen(m.apply(*p))).collect();
            painter.add(egui::Shape::closed_line(corners, frame));
        }
        PicMap::Mesh(m) => {
            for e in m.outline() {
                painter.line_segment([on_screen(e[0]), on_screen(e[1])], frame);
            }
        }
    }
    // Selection: the mask as a texture on the shown part.
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
            let vertex = |pos: Pos2, uv: [f32; 2]| egui::epaint::Vertex { pos, uv: Pos2::new(uv[0], uv[1]), color: Color32::WHITE };
            match &*t.shown {
                PicMap::Affine(a) => {
                    for uv in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
                        mesh.vertices.push(vertex(on_screen(a.apply([uv[0] * w, uv[1] * h])), uv));
                    }
                    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                }
                PicMap::Mesh(tm) => {
                    for (p, px) in tm.pos.iter().zip(&tm.px) {
                        mesh.vertices.push(vertex(on_screen(*p), [px[0] / w.max(1.0), px[1] / h.max(1.0)]));
                    }
                    mesh.indices.extend(tm.tris.iter().flat_map(|t| t.map(u32::from)));
                }
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
    // Outline being drawn (rig space, as shown).
    if !st.px.draft.is_empty() {
        let mut pts: Vec<Pos2> = st.px.draft.iter().map(|p| on_screen(*p)).collect();
        if st.tool == Tool::Cut
            && st.px.cut_tool == PixTool::Polygon
            && let Some(hover) = ui.input(|i| i.pointer.hover_pos())
        {
            pts.push(hover);
        }
        painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(1.5, accent(ui))));
        if let Some(first) = pts.first() {
            painter.circle_stroke(*first, 4.0, egui::Stroke::new(1.0, accent(ui)));
        }
    }
    // Clone source (where the part shows it).
    if st.tool == Tool::Paint
        && st.px.paint_tool == PixTool::Clone
        && let Some(src) = st.px.clone_source
        && let Some(c) = screen(src)
    {
        let s = egui::Stroke::new(1.0, accent(ui));
        painter.line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], s);
        painter.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], s);
    }
    // Brush cursor: the brush size on screen (it keeps that size on a
    // posed, stretched part; the dabs follow the local stretch).
    if st.tool == Tool::Paint
        && matches!(st.px.paint_tool, PixTool::Brush | PixTool::Eraser | PixTool::Clone)
        && let Some(hover) = ui.input(|i| i.pointer.hover_pos())
        && rect.contains(hover)
    {
        let per_rig = on_screen([0.0, 0.0]).distance(on_screen([1.0, 0.0]));
        let r = (st.px.brush.size * 0.5 / t.rest_scale() * per_rig).max(1.0);
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

    const W: u32 = 40;
    const H: u32 = 20;

    /// A bone and a part showing an opaque `W`×`H` picture (a region, or a
    /// mesh following the bone), selected; its pixels are in the document.
    fn posable(mesh: bool) -> (State, orbit_anim::rig::BoneId) {
        let mut st = state();
        let rgba: Vec<u8> = (0..W * H).flat_map(|i| [(i % W * 6) as u8, (i / W * 12) as u8, 90, 255]).collect();
        let pic = Picture { path: "p.png".into(), width: W, height: H, rgba: rgba.clone() };
        let doc = st.doc.as_mut().unwrap();
        let bone = doc.add_bone(None, [0.0, 0.0], [50.0, 0.0]).unwrap();
        let slot = if mesh {
            workflow::add_part(doc, &bone, &pic, [30.0, 12.0]).unwrap()
        } else {
            let slot = doc.add_slot(&bone).unwrap();
            doc.attach_image(&slot, DEFAULT_SKIN, "p.png", pic.size()).unwrap();
            slot
        };
        doc.set_canvas("p.png", W, H, rgba);
        st.selected_slot = Some(slot);
        st.sync();
        (st, bone)
    }

    fn pose_bone(st: &mut State, bone: &orbit_anim::rig::BoneId, deg: f32) {
        super::super::set_pose_mode(st, true);
        st.set_local(bone, orbit_anim::rig::Xform { rotation: deg, ..orbit_anim::rig::Xform::IDENTITY });
    }

    /// Paints a short stroke at picture pixels `a` → `b`, seen where the
    /// part shows them now; returns the pixels.
    fn stroke_at(st: &mut State, a: [f32; 2], b: [f32; 2]) -> Vec<u8> {
        let t = loaded_target(st).unwrap();
        let c = st.doc.as_ref().unwrap().canvas("p.png").unwrap().clone();
        let brush = Brush { size: 5.0, hardness: 1.0, opacity: 1.0, color: [255, 0, 255, 255] };
        st.px.stroke = Some(("p.png".into(), paint::Stroke::new(c.w, c.h, c.rgba.clone(), Mode::Paint, brush)));
        st.px.stroke_last = None;
        continue_stroke(st, &t, t.to_rig(a).unwrap());
        continue_stroke(st, &t, t.to_rig(b).unwrap());
        let out = st.doc.as_ref().unwrap().canvas("p.png").unwrap().rgba.clone();
        // Back to the original pixels for the next stroke.
        st.px.stroke = None;
        st.px.stroke_last = None;
        st.doc.as_mut().unwrap().paint_live("p.png", |p| {
            p.rgba.copy_from_slice(&c.rgba);
            true
        });
        out
    }

    #[test]
    fn a_stroke_on_the_posed_part_paints_the_same_pixels_as_at_rest() {
        for mesh in [false, true] {
            let (mut st, bone) = posable(mesh);
            let before = st.doc.as_ref().unwrap().canvas("p.png").unwrap().rgba.clone();
            let rest = stroke_at(&mut st, [10.5, 9.5], [26.5, 9.5]);
            let rig = st.doc.as_ref().unwrap().rig().clone();
            pose_bone(&mut st, &bone, 73.0);
            let shown_rest = loaded_target(&mut st).unwrap();
            assert!(matches!(&*shown_rest.shown, PicMap::Affine(_)) != mesh, "mesh {mesh}: the posed map follows the part");
            let posed = stroke_at(&mut st, [10.5, 9.5], [26.5, 9.5]);
            let px = |v: &[u8], x: u32, y: u32| v[((y * W + x) * 4) as usize..][..4].to_vec();
            assert_eq!(px(&rest, 18, 9), vec![255, 0, 255, 255], "mesh {mesh}");
            assert_eq!(px(&rest, 18, 2), px(&before, 18, 2), "mesh {mesh}");
            let differ = rest.chunks(4).zip(posed.chunks(4)).filter(|(a, b)| a != b).count();
            // Rigid (or rigidly skinned) part: the same pixels, to the
            // rounding of a dab edge.
            assert!(differ <= 4, "mesh {mesh}: {differ} pixels differ");
            // The tools never touch the pose or the rig.
            assert_eq!(st.doc.as_ref().unwrap().rig(), &rig);
            assert_eq!(st.pose.len(), 1);
        }
    }

    #[test]
    fn a_selection_drawn_on_the_posed_part_cuts_the_same_pixels() {
        for mesh in [false, true] {
            let (mut st, bone) = posable(mesh);
            let corners = [[8.0, 4.0], [24.0, 4.0], [24.0, 14.0], [8.0, 14.0]];
            let select = |st: &mut State| {
                let t = loaded_target(st).unwrap();
                st.px.draft = corners.iter().map(|p| t.to_rig(*p).unwrap()).collect();
                close_polygon(st, &t, false, false);
                st.px.selection.clone().unwrap().1
            };
            let rest = select(&mut st);
            st.px.set_selection(None);
            pose_bone(&mut st, &bone, -128.0);
            let posed = select(&mut st);
            assert_eq!(rest.count(), 16 * 10);
            assert_eq!(rest.bits, posed.bits);
            let doc = st.doc.as_ref().unwrap();
            let c = doc.canvas("p.png").unwrap();
            let src = Picture { path: "p.png".into(), width: c.w, height: c.h, rgba: c.rgba.clone() };
            let sid = st.selected_slot.clone().unwrap();
            let spec = rigops::CutSpec { slot: sid.as_str(), new_image: "cut.png", ..Default::default() };
            let a = rigops::cut_part(&mut doc.rig().clone(), &src, &rest, &spec).unwrap();
            let b = rigops::cut_part(&mut doc.rig().clone(), &src, &posed, &spec).unwrap();
            assert_eq!((a.part.width, a.part.height, a.offset), (b.part.width, b.part.height, b.offset));
            assert_eq!(a.part.rgba, b.part.rgba);
        }
    }

    // ---- background cut (jobs)

    /// A rig saved in `dir` with one big opaque mesh part `p.png` (`n`×`n`,
    /// alpha with a hole pattern so the trace has work), a selection of its
    /// left half.
    fn big_part(dir: &Path, n: u32) -> State {
        let mut st = state();
        let rgba: Vec<u8> = (0..n * n)
            .flat_map(|i| {
                let (x, y) = (i % n, i / n);
                let a = if (x / 37 + y / 41) % 5 == 0 { 0 } else { 255 };
                [(x % 251) as u8, (y % 241) as u8, 90, a]
            })
            .collect();
        let pic = Picture { path: "p.png".into(), width: n, height: n, rgba: rgba.clone() };
        let doc = st.doc.as_mut().unwrap();
        doc.path = Some(dir.join("a.rig.ron"));
        let bone = doc.add_bone(None, [0.0, 0.0], [50.0, 0.0]).unwrap();
        let slot = workflow::add_part(doc, &bone, &pic, [0.0, 0.0]).unwrap();
        doc.set_canvas("p.png", n, n, rgba);
        st.selected_slot = Some(slot.clone());
        st.sync();
        let mut m = Mask::new(n, n);
        for y in n / 8..n - n / 8 {
            for x in n / 8..n / 2 {
                m.set(x, y, true);
            }
        }
        combine(&mut st, &slot, m, false, false);
        st
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("orbit_rig_job_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Polls the job like the panels do each frame; returns the longest
    /// frame spent waiting (not applying) and the applying frame.
    fn run_frames(ctx: &mut EditorContext<'_>, st: &mut State) -> (std::time::Duration, std::time::Duration) {
        let start = std::time::Instant::now();
        let mut worst_wait = std::time::Duration::ZERO;
        loop {
            let frame = std::time::Instant::now();
            super::super::jobs::poll(ctx, st);
            let took = frame.elapsed();
            if st.job.is_none() {
                return (worst_wait, took);
            }
            worst_wait = worst_wait.max(took);
            assert!(start.elapsed() < std::time::Duration::from_secs(120), "the job never ended");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn a_cut_runs_in_the_background_and_lands_as_one_undo_step() {
        let dir = temp("cut");
        let mut st = big_part(&dir, 1024);
        let registry = orbit_editor_api::EditorRegistry::new();
        let mut es = orbit_editor_api::EditorState::default();
        let mut ctx = EditorContext::new(None, &mut es, &registry);
        let rig_before = st.doc.as_ref().unwrap().rig().clone();

        // What the UI thread used to do: a dry run, then the cut.
        let t = loaded_target(&mut st).unwrap();
        let (sid, mask) = st.px.selection.clone().unwrap();
        let c = st.doc.as_ref().unwrap().canvas(&t.path).unwrap().clone();
        let src = Picture { path: t.path.clone(), width: c.w, height: c.h, rgba: c.rgba };
        let dry = rigops::CutSpec { slot: sid.as_str(), new_image: "x.png", ..Default::default() };
        let spec = rigops::CutSpec { slot: sid.as_str(), new_image: "x.png", fill_hole: true, ..Default::default() };
        let old = std::time::Instant::now();
        rigops::cut_part(&mut rig_before.clone(), &src, &mask, &dry).unwrap();
        rigops::cut_part(&mut rig_before.clone(), &src, &mask, &spec).unwrap();
        let old = old.elapsed();

        let spawn = std::time::Instant::now();
        cut(&mut ctx, &mut st);
        let spawn = spawn.elapsed();
        assert!(st.job.is_some(), "the cut runs as a job");
        // While it runs, other heavy rig actions wait.
        let id = st.job.as_ref().unwrap().handle.id();
        retrace(&mut ctx, &mut st);
        assert_eq!(st.job.as_ref().unwrap().handle.id(), id, "the retrace waits");
        assert!(ctx.state().toasts.iter().any(|t| t.level == orbit_editor_api::Level::Warn));
        let (wait, apply) = run_frames(&mut ctx, &mut st);
        eprintln!("rig cut 1024²: UI thread before {old:?}; now {spawn:?} to start, frames ≤ {wait:?} while it runs, {apply:?} to apply");
        // The UI thread never waits for the cut: a frame while it runs is
        // only a poll.
        assert!(wait < std::time::Duration::from_millis(16), "{wait:?}");

        let doc = st.doc.as_mut().unwrap();
        assert_eq!(doc.rig().slots.len(), rig_before.slots.len() + 1, "one new slot");
        let written: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".png"))
            .collect();
        assert!(written.iter().any(|n| n.contains("_cut")), "the job wrote the part: {written:?}");
        // One undo step brings the rig back.
        assert!(doc.undo());
        assert_eq!(doc.rig(), &rig_before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cut_result_on_a_changed_rig_is_refused_and_its_files_removed() {
        let dir = temp("stale");
        let mut st = big_part(&dir, 256);
        let registry = orbit_editor_api::EditorRegistry::new();
        let mut es = orbit_editor_api::EditorState::default();
        let mut ctx = EditorContext::new(None, &mut es, &registry);
        cut(&mut ctx, &mut st);
        let files = st.job.as_ref().unwrap().files.lock().unwrap().clone();
        assert!(!files.is_empty());
        // The user edits the rig while the cut runs.
        let doc = st.doc.as_mut().unwrap();
        doc.add_bone(None, [0.0, 0.0], [0.0, 30.0]).unwrap();
        let edited = doc.rig().clone();
        run_frames(&mut ctx, &mut st);
        assert_eq!(st.doc.as_ref().unwrap().rig(), &edited, "nothing applied over the edit");
        assert!(files.iter().all(|f| !f.exists()), "the part written by the job is removed");
        assert!(ctx.state().toasts.iter().any(|t| t.level == orbit_editor_api::Level::Warn));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
