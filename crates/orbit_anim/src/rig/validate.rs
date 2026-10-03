//! Rig validation and web budget checks.

use std::collections::{BTreeMap, BTreeSet};

use super::model::{Attachment, Rig, RigTrack};

/// Web budget (ADR 0013 §5): warn above these.
pub const WEB_MAX_BONES: usize = 64;
pub const WEB_MAX_VERTICES: usize = 4000;
/// Influences per vertex.
pub const MAX_INFLUENCES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The rig still evaluates (fixed on the fly or harmless).
    Warning,
    /// The rig cannot be evaluated.
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RigIssueKind {
    UnsupportedVersion { found: u32, supported: u32 },
    DuplicateBone(String),
    DuplicateSlot(String),
    UnknownParent { bone: String, parent: String },
    /// A parent listed after its child (or a cycle).
    ParentAfterChild { bone: String, parent: String },
    SlotUnknownBone { slot: String, bone: String },
    UnknownAttachment { slot: String, attachment: String },
    /// Vertices, uvs and weights do not have the same length.
    MeshLengths { attachment: String },
    TriangleOutOfRange { attachment: String, triangle: usize },
    /// A vertex has more than 4 influences (the 4 heaviest are kept).
    TooManyInfluences { attachment: String, vertex: usize },
    UnknownWeightBone { attachment: String, bone: String },
    /// Weights do not sum to 1 (normalized when prepared).
    WeightsNotNormalized { attachment: String, vertex: usize },
    /// A vertex without any positive weight.
    VertexWithoutWeight { attachment: String, vertex: usize },
    ClipBadDuration(String),
    ClipBadKeys { clip: String, track: usize },
    ClipUnknownTarget { clip: String, track: usize, target: String },
    IkBoneCount(usize),
    IkUnknownBone(String),
    /// Two-bone IK where the second bone's parent is not the first.
    IkNotAChain(usize),
    WebTooManyBones(usize),
    WebTooManyVertices(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RigIssue {
    pub severity: Severity,
    pub kind: RigIssueKind,
}

impl core::fmt::Display for RigIssue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use RigIssueKind::*;
        match &self.kind {
            UnsupportedVersion { found, supported } => {
                write!(f, "rig_version {found} is newer than supported {supported}")
            }
            DuplicateBone(b) => write!(f, "duplicate bone id {b}"),
            DuplicateSlot(s) => write!(f, "duplicate slot id {s}"),
            UnknownParent { bone, parent } => write!(f, "bone {bone}: unknown parent {parent}"),
            ParentAfterChild { bone, parent } => write!(f, "bone {bone}: parent {parent} must come before it"),
            SlotUnknownBone { slot, bone } => write!(f, "slot {slot}: unknown bone {bone}"),
            UnknownAttachment { slot, attachment } => write!(f, "slot {slot}: attachment {attachment} is in no skin"),
            MeshLengths { attachment } => write!(f, "mesh {attachment}: vertices, uvs and weights differ in length"),
            TriangleOutOfRange { attachment, triangle } => {
                write!(f, "mesh {attachment}: triangle {triangle} uses a missing vertex")
            }
            TooManyInfluences { attachment, vertex } => {
                write!(f, "mesh {attachment}: vertex {vertex} has more than {MAX_INFLUENCES} bones")
            }
            UnknownWeightBone { attachment, bone } => write!(f, "mesh {attachment}: unknown weight bone {bone}"),
            WeightsNotNormalized { attachment, vertex } => {
                write!(f, "mesh {attachment}: weights of vertex {vertex} do not sum to 1")
            }
            VertexWithoutWeight { attachment, vertex } => write!(f, "mesh {attachment}: vertex {vertex} has no weight"),
            ClipBadDuration(c) => write!(f, "clip {c}: invalid duration"),
            ClipBadKeys { clip, track } => write!(f, "clip {clip}: track {track} has unsorted or invalid keys"),
            ClipUnknownTarget { clip, track, target } => write!(f, "clip {clip}: track {track} targets unknown {target}"),
            IkBoneCount(i) => write!(f, "ik {i}: needs 1 or 2 bones"),
            IkUnknownBone(b) => write!(f, "ik: unknown bone {b}"),
            IkNotAChain(i) => write!(f, "ik {i}: the second bone must be a child of the first"),
            WebTooManyBones(n) => write!(f, "{n} bones exceed the web budget of {WEB_MAX_BONES}"),
            WebTooManyVertices(n) => write!(f, "{n} vertices exceed the web budget of {WEB_MAX_VERTICES}"),
        }
    }
}

impl Rig {
    /// All problems, errors and warnings. A rig with no `Error` can be
    /// prepared ([`super::PreparedRig::new`]).
    pub fn validate(&self) -> Vec<RigIssue> {
        let mut out = Vec::new();
        let mut err = |kind| out.push(RigIssue { severity: Severity::Error, kind });
        if self.rig_version > super::RIG_VERSION {
            err(RigIssueKind::UnsupportedVersion { found: self.rig_version, supported: super::RIG_VERSION });
        }
        let all_bones: BTreeSet<&str> = self.bones.iter().map(|b| b.id.as_str()).collect();
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, b) in self.bones.iter().enumerate() {
            if seen.insert(b.id.as_str(), i).is_some() {
                err(RigIssueKind::DuplicateBone(b.id.0.clone()));
            }
            if let Some(p) = &b.parent {
                if !all_bones.contains(p.as_str()) {
                    err(RigIssueKind::UnknownParent { bone: b.id.0.clone(), parent: p.0.clone() });
                } else if !seen.contains_key(p.as_str()) || p == &b.id {
                    err(RigIssueKind::ParentAfterChild { bone: b.id.0.clone(), parent: p.0.clone() });
                }
            }
        }
        let mut slot_ids = BTreeSet::new();
        for s in &self.slots {
            if !slot_ids.insert(s.id.as_str()) {
                err(RigIssueKind::DuplicateSlot(s.id.0.clone()));
            }
            if !all_bones.contains(s.bone.as_str()) {
                err(RigIssueKind::SlotUnknownBone { slot: s.id.0.clone(), bone: s.bone.0.clone() });
            }
            if let Some(a) = &s.attachment
                && !self.skins.iter().any(|k| k.attachments.contains_key(a))
            {
                err(RigIssueKind::UnknownAttachment { slot: s.id.0.clone(), attachment: a.0.clone() });
            }
        }
        let mut warnings = Vec::new();
        let mut warn = |kind| warnings.push(RigIssue { severity: Severity::Warning, kind });
        let mut max_vertices = 0usize;
        for skin in &self.skins {
            let mut skin_vertices = 0;
            for (id, a) in &skin.attachments {
                let Attachment::Mesh(m) = a else { continue };
                let name = id.0.clone();
                skin_vertices += m.vertices.len();
                if m.uvs.len() != m.vertices.len() || (!m.weights.is_empty() && m.weights.len() != m.vertices.len()) {
                    err(RigIssueKind::MeshLengths { attachment: name.clone() });
                }
                if let Some(t) = m.triangles.iter().position(|t| t.iter().any(|&v| v as usize >= m.vertices.len())) {
                    err(RigIssueKind::TriangleOutOfRange { attachment: name.clone(), triangle: t });
                }
                for (v, w) in m.weights.iter().enumerate() {
                    if let Some((b, _)) = w.iter().find(|(b, _)| !all_bones.contains(b.as_str())) {
                        err(RigIssueKind::UnknownWeightBone { attachment: name.clone(), bone: b.0.clone() });
                    }
                    let positive: Vec<f32> = w.iter().map(|x| x.1).filter(|x| x.is_finite() && *x > 0.0).collect();
                    if positive.is_empty() {
                        err(RigIssueKind::VertexWithoutWeight { attachment: name.clone(), vertex: v });
                        continue;
                    }
                    if w.len() > MAX_INFLUENCES {
                        warn(RigIssueKind::TooManyInfluences { attachment: name.clone(), vertex: v });
                    } else if (positive.iter().sum::<f32>() - 1.0).abs() > 1e-3 {
                        warn(RigIssueKind::WeightsNotNormalized { attachment: name.clone(), vertex: v });
                    }
                }
            }
            max_vertices = max_vertices.max(skin_vertices);
        }
        let slot_known = |id: &str| slot_ids.contains(id);
        for (name, clip) in &self.clips {
            if !clip.duration.is_finite() || clip.duration < 0.0 {
                err(RigIssueKind::ClipBadDuration(name.clone()));
            }
            for (i, t) in clip.tracks.iter().enumerate() {
                if t.validate_keys().is_err() {
                    err(RigIssueKind::ClipBadKeys { clip: name.clone(), track: i });
                }
                let missing = match t {
                    RigTrack::BoneRotate { bone, .. }
                    | RigTrack::BoneTranslate { bone, .. }
                    | RigTrack::BoneScale { bone, .. } => {
                        (!all_bones.contains(bone.as_str())).then(|| format!("bone {bone}"))
                    }
                    RigTrack::SlotAttachment { slot, .. }
                    | RigTrack::SlotColor { slot, .. }
                    | RigTrack::Deform { slot, .. } => (!slot_known(slot.as_str())).then(|| format!("slot {slot}")),
                    RigTrack::DrawOrder { .. } => None,
                };
                // A dangling target is ignored at runtime: warn only.
                if let Some(target) = missing {
                    warn(RigIssueKind::ClipUnknownTarget { clip: name.clone(), track: i, target });
                }
            }
        }
        for (i, c) in self.ik.iter().enumerate() {
            if c.bones.is_empty() || c.bones.len() > 2 {
                err(RigIssueKind::IkBoneCount(i));
                continue;
            }
            let mut ok = true;
            for b in c.bones.iter().chain(Some(&c.target)) {
                if !all_bones.contains(b.as_str()) {
                    err(RigIssueKind::IkUnknownBone(b.0.clone()));
                    ok = false;
                }
            }
            if ok && c.bones.len() == 2 {
                let child = self.bones.iter().find(|b| b.id == c.bones[1]);
                if child.and_then(|b| b.parent.as_ref()) != Some(&c.bones[0]) {
                    err(RigIssueKind::IkNotAChain(i));
                }
            }
        }
        if self.bones.len() > WEB_MAX_BONES {
            warn(RigIssueKind::WebTooManyBones(self.bones.len()));
        }
        if max_vertices > WEB_MAX_VERTICES {
            warn(RigIssueKind::WebTooManyVertices(max_vertices));
        }
        out.extend(warnings);
        out
    }

    /// Rewrites every mesh's weights to at most 4 positive influences summing
    /// to 1 (the "normalized at save" step). Returns the number of vertices
    /// changed.
    pub fn normalize_weights(&mut self) -> usize {
        let mut changed = 0;
        for skin in &mut self.skins {
            for a in skin.attachments.values_mut() {
                let Attachment::Mesh(m) = a else { continue };
                for w in &mut m.weights {
                    let mut v: Vec<_> = w.iter().filter(|(_, x)| x.is_finite() && *x > 0.0).cloned().collect();
                    v.sort_by(|a, b| b.1.total_cmp(&a.1));
                    v.truncate(MAX_INFLUENCES);
                    let sum: f32 = v.iter().map(|x| x.1).sum();
                    if sum > 0.0 {
                        for x in &mut v {
                            x.1 /= sum;
                        }
                    }
                    if v != *w {
                        *w = v;
                        changed += 1;
                    }
                }
            }
        }
        changed
    }
}
