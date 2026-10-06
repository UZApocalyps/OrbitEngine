//! Viewport geometry of the rig editor: the 2D view, bone hit-testing and the
//! drag math of the move / rotate / scale handles. Pure functions, unit tested.
//!
//! Rig space is Y down (like the screen); a positive rotation is clockwise.

use orbit_anim::rig::{Affine, Rig, Xform};

/// Pixels around a bone that still select it.
pub const PICK_TOLERANCE: f32 = 7.0;
/// Radius of the rotation ring around the selected bone's head, in pixels.
pub const RING_RADIUS: f32 = 38.0;
/// Radius of the grab circles / squares, in pixels.
pub const GRAB_RADIUS: f32 = 9.0;
/// A bone is never drawn shorter than this (pixels), so it can be picked.
pub const MIN_BONE_PX: f32 = 12.0;
/// Rotation snapping step (Ctrl held), degrees.
pub const SNAP_DEGREES: f32 = 15.0;
/// A bone drag shorter than this (pixels) creates nothing.
pub const MIN_CREATE_PX: f32 = 6.0;

/// Rig space ↔ screen: `screen = center + pan + rig * zoom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f32,
    pub pan: [f32; 2],
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: 1.0, pan: [0.0, 0.0] }
    }
}

impl View {
    pub const MIN_ZOOM: f32 = 0.05;
    pub const MAX_ZOOM: f32 = 40.0;

    pub fn to_screen(&self, center: [f32; 2], p: [f32; 2]) -> [f32; 2] {
        [center[0] + self.pan[0] + p[0] * self.zoom, center[1] + self.pan[1] + p[1] * self.zoom]
    }

    pub fn to_rig(&self, center: [f32; 2], s: [f32; 2]) -> [f32; 2] {
        [(s[0] - center[0] - self.pan[0]) / self.zoom, (s[1] - center[1] - self.pan[1]) / self.zoom]
    }

    /// Zooms by `factor` keeping the rig point under `cursor` still.
    pub fn zoom_at(&mut self, center: [f32; 2], cursor: [f32; 2], factor: f32) {
        let anchor = self.to_rig(center, cursor);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let moved = self.to_screen(center, anchor);
        self.pan[0] += cursor[0] - moved[0];
        self.pan[1] += cursor[1] - moved[1];
    }

    /// Fits the rig-space box `[min, max]` in a viewport of `size` pixels.
    pub fn fit(min: [f32; 2], max: [f32; 2], size: [f32; 2]) -> Self {
        let w = (max[0] - min[0]).max(1.0);
        let h = (max[1] - min[1]).max(1.0);
        let zoom = (size[0] * 0.8 / w).min(size[1] * 0.8 / h).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let c = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
        Self { zoom, pan: [-c[0] * zoom, -c[1] * zoom] }
    }
}

/// Head and tip of a bone in rig space. The tip is `length` along the local X
/// axis; a bone shorter than [`MIN_BONE_PX`] on screen is stretched for
/// drawing and picking only.
pub fn bone_ends(world: &Affine, length: f32, zoom: f32) -> ([f32; 2], [f32; 2]) {
    let head = world.origin();
    let sx = world.a.hypot(world.b).max(1e-6);
    let shown = (length * sx).max(MIN_BONE_PX / zoom.max(1e-6));
    let tip = [head[0] + world.a / sx * shown, head[1] + world.b / sx * shown];
    (head, tip)
}

/// Distance from `p` to the segment `a`–`b`.
pub fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
    let len2 = abx * abx + aby * aby;
    let t = if len2 > 1e-12 { (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p[0] - (a[0] + abx * t)).hypot(p[1] - (a[1] + aby * t))
}

/// The bone under the screen point `p`, if any: the nearest to the pointer,
/// the later one (drawn on top, usually the child) on ties.
pub fn hit_bone(rig: &Rig, world: &[Affine], view: &View, center: [f32; 2], p: [f32; 2]) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, b) in rig.bones.iter().enumerate() {
        let Some(w) = world.get(i) else { continue };
        let (h, t) = bone_ends(w, b.length, view.zoom);
        let dist = segment_distance(p, view.to_screen(center, h), view.to_screen(center, t));
        if dist <= PICK_TOLERANCE && best.is_none_or(|(_, d)| dist <= d + 0.5) {
            best = Some((i, dist));
        }
    }
    best.map(|(i, _)| i)
}

/// What a drag on the selected bone changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Move,
    Rotate,
    Scale,
}

/// The tool selected in the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Click-drag draws a bone under the selected one.
    Create,
    Move,
    Rotate,
    Scale,
    /// Edits the selected slot's mesh (M5B-04).
    Mesh,
    /// Paints the selected bone's weights on the selected slot's mesh (M5B-05).
    Weights,
    /// Cuts the selected slot's picture into new parts (lasso, polygon, wand).
    Cut,
    /// Paints on the selected slot's picture.
    Paint,
}

/// Which handle of the selected bone (head at `head_s`, tip at `tip_s`, screen
/// pixels) is under `p` for `tool`. Move: the head (dragging the body works
/// too, see [`hit_bone`]); rotate: the ring around the head; scale: the tip.
pub fn handle_at(tool: Tool, head_s: [f32; 2], tip_s: [f32; 2], p: [f32; 2]) -> Option<Handle> {
    let dist = |a: [f32; 2]| (p[0] - a[0]).hypot(p[1] - a[1]);
    match tool {
        Tool::Move => (dist(head_s) <= GRAB_RADIUS).then_some(Handle::Move),
        Tool::Rotate => ((dist(head_s) - RING_RADIUS).abs() <= GRAB_RADIUS * 0.7).then_some(Handle::Rotate),
        Tool::Scale => (dist(tip_s) <= GRAB_RADIUS).then_some(Handle::Scale),
        Tool::Create | Tool::Mesh | Tool::Weights | Tool::Cut | Tool::Paint => None,
    }
}

fn angle_deg(from: [f32; 2], to: [f32; 2]) -> f32 {
    (to[1] - from[1]).atan2(to[0] - from[0]).to_degrees()
}

fn wrap_deg(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a > 180.0 {
        a -= 360.0;
    } else if a < -180.0 {
        a += 360.0;
    }
    a
}

/// New local transform when the bone's head follows the pointer: the world
/// displacement of the pointer, expressed in the parent's frame.
pub fn drag_move(parent_world: Option<&Affine>, start: &Xform, start_ptr: [f32; 2], ptr: [f32; 2]) -> Xform {
    let (dx, dy) = (ptr[0] - start_ptr[0], ptr[1] - start_ptr[1]);
    let (lx, ly) = match parent_world.and_then(Affine::inverse) {
        // Linear part only: a displacement, not a point.
        Some(inv) => (inv.a * dx + inv.c * dy, inv.b * dx + inv.d * dy),
        None => (dx, dy),
    };
    Xform { x: start.x + lx, y: start.y + ly, ..*start }
}

/// New local transform when the pointer turns around the bone's head by the
/// angle between `start_ptr` and `ptr`. `snap` rounds the rotation to
/// [`SNAP_DEGREES`].
pub fn drag_rotate(
    parent_world: Option<&Affine>,
    start: &Xform,
    head: [f32; 2],
    start_ptr: [f32; 2],
    ptr: [f32; 2],
    snap: bool,
) -> Xform {
    let delta = wrap_deg(angle_deg(head, ptr) - angle_deg(head, start_ptr));
    // A mirrored parent turns the world the other way.
    let sign = if parent_world.is_some_and(|p| p.det() < 0.0) { -1.0 } else { 1.0 };
    let mut rotation = start.rotation + sign * delta;
    if snap {
        rotation = (rotation / SNAP_DEGREES).round() * SNAP_DEGREES;
    }
    Xform { rotation, ..*start }
}

/// New local scale when the scale handle at the tip moves: the X scale follows
/// the pointer's distance along the bone axis (`axis` is the world direction
/// of the bone); `uniform` applies the same ratio to Y.
pub fn drag_scale(start: &Xform, head: [f32; 2], axis: [f32; 2], start_ptr: [f32; 2], ptr: [f32; 2], uniform: bool) -> Xform {
    let norm = axis[0].hypot(axis[1]).max(1e-9);
    let proj = |p: [f32; 2]| ((p[0] - head[0]) * axis[0] + (p[1] - head[1]) * axis[1]) / norm;
    let (a, b) = (proj(start_ptr), proj(ptr));
    let ratio = if a.abs() < 1e-3 { 1.0 } else { (b / a).clamp(0.01, 100.0) };
    Xform { sx: start.sx * ratio, sy: if uniform { start.sy * ratio } else { start.sy }, ..*start }
}

/// Bounding box of every bone head and tip (rig space), for "frame all".
pub fn rig_bounds(rig: &Rig, world: &[Affine]) -> Option<([f32; 2], [f32; 2])> {
    let mut out: Option<([f32; 2], [f32; 2])> = None;
    for (i, b) in rig.bones.iter().enumerate() {
        let Some(w) = world.get(i) else { continue };
        let (h, t) = bone_ends(w, b.length, 1.0);
        for p in [h, t] {
            out = Some(match out {
                None => (p, p),
                Some((lo, hi)) => ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::rigedit::model::{RigDoc, rest_locals, world_matrices};

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn view_round_trip_and_zoom_keeps_cursor_point() {
        let center = [400.0, 300.0];
        let mut v = View { zoom: 2.0, pan: [30.0, -10.0] };
        let p = [12.0, 34.0];
        let s = v.to_screen(center, p);
        let r = v.to_rig(center, s);
        assert!(near(r[0], p[0]) && near(r[1], p[1]));
        let cursor = [500.0, 350.0];
        let under = v.to_rig(center, cursor);
        v.zoom_at(center, cursor, 1.5);
        assert!(near(v.zoom, 3.0));
        let after = v.to_rig(center, cursor);
        assert!(near(under[0], after[0]) && near(under[1], after[1]));
        v.zoom_at(center, cursor, 1e9);
        assert!(near(v.zoom, View::MAX_ZOOM));
    }

    #[test]
    fn hit_test_picks_nearest_bone_and_misses_far_points() {
        let mut doc = RigDoc::new();
        let a = doc.add_bone(None, [0.0, 0.0], [100.0, 0.0]).unwrap();
        let b = doc.add_bone(None, [0.0, 40.0], [100.0, 40.0]).unwrap();
        let rig = doc.rig();
        let world = world_matrices(rig, &rest_locals(rig));
        let view = View::default();
        let c = [0.0, 0.0];
        let ia = rig.bones.iter().position(|x| x.id == a).unwrap();
        let ib = rig.bones.iter().position(|x| x.id == b).unwrap();
        assert_eq!(hit_bone(rig, &world, &view, c, [50.0, 3.0]), Some(ia));
        assert_eq!(hit_bone(rig, &world, &view, c, [50.0, 38.0]), Some(ib));
        assert_eq!(hit_bone(rig, &world, &view, c, [50.0, 20.0]), None, "between the bones");
        assert_eq!(hit_bone(rig, &world, &view, c, [150.0, 0.0]), None, "beyond the tip");
        // Zoomed in, the same screen offset is a smaller rig distance.
        let zoomed = View { zoom: 4.0, pan: [0.0, 0.0] };
        assert_eq!(hit_bone(rig, &world, &zoomed, c, [200.0, 5.0]), Some(ia));
    }

    #[test]
    fn hit_test_prefers_the_closer_of_two_overlapping_bones() {
        let mut doc = RigDoc::new();
        let _a = doc.add_bone(None, [0.0, 0.0], [100.0, 0.0]).unwrap();
        let b = doc.add_bone(None, [0.0, 4.0], [100.0, 4.0]).unwrap();
        let rig = doc.rig();
        let world = world_matrices(rig, &rest_locals(rig));
        let ib = rig.bones.iter().position(|x| x.id == b).unwrap();
        assert_eq!(hit_bone(rig, &world, &View::default(), [0.0, 0.0], [50.0, 5.0]), Some(ib));
    }

    #[test]
    fn tiny_bones_stay_pickable() {
        let (h, t) = bone_ends(&Xform::translate(10.0, 10.0).matrix(), 0.0, 1.0);
        assert!(near(h[0], 10.0) && (t[0] - h[0]).abs() >= MIN_BONE_PX - 1e-3);
    }

    #[test]
    fn handles_per_tool() {
        let (h, t) = ([100.0, 100.0], [180.0, 100.0]);
        assert_eq!(handle_at(Tool::Move, h, t, [103.0, 98.0]), Some(Handle::Move));
        assert_eq!(handle_at(Tool::Move, h, t, [180.0, 100.0]), None);
        assert_eq!(handle_at(Tool::Rotate, h, t, [100.0 + RING_RADIUS, 100.0]), Some(Handle::Rotate));
        assert_eq!(handle_at(Tool::Rotate, h, t, [103.0, 100.0]), None, "inside the ring");
        assert_eq!(handle_at(Tool::Scale, h, t, [178.0, 102.0]), Some(Handle::Scale));
        assert_eq!(handle_at(Tool::Create, h, t, h), None);
    }

    #[test]
    fn drag_move_in_a_rotated_parent_frame() {
        // Parent turned 90 degrees clockwise: world +X is parent -Y... world
        // displacement (10, 0) is local (0, -10) after the inverse.
        let parent = Xform { rotation: 90.0, ..Xform::IDENTITY }.matrix();
        let start = Xform { x: 5.0, y: 5.0, ..Xform::IDENTITY };
        let out = drag_move(Some(&parent), &start, [0.0, 0.0], [10.0, 0.0]);
        assert!(near(out.x, 5.0) && near(out.y, -5.0), "{out:?}");
        // No parent: plain displacement.
        let out = drag_move(None, &start, [0.0, 0.0], [10.0, -3.0]);
        assert!(near(out.x, 15.0) && near(out.y, 2.0));
        // Scaled parent: displacement shrinks accordingly.
        let parent = Xform { sx: 2.0, sy: 2.0, ..Xform::IDENTITY }.matrix();
        let out = drag_move(Some(&parent), &start, [0.0, 0.0], [10.0, 0.0]);
        assert!(near(out.x, 10.0));
    }

    #[test]
    fn drag_rotate_follows_pointer_angle_snaps_and_handles_mirrors() {
        let start = Xform { rotation: 10.0, ..Xform::IDENTITY };
        let head = [0.0, 0.0];
        // Pointer from east to south: 90 degrees clockwise (Y down).
        let out = drag_rotate(None, &start, head, [50.0, 0.0], [0.0, 50.0], false);
        assert!(near(out.rotation, 100.0), "{}", out.rotation);
        // Crossing the +-180 seam takes the short way.
        let out = drag_rotate(None, &start, head, [-50.0, -1.0], [-50.0, 1.0], false);
        assert!(out.rotation.abs() < 20.0, "{}", out.rotation);
        // Snap.
        let out = drag_rotate(None, &Xform::IDENTITY, head, [50.0, 0.0], [50.0, 10.0], true);
        assert!(near(out.rotation, 15.0) || near(out.rotation, 0.0), "{}", out.rotation);
        assert!(near(out.rotation % SNAP_DEGREES, 0.0));
        // Mirrored parent: opposite sense.
        let mirror = Xform { sy: -1.0, ..Xform::IDENTITY }.matrix();
        let out = drag_rotate(Some(&mirror), &Xform::IDENTITY, head, [50.0, 0.0], [0.0, 50.0], false);
        assert!(near(out.rotation, -90.0));
    }

    #[test]
    fn drag_scale_uses_axis_projection() {
        let start = Xform::IDENTITY;
        let head = [0.0, 0.0];
        let out = drag_scale(&start, head, [1.0, 0.0], [50.0, 0.0], [100.0, 30.0], false);
        assert!(near(out.sx, 2.0) && near(out.sy, 1.0));
        let out = drag_scale(&start, head, [1.0, 0.0], [50.0, 0.0], [25.0, 0.0], true);
        assert!(near(out.sx, 0.5) && near(out.sy, 0.5));
        // Starting on the head: no change, no NaN.
        let out = drag_scale(&start, head, [1.0, 0.0], [0.0, 5.0], [80.0, 0.0], true);
        assert!(near(out.sx, 1.0));
        // Dragging past the head clamps instead of flipping to zero or negative.
        let out = drag_scale(&start, head, [1.0, 0.0], [50.0, 0.0], [-500.0, 0.0], false);
        assert!(out.sx > 0.0);
    }

    #[test]
    fn bounds_and_fit() {
        let mut doc = RigDoc::new();
        doc.add_bone(None, [0.0, 0.0], [100.0, 0.0]).unwrap();
        doc.add_bone(None, [0.0, 0.0], [0.0, -60.0]).unwrap();
        let rig = doc.rig();
        let world = world_matrices(rig, &rest_locals(rig));
        let (lo, hi) = rig_bounds(rig, &world).unwrap();
        assert!(near(lo[1], -60.0) && near(hi[0], 100.0));
        let v = View::fit(lo, hi, [800.0, 600.0]);
        let s = v.to_screen([400.0, 300.0], [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0]);
        assert!(near(s[0], 400.0) && near(s[1], 300.0));
        assert!(rig_bounds(&Rig::default(), &[]).is_none());
    }
}
