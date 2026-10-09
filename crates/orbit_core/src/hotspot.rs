//! Point & click navigation (imagemap): clickable zones drawn over the
//! background, presented by a [`NodeKind::Hotspots`](crate::NodeKind) node.
//!
//! For the VM a hotspot map is a choice: every hotspot is an option (its
//! `text_id` is the tooltip, `target` a `Label`), so waiting, saves,
//! rollback, skip and replay reuse the choice machinery unchanged. Only the
//! host draws it differently: zones over the picture instead of a menu.
//!
//! Coordinates are normalized to the virtual screen (`0..1` on both axes,
//! origin top-left), so a map survives a resolution change. Hit testing is
//! pure geometry, cheap enough to run every frame on every hotspot.

use serde::{Deserialize, Serialize};

use crate::defs::Color;
use crate::expr::Expr;
use crate::ids::{AssetId, NodeId, StringId};

/// Maximum vertices of a polygon hotspot.
pub const MAX_POLYGON_POINTS: usize = 64;

/// The data of a `Hotspots` node.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HotspotMap {
    /// Background shown when the node runs (as a `Background` node without
    /// transition). `None` keeps the current background.
    #[serde(default)]
    pub background: Option<AssetId>,
    /// The zones, drawn back to front (the last one wins a click where
    /// zones overlap).
    #[serde(default)]
    pub hotspots: Vec<Hotspot>,
    /// Look shared by every hotspot (each can override it).
    #[serde(default)]
    pub style: HotspotStyle,
}

/// One clickable zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Hotspot {
    /// Name for the author (editor list, DSL); not shown to the player.
    #[serde(default)]
    pub name: String,
    /// Tooltip shown on hover / focus (localizable; may be an empty string).
    pub text_id: StringId,
    pub shape: HotspotShape,
    /// The `Label` the story jumps to when the zone is activated.
    pub target: NodeId,
    /// The zone exists only when this evaluates to `true`.
    #[serde(default)]
    pub condition: Option<Expr>,
    /// When this evaluates to `false` the zone is shown dimmed and cannot be
    /// activated.
    #[serde(default)]
    pub enabled: Option<Expr>,
    /// Overrides the map style.
    #[serde(default)]
    pub style: Option<HotspotStyle>,
    /// Image (same frame as the screen) drawn clipped to the zone on hover,
    /// instead of the procedural glow (Ren'Py imagemap `hover`).
    #[serde(default)]
    pub hover_image: Option<AssetId>,
    /// Sound played when the zone gets the hover / focus.
    #[serde(default)]
    pub hover_sound: Option<AssetId>,
}

/// The geometry of a hotspot, in normalized screen coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum HotspotShape {
    Rect { x: f32, y: f32, w: f32, h: f32 },
    Ellipse { cx: f32, cy: f32, rx: f32, ry: f32 },
    Polygon { points: Vec<(f32, f32)> },
}

/// How hotspots look. Colors are RGB; alphas go from 0 to 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HotspotStyle {
    #[serde(default = "HotspotStyle::default_color")]
    pub color: Color,
    /// Opacity of the fill tint on hover.
    #[serde(default = "HotspotStyle::default_fill")]
    pub fill_alpha: f32,
    /// Width of the glowing outline, in virtual pixels.
    #[serde(default = "HotspotStyle::default_glow")]
    pub glow_width: f32,
    /// Opacity of the outline on hover (0..1).
    #[serde(default = "HotspotStyle::default_intensity")]
    pub intensity: f32,
    /// Pulses per second of the outline (0: steady).
    #[serde(default = "HotspotStyle::default_pulse")]
    pub pulse_speed: f32,
    /// Opacity of the faint shimmering edge drawn when idle (0: invisible
    /// until hovered).
    #[serde(default = "HotspotStyle::default_idle")]
    pub idle_hint: f32,
    /// The mouse cursor becomes a hand over the zone.
    #[serde(default = "HotspotStyle::default_true")]
    pub hand_cursor: bool,
}

impl HotspotStyle {
    fn default_color() -> Color {
        Color { r: 255, g: 230, b: 150 }
    }
    fn default_fill() -> f32 {
        0.18
    }
    fn default_glow() -> f32 {
        4.0
    }
    fn default_intensity() -> f32 {
        0.9
    }
    fn default_pulse() -> f32 {
        1.2
    }
    fn default_idle() -> f32 {
        0.12
    }
    fn default_true() -> bool {
        true
    }
}

impl Default for HotspotStyle {
    fn default() -> Self {
        Self {
            color: Self::default_color(),
            fill_alpha: Self::default_fill(),
            glow_width: Self::default_glow(),
            intensity: Self::default_intensity(),
            pulse_speed: Self::default_pulse(),
            idle_hint: Self::default_idle(),
            hand_cursor: true,
        }
    }
}

impl HotspotShape {
    /// Is the normalized point `(x, y)` inside the shape?
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            HotspotShape::Rect { x: rx, y: ry, w, h } => {
                let (x0, x1) = ordered(*rx, rx + w);
                let (y0, y1) = ordered(*ry, ry + h);
                x >= x0 && x <= x1 && y >= y0 && y <= y1
            }
            HotspotShape::Ellipse { cx, cy, rx, ry } => {
                if *rx <= 0.0 || *ry <= 0.0 {
                    return false;
                }
                let dx = (x - cx) / rx;
                let dy = (y - cy) / ry;
                dx * dx + dy * dy <= 1.0
            }
            HotspotShape::Polygon { points } => point_in_polygon(points, x, y),
        }
    }

    /// Bounding box `(min_x, min_y, max_x, max_y)`.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        match self {
            HotspotShape::Rect { x, y, w, h } => {
                let (x0, x1) = ordered(*x, x + w);
                let (y0, y1) = ordered(*y, y + h);
                (x0, y0, x1, y1)
            }
            HotspotShape::Ellipse { cx, cy, rx, ry } => {
                (cx - rx.abs(), cy - ry.abs(), cx + rx.abs(), cy + ry.abs())
            }
            HotspotShape::Polygon { points } => points.iter().fold(
                (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
                |(a, b, c, d), &(x, y)| (a.min(x), b.min(y), c.max(x), d.max(y)),
            ),
        }
    }

    /// Center of the bounding box (keyboard focus order, tooltip anchor).
    pub fn center(&self) -> (f32, f32) {
        let (a, b, c, d) = self.bounds();
        ((a + c) * 0.5, (b + d) * 0.5)
    }

    /// The outline as a closed polyline (`segments` points for an ellipse),
    /// for drawing the glow.
    pub fn outline(&self, segments: usize) -> Vec<(f32, f32)> {
        match self {
            HotspotShape::Rect { .. } => {
                let (x0, y0, x1, y1) = self.bounds();
                vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            }
            HotspotShape::Ellipse { cx, cy, rx, ry } => {
                let n = segments.max(8);
                (0..n)
                    .map(|i| {
                        let a = i as f32 / n as f32 * std::f32::consts::TAU;
                        (cx + rx * a.cos(), cy + ry * a.sin())
                    })
                    .collect()
            }
            HotspotShape::Polygon { points } => points.clone(),
        }
    }

    /// Moves the shape by `(dx, dy)`.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            HotspotShape::Rect { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            HotspotShape::Ellipse { cx, cy, .. } => {
                *cx += dx;
                *cy += dy;
            }
            HotspotShape::Polygon { points } => {
                for p in points {
                    p.0 += dx;
                    p.1 += dy;
                }
            }
        }
    }

    /// The shape is usable: finite numbers, a non-empty area, a polygon of
    /// 3 to [`MAX_POLYGON_POINTS`] vertices.
    pub fn is_valid(&self) -> bool {
        match self {
            HotspotShape::Rect { x, y, w, h } => {
                [x, y, w, h].iter().all(|v| v.is_finite()) && *w != 0.0 && *h != 0.0
            }
            HotspotShape::Ellipse { cx, cy, rx, ry } => {
                [cx, cy, rx, ry].iter().all(|v| v.is_finite()) && *rx > 0.0 && *ry > 0.0
            }
            HotspotShape::Polygon { points } => {
                (3..=MAX_POLYGON_POINTS).contains(&points.len())
                    && points.iter().all(|(x, y)| x.is_finite() && y.is_finite())
            }
        }
    }
}

fn ordered(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Even-odd ray casting.
fn point_in_polygon(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (xi, yi) = points[i];
        let (xj, yj) = points[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl HotspotMap {
    /// Every asset the map uses: its background, hover images and sounds.
    pub fn assets(&self) -> Vec<AssetId> {
        self.background
            .into_iter()
            .chain(self.hotspots.iter().flat_map(|h| h.hover_image.into_iter().chain(h.hover_sound)))
            .collect()
    }

    /// The topmost hotspot among `candidates` (indices into `hotspots`)
    /// containing the point: the last one in drawing order.
    pub fn hit(&self, candidates: &[usize], x: f32, y: f32) -> Option<usize> {
        candidates
            .iter()
            .rev()
            .copied()
            .find(|&i| self.hotspots.get(i).is_some_and(|h| h.shape.contains(x, y)))
    }

    /// The effective style of hotspot `i`.
    pub fn style_of(&self, i: usize) -> &HotspotStyle {
        self.hotspots
            .get(i)
            .and_then(|h| h.style.as_ref())
            .unwrap_or(&self.style)
    }

    /// `candidates` in keyboard / gamepad focus order: reading order (top to
    /// bottom by rows, then left to right) of their centers.
    pub fn focus_order(&self, candidates: &[usize]) -> Vec<usize> {
        let mut order: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|&i| i < self.hotspots.len())
            .collect();
        order.sort_by(|&a, &b| {
            let (ax, ay) = self.hotspots[a].shape.center();
            let (bx, by) = self.hotspots[b].shape.center();
            // Same row when the centers are within 5% of the height.
            let row = |y: f32| (y / 0.05).floor() as i32;
            row(ay)
                .cmp(&row(by))
                .then(ax.total_cmp(&bx))
                .then(a.cmp(&b))
        });
        order
    }

    /// The hotspot focused after `current` (`forward`) or before it in
    /// [`focus_order`](Self::focus_order), wrapping around. With no current
    /// focus: the first (or last) one.
    pub fn next_focus(&self, candidates: &[usize], current: Option<usize>, forward: bool) -> Option<usize> {
        let order = self.focus_order(candidates);
        if order.is_empty() {
            return None;
        }
        let pos = current.and_then(|c| order.iter().position(|&i| i == c));
        let n = order.len();
        let next = match (pos, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(p), true) => (p + 1) % n,
            (Some(p), false) => (p + n - 1) % n,
        };
        Some(order[next])
    }

    /// The nearest hotspot from `current` in a direction (arrow keys /
    /// D-pad): `(dx, dy)` is the unit direction. Falls back to
    /// [`next_focus`](Self::next_focus) when nothing lies that way.
    pub fn directional_focus(
        &self,
        candidates: &[usize],
        current: Option<usize>,
        dx: f32,
        dy: f32,
    ) -> Option<usize> {
        let Some(cur) = current.filter(|c| candidates.contains(c)) else {
            return self.next_focus(candidates, None, dx + dy >= 0.0);
        };
        let (cx, cy) = self.hotspots[cur].shape.center();
        candidates
            .iter()
            .copied()
            .filter(|&i| i != cur && i < self.hotspots.len())
            .filter_map(|i| {
                let (x, y) = self.hotspots[i].shape.center();
                let (vx, vy) = (x - cx, y - cy);
                let along = vx * dx + vy * dy;
                if along <= 1e-4 {
                    return None;
                }
                let across = (vx * dy - vy * dx).abs();
                Some((along + 2.0 * across, i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
            .or_else(|| self.next_focus(candidates, Some(cur), dx + dy >= 0.0))
    }
}

/// Triangles (indices into `points`) filling a simple polygon, by ear
/// clipping (concave shapes work; O(n²), fine for a few dozen vertices).
/// A degenerate or self-intersecting outline gives what it can.
pub fn triangulate(points: &[(f32, f32)]) -> Vec<u32> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    let ccw = area > 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity((n - 2) * 3);
    let cross = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (points[ia], points[ib], points[ic]);
            let turn = cross(a, b, c);
            if (turn > 0.0) != ccw || turn == 0.0 {
                continue; // reflex or flat corner
            }
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = points[j];
                let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
                if ccw { d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0 } else { d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0 }
            });
            if inside {
                continue;
            }
            out.extend([ia as u32, ib as u32, ic as u32]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            // Self-intersecting: fan the rest.
            break;
        }
    }
    for k in 1..idx.len().saturating_sub(1) {
        out.extend([idx[0] as u32, idx[k] as u32, idx[k + 1] as u32]);
    }
    out
}

/// Pulse factor in `0..1` of an outline at `time` seconds.
pub fn pulse(style: &HotspotStyle, time: f64) -> f32 {
    if style.pulse_speed <= 0.0 {
        return 1.0;
    }
    let phase = (time * style.pulse_speed as f64 * std::f64::consts::TAU).sin() as f32;
    0.7 + 0.3 * phase
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(shape: HotspotShape) -> Hotspot {
        Hotspot {
            name: String::new(),
            text_id: StringId::new(),
            shape,
            target: NodeId::new(),
            condition: None,
            enabled: None,
            style: None,
            hover_image: None,
            hover_sound: None,
        }
    }

    #[test]
    fn rect_hit_including_negative_size() {
        let r = HotspotShape::Rect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 };
        assert!(r.contains(0.2, 0.2));
        assert!(r.contains(0.1, 0.3));
        assert!(!r.contains(0.31, 0.2));
        let flipped = HotspotShape::Rect { x: 0.3, y: 0.3, w: -0.2, h: -0.2 };
        assert!(flipped.contains(0.2, 0.2));
    }

    #[test]
    fn ellipse_hit() {
        let e = HotspotShape::Ellipse { cx: 0.5, cy: 0.5, rx: 0.2, ry: 0.1 };
        assert!(e.contains(0.5, 0.5));
        assert!(e.contains(0.69, 0.5));
        assert!(!e.contains(0.5, 0.61));
        assert!(!e.contains(0.65, 0.58)); // inside the box, outside the ellipse
        let flat = HotspotShape::Ellipse { cx: 0.5, cy: 0.5, rx: 0.0, ry: 0.1 };
        assert!(!flat.contains(0.5, 0.5));
    }

    #[test]
    fn polygon_hit_concave() {
        // An "L": the notch at the top right is outside.
        let l = HotspotShape::Polygon {
            points: vec![(0.0, 0.0), (0.2, 0.0), (0.2, 0.3), (0.5, 0.3), (0.5, 0.5), (0.0, 0.5)],
        };
        assert!(l.contains(0.1, 0.1));
        assert!(l.contains(0.4, 0.4));
        assert!(!l.contains(0.4, 0.1));
        assert!(!l.contains(0.6, 0.4));
        let degenerate = HotspotShape::Polygon { points: vec![(0.0, 0.0), (1.0, 1.0)] };
        assert!(!degenerate.contains(0.5, 0.5));
        assert!(!degenerate.is_valid());
    }

    #[test]
    fn topmost_visible_wins() {
        let map = HotspotMap {
            hotspots: vec![
                spot(HotspotShape::Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }),
                spot(HotspotShape::Rect { x: 0.4, y: 0.4, w: 0.2, h: 0.2 }),
            ],
            ..Default::default()
        };
        assert_eq!(map.hit(&[0, 1], 0.5, 0.5), Some(1));
        // Hidden by a condition: not a candidate, the zone under it gets it.
        assert_eq!(map.hit(&[0], 0.5, 0.5), Some(0));
        assert_eq!(map.hit(&[1], 0.1, 0.1), None);
    }

    #[test]
    fn focus_cycles_in_reading_order() {
        let at = |x: f32, y: f32| spot(HotspotShape::Ellipse { cx: x, cy: y, rx: 0.05, ry: 0.05 });
        let map = HotspotMap {
            hotspots: vec![at(0.8, 0.8), at(0.2, 0.2), at(0.7, 0.21), at(0.1, 0.8)],
            ..Default::default()
        };
        let all = [0, 1, 2, 3];
        assert_eq!(map.focus_order(&all), vec![1, 2, 3, 0]);
        assert_eq!(map.next_focus(&all, None, true), Some(1));
        assert_eq!(map.next_focus(&all, Some(0), true), Some(1));
        assert_eq!(map.next_focus(&all, Some(1), false), Some(0));
        assert_eq!(map.next_focus(&[], None, true), None);
        // Right of 1 is 2; down from 1 is 3.
        assert_eq!(map.directional_focus(&all, Some(1), 1.0, 0.0), Some(2));
        assert_eq!(map.directional_focus(&all, Some(1), 0.0, 1.0), Some(3));
        // Filtered candidates are skipped.
        assert_eq!(map.next_focus(&[1, 3], Some(1), true), Some(3));
    }

    #[test]
    fn triangulation_covers_a_concave_polygon() {
        let l = [(0.0, 0.0), (0.2, 0.0), (0.2, 0.3), (0.5, 0.3), (0.5, 0.5), (0.0, 0.5)];
        let tris = triangulate(&l);
        assert_eq!(tris.len(), (l.len() - 2) * 3);
        let area = |t: &[u32]| {
            let (a, b, c) = (l[t[0] as usize], l[t[1] as usize], l[t[2] as usize]);
            ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs() * 0.5
        };
        let total: f32 = tris.chunks(3).map(area).sum();
        assert!((total - (0.2 * 0.3 + 0.5 * 0.2)).abs() < 1e-5, "{total}");
        // Clockwise works too.
        let mut cw = l.to_vec();
        cw.reverse();
        assert_eq!(triangulate(&cw).len(), 12);
        assert!(triangulate(&l[..2]).is_empty());
    }

    #[test]
    fn ron_round_trip_and_defaults() {
        let mut map = HotspotMap::default();
        map.hotspots.push(spot(HotspotShape::Polygon {
            points: vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)],
        }));
        let text = ron::to_string(&map).unwrap();
        let back: HotspotMap = ron::from_str(&text).unwrap();
        assert_eq!(back, map);
        let style: HotspotStyle = ron::from_str("(glow_width: 9.0)").unwrap();
        assert_eq!(style.glow_width, 9.0);
        assert_eq!(style.fill_alpha, HotspotStyle::default().fill_alpha);
    }
}
