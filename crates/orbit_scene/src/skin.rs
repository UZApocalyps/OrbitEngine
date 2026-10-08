//! Skeletons, poses and skinning (rigged glTF characters).
//!
//! A skinned [`MeshAsset`](crate::MeshAsset) carries its [`Skeleton`] and the
//! glTF animation [`Clip`]s that target it. A scene stores, per rigged node,
//! a [`Rig`]: the current [`Pose`] (joint name -> local transform override),
//! named poses and small authored key-frame clips ([`PoseClip`]). Poses are
//! keyed by joint *name* so that they survive a re-import of the model.
//!
//! Skinning is linear blend skinning done on the CPU ([`skin_mesh`]): the
//! renderer caches the result per (mesh, pose), so a still pose — the common
//! case in a visual novel — costs nothing per frame, and the vertex format
//! and pipelines stay those of static meshes (WebGL2 friendly).
//!
//! Matrices are column major `[[f32; 4]; 4]` (`m[3]` is the translation
//! column), like `orbit_render3d::math`.

use serde::{Deserialize, Serialize};

use crate::{MeshAsset, Node3dId, PrimitiveData, Transform};

pub type Mat4 = [[f32; 4]; 4];

pub const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

// ------------------------------------------------------------------ math

pub fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (c, col) in out.iter_mut().enumerate() {
        for (r, v) in col.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

/// Translation * rotation * scale.
pub fn trs(t: &Transform) -> Mat4 {
    let [x, y, z, w] = t.rotation;
    let [sx, sy, sz] = t.scale;
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        [
            (1.0 - 2.0 * (yy + zz)) * sx,
            2.0 * (xy + wz) * sx,
            2.0 * (xz - wy) * sx,
            0.0,
        ],
        [
            2.0 * (xy - wz) * sy,
            (1.0 - 2.0 * (xx + zz)) * sy,
            2.0 * (yz + wx) * sy,
            0.0,
        ],
        [
            2.0 * (xz + wy) * sz,
            2.0 * (yz - wx) * sz,
            (1.0 - 2.0 * (xx + yy)) * sz,
            0.0,
        ],
        [t.translation[0], t.translation[1], t.translation[2], 1.0],
    ]
}

/// Inverse of an affine matrix (`None` when singular).
pub fn inverse_affine(m: &Mat4) -> Option<Mat4> {
    let a = [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ];
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    // Row-major inverse of the 3x3 block.
    let i = [
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) * d,
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * d,
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * d,
        ],
        [
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) * d,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * d,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * d,
        ],
        [
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) * d,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * d,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * d,
        ],
    ];
    let t = m[3];
    let it: [f32; 3] = std::array::from_fn(|r| -(i[r][0] * t[0] + i[r][1] * t[1] + i[r][2] * t[2]));
    Some([
        [i[0][0], i[1][0], i[2][0], 0.0],
        [i[0][1], i[1][1], i[2][1], 0.0],
        [i[0][2], i[1][2], i[2][2], 0.0],
        [it[0], it[1], it[2], 1.0],
    ])
}

/// Translation / rotation / scale of an affine matrix without shear.
pub fn decompose(m: &Mat4) -> Transform {
    let len = |c: &[f32; 4]| (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
    let mut s = [len(&m[0]), len(&m[1]), len(&m[2])];
    let det = m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2])
        - m[1][0] * (m[0][1] * m[2][2] - m[2][1] * m[0][2])
        + m[2][0] * (m[0][1] * m[1][2] - m[1][1] * m[0][2]);
    if det < 0.0 {
        s[0] = -s[0];
    }
    let col = |i: usize| {
        let k = if s[i].abs() > 1e-12 { 1.0 / s[i] } else { 0.0 };
        [m[i][0] * k, m[i][1] * k, m[i][2] * k]
    };
    let (c0, c1, c2) = (col(0), col(1), col(2));
    // Rotation matrix (row r, column c) = c{c}[r].
    let (m00, m11, m22) = (c0[0], c1[1], c2[2]);
    let trace = m00 + m11 + m22;
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (c1[2] - c2[1]) / s,
            (c2[0] - c0[2]) / s,
            (c0[1] - c1[0]) / s,
            0.25 * s,
        ]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [
            0.25 * s,
            (c1[0] + c0[1]) / s,
            (c2[0] + c0[2]) / s,
            (c1[2] - c2[1]) / s,
        ]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [
            (c1[0] + c0[1]) / s,
            0.25 * s,
            (c2[1] + c1[2]) / s,
            (c2[0] - c0[2]) / s,
        ]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [
            (c2[0] + c0[2]) / s,
            (c2[1] + c1[2]) / s,
            0.25 * s,
            (c0[1] - c1[0]) / s,
        ]
    };
    Transform {
        translation: [m[3][0], m[3][1], m[3][2]],
        rotation: quat_normalize(q),
        scale: s,
    }
}

pub fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let l = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if l > 1e-12 {
        q.map(|v| v / l)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// Shortest-path normalized lerp (good enough between close keys) refined
/// into a true slerp for wide angles.
pub fn quat_slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
    let b = if dot < 0.0 {
        dot = -dot;
        b.map(|v| -v)
    } else {
        b
    };
    if dot > 0.9995 {
        return quat_normalize(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t));
    }
    let theta = dot.clamp(-1.0, 1.0).acos();
    let s = theta.sin();
    let (wa, wb) = (((1.0 - t) * theta).sin() / s, (t * theta).sin() / s);
    quat_normalize(std::array::from_fn(|i| a[i] * wa + b[i] * wb))
}

pub fn lerp_transform(a: &Transform, b: &Transform, t: f32) -> Transform {
    let l3 = |x: [f32; 3], y: [f32; 3]| std::array::from_fn(|i| x[i] + (y[i] - x[i]) * t);
    Transform {
        translation: l3(a.translation, b.translation),
        rotation: quat_slerp(a.rotation, b.rotation, t),
        scale: l3(a.scale, b.scale),
    }
}

fn transform_point(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

fn transform_vector(m: &Mat4, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2])
}

// -------------------------------------------------------------- skeleton

/// One bone of a [`Skeleton`].
#[derive(Debug, Clone, PartialEq)]
pub struct Joint {
    /// Unique within the skeleton; poses refer to joints by name.
    pub name: String,
    /// Index of the parent joint (`None` for a root).
    pub parent: Option<usize>,
    /// Rest local transform (relative to the parent joint).
    pub rest: Transform,
    /// Roots only: matrix placing the root's parent space in the space of
    /// the skinned mesh node (identity for every other joint).
    pub base: Mat4,
    /// glTF inverse bind matrix.
    pub inverse_bind: Mat4,
}

/// Joints in the order the vertex `joints` indices refer to.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
}

impl Skeleton {
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    /// Children of `parent` (`None`: the roots), in joint order.
    pub fn children(&self, parent: Option<usize>) -> Vec<usize> {
        (0..self.joints.len())
            .filter(|&i| self.joints[i].parent == parent)
            .collect()
    }

    /// Local transforms: the rest pose overridden by `pose`.
    pub fn locals(&self, pose: &Pose) -> Vec<Transform> {
        let mut out: Vec<Transform> = self.joints.iter().map(|j| j.rest).collect();
        for jp in &pose.joints {
            if let Some(i) = self.index_of(&jp.joint) {
                out[i] = jp.transform;
            }
        }
        out
    }

    /// Joint matrices in the skinned mesh node space.
    pub fn globals(&self, locals: &[Transform]) -> Vec<Mat4> {
        let n = self.joints.len();
        let mut out: Vec<Option<Mat4>> = vec![None; n];
        for i in 0..n {
            self.global_of(i, locals, &mut out, 0);
        }
        out.into_iter().map(|m| m.unwrap_or(IDENTITY)).collect()
    }

    fn global_of(
        &self,
        i: usize,
        locals: &[Transform],
        memo: &mut [Option<Mat4>],
        depth: usize,
    ) -> Mat4 {
        if let Some(m) = memo[i] {
            return m;
        }
        let j = &self.joints[i];
        let local = trs(&locals[i]);
        let m = match j.parent {
            // Depth guard against a malformed (cyclic) parent chain.
            Some(p) if p < self.joints.len() && depth < 256 => {
                mat_mul(&self.global_of(p, locals, memo, depth + 1), &local)
            }
            _ => mat_mul(&j.base, &local),
        };
        memo[i] = Some(m);
        m
    }

    /// Skinning matrices (`global * inverse_bind`) for `pose`.
    pub fn palette(&self, pose: &Pose) -> Vec<Mat4> {
        let globals = self.globals(&self.locals(pose));
        globals
            .iter()
            .zip(&self.joints)
            .map(|(g, j)| mat_mul(g, &j.inverse_bind))
            .collect()
    }
}

/// Linear blend skinning of one primitive. Vertices without weights (or a
/// primitive without skin attributes) are returned unchanged.
pub fn skin_primitive(p: &PrimitiveData, palette: &[Mat4]) -> PrimitiveData {
    let n = p.positions.len();
    if p.joints.len() != n || p.weights.len() != n || palette.is_empty() {
        return p.clone();
    }
    let mut out = p.clone();
    for v in 0..n {
        let (js, ws) = (p.joints[v], p.weights[v]);
        let total: f32 = ws.iter().sum();
        if total <= 1e-6 {
            continue;
        }
        let mut m = [[0.0f32; 4]; 4];
        for k in 0..4 {
            let w = ws[k] / total;
            if w == 0.0 {
                continue;
            }
            let Some(jm) = palette.get(usize::from(js[k])) else {
                continue;
            };
            for c in 0..4 {
                for r in 0..4 {
                    m[c][r] += jm[c][r] * w;
                }
            }
        }
        out.positions[v] = transform_point(&m, p.positions[v]);
        if let Some(nrm) = p.normals.get(v) {
            let t = transform_vector(&m, *nrm);
            let l = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
            out.normals[v] = if l > 1e-12 { t.map(|x| x / l) } else { *nrm };
        }
        if let Some(tg) = p.tangents.get(v) {
            let t = transform_vector(&m, [tg[0], tg[1], tg[2]]);
            let l = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-12);
            out.tangents[v] = [t[0] / l, t[1] / l, t[2] / l, tg[3]];
        }
    }
    out
}

/// `mesh` deformed by `pose` (a copy of the mesh when it has no skeleton).
/// The result keeps no skin data: it is ready to upload as a static mesh.
pub fn skin_mesh(mesh: &MeshAsset, pose: &Pose, name: String) -> MeshAsset {
    let primitives = match &mesh.skin {
        Some(skel) => {
            let palette = skel.palette(pose);
            mesh.primitives
                .iter()
                .map(|p| {
                    let mut s = skin_primitive(p, &palette);
                    s.joints.clear();
                    s.weights.clear();
                    s
                })
                .collect()
        }
        None => mesh.primitives.clone(),
    };
    MeshAsset {
        name,
        primitives,
        skin: None,
        clips: Vec::new(),
    }
}

// ------------------------------------------------------------- animation

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelPath {
    Translation,
    Rotation,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
}

/// Key frames of one joint property. `values` are `[x, y, z, 0]` for
/// translation / scale and quaternions for rotation.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub joint: usize,
    pub path: ChannelPath,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<[f32; 4]>,
}

impl Channel {
    fn sample(&self, t: f32) -> Option<[f32; 4]> {
        let n = self.times.len().min(self.values.len());
        if n == 0 {
            return None;
        }
        if t <= self.times[0] {
            return Some(self.values[0]);
        }
        if t >= self.times[n - 1] {
            return Some(self.values[n - 1]);
        }
        let k = self.times[..n].partition_point(|&x| x <= t).max(1) - 1;
        let (a, b) = (self.values[k], self.values[k + 1]);
        if self.interpolation == Interpolation::Step {
            return Some(a);
        }
        let span = (self.times[k + 1] - self.times[k]).max(1e-6);
        let f = ((t - self.times[k]) / span).clamp(0.0, 1.0);
        Some(match self.path {
            ChannelPath::Rotation => quat_slerp(a, b, f),
            _ => std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f),
        })
    }
}

/// A glTF animation restricted to one skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<Channel>,
}

impl Clip {
    /// The pose at time `t` (seconds, clamped): every animated joint.
    pub fn sample(&self, skeleton: &Skeleton, t: f32) -> Pose {
        let mut locals: Vec<Option<Transform>> = vec![None; skeleton.joints.len()];
        for c in &self.channels {
            let (Some(v), Some(j)) = (c.sample(t), skeleton.joints.get(c.joint)) else {
                continue;
            };
            let tr = locals[c.joint].get_or_insert(j.rest);
            match c.path {
                ChannelPath::Translation => tr.translation = [v[0], v[1], v[2]],
                ChannelPath::Rotation => tr.rotation = quat_normalize(v),
                ChannelPath::Scale => tr.scale = [v[0], v[1], v[2]],
            }
        }
        Pose {
            joints: locals
                .into_iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    t.map(|transform| JointPose {
                        joint: skeleton.joints[i].name.clone(),
                        transform,
                    })
                })
                .collect(),
        }
    }
}

// --------------------------------------------------------- scene data

/// Local transform override of one joint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointPose {
    pub joint: String,
    pub transform: Transform,
}

/// Joint overrides; joints not listed keep their rest transform.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Pose {
    #[serde(default)]
    pub joints: Vec<JointPose>,
}

impl Pose {
    pub fn is_rest(&self) -> bool {
        self.joints.is_empty()
    }

    pub fn get(&self, joint: &str) -> Option<&Transform> {
        self.joints
            .iter()
            .find(|j| j.joint == joint)
            .map(|j| &j.transform)
    }

    /// Sets (or adds) the override of `joint`.
    pub fn set(&mut self, joint: &str, transform: Transform) {
        match self.joints.iter_mut().find(|j| j.joint == joint) {
            Some(j) => j.transform = transform,
            None => self.joints.push(JointPose {
                joint: joint.to_owned(),
                transform,
            }),
        }
    }

    pub fn reset(&mut self, joint: &str) {
        self.joints.retain(|j| j.joint != joint);
    }

    /// Stable hash of the pose (renderer cache key).
    pub fn hash64(&self) -> u64 {
        // FNV-1a over names and bit patterns: deterministic across runs.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for b in bytes {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for j in &self.joints {
            eat(j.joint.as_bytes());
            eat(&[0xff]);
            let t = &j.transform;
            for v in t.translation.iter().chain(&t.rotation).chain(&t.scale) {
                eat(&v.to_bits().to_le_bytes());
            }
        }
        h
    }

    /// Interpolates two poses joint by joint (missing joints: `rest_of`).
    pub fn blend(a: &Pose, b: &Pose, t: f32, rest_of: &dyn Fn(&str) -> Transform) -> Pose {
        let mut names: Vec<&str> = a.joints.iter().map(|j| j.joint.as_str()).collect();
        for j in &b.joints {
            if !names.contains(&j.joint.as_str()) {
                names.push(&j.joint);
            }
        }
        Pose {
            joints: names
                .into_iter()
                .map(|n| {
                    let ta = a.get(n).copied().unwrap_or_else(|| rest_of(n));
                    let tb = b.get(n).copied().unwrap_or_else(|| rest_of(n));
                    JointPose {
                        joint: n.to_owned(),
                        transform: lerp_transform(&ta, &tb, t),
                    }
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedPose {
    pub name: String,
    pub pose: Pose,
}

/// One key of an authored clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoseKey {
    /// Seconds from the clip start.
    pub time: f32,
    pub pose: Pose,
}

/// A small key-frame clip authored in the editor (poses interpolated).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PoseClip {
    pub name: String,
    #[serde(default)]
    pub keys: Vec<PoseKey>,
    #[serde(default)]
    pub looping: bool,
}

impl PoseClip {
    pub fn duration(&self) -> f32 {
        self.keys.iter().map(|k| k.time).fold(0.0, f32::max)
    }

    /// Inserts `pose` at `time`, replacing a key at the same time.
    pub fn set_key(&mut self, time: f32, pose: Pose) {
        let time = time.max(0.0);
        match self.keys.iter_mut().find(|k| (k.time - time).abs() < 1e-3) {
            Some(k) => k.pose = pose,
            None => {
                self.keys.push(PoseKey { time, pose });
                self.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            }
        }
    }

    pub fn sample(&self, t: f32, rest_of: &dyn Fn(&str) -> Transform) -> Pose {
        let Some(first) = self.keys.first() else {
            return Pose::default();
        };
        let d = self.duration();
        let t = if self.looping && d > 0.0 {
            t.rem_euclid(d)
        } else {
            t
        };
        if t <= first.time {
            return first.pose.clone();
        }
        for w in self.keys.windows(2) {
            if t <= w[1].time {
                let span = (w[1].time - w[0].time).max(1e-6);
                return Pose::blend(&w[0].pose, &w[1].pose, (t - w[0].time) / span, rest_of);
            }
        }
        self.keys.last().map(|k| k.pose.clone()).unwrap_or_default()
    }
}

/// Pose state of one rigged scene node (a skinned `Mesh` node or a prefab
/// instance containing skinned meshes): applies to every skinned mesh under
/// it whose skeleton has the posed joint names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rig {
    pub node: Node3dId,
    /// Current pose (what the game shows).
    #[serde(default)]
    pub pose: Pose,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub poses: Vec<NamedPose>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<PoseClip>,
}

impl Rig {
    pub fn new(node: Node3dId) -> Self {
        Self {
            node,
            pose: Pose::default(),
            poses: Vec::new(),
            clips: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Two bones along +Y: root at the origin, `tip` one unit up.
    pub(crate) fn two_bones() -> Skeleton {
        let tip_rest = Transform {
            translation: [0.0, 1.0, 0.0],
            ..Transform::IDENTITY
        };
        Skeleton {
            joints: vec![
                Joint {
                    name: "root".into(),
                    parent: None,
                    rest: Transform::IDENTITY,
                    base: IDENTITY,
                    inverse_bind: IDENTITY,
                },
                Joint {
                    name: "tip".into(),
                    parent: Some(0),
                    rest: tip_rest,
                    base: IDENTITY,
                    inverse_bind: inverse_affine(&trs(&tip_rest)).unwrap(),
                },
            ],
        }
    }

    #[test]
    fn rest_palette_is_identity() {
        let s = two_bones();
        for m in s.palette(&Pose::default()) {
            for c in 0..4 {
                for r in 0..4 {
                    assert!(approx(m[c][r], IDENTITY[c][r]), "{m:?}");
                }
            }
        }
    }

    #[test]
    fn rotating_the_tip_moves_only_its_vertices() {
        let s = two_bones();
        let mut pose = Pose::default();
        let half = std::f32::consts::FRAC_PI_4; // 90 degrees about Z
        pose.set(
            "tip",
            Transform {
                translation: [0.0, 1.0, 0.0],
                rotation: [0.0, 0.0, half.sin(), half.cos()],
                scale: [1.0; 3],
            },
        );
        let prim = PrimitiveData {
            positions: vec![[0.0, 0.5, 0.0], [0.0, 2.0, 0.0], [0.0, 1.5, 0.0]],
            normals: vec![[1.0, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            joints: vec![[0, 0, 0, 0], [1, 0, 0, 0], [0, 1, 0, 0]],
            weights: vec![
                [1.0, 0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [0.5, 0.5, 0.0, 0.0],
            ],
            ..Default::default()
        };
        let out = skin_primitive(&prim, &s.palette(&pose));
        assert_eq!(out.positions[0], [0.0, 0.5, 0.0]);
        // (0, 2) relative to the tip (0, 1) rotated 90 degrees: (-1, 1).
        assert!(approx(out.positions[1][0], -1.0) && approx(out.positions[1][1], 1.0));
        assert!(approx(out.normals[1][1], 1.0), "{:?}", out.normals[1]);
        // Half / half blend: midpoint of (0, 1.5) and (-0.5, 1).
        assert!(approx(out.positions[2][0], -0.25) && approx(out.positions[2][1], 1.25));
    }

    #[test]
    fn decompose_inverts_trs() {
        let t = Transform {
            translation: [1.0, -2.0, 3.0],
            rotation: quat_normalize([0.2, 0.4, -0.1, 0.9]),
            scale: [2.0, 0.5, 1.5],
        };
        let d = decompose(&trs(&t));
        for i in 0..3 {
            assert!(approx(d.translation[i], t.translation[i]));
            assert!(approx(d.scale[i], t.scale[i]));
        }
        let dot: f32 = (0..4).map(|i| d.rotation[i] * t.rotation[i]).sum();
        assert!(approx(dot.abs(), 1.0));
        let inv = inverse_affine(&trs(&t)).unwrap();
        let id = mat_mul(&trs(&t), &inv);
        for c in 0..4 {
            for r in 0..4 {
                assert!(approx(id[c][r], IDENTITY[c][r]));
            }
        }
    }

    #[test]
    fn clips_sample_linearly_and_clamp() {
        let s = two_bones();
        let clip = Clip {
            name: "wave".into(),
            duration: 2.0,
            channels: vec![Channel {
                joint: 1,
                path: ChannelPath::Translation,
                interpolation: Interpolation::Linear,
                times: vec![0.0, 2.0],
                values: vec![[0.0, 1.0, 0.0, 0.0], [0.0, 3.0, 0.0, 0.0]],
            }],
        };
        assert_eq!(
            clip.sample(&s, 1.0).get("tip").unwrap().translation,
            [0.0, 2.0, 0.0]
        );
        assert_eq!(
            clip.sample(&s, 9.0).get("tip").unwrap().translation,
            [0.0, 3.0, 0.0]
        );
        assert!(clip.sample(&s, 1.0).get("root").is_none());
    }

    #[test]
    fn pose_clips_interpolate_keys() {
        let mut a = Pose::default();
        a.set("tip", Transform::IDENTITY);
        let mut b = Pose::default();
        b.set(
            "tip",
            Transform {
                translation: [2.0, 0.0, 0.0],
                ..Transform::IDENTITY
            },
        );
        let mut clip = PoseClip {
            name: "c".into(),
            ..Default::default()
        };
        clip.set_key(1.0, b);
        clip.set_key(0.0, a);
        assert_eq!(clip.keys[0].time, 0.0);
        let rest = |_: &str| Transform::IDENTITY;
        assert_eq!(
            clip.sample(0.5, &rest).get("tip").unwrap().translation,
            [1.0, 0.0, 0.0]
        );
        clip.looping = true;
        assert_eq!(
            clip.sample(1.5, &rest).get("tip").unwrap().translation,
            [1.0, 0.0, 0.0]
        );
    }

    #[test]
    fn rigs_round_trip_through_ron() {
        let mut rig = Rig::new(Node3dId::new());
        rig.pose.set("tip", Transform::IDENTITY);
        rig.poses.push(NamedPose {
            name: "wave".into(),
            pose: rig.pose.clone(),
        });
        let mut clip = PoseClip {
            name: "nod".into(),
            looping: true,
            ..Default::default()
        };
        clip.set_key(0.5, rig.pose.clone());
        rig.clips.push(clip);
        let text = ron::to_string(&rig).unwrap();
        assert_eq!(ron::from_str::<Rig>(&text).unwrap(), rig);
        assert_ne!(Pose::default().hash64(), rig.pose.hash64());
    }

    #[test]
    fn scenes_without_rigs_are_unchanged_and_old_files_load() {
        // A scene without poses serializes exactly as before (no `rigs`).
        let plain = crate::Scene3d::new("room");
        let text = crate::to_ron(&plain).unwrap();
        assert!(!text.contains("rigs"));
        assert_eq!(crate::from_ron(&text).unwrap(), plain);
        // A v1 file (no version, no rigs) still migrates and loads.
        let v1 = text.replace("scene3d_version: 2,", "");
        let back = crate::from_ron(&v1).unwrap();
        assert!(back.rigs.is_empty());
        // Poses round-trip, and a dangling one is only a warning.
        let mut s = plain.clone();
        s.rig_mut(Node3dId::new())
            .pose
            .set("arm", Transform::IDENTITY);
        let text = crate::to_ron(&s).unwrap();
        assert_eq!(crate::from_ron(&text).unwrap(), s);
        let issues = crate::validate(&s);
        assert!(
            issues
                .iter()
                .all(|i| i.severity == crate::Severity::Warning)
                && !issues.is_empty()
        );
        s.rigs[0].pose.joints[0].transform.rotation[0] = f32::NAN;
        assert!(
            crate::validate(&s)
                .iter()
                .any(|i| i.severity == crate::Severity::Error)
        );
    }
}
