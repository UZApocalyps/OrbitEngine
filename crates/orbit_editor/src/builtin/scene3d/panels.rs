//! Hierarchy and inspector panels of the 3D editor, and the material editor
//! with its sphere preview (M6-10).

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Color32, ComboBox, DragValue, Pos2, Rect, RichText, Sense, Ui, vec2};
use orbit_editor_api::{EditorContext, icons};
use orbit_render3d::math;
use orbit_scene::{
    AlphaMode, Background, CameraDef, Fog, Light, MaterialDef, Node3dId, Node3dKind, Projection,
    Scene3d, Transform,
};

use super::geom;
use super::model::{self, NewKind};
use super::State;
use crate::builtin::ui_util::{accent, faint, icon_button, section_title, weak};
use crate::scene3d_host::{self, ModelAssets, Viewport3dRequest};

// -------------------------------------------------------------- hierarchy

pub(super) fn hierarchy(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &Rc<RefCell<State>>) {
    let mut guard = state.borrow_mut();
    let st = &mut *guard;
    st.begin_frame(ui);
    if st.doc.is_none() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ctx.t("s3d.no_scene")).color(weak(ui)));
            if ui.button(ctx.t("s3d.new")).clicked() {
                st.request_new();
            }
            if ui.button(ctx.t("s3d.open_file")).clicked() {
                st.request_open(ctx);
            }
        });
        return;
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.menu_button(
            RichText::new(format!("{}  {}", icons::PLUS, ctx.t("s3d.add"))).size(12.5),
            |ui| {
                for (kind, key) in [
                    (NewKind::Empty, "s3d.add.empty"),
                    (NewKind::Directional, "s3d.add.directional"),
                    (NewKind::Point, "s3d.add.point"),
                    (NewKind::Spot, "s3d.add.spot"),
                    (NewKind::Camera, "s3d.add.camera"),
                ] {
                    if ui.button(ctx.t(key)).clicked() {
                        let parent = st.selected;
                        if let Some(doc) = st.doc.as_mut() {
                            st.selected = Some(doc.add_node(kind, parent));
                        }
                        ui.close();
                    }
                }
                if let Some(kind) = shape_menu(ui, ctx) {
                    let parent = st.selected;
                    if let Some(doc) = st.doc.as_mut() {
                        st.selected = Some(doc.add_node(kind, parent));
                    }
                }
            },
        );
        if ui
            .button(format!("{}  {}", icons::DOWNLOAD_SIMPLE, ctx.t("s3d.import_gltf")))
            .on_hover_text(ctx.t("s3d.import_gltf_tip"))
            .clicked()
        {
            st.import_gltf(ctx);
        }
    });
    ui.separator();

    let mut action: Option<Action> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let roots = st.doc.as_ref().map(|d| d.children(None)).unwrap_or_default();
            for id in roots {
                tree_row(ui, ctx, st, id, 0, &mut action);
            }
            models_list(ui, ctx, st);
            // Drop zone: dropping here moves a node to the root.
            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
            if let Some(p) = resp.dnd_release_payload::<Node3dId>() {
                action = Some(Action::Reparent(*p, None));
            }
            if resp.dnd_hover_payload::<Node3dId>().is_some() {
                ui.painter()
                    .rect_stroke(rect, 3.0, egui::Stroke::new(1.0, accent(ui)), egui::StrokeKind::Inside);
            }
            // Click on the empty area clears the selection.
            if ui.input(|i| i.pointer.primary_clicked()) && resp.hovered() {
                st.selected = None;
            }
        });

    if let Some(action) = action
        && let Some(doc) = st.doc.as_mut()
    {
        match action {
            Action::Delete(id) => {
                doc.delete_node(id);
            }
            Action::Duplicate(id) => st.selected = doc.duplicate_node(id).or(st.selected),
            Action::Reparent(id, parent) => {
                doc.reparent(id, parent);
            }
            Action::Add(kind, parent) => st.selected = Some(doc.add_node(kind, Some(parent))),
            Action::Visible(id, v) => doc.set_visible(id, v),
            Action::Rename(id, name) => doc.rename(id, &name),
        }
    }
    if ui.ui_contains_pointer() && !ui.ctx().egui_wants_keyboard_input() {
        st.undo_redo_keys(ui);
    }
}

/// The imported models: drag one onto the viewport (or click +) to place
/// an instance.
fn models_list(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    if st.lib.prefabs.is_empty() {
        return;
    }
    let mut names: Vec<String> = st.lib.prefabs.keys().cloned().collect();
    names.sort();
    ui.add_space(6.0);
    egui::CollapsingHeader::new(RichText::new(ctx.t("s3d.models")).small().strong())
        .id_salt("s3d_models")
        .default_open(true)
        .show(ui, |ui| {
            for name in names {
                let rigged = st.lib.prefabs[&name].nodes.iter().any(|n| match &n.kind {
                    Node3dKind::Mesh { mesh, .. } => {
                        st.lib.meshes.get(&mesh.0).is_some_and(|m| m.skin.is_some())
                    }
                    _ => false,
                });
                ui.horizontal(|ui| {
                    let icon = if rigged { icons::PERSON_SIMPLE } else { icons::PACKAGE };
                    let resp = ui
                        .dnd_drag_source(
                            egui::Id::new(("s3d_model_drag", &name)),
                            super::ModelDrag(name.clone()),
                            |ui| ui.label(format!("{icon}  {name}")),
                        )
                        .response
                        .on_hover_text(ctx.t("s3d.model_drag_hint"));
                    let _ = resp;
                    if ui.small_button(icons::PLUS).on_hover_text(ctx.t("s3d.model_add")).clicked()
                        && let Some(doc) = st.doc.as_mut()
                    {
                        st.selected = Some(doc.add_prefab_instance(&name, None));
                    }
                });
            }
        });
}

enum Action {
    Delete(Node3dId),
    Duplicate(Node3dId),
    Reparent(Node3dId, Option<Node3dId>),
    Add(NewKind, Node3dId),
    Visible(Node3dId, bool),
    Rename(Node3dId, String),
}

/// The "Shape" submenu of the Add menus: the picked kind (menu closed).
fn shape_menu(ui: &mut Ui, ctx: &EditorContext<'_>) -> Option<NewKind> {
    let mut picked = None;
    ui.menu_button(format!("{}  {}", icons::SHAPES, ctx.t("s3d.add.shape")), |ui| {
        for kind in orbit_scene::shapes::SHAPE_KINDS {
            let label = format!("{}  {}", shape_icon(kind), ctx.t(&format!("s3d.shape.{kind}")));
            if ui.button(label).clicked() {
                picked = Some(NewKind::Shape(kind));
                ui.close();
            }
        }
    });
    picked
}

fn shape_icon(kind: &str) -> &'static str {
    match kind {
        "cube" => icons::CUBE,
        "sphere" => icons::SPHERE,
        "plane" => icons::SQUARE,
        "cylinder" => icons::CYLINDER,
        "cone" => icons::TRIANGLE,
        "capsule" => icons::PILL,
        _ => icons::CIRCLE,
    }
}

fn kind_icon(kind: &Node3dKind) -> &'static str {
    match kind {
        Node3dKind::Shape { shape, .. } => shape_icon(shape.kind()),
        Node3dKind::Empty => icons::CROSSHAIR,
        Node3dKind::Mesh { .. } => icons::CUBE,
        Node3dKind::Light(_) => icons::LIGHTBULB,
        Node3dKind::Camera(_) => icons::VIDEO_CAMERA,
        Node3dKind::Prefab(_) => icons::PACKAGE,
    }
}

fn tree_row(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    st: &mut State,
    id: Node3dId,
    depth: usize,
    action: &mut Option<Action>,
) {
    let Some(doc) = &st.doc else { return };
    let Some(node) = doc.node(id) else { return };
    let (name, visible, icon) = (node.name.clone(), node.visible, kind_icon(&node.kind));
    let children = doc.children(Some(id));
    let selected = st.selected == Some(id);
    let row_h = 22.0;
    let (rect, resp) = ui.allocate_exact_size(
        vec2(ui.available_width(), row_h),
        Sense::click_and_drag(),
    );
    if selected || resp.hovered() {
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else {
            ui.visuals().widgets.hovered.bg_fill
        };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if resp.dnd_hover_payload::<Node3dId>().is_some_and(|p| *p != id) {
        ui.painter()
            .rect_stroke(rect, 4.0, egui::Stroke::new(1.0, accent(ui)), egui::StrokeKind::Inside);
    }
    let x = rect.left() + 6.0 + depth as f32 * 14.0;
    let text_color = if visible {
        ui.visuals().text_color()
    } else {
        faint(ui)
    };
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(14.0),
        text_color,
    );
    let renaming = st
        .renaming
        .as_ref()
        .filter(|(rid, _)| *rid == id)
        .map(|(_, b)| b.clone());
    if let Some(mut buf) = renaming {
        let edit_rect = Rect::from_min_max(
            Pos2::new(x + 20.0, rect.top() + 1.0),
            Pos2::new(rect.right() - 28.0, rect.bottom() - 1.0),
        );
        let edit = ui.put(edit_rect, egui::TextEdit::singleline(&mut buf));
        edit.request_focus();
        if edit.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !buf.trim().is_empty() {
                *action = Some(Action::Rename(id, buf.trim().to_owned()));
            }
            st.renaming = None;
        } else {
            st.renaming = Some((id, buf));
        }
    } else {
        ui.painter().text(
            Pos2::new(x + 20.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &name,
            egui::FontId::proportional(13.0),
            text_color,
        );
    }
    // Visibility toggle at the right.
    let eye_rect = Rect::from_center_size(Pos2::new(rect.right() - 14.0, rect.center().y), vec2(20.0, 20.0));
    let eye = ui.interact(eye_rect, ui.id().with(("s3d_eye", id)), Sense::click());
    ui.painter().text(
        eye_rect.center(),
        egui::Align2::CENTER_CENTER,
        if visible { icons::EYE } else { icons::EYE_SLASH },
        egui::FontId::proportional(13.0),
        if eye.hovered() { accent(ui) } else { weak(ui) },
    );
    if eye.clicked() {
        *action = Some(Action::Visible(id, !visible));
    } else if resp.clicked() {
        st.selected = Some(id);
    }
    if resp.double_clicked() {
        st.renaming = Some((id, name.clone()));
    }
    if resp.drag_started() {
        resp.dnd_set_drag_payload(id);
    }
    if let Some(p) = resp.dnd_release_payload::<Node3dId>()
        && *p != id
    {
        *action = Some(Action::Reparent(*p, Some(id)));
    }
    resp.context_menu(|ui| {
        st.selected = Some(id);
        if ui.button(ctx.t("s3d.rename")).clicked() {
            st.renaming = Some((id, name.clone()));
            ui.close();
        }
        if ui.button(ctx.t("s3d.duplicate")).clicked() {
            *action = Some(Action::Duplicate(id));
            ui.close();
        }
        ui.menu_button(ctx.t("s3d.add_child"), |ui| {
            for (kind, key) in [
                (NewKind::Empty, "s3d.add.empty"),
                (NewKind::Directional, "s3d.add.directional"),
                (NewKind::Point, "s3d.add.point"),
                (NewKind::Spot, "s3d.add.spot"),
                (NewKind::Camera, "s3d.add.camera"),
            ] {
                if ui.button(ctx.t(key)).clicked() {
                    *action = Some(Action::Add(kind, id));
                    ui.close();
                }
            }
            if let Some(kind) = shape_menu(ui, ctx) {
                *action = Some(Action::Add(kind, id));
            }
        });
        if ui.button(ctx.t("s3d.unparent")).clicked() {
            *action = Some(Action::Reparent(id, None));
            ui.close();
        }
        if ui.button(ctx.t("s3d.delete")).clicked() {
            *action = Some(Action::Delete(id));
            ui.close();
        }
    });
    for c in children {
        tree_row(ui, ctx, st, c, depth + 1, action);
    }
}

// -------------------------------------------------------------- inspector

pub(super) fn drag3(ui: &mut Ui, label: &str, v: &mut [f32; 3], speed: f64, suffix: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(weak(ui)));
        for (k, c) in v.iter_mut().enumerate() {
            let tint = [
                Color32::from_rgb(230, 70, 70),
                Color32::from_rgb(90, 200, 90),
                Color32::from_rgb(70, 130, 240),
            ][k];
            ui.label(RichText::new(["X", "Y", "Z"][k]).color(tint).small());
            changed |= ui
                .add(DragValue::new(c).speed(speed).suffix(suffix).max_decimals(3))
                .changed();
        }
    });
    changed
}

fn labelled<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(weak(ui)));
        add(ui)
    })
    .inner
}

pub(super) fn inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, state: &Rc<RefCell<State>>) {
    let mut guard = state.borrow_mut();
    let st = &mut *guard;
    st.begin_frame(ui);
    if st.doc.is_none() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(ctx.t("s3d.no_scene")).color(weak(ui))));
        return;
    }
    if ui.ui_contains_pointer() && !ui.ctx().egui_wants_keyboard_input() {
        st.undo_redo_keys(ui);
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match st.selected {
                Some(id) => node_inspector(ui, ctx, st, id),
                None => environment_inspector(ui, ctx, st),
            }
            ui.add_space(8.0);
            ui.separator();
            materials_section(ui, ctx, st);
            problems_section(ui, ctx, st);
        });
}

fn node_inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State, id: Node3dId) {
    let Some(doc) = st.doc.as_mut() else { return };
    let Some(node) = doc.node(id).cloned() else { return };

    section_title(ui, &ctx.t("s3d.node"));
    let mut name = node.name.clone();
    if labelled(ui, &ctx.t("s3d.name"), |ui| ui.text_edit_singleline(&mut name)).changed() {
        doc.edit_node(id, "name", |n| n.name = name.clone());
    }
    let mut visible = node.visible;
    if ui.checkbox(&mut visible, ctx.t("s3d.visible")).changed() {
        doc.set_visible(id, visible);
    }

    section_title(ui, &ctx.t("s3d.transform"));
    let mut t = node.transform;
    let mut euler = geom::quat_to_euler_deg(t.rotation);
    let mut changed = drag3(ui, &ctx.t("s3d.position"), &mut t.translation, 0.05, "");
    if drag3(ui, &ctx.t("s3d.rotation"), &mut euler, 0.5, "°") {
        t.rotation = geom::quat_from_euler_deg(euler);
        changed = true;
    }
    let mut uniform = t.scale[0] == t.scale[1] && t.scale[1] == t.scale[2];
    if drag3(ui, &ctx.t("s3d.scale_label"), &mut t.scale, 0.01, "") {
        changed = true;
        uniform = false;
    }
    let _ = uniform;
    if changed {
        doc.set_transform(id, "inspector", t);
    }
    if ui.small_button(ctx.t("s3d.reset_transform")).clicked() {
        doc.set_transform(id, "reset", Transform::IDENTITY);
    }

    match &node.kind {
        Node3dKind::Light(light) => {
            section_title(ui, &ctx.t("s3d.light"));
            let mut l = light.clone();
            if light_editor(ui, ctx, &mut l) {
                doc.edit_node(id, "light", |n| n.kind = Node3dKind::Light(l.clone()));
            }
        }
        Node3dKind::Camera(cam) => {
            section_title(ui, &ctx.t("s3d.camera"));
            let mut c = cam.clone();
            if camera_editor(ui, ctx, &mut c) {
                doc.edit_node(id, "camera", |n| n.kind = Node3dKind::Camera(c.clone()));
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button(ctx.t("s3d.view_from_camera")).clicked()
                    && let Some(w) = doc.world_matrix(id)
                {
                    // Put the edit camera where the scene camera is.
                    st.cam.look_through(&w);
                }
                if ui
                    .button(ctx.t("s3d.align_camera_short"))
                    .on_hover_text(ctx.t("s3d.align_camera"))
                    .clicked()
                {
                    let w = st.cam.world();
                    doc.align_to(id, &w);
                }
            });
        }
        Node3dKind::Mesh { mesh, materials } => {
            section_title(ui, &ctx.t("s3d.mesh"));
            ui.label(format!("{}: {}", ctx.t("s3d.mesh_name"), mesh.0));
            let names: Vec<String> = doc.scene().materials.iter().map(|m| m.name.clone()).collect();
            let slots = materials.len().max(1);
            for slot in 0..slots {
                let current = materials.get(slot).map(|m| m.0.clone()).unwrap_or_default();
                let mut pick = None;
                labelled(ui, &ctx.tf("s3d.slot", &[("n", &(slot + 1))]), |ui| {
                    ComboBox::from_id_salt(("s3d_slot", id, slot))
                        .selected_text(if current.is_empty() {
                            ctx.t("s3d.none")
                        } else {
                            current.clone()
                        })
                        .show_ui(ui, |ui| {
                            for n in &names {
                                if ui.selectable_label(*n == current, n).clicked() {
                                    pick = Some(n.clone());
                                }
                            }
                        });
                });
                if let Some(p) = pick {
                    doc.assign_material(id, slot, &p);
                    st.material = Some(p);
                }
            }
        }
        Node3dKind::Prefab(p) => {
            section_title(ui, &ctx.t("s3d.prefab"));
            ui.label(format!("{}: {}", ctx.t("s3d.prefab_name"), p.0));
            if !st.lib.prefabs.contains_key(&p.0) {
                ui.label(RichText::new(ctx.t("s3d.prefab_missing")).color(crate::builtin::ui_util::warning(ui)));
            }
        }
        Node3dKind::Shape { shape, material } => {
            section_title(ui, &ctx.t("s3d.shape"));
            let mut s = *shape;
            if shape_editor(ui, ctx, id, &mut s) {
                doc.set_shape(id, s);
            }
            let names: Vec<String> = doc.scene().materials.iter().map(|m| m.name.clone()).collect();
            let current = material.as_ref().map(|m| m.0.clone()).unwrap_or_default();
            let mut pick = None;
            labelled(ui, &ctx.t("s3d.shape_material"), |ui| {
                ComboBox::from_id_salt(("s3d_shape_mat", id))
                    .selected_text(if current.is_empty() {
                        ctx.t("s3d.none")
                    } else {
                        current.clone()
                    })
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_empty(), ctx.t("s3d.none")).clicked() {
                            pick = Some(String::new());
                        }
                        for n in &names {
                            if ui.selectable_label(*n == current, n).clicked() {
                                pick = Some(n.clone());
                            }
                        }
                    });
            });
            if let Some(p) = pick {
                doc.assign_material(id, 0, &p);
                if !p.is_empty() {
                    st.material = Some(p);
                }
            }
        }
        Node3dKind::Empty => {}
    }
    super::rig::inspector(ui, ctx, st, id);
}

/// Kind and parameters of a procedural shape. Returns whether it changed.
fn shape_editor(ui: &mut Ui, ctx: &EditorContext<'_>, id: Node3dId, s: &mut orbit_scene::Shape) -> bool {
    use orbit_scene::Shape;
    let mut changed = false;
    let kind = s.kind();
    let mut new_kind = kind;
    labelled(ui, &ctx.t("s3d.shape_kind"), |ui| {
        ComboBox::from_id_salt(("s3d_shape_kind", id))
            .selected_text(ctx.t(&format!("s3d.shape.{kind}")))
            .show_ui(ui, |ui| {
                for k in orbit_scene::shapes::SHAPE_KINDS {
                    ui.selectable_value(&mut new_kind, k, ctx.t(&format!("s3d.shape.{k}")));
                }
            });
    });
    if new_kind != kind
        && let Some(fresh) = Shape::default_of(new_kind)
    {
        *s = fresh;
        changed = true;
    }
    let len = |ui: &mut Ui, label: &str, v: &mut f32| {
        labelled(ui, label, |ui| {
            ui.add(DragValue::new(v).speed(0.01).range(0.001..=1000.0).suffix(" m").max_decimals(3))
        })
        .changed()
    };
    let count = |ui: &mut Ui, label: &str, v: &mut u32, min: u32| {
        labelled(ui, label, |ui| {
            ui.add(DragValue::new(v).speed(0.2).range(min..=orbit_scene::shapes::MAX_SEGMENTS))
        })
        .changed()
    };
    let (t_size, t_radius, t_height, t_segments, t_rings) = (
        ctx.t("s3d.shape_size"),
        ctx.t("s3d.shape_radius"),
        ctx.t("s3d.shape_height"),
        ctx.t("s3d.shape_segments"),
        ctx.t("s3d.shape_rings"),
    );
    match s {
        Shape::Cube { size } => {
            let mut v = *size;
            if drag3(ui, &t_size, &mut v, 0.01, "") {
                *size = v.map(|x| x.max(0.001));
                changed = true;
            }
        }
        Shape::Sphere { radius, segments, rings } => {
            changed |= len(ui, &t_radius, radius);
            changed |= count(ui, &t_segments, segments, 3);
            changed |= count(ui, &t_rings, rings, 2);
        }
        Shape::Plane { size, subdivisions } => {
            changed |= len(ui, &format!("{t_size} X"), &mut size[0]);
            changed |= len(ui, &format!("{t_size} Z"), &mut size[1]);
            changed |= count(ui, &ctx.t("s3d.shape_subdivisions"), subdivisions, 1);
        }
        Shape::Cylinder { radius, height, segments } | Shape::Cone { radius, height, segments } => {
            changed |= len(ui, &t_radius, radius);
            changed |= len(ui, &t_height, height);
            changed |= count(ui, &t_segments, segments, 3);
        }
        Shape::Capsule { radius, height, segments, rings } => {
            changed |= len(ui, &t_radius, radius);
            changed |= len(ui, &t_height, height);
            changed |= count(ui, &t_segments, segments, 3);
            changed |= count(ui, &t_rings, rings, 1);
        }
        Shape::Torus { radius, tube, segments, sides } => {
            changed |= len(ui, &t_radius, radius);
            changed |= len(ui, &ctx.t("s3d.shape_tube"), tube);
            changed |= count(ui, &t_segments, segments, 3);
            changed |= count(ui, &ctx.t("s3d.shape_sides"), sides, 3);
        }
    }
    changed
}

fn light_editor(ui: &mut Ui, ctx: &EditorContext<'_>, l: &mut Light) -> bool {
    let mut changed = false;
    let kind = match l {
        Light::Directional { .. } => 0,
        Light::Point { .. } => 1,
        Light::Spot { .. } => 2,
    };
    let mut new_kind = kind;
    labelled(ui, &ctx.t("s3d.light_type"), |ui| {
        ComboBox::from_id_salt("s3d_light_type")
            .selected_text(ctx.t(["s3d.add.directional", "s3d.add.point", "s3d.add.spot"][kind]))
            .show_ui(ui, |ui| {
                for (i, k) in ["s3d.add.directional", "s3d.add.point", "s3d.add.spot"]
                    .iter()
                    .enumerate()
                {
                    ui.selectable_value(&mut new_kind, i, ctx.t(k));
                }
            });
    });
    if new_kind != kind {
        let (color, intensity) = match &*l {
            Light::Directional { color, intensity, .. }
            | Light::Point { color, intensity, .. }
            | Light::Spot { color, intensity, .. } => (*color, *intensity),
        };
        *l = match new_kind {
            0 => Light::Directional { color, intensity, shadows: false },
            1 => Light::Point { color, intensity, range: None },
            _ => Light::Spot {
                color,
                intensity,
                range: None,
                inner_angle: 20f32.to_radians(),
                outer_angle: 35f32.to_radians(),
            },
        };
        changed = true;
    }
    match l {
        Light::Directional { color, intensity, shadows } => {
            changed |= labelled(ui, &ctx.t("s3d.color"), |ui| ui.color_edit_button_rgb(color)).changed();
            changed |= labelled(ui, &ctx.t("s3d.intensity"), |ui| {
                ui.add(DragValue::new(intensity).speed(0.05).range(0.0..=1000.0))
            })
            .changed();
            changed |= ui.checkbox(shadows, ctx.t("s3d.shadows")).changed();
        }
        Light::Point { color, intensity, range } => {
            changed |= labelled(ui, &ctx.t("s3d.color"), |ui| ui.color_edit_button_rgb(color)).changed();
            changed |= labelled(ui, &ctx.t("s3d.intensity"), |ui| {
                ui.add(DragValue::new(intensity).speed(0.1).range(0.0..=100000.0))
            })
            .changed();
            changed |= range_editor(ui, ctx, range);
        }
        Light::Spot { color, intensity, range, inner_angle, outer_angle } => {
            changed |= labelled(ui, &ctx.t("s3d.color"), |ui| ui.color_edit_button_rgb(color)).changed();
            changed |= labelled(ui, &ctx.t("s3d.intensity"), |ui| {
                ui.add(DragValue::new(intensity).speed(0.1).range(0.0..=100000.0))
            })
            .changed();
            changed |= range_editor(ui, ctx, range);
            let mut inner = inner_angle.to_degrees();
            let mut outer = outer_angle.to_degrees();
            if labelled(ui, &ctx.t("s3d.inner_angle"), |ui| {
                ui.add(DragValue::new(&mut inner).speed(0.5).range(0.0..=89.0).suffix("°"))
            })
            .changed()
            {
                *inner_angle = inner.to_radians();
                changed = true;
            }
            if labelled(ui, &ctx.t("s3d.outer_angle"), |ui| {
                ui.add(DragValue::new(&mut outer).speed(0.5).range(1.0..=89.0).suffix("°"))
            })
            .changed()
            {
                *outer_angle = outer.to_radians();
                changed = true;
            }
        }
    }
    changed
}

fn range_editor(ui: &mut Ui, ctx: &EditorContext<'_>, range: &mut Option<f32>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(ctx.t("s3d.range")).color(weak(ui)));
        let mut limited = range.is_some();
        if ui.checkbox(&mut limited, "").changed() {
            *range = limited.then_some(10.0);
            changed = true;
        }
        if let Some(r) = range {
            changed |= ui.add(DragValue::new(r).speed(0.1).range(0.01..=100000.0)).changed();
        }
    });
    changed
}

fn camera_editor(ui: &mut Ui, ctx: &EditorContext<'_>, c: &mut CameraDef) -> bool {
    let mut changed = false;
    changed |= labelled(ui, &ctx.t("s3d.camera_name"), |ui| ui.text_edit_singleline(&mut c.name)).changed();
    let persp = matches!(c.projection, Projection::Perspective { .. });
    let mut is_persp = persp;
    labelled(ui, &ctx.t("s3d.projection"), |ui| {
        ui.selectable_value(&mut is_persp, true, ctx.t("s3d.perspective"));
        ui.selectable_value(&mut is_persp, false, ctx.t("s3d.orthographic"));
    });
    if is_persp != persp {
        c.projection = if is_persp {
            Projection::Perspective { fov_y: 50f32.to_radians() }
        } else {
            Projection::Orthographic { height: 6.0 }
        };
        changed = true;
    }
    match &mut c.projection {
        Projection::Perspective { fov_y } => {
            let mut deg = fov_y.to_degrees();
            if labelled(ui, &ctx.t("s3d.fov"), |ui| {
                ui.add(DragValue::new(&mut deg).speed(0.25).range(5.0..=150.0).suffix("°"))
            })
            .changed()
            {
                *fov_y = deg.to_radians();
                changed = true;
            }
        }
        Projection::Orthographic { height } => {
            changed |= labelled(ui, &ctx.t("s3d.ortho_height"), |ui| {
                ui.add(DragValue::new(height).speed(0.05).range(0.01..=10000.0))
            })
            .changed();
        }
    }
    changed |= labelled(ui, &ctx.t("s3d.near"), |ui| {
        ui.add(DragValue::new(&mut c.near).speed(0.01).range(0.001..=1000.0))
    })
    .changed();
    changed |= labelled(ui, &ctx.t("s3d.far"), |ui| {
        ui.add(DragValue::new(&mut c.far).speed(1.0).range(0.1..=100000.0))
    })
    .changed();
    changed
}

/// A combo box choosing a texture by name among `names` (`none_label` for
/// "no texture", when `allow_none`). Returns the new choice (`Some(None)` for
/// "none") when the user picked something.
fn texture_combo(
    ui: &mut Ui,
    ctx: &EditorContext<'_>,
    salt: &str,
    current: Option<&str>,
    names: &[String],
    allow_none: bool,
) -> Option<Option<String>> {
    let missing = current.is_some_and(|c| !names.iter().any(|n| n == c));
    let shown = match current {
        None => ctx.t("s3d.tex.none"),
        Some(c) if missing => ctx.tf("s3d.tex.missing", &[("name", &c)]),
        Some(c) => c.to_owned(),
    };
    let mut out = None;
    ComboBox::from_id_salt(salt)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            if allow_none && ui.selectable_label(current.is_none(), ctx.t("s3d.tex.none")).clicked() {
                out = Some(None);
            }
            for n in names {
                if ui.selectable_label(current == Some(n.as_str()), n).clicked() {
                    out = Some(Some(n.clone()));
                }
            }
        });
    out
}


const SLOT_KEYS: [&str; 4] = ["s3d.tex.base", "s3d.tex.orm", "s3d.tex.normal", "s3d.tex.emissive"];

fn slot_mut(m: &mut MaterialDef, i: usize) -> &mut Option<orbit_scene::TextureSlot> {
    match i {
        0 => &mut m.base_color_texture,
        1 => &mut m.orm_texture,
        2 => &mut m.normal_texture,
        _ => &mut m.emissive_texture,
    }
}

/// Points `slot` at texture `name` (None clears it), keeping the UV
/// transform of an existing binding.
pub(super) fn set_slot_texture(slot: &mut Option<orbit_scene::TextureSlot>, name: Option<String>) {
    match (name, slot.as_mut()) {
        (None, _) => *slot = None,
        (Some(n), Some(s)) => s.texture = orbit_scene::TextureRef(n),
        (Some(n), None) => *slot = Some(orbit_scene::TextureSlot::new(orbit_scene::TextureRef(n))),
    }
}

/// Default colour of a plain-colour background.
const DEFAULT_BACKGROUND: [f32; 3] = [0.12, 0.13, 0.16];

fn environment_inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let names = st.lib.texture_names();
    let mut import = false;
    environment_ui(ui, ctx, st, &names, &mut import);
    if import
        && let Some(name) = st.import_texture(ctx)
        && let Some(doc) = st.doc.as_mut()
    {
        doc.edit(Some("environment"), |s| {
            s.environment.background = Background::Sky(orbit_scene::TextureRef(name));
        });
    }
}

fn environment_ui(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    st: &mut State,
    names: &[String],
    import: &mut bool,
) {
    let Some(doc) = st.doc.as_mut() else { return };
    section_title(ui, &ctx.t("s3d.environment"));
    let mut env = doc.scene().environment.clone();
    let mut changed = false;
    let was_sky = matches!(env.background, Background::Sky(_));
    let mut sky = was_sky;
    labelled(ui, &ctx.t("s3d.background"), |ui| {
        ui.selectable_value(&mut sky, false, ctx.t("s3d.bg_color"));
        ui.selectable_value(&mut sky, true, ctx.t("s3d.bg_sky"));
    });
    if sky != was_sky {
        match (sky, names.first()) {
            (true, Some(first)) => {
                env.background = Background::Sky(orbit_scene::TextureRef(first.clone()));
                changed = true;
            }
            // No texture yet: ask for one (the file picker opens).
            (true, None) => *import = true,
            (false, _) => {
                env.background = Background::Color(DEFAULT_BACKGROUND);
                changed = true;
            }
        }
    }
    match &mut env.background {
        Background::Color(c) => {
            changed |= labelled(ui, &ctx.t("s3d.color"), |ui| ui.color_edit_button_rgb(c)).changed();
        }
        Background::Sky(t) => {
            labelled(ui, &ctx.t("s3d.sky"), |ui| {
                if let Some(Some(n)) = texture_combo(ui, ctx, "s3d_sky", Some(&t.0), names, false) {
                    t.0 = n;
                    changed = true;
                }
                if ui.small_button(ctx.t("s3d.tex.import")).clicked() {
                    *import = true;
                }
            });
        }
    }
    changed |= labelled(ui, &ctx.t("s3d.ambient_color"), |ui| {
        ui.color_edit_button_rgb(&mut env.ambient_color)
    })
    .changed();
    changed |= labelled(ui, &ctx.t("s3d.ambient"), |ui| {
        ui.add(DragValue::new(&mut env.ambient_intensity).speed(0.01).range(0.0..=100.0))
    })
    .changed();
    let fog_kind = match env.fog {
        None => 0,
        Some(Fog::Linear { .. }) => 1,
        Some(Fog::Exponential { .. }) => 2,
    };
    let mut k = fog_kind;
    labelled(ui, &ctx.t("s3d.fog"), |ui| {
        ComboBox::from_id_salt("s3d_fog")
            .selected_text(ctx.t(["s3d.fog.none", "s3d.fog.linear", "s3d.fog.exp"][fog_kind]))
            .show_ui(ui, |ui| {
                for (i, key) in ["s3d.fog.none", "s3d.fog.linear", "s3d.fog.exp"].iter().enumerate() {
                    ui.selectable_value(&mut k, i, ctx.t(key));
                }
            });
    });
    if k != fog_kind {
        env.fog = match k {
            0 => None,
            1 => Some(Fog::Linear { color: [0.6, 0.65, 0.7], start: 5.0, end: 50.0 }),
            _ => Some(Fog::Exponential { color: [0.6, 0.65, 0.7], density: 0.02 }),
        };
        changed = true;
    }
    match &mut env.fog {
        Some(Fog::Linear { color, start, end }) => {
            changed |= labelled(ui, &ctx.t("s3d.color"), |ui| ui.color_edit_button_rgb(color)).changed();
            changed |= labelled(ui, &ctx.t("s3d.fog_start"), |ui| ui.add(DragValue::new(start).speed(0.1))).changed();
            changed |= labelled(ui, &ctx.t("s3d.fog_end"), |ui| ui.add(DragValue::new(end).speed(0.1))).changed();
        }
        Some(Fog::Exponential { color, density }) => {
            changed |= labelled(ui, &ctx.t("s3d.color"), |ui| ui.color_edit_button_rgb(color)).changed();
            changed |= labelled(ui, &ctx.t("s3d.fog_density"), |ui| {
                ui.add(DragValue::new(density).speed(0.001).range(0.0..=10.0))
            })
            .changed();
        }
        None => {}
    }
    if changed {
        doc.edit(Some("environment"), |s| s.environment = env);
    }
}

// -------------------------------------------------------------- materials

fn materials_section(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    section_title(ui, &ctx.t("s3d.materials"));
    let tex_names = st.lib.texture_names();
    let names: Vec<String> = st
        .doc
        .as_ref()
        .map(|d| d.scene().materials.iter().map(|m| m.name.clone()).collect())
        .unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        for n in &names {
            if ui.selectable_label(st.material.as_deref() == Some(n), n).clicked() {
                st.material = Some(n.clone());
            }
        }
        if icon_button(ui, icons::PLUS, &ctx.t("s3d.add_material")).clicked()
            && let Some(doc) = st.doc.as_mut()
        {
            st.material = Some(doc.add_material());
        }
    });
    let Some(name) = st.material.clone() else {
        if names.is_empty() {
            ui.label(RichText::new(ctx.t("s3d.no_materials")).small().color(faint(ui)));
        }
        return;
    };
    let Some(doc) = st.doc.as_mut() else { return };
    let Some(mat) = doc.scene().materials.iter().find(|m| m.name == name).cloned() else {
        return;
    };

    material_preview(ui, st, &mat);
    let Some(doc) = st.doc.as_mut() else { return };

    // Name (committed on Enter / focus loss, renames the references).
    let buf = match &mut st.material_name_buf {
        Some((for_name, buf)) if *for_name == name => buf,
        slot => {
            *slot = Some((name.clone(), name.clone()));
            &mut slot.as_mut().expect("just set").1
        }
    };
    let resp = labelled(ui, &ctx.t("s3d.name"), |ui| ui.text_edit_singleline(buf));
    if resp.lost_focus() {
        let new = buf.trim().to_owned();
        if new != name {
            if doc.rename_material(&name, &new) {
                st.material = Some(new);
            } else {
                *buf = name.clone();
                ctx.warn(ctx.t("s3d.material_name_taken"));
            }
        }
    }

    let mut m = mat.clone();
    ui.checkbox(&mut m.unlit, ctx.t("s3d.unlit"));
    labelled(ui, &ctx.t("s3d.base_color"), |ui| {
        let mut c = egui::Rgba::from_rgba_unmultiplied(m.base_color[0], m.base_color[1], m.base_color[2], m.base_color[3]);
        if egui::color_picker::color_edit_button_rgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend).changed() {
            let [r, g, b, a] = c.to_rgba_unmultiplied();
            m.base_color = [r, g, b, a];
        }
    });
    if !m.unlit {
        labelled(ui, &ctx.t("s3d.metallic"), |ui| {
            ui.add(egui::Slider::new(&mut m.metallic, 0.0..=1.0))
        });
        labelled(ui, &ctx.t("s3d.roughness"), |ui| {
            ui.add(egui::Slider::new(&mut m.roughness, 0.0..=1.0))
        });
        labelled(ui, &ctx.t("s3d.normal_scale"), |ui| {
            ui.add(DragValue::new(&mut m.normal_scale).speed(0.01).range(0.0..=10.0))
        });
        labelled(ui, &ctx.t("s3d.occlusion"), |ui| {
            ui.add(egui::Slider::new(&mut m.occlusion_strength, 0.0..=1.0))
        });
    }
    labelled(ui, &ctx.t("s3d.emissive"), |ui| ui.color_edit_button_rgb(&mut m.emissive));
    labelled(ui, &ctx.t("s3d.emissive_strength"), |ui| {
        ui.add(DragValue::new(&mut m.emissive_strength).speed(0.05).range(0.0..=1000.0))
    });
    let modes = [
        (AlphaMode::Opaque, "s3d.alpha.opaque"),
        (AlphaMode::Mask, "s3d.alpha.mask"),
        (AlphaMode::Blend, "s3d.alpha.blend"),
    ];
    labelled(ui, &ctx.t("s3d.alpha_mode"), |ui| {
        ComboBox::from_id_salt("s3d_alpha")
            .selected_text(ctx.t(modes.iter().find(|(a, _)| *a == m.alpha_mode).map_or("", |(_, k)| k)))
            .show_ui(ui, |ui| {
                for (mode, key) in modes {
                    ui.selectable_value(&mut m.alpha_mode, mode, ctx.t(key));
                }
            });
    });
    if m.alpha_mode == AlphaMode::Mask {
        labelled(ui, &ctx.t("s3d.alpha_cutoff"), |ui| {
            ui.add(egui::Slider::new(&mut m.alpha_cutoff, 0.0..=1.0))
        });
    }
    ui.checkbox(&mut m.double_sided, ctx.t("s3d.double_sided"));
    // Texture slots: any texture of the scene (glTF textures and images
    // imported next to the scene file).
    let mut import_slot: Option<usize> = None;
    for (i, key) in SLOT_KEYS.iter().enumerate() {
        // An unlit material only reads its base colour and emission.
        if m.unlit && (i == 1 || i == 2) {
            continue;
        }
        labelled(ui, &ctx.t(key), |ui| {
            let current = slot_mut(&mut m, i).as_ref().map(|s| s.texture.0.clone());
            if let Some(choice) =
                texture_combo(ui, ctx, &format!("s3d_tex_{i}"), current.as_deref(), &tex_names, true)
            {
                set_slot_texture(slot_mut(&mut m, i), choice);
            }
            if ui.small_button(ctx.t("s3d.tex.import")).clicked() {
                import_slot = Some(i);
            }
        });
    }
    if m != mat {
        let n = name.clone();
        doc.edit_material(&n, |dst| {
            let keep = dst.name.clone();
            *dst = m;
            dst.name = keep;
        });
    }
    ui.add_space(4.0);
    if ui.button(ctx.t("s3d.delete_material")).clicked() {
        doc.remove_material(&name);
        st.material = None;
    }
    if let Some(i) = import_slot
        && let Some(texture) = st.import_texture(ctx)
        && let Some(doc) = st.doc.as_mut()
    {
        doc.edit_material(&name, |dst| set_slot_texture(slot_mut(dst, i), Some(texture)));
    }
}

/// Sphere preview of a material, rendered by the host into its own target.
fn material_preview(ui: &mut Ui, st: &mut State, mat: &MaterialDef) {
    const SIZE: f32 = 140.0;
    let ppp = ui.ctx().pixels_per_point();
    let px = (SIZE * ppp).round() as u32;
    let (rect, _) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::hover());
    // Key: the material content changes with the document revision.
    let rev = st.doc.as_ref().map_or(0, |d| d.revision);
    let key = (rev, mat.name.clone(), [px, px], st.assets_key);
    let ready = st.link.borrow().slots[scene3d_host::MATERIAL].texture.is_some();
    if st.material_sent.as_ref() != Some(&key) || !ready {
        let scene = preview_scene(mat);
        let mut assets = ModelAssets {
            meshes: vec![model::preview_sphere(PREVIEW_MESH)],
            ..ModelAssets::default()
        };
        assets.textures = st.lib.textures.clone();
        st.link.borrow_mut().slots[scene3d_host::MATERIAL].request = Some(Viewport3dRequest {
            scene,
            camera: orbit_render3d::Camera3d {
                world: math::from_trs([0.0, 0.0, 3.4], [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
                projection: Projection::Perspective { fov_y: 30f32.to_radians() },
                near: 0.1,
                far: 20.0,
            },
            size: [px, px],
            assets_key: (1 << 62) | st.assets_key,
            assets: Rc::new(assets),
        });
        st.material_sent = Some(key);
        if !ready {
            ui.ctx().request_repaint();
        }
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::from_gray(30));
    let tex = st.link.borrow().slots[scene3d_host::MATERIAL].texture;
    if let Some(tex) = tex {
        painter.image(
            tex,
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

pub(super) const PREVIEW_MESH: &str = "__orbit_material_sphere";

/// A lit sphere wearing `mat`.
pub(super) fn preview_scene(mat: &MaterialDef) -> Scene3d {
    use orbit_scene::{Node3d, MaterialRef, MeshRef};
    let mut scene = Scene3d::new("preview");
    scene.materials.push(mat.clone());
    scene.environment.background = Background::Color([0.11, 0.115, 0.13]);
    scene.environment.ambient_intensity = 0.35;
    scene.nodes.push(Node3d::new(
        "sphere",
        None,
        Node3dKind::Mesh {
            mesh: MeshRef(PREVIEW_MESH.to_owned()),
            materials: vec![MaterialRef(mat.name.clone())],
        },
    ));
    let mut sun = Node3d::new(
        "key",
        None,
        Node3dKind::Light(Light::Directional {
            color: [1.0; 3],
            intensity: 3.5,
            shadows: false,
        }),
    );
    sun.transform.rotation = geom::quat_from_euler_deg([-35.0, 40.0, 0.0]);
    scene.nodes.push(sun);
    let mut fill = Node3d::new(
        "fill",
        None,
        Node3dKind::Light(Light::Directional {
            color: [0.5, 0.6, 0.8],
            intensity: 1.0,
            shadows: false,
        }),
    );
    fill.transform.rotation = geom::quat_from_euler_deg([10.0, -140.0, 0.0]);
    scene.nodes.push(fill);
    scene
}

fn problems_section(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(doc) = &st.doc else { return };
    let issues = orbit_scene::validate(doc.scene());
    if issues.is_empty() {
        return;
    }
    section_title(ui, &ctx.t("s3d.problems"));
    for i in issues.iter().take(8) {
        let color = match i.severity {
            orbit_scene::Severity::Error => crate::builtin::ui_util::danger(ui),
            orbit_scene::Severity::Warning => crate::builtin::ui_util::warning(ui),
        };
        ui.label(RichText::new(format!("{}: {}", i.path, i.message)).small().color(color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_slots_set_clear_and_keep_their_uv_transform() {
        let mut slot = None;
        set_slot_texture(&mut slot, Some("wall".into()));
        assert_eq!(slot.as_ref().unwrap().texture.0, "wall");
        slot.as_mut().unwrap().scale = [2.0, 2.0];
        set_slot_texture(&mut slot, Some("brick".into()));
        let s = slot.as_ref().unwrap();
        assert_eq!((s.texture.0.as_str(), s.scale), ("brick", [2.0, 2.0]));
        set_slot_texture(&mut slot, None);
        assert!(slot.is_none());
        let mut m = MaterialDef::new("M");
        for i in 0..4 {
            set_slot_texture(slot_mut(&mut m, i), Some(format!("t{i}")));
        }
        assert_eq!(m.normal_texture.as_ref().unwrap().texture.0, "t2");
        assert_eq!(m.emissive_texture.as_ref().unwrap().texture.0, "t3");
    }

    #[test]
    fn preview_scene_renders_the_material_on_a_lit_sphere() {
        let mut m = MaterialDef::new("Gold");
        m.base_color = [1.0, 0.8, 0.2, 1.0];
        let s = preview_scene(&m);
        assert!(orbit_scene::validate(&s).is_empty(), "{:?}", orbit_scene::validate(&s));
        assert_eq!(s.materials[0], m);
        assert!(s.nodes.iter().any(|n| matches!(&n.kind, Node3dKind::Mesh { materials, .. } if materials[0].0 == "Gold")));
        assert!(s.nodes.iter().filter(|n| matches!(n.kind, Node3dKind::Light(_))).count() >= 1);
    }
}
