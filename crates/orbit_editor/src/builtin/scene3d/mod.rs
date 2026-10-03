//! The 3D scene editor (`orbit.scene3d`, M6-08 / M6-09 / M6-10): opens and
//! creates `.scene3d.ron` documents and edits them with three panels.
//!
//! * **Viewport**: the scene rendered by `orbit_render3d` into an egui
//!   texture (on demand: only when the document, the camera or the size
//!   changed), a grid, an orbit / pan / zoom edit camera, click selection
//!   (CPU ray against bounds) and translate / rotate / scale gizmos;
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

use self::geom::{Axis, EditCamera};
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
                for p in [
                    panel_ids::SCENE3D_VIEWPORT,
                    panel_ids::SCENE3D_HIERARCHY,
                    panel_ids::SCENE3D_INSPECTOR,
                ] {
                    ctx.request(EditorRequest::ShowPanel(p.to_owned()));
                }
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
}

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
    sent: Option<Sent>,
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
            sent: None,
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
        self.material_sent = None;
        self.error = None;
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
        let Some(models) = self.models_dir() else {
            ctx.warn(ctx.t("s3d.import_save_first"));
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("glTF", &["glb", "gltf"])
            .pick_file()
        else {
            return;
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model")
            .to_owned();
        let result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let imp = orbit_scene::import::import_gltf(&name, &bytes)?;
                std::fs::create_dir_all(&models).map_err(|e| e.to_string())?;
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("glb");
                std::fs::write(models.join(format!("{name}.{ext}")), &bytes)
                    .map_err(|e| e.to_string())?;
                Ok(imp)
            });
        match result {
            Ok(imp) => {
                for w in &imp.warnings {
                    ctx.warn(ctx.tf("s3d.import_warning", &[("name", &name), ("warning", w)]));
                }
                self.lib.add_imported(imp);
                self.refresh_assets();
                if let Some(doc) = self.doc.as_mut() {
                    let id = doc.add_prefab_instance(&name, self.selected);
                    self.selected = Some(id);
                }
                ctx.info(ctx.tf("s3d.imported", &[("name", &name)]));
            }
            Err(e) => ctx.error(ctx.tf("s3d.import_failed", &[("name", &name), ("error", &e)])),
        }
    }
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

    // ---- ask the host for a picture when something changed
    if let Some(doc) = &st.doc {
        let key = Sent {
            revision: doc.revision,
            assets: st.assets_key,
            cam: cam_bits(&st.cam),
            size: size_px,
        };
        let texture_ready = st.link.borrow().slots[scene3d_host::MAIN].texture.is_some();
        if st.sent.as_ref() != Some(&key) || !texture_ready {
            st.link.borrow_mut().slots[scene3d_host::MAIN].request = Some(Viewport3dRequest {
                scene: doc.scene().clone(),
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
    painter.text(
        rect.left_bottom() + vec2(8.0, -6.0),
        Align2::LEFT_BOTTOM,
        ctx.t("s3d.nav_hint"),
        FontId::proportional(11.0),
        faint(ui),
    );
    confirm_dialog(ui, ctx, st);
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
        ui.separator();
        ui.toggle_value(&mut st.show_grid, RichText::new(icons::GRID_FOUR).size(15.0))
            .on_hover_text(ctx.t("s3d.grid"));
        if icon_button(ui, icons::CROSSHAIR, &ctx.t("s3d.frame")).clicked() {
            frame_selection(st);
        }
    });
}

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

/// Gizmo press / drag / release and click selection.
fn gizmo_and_pick(ui: &Ui, st: &mut State, response: &egui::Response, rect: Rect) {
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
    let snap = ui.input(|i| i.modifiers.ctrl);
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
            for h in gizmo::handles(st.mode, origin, &dirs, size, &vp, &screen) {
                let active = st.drag.as_ref().is_some_and(|d| d.axis == h.axis)
                    || (st.drag.is_none() && st.hover_axis == Some(h.axis));
                let color = axis_color(h.axis, active);
                let pts: Vec<Pos2> = h.points.iter().map(|p| Pos2::new(p[0], p[1])).collect();
                painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.5, color)));
                if st.mode != Mode::Rotate
                    && let Some(tip) = pts.last()
                {
                    if st.mode == Mode::Scale {
                        painter.rect_filled(
                            Rect::from_center_size(*tip, vec2(9.0, 9.0)),
                            0.0,
                            color,
                        );
                    } else {
                        painter.circle_filled(*tip, 5.0, color);
                    }
                }
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
        rig.host_texture(MAIN);
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
        // A camera move redraws.
        rig.st.borrow_mut().cam.orbit(30.0, 0.0);
        rig.frame(viewport_panel);
        assert!(rig.take_request(MAIN).is_some());
        // Selection alone is an overlay: no new render.
        let first = rig.st.borrow().doc.as_ref().unwrap().children(None).first().copied();
        rig.st.borrow_mut().selected = first;
        rig.frame(viewport_panel);
        assert!(rig.take_request(MAIN).is_none());
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
