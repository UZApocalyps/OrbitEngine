//! Pure 3D math of the 3D editor (no egui, no GPU): edit camera, rays,
//! bounds, picking and the gizmo drag maths. Unit tested headlessly.

use orbit_render3d::math::{self, Mat4, Quat, Vec3};

pub const UP: Vec3 = [0.0, 1.0, 0.0];

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn len(a: Vec3) -> f32 {
    math::dot(a, a).sqrt()
}

// ------------------------------------------------------------ quaternions

pub fn quat_mul(a: Quat, b: Quat) -> Quat {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

pub fn quat_conj(q: Quat) -> Quat {
    [-q[0], -q[1], -q[2], q[3]]
}

pub fn quat_normalize(q: Quat) -> Quat {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l > 1e-8 && l.is_finite() {
        [q[0] / l, q[1] / l, q[2] / l, q[3] / l]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

pub fn quat_axis_angle(axis: Vec3, angle: f32) -> Quat {
    let a = math::normalize(axis);
    let (s, c) = (angle * 0.5).sin_cos();
    [a[0] * s, a[1] * s, a[2] * s, c]
}

#[cfg(test)]
pub fn quat_rotate(q: Quat, v: Vec3) -> Vec3 {
    let p = quat_mul(quat_mul(q, [v[0], v[1], v[2], 0.0]), quat_conj(q));
    [p[0], p[1], p[2]]
}

/// Euler angles in degrees (`q = qz * qy * qx`) for the inspector.
/// Round-trips with [`quat_from_euler_deg`].
pub fn quat_to_euler_deg(q: Quat) -> Vec3 {
    let [x, y, z, w] = quat_normalize(q);
    let sinr = 2.0 * (w * x + y * z);
    let cosr = 1.0 - 2.0 * (x * x + y * y);
    let rx = sinr.atan2(cosr);
    let sinp = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
    let ry = sinp.asin();
    let siny = 2.0 * (w * z + x * y);
    let cosy = 1.0 - 2.0 * (y * y + z * z);
    let rz = siny.atan2(cosy);
    [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()]
}

pub fn quat_from_euler_deg(e: Vec3) -> Quat {
    let (sx, cx) = (e[0].to_radians() * 0.5).sin_cos();
    let (sy, cy) = (e[1].to_radians() * 0.5).sin_cos();
    let (sz, cz) = (e[2].to_radians() * 0.5).sin_cos();
    quat_normalize([
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ])
}

/// Rotation of the upper 3x3 of `m` (scale removed), as a quaternion.
pub fn mat_rotation(m: &Mat4) -> Quat {
    let c0 = math::normalize([m[0][0], m[0][1], m[0][2]]);
    let c1 = math::normalize([m[1][0], m[1][1], m[1][2]]);
    let c2 = math::normalize([m[2][0], m[2][1], m[2][2]]);
    let (m00, m01, m02) = (c0[0], c1[0], c2[0]);
    let (m10, m11, m12) = (c0[1], c1[1], c2[1]);
    let (m20, m21, m22) = (c0[2], c1[2], c2[2]);
    let tr = m00 + m11 + m22;
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]
    };
    quat_normalize(q)
}

// ------------------------------------------------------------------ rays

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn at(&self, t: f32) -> Vec3 {
        add(self.origin, scale(self.dir, t))
    }
}

/// Ray through the NDC point (`x`, `y` in -1..1, y up) of a camera with the
/// given view-projection matrix (clip depth 0..1).
pub fn screen_ray(view_proj: &Mat4, ndc: [f32; 2]) -> Option<Ray> {
    let inv = math::inverse(view_proj)?;
    let unproject = |z: f32| {
        let p = math::transform4(&inv, [ndc[0], ndc[1], z, 1.0]);
        (p[3].abs() > 1e-12).then(|| [p[0] / p[3], p[1] / p[3], p[2] / p[3]])
    };
    let near = unproject(0.0)?;
    let far = unproject(1.0)?;
    let dir = math::normalize(math::sub(far, near));
    (len(dir) > 0.5).then_some(Ray { origin: near, dir })
}

/// World point -> (NDC x, NDC y, depth), `None` behind the camera.
pub fn project(view_proj: &Mat4, p: Vec3) -> Option<[f32; 3]> {
    let c = math::transform4(view_proj, [p[0], p[1], p[2], 1.0]);
    (c[3] > 1e-6).then(|| [c[0] / c[3], c[1] / c[3], c[2] / c[3]])
}

pub fn ray_transform(ray: &Ray, inv_world: &Mat4) -> Ray {
    Ray {
        origin: math::transform_point(inv_world, ray.origin),
        // Not normalized on purpose: keeps `t` comparable with the world ray.
        dir: math::transform_vector(inv_world, ray.dir),
    }
}

// ---------------------------------------------------------------- bounds

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn cube(half: f32) -> Self {
        Self {
            min: [-half; 3],
            max: [half; 3],
        }
    }

    pub fn of_points(points: &[[f32; 3]]) -> Option<Self> {
        let mut it = points.iter().filter(|p| p.iter().all(|v| v.is_finite()));
        let first = *it.next()?;
        let mut b = Self {
            min: first,
            max: first,
        };
        for p in it {
            b.include(*p);
        }
        Some(b)
    }

    pub fn include(&mut self, p: Vec3) {
        self.min = std::array::from_fn(|k| self.min[k].min(p[k]));
        self.max = std::array::from_fn(|k| self.max[k].max(p[k]));
    }

    pub fn union(&mut self, o: &Bounds) {
        self.include(o.min);
        self.include(o.max);
    }

    pub fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|i| {
            [
                if i & 1 == 0 { self.min[0] } else { self.max[0] },
                if i & 2 == 0 { self.min[1] } else { self.max[1] },
                if i & 4 == 0 { self.min[2] } else { self.max[2] },
            ]
        })
    }

    pub fn transformed(&self, m: &Mat4) -> Bounds {
        let mut it = self
            .corners()
            .into_iter()
            .map(|c| math::transform_point(m, c));
        let first = it.next().unwrap_or([0.0; 3]);
        let mut b = Bounds {
            min: first,
            max: first,
        };
        for p in it {
            b.include(p);
        }
        b
    }

    pub fn center(&self) -> Vec3 {
        scale(add(self.min, self.max), 0.5)
    }

    pub fn radius(&self) -> f32 {
        len(math::sub(self.max, self.min)) * 0.5
    }
}

/// Slab test; `t` is the distance along `ray.dir` (in its own units).
pub fn ray_aabb(ray: &Ray, b: &Bounds) -> Option<f32> {
    let mut t0 = f32::NEG_INFINITY;
    let mut t1 = f32::INFINITY;
    for k in 0..3 {
        if ray.dir[k].abs() < 1e-12 {
            if ray.origin[k] < b.min[k] || ray.origin[k] > b.max[k] {
                return None;
            }
        } else {
            let inv = 1.0 / ray.dir[k];
            let (mut a, mut c) = (
                (b.min[k] - ray.origin[k]) * inv,
                (b.max[k] - ray.origin[k]) * inv,
            );
            if a > c {
                std::mem::swap(&mut a, &mut c);
            }
            t0 = t0.max(a);
            t1 = t1.min(c);
            if t0 > t1 {
                return None;
            }
        }
    }
    if t1 < 0.0 {
        return None;
    }
    Some(t0.max(0.0))
}

/// Ray against bounds that live in a node's local space: the hit distance is
/// in world units (the world ray direction is unit length).
pub fn ray_oriented_bounds(ray: &Ray, world: &Mat4, local: &Bounds) -> Option<f32> {
    let inv = math::inverse(world)?;
    // The local ray direction keeps the world parametrization, so `t` is
    // already a world distance.
    ray_aabb(&ray_transform(ray, &inv), local)
}

// ----------------------------------------------------------- edit camera

/// Orbit camera of the viewport.
#[derive(Debug, Clone, PartialEq)]
pub struct EditCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
}

impl Default for EditCamera {
    fn default() -> Self {
        Self {
            target: [0.0, 0.5, 0.0],
            yaw: 0.6,
            pitch: 0.4,
            distance: 6.0,
            fov_y: 60f32.to_radians(),
        }
    }
}

impl EditCamera {
    pub const MIN_DISTANCE: f32 = 0.05;
    pub const MAX_DISTANCE: f32 = 5000.0;

    pub fn eye(&self) -> Vec3 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        add(self.target, scale([cp * sy, sp, cp * cy], self.distance))
    }

    /// Camera-to-world matrix.
    pub fn world(&self) -> Mat4 {
        let view = math::look_at(self.eye(), self.target, UP);
        math::inverse(&view).unwrap_or(math::IDENTITY)
    }

    pub fn right_up(&self) -> (Vec3, Vec3) {
        let w = self.world();
        ([w[0][0], w[0][1], w[0][2]], [w[1][0], w[1][1], w[1][2]])
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.01;
        let lim = std::f32::consts::FRAC_PI_2 - 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(-lim, lim);
    }

    /// Pan by a screen delta in points; `viewport_h` is the viewport height.
    pub fn pan(&mut self, dx: f32, dy: f32, viewport_h: f32) {
        let per_px = 2.0 * self.distance * (self.fov_y * 0.5).tan() / viewport_h.max(1.0);
        let (r, u) = self.right_up();
        self.target = add(
            self.target,
            add(scale(r, -dx * per_px), scale(u, dy * per_px)),
        );
    }

    /// Zoom (scroll `amount`: positive zooms in), exponential.
    pub fn zoom(&mut self, amount: f32) {
        self.distance = (self.distance * (-amount * 0.0015).exp())
            .clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Frames a sphere.
    pub fn focus(&mut self, center: Vec3, radius: f32) {
        self.target = center;
        let r = radius.max(0.1);
        self.distance =
            (r / (self.fov_y * 0.5).sin() * 1.1).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Looks at the target from a fixed direction (keeps target and distance).
    pub fn set_view(&mut self, view: ViewPreset) {
        let lim = std::f32::consts::FRAC_PI_2 - 0.01;
        let half_pi = std::f32::consts::FRAC_PI_2;
        let (yaw, pitch) = match view {
            ViewPreset::Front => (0.0, 0.0),
            ViewPreset::Back => (std::f32::consts::PI, 0.0),
            ViewPreset::Right => (half_pi, 0.0),
            ViewPreset::Left => (-half_pi, 0.0),
            ViewPreset::Top => (0.0, lim),
            ViewPreset::Bottom => (0.0, -lim),
        };
        self.yaw = yaw;
        self.pitch = pitch;
    }

    /// Puts the edit camera where a scene camera (world matrix `w`) is,
    /// looking the same way.
    pub fn look_through(&mut self, w: &Mat4) {
        let eye = [w[3][0], w[3][1], w[3][2]];
        let fwd = math::normalize(math::transform_vector(w, [0.0, 0.0, -1.0]));
        self.target = add(eye, scale(fwd, self.distance));
        self.yaw = (-fwd[0]).atan2(-fwd[2]);
        let lim = std::f32::consts::FRAC_PI_2 - 0.01;
        self.pitch = (-fwd[1]).asin().clamp(-lim, lim);
    }

    pub fn camera3d(&self) -> orbit_render3d::Camera3d {
        orbit_render3d::Camera3d {
            world: self.world(),
            projection: orbit_scene::Projection::Perspective { fov_y: self.fov_y },
            near: (self.distance * 0.01).clamp(0.01, 1.0),
            far: 20000.0,
        }
    }
}

/// Fixed viewpoints of the edit camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPreset {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

/// The 8 corners of a camera frustum drawn `length` deep (clamped to the
/// camera range): the near rectangle then the far one, each
/// bottom-left, bottom-right, top-right, top-left, in world space.
pub fn frustum_corners(
    def: &orbit_scene::CameraDef,
    world: &Mat4,
    aspect: f32,
    length: f32,
) -> [Vec3; 8] {
    let near = def.near.max(1e-3);
    let far = length.clamp(near, def.far.max(near));
    let half = |d: f32| match def.projection {
        orbit_scene::Projection::Perspective { fov_y } => d * (fov_y * 0.5).tan(),
        orbit_scene::Projection::Orthographic { height } => height * 0.5,
    };
    let mut out = [[0.0; 3]; 8];
    for (k, d) in [near, far].into_iter().enumerate() {
        let hh = half(d);
        let hw = hh * aspect;
        for (c, (x, y)) in [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].into_iter().enumerate() {
            out[k * 4 + c] = math::transform_point(world, [x, y, -d]);
        }
    }
    out
}

// ----------------------------------------------------------------- gizmo

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn dir(self) -> Vec3 {
        match self {
            Axis::X => [1.0, 0.0, 0.0],
            Axis::Y => [0.0, 1.0, 0.0],
            Axis::Z => [0.0, 0.0, 1.0],
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Parameter `s` of the point of the line `p0 + s * dir` (`dir` unit) closest
/// to `ray`. `None` when the ray is parallel to the line.
pub fn ray_line_param(ray: &Ray, p0: Vec3, dir: Vec3) -> Option<f32> {
    let w = math::sub(p0, ray.origin);
    let b = math::dot(dir, ray.dir);
    let d = math::dot(dir, w);
    let e = math::dot(ray.dir, w);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    Some((b * e - d) / denom)
}

/// Intersection of `ray` with the plane through `p0` of normal `n`.
pub fn ray_plane(ray: &Ray, p0: Vec3, n: Vec3) -> Option<Vec3> {
    let denom = math::dot(n, ray.dir);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = math::dot(n, math::sub(p0, ray.origin)) / denom;
    (t >= 0.0).then(|| ray.at(t))
}

/// Signed angle (radians) from `a` to `b` around `axis` (all unit-ish).
pub fn signed_angle(a: Vec3, b: Vec3, axis: Vec3) -> f32 {
    let c = math::cross(a, b);
    math::dot(axis, c).atan2(math::dot(a, b))
}

/// Distance in points from `p` to the segment `a`-`b`.
pub fn dist_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 < 1e-9 {
        0.0
    } else {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    };
    let c = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt()
}

/// World length that makes a gizmo about `px` points long on screen.
pub fn gizmo_world_size(cam: &EditCamera, viewport_h: f32, px: f32) -> f32 {
    px * 2.0 * cam.distance * (cam.fov_y * 0.5).tan() / viewport_h.max(1.0)
}

/// Clips a world segment to the camera's near side and returns the two NDC
/// points, for the grid / overlays.
pub fn project_segment(view_proj: &Mat4, a: Vec3, b: Vec3) -> Option<([f32; 2], [f32; 2])> {
    let ca = math::transform4(view_proj, [a[0], a[1], a[2], 1.0]);
    let cb = math::transform4(view_proj, [b[0], b[1], b[2], 1.0]);
    let eps = 1e-3;
    let (mut ca, mut cb) = (ca, cb);
    if ca[3] < eps && cb[3] < eps {
        return None;
    }
    let lerp = |p: [f32; 4], q: [f32; 4]| {
        let t = (eps - p[3]) / (q[3] - p[3]);
        std::array::from_fn::<f32, 4, _>(|k| p[k] + (q[k] - p[k]) * t)
    };
    if ca[3] < eps {
        ca = lerp(ca, cb);
    } else if cb[3] < eps {
        cb = lerp(cb, ca);
    }
    Some((
        [ca[0] / ca[3], ca[1] / ca[3]],
        [cb[0] / cb[3], cb[1] / cb[3]],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn view_presets_look_along_the_axes() {
        let mut cam = EditCamera::default();
        cam.target = [1.0, 2.0, 3.0];
        cam.set_view(ViewPreset::Front);
        let e = cam.eye();
        assert!(close(e[0], 1.0) && close(e[1], 2.0) && e[2] > 3.0, "{e:?}");
        cam.set_view(ViewPreset::Right);
        assert!(cam.eye()[0] > 1.0 + cam.distance * 0.99);
        cam.set_view(ViewPreset::Top);
        assert!(cam.eye()[1] > 2.0 + cam.distance * 0.99);
        // The rendering stays finite even straight from above.
        let vp = cam.camera3d().view_proj(1.0);
        assert!(vp.iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn looking_through_a_camera_matches_its_direction() {
        let rot = quat_from_euler_deg([-20.0, 35.0, 0.0]);
        let w = math::from_trs([2.0, 1.0, 4.0], rot, [1.0; 3]);
        let mut cam = EditCamera::default();
        cam.look_through(&w);
        let e = cam.eye();
        assert!(len(math::sub(e, [2.0, 1.0, 4.0])) < 1e-3, "{e:?}");
        let fwd = math::transform_vector(&w, [0.0, 0.0, -1.0]);
        let mine = math::normalize(math::sub(cam.target, e));
        assert!(math::dot(fwd, mine) > 0.9999);
    }

    #[test]
    fn frustum_corners_follow_fov_aspect_and_range() {
        let def = orbit_scene::CameraDef {
            name: "c".into(),
            projection: orbit_scene::Projection::Perspective {
                fov_y: 90f32.to_radians(),
            },
            near: 0.5,
            far: 10.0,
        };
        let c = frustum_corners(&def, &math::IDENTITY, 2.0, 4.0);
        // Far plane 4 deep: half height 4 (90 degrees), half width 8.
        assert!(close(c[6][0], 8.0) && close(c[6][1], 4.0) && close(c[6][2], -4.0), "{:?}", c[6]);
        assert!(close(c[0][2], -0.5) && close(c[0][1], -0.5));
        // Clamped to the camera range.
        let c = frustum_corners(&def, &math::IDENTITY, 1.0, 100.0);
        assert!(close(c[4][2], -10.0));
        let ortho = orbit_scene::CameraDef {
            projection: orbit_scene::Projection::Orthographic { height: 2.0 },
            ..def
        };
        let c = frustum_corners(&ortho, &math::IDENTITY, 1.0, 3.0);
        assert!(close(c[2][1], 1.0) && close(c[6][1], 1.0));
    }

    #[test]
    fn center_ray_hits_the_target() {
        let cam = EditCamera::default();
        let vp = cam.camera3d().view_proj(1.5);
        let ray = screen_ray(&vp, [0.0, 0.0]).unwrap();
        let b = Bounds {
            min: add(cam.target, [-0.2; 3]),
            max: add(cam.target, [0.2; 3]),
        };
        let t = ray_aabb(&ray, &b).expect("hit");
        assert!(t > cam.distance - 0.5 && t < cam.distance + 0.1, "{t}");
        let side = screen_ray(&vp, [0.95, 0.9]).unwrap();
        assert!(ray_aabb(&side, &b).is_none());
    }

    #[test]
    fn project_then_unproject_roundtrips() {
        let cam = EditCamera::default();
        let vp = cam.camera3d().view_proj(1.0);
        let p = [0.3, 0.2, -0.4];
        let n = project(&vp, p).unwrap();
        let ray = screen_ray(&vp, [n[0], n[1]]).unwrap();
        let t = math::dot(math::sub(p, ray.origin), ray.dir);
        let q = ray.at(t);
        assert!(len(math::sub(p, q)) < 1e-2);
    }

    #[test]
    fn oriented_bounds_pick_scaled_rotated_node() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        // Long thin box along local X, rotated 90 degrees about Y => along Z.
        let world = math::from_trs([0.0; 3], [0.0, h, 0.0, h], [4.0, 1.0, 1.0]);
        let local = Bounds::cube(0.5);
        let along_z = Ray {
            origin: [0.0, 0.0, -10.0],
            dir: [0.0, 0.0, 1.0],
        };
        let t = ray_oriented_bounds(&along_z, &world, &local).unwrap();
        assert!(close(t, 8.0), "{t}");
        let along_x = Ray {
            origin: [-10.0, 0.0, 0.0],
            dir: [1.0, 0.0, 0.0],
        };
        let t = ray_oriented_bounds(&along_x, &world, &local).unwrap();
        assert!(close(t, 9.5), "{t}");
        let miss = Ray {
            origin: [1.5, 0.0, -10.0],
            dir: [0.0, 0.0, 1.0],
        };
        assert!(ray_oriented_bounds(&miss, &world, &local).is_none());
    }

    #[test]
    fn ray_inside_or_behind() {
        let b = Bounds::cube(1.0);
        let inside = Ray {
            origin: [0.0; 3],
            dir: [1.0, 0.0, 0.0],
        };
        assert_eq!(ray_aabb(&inside, &b), Some(0.0));
        let behind = Ray {
            origin: [5.0, 0.0, 0.0],
            dir: [1.0, 0.0, 0.0],
        };
        assert!(ray_aabb(&behind, &b).is_none());
    }

    #[test]
    fn translate_drag_param_follows_the_mouse() {
        let cam = EditCamera::default();
        let vp = cam.camera3d().view_proj(1.0);
        let origin = [0.0, 0.0, 0.0];
        let target = [1.5, 0.0, 0.0];
        let n = project(&vp, target).unwrap();
        let ray = screen_ray(&vp, [n[0], n[1]]).unwrap();
        let s = ray_line_param(&ray, origin, Axis::X.dir()).unwrap();
        assert!(close(s, 1.5), "{s}");
        let par = Ray {
            origin: [0.0, 1.0, 0.0],
            dir: [1.0, 0.0, 0.0],
        };
        assert!(ray_line_param(&par, origin, Axis::X.dir()).is_none());
    }

    #[test]
    fn rotation_angle_in_plane() {
        let a = [1.0, 0.0, 0.0];
        let b = [0.0, 0.0, -1.0];
        // +X to -Z is +90 degrees about +Y.
        let ang = signed_angle(a, b, Axis::Y.dir());
        assert!(close(ang, std::f32::consts::FRAC_PI_2));
        let q = quat_axis_angle(Axis::Y.dir(), ang);
        let r = quat_rotate(q, a);
        assert!(len(math::sub(r, b)) < 1e-4);
    }

    #[test]
    fn euler_roundtrip_and_mat_rotation() {
        for e in [[10.0, 20.0, 30.0], [-45.0, 60.0, 5.0], [0.0, 0.0, 0.0]] {
            let q = quat_from_euler_deg(e);
            let back = quat_to_euler_deg(q);
            let q2 = quat_from_euler_deg(back);
            let dotq = (0..4).map(|k| q[k] * q2[k]).sum::<f32>().abs();
            assert!(dotq > 0.9999, "{e:?} {back:?}");
        }
        let q = quat_from_euler_deg([30.0, 40.0, 50.0]);
        let m = math::from_trs([1.0, 2.0, 3.0], q, [2.0, 3.0, 4.0]);
        let r = mat_rotation(&m);
        let dotq = (0..4).map(|k| q[k] * r[k]).sum::<f32>().abs();
        assert!(dotq > 0.9999);
    }

    #[test]
    fn camera_orbit_pan_zoom_focus() {
        let mut c = EditCamera::default();
        let d0 = c.distance;
        c.zoom(100.0);
        assert!(c.distance < d0);
        c.zoom(-1e6);
        assert!(c.distance <= EditCamera::MAX_DISTANCE);
        c.orbit(0.0, 1e6);
        assert!(c.pitch < std::f32::consts::FRAC_PI_2);
        let t0 = c.target;
        c.pan(50.0, 0.0, 600.0);
        assert!(len(math::sub(c.target, t0)) > 0.0);
        c.focus([5.0, 0.0, 0.0], 2.0);
        assert_eq!(c.target, [5.0, 0.0, 0.0]);
        assert!(c.distance > 2.0);
        assert!(close(len(math::sub(c.eye(), c.target)), c.distance));
    }

    #[test]
    fn segment_clipping_behind_camera() {
        let cam = EditCamera::default();
        let vp = cam.camera3d().view_proj(1.0);
        let eye = cam.eye();
        let behind = scale(math::normalize(math::sub(eye, cam.target)), 50.0);
        assert!(project_segment(&vp, add(eye, behind), add(eye, scale(behind, 2.0))).is_none());
        assert!(project_segment(&vp, cam.target, add(eye, behind)).is_some());
    }

    #[test]
    fn segment_distance() {
        assert!(close(
            dist_to_segment([0.0, 1.0], [-1.0, 0.0], [1.0, 0.0]),
            1.0
        ));
        assert!(close(
            dist_to_segment([3.0, 0.0], [-1.0, 0.0], [1.0, 0.0]),
            2.0
        ));
    }
}
