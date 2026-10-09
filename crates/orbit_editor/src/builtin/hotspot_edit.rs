//! Drawing and editing the zones of a hotspot map (point & click), the pure
//! part: shapes made from drags and clicks, handles, hit tests. The scene
//! view (`sceneview::hotspots`) turns gestures into these calls and the
//! results into undoable `SetNodeKind` commands; the inspector lists the
//! zones. Coordinates are normalized to the screen (0..1).

use orbit_core::hotspot::MAX_POLYGON_POINTS;
use orbit_core::{Hotspot, HotspotMap, HotspotShape, NodeId, StringId};

/// Smallest side of a zone drawn by a drag (a click is not a zone).
pub const MIN_SIZE: f32 = 0.01;

/// The drawing tool of the scene view on a hotspot map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tool {
    /// Select, move, resize, edit vertices.
    #[default]
    Select,
    Rect,
    Ellipse,
    /// Click the vertices; click the first one (or double-click, Enter) to close.
    Polygon,
}

fn clamp01(p: (f32, f32)) -> (f32, f32) {
    (p.0.clamp(0.0, 1.0), p.1.clamp(0.0, 1.0))
}

/// The zone a drag from `a` to `b` draws with `tool` (`None`: too small, or
/// not a drag tool).
pub fn shape_from_drag(tool: Tool, a: (f32, f32), b: (f32, f32)) -> Option<HotspotShape> {
    let (a, b) = (clamp01(a), clamp01(b));
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    if x1 - x0 < MIN_SIZE || y1 - y0 < MIN_SIZE {
        return None;
    }
    match tool {
        Tool::Rect => Some(HotspotShape::Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }),
        Tool::Ellipse => Some(HotspotShape::Ellipse {
            cx: (x0 + x1) * 0.5,
            cy: (y0 + y1) * 0.5,
            rx: (x1 - x0) * 0.5,
            ry: (y1 - y0) * 0.5,
        }),
        Tool::Select | Tool::Polygon => None,
    }
}

/// A polygon being clicked point by point.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolygonDraft {
    pub points: Vec<(f32, f32)>,
}

/// What a click did to a [`PolygonDraft`].
#[derive(Clone, Debug, PartialEq)]
pub enum PolygonStep {
    /// A vertex was added.
    Added,
    /// The polygon is closed: here is the zone.
    Closed(HotspotShape),
    /// Nothing (a click on the last point, or the vertex limit).
    Ignored,
}

impl PolygonDraft {
    /// A click at `p`; `close_radius` is how near the first vertex closes.
    pub fn click(&mut self, p: (f32, f32), close_radius: f32) -> PolygonStep {
        let p = clamp01(p);
        if let Some(&first) = self.points.first()
            && self.points.len() >= 3
            && dist(first, p) <= close_radius
        {
            return self.close().map_or(PolygonStep::Ignored, PolygonStep::Closed);
        }
        if self.points.last().is_some_and(|&l| dist(l, p) <= close_radius * 0.25)
            || self.points.len() >= MAX_POLYGON_POINTS
        {
            return PolygonStep::Ignored;
        }
        self.points.push(p);
        PolygonStep::Added
    }

    /// Closes the polygon (Enter, double-click): `None` under 3 vertices.
    pub fn close(&mut self) -> Option<HotspotShape> {
        if self.points.len() < 3 {
            return None;
        }
        let shape = HotspotShape::Polygon { points: std::mem::take(&mut self.points) };
        shape.is_valid().then_some(shape)
    }

    /// Backspace: drops the last vertex.
    pub fn undo_point(&mut self) {
        self.points.pop();
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// The handles of a shape: the 4 corners of a rect or of an ellipse's box
/// (top-left, top-right, bottom-right, bottom-left), a polygon's vertices.
pub fn handles(shape: &HotspotShape) -> Vec<(f32, f32)> {
    match shape {
        HotspotShape::Polygon { points } => points.clone(),
        _ => {
            let (x0, y0, x1, y1) = shape.bounds();
            vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        }
    }
}

/// `shape` with handle `index` dragged to `p`: a corner resizes against the
/// opposite one, a vertex moves.
pub fn drag_handle(shape: &HotspotShape, index: usize, p: (f32, f32)) -> HotspotShape {
    let p = clamp01(p);
    match shape {
        HotspotShape::Polygon { points } => {
            let mut points = points.clone();
            if let Some(v) = points.get_mut(index) {
                *v = p;
            }
            HotspotShape::Polygon { points }
        }
        _ => {
            let corners = handles(shape);
            let Some(&opposite) = corners.get((index + 2) % 4) else {
                return shape.clone();
            };
            let tool = if matches!(shape, HotspotShape::Ellipse { .. }) {
                Tool::Ellipse
            } else {
                Tool::Rect
            };
            shape_from_drag(tool, opposite, p).unwrap_or_else(|| shape.clone())
        }
    }
}

/// `shape` moved by `(dx, dy)`, kept on the screen.
pub fn moved(shape: &HotspotShape, dx: f32, dy: f32) -> HotspotShape {
    let (x0, y0, x1, y1) = shape.bounds();
    let dx = dx.clamp(-x0, 1.0 - x1);
    let dy = dy.clamp(-y0, 1.0 - y1);
    let mut s = shape.clone();
    s.translate(dx, dy);
    s
}

/// The polygon with a vertex inserted on its edge nearest to `p` (`None`:
/// not a polygon, or full). Returns the new vertex's index too.
pub fn insert_vertex(shape: &HotspotShape, p: (f32, f32)) -> Option<(HotspotShape, usize)> {
    let HotspotShape::Polygon { points } = shape else { return None };
    if points.len() < 2 || points.len() >= MAX_POLYGON_POINTS {
        return None;
    }
    let n = points.len();
    let best = (0..n)
        .map(|i| (segment_distance(p, points[i], points[(i + 1) % n]), i))
        .min_by(|a, b| a.0.total_cmp(&b.0))?
        .1;
    let mut points = points.clone();
    points.insert(best + 1, clamp01(p));
    Some((HotspotShape::Polygon { points }, best + 1))
}

/// The polygon without vertex `index` (`None`: it would have under 3).
pub fn remove_vertex(shape: &HotspotShape, index: usize) -> Option<HotspotShape> {
    let HotspotShape::Polygon { points } = shape else { return None };
    if points.len() <= 3 || index >= points.len() {
        return None;
    }
    let mut points = points.clone();
    points.remove(index);
    Some(HotspotShape::Polygon { points })
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(p, (a.0 + vx * t, a.1 + vy * t))
}

/// What the pointer is over in select mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A handle of the selected zone.
    Handle(usize, usize),
    /// Inside a zone (the topmost).
    Zone(usize),
}

/// The handle of `selected` within `radius` of `p` (handles win), else the
/// topmost zone containing `p`. `radius` is in normalized units along x;
/// `aspect` is width / height of the screen (handles are round on screen).
pub fn pick(map: &HotspotMap, selected: Option<usize>, p: (f32, f32), radius: f32, aspect: f32) -> Option<Pick> {
    if let Some(s) = selected
        && let Some(h) = map.hotspots.get(s)
    {
        let near = handles(&h.shape).iter().position(|&(x, y)| {
            let dx = x - p.0;
            let dy = (y - p.1) / aspect;
            (dx * dx + dy * dy).sqrt() <= radius
        });
        if let Some(i) = near {
            return Some(Pick::Handle(s, i));
        }
    }
    let all: Vec<usize> = (0..map.hotspots.len()).collect();
    map.hit(&all, p.0, p.1).map(Pick::Zone)
}

/// A new zone of `shape` leading to `target`, with its tooltip string.
pub fn new_zone(shape: HotspotShape, target: NodeId) -> Hotspot {
    Hotspot {
        name: String::new(),
        text_id: StringId::new(),
        shape,
        target,
        condition: None,
        enabled: None,
        style: None,
        hover_image: None,
        hover_sound: None,
    }
}

/// The zone selected in the editor for map `node` (shared by the scene view
/// and the inspector through egui's memory).
pub fn selected_zone(ctx: &egui::Context, node: NodeId) -> Option<usize> {
    ctx.data(|d| d.get_temp::<usize>(selection_id(node)))
}

pub fn select_zone(ctx: &egui::Context, node: NodeId, zone: Option<usize>) {
    ctx.data_mut(|d| match zone {
        Some(z) => {
            d.insert_temp(selection_id(node), z);
        }
        None => {
            d.remove::<usize>(selection_id(node));
        }
    });
}

fn selection_id(node: NodeId) -> egui::Id {
    egui::Id::new(("orbit_hotspot_zone", node.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drags_draw_rects_and_ellipses_in_any_direction() {
        let r = shape_from_drag(Tool::Rect, (0.5, 0.6), (0.2, 0.1)).unwrap();
        assert_eq!(r, HotspotShape::Rect { x: 0.2, y: 0.1, w: 0.3, h: 0.5 });
        let e = shape_from_drag(Tool::Ellipse, (0.2, 0.2), (0.4, 0.3)).unwrap();
        let HotspotShape::Ellipse { cx, cy, rx, ry } = e else { panic!() };
        assert!((cx - 0.3).abs() < 1e-6 && (cy - 0.25).abs() < 1e-6);
        assert!((rx - 0.1).abs() < 1e-6 && (ry - 0.05).abs() < 1e-6);
        // A click is not a zone; off-screen is clamped.
        assert!(shape_from_drag(Tool::Rect, (0.5, 0.5), (0.502, 0.6)).is_none());
        let clamped = shape_from_drag(Tool::Rect, (-0.5, 0.5), (0.2, 1.5)).unwrap();
        assert_eq!(clamped.bounds(), (0.0, 0.5, 0.2, 1.0));
        assert!(shape_from_drag(Tool::Select, (0.0, 0.0), (1.0, 1.0)).is_none());
    }

    #[test]
    fn polygons_are_clicked_and_closed_on_the_first_point() {
        let mut d = PolygonDraft::default();
        assert_eq!(d.click((0.1, 0.1), 0.02), PolygonStep::Added);
        assert_eq!(d.click((0.1, 0.1), 0.02), PolygonStep::Ignored, "same point twice");
        // Closing needs 3 points: near the first one it is just a vertex... too near: ignored
        assert_eq!(d.click((0.5, 0.1), 0.02), PolygonStep::Added);
        assert_eq!(d.click((0.3, 0.4), 0.02), PolygonStep::Added);
        d.undo_point();
        assert_eq!(d.points.len(), 2);
        assert_eq!(d.click((0.3, 0.5), 0.02), PolygonStep::Added);
        let PolygonStep::Closed(shape) = d.click((0.11, 0.1), 0.02) else {
            panic!("closes on the first vertex")
        };
        assert!(shape.contains(0.3, 0.2));
        assert!(d.points.is_empty());
        let mut two = PolygonDraft { points: vec![(0.0, 0.0), (1.0, 1.0)] };
        assert!(two.close().is_none());
    }

    #[test]
    fn handles_resize_and_edit_vertices() {
        let r = HotspotShape::Rect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 };
        // Bottom-right corner dragged: top-left stays.
        assert_eq!(drag_handle(&r, 2, (0.5, 0.4)), HotspotShape::Rect { x: 0.1, y: 0.1, w: 0.4, h: 0.3 });
        // Past the opposite corner: the rect flips, still valid.
        assert_eq!(drag_handle(&r, 2, (0.0, 0.0)).bounds(), (0.0, 0.0, 0.1, 0.1));
        let e = HotspotShape::Ellipse { cx: 0.5, cy: 0.5, rx: 0.1, ry: 0.1 };
        let HotspotShape::Ellipse { cx, rx, .. } = drag_handle(&e, 1, (0.8, 0.4)) else { panic!() };
        assert!((cx - 0.6).abs() < 1e-6 && (rx - 0.2).abs() < 1e-6);
        let tri = HotspotShape::Polygon { points: vec![(0.0, 0.0), (0.4, 0.0), (0.2, 0.4)] };
        let HotspotShape::Polygon { points } = drag_handle(&tri, 2, (0.2, 0.6)) else { panic!() };
        assert_eq!(points[2], (0.2, 0.6));
        let (quad, i) = insert_vertex(&tri, (0.2, -0.01)).unwrap();
        assert_eq!(i, 1, "on the top edge");
        assert!(remove_vertex(&quad, i).is_some());
        assert!(remove_vertex(&tri, 0).is_none(), "a triangle keeps its 3 vertices");
        assert!(insert_vertex(&r, (0.0, 0.0)).is_none());
    }

    #[test]
    fn moves_stay_on_screen_and_picks_prefer_handles() {
        let r = HotspotShape::Rect { x: 0.8, y: 0.1, w: 0.1, h: 0.1 };
        assert_eq!(moved(&r, 0.5, -0.5).bounds(), (0.9, 0.0, 1.0, 0.1));
        let map = HotspotMap {
            hotspots: vec![
                new_zone(HotspotShape::Rect { x: 0.0, y: 0.0, w: 0.5, h: 0.5 }, NodeId::new()),
                new_zone(HotspotShape::Rect { x: 0.4, y: 0.4, w: 0.2, h: 0.2 }, NodeId::new()),
            ],
            ..Default::default()
        };
        assert_eq!(pick(&map, None, (0.45, 0.45), 0.01, 16.0 / 9.0), Some(Pick::Zone(1)));
        assert_eq!(pick(&map, Some(0), (0.5, 0.5), 0.01, 16.0 / 9.0), Some(Pick::Handle(0, 2)));
        assert_eq!(pick(&map, Some(0), (0.9, 0.9), 0.01, 16.0 / 9.0), None);
    }
}
