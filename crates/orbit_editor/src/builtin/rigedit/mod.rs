//! The 2D rig editor (`orbit.rig`, M5B-03): creates, opens and saves
//! `.rig.ron` documents and edits them with three panels.
//!
//! * **Viewport**: the rig drawn by `orbit_render2d` into an egui texture (on
//!   demand), a grid, pan / zoom, bone creation by click-drag (the parent is
//!   the selected bone), bone picking and move / rotate / scale handles, in
//!   *rest pose* (edits the setup pose) or *pose mode* (a throw-away pose to
//!   try the rig, IK included);
//! * **Hierarchy**: bones (select, rename, reparent, delete) and slots in draw
//!   order;
//! * **Inspector**: bone transform, slot pictures per skin (outfit), skins,
//!   IK constraints and the validation diagnostics.
//!
//! The document logic is in [`model`] (undo / redo as a snapshot stack), the
//! viewport math in [`geom`]: headless and unit tested. The GPU work is done
//! by the host ([`crate::rig_host`]), which this plugin only talks to through
//! the shared [`RigLink`].

mod geom;
mod meshedit;
mod clippanel;
mod clips;
use orbit_project::rigmesh::meshgen;
mod model;
mod paint;
mod panels;
mod pixeltools;
use orbit_project::rigmesh::weights;
mod workflow;
mod layered_import;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};
use orbit_anim::rig::{Affine, Attachment, BoneId, PreparedRig, RigPose, SlotId, Xform};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, PanelDef, Plugin,
    PluginManifest, icons,
};

use self::geom::{Handle, Tool, View};
use self::model::RigDoc;
use super::ids;
use super::panels as panel_ids;
use super::ui_util::{accent, danger, faint, icon_button, weak};
use crate::rig_host::{RigRequest, SharedLink};

pub struct RigPlugin {
    state: Rc<RefCell<State>>,
}

impl RigPlugin {
    pub fn new(link: SharedLink) -> Self {
        let state = Rc::new(RefCell::new(State::new(link)));
        // The MCP rig tools edit the open rig through its document (undo,
        // refresh); they run on this thread.
        orbit_mcp::rig_tools::set_live_rig(Some(Rc::new(LiveBridge { state: state.clone() })));
        Self { state }
    }
}

/// What the MCP rig tools see of the rig editor.
struct LiveBridge {
    state: Rc<RefCell<State>>,
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

impl orbit_mcp::rig_tools::LiveRig for LiveBridge {
    fn current(&self, abs: &std::path::Path) -> Option<orbit_anim::rig::Rig> {
        let st = self.state.try_borrow().ok()?;
        let doc = st.doc.as_ref()?;
        same_file(doc.path.as_deref()?, abs).then(|| doc.rig().clone())
    }

    fn apply(&self, abs: &std::path::Path, rig: &orbit_anim::rig::Rig, _label: &str) -> Result<bool, String> {
        let Ok(mut st) = self.state.try_borrow_mut() else { return Ok(false) };
        let Some(doc) = st.doc.as_mut() else { return Ok(false) };
        if !doc.path.as_deref().is_some_and(|p| same_file(p, abs)) {
            return Ok(false);
        }
        doc.replace(rig.clone());
        doc.save()?;
        Ok(true)
    }
}

impl Plugin for RigPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::RIG, "Rig editor", "0.1.0")
    }
}

impl EditorPlugin for RigPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        // Saving the project saves the open rig too.
        let saver = self.state.clone();
        reg.project.on_project_event(move |event| {
            if matches!(event, orbit_plugin_api::ProjectEvent::BeforeSave)
                && let Ok(mut st) = saver.try_borrow_mut()
                && let Some(doc) = st.doc.as_mut()
                && doc.dirty()
                && doc.path.is_some()
            {
                let _ = doc.save();
            }
        });
        let state = self.state.clone();
        reg.panel(PanelDef::new("viewport", "rig.title.viewport", icons::BONE, DockSlot::Hidden, move |ui, ctx| {
            viewport_panel(ui, ctx, &state)
        }));
        let state = self.state.clone();
        reg.panel(PanelDef::new(
            "hierarchy",
            "rig.title.hierarchy",
            icons::TREE_STRUCTURE,
            DockSlot::Hidden,
            move |ui, ctx| panels::hierarchy(ui, ctx, &state),
        ));
        let state = self.state.clone();
        reg.panel(PanelDef::new("inspector", "rig.title.inspector", icons::SLIDERS, DockSlot::Hidden, move |ui, ctx| {
            panels::inspector(ui, ctx, &state)
        }));
        let state = self.state.clone();
        reg.panel(PanelDef::new("clips", "rig.title.clips", icons::FILM_STRIP, DockSlot::Hidden, move |ui, ctx| {
            clippanel::clips_panel(ui, ctx, &state)
        }));
        let state = self.state.clone();
        reg.panel(PanelDef::new("timeline", "rig.title.timeline", icons::FILM_STRIP, DockSlot::Hidden, move |ui, ctx| {
            clippanel::timeline_panel(ui, ctx, &state)
        }));
        reg.action(
            ActionDef::new("open", "rig.open", |ctx| {
                ctx.request(EditorRequest::ShowWorkspace(rig_workspace()));
            })
            .icon(icons::BONE)
            .category("category.tools")
            .menu("tools", 165),
        );
        let state = self.state.clone();
        reg.action(
            ActionDef::new("new_from_image", "rig.image.new", move |ctx| {
                ctx.request(EditorRequest::ShowWorkspace(rig_workspace()));
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.request_new_from_image(ctx);
                }
            })
            .icon(icons::USER_SQUARE)
            .category("category.tools")
            .menu("tools", 166),
        );
        let state = self.state.clone();
        reg.action(
            ActionDef::new("import_layers", "rig.layers.new", move |ctx| {
                ctx.request(EditorRequest::ShowWorkspace(rig_workspace()));
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.request_import_layers(ctx, true);
                }
            })
            .icon(icons::STACK)
            .category("category.tools")
            .menu("tools", 167),
        );
        let state = self.state.clone();
        reg.action(
            ActionDef::new("add_layers", "rig.layers.add", move |ctx| {
                ctx.request(EditorRequest::ShowWorkspace(rig_workspace()));
                if let Ok(mut st) = state.try_borrow_mut() {
                    st.request_import_layers(ctx, false);
                }
            })
            .icon(icons::STACK)
            .category("category.tools")
            .menu("tools", 168),
        );
    }
}

// ------------------------------------------------------------------ state

/// An action waiting for the answer to "discard unsaved changes?".
pub(super) enum Pending {
    New,
    Open(PathBuf),
    /// "New rig from image": the picture picked or dropped.
    NewFromImage(PathBuf),
}

/// What the viewport last asked the host to draw.
#[derive(PartialEq)]
struct Sent {
    revision: u64,
    pixel_rev: u64,
    pose_rev: u64,
    view: [u32; 3],
    size: [u32; 2],
    skin: String,
}

/// A handle drag in progress.
pub(super) struct Drag {
    bone: BoneId,
    handle: Handle,
    start: Xform,
    start_ptr: [f32; 2],
    head: [f32; 2],
    axis: [f32; 2],
    parent_world: Option<Affine>,
}

pub(super) struct State {
    pub link: SharedLink,
    pub doc: Option<RigDoc>,
    pub view: View,
    pub tool: Tool,
    pub pose_mode: bool,
    /// Pose mode transforms (local, absolute) by bone; never saved.
    pub pose: HashMap<BoneId, Xform>,
    pose_rev: u64,
    pub selected: Option<BoneId>,
    pub selected_slot: Option<SlotId>,
    /// Skin shown in the viewport and edited by the picture pickers.
    pub skin: String,
    pub show_grid: bool,
    drag: Option<Drag>,
    /// Bone being drawn: head in rig space.
    create: Option<[f32; 2]>,
    pub pending: Option<Pending>,
    /// Inline rename: `(bone or slot id, buffer)`.
    pub renaming: Option<(String, String)>,
    pub error: Option<String>,
    prepared_rev: u64,
    pub prepared: Option<Rc<PreparedRig>>,
    sent: Option<Sent>,
    framed: bool,
    pub image_sizes: HashMap<String, [f32; 2]>,
    /// Target bone chosen in the IK form, and its chain length.
    pub ik_target: Option<BoneId>,
    pub ik_two: bool,
    /// Mesh and weight tools.
    pub(super) mesh: meshedit::MeshEdit,
    /// Clip editing and preview (M5B-06).
    anim: clippanel::AnimState,
    /// Bones of the last "Bind to bones" (a bone added since asks to bind
    /// again in the step bar).
    pub bound_bones: Option<Vec<BoneId>>,
    /// Cut and paint tools.
    pub(super) px: pixeltools::PixelState,
}

impl State {
    fn new(link: SharedLink) -> Self {
        Self {
            link,
            doc: None,
            view: View::default(),
            tool: Tool::Create,
            pose_mode: false,
            pose: HashMap::new(),
            pose_rev: 0,
            selected: None,
            selected_slot: None,
            skin: model::DEFAULT_SKIN.to_owned(),
            show_grid: true,
            drag: None,
            create: None,
            pending: None,
            renaming: None,
            error: None,
            prepared_rev: 0,
            prepared: None,
            sent: None,
            framed: false,
            image_sizes: HashMap::new(),
            ik_target: None,
            ik_two: true,
            mesh: meshedit::MeshEdit::default(),
            anim: clippanel::AnimState::default(),
            bound_bones: None,
            px: pixeltools::PixelState::default(),
        }
    }

    pub fn set_doc(&mut self, doc: RigDoc) {
        self.skin = doc
            .rig()
            .skins
            .iter()
            .find(|s| s.name == model::DEFAULT_SKIN)
            .or(doc.rig().skins.first())
            .map_or_else(|| model::DEFAULT_SKIN.to_owned(), |s| s.name.clone());
        let edited = self.doc.as_ref().map(|d| d.canvases().map(|(p, _)| p.clone()).collect()).unwrap_or_default();
        self.px.reset(edited);
        self.doc = Some(doc);
        self.selected = None;
        self.selected_slot = None;
        self.pose.clear();
        self.pose_mode = false;
        self.anim.reset();
        self.drag = None;
        self.create = None;
        self.renaming = None;
        self.prepared_rev = 0;
        self.prepared = None;
        self.sent = None;
        self.framed = false;
        self.error = None;
        self.view = View::default();
        self.bound_bones = None;
    }

    pub fn new_rig(&mut self) {
        self.set_doc(RigDoc::new());
        self.tool = Tool::Create;
    }

    pub fn open_path(&mut self, path: &std::path::Path, ctx: &mut EditorContext<'_>) {
        match RigDoc::load(path) {
            Ok(doc) => self.set_doc(doc),
            Err(e) => {
                ctx.error(ctx.tf("rig.load_failed", &[("error", &e)]));
                self.error = Some(e);
            }
        }
    }

    pub fn save(&mut self, ctx: &mut EditorContext<'_>) {
        let Some(doc) = self.doc.as_mut() else { return };
        let target = match doc.path.clone() {
            Some(p) => Some(p),
            None => {
                let mut dlg = rfd::FileDialog::new().add_filter("Rig", &["ron"]).set_file_name("character.rig.ron");
                if let Some(root) = ctx.project_info().map(|p| p.root.clone()) {
                    dlg = dlg.set_directory(root);
                }
                dlg.save_file()
            }
        };
        let Some(path) = target else { return };
        // Painted / cut pictures first: the rig names them.
        if !pixeltools::save_canvases(ctx, self) {
            return;
        }
        let Some(doc) = self.doc.as_mut() else { return };
        match doc.save_as(&path) {
            Ok(()) => {
                ctx.info(ctx.tf("rig.saved", &[("path", &path.display())]));
                self.declare_if_in_rig_dir(ctx);
            }
            Err(e) => ctx.error(ctx.tf("rig.save_failed", &[("error", &e)])),
        }
    }

    /// The manifest path of the open rig when it lies under `assets/`.
    fn project_rel(&self, ctx: &EditorContext<'_>) -> Option<String> {
        let root = ctx.project_info()?.root.clone();
        crate::assets::asset_rel_path(&root, self.doc.as_ref()?.path.as_ref()?)
    }

    /// Whether the open rig is a declared asset of the project.
    pub fn in_project(&self, ctx: &EditorContext<'_>) -> bool {
        let (Some(story), Some(rel)) = (ctx.story(), self.project_rel(ctx)) else {
            return false;
        };
        story.assets.assets.iter().any(|a| a.path == rel)
    }

    /// A rig saved in the project's rig folder is declared as an asset, so
    /// it shows in the characters panel.
    fn declare_if_in_rig_dir(&mut self, ctx: &mut EditorContext<'_>) {
        let Some(root) = ctx.project_info().map(|p| p.root.clone()) else { return };
        let Some(path) = self.doc.as_ref().and_then(|d| d.path.clone()) else { return };
        if !crate::assets::is_in_rig_dir(&root, &path) || self.in_project(ctx) {
            return;
        }
        let Some(rel) = self.project_rel(ctx) else { return };
        if ctx
            .ops_or_toast(|p| orbit_project::ops::add_asset(p, orbit_core::AssetKind::Rig, &rel))
            .is_some()
        {
            ctx.info(ctx.tf("rig.declared", &[("path", &rel)]));
        }
    }

    /// Saves the open rig into the project (where it is when already under
    /// `assets/`, else `assets/rigs/`) and declares it as a `Rig` asset.
    pub fn add_to_project(&mut self, ctx: &mut EditorContext<'_>) {
        let Some(root) = ctx.project_info().map(|p| p.root.clone()) else {
            ctx.warn(ctx.t("rig.no_project"));
            return;
        };
        let Some(doc) = self.doc.as_mut() else { return };
        let name = doc
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "character".to_owned(), |n| n.to_string_lossy().into_owned());
        let name = name.strip_suffix(".rig").map_or(name.clone(), str::to_owned);
        let target = {
            let Some(story) = ctx.story() else { return };
            crate::assets::plan_rig_target(&root, story, &name, doc.path.as_deref())
        };
        if let Some(parent) = target.abs.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = doc.save_as(&target.abs) {
            ctx.error(ctx.tf("rig.save_failed", &[("error", &e)]));
            return;
        }
        let rel = target.rel.clone();
        let declared = ctx.ops_or_toast(|p| orbit_project::ops::add_asset(p, orbit_core::AssetKind::Rig, &rel));
        if declared.is_some() {
            ctx.info(ctx.tf("rig.declared", &[("path", &target.rel)]));
        }
    }

    pub fn request_open(&mut self, ctx: &mut EditorContext<'_>) {
        let mut dlg = rfd::FileDialog::new().add_filter("Rig", &["ron"]);
        if let Some(root) = ctx.project_info().map(|p| p.root.clone()) {
            dlg = dlg.set_directory(root);
        }
        if let Some(path) = dlg.pick_file() {
            if self.doc.as_ref().is_some_and(RigDoc::dirty) {
                self.pending = Some(Pending::Open(path));
            } else {
                self.open_path(&path, ctx);
            }
        }
    }

    pub fn request_new(&mut self) {
        if self.doc.as_ref().is_some_and(RigDoc::dirty) {
            self.pending = Some(Pending::New);
        } else {
            self.new_rig();
        }
    }

    /// Drops selections of things that no longer exist and rebuilds the
    /// prepared rig when the document changed.
    pub fn sync(&mut self) {
        let Some(doc) = &self.doc else { return };
        let rig = doc.rig();
        if let Some(id) = &self.selected
            && model::bone(rig, id).is_none()
        {
            self.selected = None;
            self.drag = None;
        }
        if let Some(id) = &self.selected_slot
            && model::slot(rig, id).is_none()
        {
            self.selected_slot = None;
        }
        if !rig.skins.iter().any(|s| s.name == self.skin)
            && let Some(first) = rig.skins.first()
        {
            self.skin = first.name.clone();
        }
        let before = self.pose.len();
        self.pose.retain(|k, _| model::bone(rig, k).is_some());
        if self.pose.len() != before {
            self.pose_rev += 1;
        }
        if self.prepared_rev != doc.revision {
            self.prepared_rev = doc.revision;
            self.prepared = PreparedRig::new(rig.clone()).ok().map(Rc::new);
        }
    }

    pub fn begin_frame(&mut self, ui: &Ui) {
        if !ui.input(|i| i.pointer.any_down()) {
            // A paint stroke ends with its pixels in the same undo step.
            pixeltools::finish_stroke(self);
            if let Some(doc) = self.doc.as_mut() {
                doc.end_gesture();
            }
        }
        self.sync();
    }

    pub fn undo_redo_keys(&mut self, ui: &Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (undo, redo) = ui.input(|i| {
            let ctrl = i.modifiers.command;
            (
                ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                ctrl && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            )
        });
        if let Some(doc) = self.doc.as_mut() {
            if undo {
                doc.undo();
            } else if redo {
                doc.redo();
            }
        }
    }

    /// Local transforms shown: the rest pose, or in pose mode the pose
    /// overrides over the clip preview (or the rest pose).
    pub fn locals(&self) -> Vec<Xform> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let base = self.anim_pose();
        doc.rig()
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if !self.pose_mode {
                    return b.rest;
                }
                self.pose
                    .get(&b.id)
                    .copied()
                    .or_else(|| base.as_ref().and_then(|p| p.bones.get(i).copied()))
                    .unwrap_or(b.rest)
            })
            .collect()
    }

    /// The pose to draw: the clip preview (slots, deforms, draw order) or
    /// the setup pose, with the displayed bone transforms.
    pub fn rig_pose(&self) -> Option<RigPose> {
        let prepared = self.prepared.as_ref()?;
        let mut pose = self.anim_pose().unwrap_or_else(|| prepared.setup_pose());
        pose.bones = self.locals();
        Some(pose)
    }

    /// World matrices of the displayed pose (IK applied in pose mode).
    pub fn world(&self) -> Vec<Affine> {
        let Some(doc) = &self.doc else { return Vec::new() };
        if self.pose_mode
            && !doc.rig().ik.is_empty()
            && let (Some(p), Some(pose)) = (&self.prepared, self.rig_pose())
        {
            return p.world(&pose).bones;
        }
        model::world_matrices(doc.rig(), &self.locals())
    }

    /// Local transform of a bone as displayed.
    pub fn local_of(&self, id: &BoneId) -> Option<Xform> {
        let rig = self.doc.as_ref()?.rig();
        let i = model::bone_index(rig, id)?;
        let b = &rig.bones[i];
        if !self.pose_mode {
            return Some(b.rest);
        }
        Some(
            self.pose
                .get(id)
                .copied()
                .or_else(|| self.anim_pose().and_then(|p| p.bones.get(i).copied()))
                .unwrap_or(b.rest),
        )
    }

    /// Writes a bone transform where the current mode edits it: the rest
    /// pose, a key at the playhead (clip selected, auto-key on), or the
    /// throw-away pose.
    pub fn set_local(&mut self, id: &BoneId, x: Xform) {
        if self.pose_mode {
            if self.anim.fade.is_some() {
                return;
            }
            if self.recording() {
                self.pose.remove(id);
                self.record_bone_edit(id, x);
                return;
            }
            if self.pose.get(id) != Some(&x) {
                self.pose.insert(id.clone(), x);
                self.pose_rev += 1;
            }
        } else if let Some(doc) = self.doc.as_mut() {
            doc.begin_gesture();
            let _ = doc.set_rest(id, x);
        }
    }

    pub fn reset_pose(&mut self) {
        if !self.pose.is_empty() {
            self.pose.clear();
            self.pose_rev += 1;
        }
    }

    /// Directory the rig's picture paths are relative to: the project's
    /// assets root (where the importer copies files; a story kept in a
    /// sub-folder has it above its own folder), else the rig's folder.
    pub fn project_root(&self, ctx: &EditorContext<'_>) -> PathBuf {
        ctx.project_info()
            .map(|p| match ctx.story() {
                Some(story) => orbit_project::find_assets_root(&p.root, story),
                None => p.root.clone(),
            })
            .or_else(|| self.doc.as_ref()?.path.as_ref()?.parent().map(ToOwned::to_owned))
            .unwrap_or_default()
    }

    /// Frames the bones and the pictures.
    pub fn frame_all(&mut self, size: [f32; 2]) {
        let world = self.world();
        let pictures = match (&self.prepared, self.rig_pose()) {
            (Some(p), Some(pose)) => {
                let w = p.world(&pose);
                let list = p.draw_list(&pose, &w, self.shown_skin().as_deref(), &orbit_anim::rig::CpuSkinner);
                crate::rig_host::draw_bounds(&list)
            }
            _ => None,
        };
        let bones = self.doc.as_ref().and_then(|d| geom::rig_bounds(d.rig(), &world));
        let all = match (bones, pictures) {
            (Some((a, b)), Some((c, d))) => {
                Some(([a[0].min(c[0]), a[1].min(c[1])], [b[0].max(d[0]), b[1].max(d[1])]))
            }
            (x, y) => x.or(y),
        };
        if let Some((lo, hi)) = all {
            self.view = View::fit(lo, hi, size);
        } else {
            self.view = View::default();
        }
    }
}

// -------------------------------------------------------------- workflow

impl State {
    /// The skin to draw, when the rig has it.
    fn shown_skin(&self) -> Option<String> {
        let doc = self.doc.as_ref()?;
        doc.rig().skins.iter().any(|s| s.name == self.skin).then(|| self.skin.clone())
    }

    /// Path (as the rig stores it) of the picture file `abs`: an image asset
    /// of the project with the same content when there is one (a file just
    /// dropped on the window was imported by the editor already), else the
    /// file imported now. Without a project: the absolute path.
    fn resolve_picture(&mut self, ctx: &mut EditorContext<'_>, abs: &std::path::Path) -> Option<String> {
        if !ctx.has_project() {
            ctx.warn(ctx.t("rig.image.no_project"));
            return Some(abs.to_string_lossy().replace('\\', "/"));
        }
        let root = self.project_root(ctx);
        let bytes = match std::fs::read(abs) {
            Ok(b) => b,
            Err(e) => {
                ctx.error(ctx.tf("rig.mesh.err.image", &[("path", &abs.display()), ("error", &e)]));
                return None;
            }
        };
        let same = ctx.story().and_then(|story| {
            story
                .assets
                .assets
                .iter()
                .filter(|a| a.kind == orbit_core::AssetKind::Image)
                .find(|a| {
                    let p = root.join(&a.path);
                    std::fs::metadata(&p).is_ok_and(|m| m.len() == bytes.len() as u64)
                        && std::fs::read(&p).is_ok_and(|b| b == bytes)
                })
                .map(|a| a.path.clone())
        });
        if same.is_some() {
            return same;
        }
        let report = crate::builtin::import_paths(ctx, &[abs.to_owned()]);
        report.imported.into_iter().find(|i| i.kind == orbit_core::AssetKind::Image).map(|i| i.path)
    }

    /// The picture file `abs` as a project image (imported when needed).
    pub fn import_picture(&mut self, ctx: &mut EditorContext<'_>, abs: &std::path::Path) -> Option<String> {
        let path = self.resolve_picture(ctx, abs)?;
        self.image_sizes.remove(&path);
        Some(path)
    }

    fn load_picture(&mut self, ctx: &mut EditorContext<'_>, abs: &std::path::Path) -> Option<workflow::Picture> {
        let path = self.resolve_picture(ctx, abs)?;
        let root = self.project_root(ctx);
        match workflow::load_picture(&root, &path) {
            Ok(p) => {
                self.image_sizes.insert(path, [p.width as f32, p.height as f32]);
                Some(p)
            }
            Err(e) => {
                ctx.error(ctx.tf("rig.mesh.err.image", &[("path", &path), ("error", &e)]));
                None
            }
        }
    }

    /// "New rig from image…": asks for a picture (unsaved changes first).
    pub fn request_new_from_image(&mut self, ctx: &mut EditorContext<'_>) {
        let mut dlg = rfd::FileDialog::new().add_filter(ctx.t("rig.image.filter"), &["png", "jpg", "jpeg"]);
        if let Some(root) = ctx.project_info().map(|p| p.root.clone()) {
            dlg = dlg.set_directory(root);
        }
        if let Some(path) = dlg.pick_file() {
            self.request_new_from_path(ctx, path);
        }
    }

    pub fn request_new_from_path(&mut self, ctx: &mut EditorContext<'_>, path: PathBuf) {
        if self.doc.as_ref().is_some_and(RigDoc::dirty) {
            self.pending = Some(Pending::NewFromImage(path));
        } else {
            self.new_from_image(ctx, &path);
        }
    }

    /// Step 1: a rig with a root bone and the picture as a mesh, framed,
    /// the bone tool ready to draw over it.
    pub fn new_from_image(&mut self, ctx: &mut EditorContext<'_>, abs: &std::path::Path) {
        let Some(pic) = self.load_picture(ctx, abs) else { return };
        match workflow::rig_from_picture(&pic) {
            Ok((doc, root, slot)) => {
                self.set_doc(doc);
                self.selected = Some(root);
                self.selected_slot = Some(slot);
                self.tool = Tool::Create;
                self.sync();
                ctx.info(ctx.t("rig.image.created"));
            }
            Err(e) => ctx.warn(ctx.t(e.key())),
        }
    }

    /// "Add image part…": another picture as its own slot.
    pub fn request_add_part(&mut self, ctx: &mut EditorContext<'_>) {
        let mut dlg = rfd::FileDialog::new().add_filter(ctx.t("rig.image.filter"), &["png", "jpg", "jpeg"]);
        if let Some(root) = ctx.project_info().map(|p| p.root.clone()) {
            dlg = dlg.set_directory(root);
        }
        if let Some(paths) = dlg.pick_files() {
            for p in paths {
                self.add_part(ctx, &p, None);
            }
        }
    }

    /// Adds the picture `abs` as a slot on the selected bone (else the root):
    /// lined up with the first picture when it has its size, else at `at`
    /// (rig space) or the middle of the view. A rig without picture gets it
    /// as its main picture, standing on the origin.
    pub fn add_part(&mut self, ctx: &mut EditorContext<'_>, abs: &std::path::Path, at: Option<[f32; 2]>) {
        let Some(pic) = self.load_picture(ctx, abs) else { return };
        let Some(doc) = self.doc.as_mut() else { return };
        let first = !doc.rig().slots.iter().any(|s| s.attachment.is_some());
        doc.begin_gesture();
        let bone = match workflow::part_bone(doc.rig(), self.selected.as_ref()) {
            Some(b) => Ok(b),
            None => workflow::add_root(doc, &pic),
        };
        let view_center = [-self.view.pan[0] / self.view.zoom, -self.view.pan[1] / self.view.zoom];
        let center = if first {
            workflow::standing_center(&pic)
        } else {
            workflow::part_center(doc.rig(), &pic, at.unwrap_or(view_center))
        };
        let result = bone.and_then(|b| workflow::add_part(doc, &b, &pic, center).map(|s| (b, s)));
        doc.end_gesture();
        match result {
            Ok((bone, slot)) => {
                self.selected_slot = Some(slot);
                if self.selected.is_none() {
                    self.selected = Some(bone);
                }
                if first {
                    self.framed = false;
                }
                ctx.info(ctx.tf("rig.image.part_added", &[("name", &pic.path)]));
            }
            Err(e) => ctx.warn(ctx.t(e.key())),
        }
    }

    /// Step 3: automatic weights for every mesh from the bones, then pose
    /// mode with the rotate tool so the next drag bends the picture.
    pub fn bind_to_bones(&mut self, ctx: &mut EditorContext<'_>) {
        let falloff = self.mesh.falloff;
        let Some(doc) = self.doc.as_mut() else { return };
        if doc.rig().bones.len() < 2 {
            ctx.warn(ctx.t("rig.bind.need_bones"));
            return;
        }
        match workflow::bind(doc, falloff) {
            Ok(0) => ctx.warn(ctx.t("rig.bind.no_mesh")),
            Ok(n) => {
                self.bound_bones = Some(doc.rig().bones.iter().map(|b| b.id.clone()).collect());
                if self.selected_slot.is_none() {
                    self.selected_slot = doc
                        .rig()
                        .slots
                        .iter()
                        .find(|s| s.attachment.is_some())
                        .map(|s| s.id.clone());
                }
                if self.selected.as_ref().and_then(|b| model::bone(doc.rig(), b)).is_none_or(|b| b.parent.is_none()) {
                    self.selected = doc.rig().bones.iter().rev().find(|b| b.parent.is_some()).map(|b| b.id.clone());
                }
                set_pose_mode(self, true);
                self.tool = Tool::Rotate;
                ctx.info(ctx.tf("rig.bind.done", &[("count", &n)]));
            }
            Err(e) => ctx.warn(ctx.t(e.key())),
        }
    }

    /// Where the beginner workflow is.
    pub fn step(&self) -> workflow::Step {
        self.doc.as_ref().map_or(workflow::Step::Import, |d| workflow::step(d.rig(), self.bound_bones.as_deref()))
    }
}

/// Picture files dropped on `rect` this frame (a drop whose position is
/// unknown counts when the pointer was last seen over it).
/// The rig editor workspace: the viewport in the center, its side panels as
/// one sub-tab group under it.
fn rig_workspace() -> Vec<String> {
    [
        panel_ids::RIG_VIEWPORT,
        panel_ids::RIG_HIERARCHY,
        panel_ids::RIG_INSPECTOR,
        panel_ids::RIG_CLIPS,
        panel_ids::RIG_TIMELINE,
    ]
    .map(str::to_owned)
    .to_vec()
}

fn dropped_pictures(ui: &Ui, rect: Rect) -> Vec<PathBuf> {
    ui.input(|i| {
        if i.raw.dropped_files.is_empty() {
            return Vec::new();
        }
        let over = i.pointer.latest_pos().is_none_or(|p| rect.contains(p));
        if !over {
            return Vec::new();
        }
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| workflow::is_picture_file(p))
            .collect()
    })
}

/// Highlights `rect` while picture files are dragged over the window.
fn paint_drop_hint(ui: &Ui, ctx: &EditorContext<'_>, rect: Rect, has_doc: bool) {
    let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
    if !hovering {
        return;
    }
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("rig_drop")));
    painter.rect_filled(rect, 4.0, accent(ui).gamma_multiply(0.15));
    painter.rect_stroke(rect.shrink(2.0), 4.0, Stroke::new(2.0, accent(ui)), egui::StrokeKind::Inside);
    let key = if has_doc { "rig.drop.part" } else { "rig.drop.new" };
    painter.text(rect.center(), Align2::CENTER_CENTER, ctx.t(key), FontId::proportional(16.0), accent(ui));
}

/// The drops on the viewport: no rig → a new rig from the first picture;
/// else every picture becomes a part where it was dropped.
fn handle_drops(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State, rect: Rect) {
    let dropped = dropped_pictures(ui, rect);
    if dropped.is_empty() {
        return;
    }
    if st.doc.is_none() {
        st.request_new_from_path(ctx, dropped[0].clone());
        for p in &dropped[1..] {
            st.add_part(ctx, p, None);
        }
        return;
    }
    let at = ui.input(|i| i.pointer.latest_pos()).filter(|p| rect.contains(*p)).map(|p| pointer_rig(st, rect, p));
    for p in &dropped {
        st.add_part(ctx, p, at);
    }
}

/// The four steps of the beginner workflow, the current one highlighted,
/// with what to do next.
fn step_bar(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let step = st.step();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, key) in ["rig.step.import", "rig.step.bones", "rig.step.bind", "rig.step.pose"].iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new(icons::CARET_RIGHT).color(faint(ui)));
            }
            let text = format!("{}. {}", i + 1, ctx.t(key));
            let text = match i.cmp(&step.index()) {
                std::cmp::Ordering::Less => RichText::new(format!("{} {text}", icons::CHECK)).color(weak(ui)),
                std::cmp::Ordering::Equal => RichText::new(text).strong().color(accent(ui)),
                std::cmp::Ordering::Greater => RichText::new(text).color(faint(ui)),
            };
            ui.label(text);
        }
        ui.separator();
        let hint = match step {
            workflow::Step::Import => "rig.step.import_hint",
            workflow::Step::Bones => "rig.step.bones_hint",
            workflow::Step::Bind => "rig.step.bind_hint",
            workflow::Step::Pose => "rig.step.pose_hint",
        };
        ui.label(RichText::new(ctx.t(hint)).small().color(weak(ui)));
        match step {
            workflow::Step::Import => {
                if ui.button(format!("{}  {}", icons::IMAGE, ctx.t("rig.image.add_part"))).clicked() {
                    st.request_add_part(ctx);
                }
            }
            workflow::Step::Bones => {
                if st.tool != Tool::Create && ui.button(ctx.t("rig.tool.create")).clicked() {
                    set_pose_mode(st, false);
                    st.tool = Tool::Create;
                }
            }
            workflow::Step::Bind => {
                if ui.button(RichText::new(format!("{}  {}", icons::LINK, ctx.t("rig.bind"))).strong()).clicked() {
                    st.bind_to_bones(ctx);
                }
            }
            workflow::Step::Pose => {
                if !st.pose_mode && ui.button(ctx.t("rig.mode.pose")).clicked() {
                    set_pose_mode(st, true);
                    st.tool = Tool::Rotate;
                }
            }
        }
    });
}

// --------------------------------------------------------------- viewport

fn viewport_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &Rc<RefCell<State>>) {
    let mut guard = state.borrow_mut();
    let st = &mut *guard;
    st.begin_frame(ui);
    clippanel::tick(ui, st);
    if st.doc.is_none() {
        let full = ui.available_rect_before_wrap();
        panels::empty_state(ui, ctx, st);
        paint_drop_hint(ui, ctx, full, false);
        handle_drops(ui, ctx, st, full);
        confirm_dialog(ui, ctx, st);
        return;
    }
    toolbar(ui, ctx, st);
    pixeltools::options(ui, ctx, st);
    step_bar(ui, ctx, st);
    pixeltools::poll_external(ctx, st);
    ui.separator();
    let avail = ui.available_size();
    let (rect, response) =
        ui.allocate_exact_size(vec2(avail.x.max(32.0), avail.y.max(32.0)), Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let size_px = [(rect.width() * ppp).round().max(16.0) as u32, (rect.height() * ppp).round().max(16.0) as u32];
    if !st.framed {
        st.frame_all([rect.width(), rect.height()]);
        st.framed = true;
    }

    // ---- input
    navigate(ui, st, &response, rect);
    if response.hovered() || response.has_focus() || st.drag.is_some() {
        keys(ui, ctx, st, rect);
    }
    interact(ui, ctx, st, &response, rect);
    handle_drops(ui, ctx, st, rect);
    st.sync();
    if !st.framed {
        st.frame_all([rect.width(), rect.height()]);
        st.framed = true;
    }

    // ---- ask the host for a picture when something changed
    if let (Some(doc), Some(prepared), Some(pose)) = (&st.doc, st.prepared.clone(), st.rig_pose()) {
        let key = Sent {
            revision: doc.revision,
            pixel_rev: doc.pixel_rev,
            pose_rev: st.pose_rev,
            view: [st.view.zoom.to_bits(), st.view.pan[0].to_bits(), st.view.pan[1].to_bits()],
            size: size_px,
            skin: st.skin.clone(),
        };
        let texture_ready = st.link.borrow().texture.is_some();
        if st.sent.as_ref() != Some(&key) || !texture_ready {
            let skin = doc.rig().skins.iter().any(|s| s.name == st.skin).then(|| st.skin.clone());
            let request = RigRequest {
                rig: prepared,
                pose,
                skin,
                size: size_px,
                zoom: st.view.zoom * ppp,
                pan: [st.view.pan[0] * ppp, st.view.pan[1] * ppp],
                root: st.project_root(ctx),
                pixels: Vec::new(),
                forget: std::mem::take(&mut st.px.forget),
            };
            let request = RigRequest { pixels: pixeltools::uploads(st), ..request };
            st.link.borrow_mut().request = Some(request);
            st.sent = Some(key);
            if !texture_ready {
                ui.ctx().request_repaint();
            }
        }
    }

    // ---- paint
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(24));
    let (texture, available, error) = {
        let l = st.link.borrow();
        (l.texture, l.available, l.error.clone())
    };
    match texture {
        Some(tex) if available && st.prepared.is_some() => {
            painter.image(tex, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        _ => {
            let key = if st.prepared.is_none() { "rig.invalid" } else { "rig.no_gpu" };
            if st.show_grid {
                paint_grid(ui, st, &painter, rect);
            }
            painter.text(rect.center(), Align2::CENTER_CENTER, ctx.t(key), FontId::proportional(13.0), weak(ui));
        }
    }
    if texture.is_some() && st.show_grid {
        paint_grid(ui, st, &painter, rect);
    }
    if let Some(e) = error {
        painter.text(rect.left_top() + vec2(8.0, 8.0), Align2::LEFT_TOP, e, FontId::proportional(12.0), danger(ui));
    }
    paint_slot_outline(ui, st, &painter, rect);
    if pixeltools::is_pixel_tool(st.tool) {
        pixeltools::paint_overlay(ui, st, &painter, rect);
    }
    meshedit::paint(ui, st, &painter, rect);
    paint_bones(ui, st, &painter, rect);
    paint_handles(ui, st, &painter, rect);
    let mut mode = if st.pose_mode { ctx.t("rig.mode.pose") } else { ctx.t("rig.mode.rest") };
    if st.pose_mode && let Some(clip) = &st.anim.clip {
        let rec = if st.recording() { "  \u{25CF}" } else { "" };
        mode = format!("{mode}  -  {clip}  {:.2}s{rec}", st.anim.playhead);
    }
    painter.text(
        rect.right_top() + vec2(-8.0, 8.0),
        Align2::RIGHT_TOP,
        mode,
        FontId::proportional(12.0),
        if st.pose_mode { accent(ui) } else { weak(ui) },
    );
    painter.text(
        rect.left_bottom() + vec2(8.0, -6.0),
        Align2::LEFT_BOTTOM,
        ctx.t("rig.nav_hint"),
        FontId::proportional(11.0),
        faint(ui),
    );
    paint_drop_hint(ui, ctx, rect, true);
    pixeltools::new_layer_dialog(ui, ctx, st);
    confirm_dialog(ui, ctx, st);
}

fn center_of(rect: Rect) -> [f32; 2] {
    [rect.center().x, rect.center().y]
}

fn to_pos(p: [f32; 2]) -> Pos2 {
    Pos2::new(p[0], p[1])
}

fn pointer_rig(st: &State, rect: Rect, pos: Pos2) -> [f32; 2] {
    st.view.to_rig(center_of(rect), [pos.x, pos.y])
}

/// Pan (middle / right drag) and zoom (wheel).
fn navigate(ui: &Ui, st: &mut State, response: &egui::Response, rect: Rect) {
    if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) {
        let d = response.drag_delta();
        st.view.pan[0] += d.x;
        st.view.pan[1] += d.y;
    }
    if response.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = if (zoom - 1.0).abs() > 1e-4 { zoom } else { (scroll * 0.0015).exp() };
        if (factor - 1.0).abs() > 1e-4
            && let Some(pos) = response.hover_pos()
        {
            st.view.zoom_at(center_of(rect), [pos.x, pos.y], factor);
        }
    }
}

fn keys(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State, rect: Rect) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    st.undo_redo_keys(ui);
    if pixeltools::is_pixel_tool(st.tool) {
        pixeltools::keys(ui, st);
    }
    let (m, p) = ui.input(|i| {
        let plain = !i.modifiers.command && !i.modifiers.alt;
        (plain && i.key_pressed(egui::Key::M), plain && i.key_pressed(egui::Key::P))
    });
    if meshedit::has_mesh(st) {
        if m && !st.pose_mode {
            st.tool = Tool::Mesh;
        }
        if p {
            st.tool = Tool::Weights;
        }
    }
    let (b, w, e, r, f, del, tab) = ui.input(|i| {
        let plain = !i.modifiers.command && !i.modifiers.alt;
        (
            plain && i.key_pressed(egui::Key::B),
            plain && i.key_pressed(egui::Key::W),
            plain && i.key_pressed(egui::Key::E),
            plain && i.key_pressed(egui::Key::R),
            plain && i.key_pressed(egui::Key::F),
            plain && (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)),
            plain && i.key_pressed(egui::Key::Tab),
        )
    });
    if b && !st.pose_mode {
        st.tool = Tool::Create;
    }
    if w {
        st.tool = Tool::Move;
    }
    if e {
        st.tool = Tool::Rotate;
    }
    if r {
        st.tool = Tool::Scale;
    }
    if f {
        st.frame_all([rect.width(), rect.height()]);
    }
    if tab {
        set_pose_mode(st, !st.pose_mode);
    }
    if ui.input(|i| !i.modifiers.command && !i.modifiers.alt && i.key_pressed(egui::Key::K)) && st.pose_mode {
        st.key_pose();
    }
    if del && st.tool == Tool::Mesh {
        meshedit::delete_selected_vertex(ctx, st);
    } else if del && st.tool != Tool::Weights && let Some(id) = st.selected.clone() {
        panels::delete_bone(ctx, st, &id);
    }
}

pub(super) fn set_pose_mode(st: &mut State, pose: bool) {
    if st.pose_mode != pose {
        st.pose_mode = pose;
        st.pose_rev += 1;
        if !pose {
            // The clip preview only exists in pose mode.
            st.anim.playing = false;
            st.anim.fade = None;
        }
        st.drag = None;
        st.create = None;
        if pose && matches!(st.tool, Tool::Create | Tool::Mesh | Tool::Cut | Tool::Paint) {
            st.tool = Tool::Rotate;
        }
    }
}

fn interact(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State, response: &egui::Response, rect: Rect) {
    if matches!(st.tool, Tool::Mesh | Tool::Weights) {
        meshedit::interact(ui, ctx, st, response, rect);
        return;
    }
    if pixeltools::is_pixel_tool(st.tool) {
        pixeltools::interact(ui, ctx, st, response, rect);
        return;
    }
    let Some(doc) = st.doc.as_ref() else { return };
    let center = center_of(rect);
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let primary = egui::PointerButton::Primary;
    let world = st.world();
    let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));

    // ---- press
    if response.drag_started_by(primary)
        && let Some(pos) = response.interact_pointer_pos().or(pointer)
        && !ui.input(|i| i.key_down(egui::Key::Space))
    {
        let ptr = pointer_rig(st, rect, pos);
        let sp = [pos.x, pos.y];
        let selected = st.selected.clone();
        let mut began = false;
        if st.tool == Tool::Create && !st.pose_mode {
            st.create = Some(ptr);
            began = true;
        } else if let Some(sel) = &selected
            && let Some(i) = model::bone_index(doc.rig(), sel)
        {
            let b = &doc.rig().bones[i];
            let (h, t) = geom::bone_ends(&world[i], b.length, st.view.zoom);
            let (hs, ts) = (st.view.to_screen(center, h), st.view.to_screen(center, t));
            let hit = geom::handle_at(st.tool, hs, ts, sp).or_else(|| {
                (st.tool == Tool::Move && geom::hit_bone(doc.rig(), &world, &st.view, center, sp) == Some(i))
                    .then_some(Handle::Move)
            });
            if let Some(handle) = hit {
                st.drag = Some(drag_for(st, i, handle, ptr, &world));
                began = true;
            }
        }
        if !began {
            // Pick (and, with the move tool, start dragging what was picked).
            let picked = geom::hit_bone(doc.rig(), &world, &st.view, center, sp);
            st.selected = picked.map(|i| doc.rig().bones[i].id.clone());
            if let Some(i) = picked
                && st.tool == Tool::Move
            {
                st.drag = Some(drag_for(st, i, Handle::Move, ptr, &world));
            }
        }
    }

    // ---- drag
    if response.dragged_by(primary)
        && let Some(pos) = pointer
        && let Some(d) = st.drag.take()
    {
        let ptr = pointer_rig(st, rect, pos);
        let x = match d.handle {
            Handle::Move => geom::drag_move(d.parent_world.as_ref(), &d.start, d.start_ptr, ptr),
            Handle::Rotate => geom::drag_rotate(d.parent_world.as_ref(), &d.start, d.head, d.start_ptr, ptr, ctrl),
            Handle::Scale => geom::drag_scale(&d.start, d.head, d.axis, d.start_ptr, ptr, shift),
        };
        let id = d.bone.clone();
        st.drag = Some(d);
        st.set_local(&id, x);
    }

    // ---- release
    if response.drag_stopped_by(primary) {
        st.drag = None;
        if let Some(head) = st.create.take()
            && let Some(pos) = pointer
        {
            let tip = pointer_rig(st, rect, pos);
            let px = ((tip[0] - head[0]) * st.view.zoom).hypot((tip[1] - head[1]) * st.view.zoom);
            if px >= geom::MIN_CREATE_PX {
                let parent = st.selected.clone();
                if let Some(doc) = st.doc.as_mut()
                    && let Ok(id) = doc.add_bone(parent.as_ref(), head, tip)
                {
                    st.selected = Some(id);
                }
            }
        }
    } else if response.clicked_by(primary) {
        // A plain click (no drag): select / deselect.
        if let Some(pos) = pointer {
            let picked = st.doc.as_ref().and_then(|d| {
                geom::hit_bone(d.rig(), &world, &st.view, center, [pos.x, pos.y]).map(|i| d.rig().bones[i].id.clone())
            });
            st.selected = picked;
        }
    }
}

fn drag_for(st: &State, i: usize, handle: Handle, ptr: [f32; 2], world: &[Affine]) -> Drag {
    let rig = st.doc.as_ref().map(RigDoc::rig).expect("document");
    let b = &rig.bones[i];
    let parent_world = b.parent.as_ref().and_then(|p| model::bone_index(rig, p)).map(|pi| world[pi]);
    Drag {
        bone: b.id.clone(),
        handle,
        start: st.local_of(&b.id).unwrap_or(b.rest),
        start_ptr: ptr,
        head: world[i].origin(),
        axis: [world[i].a, world[i].b],
        parent_world,
    }
}

// ---------------------------------------------------------------- overlay

fn paint_grid(ui: &Ui, st: &State, painter: &egui::Painter, rect: Rect) {
    let center = center_of(rect);
    // A step of 10^k rig units that is at least ~24 px wide.
    let mut step = 1.0f32;
    while step * st.view.zoom < 24.0 {
        step *= 10.0;
    }
    let weak_line = Stroke::new(1.0, Color32::from_white_alpha(10));
    let [x0, y0] = st.view.to_rig(center, [rect.left(), rect.top()]);
    let [x1, y1] = st.view.to_rig(center, [rect.right(), rect.bottom()]);
    let mut x = (x0 / step).floor() * step;
    while x <= x1 {
        let sx = st.view.to_screen(center, [x, 0.0])[0];
        painter.line_segment([Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())], weak_line);
        x += step;
    }
    let mut y = (y0 / step).floor() * step;
    while y <= y1 {
        let sy = st.view.to_screen(center, [0.0, y])[1];
        painter.line_segment([Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)], weak_line);
        y += step;
    }
    let o = st.view.to_screen(center, [0.0, 0.0]);
    let axis = |c: Color32| Stroke::new(1.0, c.gamma_multiply(0.5));
    painter.line_segment([Pos2::new(rect.left(), o[1]), Pos2::new(rect.right(), o[1])], axis(Color32::from_rgb(200, 80, 80)));
    painter.line_segment([Pos2::new(o[0], rect.top()), Pos2::new(o[0], rect.bottom())], axis(Color32::from_rgb(80, 200, 110)));
    let _ = ui;
}

fn paint_slot_outline(ui: &Ui, st: &State, painter: &egui::Painter, rect: Rect) {
    let (Some(doc), Some(sid)) = (&st.doc, &st.selected_slot) else { return };
    let rig = doc.rig();
    let Some(slot) = model::slot(rig, sid) else { return };
    let Some(att) = slot.attachment.as_ref() else { return };
    let skin = rig.skins.iter().find(|s| s.name == st.skin).filter(|s| s.attachments.contains_key(att));
    let skin = skin.or_else(|| rig.skins.first().filter(|s| s.attachments.contains_key(att)));
    let Some(Attachment::Region(r)) = skin.and_then(|s| s.attachments.get(att)) else { return };
    let Some(bi) = model::bone_index(rig, &slot.bone) else { return };
    let world = st.world();
    let m = world[bi].mul(&r.xform.matrix());
    let (w, h) = (r.size[0] * 0.5, r.size[1] * 0.5);
    let center = center_of(rect);
    let pts: Vec<Pos2> = [[-w, -h], [w, -h], [w, h], [-w, h]]
        .iter()
        .map(|p| to_pos(st.view.to_screen(center, m.apply(*p))))
        .collect();
    painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5, accent(ui))));
}

fn paint_bones(ui: &Ui, st: &State, painter: &egui::Painter, rect: Rect) {
    let Some(doc) = &st.doc else { return };
    let rig = doc.rig();
    let world = st.world();
    let center = center_of(rect);
    let ik_bones: Vec<&BoneId> = rig.ik.iter().flat_map(|c| c.bones.iter()).collect();
    let ik_targets: Vec<&BoneId> = rig.ik.iter().map(|c| &c.target).collect();
    let hover = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|p| rect.contains(*p))
        .and_then(|p| geom::hit_bone(rig, &world, &st.view, center, [p.x, p.y]));
    for (i, b) in rig.bones.iter().enumerate() {
        let Some(w) = world.get(i) else { continue };
        let (h, t) = geom::bone_ends(w, b.length, st.view.zoom);
        let (hs, ts) = (to_pos(st.view.to_screen(center, h)), to_pos(st.view.to_screen(center, t)));
        let selected = st.selected.as_ref() == Some(&b.id);
        let base = if ik_targets.contains(&&b.id) {
            Color32::from_rgb(230, 170, 60)
        } else if ik_bones.contains(&&b.id) {
            Color32::from_rgb(110, 200, 150)
        } else {
            Color32::from_rgb(110, 160, 230)
        };
        let color = if selected { accent(ui) } else if hover == Some(i) { base } else { base.gamma_multiply(0.75) };
        let dir = ts - hs;
        let len = dir.length().max(1.0);
        let n = vec2(-dir.y, dir.x) / len;
        let half = (len * 0.14).clamp(2.5, 8.0);
        let mid = hs + dir * 0.18;
        let poly = vec![hs, mid + n * half, ts, mid - n * half];
        painter.add(egui::Shape::convex_polygon(poly, color.gamma_multiply(0.45), Stroke::new(1.3, color)));
        painter.circle_filled(hs, 3.2, color);
        if selected || hover == Some(i) {
            painter.text(ts + vec2(6.0, -6.0), Align2::LEFT_BOTTOM, &b.name, FontId::proportional(11.5), color);
        }
    }
    // The bone being drawn.
    if let (Some(head), Some(p)) = (st.create, ui.input(|i| i.pointer.latest_pos())) {
        let hs = to_pos(st.view.to_screen(center, head));
        painter.line_segment([hs, p], Stroke::new(2.0, accent(ui)));
        painter.circle_filled(hs, 4.0, accent(ui));
    }
}

fn paint_handles(ui: &Ui, st: &State, painter: &egui::Painter, rect: Rect) {
    let (Some(doc), Some(sel)) = (&st.doc, &st.selected) else { return };
    let rig = doc.rig();
    let Some(i) = model::bone_index(rig, sel) else { return };
    let world = st.world();
    let center = center_of(rect);
    let (h, t) = geom::bone_ends(&world[i], rig.bones[i].length, st.view.zoom);
    let (hs, ts) = (to_pos(st.view.to_screen(center, h)), to_pos(st.view.to_screen(center, t)));
    let c = accent(ui);
    match st.tool {
        Tool::Create | Tool::Mesh | Tool::Weights | Tool::Cut | Tool::Paint => {}
        Tool::Move => {
            painter.circle_stroke(hs, geom::GRAB_RADIUS, Stroke::new(2.0, c));
            painter.circle_filled(hs, 3.0, c);
        }
        Tool::Rotate => {
            painter.circle_stroke(hs, geom::RING_RADIUS, Stroke::new(2.0, c.gamma_multiply(0.85)));
            if let Some(p) = ui.input(|i| i.pointer.latest_pos())
                && st.drag.is_some()
            {
                painter.line_segment([hs, p], Stroke::new(1.0, c.gamma_multiply(0.7)));
            }
        }
        Tool::Scale => {
            let r = Rect::from_center_size(ts, vec2(geom::GRAB_RADIUS * 1.6, geom::GRAB_RADIUS * 1.6));
            painter.rect_stroke(r, 2.0, Stroke::new(2.0, c), egui::StrokeKind::Middle);
        }
    }
}

// ---------------------------------------------------------------- toolbar

fn toolbar(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let (dirty, can_undo, can_redo, name, has_path) = st.doc.as_ref().map_or(
        (false, false, false, String::new(), false),
        |d| {
            (
                d.dirty(),
                d.can_undo(),
                d.can_redo(),
                d.path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| ctx.t("rig.untitled")),
                d.path.is_some(),
            )
        },
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if icon_button(ui, icons::FILE_PLUS, &ctx.t("rig.new")).clicked() {
            st.request_new();
        }
        if icon_button(ui, icons::FOLDER_OPEN, &ctx.t("rig.open_file")).clicked() {
            st.request_open(ctx);
        }
        if icon_button(ui, icons::USER_SQUARE, &ctx.t("rig.image.new")).clicked() {
            st.request_new_from_image(ctx);
        }
        if icon_button(ui, icons::IMAGE, &ctx.t("rig.image.add_part")).clicked() {
            st.request_add_part(ctx);
        }
        if icon_button(ui, icons::STACK, &ctx.t(if st.doc.is_some() { "rig.layers.add" } else { "rig.layers.new" })).clicked() {
            st.request_import_layers(ctx, st.doc.is_none());
        }
        let can_bind = st.doc.as_ref().is_some_and(|d| d.rig().bones.len() >= 2);
        if ui
            .add_enabled(can_bind, egui::Button::new(RichText::new(icons::LINK).size(15.0)).frame(false))
            .on_hover_text(ctx.t("rig.bind_hint"))
            .on_disabled_hover_text(ctx.t("rig.bind.need_bones"))
            .clicked()
        {
            st.bind_to_bones(ctx);
        }
        if ui
            .add_enabled(dirty || !has_path, egui::Button::new(RichText::new(icons::FLOPPY_DISK).size(15.0)).frame(false))
            .on_hover_text(ctx.t("rig.save"))
            .clicked()
        {
            st.save(ctx);
        }
        ui.label(RichText::new(format!("{name}{}", if dirty { " •" } else { "" })).small());
        if ctx.has_project() && st.doc.is_some() && !st.in_project(ctx) {
            if ui
                .small_button(format!("{}  {}", icons::PLUS, ctx.t("rig.add_to_project")))
                .on_hover_text(ctx.t("rig.add_to_project.hint"))
                .clicked()
            {
                st.add_to_project(ctx);
            }
        }
        ui.separator();
        if ui
            .add_enabled(can_undo, egui::Button::new(RichText::new(icons::ARROW_COUNTER_CLOCKWISE).size(15.0)).frame(false))
            .on_hover_text(ctx.t("rig.undo"))
            .clicked()
            && let Some(d) = st.doc.as_mut()
        {
            d.undo();
        }
        if ui
            .add_enabled(can_redo, egui::Button::new(RichText::new(icons::ARROW_CLOCKWISE).size(15.0)).frame(false))
            .on_hover_text(ctx.t("rig.redo"))
            .clicked()
            && let Some(d) = st.doc.as_mut()
        {
            d.redo();
        }
        ui.separator();
        for (tool, icon, key) in [
            (Tool::Create, icons::PLUS_CIRCLE, "rig.tool.create"),
            (Tool::Move, icons::ARROWS_OUT_CARDINAL, "rig.tool.move"),
            (Tool::Rotate, icons::ARROWS_CLOCKWISE, "rig.tool.rotate"),
            (Tool::Scale, icons::ARROWS_OUT_SIMPLE, "rig.tool.scale"),
            (Tool::Mesh, icons::POLYGON, "rig.tool.mesh"),
            (Tool::Weights, icons::PAINT_BRUSH, "rig.tool.weights"),
            (Tool::Cut, icons::SCISSORS, "rig.tool.cut"),
            (Tool::Paint, icons::PAINT_BRUSH_BROAD, "rig.tool.paint"),
        ] {
            let mesh_ok = !matches!(tool, Tool::Mesh | Tool::Weights) || meshedit::has_mesh(st);
            let rest_only = matches!(tool, Tool::Create | Tool::Mesh | Tool::Cut | Tool::Paint);
            let enabled = !(st.pose_mode && rest_only) && mesh_ok;
            let on = st.tool == tool;
            let mut text = RichText::new(icon).size(15.0);
            if on {
                text = text.color(accent(ui));
            }
            if ui
                .add_enabled(enabled, egui::Button::new(text).frame(on))
                .on_hover_text(ctx.t(key))
                .clicked()
            {
                st.tool = tool;
            }
        }
        ui.separator();
        let mut pose = st.pose_mode;
        if ui.selectable_value(&mut pose, false, ctx.t("rig.mode.rest")).clicked()
            | ui.selectable_value(&mut pose, true, ctx.t("rig.mode.pose")).clicked()
        {
            set_pose_mode(st, pose);
        }
        if st.pose_mode && ui.button(ctx.t("rig.reset_pose")).clicked() {
            st.reset_pose();
        }
        ui.separator();
        ui.checkbox(&mut st.show_grid, ctx.t("rig.grid"));
        if icon_button(ui, icons::FRAME_CORNERS, &ctx.t("rig.frame")).clicked() {
            st.framed = false;
        }
        // Skin shown (outfit).
        let names: Vec<String> = st.doc.as_ref().map(|d| d.rig().skins.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
        if names.len() > 1 {
            egui::ComboBox::from_id_salt("rig_skin_view").selected_text(&st.skin).width(110.0).show_ui(ui, |ui| {
                for n in &names {
                    ui.selectable_value(&mut st.skin, n.clone(), n);
                }
            });
        }
    });
}

fn confirm_dialog(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    if st.pending.is_none() {
        return;
    }
    let mut choice = None;
    egui::Window::new(ctx.t("rig.discard_title"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(ctx.t("rig.discard_msg"));
            ui.horizontal(|ui| {
                if ui.button(ctx.t("rig.discard")).clicked() {
                    choice = Some(true);
                }
                if ui.button(ctx.t("rig.cancel")).clicked() {
                    choice = Some(false);
                }
            });
        });
    match (choice, st.pending.take()) {
        (Some(true), Some(Pending::New)) => st.new_rig(),
        (Some(true), Some(Pending::Open(p))) => st.open_path(&p, ctx),
        (Some(true), Some(Pending::NewFromImage(p))) => st.new_from_image(ctx, &p),
        (None, pending) => st.pending = pending,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rig_host::RigLink;

    fn state() -> State {
        let mut st = State::new(Rc::new(RefCell::new(RigLink::default())));
        st.new_rig();
        st
    }

    #[test]
    fn pose_mode_edits_never_touch_the_rest_pose_or_history() {
        let mut st = state();
        let doc = st.doc.as_mut().unwrap();
        let id = doc.add_bone(None, [0.0, 0.0], [50.0, 0.0]).unwrap();
        st.sync();
        let rest = st.doc.as_ref().unwrap().rig().clone();
        let depth_rev = st.doc.as_ref().unwrap().revision;
        set_pose_mode(&mut st, true);
        st.set_local(&id, Xform { rotation: 45.0, ..Xform::IDENTITY });
        assert_eq!(st.doc.as_ref().unwrap().rig(), &rest);
        assert_eq!(st.doc.as_ref().unwrap().revision, depth_rev);
        let w = st.world();
        assert!((w[0].x_angle() - 45.0).abs() < 1e-3);
        // Rest mode shows the rest pose again; the pose survives for later.
        set_pose_mode(&mut st, false);
        assert!(st.world()[0].x_angle().abs() < 1e-3);
        st.reset_pose();
        assert!(st.pose.is_empty());
    }

    #[test]
    fn rest_edit_is_one_undo_step_per_gesture() {
        let mut st = state();
        let id = st.doc.as_mut().unwrap().add_bone(None, [0.0, 0.0], [50.0, 0.0]).unwrap();
        st.sync();
        st.set_local(&id, Xform { x: 5.0, ..Xform::IDENTITY });
        st.set_local(&id, Xform { x: 9.0, ..Xform::IDENTITY });
        st.doc.as_mut().unwrap().end_gesture();
        st.doc.as_mut().unwrap().undo();
        assert_eq!(st.doc.as_ref().unwrap().rig().bones[0].rest.x, 0.0);
    }

    #[test]
    fn deleted_selection_is_dropped_and_prepared_follows_the_document() {
        let mut st = state();
        let id = st.doc.as_mut().unwrap().add_bone(None, [0.0, 0.0], [50.0, 0.0]).unwrap();
        st.sync();
        assert!(st.prepared.is_some());
        st.selected = Some(id.clone());
        st.doc.as_mut().unwrap().delete_bone(&id).unwrap();
        st.sync();
        assert!(st.selected.is_none());
        assert!(st.prepared.is_some(), "an empty rig is valid");
    }
}
