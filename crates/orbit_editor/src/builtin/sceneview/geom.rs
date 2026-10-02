//! The scene view's geometry: pure functions on the virtual 1920x1080 canvas
//! (no egui, no GPU), unit tested. Dragging, scale handles, snapping and
//! keyboard nudging are all decided here; the panel only feeds pointer
//! positions in and turns the results into commands.
//!
//! Coordinates: *virtual pixels* are the canvas of the game (1920x1080, origin
//! top-left); *normalized* ones are `0..=1` over that canvas, which is what a
//! story stores. A sprite is anchored by its bottom-center point.

use orbit_core::{FREE_POSITION_RANGE, Position, SCALE_MAX, SCALE_MIN, Story};

pub const CANVAS: [f32; 2] = [1920.0, 1080.0];
/// Grid spacing, as a fraction of the canvas.
pub const GRID_STEP: f32 = 0.05;
/// Title-safe margin, as a fraction of the canvas on each side.
pub const SAFE_MARGIN: f32 = 0.05;
/// Arrow-key step, as a fraction of the canvas (about 4 virtual pixels)...
pub const NUDGE: f32 = 0.002;
/// ...and with Shift (about 38 virtual pixels).
pub const NUDGE_BIG: f32 = 0.02;
/// Distance, in screen points, within which snapping engages.
pub const SNAP_POINTS: f32 = 8.0;

/// Where and how big a character stands: what the gizmos edit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Normalized position of the bottom-center anchor.
    pub x: f32,
    pub y: f32,
    /// Scale `[x, y]`.
    pub scale: [f32; 2],
}

/// Bounding box `[left, top, right, bottom]` in virtual pixels of a sprite
/// whose unscaled size is `base`, anchored at its bottom-center.
pub fn bounds(pose: &Pose, base: [f32; 2]) -> [f32; 4] {
    let (w, h) = (base[0] * pose.scale[0], base[1] * pose.scale[1]);
    let (ax, ay) = (pose.x * CANVAS[0], pose.y * CANVAS[1]);
    [ax - w * 0.5, ay - h, ax + w * 0.5, ay]
}

pub fn contains(b: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

/// The anchor of a pose in virtual pixels.
pub fn anchor(pose: &Pose) -> [f32; 2] {
    [pose.x * CANVAS[0], pose.y * CANVAS[1]]
}

/// A scale handle. The bottom edge is the anchor line and has no handle of
/// its own; the bottom corners scale evenly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Left,
    Right,
    BottomLeft,
    BottomRight,
}

impl Handle {
    pub const ALL: [Handle; 7] = [
        Handle::TopLeft,
        Handle::Top,
        Handle::TopRight,
        Handle::Left,
        Handle::Right,
        Handle::BottomLeft,
        Handle::BottomRight,
    ];

    pub fn is_corner(self) -> bool {
        matches!(
            self,
            Handle::TopLeft | Handle::TopRight | Handle::BottomLeft | Handle::BottomRight
        )
    }

    /// Position of the handle on `b` (virtual pixels).
    pub fn point(self, b: [f32; 4]) -> [f32; 2] {
        let (cx, cy) = ((b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5);
        match self {
            Handle::TopLeft => [b[0], b[1]],
            Handle::Top => [cx, b[1]],
            Handle::TopRight => [b[2], b[1]],
            Handle::Left => [b[0], cy],
            Handle::Right => [b[2], cy],
            Handle::BottomLeft => [b[0], b[3]],
            Handle::BottomRight => [b[2], b[3]],
        }
    }
}

/// The handle under `p` (virtual pixels), within `radius`; the nearest wins.
pub fn hit_handle(b: [f32; 4], p: [f32; 2], radius: f32) -> Option<Handle> {
    Handle::ALL
        .into_iter()
        .map(|h| {
            let q = h.point(b);
            (h, (q[0] - p[0]).hypot(q[1] - p[1]))
        })
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
}

/// The scale `[x, y]` after dragging `handle` to `pointer` (virtual pixels).
///
/// The sprite scales around its anchor, so the sides move symmetrically:
/// the horizontal factor is twice the pointer's distance from the anchor
/// over the current width, the vertical one is the pointer's height above
/// the anchor over the current height. Edges change one axis; corners change
/// both and keep the ratio when the sprite is uniform or `keep_ratio`
/// (Shift) is held; with `keep_ratio` an edge scales both axes evenly too.
/// The result stays within the valid scale range.
pub fn scale_with_handle(
    handle: Handle,
    start: [f32; 2],
    base: [f32; 2],
    anchor: [f32; 2],
    pointer: [f32; 2],
    keep_ratio: bool,
) -> [f32; 2] {
    let (w0, h0) = (base[0] * start[0], base[1] * start[1]);
    if w0 <= 0.0 || h0 <= 0.0 {
        return start;
    }
    let kx = 2.0 * (pointer[0] - anchor[0]).abs() / w0;
    let ky = (anchor[1] - pointer[1]) / h0;
    let uniform = (start[0] - start[1]).abs() <= 1e-4 * start[0].max(start[1]);
    let (mut kx, mut ky) = match handle {
        Handle::Left | Handle::Right => (kx, 1.0),
        Handle::Top => (1.0, ky),
        Handle::TopLeft | Handle::TopRight => (kx, ky),
        // The bottom edge is the anchor: only the horizontal pull counts.
        Handle::BottomLeft | Handle::BottomRight => (kx, kx),
    };
    let evenly = keep_ratio || (handle.is_corner() && uniform);
    if evenly {
        let k = match handle {
            Handle::Left | Handle::Right => kx,
            Handle::Top => ky,
            _ => (kx + ky) * 0.5,
        };
        // Keep both axes within range with the ratio intact.
        let lo = (SCALE_MIN / start[0]).max(SCALE_MIN / start[1]);
        let hi = (SCALE_MAX / start[0]).min(SCALE_MAX / start[1]);
        let k = k.clamp(lo, hi.max(lo));
        (kx, ky) = (k, k);
    }
    [
        (start[0] * kx).clamp(SCALE_MIN, SCALE_MAX),
        (start[1] * ky).clamp(SCALE_MIN, SCALE_MAX),
    ]
}

/// Snaps a scale to a nearby "nice" value (1.0, the character's default...)
/// when it is within 3 %; a near-uniform pair becomes exactly uniform.
pub fn snap_scale(scale: [f32; 2], nice: &[f32]) -> [f32; 2] {
    let snap = |v: f32| {
        nice.iter()
            .copied()
            .find(|n| (v - n).abs() <= 0.03 * n)
            .unwrap_or(v)
    };
    let mut out = [snap(scale[0]), snap(scale[1])];
    if (out[0] - out[1]).abs() <= 0.01 * out[0].max(out[1]) {
        out[1] = out[0];
    }
    out
}

/// A named place on the stage a character can snap to.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedPoint {
    pub position: Position,
    pub name: String,
    pub x: f32,
    pub y: f32,
}

/// Left, center, right and the project's custom positions.
pub fn named_points(story: &Story) -> Vec<NamedPoint> {
    let mut out: Vec<NamedPoint> = [
        (Position::Left, "Gauche"),
        (Position::Center, "Centre"),
        (Position::Right, "Droite"),
    ]
    .into_iter()
    .filter_map(|(position, name)| {
        let (x, y) = story.resolve_position(&position)?;
        Some(NamedPoint {
            position,
            name: name.to_owned(),
            x,
            y,
        })
    })
    .collect();
    out.extend(story.positions.iter().map(|p| NamedPoint {
        position: Position::Custom(p.name.clone()),
        name: p.name.clone(),
        x: p.x,
        y: p.y,
    }));
    out
}

/// What snapping was asked to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapOptions {
    /// Snap to the named positions (left, center, right, custom ones).
    pub named: bool,
    /// Snap to the grid.
    pub grid: bool,
    /// Engagement distance per axis, in normalized units.
    pub threshold: [f32; 2],
}

/// The result of [`snap_position`].
#[derive(Clone, Debug, PartialEq)]
pub struct Snapped {
    pub x: f32,
    pub y: f32,
    /// A guide to draw: the normalized `x` / `y` the position snapped to.
    pub guide_x: Option<f32>,
    pub guide_y: Option<f32>,
    /// Both coordinates landed on this named position.
    pub named: Option<Position>,
}

/// Snaps a normalized position, axis by axis: a named position wins over the
/// grid; no snapping engages outside the threshold. The result is always
/// within [`FREE_POSITION_RANGE`].
pub fn snap_position(pos: [f32; 2], points: &[NamedPoint], opts: SnapOptions) -> Snapped {
    let clamp = |v: f32| v.clamp(*FREE_POSITION_RANGE.start(), *FREE_POSITION_RANGE.end());
    let mut out = Snapped {
        x: clamp(pos[0]),
        y: clamp(pos[1]),
        guide_x: None,
        guide_y: None,
        named: None,
    };
    let mut hit_x: Option<&NamedPoint> = None;
    let mut hit_y: Option<&NamedPoint> = None;
    if opts.named {
        let mut best = opts.threshold[0];
        for p in points {
            let d = (p.x - out.x).abs();
            if d <= best {
                best = d;
                hit_x = Some(p);
            }
        }
        let mut best = opts.threshold[1];
        for p in points {
            let d = (p.y - out.y).abs();
            if d <= best {
                best = d;
                hit_y = Some(p);
            }
        }
    }
    match hit_x {
        Some(p) => {
            out.x = p.x;
            out.guide_x = Some(p.x);
        }
        None if opts.grid => {
            let g = (out.x / GRID_STEP).round() * GRID_STEP;
            if (g - out.x).abs() <= opts.threshold[0] {
                out.x = g;
                out.guide_x = Some(g);
            }
        }
        None => {}
    }
    match hit_y {
        Some(p) => {
            out.y = p.y;
            out.guide_y = Some(p.y);
        }
        None if opts.grid => {
            let g = (out.y / GRID_STEP).round() * GRID_STEP;
            if (g - out.y).abs() <= opts.threshold[1] {
                out.y = g;
                out.guide_y = Some(g);
            }
        }
        None => {}
    }
    // On a named position exactly: keep the story readable (`Left`, not
    // `Free(0.25, 1.0)`).
    out.named = points
        .iter()
        .find(|p| (p.x - out.x).abs() < 1e-4 && (p.y - out.y).abs() < 1e-4)
        .map(|p| p.position.clone());
    out
}

/// An arrow key press moves the anchor by `NUDGE` (`NUDGE_BIG` with Shift);
/// `dir` is `[-1|0|1, -1|0|1]` (screen directions, y down).
pub fn nudged(pose: &Pose, dir: [i8; 2], big: bool) -> [f32; 2] {
    let step = if big { NUDGE_BIG } else { NUDGE };
    let clamp = |v: f32| v.clamp(*FREE_POSITION_RANGE.start(), *FREE_POSITION_RANGE.end());
    [
        clamp(pose.x + f32::from(dir[0]) * step),
        clamp(pose.y + f32::from(dir[1]) * step),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32, y: f32, s: f32) -> Pose {
        Pose {
            x,
            y,
            scale: [s, s],
        }
    }

    fn opts(named: bool, grid: bool) -> SnapOptions {
        SnapOptions {
            named,
            grid,
            threshold: [0.01, 0.01],
        }
    }

    fn points() -> Vec<NamedPoint> {
        named_points(&Story::new("t"))
    }

    #[test]
    fn bounds_hang_from_the_bottom_center_anchor() {
        let b = bounds(&pose(0.5, 1.0, 1.0), [600.0, 920.0]);
        assert_eq!(b, [660.0, 160.0, 1260.0, 1080.0]);
        let half = bounds(&pose(0.25, 0.9, 0.5), [600.0, 920.0]);
        assert_eq!(half, [330.0, 512.0, 630.0, 972.0]);
        // Scale only grows from the anchor: the bottom-center never moves.
        let (a, b2) = (
            bounds(&pose(0.3, 0.8, 1.0), [100.0, 200.0]),
            bounds(&pose(0.3, 0.8, 2.0), [100.0, 200.0]),
        );
        assert_eq!(((a[0] + a[2]) * 0.5, a[3]), ((b2[0] + b2[2]) * 0.5, b2[3]));
    }

    #[test]
    fn handles_are_found_within_their_radius_nearest_first() {
        let b = [100.0, 100.0, 300.0, 500.0];
        assert_eq!(hit_handle(b, [100.0, 100.0], 10.0), Some(Handle::TopLeft));
        assert_eq!(hit_handle(b, [200.0, 104.0], 10.0), Some(Handle::Top));
        assert_eq!(hit_handle(b, [301.0, 300.0], 10.0), Some(Handle::Right));
        assert_eq!(
            hit_handle(b, [300.0, 500.0], 10.0),
            Some(Handle::BottomRight)
        );
        assert_eq!(
            hit_handle(b, [200.0, 300.0], 10.0),
            None,
            "inside, no handle"
        );
    }

    #[test]
    fn a_corner_drag_scales_a_uniform_sprite_evenly_around_the_anchor() {
        // 200x400 at scale 1, anchored at (200, 500): pulling the top-right
        // corner out to twice the distance doubles both axes.
        let s = scale_with_handle(
            Handle::TopRight,
            [1.0, 1.0],
            [200.0, 400.0],
            [200.0, 500.0],
            [400.0, -300.0],
            false,
        );
        assert!(
            (s[0] - 2.0).abs() < 1e-4 && (s[1] - 2.0).abs() < 1e-4,
            "{s:?}"
        );
        // Pulling toward the anchor shrinks.
        let s = scale_with_handle(
            Handle::TopLeft,
            [1.0, 1.0],
            [200.0, 400.0],
            [200.0, 500.0],
            [150.0, 300.0],
            false,
        );
        assert!(s[0] < 1.0 && (s[0] - s[1]).abs() < 1e-6, "{s:?}");
    }

    #[test]
    fn edges_stretch_one_axis_and_shift_keeps_the_ratio() {
        let base = [200.0, 400.0];
        let anchor = [1000.0, 1000.0];
        // Right edge at 2x the half-width: X doubles, Y unchanged.
        let s = scale_with_handle(
            Handle::Right,
            [1.0, 1.0],
            base,
            anchor,
            [1200.0, 800.0],
            false,
        );
        assert_eq!(s, [2.0, 1.0]);
        // Top edge at half the height: Y halves.
        let s = scale_with_handle(
            Handle::Top,
            [1.0, 1.0],
            base,
            anchor,
            [1000.0, 800.0],
            false,
        );
        assert_eq!(s, [1.0, 0.5]);
        // Shift on an edge scales both axes by the same factor.
        let s = scale_with_handle(
            Handle::Right,
            [1.0, 1.0],
            base,
            anchor,
            [1200.0, 800.0],
            true,
        );
        assert_eq!(s, [2.0, 2.0]);
        // A non uniform sprite: a corner moves both axes freely...
        let free = scale_with_handle(
            Handle::TopRight,
            [1.0, 0.5],
            base,
            anchor,
            [1200.0, 500.0],
            false,
        );
        assert!(
            (free[0] - 2.0).abs() < 1e-4 && (free[1] - 1.25).abs() < 1e-4,
            "{free:?}"
        );
        // ...and Shift keeps its 2:1 ratio.
        let kept = scale_with_handle(
            Handle::TopRight,
            [1.0, 0.5],
            base,
            anchor,
            [1200.0, 500.0],
            true,
        );
        assert!((kept[0] / kept[1] - 2.0).abs() < 1e-4, "{kept:?}");
    }

    #[test]
    fn scales_stay_within_the_valid_range_and_degenerate_input_is_safe() {
        let huge = scale_with_handle(
            Handle::TopRight,
            [1.0, 1.0],
            [200.0, 400.0],
            [200.0, 500.0],
            [1.0e9, -1.0e9],
            false,
        );
        assert_eq!(huge, [SCALE_MAX, SCALE_MAX]);
        let tiny = scale_with_handle(
            Handle::Right,
            [1.0, 1.0],
            [200.0, 400.0],
            [200.0, 500.0],
            [200.0, 100.0],
            false,
        );
        assert_eq!(tiny[0], SCALE_MIN);
        // A ratio-keeping drag that hits the limit keeps the ratio.
        let kept = scale_with_handle(
            Handle::TopRight,
            [4.0, 2.0],
            [200.0, 400.0],
            [200.0, 500.0],
            [1.0e9, -1.0e9],
            true,
        );
        assert!((kept[0] / kept[1] - 2.0).abs() < 1e-4 && kept[0] <= SCALE_MAX);
        // No size: nothing to scale.
        assert_eq!(
            scale_with_handle(
                Handle::Right,
                [1.0, 1.0],
                [0.0, 0.0],
                [0.0, 0.0],
                [5.0, 5.0],
                false
            ),
            [1.0, 1.0]
        );
    }

    #[test]
    fn scale_snaps_to_nice_values_and_near_uniform_becomes_uniform() {
        assert_eq!(snap_scale([1.02, 1.02], &[1.0]), [1.0, 1.0]);
        assert_eq!(snap_scale([0.8, 0.8], &[1.0, 0.8]), [0.8, 0.8]);
        assert_eq!(snap_scale([1.3, 1.3], &[1.0]), [1.3, 1.3]);
        assert_eq!(snap_scale([1.3, 1.305], &[]), [1.3, 1.3]);
        assert_eq!(snap_scale([1.3, 1.6], &[]), [1.3, 1.6]);
    }

    #[test]
    fn positions_snap_to_named_places_and_report_an_exact_hit() {
        let p = points();
        // Near "Left" on both axes: lands on it exactly.
        let s = snap_position([0.255, 0.995], &p, opts(true, false));
        assert_eq!((s.x, s.y), (0.25, 1.0));
        assert_eq!(s.named, Some(Position::Left));
        assert_eq!((s.guide_x, s.guide_y), (Some(0.25), Some(1.0)));
        // Only x is near: x snaps, y stays free, no named position.
        let s = snap_position([0.505, 0.7], &p, opts(true, false));
        assert_eq!((s.x, s.y), (0.5, 0.7));
        assert_eq!((s.guide_x, s.guide_y, s.named), (Some(0.5), None, None));
        // Far from everything, or snapping off: untouched.
        assert_eq!(snap_position([0.4, 0.6], &p, opts(true, false)).x, 0.4);
        assert_eq!(
            snap_position([0.255, 0.995], &p, opts(false, false)).x,
            0.255
        );
    }

    #[test]
    fn custom_positions_are_snap_targets_too() {
        let mut story = Story::new("t");
        story.positions.push(orbit_core::PositionDef {
            name: "porte".into(),
            x: 0.9,
            y: 0.8,
        });
        let p = named_points(&story);
        assert_eq!(p.len(), 4);
        let s = snap_position([0.903, 0.797], &p, opts(true, false));
        assert_eq!((s.x, s.y), (0.9, 0.8));
        assert_eq!(s.named, Some(Position::Custom("porte".into())));
    }

    #[test]
    fn the_grid_snaps_per_axis_and_named_positions_win_over_it() {
        let p = points();
        let s = snap_position([0.302, 0.598], &p, opts(false, true));
        assert!(
            (s.x - 0.3).abs() < 1e-6 && (s.y - 0.6).abs() < 1e-6,
            "{s:?}"
        );
        assert_eq!(s.named, None);
        // 0.253 is near both the 0.25 grid line and Left: both give 0.25.
        let s = snap_position([0.253, 0.5], &p, opts(true, true));
        assert_eq!(s.x, 0.25);
        // Outside the threshold the grid does not engage.
        let s = snap_position([0.325, 0.5], &p, opts(false, true));
        assert_eq!(s.x, 0.325);
    }

    #[test]
    fn positions_are_kept_within_the_valid_range() {
        let p = points();
        let s = snap_position([-9.0, 9.0], &p, opts(true, true));
        assert_eq!((s.x, s.y), (-2.0, 3.0));
        assert_eq!(
            nudged(&pose(3.0, -2.0, 1.0), [1, -1], true),
            [3.0, -2.0],
            "nudging cannot leave the range"
        );
    }

    #[test]
    fn arrows_nudge_finely_and_shift_takes_a_big_step() {
        let p = pose(0.5, 0.9, 1.0);
        let fine = nudged(&p, [1, 0], false);
        assert!((fine[0] - 0.502).abs() < 1e-6 && fine[1] == 0.9);
        let big = nudged(&p, [0, -1], true);
        assert!((big[1] - 0.88).abs() < 1e-6 && big[0] == 0.5);
    }
}
