//! Mesh and weight tools of the rig viewport (M5B-04, M5B-05) and their
//! inspector section. The geometry is in [`super::meshgen`], the weights in
//! [`super::weights`]; this file only maps pointer gestures to them.
//!
//! * **Mesh tool** (rest pose): click inside the mesh adds a vertex (on an
//!   outline edge: splits it), drag moves one (Alt: keeps its UV, the picture
//!   warps), Ctrl-drag from a vertex to another adds a fixed edge, Alt-click
//!   on an inner edge flips it, Del deletes the selected vertex.
//! * **Weight tool** (rest or pose mode): paints the selected bone with the
//!   brush (add / subtract / smooth); Ctrl-click picks the bone. The heat
//!   map shows the selected bone's weights.

use egui::{Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, epaint::Vertex, vec2};
use orbit_anim::rig::{Attachment, MeshAttachment, SlotId};
use orbit_editor_api::{EditorContext, icons};

use super::geom::{self, Tool};
use super::meshgen::{self, MeshError, MeshParams, P};
use super::model::{self, DEFAULT_SKIN};
use super::weights::{self, BrushMode};
use super::{State, center_of, pointer_rig, to_pos};
use crate::builtin::ui_util::{accent, faint, section_title, weak};

/// Screen pixels to grab a vertex or an edge.
const GRAB: f32 = 8.0;

/// Settings and gesture state of the mesh and weight tools.
pub(crate) struct MeshEdit {
    pub params: MeshParams,
    /// Picture the params were initialized for.
    params_for: Option<String>,
    pub auto_weights: bool,
    pub falloff: f32,
    /// User edges kept by re-triangulations, for `pinned_for`.
    pinned: Vec<[usize; 2]>,
    pinned_for: Option<(SlotId, String)>,
    pub selected_vertex: Option<usize>,
    drag_vertex: Option<usize>,
    edge_from: Option<usize>,
    pub brush: BrushMode,
    /// Brush radius in screen pixels.
    pub radius: f32,
    pub strength: f32,
    pub heatmap: bool,
    last_dab: Option<P>,
}

impl Default for MeshEdit {
    fn default() -> Self {
        Self {
            params: MeshParams::default(),
            params_for: None,
            auto_weights: true,
            falloff: 3.0,
            pinned: Vec::new(),
            pinned_for: None,
            selected_vertex: None,
            drag_vertex: None,
            edge_from: None,
            brush: BrushMode::Add,
            radius: 40.0,
            strength: 0.3,
            heatmap: true,
            last_dab: None,
        }
    }
}

/// The attachment of the selected slot, as edited.
pub(super) struct Target {
    pub slot: SlotId,
    pub skin: String,
    /// Index of the slot's bone.
    pub bone: usize,
    pub att: Attachment,
}

impl Target {
    pub fn mesh(&self) -> Option<&MeshAttachment> {
        match &self.att {
            Attachment::Mesh(m) => Some(m),
            Attachment::Region(_) => None,
        }
    }
}

/// The selected slot's attachment in the shown skin (or the skin that has it).
pub(super) fn target(st: &State) -> Option<Target> {
    let rig = st.doc.as_ref()?.rig();
    // No slot selected: the first slot showing a mesh, so the weight tool
    // works right after "Bind to bones" without visiting the hierarchy.
    let slot = match st.selected_slot.as_ref() {
        Some(id) => model::slot(rig, id)?,
        None => rig.slots.iter().find(|s| {
            s.attachment.as_ref().is_some_and(|a| {
                rig.skins
                    .iter()
                    .any(|k| matches!(k.attachments.get(a), Some(Attachment::Mesh(_))))
            })
        })?,
    };
    let att = slot.attachment.as_ref()?;
    let has = |n: &str| {
        rig.skins
            .iter()
            .any(|k| k.name == n && k.attachments.contains_key(att))
    };
    let skin = [st.skin.as_str(), DEFAULT_SKIN]
        .into_iter()
        .find(|n| has(n))
        .map(str::to_owned)
        .or_else(|| {
            rig.skins
                .iter()
                .find(|k| k.attachments.contains_key(att))
                .map(|k| k.name.clone())
        })?;
    let a = rig
        .skins
        .iter()
        .find(|k| k.name == skin)?
        .attachments
        .get(att)?
        .clone();
    Some(Target {
        slot: slot.id.clone(),
        skin,
        bone: model::bone_index(rig, &slot.bone)?,
        att: a,
    })
}

/// Whether the mesh tools apply to the current selection.
pub(super) fn has_mesh(st: &State) -> bool {
    target(st).is_some_and(|t| t.mesh().is_some())
}

fn report<T>(ctx: &mut EditorContext<'_>, r: Result<T, MeshError>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            ctx.warn(ctx.t(e.key()));
            None
        }
    }
}

/// Forgets user edges of another mesh, or pointing past the vertices
/// (after an undo).
fn sync_pinned(st: &mut State, t: &Target) {
    let key = Some((t.slot.clone(), t.skin.clone()));
    let n = t.mesh().map_or(0, |m| m.vertices.len());
    let me = &mut st.mesh;
    if me.pinned_for != key {
        me.pinned_for = key;
        me.pinned.clear();
        me.selected_vertex = None;
    }
    me.pinned.retain(|e| e[0] < n && e[1] < n);
    if me.selected_vertex.is_some_and(|v| v >= n) {
        me.selected_vertex = None;
    }
}

/// Mesh positions on screen, skinned by the displayed pose.
fn screen_positions(st: &State, rect: Rect, t: &Target, m: &MeshAttachment) -> Vec<P> {
    let Some(doc) = &st.doc else {
        return Vec::new();
    };
    let rig = doc.rig();
    let bind = model::world_matrices(rig, &model::rest_locals(rig));
    let world = st.world();
    let center = center_of(rect);
    weights::skin_positions(rig, m, t.bone, &world, &bind)
        .into_iter()
        .map(|p| st.view.to_screen(center, p))
        .collect()
}

// ---------------------------------------------------------------- viewport

/// Pointer gestures of the mesh and weight tools.
pub(super) fn interact(
    ui: &Ui,
    ctx: &mut EditorContext<'_>,
    st: &mut State,
    response: &egui::Response,
    rect: Rect,
) {
    let Some(t) = target(st) else { return };
    sync_pinned(st, &t);
    let Some(mesh) = t.mesh().cloned() else {
        return;
    };
    match st.tool {
        Tool::Mesh if !st.pose_mode => mesh_tool(ui, ctx, st, response, rect, &t, mesh),
        Tool::Weights => weight_tool(ui, st, response, rect, &t, mesh),
        _ => {}
    }
}

fn commit(ctx: &mut EditorContext<'_>, st: &mut State, t: &Target, m: MeshAttachment) {
    if let Some(doc) = st.doc.as_mut()
        && let Err(e) = doc.set_mesh(&t.slot, &t.skin, m)
    {
        ctx.warn(ctx.t(e.key()));
    }
}

fn mesh_tool(
    ui: &Ui,
    ctx: &mut EditorContext<'_>,
    st: &mut State,
    response: &egui::Response,
    rect: Rect,
    t: &Target,
    mut m: MeshAttachment,
) {
    let primary = egui::PointerButton::Primary;
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let (ctrl, alt) = ui.input(|i| (i.modifiers.command, i.modifiers.alt));
    let screen = screen_positions(st, rect, t, &m);
    let under = |pos: Pos2| meshgen::nearest_vertex(&screen, [pos.x, pos.y], GRAB);

    if response.drag_started_by(primary)
        && let Some(pos) = response.interact_pointer_pos().or(pointer)
        && let Some(v) = under(pos)
    {
        st.mesh.selected_vertex = Some(v);
        if ctrl {
            st.mesh.edge_from = Some(v);
        } else {
            st.mesh.drag_vertex = Some(v);
            if let Some(doc) = st.doc.as_mut() {
                doc.begin_gesture();
            }
        }
    }
    if response.dragged_by(primary)
        && let (Some(v), Some(pos)) = (st.mesh.drag_vertex, pointer)
    {
        let p = pointer_rig(st, rect, pos);
        let map = meshgen::uv_map(&m);
        meshgen::move_vertex(&mut m, v, p, map.as_ref(), alt);
        commit(ctx, st, t, m);
        return;
    }
    if response.drag_stopped_by(primary) {
        st.mesh.drag_vertex = None;
        if let (Some(a), Some(pos)) = (st.mesh.edge_from.take(), pointer)
            && let Some(b) = under(pos)
            && a != b
        {
            let mut pinned = st.mesh.pinned.clone();
            if report(ctx, meshgen::add_edge(&mut m, a, b, &mut pinned)).is_some() {
                st.mesh.pinned = pinned;
                commit(ctx, st, t, m);
            }
        }
        return;
    }
    if response.clicked_by(primary)
        && let Some(pos) = pointer
    {
        if let Some(v) = under(pos) {
            st.mesh.selected_vertex = Some(v);
            return;
        }
        let rig_p = pointer_rig(st, rect, pos);
        let snap = GRAB / st.view.zoom.max(1e-3);
        if alt {
            let inner: Vec<[usize; 2]> = {
                let b = meshgen::boundary_edges(&m.triangles);
                meshgen::edges(&m.triangles)
                    .into_iter()
                    .filter(|e| !b.iter().any(|x| [x[0].min(x[1]), x[0].max(x[1])] == *e))
                    .collect()
            };
            if let Some(e) = meshgen::nearest_edge(&m.vertices, &inner, rig_p, snap)
                && meshgen::flip_edge(&mut m, e[0], e[1])
            {
                st.mesh.pinned.retain(|x| x != &e);
                commit(ctx, st, t, m);
            }
            return;
        }
        let mut pinned = st.mesh.pinned.clone();
        if let Some(v) = report(ctx, meshgen::add_vertex(&mut m, rig_p, snap, &mut pinned)) {
            st.mesh.pinned = pinned;
            st.mesh.selected_vertex = Some(v);
            commit(ctx, st, t, m);
        }
    }
}

/// Deletes the selected vertex (Del in the mesh tool).
pub(super) fn delete_selected_vertex(ctx: &mut EditorContext<'_>, st: &mut State) -> bool {
    let (Some(t), Some(v)) = (target(st), st.mesh.selected_vertex) else {
        return false;
    };
    let Some(mut m) = t.mesh().cloned() else {
        return false;
    };
    let mut pinned = st.mesh.pinned.clone();
    if report(ctx, meshgen::delete_vertex(&mut m, v, &mut pinned)).is_some() {
        st.mesh.pinned = pinned;
        st.mesh.selected_vertex = None;
        commit(ctx, st, &t, m);
    }
    true
}

fn weight_tool(
    ui: &Ui,
    st: &mut State,
    response: &egui::Response,
    rect: Rect,
    t: &Target,
    mut m: MeshAttachment,
) {
    let primary = egui::PointerButton::Primary;
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let ctrl = ui.input(|i| i.modifiers.command);
    // Ctrl-click: pick the bone to paint.
    if ctrl {
        if response.clicked_by(primary)
            && let Some(pos) = pointer
            && let Some(doc) = &st.doc
        {
            let world = st.world();
            st.selected =
                geom::hit_bone(doc.rig(), &world, &st.view, center_of(rect), [pos.x, pos.y])
                    .map(|i| doc.rig().bones[i].id.clone());
        }
        return;
    }
    let Some(bone) = st.selected.clone() else {
        return;
    };
    if response.drag_stopped_by(primary) {
        st.mesh.last_dab = None;
        return;
    }
    let pressing = response.drag_started_by(primary)
        || response.dragged_by(primary)
        || response.clicked_by(primary);
    let Some(pos) = pointer.filter(|_| pressing) else {
        return;
    };
    let c = [pos.x, pos.y];
    if response.dragged_by(primary)
        && !response.drag_started_by(primary)
        && let Some(last) = st.mesh.last_dab
        && (last[0] - c[0]).hypot(last[1] - c[1]) < st.mesh.radius * 0.15
    {
        return;
    }
    st.mesh.last_dab = Some(c);
    let screen = screen_positions(st, rect, t, &m);
    let slot_bone = st.doc.as_ref().map(|d| d.rig().bones[t.bone].id.clone());
    let Some(slot_bone) = slot_bone else { return };
    weights::ensure_weights(&mut m, &slot_bone);
    let adj = weights::adjacency(m.vertices.len(), &m.triangles);
    if weights::apply_brush(
        &mut m.weights,
        &screen,
        &adj,
        c,
        st.mesh.radius,
        st.mesh.strength,
        st.mesh.brush,
        &bone,
    ) && let Some(doc) = st.doc.as_mut()
    {
        doc.begin_gesture();
        let _ = doc.set_mesh(&t.slot, &t.skin, m);
    }
}

/// Wireframe, heat map and brush circle.
pub(super) fn paint(ui: &Ui, st: &State, painter: &egui::Painter, rect: Rect) {
    let Some(t) = target(st) else { return };
    let Some(m) = t.mesh() else { return };
    let screen = screen_positions(st, rect, &t, m);
    let tool = st.tool;
    let editing = matches!(tool, Tool::Mesh | Tool::Weights);
    // Heat map of the selected bone.
    if (tool == Tool::Weights || (editing && st.mesh.heatmap))
        && let (Some(doc), Some(bone)) = (&st.doc, &st.selected)
    {
        let rigid = model::bone_index(doc.rig(), bone) == Some(t.bone);
        let mut mesh = egui::Mesh::default();
        for (i, p) in screen.iter().enumerate() {
            let w = m
                .weights
                .get(i)
                .filter(|l| !l.is_empty())
                .map_or(if rigid { 1.0 } else { 0.0 }, |l| {
                    weights::weight_of(l, bone)
                });
            let [r, g, b] = weights::heat_color(w);
            mesh.vertices.push(Vertex {
                pos: to_pos(*p),
                uv: egui::epaint::WHITE_UV,
                color: Color32::from_rgba_unmultiplied(r, g, b, 150),
            });
        }
        mesh.indices = m
            .triangles
            .iter()
            .flat_map(|t| t.iter().map(|&i| i as u32))
            .collect();
        painter.add(Shape::mesh(mesh));
    }
    let line = if editing {
        Stroke::new(1.0, Color32::from_white_alpha(150))
    } else {
        Stroke::new(1.0, Color32::from_white_alpha(45))
    };
    for e in meshgen::edges(&m.triangles) {
        let (Some(a), Some(b)) = (screen.get(e[0]), screen.get(e[1])) else {
            continue;
        };
        let pinned = st.mesh.pinned.contains(&e);
        let stroke = if pinned {
            Stroke::new(2.0, Color32::from_rgb(230, 170, 60))
        } else {
            line
        };
        painter.line_segment([to_pos(*a), to_pos(*b)], stroke);
    }
    if tool == Tool::Mesh {
        for (i, p) in screen.iter().enumerate() {
            let sel = st.mesh.selected_vertex == Some(i);
            painter.circle_filled(
                to_pos(*p),
                if sel { 4.5 } else { 3.0 },
                if sel { accent(ui) } else { Color32::WHITE },
            );
        }
        if let (Some(a), Some(p)) = (
            st.mesh.edge_from.and_then(|v| screen.get(v)),
            ui.input(|i| i.pointer.latest_pos()),
        ) {
            painter.line_segment([to_pos(*a), p], Stroke::new(1.5, accent(ui)));
        }
    }
    if tool == Tool::Weights
        && let Some(p) = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|p| rect.contains(*p))
    {
        painter.circle_stroke(
            p,
            st.mesh.radius,
            Stroke::new(1.5, Color32::from_white_alpha(200)),
        );
        painter.circle_stroke(
            p,
            st.mesh.radius * st.mesh.strength.clamp(0.05, 1.0),
            Stroke::new(1.0, Color32::from_white_alpha(90)),
        );
    }
}

// ---------------------------------------------------------------- inspector

/// Pixel size of the picture, cached.
fn picture_size(st: &mut State, ctx: &EditorContext<'_>, path: &str) -> [f32; 2] {
    let root = st.project_root(ctx);
    super::panels::image_size(st, &root, path)
}

/// Builds the automatic mesh of the target's picture (region: placed like
/// the region; mesh: on the same picture mapping) and stores it.
fn generate(ctx: &mut EditorContext<'_>, st: &mut State, t: &Target) {
    let image = t.att.image().to_owned();
    let root = st.project_root(ctx);
    let decoded = std::fs::read(root.join(&image))
        .map_err(|e| e.to_string())
        .and_then(|b| orbit_assets::decode_image(&b));
    let img = match decoded {
        Ok(i) => i,
        Err(e) => {
            ctx.warn(ctx.tf("rig.mesh.err.image", &[("path", &image), ("error", &e)]));
            return;
        }
    };
    let Some(doc) = st.doc.as_ref() else { return };
    let rig = doc.rig();
    let uv_to_rig = match &t.att {
        Attachment::Region(r) => {
            let bind = model::world_matrices(rig, &model::rest_locals(rig));
            meshgen::region_uv_to_rig(&bind[t.bone].mul(&r.xform.matrix()), r.size)
        }
        Attachment::Mesh(m) => match meshgen::uv_map(m).and_then(|u| u.inverse()) {
            Some(a) => a,
            None => {
                ctx.warn(ctx.t(MeshError::Invalid.key()));
                return;
            }
        },
    };
    let g = match meshgen::generate(img.width, img.height, &img.rgba, &st.mesh.params) {
        Ok(g) => g,
        Err(e) => {
            ctx.warn(ctx.t(e.key()));
            return;
        }
    };
    let mut mesh = meshgen::to_attachment(&image, &g, img.width, img.height, &uv_to_rig);
    if st.mesh.auto_weights && !rig.bones.is_empty() {
        mesh.weights = auto_weights_for(rig, &mesh, st.mesh.falloff);
    }
    let (nv, nt) = (mesh.vertices.len(), mesh.triangles.len());
    if g.skipped > 0 {
        ctx.warn(ctx.tf("rig.mesh.skipped", &[("count", &g.skipped)]));
    }
    if let Some(doc) = st.doc.as_mut()
        && doc.set_mesh(&t.slot, &t.skin, mesh).is_ok()
    {
        ctx.info(ctx.tf(
            "rig.mesh.generated",
            &[("vertices", &nv), ("triangles", &nt)],
        ));
    }
    st.mesh.pinned.clear();
    st.mesh.selected_vertex = None;
    if st.tool != Tool::Weights && !st.pose_mode {
        st.tool = Tool::Mesh;
    }
}

fn auto_weights_for(
    rig: &orbit_anim::rig::Rig,
    m: &MeshAttachment,
    falloff: f32,
) -> Vec<Vec<orbit_anim::rig::Influence>> {
    weights::bind_mesh(rig, m, falloff)
}

/// "Mesh" and "Weights" sections of the slot inspector.
pub(super) fn inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let Some(t) = target(st) else { return };
    sync_pinned(st, &t);
    section_title(ui, &ctx.t("rig.mesh.title"));
    let image = t.att.image().to_owned();
    if st.mesh.params_for.as_deref() != Some(image.as_str()) {
        let s = picture_size(st, ctx, &image);
        st.mesh.params = MeshParams::for_image(s[0] as u32, s[1] as u32);
        st.mesh.params_for = Some(image.clone());
    }
    if let Some(m) = t.mesh() {
        ui.label(
            RichText::new(ctx.tf(
                "rig.mesh.stats",
                &[
                    ("vertices", &m.vertices.len()),
                    ("triangles", &m.triangles.len()),
                ],
            ))
            .small()
            .color(weak(ui)),
        );
    }
    let p = &mut st.mesh.params;
    let row = |ui: &mut Ui, label: String, w: &mut dyn FnMut(&mut Ui) -> egui::Response| {
        ui.horizontal(|ui| {
            ui.add_sized(
                vec2(110.0, 18.0),
                egui::Label::new(RichText::new(label).color(weak(ui))),
            );
            w(ui);
        });
    };
    row(ui, ctx.t("rig.mesh.alpha"), &mut |ui| {
        ui.add(egui::Slider::new(&mut p.alpha_threshold, 0..=254))
    });
    row(ui, ctx.t("rig.mesh.padding"), &mut |ui| {
        ui.add(egui::Slider::new(&mut p.padding, 0..=16).suffix(" px"))
    });
    row(ui, ctx.t("rig.mesh.tolerance"), &mut |ui| {
        ui.add(egui::Slider::new(&mut p.tolerance, 0.5..=32.0).suffix(" px"))
    });
    row(ui, ctx.t("rig.mesh.spacing"), &mut |ui| {
        ui.add(egui::Slider::new(&mut p.spacing, 0.0..=512.0).suffix(" px"))
            .on_hover_text(ctx.t("rig.mesh.spacing_hint"))
    });
    ui.checkbox(
        &mut st.mesh.auto_weights,
        ctx.t("rig.mesh.auto_weights_on_generate"),
    );
    let label = if t.mesh().is_some() {
        "rig.mesh.regenerate"
    } else {
        "rig.mesh.generate"
    };
    if ui
        .button(format!("{}  {}", icons::POLYGON, ctx.t(label)))
        .on_hover_text(ctx.t("rig.mesh.generate_hint"))
        .clicked()
    {
        generate(ctx, st, &t);
        return;
    }
    let Some(m) = t.mesh().cloned() else { return };
    if !st.pose_mode {
        ui.label(
            RichText::new(ctx.t("rig.mesh.edit_hint"))
                .small()
                .color(faint(ui)),
        );
    }

    // ---- weights
    section_title(ui, &ctx.t("rig.weights.title"));
    match st
        .selected
        .as_ref()
        .and_then(|b| st.doc.as_ref().and_then(|d| model::bone(d.rig(), b)))
    {
        Some(b) => {
            ui.label(RichText::new(ctx.tf("rig.weights.painting", &[("bone", &b.name)])).small())
        }
        None => ui.label(
            RichText::new(ctx.t("rig.weights.pick_bone"))
                .small()
                .color(weak(ui)),
        ),
    };
    ui.horizontal(|ui| {
        for (mode, key) in [
            (BrushMode::Add, "rig.weights.add"),
            (BrushMode::Subtract, "rig.weights.subtract"),
            (BrushMode::Smooth, "rig.weights.smooth"),
        ] {
            ui.selectable_value(&mut st.mesh.brush, mode, ctx.t(key));
        }
    });
    let me = &mut st.mesh;
    row(ui, ctx.t("rig.weights.radius"), &mut |ui| {
        ui.add(egui::Slider::new(&mut me.radius, 4.0..=300.0).suffix(" px"))
    });
    row(ui, ctx.t("rig.weights.strength"), &mut |ui| {
        ui.add(egui::Slider::new(&mut me.strength, 0.01..=1.0))
    });
    row(ui, ctx.t("rig.weights.falloff"), &mut |ui| {
        ui.add(egui::Slider::new(&mut me.falloff, 0.5..=8.0))
            .on_hover_text(ctx.t("rig.weights.falloff_hint"))
    });
    ui.checkbox(&mut st.mesh.heatmap, ctx.t("rig.weights.heatmap"));
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(ctx.t("rig.weights.auto"))
            .on_hover_text(ctx.t("rig.weights.auto_hint"))
            .clicked()
            && let Some(doc) = st.doc.as_ref()
        {
            let mut mm = m.clone();
            mm.weights = auto_weights_for(doc.rig(), &mm, st.mesh.falloff);
            commit(ctx, st, &t, mm);
        }
        if ui
            .button(ctx.t("rig.weights.rigid"))
            .on_hover_text(ctx.t("rig.weights.rigid_hint"))
            .clicked()
        {
            let mut mm = m.clone();
            mm.weights.clear();
            commit(ctx, st, &t, mm);
        }
        if st.pose_mode {
            if ui.button(ctx.t("rig.reset_pose")).clicked() {
                st.reset_pose();
            }
        } else if ui
            .button(ctx.t("rig.weights.try_pose"))
            .on_hover_text(ctx.t("rig.weights.try_pose_hint"))
            .clicked()
        {
            super::set_pose_mode(st, true);
            st.tool = Tool::Rotate;
        }
    });
    // Vertex inspector: weights of the selected vertex.
    if let (Some(v), Some(doc)) = (st.mesh.selected_vertex, st.doc.as_ref())
        && let Some(list) = m.weights.get(v)
    {
        ui.label(
            RichText::new(ctx.tf("rig.weights.vertex", &[("index", &v)]))
                .small()
                .color(faint(ui)),
        );
        for (b, w) in list {
            let name = model::bone(doc.rig(), b).map_or_else(|| b.to_string(), |x| x.name.clone());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                let [cr, cg, cb] = weights::heat_color(*w);
                ui.painter()
                    .rect_filled(r, 2.0, Color32::from_rgb(cr, cg, cb));
                ui.label(RichText::new(format!("{name}: {:.0} %", w * 100.0)).small());
            });
        }
    }
}
