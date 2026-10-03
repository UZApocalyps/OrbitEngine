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

enum Action {
    Delete(Node3dId),
    Duplicate(Node3dId),
    Reparent(Node3dId, Option<Node3dId>),
    Add(NewKind, Node3dId),
    Visible(Node3dId, bool),
    Rename(Node3dId, String),
}

fn kind_icon(kind: &Node3dKind) -> &'static str {
    match kind {
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

fn drag3(ui: &mut Ui, label: &str, v: &mut [f32; 3], speed: f64, suffix: &str) -> bool {
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
            if ui.button(ctx.t("s3d.view_from_camera")).clicked()
                && let Some(w) = doc.world_matrix(id)
            {
                // Put the edit camera where the scene camera is.
                let eye = [w[3][0], w[3][1], w[3][2]];
                let fwd = math::normalize(math::transform_vector(&w, [0.0, 0.0, -1.0]));
                let dist = st.cam.distance;
                st.cam.target = geom::add(eye, geom::scale(fwd, dist));
                st.cam.yaw = (-fwd[0]).atan2(-fwd[2]);
                st.cam.pitch = (-fwd[1]).asin();
            }
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
        Node3dKind::Empty => {}
    }
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

fn environment_inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(doc) = st.doc.as_mut() else { return };
    section_title(ui, &ctx.t("s3d.environment"));
    let mut env = doc.scene().environment.clone();
    let mut changed = false;
    match &mut env.background {
        Background::Color(c) => {
            changed |= labelled(ui, &ctx.t("s3d.background"), |ui| ui.color_edit_button_rgb(c)).changed();
        }
        Background::Sky(t) => {
            ui.label(format!("{}: {}", ctx.t("s3d.sky"), t.0));
            if ui.small_button(ctx.t("s3d.sky_clear")).clicked() {
                env.background = Background::Color([0.12, 0.13, 0.16]);
                changed = true;
            }
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
    // Textures are shown, not edited here (they come from the glTF import).
    for (key, slot) in [
        ("s3d.tex.base", &m.base_color_texture),
        ("s3d.tex.orm", &m.orm_texture),
        ("s3d.tex.normal", &m.normal_texture),
        ("s3d.tex.emissive", &m.emissive_texture),
    ] {
        if let Some(s) = slot {
            ui.label(RichText::new(format!("{}: {}", ctx.t(key), s.texture.0)).small().color(faint(ui)));
        }
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
