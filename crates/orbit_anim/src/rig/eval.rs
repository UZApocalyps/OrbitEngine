//! Pose evaluation: ids resolved once ([`PreparedRig`]), clip sampling,
//! cross-fades, IK, world matrices, CPU skinning and the draw list.

use std::collections::BTreeMap;

use super::math::{Affine, Xform};
use super::model::{
    Attachment, AttachmentId, ClipBlend, MeshAttachment, RegionAttachment, Rig, RigClip, RigTrack,
};
use super::validate::{RigIssue, Severity};
use crate::curve::Curve;
use crate::lerp::{Angle, Lerp, Rgba};

/// A mesh's influences resolved to bone indices, normalized, in CSR form.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreparedMesh {
    /// Per vertex `offsets[i]..offsets[i + 1]` into `influences`. Empty when
    /// the mesh follows its slot's bone rigidly.
    pub offsets: Vec<u32>,
    pub influences: Vec<(u16, f32)>,
}

impl PreparedMesh {
    pub fn is_rigid(&self) -> bool {
        self.offsets.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PreparedIk {
    bones: Vec<usize>,
    target: usize,
    bend_positive: bool,
    mix: f32,
}

/// A validated rig with ids resolved to indices: build once per loaded asset,
/// evaluate every frame without string lookups.
#[derive(Clone, Debug)]
pub struct PreparedRig {
    rig: Rig,
    bone_index: BTreeMap<String, usize>,
    slot_index: BTreeMap<String, usize>,
    parents: Vec<Option<usize>>,
    slot_bones: Vec<usize>,
    inv_bind: Vec<Affine>,
    default_skin: Option<usize>,
    /// Per skin, per mesh attachment.
    meshes: Vec<BTreeMap<AttachmentId, PreparedMesh>>,
    ik: Vec<PreparedIk>,
    /// Warnings found while preparing (weights normalized, budget...).
    pub warnings: Vec<RigIssue>,
}

/// Local transforms and slot states of a rig at one instant.
#[derive(Clone, Debug, PartialEq)]
pub struct RigPose {
    /// Local transform per bone (rig order).
    pub bones: Vec<Xform>,
    pub slots: Vec<SlotPose>,
    /// Vertex offsets per (slot index, mesh attachment).
    pub deforms: BTreeMap<(usize, AttachmentId), Vec<[f32; 2]>>,
    /// Slot indices, back to front.
    pub draw_order: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlotPose {
    pub attachment: Option<AttachmentId>,
    pub color: Rgba,
}

/// World matrices of a pose (after IK).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldPose {
    /// World matrix per bone.
    pub bones: Vec<Affine>,
    /// `bones[i] * inverse_bind[i]`: maps bind-space vertices to the pose.
    pub skinning: Vec<Affine>,
}

/// Skins mesh vertices. CPU now; a GPU implementation can come later without
/// changing the format (ADR 0013 §5).
pub trait Skinner {
    /// Writes one deformed position per vertex of `job` into `out` (cleared
    /// first).
    fn skin(&self, job: &SkinJob<'_>, out: &mut Vec<[f32; 2]>);
}

/// Inputs of one mesh skinning.
pub struct SkinJob<'a> {
    /// Bind-space positions.
    pub vertices: &'a [[f32; 2]],
    /// Optional per-vertex offsets added before skinning.
    pub deform: Option<&'a [[f32; 2]]>,
    pub mesh: &'a PreparedMesh,
    /// [`WorldPose::skinning`].
    pub skinning: &'a [Affine],
    /// Bone followed by a rigid mesh.
    pub slot_bone: usize,
}

/// Linear blend skinning on the CPU.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuSkinner;

impl Skinner for CpuSkinner {
    fn skin(&self, job: &SkinJob<'_>, out: &mut Vec<[f32; 2]>) {
        out.clear();
        out.reserve(job.vertices.len());
        let rigid = job.skinning.get(job.slot_bone).copied().unwrap_or(Affine::IDENTITY);
        for (i, v) in job.vertices.iter().enumerate() {
            let mut p = *v;
            if let Some(d) = job.deform.and_then(|d| d.get(i)) {
                p = [p[0] + d[0], p[1] + d[1]];
            }
            if job.mesh.is_rigid() {
                out.push(rigid.apply(p));
                continue;
            }
            let (s, e) = (job.mesh.offsets[i] as usize, job.mesh.offsets[i + 1] as usize);
            let mut m = Affine::ZERO;
            for &(bone, w) in &job.mesh.influences[s..e] {
                if let Some(b) = job.skinning.get(bone as usize) {
                    Affine::add_scaled(&mut m, b, w);
                }
            }
            out.push(m.apply(p));
        }
    }
}

/// What to draw for one visible slot.
#[derive(Clone, Debug, PartialEq)]
pub enum RigDrawItem<'a> {
    /// A rigid picture: the quad `[-w/2, -h/2]..[w/2, h/2]` mapped by
    /// `transform` (rig space).
    Region { slot: usize, image: &'a str, size: [f32; 2], transform: Affine, color: Rgba },
    /// Skinned triangles; positions are `RigDrawList::positions[range]`.
    Mesh {
        slot: usize,
        image: &'a str,
        range: core::ops::Range<usize>,
        uvs: &'a [[f32; 2]],
        triangles: &'a [[u16; 3]],
        color: Rgba,
    },
}

/// Everything needed to draw a posed rig, back to front, in rig space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RigDrawList<'a> {
    pub items: Vec<RigDrawItem<'a>>,
    pub positions: Vec<[f32; 2]>,
}

fn normalize_influences(list: &[(usize, f32)]) -> (Vec<(u16, f32)>, bool) {
    let mut v: Vec<(usize, f32)> = list.iter().copied().filter(|(_, w)| w.is_finite() && *w > 0.0).collect();
    // Keep the 4 heaviest.
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    let truncated = v.len() > 4;
    v.truncate(4);
    let sum: f32 = v.iter().map(|x| x.1).sum();
    let changed = truncated || (sum - 1.0).abs() > 1e-3;
    if sum <= 0.0 {
        return (Vec::new(), true);
    }
    (v.into_iter().map(|(b, w)| (b as u16, w / sum)).collect(), changed)
}

impl PreparedRig {
    /// Resolves the rig. Fails with the blocking issues of
    /// [`Rig::validate`]; warnings are kept in [`PreparedRig::warnings`].
    pub fn new(rig: Rig) -> Result<Self, Vec<RigIssue>> {
        let issues = rig.validate();
        if issues.iter().any(|i| i.severity == Severity::Error) {
            return Err(issues.into_iter().filter(|i| i.severity == Severity::Error).collect());
        }
        let bone_index: BTreeMap<String, usize> =
            rig.bones.iter().enumerate().map(|(i, b)| (b.id.0.clone(), i)).collect();
        let slot_index: BTreeMap<String, usize> =
            rig.slots.iter().enumerate().map(|(i, s)| (s.id.0.clone(), i)).collect();
        let parents: Vec<Option<usize>> =
            rig.bones.iter().map(|b| b.parent.as_ref().and_then(|p| bone_index.get(&p.0).copied())).collect();
        let slot_bones = rig.slots.iter().map(|s| bone_index[&s.bone.0]).collect();
        let default_skin = rig.skins.iter().position(|s| s.name == "default").or(if rig.skins.is_empty() {
            None
        } else {
            Some(0)
        });
        let meshes = rig
            .skins
            .iter()
            .map(|skin| {
                skin.attachments
                    .iter()
                    .filter_map(|(id, a)| match a {
                        Attachment::Mesh(m) => Some((id.clone(), Self::prepare_mesh(m, &bone_index))),
                        Attachment::Region(_) => None,
                    })
                    .collect()
            })
            .collect();
        let ik = rig
            .ik
            .iter()
            .map(|c| PreparedIk {
                bones: c.bones.iter().map(|b| bone_index[&b.0]).collect(),
                target: bone_index[&c.target.0],
                bend_positive: c.bend_positive,
                mix: c.mix.clamp(0.0, 1.0),
            })
            .collect();
        let mut prepared = Self {
            rig,
            bone_index,
            slot_index,
            parents,
            slot_bones,
            inv_bind: Vec::new(),
            default_skin,
            meshes,
            ik,
            warnings: issues,
        };
        let rest: Vec<Xform> = prepared.rig.bones.iter().map(|b| b.rest).collect();
        let bind = prepared.world_matrices(&rest);
        prepared.inv_bind = bind.iter().map(|m| m.inverse().unwrap_or(Affine::IDENTITY)).collect();
        Ok(prepared)
    }

    fn prepare_mesh(m: &MeshAttachment, bones: &BTreeMap<String, usize>) -> PreparedMesh {
        if m.weights.is_empty() {
            return PreparedMesh::default();
        }
        let mut out = PreparedMesh { offsets: vec![0], influences: Vec::new() };
        for i in 0..m.vertices.len() {
            let list: Vec<(usize, f32)> = m
                .weights
                .get(i)
                .map(|w| w.iter().filter_map(|(b, w)| bones.get(&b.0).map(|&i| (i, *w))).collect())
                .unwrap_or_default();
            out.influences.extend(normalize_influences(&list).0);
            out.offsets.push(out.influences.len() as u32);
        }
        out
    }

    pub fn rig(&self) -> &Rig {
        &self.rig
    }

    pub fn bone_index(&self, id: &str) -> Option<usize> {
        self.bone_index.get(id).copied()
    }

    pub fn slot_index(&self, id: &str) -> Option<usize> {
        self.slot_index.get(id).copied()
    }

    /// Inverse of the setup-pose world matrix of each bone.
    pub fn inverse_bind(&self) -> &[Affine] {
        &self.inv_bind
    }

    /// The setup pose.
    pub fn setup_pose(&self) -> RigPose {
        RigPose {
            bones: self.rig.bones.iter().map(|b| b.rest).collect(),
            slots: self
                .rig
                .slots
                .iter()
                .map(|s| SlotPose { attachment: s.attachment.clone(), color: s.color })
                .collect(),
            deforms: BTreeMap::new(),
            draw_order: (0..self.rig.slots.len()).collect(),
        }
    }

    /// Samples the clip named `name` at elapsed time `t` (setup pose when the
    /// clip is unknown).
    pub fn sample(&self, name: &str, t: f32) -> RigPose {
        self.sample_with(name, t, |_| None)
    }

    /// Like [`PreparedRig::sample`], resolving named easing curves through
    /// `resolve` (the story's curve table).
    pub fn sample_with<'a>(&self, name: &str, t: f32, resolve: impl Fn(&str) -> Option<&'a Curve> + Copy) -> RigPose {
        match self.rig.clips.get(name) {
            Some(clip) => self.sample_clip_with(clip, t, resolve),
            None => self.setup_pose(),
        }
    }

    /// Samples `clip` at elapsed time `t` (looping applied).
    pub fn sample_clip(&self, clip: &RigClip, t: f32) -> RigPose {
        self.sample_clip_with(clip, t, |_| None)
    }

    /// Like [`PreparedRig::sample_clip`] with a named-curve table.
    pub fn sample_clip_with<'a>(
        &self,
        clip: &RigClip,
        t: f32,
        resolve: impl Fn(&str) -> Option<&'a Curve> + Copy,
    ) -> RigPose {
        self.sample_clip_local_with(clip, clip.local_time(t), resolve)
    }

    /// Samples `clip` at a time already inside one pass (no looping applied):
    /// what an editor scrubbing a timeline wants.
    pub fn sample_clip_local_with<'a>(
        &self,
        clip: &RigClip,
        local: f32,
        resolve: impl Fn(&str) -> Option<&'a Curve> + Copy,
    ) -> RigPose {
        let mut pose = self.setup_pose();
        for track in &clip.tracks {
            self.apply_track(&mut pose, track, local, resolve);
        }
        pose
    }

    fn apply_track<'a>(
        &self,
        pose: &mut RigPose,
        track: &RigTrack,
        t: f32,
        resolve: impl Fn(&str) -> Option<&'a Curve> + Copy,
    ) {
        let bone = |id: &str| self.bone_index.get(id).copied();
        let slot = |id: &str| self.slot_index.get(id).copied();
        match track {
            RigTrack::BoneRotate { bone: b, track } => {
                if let (Some(i), Some(v)) = (bone(&b.0), track.sample_with(t, resolve)) {
                    pose.bones[i].rotation = self.rig.bones[i].rest.rotation + v.0;
                }
            }
            RigTrack::BoneTranslate { bone: b, track } => {
                if let (Some(i), Some(v)) = (bone(&b.0), track.sample_with(t, resolve)) {
                    let r = self.rig.bones[i].rest;
                    pose.bones[i].x = r.x + v.x;
                    pose.bones[i].y = r.y + v.y;
                }
            }
            RigTrack::BoneScale { bone: b, track } => {
                if let (Some(i), Some(v)) = (bone(&b.0), track.sample_with(t, resolve)) {
                    let r = self.rig.bones[i].rest;
                    pose.bones[i].sx = r.sx * v.x;
                    pose.bones[i].sy = r.sy * v.y;
                }
            }
            RigTrack::SlotAttachment { slot: s, track } => {
                if let (Some(i), Some(v)) = (slot(&s.0), track.sample_with(t, resolve)) {
                    pose.slots[i].attachment = v.0;
                }
            }
            RigTrack::SlotColor { slot: s, track } => {
                if let (Some(i), Some(v)) = (slot(&s.0), track.sample_with(t, resolve)) {
                    pose.slots[i].color = v;
                }
            }
            RigTrack::Deform { slot: s, attachment, track } => {
                if let (Some(i), Some(v)) = (slot(&s.0), track.sample_with(t, resolve)) {
                    pose.deforms.insert((i, attachment.clone()), v.0);
                }
            }
            RigTrack::DrawOrder { track } => {
                if let Some(v) = track.sample_with(t, resolve) {
                    let mut order: Vec<usize> = Vec::with_capacity(pose.slots.len());
                    for id in &v.0 {
                        if let Some(i) = slot(&id.0)
                            && !order.contains(&i)
                        {
                            order.push(i);
                        }
                    }
                    for i in 0..pose.slots.len() {
                        if !order.contains(&i) {
                            order.push(i);
                        }
                    }
                    pose.draw_order = order;
                }
            }
        }
    }

    /// Samples a cross-fade at clock time `now`.
    pub fn sample_blend(&self, state: &ClipBlend, now: f32) -> RigPose {
        self.sample_blend_with(state, now, |_| None)
    }

    /// Like [`PreparedRig::sample_blend`] with a named-curve table.
    pub fn sample_blend_with<'a>(
        &self,
        state: &ClipBlend,
        now: f32,
        resolve: impl Fn(&str) -> Option<&'a Curve> + Copy,
    ) -> RigPose {
        let cur = self.sample_with(&state.current.clip, now - state.current.started_at, resolve);
        let w = state.weight(now);
        match &state.previous {
            Some(prev) if w < 1.0 => {
                let old = self.sample_with(&prev.clip, now - prev.started_at, resolve);
                RigPose::blend(&old, &cur, w)
            }
            _ => cur,
        }
    }

    /// World matrices of local transforms, no IK.
    pub fn world_matrices(&self, locals: &[Xform]) -> Vec<Affine> {
        let mut out: Vec<Affine> = Vec::with_capacity(locals.len());
        for (i, l) in locals.iter().enumerate() {
            let m = l.matrix();
            let w = match self.parents.get(i).copied().flatten() {
                Some(p) if p < out.len() => out[p].mul(&m),
                _ => m,
            };
            out.push(w);
        }
        out
    }

    /// World pose (IK applied, in declaration order).
    pub fn world(&self, pose: &RigPose) -> WorldPose {
        let mut locals = pose.bones.clone();
        let mut world = self.world_matrices(&locals);
        for ik in &self.ik {
            if ik.mix <= 0.0 {
                continue;
            }
            match ik.bones.as_slice() {
                [a] => self.ik_one(&mut locals, &world, *a, ik.target, ik.mix),
                [a, b] => self.ik_two(&mut locals, &world, *a, *b, ik),
                _ => continue,
            }
            world = self.world_matrices(&locals);
        }
        let skinning = world.iter().zip(&self.inv_bind).map(|(w, i)| w.mul(i)).collect();
        WorldPose { bones: world, skinning }
    }

    /// Target position in the parent space of `bone`.
    fn target_in_parent(&self, world: &[Affine], bone: usize, target: usize) -> Option<[f32; 2]> {
        let parent = match self.parents[bone] {
            Some(p) => world[p],
            None => Affine::IDENTITY,
        };
        Some(parent.inverse()?.apply(world[target].origin()))
    }

    fn ik_one(&self, locals: &mut [Xform], world: &[Affine], a: usize, target: usize, mix: f32) {
        let Some(p) = self.target_in_parent(world, a, target) else { return };
        let l = locals[a];
        let (dx, dy) = (p[0] - l.x, p[1] - l.y);
        if dx * dx + dy * dy < 1e-12 {
            return;
        }
        let mut angle = libm::atan2f(dy, dx).to_degrees();
        if l.sx < 0.0 {
            angle += 180.0;
        }
        locals[a].rotation = Angle::lerp(&Angle(l.rotation), &Angle(angle), mix).0;
    }

    /// Law of cosines. Assumes a uniform, positive scale on the parent bone.
    fn ik_two(&self, locals: &mut [Xform], world: &[Affine], a: usize, b: usize, ik: &PreparedIk) {
        let Some(p) = self.target_in_parent(world, a, ik.target) else { return };
        let (la, lb) = (locals[a], locals[b]);
        let seg = [lb.x * la.sx, lb.y * la.sy];
        let l1 = libm::hypotf(seg[0], seg[1]);
        let l2 = self.rig.bones[b].length * (lb.sx * la.sx).abs();
        if l1 < 1e-6 || l2 < 1e-6 {
            self.ik_one(locals, world, a, ik.target, ik.mix);
            return;
        }
        let (dx, dy) = (p[0] - la.x, p[1] - la.y);
        let dist = libm::hypotf(dx, dy).clamp((l1 - l2).abs() + 1e-5, l1 + l2 - 1e-5).max(1e-5);
        let s = if ik.bend_positive { 1.0 } else { -1.0 };
        let cos_a = ((l1 * l1 + dist * dist - l2 * l2) / (2.0 * l1 * dist)).clamp(-1.0, 1.0);
        let cos_g = ((l1 * l1 + l2 * l2 - dist * dist) / (2.0 * l1 * l2)).clamp(-1.0, 1.0);
        let alpha = libm::acosf(cos_a);
        let gamma = libm::acosf(cos_g);
        let offset = libm::atan2f(seg[1], seg[0]);
        let theta1 = libm::atan2f(dy, dx) - s * alpha;
        let rot_a = (theta1 - offset).to_degrees();
        let rot_b = (offset + s * (core::f32::consts::PI - gamma)).to_degrees();
        locals[a].rotation = Angle::lerp(&Angle(la.rotation), &Angle(rot_a), ik.mix).0;
        locals[b].rotation = Angle::lerp(&Angle(lb.rotation), &Angle(rot_b), ik.mix).0;
    }

    /// The attachment `id` in skin `skin` (by name), else in the default skin.
    pub fn attachment(&self, skin: Option<&str>, id: &AttachmentId) -> Option<(usize, &Attachment)> {
        let named = skin.and_then(|n| self.rig.skins.iter().position(|s| s.name == n));
        for si in named.into_iter().chain(self.default_skin) {
            if let Some(a) = self.rig.skins[si].attachments.get(id) {
                return Some((si, a));
            }
        }
        None
    }

    /// Resolves, skins and orders everything visible in `pose`.
    pub fn draw_list<'a>(
        &'a self,
        pose: &RigPose,
        world: &WorldPose,
        skin: Option<&str>,
        skinner: &dyn Skinner,
    ) -> RigDrawList<'a> {
        let mut list = RigDrawList::default();
        let mut scratch = Vec::new();
        for &si in &pose.draw_order {
            let (Some(sp), Some(&bone)) = (pose.slots.get(si), self.slot_bones.get(si)) else { continue };
            let Some(att_id) = &sp.attachment else { continue };
            let Some((skin_idx, att)) = self.attachment(skin, att_id) else { continue };
            let color = sp.color;
            if color.a <= 0.0 {
                continue;
            }
            match att {
                Attachment::Region(RegionAttachment { image, size, xform }) => {
                    let Some(bw) = world.bones.get(bone) else { continue };
                    list.items.push(RigDrawItem::Region {
                        slot: si,
                        image,
                        size: *size,
                        transform: bw.mul(&xform.matrix()),
                        color,
                    });
                }
                Attachment::Mesh(m) => {
                    let Some(prepared) = self.meshes[skin_idx].get(att_id) else { continue };
                    let job = SkinJob {
                        vertices: &m.vertices,
                        deform: pose.deforms.get(&(si, att_id.clone())).map(|d| d.as_slice()),
                        mesh: prepared,
                        skinning: &world.skinning,
                        slot_bone: bone,
                    };
                    skinner.skin(&job, &mut scratch);
                    let start = list.positions.len();
                    list.positions.extend_from_slice(&scratch);
                    list.items.push(RigDrawItem::Mesh {
                        slot: si,
                        image: &m.image,
                        range: start..list.positions.len(),
                        uvs: &m.uvs,
                        triangles: &m.triangles,
                        color,
                    });
                }
            }
        }
        list
    }
}

impl RigPose {
    /// Cross-fade: `u = 0` → `a`, `u = 1` → `b`. Continuous values are
    /// interpolated (rotations on the shortest arc); discrete ones
    /// (attachments, draw order) switch to `b` as soon as `u > 0`.
    pub fn blend(a: &RigPose, b: &RigPose, u: f32) -> RigPose {
        if u <= 0.0 {
            return a.clone();
        }
        if u >= 1.0 {
            return b.clone();
        }
        let bones = a.bones.iter().zip(&b.bones).map(|(x, y)| Xform::lerp(x, y, u)).collect();
        let slots = a
            .slots
            .iter()
            .zip(&b.slots)
            .map(|(x, y)| SlotPose { attachment: y.attachment.clone(), color: Rgba::lerp(&x.color, &y.color, u) })
            .collect();
        let mut deforms = BTreeMap::new();
        let empty = super::model::Deform::default();
        for key in a.deforms.keys().chain(b.deforms.keys()) {
            if deforms.contains_key(key) {
                continue;
            }
            let da = a.deforms.get(key).map(|v| super::model::Deform(v.clone())).unwrap_or_else(|| empty.clone());
            let db = b.deforms.get(key).map(|v| super::model::Deform(v.clone())).unwrap_or_else(|| empty.clone());
            deforms.insert(key.clone(), super::model::Deform::lerp(&da, &db, u).0);
        }
        RigPose { bones, slots, deforms, draw_order: b.draw_order.clone() }
    }
}
