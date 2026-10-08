//! Procedural primitive shapes (`scene3d_version` 2): cube, UV sphere,
//! plane, cylinder, cone, capsule and torus.
//!
//! A scene stores a [`Shape`] (kind + parameters) instead of a mesh file;
//! renderers generate the triangles when they need them ([`Shape::mesh`]),
//! with normals, UVs (glTF convention, `v` grows downwards) and tangents.
//! Faces are counter-clockwise seen from outside (glTF front faces), the
//! shape is centred on the origin, `+Y` up. Generation is deterministic, so
//! the same parameters always give the same mesh and the same
//! [`Shape::mesh_key`].

use serde::{Deserialize, Serialize};

use crate::{MeshAsset, PrimitiveData};

/// Segment counts are clamped to this range (a bad file cannot ask for
/// billions of vertices).
pub const MIN_SEGMENTS: u32 = 3;
pub const MAX_SEGMENTS: u32 = 256;
/// Plane subdivisions are clamped to `1..=MAX_SUBDIVISIONS`.
pub const MAX_SUBDIVISIONS: u32 = 256;
/// Prefix of the mesh names of generated shapes (never a file name).
pub const MESH_PREFIX: &str = "@shape/";

/// A procedural primitive and its parameters (metres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// Box of `size` (x, y, z).
    Cube { size: [f32; 3] },
    /// UV sphere: `segments` around Y, `rings` from pole to pole.
    Sphere { radius: f32, segments: u32, rings: u32 },
    /// Horizontal plane (XZ, facing +Y) of `size` (x, z), split into
    /// `subdivisions` x `subdivisions` quads.
    Plane { size: [f32; 2], subdivisions: u32 },
    /// Capped cylinder along Y.
    Cylinder { radius: f32, height: f32, segments: u32 },
    /// Cone along Y, apex up, capped base.
    Cone { radius: f32, height: f32, segments: u32 },
    /// Capsule along Y; `height` is the total height (at least `2 * radius`).
    Capsule { radius: f32, height: f32, segments: u32, rings: u32 },
    /// Torus around Y: `radius` to the tube centre, `tube` radius.
    Torus { radius: f32, tube: f32, segments: u32, sides: u32 },
}

/// The kinds of shape, in menu order (stable identifiers for tools).
pub const SHAPE_KINDS: [&str; 7] = ["cube", "sphere", "plane", "cylinder", "cone", "capsule", "torus"];

impl Shape {
    /// The shape `kind` (see [`SHAPE_KINDS`]) with its default parameters
    /// (about one metre).
    pub fn default_of(kind: &str) -> Option<Self> {
        Some(match kind {
            "cube" => Self::Cube { size: [1.0; 3] },
            "sphere" => Self::Sphere { radius: 0.5, segments: 32, rings: 16 },
            "plane" => Self::Plane { size: [2.0, 2.0], subdivisions: 1 },
            "cylinder" => Self::Cylinder { radius: 0.5, height: 1.0, segments: 32 },
            "cone" => Self::Cone { radius: 0.5, height: 1.0, segments: 32 },
            "capsule" => Self::Capsule { radius: 0.25, height: 1.0, segments: 32, rings: 8 },
            "torus" => Self::Torus { radius: 0.5, tube: 0.15, segments: 32, sides: 16 },
            _ => return None,
        })
    }

    /// Stable identifier of the kind (see [`SHAPE_KINDS`]).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Cube { .. } => "cube",
            Self::Sphere { .. } => "sphere",
            Self::Plane { .. } => "plane",
            Self::Cylinder { .. } => "cylinder",
            Self::Cone { .. } => "cone",
            Self::Capsule { .. } => "capsule",
            Self::Torus { .. } => "torus",
        }
    }

    /// Name under which renderers cache the generated mesh: identical
    /// parameters share one mesh. Starts with [`MESH_PREFIX`].
    pub fn mesh_key(&self) -> String {
        format!("{MESH_PREFIX}{self:?}")
    }

    /// Whether every size is finite and positive (the validator reports
    /// the others; generation clamps them anyway).
    pub fn is_valid(&self) -> bool {
        let ok = |v: &[f32]| v.iter().all(|x| x.is_finite() && *x > 0.0);
        match *self {
            Self::Cube { size } => ok(&size),
            Self::Sphere { radius, .. } => ok(&[radius]),
            Self::Plane { size, .. } => ok(&size),
            Self::Cylinder { radius, height, .. } | Self::Cone { radius, height, .. } => {
                ok(&[radius, height])
            }
            Self::Capsule { radius, height, .. } => ok(&[radius, height]),
            Self::Torus { radius, tube, .. } => ok(&[radius, tube]),
        }
    }

    /// Generates the mesh (one primitive) named [`Shape::mesh_key`].
    pub fn mesh(&self) -> MeshAsset {
        MeshAsset {
            name: self.mesh_key(),
            primitives: vec![self.primitive()],
            ..Default::default()
        }
    }

    /// Triangle count of the generated mesh.
    pub fn triangles(&self) -> u64 {
        (self.primitive().indices.len() / 3) as u64
    }

    /// Generates the triangles.
    pub fn primitive(&self) -> PrimitiveData {
        let mut b = Builder::default();
        match *self {
            Self::Cube { size } => {
                let h = size.map(|s| pos(s) * 0.5);
                for (n, u, v) in CUBE_FACES {
                    let scale = |a: [f32; 3]| [a[0] * h[0], a[1] * h[1], a[2] * h[2]];
                    b.grid(scale(n), scale(u), scale(v), n, 1, 1);
                }
            }
            Self::Sphere { radius, segments, rings } => {
                let r = pos(radius);
                let rings = rings.clamp(2, MAX_SEGMENTS);
                let profile: Vec<Ring> = (0..=rings)
                    .map(|j| {
                        let phi = -FRAC_PI_2 + PI * j as f32 / rings as f32;
                        Ring::new(r * phi.cos(), r * phi.sin(), phi.cos(), phi.sin())
                    })
                    .collect();
                b.lathe(&profile, seg(segments));
            }
            Self::Plane { size, subdivisions } => {
                let n = [0.0, 1.0, 0.0];
                b.grid(
                    [0.0; 3],
                    [pos(size[0]) * 0.5, 0.0, 0.0],
                    [0.0, 0.0, -pos(size[1]) * 0.5],
                    n,
                    subdivisions.clamp(1, MAX_SUBDIVISIONS),
                    subdivisions.clamp(1, MAX_SUBDIVISIONS),
                );
            }
            Self::Cylinder { radius, height, segments } => {
                let (r, h, s) = (pos(radius), pos(height) * 0.5, seg(segments));
                b.lathe(&[Ring::new(r, -h, 1.0, 0.0), Ring::new(r, h, 1.0, 0.0)], s);
                b.disk(r, h, true, s);
                b.disk(r, -h, false, s);
            }
            Self::Cone { radius, height, segments } => {
                let (r, h, s) = (pos(radius), pos(height), seg(segments));
                let len = (h * h + r * r).sqrt();
                let (nr, ny) = (h / len, r / len);
                b.lathe(
                    &[Ring::new(r, -h * 0.5, nr, ny), Ring::new(0.0, h * 0.5, nr, ny)],
                    s,
                );
                b.disk(r, -h * 0.5, false, s);
            }
            Self::Capsule { radius, height, segments, rings } => {
                let r = pos(radius);
                let half = (pos(height) * 0.5 - r).max(0.0);
                let rings = rings.clamp(1, MAX_SEGMENTS);
                let mut profile = Vec::new();
                for j in 0..=rings {
                    let phi = -FRAC_PI_2 + FRAC_PI_2 * j as f32 / rings as f32;
                    profile.push(Ring::new(r * phi.cos(), r * phi.sin() - half, phi.cos(), phi.sin()));
                }
                // Without a cylinder part the equator row is shared.
                for j in u32::from(half == 0.0)..=rings {
                    let phi = FRAC_PI_2 * j as f32 / rings as f32;
                    profile.push(Ring::new(r * phi.cos(), r * phi.sin() + half, phi.cos(), phi.sin()));
                }
                b.lathe(&profile, seg(segments));
            }
            Self::Torus { radius, tube, segments, sides } => {
                let (big, t) = (pos(radius), pos(tube));
                let sides = seg(sides);
                let profile: Vec<Ring> = (0..=sides)
                    .map(|j| {
                        let psi = -PI + 2.0 * PI * j as f32 / sides as f32;
                        Ring::new(big + t * psi.cos(), t * psi.sin(), psi.cos(), psi.sin())
                    })
                    .collect();
                b.lathe(&profile, seg(segments));
            }
        }
        b.finish()
    }
}

/// Shape parameters by name, every one optional (tools: MCP, CLI). Which
/// ones a kind uses: cube `size`; sphere `radius`, `segments`, `rings`;
/// plane `size` (x, z: the first and last components, or the first two
/// when `y` is 0), `subdivisions`; cylinder / cone `radius`, `height`,
/// `segments`; capsule `radius`, `height`, `segments`, `rings`; torus
/// `radius`, `tube`, `segments`, `sides`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ShapeParams {
    pub size: Option<[f32; 3]>,
    pub radius: Option<f32>,
    pub height: Option<f32>,
    pub tube: Option<f32>,
    pub segments: Option<u32>,
    pub rings: Option<u32>,
    pub sides: Option<u32>,
    pub subdivisions: Option<u32>,
}

impl ShapeParams {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl Shape {
    /// Applies the given parameters; one that the kind does not use is an
    /// error (it would be silently ignored otherwise).
    pub fn apply(&mut self, p: &ShapeParams) -> Result<(), String> {
        let kind = self.kind();
        let unused = |name: &str| Err(format!("a {kind} has no `{name}` parameter"));
        macro_rules! set {
            ($field:ident, $target:expr) => {
                if let Some(v) = p.$field {
                    *$target = v;
                }
            };
        }
        macro_rules! refuse {
            ($($field:ident),*) => {
                $(if p.$field.is_some() {
                    return unused(stringify!($field));
                })*
            };
        }
        match self {
            Self::Cube { size } => {
                refuse!(radius, height, tube, segments, rings, sides, subdivisions);
                set!(size, size);
            }
            Self::Sphere { radius, segments, rings } => {
                refuse!(size, height, tube, sides, subdivisions);
                set!(radius, radius);
                set!(segments, segments);
                set!(rings, rings);
            }
            Self::Plane { size, subdivisions } => {
                refuse!(radius, height, tube, segments, rings, sides);
                if let Some(s) = p.size {
                    *size = if s[2] > 0.0 { [s[0], s[2]] } else { [s[0], s[1]] };
                }
                set!(subdivisions, subdivisions);
            }
            Self::Cylinder { radius, height, segments } | Self::Cone { radius, height, segments } => {
                refuse!(size, tube, rings, sides, subdivisions);
                set!(radius, radius);
                set!(height, height);
                set!(segments, segments);
            }
            Self::Capsule { radius, height, segments, rings } => {
                refuse!(size, tube, sides, subdivisions);
                set!(radius, radius);
                set!(height, height);
                set!(segments, segments);
                set!(rings, rings);
            }
            Self::Torus { radius, tube, segments, sides } => {
                refuse!(size, height, rings, subdivisions);
                set!(radius, radius);
                set!(tube, tube);
                set!(segments, segments);
                set!(sides, sides);
            }
        }
        Ok(())
    }

    /// The parameters of this shape (only the ones its kind uses).
    pub fn params(&self) -> ShapeParams {
        let mut p = ShapeParams::default();
        match *self {
            Self::Cube { size } => p.size = Some(size),
            Self::Sphere { radius, segments, rings } => {
                (p.radius, p.segments, p.rings) = (Some(radius), Some(segments), Some(rings));
            }
            Self::Plane { size, subdivisions } => {
                (p.size, p.subdivisions) = (Some([size[0], 0.0, size[1]]), Some(subdivisions));
            }
            Self::Cylinder { radius, height, segments } | Self::Cone { radius, height, segments } => {
                (p.radius, p.height, p.segments) = (Some(radius), Some(height), Some(segments));
            }
            Self::Capsule { radius, height, segments, rings } => {
                (p.radius, p.height, p.segments, p.rings) =
                    (Some(radius), Some(height), Some(segments), Some(rings));
            }
            Self::Torus { radius, tube, segments, sides } => {
                (p.radius, p.tube, p.segments, p.sides) =
                    (Some(radius), Some(tube), Some(segments), Some(sides));
            }
        }
        p
    }
}

use std::f32::consts::{FRAC_PI_2, PI};

/// (normal, u half-axis, v half-axis) of the unit cube faces, with
/// `u x v = normal` (counter-clockwise from outside).
const CUBE_FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
];

/// A positive, finite size (bad values become a small default).
fn pos(x: f32) -> f32 {
    if x.is_finite() && x > 0.0 { x.min(1.0e6) } else { 1.0e-3 }
}

fn seg(n: u32) -> u32 {
    n.clamp(MIN_SEGMENTS, MAX_SEGMENTS)
}

/// One row of a surface of revolution: radius, height, normal (radial, y).
struct Ring {
    r: f32,
    y: f32,
    nr: f32,
    ny: f32,
}

impl Ring {
    fn new(r: f32, y: f32, nr: f32, ny: f32) -> Self {
        Self { r: r.max(0.0), y, nr, ny }
    }
}

#[derive(Default)]
struct Builder {
    p: PrimitiveData,
}

impl Builder {
    fn vertex(&mut self, position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> u32 {
        self.p.positions.push(position);
        self.p.normals.push(normal);
        self.p.uvs.push(uv);
        (self.p.positions.len() - 1) as u32
    }

    /// A flat grid `center + s*u + t*v` (`s`, `t` in -1..1).
    fn grid(&mut self, center: [f32; 3], u: [f32; 3], v: [f32; 3], n: [f32; 3], nu: u32, nv: u32) {
        let base = self.p.positions.len() as u32;
        for j in 0..=nv {
            let t = j as f32 / nv as f32;
            for i in 0..=nu {
                let s = i as f32 / nu as f32;
                let (a, b) = (s * 2.0 - 1.0, t * 2.0 - 1.0);
                let p = [0, 1, 2].map(|k| center[k] + a * u[k] + b * v[k]);
                self.vertex(p, n, [s, 1.0 - t]);
            }
        }
        let row = nu + 1;
        for j in 0..nv {
            for i in 0..nu {
                let a = base + j * row + i;
                let (b, c, d) = (a + 1, a + row, a + row + 1);
                self.p.indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
    }

    /// Surface of revolution around Y; `profile` goes upwards on the
    /// outside. The seam is duplicated (UVs wrap), rows of radius 0 (poles,
    /// apex) drop their degenerate triangles.
    fn lathe(&mut self, profile: &[Ring], segments: u32) {
        let base = self.p.positions.len() as u32;
        let rows = profile.len() as u32;
        for (j, ring) in profile.iter().enumerate() {
            let v = 1.0 - j as f32 / (rows - 1) as f32;
            for i in 0..=segments {
                let theta = 2.0 * PI * i as f32 / segments as f32;
                let (s, c) = theta.sin_cos();
                self.vertex(
                    [ring.r * s, ring.y, ring.r * c],
                    normalize([ring.nr * s, ring.ny, ring.nr * c]),
                    [i as f32 / segments as f32, v],
                );
            }
        }
        let row = segments + 1;
        for j in 0..rows - 1 {
            let (low, high) = (profile[j as usize].r, profile[j as usize + 1].r);
            for i in 0..segments {
                let a = base + j * row + i;
                let (b, c, d) = (a + 1, a + row, a + row + 1);
                if low > 0.0 {
                    self.p.indices.extend_from_slice(&[a, b, c]);
                }
                if high > 0.0 {
                    self.p.indices.extend_from_slice(&[b, d, c]);
                }
            }
        }
    }

    /// A flat cap at height `y`, facing up or down.
    fn disk(&mut self, r: f32, y: f32, up: bool, segments: u32) {
        let n = [0.0, if up { 1.0 } else { -1.0 }, 0.0];
        let center = self.vertex([0.0, y, 0.0], n, [0.5, 0.5]);
        for i in 0..=segments {
            let theta = 2.0 * PI * i as f32 / segments as f32;
            let (s, c) = theta.sin_cos();
            let flip = if up { 1.0 } else { -1.0 };
            self.vertex([r * s, y, r * c], n, [0.5 + 0.5 * s, 0.5 - 0.5 * c * flip]);
        }
        for i in 0..segments {
            let (a, b) = (center + 1 + i, center + 2 + i);
            if up {
                self.p.indices.extend_from_slice(&[center, a, b]);
            } else {
                self.p.indices.extend_from_slice(&[center, b, a]);
            }
        }
    }

    fn finish(mut self) -> PrimitiveData {
        self.p.tangents = tangents(&self.p);
        self.p
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0 { v.map(|x| x / len) } else { [0.0, 1.0, 0.0] }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Per-vertex tangents (`xyz`, `w` = bitangent sign) from the UVs,
/// accumulated per triangle and orthogonalised against the normal.
pub fn tangents(p: &PrimitiveData) -> Vec<[f32; 4]> {
    let n = p.positions.len();
    let mut tan = vec![[0.0f32; 3]; n];
    let mut bit = vec![[0.0f32; 3]; n];
    for t in p.indices.chunks_exact(3) {
        let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
        if a >= n || b >= n || c >= n {
            continue;
        }
        let (e1, e2) = (sub(p.positions[b], p.positions[a]), sub(p.positions[c], p.positions[a]));
        let uv = |i: usize| p.uvs.get(i).copied().unwrap_or([0.0; 2]);
        let (du1, dv1) = (uv(b)[0] - uv(a)[0], uv(b)[1] - uv(a)[1]);
        let (du2, dv2) = (uv(c)[0] - uv(a)[0], uv(c)[1] - uv(a)[1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-12 {
            continue;
        }
        let k = 1.0 / det;
        let tv = [0, 1, 2].map(|i| (e1[i] * dv2 - e2[i] * dv1) * k);
        let bv = [0, 1, 2].map(|i| (e2[i] * du1 - e1[i] * du2) * k);
        for i in [a, b, c] {
            for x in 0..3 {
                tan[i][x] += tv[x];
                bit[i][x] += bv[x];
            }
        }
    }
    (0..n)
        .map(|i| {
            let normal = p.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
            let t = sub(tan[i], normal.map(|x| x * dot(normal, tan[i])));
            let mut t3 = normalize(t);
            if dot(t, t) < 1e-12 {
                // No UV gradient (pole, apex): any vector orthogonal to n.
                let helper = if normal[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
                t3 = normalize(cross(helper, normal));
            }
            let w = if dot(cross(normal, t3), bit[i]) < 0.0 { -1.0 } else { 1.0 };
            [t3[0], t3[1], t3[2], w]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Shape> {
        SHAPE_KINDS.iter().map(|k| Shape::default_of(k).unwrap()).collect()
    }

    #[test]
    fn every_kind_has_a_default_and_round_trips_its_id() {
        for shape in all() {
            assert_eq!(Shape::default_of(shape.kind()), Some(shape));
            assert!(shape.is_valid());
            assert!(shape.mesh_key().starts_with(MESH_PREFIX));
        }
        assert_eq!(Shape::default_of("teapot"), None);
    }

    #[test]
    fn generated_meshes_are_complete_and_outward_facing() {
        for shape in all() {
            let p = shape.primitive();
            let n = p.positions.len();
            assert!(n > 0 && !p.indices.is_empty(), "{shape:?}");
            assert_eq!(p.normals.len(), n);
            assert_eq!(p.uvs.len(), n);
            assert_eq!(p.tangents.len(), n);
            assert_eq!(p.indices.len() % 3, 0);
            for t in p.indices.chunks_exact(3) {
                let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
                assert!(a < n && b < n && c < n);
                let face = cross(
                    sub(p.positions[b], p.positions[a]),
                    sub(p.positions[c], p.positions[a]),
                );
                let avg = [0, 1, 2].map(|k| p.normals[a][k] + p.normals[b][k] + p.normals[c][k]);
                assert!(dot(face, face) > 0.0, "{shape:?}: degenerate triangle");
                assert!(dot(face, avg) > 0.0, "{shape:?}: inward triangle {t:?}");
            }
            for (i, nrm) in p.normals.iter().enumerate() {
                assert!((dot(*nrm, *nrm) - 1.0).abs() < 1e-4, "{shape:?}");
                let tg = p.tangents[i];
                assert!(dot([tg[0], tg[1], tg[2]], *nrm).abs() < 1e-3, "{shape:?}");
                assert!(tg[3] == 1.0 || tg[3] == -1.0);
            }
            for uv in &p.uvs {
                assert!((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
            }
            assert_eq!(shape.triangles(), (p.indices.len() / 3) as u64);
        }
    }

    #[test]
    fn sizes_are_respected() {
        let extent = |s: Shape| {
            let p = s.primitive();
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for v in &p.positions {
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            [0, 1, 2].map(|k| hi[k] - lo[k])
        };
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        assert!(close(extent(Shape::Cube { size: [1.0, 2.0, 3.0] }), [1.0, 2.0, 3.0]));
        assert!(close(extent(Shape::Plane { size: [4.0, 2.0], subdivisions: 3 }), [4.0, 0.0, 2.0]));
        assert!(close(
            extent(Shape::Capsule { radius: 0.5, height: 3.0, segments: 16, rings: 4 }),
            [1.0, 3.0, 1.0]
        ));
        assert!(close(
            extent(Shape::Cylinder { radius: 1.0, height: 2.0, segments: 64 }),
            [2.0, 2.0, 2.0]
        ));
        assert_eq!(Shape::Cube { size: [1.0; 3] }.triangles(), 12);
        assert_eq!(Shape::Plane { size: [1.0; 2], subdivisions: 2 }.triangles(), 8);
    }

    #[test]
    fn hostile_parameters_are_clamped() {
        let s = Shape::Sphere { radius: f32::NAN, segments: u32::MAX, rings: 0 };
        assert!(!s.is_valid());
        let p = s.primitive();
        assert!(p.positions.len() <= ((MAX_SEGMENTS + 1) * 3) as usize);
        assert!(p.positions.iter().flatten().all(|x| x.is_finite()));
    }

    #[test]
    fn params_apply_and_read_back() {
        for shape in all() {
            let mut copy = Shape::default_of(shape.kind()).unwrap();
            copy.apply(&shape.params()).unwrap();
            assert_eq!(copy, shape);
        }
        let mut s = Shape::default_of("cylinder").unwrap();
        s.apply(&ShapeParams { radius: Some(2.0), ..Default::default() }).unwrap();
        assert_eq!(s, Shape::Cylinder { radius: 2.0, height: 1.0, segments: 32 });
        let err = s.apply(&ShapeParams { tube: Some(1.0), ..Default::default() }).unwrap_err();
        assert!(err.contains("`tube`"), "{err}");
        let mut plane = Shape::default_of("plane").unwrap();
        plane.apply(&ShapeParams { size: Some([3.0, 4.0, 0.0]), ..Default::default() }).unwrap();
        assert_eq!(plane, Shape::Plane { size: [3.0, 4.0], subdivisions: 1 });
    }

    #[test]
    fn same_parameters_same_key() {
        let a = Shape::default_of("torus").unwrap();
        assert_eq!(a.mesh_key(), a.mesh_key());
        let b = Shape::Torus { radius: 0.5, tube: 0.2, segments: 32, sides: 16 };
        assert_ne!(a.mesh_key(), b.mesh_key());
    }
}
