//! Own transform gizmo (translate / rotate / scale). `transform-gizmo-egui`
//! was not adopted: it brings its own math crates (glam / ecolor) with their
//! own egui version pins and draws through its own painter; a few hundred
//! lines here keep the repo's egui, the quaternion-in-`[f32; 4]` data of
//! `orbit_scene` and headless tests of the drag maths.
//!
//! The handles are polylines in screen space ([`handles`]), used both to
//! paint and to hit test; the drag maths are pure functions of two rays.

use orbit_render3d::math::{self, Mat4, Vec3};
use orbit_scene::Transform;

use super::geom::{self, Axis, EditCamera, Ray};
use super::model;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Translate,
    Rotate,
    Scale,
}

/// Screen-space polyline of one handle, in points.
pub struct Handle {
    pub axis: Axis,
    pub points: Vec<[f32; 2]>,
}

/// Where the viewport is on screen (points).
#[derive(Debug, Clone, Copy)]
pub struct ScreenRect {
    pub min: [f32; 2],
    pub size: [f32; 2],
}

impl ScreenRect {
    pub fn to_screen(self, ndc: [f32; 2]) -> [f32; 2] {
        [
            self.min[0] + (ndc[0] + 1.0) * 0.5 * self.size[0],
            self.min[1] + (1.0 - ndc[1]) * 0.5 * self.size[1],
        ]
    }

    pub fn to_ndc(self, p: [f32; 2]) -> [f32; 2] {
        [
            (p[0] - self.min[0]) / self.size[0].max(1.0) * 2.0 - 1.0,
            1.0 - (p[1] - self.min[1]) / self.size[1].max(1.0) * 2.0,
        ]
    }
}

/// World directions of the axes the gizmo of `mode` works along: world axes
/// for translate / rotate, the node's own axes for scale.
pub fn axes(mode: Mode, world: &Mat4) -> [Vec3; 3] {
    match mode {
        Mode::Scale => std::array::from_fn(|k| {
            math::normalize([world[k][0], world[k][1], world[k][2]])
        }),
        _ => [Axis::X.dir(), Axis::Y.dir(), Axis::Z.dir()],
    }
}

/// Handle polylines for painting and hit testing. `size` is the gizmo's
/// world length.
pub fn handles(
    mode: Mode,
    origin: Vec3,
    dirs: &[Vec3; 3],
    size: f32,
    vp: &Mat4,
    screen: &ScreenRect,
) -> Vec<Handle> {
    let proj = |p: Vec3| geom::project(vp, p).map(|n| screen.to_screen([n[0], n[1]]));
    let mut out = Vec::new();
    for axis in Axis::ALL {
        let d = dirs[axis.index()];
        let points: Vec<[f32; 2]> = match mode {
            Mode::Translate | Mode::Scale => {
                let a = proj(origin);
                let b = proj(geom::add(origin, geom::scale(d, size)));
                match (a, b) {
                    (Some(a), Some(b)) => vec![a, b],
                    _ => continue,
                }
            }
            Mode::Rotate => {
                let (u, v) = plane_basis(d);
                let r = size * 0.85;
                (0..=48)
                    .filter_map(|i| {
                        let t = i as f32 / 48.0 * std::f32::consts::TAU;
                        let p = geom::add(
                            origin,
                            geom::add(geom::scale(u, t.cos() * r), geom::scale(v, t.sin() * r)),
                        );
                        proj(p)
                    })
                    .collect()
            }
        };
        if points.len() >= 2 {
            out.push(Handle { axis, points });
        }
    }
    out
}

/// Two unit vectors spanning the plane perpendicular to `n`.
pub fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let helper = if n[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = math::normalize(math::cross(n, helper));
    let v = math::normalize(math::cross(n, u));
    (u, v)
}

/// The handle under `p` (points), within `tolerance`.
pub fn hit(handles: &[Handle], p: [f32; 2], tolerance: f32) -> Option<Axis> {
    handles
        .iter()
        .filter_map(|h| {
            let d = h
                .points
                .windows(2)
                .map(|w| geom::dist_to_segment(p, w[0], w[1]))
                .fold(f32::INFINITY, f32::min);
            (d <= tolerance).then_some((d, h.axis))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, a)| a)
}

/// A gizmo drag in progress. Every frame recomputes the result from the
/// state at the press (absolute, no drift).
#[derive(Debug, Clone)]
pub struct Drag {
    pub mode: Mode,
    pub axis: Axis,
    pub base: Transform,
    pub parent_world: Mat4,
    pub origin: Vec3,
    /// Direction the drag works along (world).
    pub dir: Vec3,
    /// Translate / scale: line parameter at the press. Rotate: unused.
    pub grab: f32,
    /// Rotate: unit vector from the origin to the press point, in the plane.
    pub grab_vec: Vec3,
}

impl Drag {
    /// Starts a drag with the press ray. `None` when the maths degenerate
    /// (ray parallel to the axis / plane).
    pub fn begin(
        mode: Mode,
        axis: Axis,
        base: Transform,
        world: &Mat4,
        parent_world: Mat4,
        ray: &Ray,
    ) -> Option<Self> {
        let origin = [world[3][0], world[3][1], world[3][2]];
        let dir = axes(mode, world)[axis.index()];
        let (grab, grab_vec) = match mode {
            Mode::Translate | Mode::Scale => (geom::ray_line_param(ray, origin, dir)?, [0.0; 3]),
            Mode::Rotate => {
                let hit = geom::ray_plane(ray, origin, dir)?;
                let v = math::normalize(math::sub(hit, origin));
                if geom::len(v) < 0.5 {
                    return None;
                }
                (0.0, v)
            }
        };
        Some(Self {
            mode,
            axis,
            base,
            parent_world,
            origin,
            dir,
            grab,
            grab_vec,
        })
    }

    /// The local transform for the current ray. `snap` rounds the move (to
    /// 0.25), the angle (to 15 degrees) and the scale factor (to 0.1).
    pub fn apply(&self, ray: &Ray, snap: bool) -> Option<Transform> {
        match self.mode {
            Mode::Translate => {
                let s = geom::ray_line_param(ray, self.origin, self.dir)?;
                let mut delta = s - self.grab;
                if snap {
                    delta = (delta / 0.25).round() * 0.25;
                }
                let pos = geom::add(self.origin, geom::scale(self.dir, delta));
                Some(model::moved_local(&self.base, &self.parent_world, pos))
            }
            Mode::Rotate => {
                let hit = geom::ray_plane(ray, self.origin, self.dir)?;
                let v = math::normalize(math::sub(hit, self.origin));
                if geom::len(v) < 0.5 {
                    return None;
                }
                let mut angle = geom::signed_angle(self.grab_vec, v, self.dir);
                if snap {
                    let step = 15f32.to_radians();
                    angle = (angle / step).round() * step;
                }
                let delta = geom::quat_axis_angle(self.dir, angle);
                Some(model::rotated_local(&self.base, &self.parent_world, delta))
            }
            Mode::Scale => {
                let s = geom::ray_line_param(ray, self.origin, self.dir)?;
                if self.grab.abs() < 1e-3 {
                    return None;
                }
                let mut factor = s / self.grab;
                if snap {
                    factor = (factor / 0.1).round() * 0.1;
                }
                let k = self.axis.index();
                let mut t = self.base;
                t.scale[k] = (self.base.scale[k] * factor).max(0.001);
                Some(t)
            }
        }
    }
}

/// World length of the gizmo for a camera (about 90 points on screen).
pub fn world_size(cam: &EditCamera, viewport_h: f32) -> f32 {
    geom::gizmo_world_size(cam, viewport_h, 90.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (EditCamera, Mat4, ScreenRect) {
        let cam = EditCamera::default();
        let screen = ScreenRect {
            min: [10.0, 20.0],
            size: [600.0, 400.0],
        };
        let vp = cam.camera3d().view_proj(screen.size[0] / screen.size[1]);
        (cam, vp, screen)
    }

    fn ray_at(vp: &Mat4, screen: &ScreenRect, p: [f32; 2]) -> Ray {
        geom::screen_ray(vp, screen.to_ndc(p)).unwrap()
    }

    fn screen_of(vp: &Mat4, screen: &ScreenRect, w: Vec3) -> [f32; 2] {
        let n = geom::project(vp, w).unwrap();
        screen.to_screen([n[0], n[1]])
    }

    #[test]
    fn screen_mapping_roundtrips() {
        let (_, _, s) = setup();
        let p = [123.0, 77.0];
        let q = s.to_screen(s.to_ndc(p));
        assert!((p[0] - q[0]).abs() < 1e-3 && (p[1] - q[1]).abs() < 1e-3);
    }

    #[test]
    fn hit_picks_the_axis_under_the_pointer() {
        let (cam, vp, screen) = setup();
        let origin = [0.0; 3];
        let size = world_size(&cam, screen.size[1]);
        let dirs = axes(Mode::Translate, &math::IDENTITY);
        let hs = handles(Mode::Translate, origin, &dirs, size, &vp, &screen);
        assert_eq!(hs.len(), 3);
        for axis in Axis::ALL {
            let tip = geom::scale(axis.dir(), size * 0.6);
            let p = screen_of(&vp, &screen, tip);
            assert_eq!(hit(&hs, p, 6.0), Some(axis), "{axis:?}");
        }
        assert_eq!(hit(&hs, [screen.min[0] + 2.0, screen.min[1] + 2.0], 6.0), None);
    }

    #[test]
    fn rotate_handles_are_rings() {
        let (cam, vp, screen) = setup();
        let size = world_size(&cam, screen.size[1]);
        let dirs = axes(Mode::Rotate, &math::IDENTITY);
        let hs = handles(Mode::Rotate, [0.0; 3], &dirs, size, &vp, &screen);
        assert_eq!(hs.len(), 3);
        assert!(hs.iter().all(|h| h.points.len() > 40));
        // A point of the Y ring is hit as Y.
        let (u, v) = plane_basis(Axis::Y.dir());
        let diag = math::normalize(geom::add(u, v));
        let p = screen_of(&vp, &screen, geom::scale(diag, size * 0.85));
        assert_eq!(hit(&hs, p, 4.0), Some(Axis::Y));
    }

    #[test]
    fn translate_drag_moves_along_the_axis_only() {
        let (_, vp, screen) = setup();
        let base = Transform::IDENTITY;
        let world = model_world(&base);
        let press = ray_at(&vp, &screen, screen_of(&vp, &screen, [0.5, 0.0, 0.0]));
        let drag = Drag::begin(
            Mode::Translate,
            Axis::X,
            base,
            &world,
            math::IDENTITY,
            &press,
        )
        .unwrap();
        // Move the mouse to where world x = 2.5 would be on screen.
        let now = ray_at(&vp, &screen, screen_of(&vp, &screen, [2.5, 0.0, 0.0]));
        let t = drag.apply(&now, false).unwrap();
        assert!((t.translation[0] - 2.0).abs() < 1e-2, "{t:?}");
        assert!(t.translation[1].abs() < 1e-3 && t.translation[2].abs() < 1e-3);
        // Snapping to 0.25.
        let t = drag.apply(&now, true).unwrap();
        assert!((t.translation[0] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn rotate_drag_follows_the_pointer_angle() {
        let (_, vp, screen) = setup();
        let base = Transform::IDENTITY;
        let world = model_world(&base);
        let p0 = geom::ray_plane(
            &ray_at(&vp, &screen, screen_of(&vp, &screen, [1.0, 0.0, 0.0])),
            [0.0; 3],
            Axis::Y.dir(),
        )
        .unwrap();
        assert!((p0[0] - 1.0).abs() < 1e-2);
        let press = ray_at(&vp, &screen, screen_of(&vp, &screen, [1.0, 0.0, 0.0]));
        let drag = Drag::begin(Mode::Rotate, Axis::Y, base, &world, math::IDENTITY, &press)
            .unwrap();
        // Pointer at world (0, 0, -1): +90 degrees about +Y.
        let now = ray_at(&vp, &screen, screen_of(&vp, &screen, [0.0, 0.0, -1.0]));
        let t = drag.apply(&now, false).unwrap();
        let v = geom::quat_rotate(t.rotation, [1.0, 0.0, 0.0]);
        assert!((v[2] + 1.0).abs() < 1e-2 && v[0].abs() < 1e-2, "{v:?}");
        // Snap keeps multiples of 15 degrees: 90 stays 90.
        let t = drag.apply(&now, true).unwrap();
        let v = geom::quat_rotate(t.rotation, [1.0, 0.0, 0.0]);
        assert!((v[2] + 1.0).abs() < 1e-3);
    }

    #[test]
    fn scale_drag_scales_the_local_axis() {
        let (_, vp, screen) = setup();
        let mut base = Transform::IDENTITY;
        base.scale = [2.0, 1.0, 1.0];
        let world = model_world(&base);
        let press = ray_at(&vp, &screen, screen_of(&vp, &screen, [1.0, 0.0, 0.0]));
        let drag = Drag::begin(Mode::Scale, Axis::X, base, &world, math::IDENTITY, &press)
            .unwrap();
        let now = ray_at(&vp, &screen, screen_of(&vp, &screen, [1.5, 0.0, 0.0]));
        let t = drag.apply(&now, false).unwrap();
        assert!((t.scale[0] - 3.0).abs() < 2e-2, "{t:?}");
        assert!((t.scale[1] - 1.0).abs() < 1e-6);
        // Dragging through the origin never gives a zero / negative scale.
        let back = ray_at(&vp, &screen, screen_of(&vp, &screen, [-3.0, 0.0, 0.0]));
        assert!(drag.apply(&back, false).unwrap().scale[0] >= 0.001);
    }

    #[test]
    fn begin_fails_on_a_degenerate_ray() {
        let base = Transform::IDENTITY;
        let world = model_world(&base);
        let parallel = Ray {
            origin: [0.0, 1.0, 0.0],
            dir: [1.0, 0.0, 0.0],
        };
        assert!(
            Drag::begin(Mode::Translate, Axis::X, base, &world, math::IDENTITY, &parallel)
                .is_none()
        );
        assert!(
            Drag::begin(Mode::Rotate, Axis::Y, base, &world, math::IDENTITY, &parallel).is_none()
        );
    }

    fn model_world(t: &Transform) -> Mat4 {
        math::from_trs(t.translation, t.rotation, t.scale)
    }
}
