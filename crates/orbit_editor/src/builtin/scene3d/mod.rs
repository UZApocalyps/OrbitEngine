//! The 3D scene editor (`orbit.scene3d`, M6-08 / M6-09 / M6-10): opens and
//! creates `.scene3d.ron` documents and edits them with three panels.
//!
//! * **Viewport**: the scene rendered by `orbit_render3d` into an egui
//!   texture (on demand: only when the document, the camera or the size
//!   changed), a grid, an orbit / pan / zoom edit camera with view presets,
//!   click selection (CPU ray against bounds), translate / rotate / scale
//!   gizmos with snapping, the frustum of the game cameras, light gizmos,
//!   a picture-in-picture of what the game camera sees, the bones of a
//!   rigged character (pose mode: pick and rotate bones) and drops of
//!   models (from the hierarchy's model list or glTF files from the OS);
//! * **Hierarchy**: the node tree (add, rename, duplicate, delete, drag and
//!   drop to reparent, visibility) and the glTF import;
//! * **Inspector**: transform, light and camera properties, the
//!   environment, and the material editor with its sphere preview.
//!
//! The document logic is in [`model`] (undo / redo as a snapshot stack, like
//! the interface editor), the math in [`geom`] and [`gizmo`]: all headless
//! and unit tested. The GPU work is done by the host
//! ([`crate::scene3d_host`]), which this plugin only talks to through the
//! shared [`Viewport3dLink`].

mod geom;
mod gizmo;
pub(crate) mod model;
mod panels;
mod rig;
#[cfg(test)]
mod gpu_tests;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, PanelDef,
    Plugin, PluginManifest, icons,
};
use orbit_scene::{Node3dId, Node3dKind};

use self::geom::{Axis, EditCamera, ViewPreset};
use self::gizmo::{Drag, Mode, ScreenRect};
use self::model::{ModelLib, SceneDoc};
use super::ids;
use super::panels as panel_ids;
use super::ui_util::{accent, danger, faint, icon_button, weak};
use crate::scene3d_host::{self, ModelAssets, SharedLink, Viewport3dRequest};

pub struct Scene3dPlugin {
    state: Rc<RefCell<State>>,
}

impl Scene3dPlugin {
    pub fn new(link: SharedLink) -> Self {
        Self {
            state: Rc::new(RefCell::new(State::new(link))),
        }
    }
}

impl Plugin for Scene3dPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::SCENE3D, "3D scene editor", "0.1.0")
    }
}

impl EditorPlugin for Scene3dPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        self.state.borrow_mut().ui_link = Some(reg.ui_link.clone());
        // Saving the project saves the open 3D scene too; closing the project
        // drops the scene (its dirty flag must not leak into the next one).
        let saver = self.state.clone();
        reg.project.on_project_event(move |event| {
            let Ok(mut st) = saver.try_borrow_mut() else {
                return;
            };
            match event {
                orbit_plugin_api::ProjectEvent::BeforeSave => {
                    if let Some(doc) = st.doc.as_mut()
                        && doc.dirty()
                        && doc.path.is_some()
                    {
                        let _ = doc.save();
                    }
                }
                orbit_plugin_api::ProjectEvent::Closed => {
                    st.doc = None;
                    st.selected = None;
                }
                _ => {}
            }
            st.sync_dirty();
        });
        let state = self.state.clone();
        reg.panel(PanelDef::new(
            "viewport",
            "s3d.title.viewport",
            icons::CUBE,
            DockSlot::Hidden,
            move |ui, ctx| viewport_panel(ui, ctx, &state),
        ));
        let state = self.state.clone();
        reg.panel(PanelDef::new(
            "hierarchy",
            "s3d.title.hierarchy",
            icons::TREE_STRUCTURE,
            DockSlot::Hidden,
            move |ui, ctx| panels::hierarchy(ui, ctx, &state),
        ));
        let state = self.state.clone();
        reg.panel(PanelDef::new(
            "inspector",
            "s3d.title.inspector",
            icons::SLIDERS,
            DockSlot::Hidden,
            move |ui, ctx| panels::inspector(ui, ctx, &state),
        ));
        reg.action(
            ActionDef::new("open", "s3d.open", |ctx| {
                ctx.request(EditorRequest::ShowWorkspace(
                    [
                        panel_ids::SCENE3D_VIEWPORT,
                        panel_ids::SCENE3D_HIERARCHY,
                        panel_ids::SCENE3D_INSPECTOR,
                    ]
                    .map(str::to_owned)
                    .to_vec(),
                ));
            })
            .icon(icons::CUBE)
            .category("category.tools")
            .menu("tools", 160),
        );
    }
}

// ------------------------------------------------------------------ state

/// An action waiting for the answer to "discard unsaved changes?".
pub(super) enum Pending {
    New,
    Open(PathBuf),
}

/// What the viewport last asked the host to draw.
#[derive(PartialEq)]
struct Sent {
    revision: u64,
    assets: u64,
    cam: [u32; 7],
    size: [u32; 2],
    /// Time of the previewed clip (bits), 0 when none.
    anim: u32,
    /// Picture-in-picture: the scene camera drawn.
    view: Option<Node3dId>,
}

/// Aspect ratio of the game picture (frustums and the camera preview).
const GAME_ASPECT: f32 = 16.0 / 9.0;

/// A model dragged from the hierarchy's model list (prefab name).
#[derive(Clone, Debug)]
pub(super) struct ModelDrag(pub String);

pub(super) struct State {
    pub link: SharedLink,
    /// Host flags: the open scene's dirty state counts in the project's
    /// "modified".
    pub ui_link: Option<Rc<orbit_editor_api::UiEditorLink>>,
    pub doc: Option<SceneDoc>,
    pub lib: ModelLib,
    pub assets: Rc<ModelAssets>,
    pub assets_key: u64,
    pub cam: EditCamera,
    pub selected: Option<Node3dId>,
    pub mode: Mode,
    pub show_grid: bool,
    pub drag: Option<Drag>,
    pub hover_axis: Option<Axis>,
    pub pending: Option<Pending>,
    pub renaming: Option<(Node3dId, String)>,
    pub material: Option<String>,
    pub material_sent: Option<(u64, String, [u32; 2], u64)>,
    pub material_name_buf: Option<(String, String)>,
    pub error: Option<String>,
    /// Gizmo snapping on (Ctrl inverts it while dragging).
    pub snap: bool,
    /// Picture-in-picture of the game camera.
    pub show_pip: bool,
    /// Pose mode: the gizmo moves the bones of the selected rigged node.
    pub pose_mode: bool,
    /// Selected bone (joint name) in pose mode.
    pub bone: Option<String>,
    /// Clip being previewed (never written to the document).
    pub play: Option<rig::Play>,
    /// Name typed for "save pose".
    pub pose_name: String,
    sent: Option<Sent>,
    pip_sent: Option<Sent>,
    press: Option<(Pos2, bool)>,
    next_key: u64,
}

impl State {
    fn new(link: SharedLink) -> Self {
        Self {
            link,
            ui_link: None,
            doc: None,
            lib: ModelLib::default(),
            assets: Rc::new(ModelAssets::default()),
            assets_key: 0,
            cam: EditCamera::default(),
            selected: None,
            mode: Mode::Translate,
            show_grid: true,
            drag: None,
            hover_axis: None,
            pending: None,
            renaming: None,
            material: None,
            material_sent: None,
            material_name_buf: None,
            error: None,
            snap: false,
            show_pip: true,
            pose_mode: false,
            bone: None,
            play: None,
            pose_name: String::new(),
            sent: None,
            pip_sent: None,
            press: None,
            next_key: 1,
        }
    }

    /// Where the imported models of the open scene live.
    pub fn models_dir(&self) -> Option<PathBuf> {
        let dir = self.doc.as_ref()?.path.as_ref()?.parent()?.to_owned();
        Some(dir.join("models"))
    }

    /// Rebuilds the asset bundle the host uploads.
    pub fn refresh_assets(&mut self) {
        self.assets = Rc::new(ModelAssets {
            prefabs: self.lib.prefabs.values().cloned().collect(),
            meshes: self.lib.meshes.values().cloned().collect(),
            textures: self.lib.textures.clone(),
        });
        self.assets_key = self.next_key;
        self.next_key += 1;
    }

    pub fn set_doc(&mut self, doc: SceneDoc, lib: ModelLib) {
        self.doc = Some(doc);
        self.lib = lib;
        self.refresh_assets();
        self.selected = None;
        self.drag = None;
        self.renaming = None;
        self.material = None;
        self.sent = None;
        self.pip_sent = None;
        self.material_sent = None;
        self.error = None;
        self.pose_mode = false;
        self.bone = None;
        self.play = None;
        if let Some(b) = self.doc.as_ref().and_then(|d| d.scene_bounds(&self.lib)) {
            self.cam.focus(b.center(), b.radius().max(3.0));
        } else {
            self.cam = EditCamera::default();
        }
    }

    pub fn ensure_selection(&mut self) {
        if let (Some(doc), Some(id)) = (&self.doc, self.selected)
            && doc.node(id).is_none()
        {
            self.selected = None;
            self.drag = None;
        }
        if let (Some(doc), Some(play)) = (&self.doc, &self.play)
            && doc.node(play.node).is_none()
        {
            self.play = None;
        }
        if let (Some(doc), Some(name)) = (&self.doc, &self.material)
            && !doc.scene().materials.iter().any(|m| &m.name == name)
        {
            self.material = None;
        }
    }

    /// Ends merged edits once the pointer is up.
    pub fn begin_frame(&mut self, ui: &Ui) {
        if !ui.input(|i| i.pointer.any_down())
            && let Some(doc) = self.doc.as_mut()
        {
            doc.end_gesture();
        }
        self.ensure_selection();
        self.sync_dirty();
    }

    /// Handles the keyboard shortcuts shared by the panels.
    pub fn undo_redo_keys(&mut self, ui: &Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (undo, redo) = ui.input(|i| {
            let ctrl = i.modifiers.command;
            (
                ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                ctrl && (i.key_pressed(egui::Key::Y)
                    || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
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

    pub fn new_scene(&mut self) {
        self.set_doc(SceneDoc::new("Scene"), ModelLib::default());
    }

    pub fn open_path(&mut self, path: &std::path::Path, ctx: &mut EditorContext<'_>) {
        match SceneDoc::load(path) {
            Ok(doc) => {
                let dir = path.parent().map(|d| d.join("models"));
                let (lib, errors) = dir.map_or((ModelLib::default(), Vec::new()), |d| {
                    ModelLib::load_dir(&d)
                });
                for e in errors {
                    ctx.warn(ctx.tf("s3d.model_failed", &[("error", &e)]));
                }
                self.set_doc(doc, lib);
            }
            Err(e) => {
                ctx.error(ctx.tf("s3d.load_failed", &[("error", &e)]));
                self.error = Some(e);
            }
        }
    }

    pub fn save(&mut self, ctx: &mut EditorContext<'_>) {
        let Some(doc) = self.doc.as_mut() else { return };
        let target = match doc.path.clone() {
            Some(p) => Some(p),
            None => {
                let mut dlg = rfd::FileDialog::new()
                    .add_filter("3D scene", &["ron"])
                    .set_file_name("scene.scene3d.ron");
                if let Some(root) = ctx.project_info().map(|p| p.root.clone()) {
                    dlg = dlg.set_directory(root);
                }
                dlg.save_file()
            }
        };
        let Some(path) = target else { return };
        match doc.save_as(&path) {
            Ok(()) => ctx.info(ctx.tf("s3d.saved", &[("path", &path.display())])),
            Err(e) => ctx.error(ctx.tf("s3d.save_failed", &[("error", &e)])),
        }
    }

    pub fn request_open(&mut self, ctx: &mut EditorContext<'_>) {
        let mut dlg = rfd::FileDialog::new().add_filter("3D scene", &["ron"]);
        if let Some(root) = ctx.project_info().map(|p| p.root.clone()) {
            dlg = dlg.set_directory(root);
        }
        if let Some(path) = dlg.pick_file() {
            if self.doc.as_ref().is_some_and(SceneDoc::dirty) {
                self.pending = Some(Pending::Open(path));
            } else {
                self.open_path(&path, ctx);
            }
        }
    }

    pub fn request_new(&mut self) {
        if self.doc.as_ref().is_some_and(SceneDoc::dirty) {
            self.pending = Some(Pending::New);
        } else {
            self.new_scene();
        }
    }

    /// Publishes the open scene's dirty state to the host (the project's
    /// unsaved-changes counter).
    pub fn sync_dirty(&self) {
        if let Some(link) = &self.ui_link {
            link.scene3d_dirty
                .set(self.doc.as_ref().is_some_and(SceneDoc::dirty));
        }
    }

    /// "Add to project": saves the scene under the project's `assets/3d`
    /// (or where it already is, under `assets/`) and declares it in the
    /// asset library so stories can show it. Returns the manifest path.
    pub fn add_to_project(&mut self, ctx: &mut EditorContext<'_>) -> Option<String> {
        let Some(root) = ctx.project_info().map(|p| p.root.clone()) else {
            ctx.warn(ctx.t("s3d.no_project"));
            return None;
        };
        let doc = self.doc.as_mut()?;
        let target = {
            let story = ctx.story()?;
            crate::assets::plan_scene3d_target(
                &root,
                story,
                &doc.scene().name,
                doc.path.as_deref(),
            )
        };
        if let Err(e) = doc.save_as(&target.abs) {
            ctx.error(ctx.tf("s3d.save_failed", &[("error", &e)]));
            return None;
        }
        let rel = target.rel.clone();
        let declared = ctx.ops_or_toast(|p| {
            orbit_project::ops::add_asset(p, orbit_core::AssetKind::Scene3d, &rel)
        });
        self.sync_dirty();
        declared?;
        ctx.info(ctx.tf("s3d.added_to_project", &[("path", &target.rel)]));
        Some(target.rel)
    }

    /// Whether the open scene is a declared asset of the project.
    pub fn in_project(&self, ctx: &EditorContext<'_>) -> bool {
        let (Some(root), Some(story), Some(path)) = (
            ctx.project_info().map(|p| p.root.clone()),
            ctx.story(),
            self.doc.as_ref().and_then(|d| d.path.clone()),
        ) else {
            return false;
        };
        crate::assets::asset_rel_path(&root, &path)
            .is_some_and(|rel| story.assets.assets.iter().any(|a| a.path == rel))
    }

    /// Imports a PNG / JPEG as a texture of the scene (stored in `models/`
    /// next to the scene file, so it is found again when the scene is
    /// opened). Returns the texture name.
    pub fn import_texture(&mut self, ctx: &mut EditorContext<'_>) -> Option<String> {
        let Some(models) = self.models_dir() else {
            ctx.warn(ctx.t("s3d.import_save_first"));
            return None;
        };
        let path = rfd::FileDialog::new()
            .add_filter("Image", &["png", "jpg", "jpeg"])
            .pick_file()?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("texture")
            .to_owned();
        let result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                self.lib.add_texture_file(&name, &bytes)?;
                std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png");
                std::fs::write(models.join(format!("{name}.{ext}")), &bytes)
                    .map_err(|e| e.to_string())
            });
        match result {
            Ok(()) => {
                self.refresh_assets();
                Some(name)
            }
            Err(e) => {
                ctx.error(ctx.tf("s3d.import_failed", &[("name", &name), ("error", &e)]));
                None
            }
        }
    }

    /// glTF import: copies the file into `models/`, imports it and adds an
    /// instance to the scene.
    pub fn import_gltf(&mut self, ctx: &mut EditorContext<'_>) {
        if self.models_dir().is_none() {
            ctx.warn(ctx.t("s3d.import_save_first"));
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("glTF", &["glb", "gltf"])
            .pick_file()
        else {
            return;
        };
        self.import_model_file(ctx, &path, None);
    }

    /// Imports the glTF at `path` into `models/` and adds an instance of it
    /// (under the selection, or at the world position `at` for a drop).
    pub fn import_model_file(
        &mut self,
        ctx: &mut EditorContext<'_>,
        path: &std::path::Path,
        at: Option<[f32; 3]>,
    ) {
        let Some(models) = self.models_dir() else {
            ctx.warn(ctx.t("s3d.import_save_first"));
            return;
        };
        let path = path.to_owned();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model")
            .to_owned();
        let result = copy_model_into(&path, &name, &models);
        match result {
            Ok(imp) => {
                for w in &imp.warnings {
                    ctx.warn(ctx.tf("s3d.import_warning", &[("name", &name), ("warning", w)]));
                }
                self.lib.add_imported(imp);
                self.refresh_assets();
                if let Some(doc) = self.doc.as_mut() {
                    let id = match at {
                        Some(at) => doc.add_prefab_instance_at(&name, None, at),
                        None => doc.add_prefab_instance(&name, self.selected),
                    };
                    self.selected = Some(id);
                }
                ctx.info(ctx.tf("s3d.imported", &[("name", &name)]));
            }
            Err(e) => ctx.error(ctx.tf("s3d.import_failed", &[("name", &name), ("error", &e)])),
        }
    }
}

/// Imports the glTF at `path` (external buffers and images resolved from
/// its folder) and copies it into `models/` as `<name>.<ext>`. The external
/// files of a multi-file `.gltf` go to `models/<name>.files/...` (its URIs
/// are rewritten), so two Sketchfab exports that both use `scene.bin` do
/// not clash. Nothing is written when the import fails.
pub(crate) fn copy_model_into(
    path: &std::path::Path,
    name: &str,
    models: &std::path::Path,
) -> Result<orbit_scene::import::Imported, String> {
    let imp = orbit_scene::import::import_gltf_file(path)?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let src_dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let prefix = format!("{name}.files");
    let (bytes, moved) = orbit_scene::import::relocate_gltf(&bytes, &prefix)?;
    std::fs::create_dir_all(models).map_err(|e| e.to_string())?;
    if !moved.is_empty() {
        let _ = std::fs::remove_dir_all(models.join(&prefix));
    }
    for (from, to) in &moved {
        let dst = models.join(to);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(src_dir.join(from), &dst).map_err(|e| format!("{from}: {e}"))?;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("glb");
    std::fs::write(models.join(format!("{name}.{ext}")), &bytes).map_err(|e| e.to_string())?;
    Ok(imp)
}

// --------------------------------------------------------------- viewport

fn cam_bits(c: &EditCamera) -> [u32; 7] {
    [
        c.target[0].to_bits(),
        c.target[1].to_bits(),
        c.target[2].to_bits(),
        c.yaw.to_bits(),
        c.pitch.to_bits(),
        c.distance.to_bits(),
        c.fov_y.to_bits(),
    ]
}

fn viewport_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &Rc<RefCell<State>>) {
    let mut guard = state.borrow_mut();
    let st = &mut *guard;
    st.begin_frame(ui);
    if st.doc.is_none() {
        empty_state(ui, ctx, st);
        confirm_dialog(ui, ctx, st);
        return;
    }
    toolbar(ui, ctx, st);
    ui.separator();
    let avail = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(
        vec2(avail.x.max(32.0), avail.y.max(32.0)),
        Sense::click_and_drag(),
    );
    let ppp = ui.ctx().pixels_per_point();
    let size_px = [
        (rect.width() * ppp).round().max(16.0) as u32,
        (rect.height() * ppp).round().max(16.0) as u32,
    ];

    // ---- input
    let hovered = response.hovered() || st.drag.is_some();
    navigate(ui, st, &response, rect);
    if hovered || response.has_focus() {
        keys(ui, st);
    }
    gizmo_and_pick(ui, st, &response, rect);
    drops(ui, ctx, st, &response, rect);

    // ---- clip preview: advance and keep animating
    if let Some(play) = st.play.as_mut()
        && play.playing
    {
        play.t += ui.input(|i| i.stable_dt).min(0.1);
        ui.ctx().request_repaint();
    }
    let anim = st.play.as_ref().map_or(0, |p| p.t.to_bits() | 1);

    // ---- ask the host for a picture when something changed
    if let Some(doc) = &st.doc {
        let key = Sent {
            revision: doc.revision,
            assets: st.assets_key,
            cam: cam_bits(&st.cam),
            size: size_px,
            anim,
            view: None,
        };
        let texture_ready = st.link.borrow().slots[scene3d_host::MAIN].texture.is_some();
        if (st.sent.as_ref() != Some(&key) || !texture_ready)
            && let Some(scene) = rig::scene_for_render(st)
        {
            st.link.borrow_mut().slots[scene3d_host::MAIN].request = Some(Viewport3dRequest {
                scene,
                camera: st.cam.camera3d(),
                size: size_px,
                assets_key: st.assets_key,
                assets: st.assets.clone(),
            });
            st.sent = Some(key);
            // The host answers during this frame; a texture that is not
            // published yet needs one more frame to show up.
            if !texture_ready {
                ui.ctx().request_repaint();
            }
        }
    }
    // ---- picture-in-picture: what the game camera sees
    let pip = pip_rect(rect);
    let pip_cam = if st.show_pip { active_camera(st) } else { None };
    if let (Some(pip), Some((cam_id, camera)), Some(doc)) = (pip, pip_cam.as_ref().and_then(|&c| Some((c, scene_camera(st, c)?))), &st.doc) {
        let pip_px = [
            (pip.width() * ppp).round().max(16.0) as u32,
            (pip.height() * ppp).round().max(16.0) as u32,
        ];
        let key = Sent {
            revision: doc.revision,
            assets: st.assets_key,
            cam: [0; 7],
            size: pip_px,
            anim,
            view: Some(cam_id),
        };
        let ready = st.link.borrow().slots[scene3d_host::CAMERA].texture.is_some();
        if (st.pip_sent.as_ref() != Some(&key) || !ready)
            && let Some(scene) = rig::scene_for_render(st)
        {
            st.link.borrow_mut().slots[scene3d_host::CAMERA].request = Some(Viewport3dRequest {
                scene,
                camera,
                size: pip_px,
                assets_key: st.assets_key,
                assets: st.assets.clone(),
            });
            st.pip_sent = Some(key);
            if !ready {
                ui.ctx().request_repaint();
            }
        }
    }

    // ---- paint
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(24));
    let (texture, available, error) = {
        let l = st.link.borrow();
        (
            l.slots[scene3d_host::MAIN].texture,
            l.available,
            l.error.clone(),
        )
    };
    match texture {
        Some(tex) if available => {
            painter.image(
                tex,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        _ => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                ctx.t("s3d.no_gpu"),
                FontId::proportional(13.0),
                weak(ui),
            );
        }
    }
    if let Some(e) = error {
        painter.text(
            rect.left_top() + vec2(8.0, 8.0),
            Align2::LEFT_TOP,
            e,
            FontId::proportional(12.0),
            danger(ui),
        );
    }
    paint_overlays(ui, st, &painter, rect);
    if let (Some(pip), Some(cam_id)) = (pip, pip_cam) {
        paint_pip(ui, ctx, st, &painter, pip, cam_id);
    }
    painter.text(
        rect.left_bottom() + vec2(8.0, -6.0),
        Align2::LEFT_BOTTOM,
        ctx.t(if st.pose_mode && selected_rig(st).is_some() {
            "s3d.pose_hint"
        } else {
            "s3d.nav_hint"
        }),
        FontId::proportional(11.0),
        faint(ui),
    );
    confirm_dialog(ui, ctx, st);
}

/// Where the camera preview goes: bottom right, a quarter of the width.
fn pip_rect(rect: Rect) -> Option<Rect> {
    let w = (rect.width() * 0.28).min(360.0);
    if w < 96.0 || rect.height() < 160.0 {
        return None;
    }
    let size = vec2(w, w / GAME_ASPECT);
    let max = rect.right_bottom() - vec2(10.0, 24.0);
    Some(Rect::from_min_max(max - size, max))
}

/// The scene camera the preview, the frustum highlight and "align camera to
/// view" use: the selected camera, else the first one.
fn active_camera(st: &State) -> Option<Node3dId> {
    let doc = st.doc.as_ref()?;
    let is_cam = |id: Node3dId| doc.node(id).is_some_and(|n| matches!(n.kind, Node3dKind::Camera(_)));
    st.selected
        .filter(|&id| is_cam(id))
        .or_else(|| doc.scene().cameras().map(|(n, _)| n.id).next())
}

/// The game view of camera node `id`.
fn scene_camera(st: &State, id: Node3dId) -> Option<orbit_render3d::Camera3d> {
    let doc = st.doc.as_ref()?;
    let Node3dKind::Camera(def) = &doc.node(id)?.kind else {
        return None;
    };
    Some(orbit_render3d::Camera3d {
        world: doc.world_matrix(id)?,
        projection: def.projection.clone(),
        near: def.near,
        far: def.far,
    })
}

fn paint_pip(
    ui: &Ui,
    ctx: &EditorContext<'_>,
    st: &State,
    painter: &egui::Painter,
    pip: Rect,
    cam_id: Node3dId,
) {
    let texture = st.link.borrow().slots[scene3d_host::CAMERA].texture;
    painter.rect_filled(pip.expand(2.0), 3.0, Color32::from_black_alpha(200));
    if let Some(tex) = texture {
        painter.image(
            tex,
            pip,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    painter.rect_stroke(pip, 2.0, Stroke::new(1.0, accent(ui)), egui::StrokeKind::Outside);
    let name = st
        .doc
        .as_ref()
        .and_then(|d| d.node(cam_id))
        .map(|n| match &n.kind {
            Node3dKind::Camera(c) => c.name.clone(),
            _ => n.name.clone(),
        })
        .unwrap_or_default();
    painter.text(
        pip.left_top() + vec2(4.0, -3.0),
        Align2::LEFT_BOTTOM,
        format!("{}  {}", icons::VIDEO_CAMERA, ctx.tf("s3d.pip_label", &[("name", &name)])),
        FontId::proportional(11.0),
        Color32::from_white_alpha(220),
    );
}

/// Drops on the viewport: a model from the hierarchy's list, or glTF files
/// from the OS; placed where the pointer meets the ground.
fn drops(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State, response: &egui::Response, rect: Rect) {
    let ground_at = |st: &State, p: Option<Pos2>| -> [f32; 3] {
        let vp = view_proj(st, rect);
        p.and_then(|p| geom::screen_ray(&vp, screen_rect(rect).to_ndc([p.x, p.y])))
            .and_then(|ray| geom::ray_plane(&ray, [0.0; 3], geom::UP))
            .filter(|h| geom::len(math_sub(*h, st.cam.target)) < st.cam.distance * 20.0)
            .unwrap_or([st.cam.target[0], 0.0, st.cam.target[2]])
    };
    if response.dnd_hover_payload::<ModelDrag>().is_some() {
        ui.painter()
            .rect_stroke(rect.shrink(2.0), 2.0, Stroke::new(2.0, accent(ui)), egui::StrokeKind::Inside);
    }
    if let Some(drag) = response.dnd_release_payload::<ModelDrag>() {
        let at = ground_at(st, ui.input(|i| i.pointer.hover_pos()));
        if let Some(doc) = st.doc.as_mut() {
            st.selected = Some(doc.add_prefab_instance_at(&drag.0, None, at));
        }
    }
    // Files dropped from the OS while the pointer is over the viewport.
    let files: Vec<std::path::PathBuf> = ui.input(|i| {
        let over = i.pointer.hover_pos().is_some_and(|p| rect.contains(p));
        if !over {
            return Vec::new();
        }
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("glb") || e.eq_ignore_ascii_case("gltf"))
            })
            .collect()
    });
    for path in files {
        let at = ground_at(st, ui.input(|i| i.pointer.hover_pos()));
        st.import_model_file(ctx, &path, Some(at));
    }
}

fn math_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The selected node when it poses a skinned mesh: (node, mesh target).
fn selected_rig(st: &State) -> Option<(Node3dId, rig::RigTarget)> {
    let id = st.selected?;
    let t = rig::target(st.doc.as_ref()?, &st.lib, id)?;
    Some((id, t))
}

/// Moves the active scene camera onto the edit camera ("align camera to
/// view").
fn align_camera_to_view(st: &mut State) -> bool {
    let Some(id) = active_camera(st) else { return false };
    let w = st.cam.world();
    let Some(doc) = st.doc.as_mut() else { return false };
    doc.align_to(id, &w)
}

fn snap_selection_to_ground(st: &mut State) {
    if let (Some(doc), Some(id)) = (st.doc.as_mut(), st.selected) {
        doc.snap_to_ground(id, &st.lib);
    }
}

fn empty_state(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(ctx.t("s3d.no_scene")).color(weak(ui)));
        ui.add_space(8.0);
        if ui
            .button(format!("{}  {}", icons::FILE_PLUS, ctx.t("s3d.new")))
            .clicked()
        {
            st.request_new();
        }
        if ui
            .button(format!("{}  {}", icons::FOLDER_OPEN, ctx.t("s3d.open_file")))
            .clicked()
        {
            st.request_open(ctx);
        }
        if let Some(e) = &st.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).small().color(danger(ui)));
        }
    });
}

fn toolbar(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let (dirty, can_undo, can_redo, name) = st.doc.as_ref().map_or(
        (false, false, false, String::new()),
        |d| {
            (
                d.dirty(),
                d.can_undo(),
                d.can_redo(),
                d.path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| ctx.t("s3d.untitled")),
            )
        },
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if icon_button(ui, icons::FILE_PLUS, &ctx.t("s3d.new")).clicked() {
            st.request_new();
        }
        if icon_button(ui, icons::FOLDER_OPEN, &ctx.t("s3d.open_file")).clicked() {
            st.request_open(ctx);
        }
        if ui
            .add_enabled(
                dirty || st.doc.as_ref().is_some_and(|d| d.path.is_none()),
                egui::Button::new(RichText::new(icons::FLOPPY_DISK).size(15.0)).frame(false),
            )
            .on_hover_text(ctx.t("s3d.save"))
            .clicked()
        {
            st.save(ctx);
        }
        let in_project = st.in_project(ctx);
        if ui
            .add_enabled(
                ctx.has_project(),
                egui::Button::new(RichText::new(icons::FOLDER_PLUS).size(15.0)).frame(false),
            )
            .on_hover_text(ctx.t(if in_project {
                "s3d.add_to_project_done"
            } else {
                "s3d.add_to_project"
            }))
            .clicked()
        {
            st.add_to_project(ctx);
        }
        ui.label(RichText::new(format!("{name}{}", if dirty { " •" } else { "" })).small());
        ui.separator();
        if ui
            .add_enabled(
                can_undo,
                egui::Button::new(RichText::new(icons::ARROW_COUNTER_CLOCKWISE).size(15.0))
                    .frame(false),
            )
            .on_hover_text(ctx.t("s3d.undo"))
            .clicked()
            && let Some(d) = st.doc.as_mut()
        {
            d.undo();
        }
        if ui
            .add_enabled(
                can_redo,
                egui::Button::new(RichText::new(icons::ARROW_CLOCKWISE).size(15.0)).frame(false),
            )
            .on_hover_text(ctx.t("s3d.redo"))
            .clicked()
            && let Some(d) = st.doc.as_mut()
        {
            d.redo();
        }
        ui.separator();
        for (mode, icon, key) in [
            (Mode::Translate, icons::ARROWS_OUT_CARDINAL, "s3d.translate"),
            (Mode::Rotate, icons::ARROWS_CLOCKWISE, "s3d.rotate"),
            (Mode::Scale, icons::ARROWS_OUT, "s3d.scale"),
        ] {
            if ui
                .selectable_label(st.mode == mode, RichText::new(icon).size(15.0))
                .on_hover_text(ctx.t(key))
                .clicked()
            {
                st.mode = mode;
            }
        }
        ui.toggle_value(&mut st.snap, RichText::new(icons::MAGNET).size(15.0))
            .on_hover_text(ctx.t("s3d.snap"));
        ui.separator();
        ui.toggle_value(&mut st.show_grid, RichText::new(icons::GRID_FOUR).size(15.0))
            .on_hover_text(ctx.t("s3d.grid"));
        if icon_button(ui, icons::CROSSHAIR, &ctx.t("s3d.frame")).clicked() {
            frame_selection(st);
        }
        ui.menu_button(RichText::new(icons::CUBE_FOCUS).size(15.0), |ui| {
            for (view, key) in VIEWS {
                if ui.button(ctx.t(key)).clicked() {
                    st.cam.set_view(view);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(ctx.t("s3d.views"));
        ui.separator();
        ui.toggle_value(&mut st.show_pip, RichText::new(icons::PICTURE_IN_PICTURE).size(15.0))
            .on_hover_text(ctx.t("s3d.pip"));
        let has_cam = active_camera(st).is_some();
        if ui
            .add_enabled(
                has_cam,
                egui::Button::new(RichText::new(icons::VIDEO_CAMERA).size(15.0)).frame(false),
            )
            .on_hover_text(ctx.t("s3d.align_camera"))
            .clicked()
        {
            align_camera_to_view(st);
        }
        if ui
            .add_enabled(
                st.selected.is_some(),
                egui::Button::new(RichText::new(icons::ARROW_LINE_DOWN).size(15.0)).frame(false),
            )
            .on_hover_text(ctx.t("s3d.snap_ground"))
            .clicked()
        {
            snap_selection_to_ground(st);
        }
        if selected_rig(st).is_some() {
            ui.separator();
            ui.toggle_value(&mut st.pose_mode, RichText::new(icons::BONE).size(15.0))
                .on_hover_text(ctx.t("s3d.pose_mode_hint"));
        }
    });
}

const VIEWS: [(ViewPreset, &str); 6] = [
    (ViewPreset::Front, "s3d.view.front"),
    (ViewPreset::Back, "s3d.view.back"),
    (ViewPreset::Right, "s3d.view.right"),
    (ViewPreset::Left, "s3d.view.left"),
    (ViewPreset::Top, "s3d.view.top"),
    (ViewPreset::Bottom, "s3d.view.bottom"),
];

fn marker_half(st: &State) -> f32 {
    st.cam.distance * 0.035
}

fn frame_selection(st: &mut State) {
    let Some(doc) = &st.doc else { return };
    let b = st
        .selected
        .and_then(|id| doc.world_bounds(id, &st.lib, 0.5))
        .or_else(|| doc.scene_bounds(&st.lib));
    if let Some(b) = b {
        st.cam.focus(b.center(), b.radius().max(0.5));
    }
}

/// Orbit (right drag / Alt + left drag), pan (middle drag), zoom (wheel).
fn navigate(ui: &Ui, st: &mut State, response: &egui::Response, rect: Rect) {
    use egui::PointerButton::{Middle, Primary, Secondary};
    let delta = response.drag_delta();
    let alt = ui.input(|i| i.modifiers.alt);
    if response.dragged_by(Secondary) || (alt && response.dragged_by(Primary) && st.drag.is_none())
    {
        st.cam.orbit(delta.x, delta.y);
    } else if response.dragged_by(Middle) {
        st.cam.pan(delta.x, delta.y, rect.height());
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            st.cam.zoom(scroll);
        }
    }
}

fn keys(ui: &Ui, st: &mut State) {
    let typing = ui.ctx().egui_wants_keyboard_input();
    if typing {
        return;
    }
    st.undo_redo_keys(ui);
    let (w, e, r, f, del, dup) = ui.input(|i| {
        let plain = !i.modifiers.command && !i.modifiers.alt;
        (
            plain && i.key_pressed(egui::Key::W),
            plain && i.key_pressed(egui::Key::E),
            plain && i.key_pressed(egui::Key::R),
            plain && i.key_pressed(egui::Key::F),
            i.key_pressed(egui::Key::Delete),
            i.modifiers.command && i.key_pressed(egui::Key::D),
        )
    });
    // View presets (1 front, 3 right, 7 top; Ctrl: the opposite side),
    // G: snap to ground, P: pose mode, Esc: leave pose mode.
    let (view, ground, pose, escape) = ui.input(|i| {
        let ctrl = i.modifiers.command;
        let view = if i.key_pressed(egui::Key::Num1) {
            Some(if ctrl { ViewPreset::Back } else { ViewPreset::Front })
        } else if i.key_pressed(egui::Key::Num3) {
            Some(if ctrl { ViewPreset::Left } else { ViewPreset::Right })
        } else if i.key_pressed(egui::Key::Num7) {
            Some(if ctrl { ViewPreset::Bottom } else { ViewPreset::Top })
        } else {
            None
        };
        let plain = !i.modifiers.command && !i.modifiers.alt;
        (
            view,
            plain && i.key_pressed(egui::Key::G),
            plain && i.key_pressed(egui::Key::P),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if let Some(v) = view {
        st.cam.set_view(v);
    }
    if ground {
        snap_selection_to_ground(st);
    }
    if pose && selected_rig(st).is_some() {
        st.pose_mode = !st.pose_mode;
    }
    if escape && st.pose_mode {
        st.pose_mode = false;
    }
    if st.pose_mode
        && selected_rig(st).is_some()
        && (del || dup)
    {
        // In pose mode Delete resets the selected bone; nothing is deleted.
        if del
            && let (Some(id), Some(bone), Some(doc)) = (st.selected, st.bone.clone(), st.doc.as_mut())
        {
            doc.edit(None, |s| s.rig_mut(id).pose.reset(&bone));
        }
        return;
    }
    if w {
        st.mode = Mode::Translate;
    }
    if e {
        st.mode = Mode::Rotate;
    }
    if r {
        st.mode = Mode::Scale;
    }
    if f {
        frame_selection(st);
    }
    if let (Some(doc), Some(id)) = (st.doc.as_mut(), st.selected) {
        if del {
            doc.delete_node(id);
            st.selected = None;
        } else if dup {
            st.selected = doc.duplicate_node(id);
        }
    }
}

fn view_proj(st: &State, rect: Rect) -> orbit_render3d::math::Mat4 {
    st.cam
        .camera3d()
        .view_proj(rect.width().max(1.0) / rect.height().max(1.0))
}

fn screen_rect(rect: Rect) -> ScreenRect {
    ScreenRect {
        min: [rect.min.x, rect.min.y],
        size: [rect.width(), rect.height()],
    }
}

/// Bone matrices of the selected rigged node, for pose mode.
struct PoseView {
    node: Node3dId,
    skel: orbit_scene::skin::Skeleton,
    pose: orbit_scene::Pose,
    mesh_world: orbit_render3d::math::Mat4,
    worlds: Vec<orbit_render3d::math::Mat4>,
}

fn pose_view(st: &State) -> Option<PoseView> {
    let (node, t) = selected_rig(st)?;
    let mesh = st.lib.meshes.get(&t.mesh)?;
    let skel = mesh.skin.clone()?;
    let pose = rig::shown_pose(st, node, mesh);
    let worlds = rig::bone_worlds(&skel, &pose, &t.mesh_world);
    Some(PoseView {
        node,
        skel,
        pose,
        mesh_world: t.mesh_world,
        worlds,
    })
}

/// The gizmo mode used on a bone: rotate, or translate for a root bone.
fn bone_mode(st: &State, pv: &PoseView, i: usize) -> Mode {
    if st.mode == Mode::Translate && pv.skel.joints[i].parent.is_none() {
        Mode::Translate
    } else {
        Mode::Rotate
    }
}

/// Pose mode: bone gizmo, bone picking (falls back to node picking).
fn pose_gizmo_and_pick(ui: &Ui, st: &mut State, pv: PoseView, response: &egui::Response, rect: Rect) {
    let screen = screen_rect(rect);
    let vp = view_proj(st, rect);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let (pressed, down, released) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
        )
    });
    let snap = st.snap != ui.input(|i| i.modifiers.ctrl);
    let alt = ui.input(|i| i.modifiers.alt);
    let bone = st.bone.as_deref().and_then(|b| pv.skel.index_of(b));
    let handles = bone.map(|i| {
        let mode = bone_mode(st, &pv, i);
        let w = pv.worlds[i];
        let origin = [w[3][0], w[3][1], w[3][2]];
        let size = gizmo::world_size(&st.cam, rect.height()) * 0.7;
        (i, mode, gizmo::handles(mode, origin, &gizmo::axes(mode, &w), size, &vp, &screen))
    });
    st.hover_axis = match (&handles, pointer) {
        (Some((_, _, h)), Some(p)) if response.hovered() && st.drag.is_none() => {
            gizmo::hit(h, [p.x, p.y], 8.0)
        }
        _ => None,
    };
    if pressed
        && response.hovered()
        && !alt
        && let Some(p) = pointer
    {
        let mut started = false;
        if let Some((i, mode, h)) = &handles
            && let Some(axis) = gizmo::hit(h, [p.x, p.y], 8.0)
            && let Some(ray) = geom::screen_ray(&vp, screen.to_ndc([p.x, p.y]))
        {
            st.drag = Drag::begin(
                *mode,
                axis,
                rig::local_of(&pv.skel, &pv.pose, *i),
                &pv.worlds[*i],
                rig::bone_parent_world(&pv.skel, &pv.worlds, &pv.mesh_world, *i),
                &ray,
            );
            started = st.drag.is_some();
            if started {
                // Editing a bone leaves the clip preview.
                st.play = None;
            }
        }
        st.press = Some((p, started));
    }
    if down
        && let (Some(drag), Some(i), Some(p)) = (st.drag.clone(), bone, pointer)
        && let Some(ray) = geom::screen_ray(&vp, screen.to_ndc([p.x, p.y]))
        && let Some(t) = drag.apply(&ray, snap)
        && let Some(doc) = st.doc.as_mut()
    {
        let name = pv.skel.joints[i].name.clone();
        doc.edit(Some("bone-gizmo"), |s| s.rig_mut(pv.node).pose.set(&name, t));
    }
    if released {
        if let Some((start, on_gizmo)) = st.press.take()
            && !on_gizmo
            && let Some(p) = pointer
            && p.distance(start) < 4.0
            && response.hovered()
        {
            let points: Vec<Option<[f32; 2]>> = pv
                .worlds
                .iter()
                .map(|w| {
                    geom::project(&vp, [w[3][0], w[3][1], w[3][2]])
                        .map(|n| screen.to_screen([n[0], n[1]]))
                })
                .collect();
            match rig::pick_point(&points, [p.x, p.y], 9.0) {
                Some(i) => st.bone = Some(pv.skel.joints[i].name.clone()),
                None => {
                    if let Some(ray) = geom::screen_ray(&vp, screen.to_ndc([p.x, p.y]))
                        && let Some(doc) = &st.doc
                    {
                        let picked = doc.pick(&ray, &st.lib, marker_half(st));
                        if picked != Some(pv.node) {
                            st.selected = picked;
                            st.bone = None;
                        }
                    }
                }
            }
        }
        st.drag = None;
        if let Some(doc) = st.doc.as_mut() {
            doc.end_gesture();
        }
    }
}

/// Gizmo press / drag / release and click selection.
fn gizmo_and_pick(ui: &Ui, st: &mut State, response: &egui::Response, rect: Rect) {
    if st.pose_mode
        && let Some(pv) = pose_view(st)
    {
        pose_gizmo_and_pick(ui, st, pv, response, rect);
        return;
    }
    let screen = screen_rect(rect);
    let vp = view_proj(st, rect);
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let (pressed, down, released) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
        )
    });
    let snap = st.snap != ui.input(|i| i.modifiers.ctrl);
    let alt = ui.input(|i| i.modifiers.alt);

    // Handles of the selected node.
    let handles = st.selected.and_then(|id| {
        let doc = st.doc.as_ref()?;
        let world = doc.world_matrix(id)?;
        let origin = [world[3][0], world[3][1], world[3][2]];
        let dirs = gizmo::axes(st.mode, &world);
        let size = gizmo::world_size(&st.cam, rect.height());
        Some((
            world,
            gizmo::handles(st.mode, origin, &dirs, size, &vp, &screen),
        ))
    });
    st.hover_axis = match (&handles, pointer) {
        (Some((_, h)), Some(p)) if response.hovered() && st.drag.is_none() => {
            gizmo::hit(h, [p.x, p.y], 8.0)
        }
        _ => None,
    };

    if pressed
        && response.hovered()
        && !alt
        && let Some(p) = pointer
    {
        let mut started = false;
        if let (Some((world, h)), Some(id)) = (&handles, st.selected)
            && let Some(axis) = gizmo::hit(h, [p.x, p.y], 8.0)
            && let Some(doc) = &st.doc
            && let Some(node) = doc.node(id)
            && let Some(ray) = geom::screen_ray(&vp, screen.to_ndc([p.x, p.y]))
        {
            st.drag = Drag::begin(
                st.mode,
                axis,
                node.transform,
                world,
                doc.parent_world(id),
                &ray,
            );
            started = st.drag.is_some();
        }
        st.press = Some((p, started));
    }

    if down
        && let (Some(drag), Some(id), Some(p)) = (st.drag.clone(), st.selected, pointer)
        && let Some(ray) = geom::screen_ray(&vp, screen.to_ndc([p.x, p.y]))
        && let Some(t) = drag.apply(&ray, snap)
        && let Some(doc) = st.doc.as_mut()
    {
        doc.set_transform(id, "gizmo", t);
    }

    if released {
        if let Some((start, on_gizmo)) = st.press.take()
            && !on_gizmo
            && let Some(p) = pointer
            && p.distance(start) < 4.0
            && response.hovered()
            && let Some(ray) = geom::screen_ray(&vp, screen.to_ndc([p.x, p.y]))
            && let Some(doc) = &st.doc
        {
            st.selected = doc.pick(&ray, &st.lib, marker_half(st));
        }
        st.drag = None;
        if let Some(doc) = st.doc.as_mut() {
            doc.end_gesture();
        }
    }
}

fn axis_color(axis: Axis, active: bool) -> Color32 {
    if active {
        return Color32::from_rgb(255, 214, 64);
    }
    match axis {
        Axis::X => Color32::from_rgb(230, 70, 70),
        Axis::Y => Color32::from_rgb(90, 200, 90),
        Axis::Z => Color32::from_rgb(70, 130, 240),
    }
}

fn paint_overlays(ui: &Ui, st: &State, painter: &egui::Painter, rect: Rect) {
    let Some(doc) = &st.doc else { return };
    let screen = screen_rect(rect);
    let vp = view_proj(st, rect);
    let to_pos = |n: [f32; 2]| {
        let s = screen.to_screen(n);
        Pos2::new(s[0], s[1])
    };

    // Grid on the y = 0 plane, spacing adapted to the camera distance.
    if st.show_grid {
        let step = 10f32.powf((st.cam.distance / 4.0).max(0.05).log10().floor());
        let half = 20.0 * step;
        let cx = (st.cam.target[0] / step).round() * step;
        let cz = (st.cam.target[2] / step).round() * step;
        for i in -20i32..=20 {
            let o = i as f32 * step;
            let major = (i % 10 == 0) || i == 0;
            let color = if major {
                Color32::from_white_alpha(46)
            } else {
                Color32::from_white_alpha(18)
            };
            let lines = [
                ([cx + o, 0.0, cz - half], [cx + o, 0.0, cz + half], i == 0 && cx + o == 0.0),
                ([cx - half, 0.0, cz + o], [cx + half, 0.0, cz + o], i == 0 && cz + o == 0.0),
            ];
            for (n, (a, b, _)) in lines.into_iter().enumerate() {
                if let Some((p, q)) = geom::project_segment(&vp, a, b) {
                    // The world axes through the origin are tinted.
                    let c = if n == 0 && (cx + o).abs() < 1e-4 {
                        axis_color(Axis::Z, false).gamma_multiply(0.6)
                    } else if n == 1 && (cz + o).abs() < 1e-4 {
                        axis_color(Axis::X, false).gamma_multiply(0.6)
                    } else {
                        color
                    };
                    painter.line_segment([to_pos(p), to_pos(q)], Stroke::new(1.0, c));
                }
            }
        }
    }

    // Game camera frustums (the active one brighter) and light directions.
    let active = active_camera(st);
    let line = |a: [f32; 3], b: [f32; 3], stroke: Stroke| {
        if let Some((p, q)) = geom::project_segment(&vp, a, b) {
            painter.line_segment([to_pos(p), to_pos(q)], stroke);
        }
    };
    for n in &doc.scene().nodes {
        if !doc.effectively_visible(n.id) {
            continue;
        }
        let Some(w) = doc.world_matrix(n.id) else { continue };
        let origin = [w[3][0], w[3][1], w[3][2]];
        let selected = st.selected == Some(n.id);
        match &n.kind {
            Node3dKind::Camera(def) => {
                let strong = active == Some(n.id);
                let color = if strong {
                    Color32::from_rgb(255, 196, 92)
                } else {
                    Color32::from_white_alpha(70)
                };
                let stroke = Stroke::new(if selected { 1.6 } else { 1.0 }, color);
                let depth = (st.cam.distance * 0.35).clamp(0.5, 8.0);
                let c = geom::frustum_corners(def, &w, GAME_ASPECT, depth);
                for k in 0..4 {
                    line(c[k], c[(k + 1) % 4], stroke);
                    line(c[4 + k], c[4 + (k + 1) % 4], stroke);
                    line(origin, c[4 + k], stroke);
                }
                // "Up" marker above the far rectangle.
                let top = geom::scale(geom::add(c[6], c[7]), 0.5);
                let up = orbit_render3d::math::transform_vector(&w, [0.0, 1.0, 0.0]);
                let tip = geom::add(top, geom::scale(math_norm(up), geom::len(math_sub(c[6], c[7])) * 0.15));
                line(c[6], tip, stroke);
                line(tip, c[7], stroke);
            }
            Node3dKind::Light(light) => {
                let color = Color32::from_rgba_unmultiplied(255, 220, 120, if selected { 220 } else { 90 });
                let stroke = Stroke::new(1.0, color);
                let dir = math_norm(orbit_render3d::math::transform_vector(&w, [0.0, 0.0, -1.0]));
                let reach = marker_half(st) * 14.0;
                match light {
                    orbit_scene::Light::Directional { .. } => {
                        // Parallel rays.
                        let (r, u) = gizmo::plane_basis(dir);
                        for off in [[0.0, 0.0], [1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]] {
                            let o = geom::add(
                                origin,
                                geom::add(geom::scale(r, off[0] * reach * 0.15), geom::scale(u, off[1] * reach * 0.15)),
                            );
                            line(o, geom::add(o, geom::scale(dir, reach)), stroke);
                        }
                    }
                    orbit_scene::Light::Spot { range, outer_angle, .. } => {
                        let len = range.unwrap_or(reach).min(reach * 4.0);
                        let (r, u) = gizmo::plane_basis(dir);
                        let radius = len * outer_angle.tan();
                        let center = geom::add(origin, geom::scale(dir, len));
                        let ring: Vec<[f32; 3]> = (0..=24)
                            .map(|k| {
                                let a = k as f32 / 24.0 * std::f32::consts::TAU;
                                geom::add(center, geom::add(geom::scale(r, a.cos() * radius), geom::scale(u, a.sin() * radius)))
                            })
                            .collect();
                        for k in 0..24 {
                            line(ring[k], ring[k + 1], stroke);
                        }
                        for k in [0, 6, 12, 18] {
                            line(origin, ring[k], stroke);
                        }
                    }
                    orbit_scene::Light::Point { range, .. } => {
                        if selected && let Some(range) = range {
                            for (a, b) in [
                                ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                                ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
                                ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
                            ] {
                                let ring: Vec<[f32; 3]> = (0..=32)
                                    .map(|k| {
                                        let t = k as f32 / 32.0 * std::f32::consts::TAU;
                                        geom::add(origin, geom::add(geom::scale(a, t.cos() * range), geom::scale(b, t.sin() * range)))
                                    })
                                    .collect();
                                for k in 0..32 {
                                    line(ring[k], ring[k + 1], stroke);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Markers (lights, cameras, empties).
    let half = marker_half(st);
    for n in &doc.scene().nodes {
        if !doc.effectively_visible(n.id) {
            continue;
        }
        let glyph = match &n.kind {
            Node3dKind::Light(_) => icons::LIGHTBULB,
            Node3dKind::Camera(_) => icons::VIDEO_CAMERA,
            Node3dKind::Empty => icons::CROSSHAIR,
            _ => continue,
        };
        if let Some(w) = doc.world_matrix(n.id)
            && let Some(p) = geom::project(&vp, [w[3][0], w[3][1], w[3][2]])
        {
            let selected = st.selected == Some(n.id);
            painter.text(
                to_pos([p[0], p[1]]),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(18.0),
                if selected {
                    accent(ui)
                } else {
                    Color32::from_white_alpha(200)
                },
            );
        }
    }

    // Bones of the selected rigged node (pose mode: pickable, with a gizmo).
    if let Some(pv) = pose_view(st) {
        let pos_of = |w: &orbit_render3d::math::Mat4| {
            geom::project(&vp, [w[3][0], w[3][1], w[3][2]]).map(|n| to_pos([n[0], n[1]]))
        };
        let alpha = if st.pose_mode { 230 } else { 90 };
        let bone_color = Color32::from_rgba_unmultiplied(120, 220, 255, alpha);
        let sel = st.bone.as_deref().and_then(|b| pv.skel.index_of(b));
        for (i, j) in pv.skel.joints.iter().enumerate() {
            if let Some(p) = j.parent
                && let Some((a, b)) = geom::project_segment(
                    &vp,
                    [pv.worlds[p][3][0], pv.worlds[p][3][1], pv.worlds[p][3][2]],
                    [pv.worlds[i][3][0], pv.worlds[i][3][1], pv.worlds[i][3][2]],
                )
            {
                painter.line_segment([to_pos(a), to_pos(b)], Stroke::new(2.0, bone_color));
            }
        }
        if st.pose_mode {
            for (i, w) in pv.worlds.iter().enumerate() {
                if let Some(p) = pos_of(w) {
                    let selected = sel == Some(i);
                    painter.circle(
                        p,
                        if selected { 5.5 } else { 3.5 },
                        if selected { accent(ui) } else { bone_color },
                        Stroke::new(1.0, Color32::from_black_alpha(160)),
                    );
                }
            }
            if let Some(i) = sel {
                let mode = bone_mode(st, &pv, i);
                let w = pv.worlds[i];
                let origin = [w[3][0], w[3][1], w[3][2]];
                let size = gizmo::world_size(&st.cam, rect.height()) * 0.7;
                let handles = gizmo::handles(mode, origin, &gizmo::axes(mode, &w), size, &vp, &screen);
                paint_handles(painter, st, mode, &handles);
            }
            return;
        }
    }

    // Selection box.
    if let Some(id) = st.selected
        && let Some(b) = doc.world_bounds(id, &st.lib, half)
    {
        // The box of the local bounds under the node transform.
        let corners = match (doc.node(id), doc.world_matrix(id)) {
            (Some(n), Some(w)) => match st.lib.kind_bounds(&n.kind) {
                Some(local) => local
                    .corners()
                    .map(|c| orbit_render3d::math::transform_point(&w, c)),
                None => b.corners(),
            },
            _ => b.corners(),
        };
        let edges = [
            (0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5),
            (2, 6), (3, 7),
        ];
        for (a, c) in edges {
            if let Some((p, q)) = geom::project_segment(&vp, corners[a], corners[c]) {
                painter.line_segment(
                    [to_pos(p), to_pos(q)],
                    Stroke::new(1.5, accent(ui)),
                );
            }
        }
        // Gizmo.
        if let Some(world) = doc.world_matrix(id) {
            let origin = [world[3][0], world[3][1], world[3][2]];
            let dirs = gizmo::axes(st.mode, &world);
            let size = gizmo::world_size(&st.cam, rect.height());
            let handles = gizmo::handles(st.mode, origin, &dirs, size, &vp, &screen);
            paint_handles(painter, st, st.mode, &handles);
        }
    }
}

fn math_norm(v: [f32; 3]) -> [f32; 3] {
    orbit_render3d::math::normalize(v)
}

fn paint_handles(painter: &egui::Painter, st: &State, mode: Mode, handles: &[gizmo::Handle]) {
    for h in handles {
        let active = st.drag.as_ref().is_some_and(|d| d.axis == h.axis)
            || (st.drag.is_none() && st.hover_axis == Some(h.axis));
        let color = axis_color(h.axis, active);
        let pts: Vec<Pos2> = h.points.iter().map(|p| Pos2::new(p[0], p[1])).collect();
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.5, color)));
        if mode != Mode::Rotate
            && let Some(tip) = pts.last()
        {
            if mode == Mode::Scale {
                painter.rect_filled(Rect::from_center_size(*tip, vec2(9.0, 9.0)), 0.0, color);
            } else {
                painter.circle_filled(*tip, 5.0, color);
            }
        }
    }
}

/// "Discard the unsaved changes?" before a new / open that would drop them.
fn confirm_dialog(ui: &Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    if st.pending.is_none() {
        return;
    }
    let mut choice = None;
    egui::Window::new(ctx.t("s3d.discard_title"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(ctx.t("s3d.discard_msg"));
            ui.horizontal(|ui| {
                if ui.button(ctx.t("s3d.discard")).clicked() {
                    choice = Some(true);
                }
                if ui.button(ctx.t("s3d.cancel")).clicked() {
                    choice = Some(false);
                }
            });
        });
    match (choice, st.pending.take()) {
        (Some(true), Some(Pending::New)) => st.new_scene(),
        (Some(true), Some(Pending::Open(p))) => st.open_path(&p, ctx),
        (None, pending) => st.pending = pending,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene3d_host::{MATERIAL, MAIN, Viewport3dLink};
    use orbit_editor_api::EditorState;

    struct Rig {
        ctx: egui::Context,
        registry: EditorRegistry,
        state: EditorState,
        link: SharedLink,
        st: Rc<RefCell<State>>,
        time: f64,
    }

    impl Rig {
        fn new() -> Self {
            let link: SharedLink = Rc::new(RefCell::new(Viewport3dLink::default()));
            let st = Rc::new(RefCell::new(State::new(link.clone())));
            st.borrow_mut().new_scene();
            Self {
                ctx: {
                    let c = egui::Context::default();
                    crate::theme::install_fonts(&c);
                    c
                },
                registry: EditorRegistry::new(),
                state: EditorState::default(),
                link,
                st,
                time: 0.0,
            }
        }

        /// One frame of `panel`; returns whether egui wants another frame at once.
        fn frame(
            &mut self,
            panel: fn(&mut Ui, &mut EditorContext<'_>, &Rc<RefCell<State>>),
        ) -> bool {
            self.time += 0.016;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                time: Some(self.time),
                ..egui::RawInput::default()
            };
            let (registry, state, st) = (&self.registry, &mut self.state, &self.st);
            let out = self.ctx.run_ui(input, |ui| {
                let mut ctx = EditorContext::new(None, state, registry);
                panel(ui, &mut ctx, st);
            });
            let again = out
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|v| v.repaint_delay.is_zero());
            out.drop_without_applying_deltas();
            again
        }

        fn take_request(&self, slot: usize) -> Option<Viewport3dRequest> {
            self.link.borrow_mut().slots[slot].request.take()
        }

        /// Plays the host: publishes a texture so the viewport settles.
        fn host_texture(&self, slot: usize) {
            self.link.borrow_mut().slots[slot].texture = Some(egui::TextureId::Managed(7));
            self.link.borrow_mut().available = true;
        }
    }

    #[test]
    fn viewport_renders_on_demand_only() {
        let mut rig = Rig::new();
        rig.frame(viewport_panel);
        let req = rig.take_request(MAIN).expect("first frame asks for a picture");
        assert!(req.size[0] > 100 && req.size[1] > 100);
        assert!(req.scene.nodes.len() >= 2);
        let pip = rig.take_request(scene3d_host::CAMERA).expect("the camera preview too");
        assert!(pip.size[0] > 50 && pip.size[0] < req.size[0]);
        rig.host_texture(MAIN);
        rig.host_texture(scene3d_host::CAMERA);
        // Idle: no request, and no endless redraw.
        for _ in 0..4 {
            rig.frame(viewport_panel);
        }
        assert!(rig.take_request(MAIN).is_none(), "an idle scene is not redrawn");
        assert!(!rig.frame(viewport_panel), "the panel settles");
        // A document edit redraws.
        rig.st.borrow_mut().doc.as_mut().unwrap().add_node(model::NewKind::Empty, None);
        rig.frame(viewport_panel);
        assert!(rig.take_request(MAIN).is_some());
        assert!(rig.take_request(scene3d_host::CAMERA).is_some(), "the preview follows edits");
        // A camera move redraws the view, not the game camera preview.
        rig.st.borrow_mut().cam.orbit(30.0, 0.0);
        rig.frame(viewport_panel);
        assert!(rig.take_request(MAIN).is_some());
        assert!(rig.take_request(scene3d_host::CAMERA).is_none());
        // Selection alone is an overlay: no new render.
        let first = rig.st.borrow().doc.as_ref().unwrap().children(None).first().copied();
        rig.st.borrow_mut().selected = first;
        rig.frame(viewport_panel);
        assert!(rig.take_request(MAIN).is_none());
        // Hiding the preview stops its renders.
        rig.st.borrow_mut().show_pip = false;
        rig.st.borrow_mut().doc.as_mut().unwrap().add_node(model::NewKind::Empty, None);
        rig.frame(viewport_panel);
        assert!(rig.take_request(MAIN).is_some());
        assert!(rig.take_request(scene3d_host::CAMERA).is_none());
    }

    #[test]
    fn align_camera_to_view_moves_the_game_camera() {
        let mut rig = Rig::new();
        rig.host_texture(MAIN);
        rig.host_texture(scene3d_host::CAMERA);
        rig.frame(viewport_panel);
        let _ = rig.take_request(scene3d_host::CAMERA);
        {
            let mut st = rig.st.borrow_mut();
            st.cam.orbit(80.0, 20.0);
            assert!(align_camera_to_view(&mut st));
        }
        rig.frame(viewport_panel);
        let pip = rig.take_request(scene3d_host::CAMERA).expect("preview redrawn");
        let want = rig.st.borrow().cam.world();
        for c in 0..4 {
            for r in 0..3 {
                assert!((pip.camera.world[c][r] - want[c][r]).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn panels_draw_for_every_kind_of_selection() {
        let mut rig = Rig::new();
        rig.host_texture(MAIN);
        rig.host_texture(MATERIAL);
        let ids: Vec<Node3dId> = {
            let mut st = rig.st.borrow_mut();
            let doc = st.doc.as_mut().unwrap();
            let mesh = doc.add_mesh("m", None, None);
            let spot = doc.add_node(model::NewKind::Spot, None);
            let point = doc.add_node(model::NewKind::Point, None);
            let mut ids = doc.children(None);
            ids.extend([mesh, spot, point]);
            ids
        };
        for id in ids {
            rig.st.borrow_mut().selected = Some(id);
            rig.frame(viewport_panel);
            rig.frame(panels::hierarchy);
            rig.frame(panels::inspector);
        }
        // Environment (no selection).
        rig.st.borrow_mut().selected = None;
        rig.frame(panels::inspector);
    }

    /// A two-bone skinned mesh `arm` with a `wave` clip.
    fn rigged_lib() -> ModelLib {
        use orbit_scene::skin::{Channel, ChannelPath, Clip, IDENTITY, Interpolation, Joint, Skeleton};
        let mut mesh = orbit_scene::Shape::default_of("cube").unwrap().mesh();
        mesh.name = "arm".into();
        let n = mesh.primitives[0].positions.len();
        mesh.primitives[0].joints = vec![[1, 0, 0, 0]; n];
        mesh.primitives[0].weights = vec![[1.0, 0.0, 0.0, 0.0]; n];
        mesh.skin = Some(Skeleton {
            joints: vec![
                Joint {
                    name: "shoulder".into(),
                    parent: None,
                    rest: orbit_scene::Transform::IDENTITY,
                    base: IDENTITY,
                    inverse_bind: IDENTITY,
                },
                Joint {
                    name: "elbow".into(),
                    parent: Some(0),
                    rest: orbit_scene::Transform {
                        translation: [0.0, 1.0, 0.0],
                        ..orbit_scene::Transform::IDENTITY
                    },
                    base: IDENTITY,
                    inverse_bind: IDENTITY,
                },
            ],
        });
        mesh.clips.push(Clip {
            name: "wave".into(),
            duration: 1.0,
            channels: vec![Channel {
                joint: 1,
                path: ChannelPath::Rotation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 1.0],
                values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.7071, 0.7071]],
            }],
        });
        let mut lib = ModelLib::default();
        lib.meshes.insert("arm".into(), mesh);
        lib
    }

    #[test]
    fn rigged_nodes_get_pose_mode_bones_and_clip_preview() {
        let mut t = Rig::new();
        t.host_texture(MAIN);
        t.host_texture(scene3d_host::CAMERA);
        let id = {
            let mut st = t.st.borrow_mut();
            st.lib = rigged_lib();
            st.refresh_assets();
            let id = st.doc.as_mut().unwrap().add_mesh("arm", None, None);
            st.selected = Some(id);
            st.pose_mode = true;
            st.bone = Some("elbow".into());
            id
        };
        // Every panel draws the rig UI without panicking.
        t.frame(viewport_panel);
        t.frame(panels::hierarchy);
        t.frame(panels::inspector);
        {
            let st = t.st.borrow();
            let pv = pose_view(&st).expect("the selection is rigged");
            assert_eq!(pv.worlds.len(), 2);
            assert!((pv.worlds[1][3][1] - 1.0).abs() < 1e-5);
            // Rotate mode on a child bone, translate allowed on the root.
            assert_eq!(bone_mode(&st, &pv, 1), Mode::Rotate);
            assert_eq!(bone_mode(&st, &pv, 0), Mode::Translate);
        }
        // Previewing a clip changes the rendered scene, not the document.
        let _ = t.take_request(MAIN);
        t.st.borrow_mut().play = Some(rig::Play {
            node: id,
            source: rig::Source::Gltf("wave".into()),
            t: 0.5,
            playing: false,
        });
        t.frame(viewport_panel);
        let req = t.take_request(MAIN).expect("the preview frame is drawn");
        let posed = req.scene.rig(id).expect("pose applied to the render");
        assert!(posed.pose.get("elbow").is_some());
        assert!(t.st.borrow().doc.as_ref().unwrap().scene().rig(id).is_none());
        // A bone edit is one undoable document change.
        {
            let mut st = t.st.borrow_mut();
            st.play = None;
            let doc = st.doc.as_mut().unwrap();
            doc.edit(Some("bone-gizmo"), |s| {
                s.rig_mut(id).pose.set("elbow", orbit_scene::Transform::IDENTITY)
            });
            doc.end_gesture();
            assert!(doc.scene().rig(id).is_some());
            assert!(doc.undo());
            assert!(doc.scene().rig(id).is_none_or(|r| r.pose.is_rest()));
        }
        // Leaving the rigged node hides pose mode's bones.
        t.st.borrow_mut().selected = None;
        t.frame(viewport_panel);
        assert!(pose_view(&t.st.borrow()).is_none());
    }

    #[test]
    fn material_editor_posts_a_sphere_preview() {
        let mut rig = Rig::new();
        let name = {
            let mut st = rig.st.borrow_mut();
            let n = st.doc.as_mut().unwrap().add_material();
            st.material = Some(n.clone());
            n
        };
        rig.frame(panels::inspector);
        let req = rig.take_request(MATERIAL).expect("preview requested");
        assert_eq!(req.scene.materials[0].name, name);
        assert_eq!(req.size[0], req.size[1]);
        assert!(req.assets.meshes.iter().any(|m| m.primitives[0].indices.len() > 100));
        rig.host_texture(MATERIAL);
        rig.frame(panels::inspector);
        assert!(rig.take_request(MATERIAL).is_none(), "unchanged material: no redraw");
        // Editing the material redraws the preview.
        rig.st
            .borrow_mut()
            .doc
            .as_mut()
            .unwrap()
            .edit_material(&name, |m| m.metallic = 0.9);
        rig.frame(panels::inspector);
        let req = rig.take_request(MATERIAL).expect("redraw after an edit");
        assert!((req.scene.materials[0].metallic - 0.9).abs() < 1e-6);
    }

    #[test]
    fn no_scene_shows_the_empty_state() {
        let mut rig = Rig::new();
        rig.st.borrow_mut().doc = None;
        rig.frame(viewport_panel);
        rig.frame(panels::hierarchy);
        rig.frame(panels::inspector);
        assert!(rig.take_request(MAIN).is_none());
    }
}

#[cfg(test)]
mod project_tests {
    use super::*;

    #[test]
    fn the_scene_dirty_state_is_published_to_the_host() {
        let link = Rc::new(orbit_editor_api::UiEditorLink::default());
        let mut st = State::new(SharedLink::default());
        st.ui_link = Some(link.clone());
        st.sync_dirty();
        assert!(!link.scene3d_dirty.get(), "no scene, nothing to save");

        st.new_scene();
        st.sync_dirty();
        assert!(link.scene3d_dirty.get(), "a scene never saved is unsaved");

        let path = std::env::temp_dir()
            .join(format!("orbit_s3d_dirty_{}", Node3dId::new()))
            .join("a.scene3d.ron");
        st.doc.as_mut().unwrap().save_as(&path).unwrap();
        st.sync_dirty();
        assert!(!link.scene3d_dirty.get());

        st.doc.as_mut().unwrap().add_node(model::NewKind::Empty, None);
        st.sync_dirty();
        assert!(link.scene3d_dirty.get());

        st.doc = None;
        st.sync_dirty();
        assert!(!link.scene3d_dirty.get());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
