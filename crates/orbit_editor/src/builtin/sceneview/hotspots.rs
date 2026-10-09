//! The scene view on a hotspot map: the zones drawn over the stage, and the
//! tools to draw them (drag a rectangle or an ellipse, click a polygon's
//! vertices) and edit them (move, resize by the corners, drag a vertex,
//! double-click an edge to add one, right-click a vertex to remove it,
//! Delete removes the zone). Each gesture is one undo entry; the geometry
//! lives in `hotspot_edit` (pure, tested).

use egui::{
    Align2, Color32, CursorIcon, FontId, Id, Key, Pos2, Rect, Response, Shape, Stroke, Ui,
    pos2, vec2,
};
use orbit_core::{HotspotMap, HotspotShape, NodeId, NodeKind};
use orbit_editor_api::{EditorContext, Selection, UiText, icons};
use orbit_project::Command;

use super::super::hotspot_edit::{
    self as edit, Pick, PolygonDraft, PolygonStep, Tool, select_zone, selected_zone,
};
use super::super::inspector_hotspots::{add_zone, remove_zone};
use super::super::ui_util::accent;

/// Pick radius of a handle, in screen points.
const HANDLE_RADIUS: f32 = 7.0;

/// The selected node when it is a hotspot map, with the map.
pub(super) fn selected_map(ctx: &EditorContext<'_>) -> Option<(NodeId, HotspotMap)> {
    let Selection::Node(id) = ctx.selection()? else { return None };
    match &ctx.story()?.node(id)?.kind {
        NodeKind::Hotspots(map) => Some((id, map.clone())),
        _ => None,
    }
}

fn tool_id() -> Id {
    Id::new("orbit_hotspot_tool")
}

fn draft_id() -> Id {
    Id::new("orbit_hotspot_draft")
}

fn gesture_id() -> Id {
    Id::new("orbit_hotspot_gesture")
}

fn tool(ui: &Ui) -> Tool {
    ui.data(|d| d.get_temp(tool_id())).unwrap_or_default()
}

fn set_tool(ui: &Ui, t: Tool) {
    ui.data_mut(|d| {
        d.insert_temp(tool_id(), t);
        d.remove::<PolygonDraft>(draft_id());
    });
}

/// A pointer gesture in progress.
#[derive(Clone, Debug)]
enum Gesture {
    /// Drawing a rect / ellipse from `start` (normalized).
    Draw { start: (f32, f32), now: (f32, f32) },
    /// Moving zone `zone`, grabbed at `start`.
    Move { zone: usize, start: (f32, f32), orig: HotspotShape },
    /// Dragging handle `handle` of zone `zone`.
    Handle { zone: usize, handle: usize, orig: HotspotShape },
}

/// The tool row shown above the picture on a hotspot map.
pub(super) fn toolbar(ui: &mut Ui, ctx: &mut EditorContext<'_>, node: NodeId) {
    let current = tool(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(ui.t("sceneview.hotspots.tools")).small());
        for (t, icon, key) in [
            (Tool::Select, icons::CURSOR, "sceneview.hotspots.select"),
            (Tool::Rect, icons::RECTANGLE, "sceneview.hotspots.rect"),
            (Tool::Ellipse, icons::CIRCLE, "sceneview.hotspots.ellipse"),
            (Tool::Polygon, icons::POLYGON, "sceneview.hotspots.polygon"),
        ] {
            if ui
                .selectable_label(current == t, format!("{icon}  {}", ui.t(key)))
                .on_hover_text(ui.t(&format!("{key}_tip")))
                .clicked()
            {
                set_tool(ui, t);
            }
        }
        ui.separator();
        let selected = selected_zone(ui.ctx(), node);
        if ui
            .add_enabled(
                selected.is_some(),
                egui::Button::new(format!("{}  {}", icons::TRASH, ui.t("sceneview.hotspots.delete"))),
            )
            .clicked()
            && let Some(z) = selected
        {
            remove_zone(ctx, &ui.ctx().clone(), node, z);
        }
    });
}

struct Frame {
    picture: Rect,
}

impl Frame {
    fn norm(&self, p: Pos2) -> (f32, f32) {
        (
            (p.x - self.picture.left()) / self.picture.width(),
            (p.y - self.picture.top()) / self.picture.height(),
        )
    }

    fn screen(&self, (x, y): (f32, f32)) -> Pos2 {
        pos2(
            self.picture.left() + x * self.picture.width(),
            self.picture.top() + y * self.picture.height(),
        )
    }
}

/// Applies `map` to the node live: every frame of a gesture merges into
/// one undo entry (ended by `end_merge` when the gesture stops).
fn live(ctx: &mut EditorContext<'_>, node: NodeId, map: HotspotMap) {
    let command = Command::SetNodeKind {
        id: node,
        kind: NodeKind::Hotspots(map),
        expected_version: ctx.node_version(node),
    };
    if let Err(e) = ctx.exec_merged(command, &format!("hotspot-drag:{node}")) {
        ctx.error(e.to_string());
    }
}

fn with_shape(map: &HotspotMap, zone: usize, shape: HotspotShape) -> HotspotMap {
    let mut m = map.clone();
    if let Some(z) = m.hotspots.get_mut(zone) {
        z.shape = shape;
    }
    m
}

/// Pointer and keyboard on the picture, then the overlay.
pub(super) fn interact_and_paint(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    response: &Response,
    picture: Rect,
    node: NodeId,
    map: &HotspotMap,
) {
    let frame = Frame { picture };
    let egui_ctx = ui.ctx().clone();
    let tool = tool(ui);
    let mut draft: PolygonDraft = ui.data(|d| d.get_temp(draft_id())).unwrap_or_default();
    let mut gesture: Option<Gesture> = ui.data(|d| d.get_temp(gesture_id()));
    let mut selected = selected_zone(&egui_ctx, node).filter(|&z| z < map.hotspots.len());
    let radius = HANDLE_RADIUS / picture.width().max(1.0);
    let aspect = picture.width() / picture.height().max(1.0);
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos())
        .map(|p| frame.norm(p));

    // ---- pointer
    if response.drag_started()
        && let Some(p) = pointer
    {
        gesture = match tool {
            Tool::Rect | Tool::Ellipse => Some(Gesture::Draw { start: p, now: p }),
            Tool::Polygon => None,
            Tool::Select => match edit::pick(map, selected, p, radius, aspect) {
                Some(Pick::Handle(zone, handle)) => Some(Gesture::Handle {
                    zone,
                    handle,
                    orig: map.hotspots[zone].shape.clone(),
                }),
                Some(Pick::Zone(zone)) => {
                    selected = Some(zone);
                    Some(Gesture::Move { zone, start: p, orig: map.hotspots[zone].shape.clone() })
                }
                None => {
                    selected = None;
                    None
                }
            },
        };
    }
    if response.dragged()
        && let (Some(p), Some(g)) = (pointer, gesture.as_mut())
    {
        match g {
            Gesture::Draw { now, .. } => *now = p,
            Gesture::Move { zone, start, orig } => {
                let shape = edit::moved(orig, p.0 - start.0, p.1 - start.1);
                if map.hotspots.get(*zone).is_some_and(|z| z.shape != shape) {
                    live(ctx, node, with_shape(map, *zone, shape));
                }
            }
            Gesture::Handle { zone, handle, orig } => {
                let shape = edit::drag_handle(orig, *handle, p);
                if map.hotspots.get(*zone).is_some_and(|z| z.shape != shape) {
                    live(ctx, node, with_shape(map, *zone, shape));
                }
            }
        }
    }
    if response.drag_stopped() {
        match gesture.take() {
            Some(Gesture::Draw { start, now }) => {
                if let Some(shape) = edit::shape_from_drag(tool, start, now) {
                    add_zone(ctx, &egui_ctx, node, shape);
                    selected = selected_zone(&egui_ctx, node);
                    set_tool(ui, Tool::Select);
                }
            }
            Some(_) => ctx.end_merge(),
            None => {}
        }
    }
    if response.clicked()
        && let Some(p) = pointer
    {
        match tool {
            Tool::Polygon => match draft.click(p, radius) {
                PolygonStep::Closed(shape) => {
                    add_zone(ctx, &egui_ctx, node, shape);
                    selected = selected_zone(&egui_ctx, node);
                    set_tool(ui, Tool::Select);
                    draft = PolygonDraft::default();
                }
                PolygonStep::Added | PolygonStep::Ignored => {}
            },
            Tool::Select => {
                selected = match edit::pick(map, selected, p, radius, aspect) {
                    Some(Pick::Handle(z, _) | Pick::Zone(z)) => Some(z),
                    None => None,
                };
            }
            Tool::Rect | Tool::Ellipse => {}
        }
    }
    if response.double_clicked()
        && let Some(p) = pointer
    {
        match tool {
            Tool::Polygon => {
                if let Some(shape) = draft.close() {
                    add_zone(ctx, &egui_ctx, node, shape);
                    selected = selected_zone(&egui_ctx, node);
                    set_tool(ui, Tool::Select);
                }
            }
            Tool::Select => {
                if let Some(z) = selected
                    && let Some((shape, _)) = edit::insert_vertex(&map.hotspots[z].shape, p)
                {
                    crate::builtin::inspector_hotspots::commit_map(ctx, node, with_shape(map, z, shape), "vertex");
                    ctx.end_merge();
                }
            }
            Tool::Rect | Tool::Ellipse => {}
        }
    }
    if response.secondary_clicked()
        && tool == Tool::Select
        && let Some(p) = pointer
        && let Some(Pick::Handle(z, h)) = edit::pick(map, selected, p, radius, aspect)
        && let Some(shape) = edit::remove_vertex(&map.hotspots[z].shape, h)
    {
        crate::builtin::inspector_hotspots::commit_map(ctx, node, with_shape(map, z, shape), "vertex");
        ctx.end_merge();
    }

    // ---- keyboard (when the pointer is over the view or it has the focus)
    if response.hovered() || response.has_focus() {
        let (enter, escape, back, delete) = ui.input(|i| {
            (
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Delete),
            )
        });
        if tool == Tool::Polygon && enter && let Some(shape) = draft.close() {
            add_zone(ctx, &egui_ctx, node, shape);
            selected = selected_zone(&egui_ctx, node);
            set_tool(ui, Tool::Select);
        }
        if tool == Tool::Polygon && back {
            draft.undo_point();
        }
        if escape {
            if gesture.take().is_some() {
                ctx.end_merge();
            }
            draft = PolygonDraft::default();
            set_tool(ui, Tool::Select);
        }
        if delete
            && tool == Tool::Select
            && let Some(z) = selected
        {
            remove_zone(ctx, &egui_ctx, node, z);
            selected = None;
        }
    }

    // ---- cursor
    if response.hovered() {
        let icon = match tool {
            Tool::Rect | Tool::Ellipse | Tool::Polygon => CursorIcon::Crosshair,
            Tool::Select => match (gesture.as_ref(), pointer.and_then(|p| edit::pick(map, selected, p, radius, aspect))) {
                (Some(Gesture::Move { .. }), _) => CursorIcon::Grabbing,
                (Some(Gesture::Handle { .. }), _) | (_, Some(Pick::Handle(..))) => CursorIcon::Crosshair,
                (_, Some(Pick::Zone(_))) => CursorIcon::Grab,
                _ => CursorIcon::Default,
            },
        };
        ui.ctx().set_cursor_icon(icon);
    }

    select_zone(&egui_ctx, node, selected);
    ui.data_mut(|d| {
        match &gesture {
            Some(g) => {
                d.insert_temp(gesture_id(), g.clone());
            }
            None => {
                d.remove::<Gesture>(gesture_id());
            }
        }
        d.insert_temp(draft_id(), draft.clone());
    });

    // ---- overlay (from the story as it is now, after this frame's edits)
    let map = ctx
        .story()
        .and_then(|s| s.node(node))
        .and_then(|n| match &n.kind {
            NodeKind::Hotspots(m) => Some(m.clone()),
            _ => None,
        })
        .unwrap_or_else(|| map.clone());
    let story_texts: Vec<String> = ctx.story().map_or_else(Vec::new, |s| {
        map.hotspots
            .iter()
            .map(|h| if h.name.is_empty() { s.text(h.text_id).unwrap_or("").to_owned() } else { h.name.clone() })
            .collect()
    });
    paint(ui, &frame, &map, selected, &story_texts, gesture.as_ref(), &draft, tool, pointer);
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &Ui,
    frame: &Frame,
    map: &HotspotMap,
    selected: Option<usize>,
    names: &[String],
    gesture: Option<&Gesture>,
    draft: &PolygonDraft,
    tool: Tool,
    pointer: Option<(f32, f32)>,
) {
    let painter = ui.painter_at(frame.picture);
    let accent = accent(ui);
    let c = map.style.color;
    let zone_color = Color32::from_rgb(c.r, c.g, c.b);
    for (i, h) in map.hotspots.iter().enumerate() {
        let is_sel = selected == Some(i);
        let outline: Vec<Pos2> = h.shape.outline(48).into_iter().map(|p| frame.screen(p)).collect();
        fill(&painter, &outline, zone_color.gamma_multiply(if is_sel { 0.28 } else { 0.14 }));
        let stroke = if is_sel {
            Stroke::new(2.0, accent)
        } else {
            Stroke::new(1.5, zone_color.gamma_multiply(0.9))
        };
        painter.add(Shape::closed_line(outline.clone(), stroke));
        let (cx, cy) = h.shape.center();
        let label = names.get(i).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("#{}", i + 1));
        let at = frame.screen((cx, cy));
        painter.text(at + vec2(1.0, 1.0), Align2::CENTER_CENTER, &label, FontId::proportional(13.0), Color32::from_black_alpha(200));
        painter.text(at, Align2::CENTER_CENTER, &label, FontId::proportional(13.0), Color32::WHITE);
        if is_sel {
            for p in edit::handles(&h.shape) {
                let r = Rect::from_center_size(frame.screen(p), vec2(9.0, 9.0));
                painter.rect_filled(r, 2.0, Color32::WHITE);
                painter.rect_stroke(r, 2.0, Stroke::new(1.5, accent), egui::StrokeKind::Inside);
            }
        }
    }
    // Shape being drawn.
    if let Some(Gesture::Draw { start, now }) = gesture
        && let Some(shape) = edit::shape_from_drag(tool, *start, *now)
    {
        let pts: Vec<Pos2> = shape.outline(48).into_iter().map(|p| frame.screen(p)).collect();
        fill(&painter, &pts, accent.gamma_multiply(0.2));
        painter.add(Shape::closed_line(pts, Stroke::new(2.0, accent)));
    }
    if tool == Tool::Polygon && !draft.points.is_empty() {
        let mut pts: Vec<Pos2> = draft.points.iter().map(|&p| frame.screen(p)).collect();
        if let Some(p) = pointer {
            pts.push(frame.screen(p));
        }
        painter.add(Shape::line(pts, Stroke::new(2.0, accent)));
        for (k, &p) in draft.points.iter().enumerate() {
            let r = if k == 0 && draft.points.len() >= 3 { 6.0 } else { 4.0 };
            painter.circle_filled(frame.screen(p), r, accent);
        }
    }
    let hint = match tool {
        Tool::Select if map.hotspots.is_empty() => Some("sceneview.hotspots.hint_empty"),
        Tool::Select => None,
        Tool::Rect | Tool::Ellipse => Some("sceneview.hotspots.hint_drag"),
        Tool::Polygon => Some("sceneview.hotspots.hint_polygon"),
    };
    if let Some(key) = hint {
        let at = pos2(frame.picture.center().x, frame.picture.bottom() - 14.0);
        let text = ui.t(key);
        let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), Color32::WHITE);
        let r = Rect::from_center_size(at, galley.size() + vec2(16.0, 8.0));
        painter.rect_filled(r, 4.0, Color32::from_black_alpha(170));
        painter.galley(r.min + vec2(8.0, 4.0), galley, Color32::WHITE);
    }
}

/// A filled polygon (concave too), triangulated like the player does.
fn fill(painter: &egui::Painter, pts: &[Pos2], color: Color32) {
    let tris = orbit_core::hotspot::triangulate(&pts.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>());
    if tris.is_empty() {
        return;
    }
    let mut mesh = egui::Mesh::default();
    for p in pts {
        mesh.colored_vertex(*p, color);
    }
    mesh.indices = tris;
    painter.add(Shape::mesh(mesh));
}

