//! The rig document of the rig editor: bone, slot, skin and IK operations,
//! validation diagnostics, undo / redo (snapshot stack, like the 3D scene and
//! interface editors) and save / load of a `.rig.ron`. No egui, no GPU: unit
//! tested headlessly.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_anim::rig::{
    Affine, Attachment, AttachmentId, Bone, BoneId, IkConstraint, RegionAttachment, Rig, RigIssue, RigIssueKind,
    RigTrack, Severity, Skin, Slot, SlotId, WEB_MAX_BONES, Xform, load_rig, save_rig,
};

pub const MAX_UNDO: usize = 200;
/// Name of the fallback skin.
pub const DEFAULT_SKIN: &str = "default";

/// Why an operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpError {
    Unknown,
    /// Reparenting under itself or a descendant.
    Cycle,
    EmptyName,
    /// IK needs 1 or 2 bones (the second a child of the first) and a target
    /// that is not part of the chain.
    BadIk,
    /// The `default` skin, or the last skin, cannot be deleted.
    KeepSkin,
}

impl OpError {
    pub fn key(self) -> &'static str {
        match self {
            OpError::Unknown => "rig.err.unknown",
            OpError::Cycle => "rig.err.cycle",
            OpError::EmptyName => "rig.err.empty_name",
            OpError::BadIk => "rig.err.bad_ik",
            OpError::KeepSkin => "rig.err.keep_skin",
        }
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// A short unique id (`prefix` + 10 hex digits): time, a counter and a mix.
pub fn fresh_id(prefix: &str, taken: impl Fn(&str) -> bool) -> String {
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let mut x = t ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 32;
        let id = format!("{prefix}{:010x}", x & 0xFF_FFFF_FFFF);
        if !taken(&id) {
            return id;
        }
    }
}

/// `base`, or `base2`, `base3`... when taken.
pub fn unique_name<'a>(base: &str, existing: impl Iterator<Item = &'a str> + Clone) -> String {
    let taken = |n: &str| existing.clone().any(|e| e == n);
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_owned())
}

// ------------------------------------------------------------------- math

/// World matrices of `locals` (one per bone, rig order), no IK.
pub fn world_matrices(rig: &Rig, locals: &[Xform]) -> Vec<Affine> {
    let index: HashMap<&str, usize> = rig.bones.iter().enumerate().map(|(i, b)| (b.id.as_str(), i)).collect();
    let mut out: Vec<Affine> = Vec::with_capacity(rig.bones.len());
    for (i, b) in rig.bones.iter().enumerate() {
        let m = locals.get(i).copied().unwrap_or(b.rest).matrix();
        let w = match b.parent.as_ref().and_then(|p| index.get(p.as_str())) {
            Some(&p) if p < out.len() => out[p].mul(&m),
            _ => m,
        };
        out.push(w);
    }
    out
}

pub fn rest_locals(rig: &Rig) -> Vec<Xform> {
    rig.bones.iter().map(|b| b.rest).collect()
}

/// Splits a (shear free) matrix into a local transform.
pub fn decompose(m: &Affine) -> Xform {
    let sx = m.a.hypot(m.b);
    if sx < 1e-9 {
        return Xform { x: m.tx, y: m.ty, rotation: 0.0, sx: 0.0, sy: 1.0 };
    }
    Xform { x: m.tx, y: m.ty, rotation: m.b.atan2(m.a).to_degrees(), sx, sy: m.det() / sx }
}

pub fn bone_index(rig: &Rig, id: &BoneId) -> Option<usize> {
    rig.bones.iter().position(|b| &b.id == id)
}

pub fn bone<'a>(rig: &'a Rig, id: &BoneId) -> Option<&'a Bone> {
    rig.bones.iter().find(|b| &b.id == id)
}

pub fn slot<'a>(rig: &'a Rig, id: &SlotId) -> Option<&'a Slot> {
    rig.slots.iter().find(|s| &s.id == id)
}

pub fn children(rig: &Rig, parent: Option<&BoneId>) -> Vec<BoneId> {
    rig.bones.iter().filter(|b| b.parent.as_ref() == parent).map(|b| b.id.clone()).collect()
}

/// `id` and everything under it.
pub fn subtree(rig: &Rig, id: &BoneId) -> HashSet<BoneId> {
    let mut set = HashSet::new();
    if bone(rig, id).is_none() {
        return set;
    }
    set.insert(id.clone());
    // Topological order: one pass is enough.
    for b in &rig.bones {
        if let Some(p) = &b.parent
            && set.contains(p)
        {
            set.insert(b.id.clone());
        }
    }
    set
}

/// Puts parents before children again (stable).
fn fix_order(rig: &mut Rig) {
    let mut placed: HashSet<BoneId> = HashSet::new();
    let mut rest: Vec<Bone> = std::mem::take(&mut rig.bones);
    let mut out = Vec::with_capacity(rest.len());
    while !rest.is_empty() {
        let before = rest.len();
        let mut keep = Vec::new();
        for b in rest {
            let ready = b.parent.as_ref().is_none_or(|p| placed.contains(p));
            if ready {
                placed.insert(b.id.clone());
                out.push(b);
            } else {
                keep.push(b);
            }
        }
        rest = keep;
        if rest.len() == before {
            // Unknown parent or a cycle: keep them, as roots.
            for mut b in rest.drain(..) {
                b.parent = None;
                out.push(b);
            }
        }
    }
    rig.bones = out;
}

// ------------------------------------------------------------ diagnostics

/// A problem shown to the user: an error or a warning, localized by the UI.
#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    pub severity: Severity,
    pub key: &'static str,
    pub args: Vec<(&'static str, String)>,
}

fn d(severity: Severity, key: &'static str, args: Vec<(&'static str, String)>) -> Diag {
    Diag { severity, key, args }
}

fn from_issue(i: &RigIssue) -> Diag {
    use RigIssueKind::*;
    let s = i.severity;
    match &i.kind {
        UnsupportedVersion { found, supported } => {
            d(s, "rig.diag.version", vec![("found", found.to_string()), ("supported", supported.to_string())])
        }
        DuplicateBone(b) => d(s, "rig.diag.dup_bone", vec![("bone", b.clone())]),
        DuplicateSlot(x) => d(s, "rig.diag.dup_slot", vec![("slot", x.clone())]),
        UnknownParent { bone, parent } => {
            d(s, "rig.diag.unknown_parent", vec![("bone", bone.clone()), ("parent", parent.clone())])
        }
        ParentAfterChild { bone, parent } => {
            d(s, "rig.diag.parent_order", vec![("bone", bone.clone()), ("parent", parent.clone())])
        }
        SlotUnknownBone { slot, bone } => {
            d(s, "rig.diag.slot_bone", vec![("slot", slot.clone()), ("bone", bone.clone())])
        }
        UnknownAttachment { slot, attachment } => {
            d(s, "rig.diag.unknown_attachment", vec![("slot", slot.clone()), ("attachment", attachment.clone())])
        }
        MeshLengths { attachment } => d(s, "rig.diag.mesh_lengths", vec![("attachment", attachment.clone())]),
        TriangleOutOfRange { attachment, triangle } => d(
            s,
            "rig.diag.mesh_triangle",
            vec![("attachment", attachment.clone()), ("triangle", triangle.to_string())],
        ),
        TooManyInfluences { attachment, vertex } => d(
            s,
            "rig.diag.mesh_influences",
            vec![("attachment", attachment.clone()), ("vertex", vertex.to_string())],
        ),
        UnknownWeightBone { attachment, bone } => {
            d(s, "rig.diag.mesh_weight_bone", vec![("attachment", attachment.clone()), ("bone", bone.clone())])
        }
        WeightsNotNormalized { attachment, vertex } => d(
            s,
            "rig.diag.mesh_normalize",
            vec![("attachment", attachment.clone()), ("vertex", vertex.to_string())],
        ),
        VertexWithoutWeight { attachment, vertex } => d(
            s,
            "rig.diag.mesh_no_weight",
            vec![("attachment", attachment.clone()), ("vertex", vertex.to_string())],
        ),
        ClipBadDuration(c) => d(s, "rig.diag.clip_duration", vec![("clip", c.clone())]),
        ClipBadKeys { clip, track } => {
            d(s, "rig.diag.clip_keys", vec![("clip", clip.clone()), ("track", track.to_string())])
        }
        ClipUnknownTarget { clip, track, target } => d(
            s,
            "rig.diag.clip_target",
            vec![("clip", clip.clone()), ("track", track.to_string()), ("target", target.clone())],
        ),
        IkBoneCount(n) => d(s, "rig.diag.ik_count", vec![("ik", n.to_string())]),
        IkUnknownBone(b) => d(s, "rig.diag.ik_bone", vec![("bone", b.clone())]),
        IkNotAChain(n) => d(s, "rig.diag.ik_chain", vec![("ik", n.to_string())]),
        WebTooManyBones(n) => {
            d(s, "rig.diag.web_bones", vec![("count", n.to_string()), ("max", WEB_MAX_BONES.to_string())])
        }
        WebTooManyVertices(n) => d(
            s,
            "rig.diag.web_vertices",
            vec![("count", n.to_string()), ("max", orbit_anim::rig::WEB_MAX_VERTICES.to_string())],
        ),
    }
}

/// Everything wrong or suspicious about `rig`: the engine's validation (with
/// the web budgets) plus editor level checks (duplicate names, empty slots).
pub fn diagnostics(rig: &Rig) -> Vec<Diag> {
    let mut out: Vec<Diag> = rig.validate().iter().map(from_issue).collect();
    let mut seen = HashSet::new();
    for b in &rig.bones {
        if !seen.insert(b.name.as_str()) {
            out.push(d(Severity::Warning, "rig.diag.dup_bone_name", vec![("bone", b.name.clone())]));
        }
    }
    let mut seen = HashSet::new();
    for s in &rig.slots {
        if !seen.insert(s.name.as_str()) {
            out.push(d(Severity::Warning, "rig.diag.dup_slot_name", vec![("slot", s.name.clone())]));
        }
        if s.attachment.is_none() {
            out.push(d(Severity::Warning, "rig.diag.slot_empty", vec![("slot", s.name.clone())]));
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.severity));
    out
}

// -------------------------------------------------------------- document

pub struct RigDoc {
    rig: Rig,
    saved: Rig,
    pub path: Option<PathBuf>,
    /// Bumped on every change (edit, undo, redo): consumers redraw.
    pub revision: u64,
    undo: Vec<Rig>,
    redo: Vec<Rig>,
    in_gesture: bool,
}

impl RigDoc {
    pub fn new() -> Self {
        let rig = Rig::default();
        Self { saved: rig.clone(), rig, path: None, revision: 1, undo: Vec::new(), redo: Vec::new(), in_gesture: false }
    }

    pub fn from_rig(rig: Rig, path: Option<PathBuf>) -> Self {
        Self { saved: rig.clone(), rig, path, revision: 1, undo: Vec::new(), redo: Vec::new(), in_gesture: false }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let loaded = load_rig(&text).map_err(|e| e.to_string())?;
        Ok(Self::from_rig(loaded.rig, Some(path.to_owned())))
    }

    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        let text = save_rig(&self.rig)?;
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        self.path = Some(path.to_owned());
        self.saved = self.rig.clone();
        Ok(())
    }

    pub fn save(&mut self) -> Result<(), String> {
        let path = self.path.clone().ok_or_else(|| "no path".to_owned())?;
        self.save_as(&path)
    }

    pub fn rig(&self) -> &Rig {
        &self.rig
    }

    pub fn dirty(&self) -> bool {
        self.rig != self.saved
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        self.in_gesture = false;
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.rig, prev));
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        self.in_gesture = false;
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.rig, next));
        self.revision += 1;
        true
    }

    /// Starts a continuous edit (a drag): everything until
    /// [`RigDoc::end_gesture`] is one undo step.
    pub fn begin_gesture(&mut self) {
        if self.in_gesture {
            return;
        }
        self.push_undo(self.rig.clone());
        self.in_gesture = true;
    }

    pub fn end_gesture(&mut self) {
        if !self.in_gesture {
            return;
        }
        self.in_gesture = false;
        if self.undo.last() == Some(&self.rig) {
            self.undo.pop();
        }
    }

    fn push_undo(&mut self, snapshot: Rig) {
        self.undo.push(snapshot);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Runs `f` on the rig as one undo step; a refused or ineffective
    /// operation leaves rig and history untouched.
    fn mutate<R>(&mut self, f: impl FnOnce(&mut Rig) -> Result<R, OpError>) -> Result<R, OpError> {
        let before = self.rig.clone();
        match f(&mut self.rig) {
            Err(e) => {
                self.rig = before;
                Err(e)
            }
            Ok(r) => {
                if self.rig != before {
                    if !self.in_gesture {
                        self.push_undo(before);
                    }
                    self.revision += 1;
                }
                Ok(r)
            }
        }
    }

    // ---- bones

    /// Adds a bone from a head to a tip given in rig space (what a
    /// click-drag draws), under `parent`. Returns its id.
    pub fn add_bone(&mut self, parent: Option<&BoneId>, head: [f32; 2], tip: [f32; 2]) -> Result<BoneId, OpError> {
        self.mutate(|rig| {
            let parent_world = match parent {
                Some(p) => {
                    let i = bone_index(rig, p).ok_or(OpError::Unknown)?;
                    world_matrices(rig, &rest_locals(rig))[i]
                }
                None => Affine::IDENTITY,
            };
            let inv = parent_world.inverse().unwrap_or(Affine::IDENTITY);
            let (h, t) = (inv.apply(head), inv.apply(tip));
            let (dx, dy) = (t[0] - h[0], t[1] - h[1]);
            let length = dx.hypot(dy);
            let rotation = if length > 1e-6 { dy.atan2(dx).to_degrees() } else { 0.0 };
            let id = BoneId::new(fresh_id("b", |s| rig.bones.iter().any(|b| b.id.as_str() == s)));
            let name = unique_name("bone", rig.bones.iter().map(|b| b.name.as_str()));
            rig.bones.push(Bone {
                id: id.clone(),
                name,
                parent: parent.cloned(),
                rest: Xform { x: h[0], y: h[1], rotation, sx: 1.0, sy: 1.0 },
                length: if length > 1e-6 { length } else { 50.0 },
            });
            Ok(id)
        })
    }

    pub fn rename_bone(&mut self, id: &BoneId, name: &str) -> Result<(), OpError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(OpError::EmptyName);
        }
        self.mutate(|rig| {
            rig.bones.iter_mut().find(|b| &b.id == id).ok_or(OpError::Unknown)?.name = name.to_owned();
            Ok(())
        })
    }

    pub fn set_rest(&mut self, id: &BoneId, rest: Xform) -> Result<(), OpError> {
        self.mutate(|rig| {
            rig.bones.iter_mut().find(|b| &b.id == id).ok_or(OpError::Unknown)?.rest = rest;
            Ok(())
        })
    }

    pub fn set_bone_length(&mut self, id: &BoneId, length: f32) -> Result<(), OpError> {
        self.mutate(|rig| {
            rig.bones.iter_mut().find(|b| &b.id == id).ok_or(OpError::Unknown)?.length = length.max(0.0);
            Ok(())
        })
    }

    /// Moves `id` under `parent` (`None`: root) keeping its place in the
    /// world; a descendant (or itself) is refused.
    pub fn reparent(&mut self, id: &BoneId, parent: Option<&BoneId>) -> Result<(), OpError> {
        self.mutate(|rig| {
            let i = bone_index(rig, id).ok_or(OpError::Unknown)?;
            if let Some(p) = parent {
                if bone(rig, p).is_none() {
                    return Err(OpError::Unknown);
                }
                if subtree(rig, id).contains(p) {
                    return Err(OpError::Cycle);
                }
            }
            if rig.bones[i].parent.as_ref() == parent {
                return Ok(());
            }
            let world = world_matrices(rig, &rest_locals(rig));
            let parent_world = parent.and_then(|p| bone_index(rig, p)).map_or(Affine::IDENTITY, |pi| world[pi]);
            let local = parent_world.inverse().unwrap_or(Affine::IDENTITY).mul(&world[i]);
            let b = &mut rig.bones[i];
            b.rest = decompose(&local);
            b.parent = parent.cloned();
            fix_order(rig);
            Ok(())
        })
    }

    /// Deletes a bone and its descendants, the slots that rode on them and
    /// whatever referred to them (IK, clip tracks, mesh weights).
    pub fn delete_bone(&mut self, id: &BoneId) -> Result<(), OpError> {
        self.mutate(|rig| {
            let gone = subtree(rig, id);
            if gone.is_empty() {
                return Err(OpError::Unknown);
            }
            rig.bones.retain(|b| !gone.contains(&b.id));
            let dead_slots: HashSet<SlotId> =
                rig.slots.iter().filter(|s| gone.contains(&s.bone)).map(|s| s.id.clone()).collect();
            remove_slots(rig, &dead_slots);
            rig.ik.retain(|c| !c.bones.iter().any(|b| gone.contains(b)) && !gone.contains(&c.target));
            purge_tracks(rig, &gone, &HashSet::new());
            for skin in &mut rig.skins {
                for att in skin.attachments.values_mut() {
                    if let Attachment::Mesh(m) = att {
                        for w in &mut m.weights {
                            w.retain(|(b, _)| !gone.contains(b));
                        }
                    }
                }
            }
            rig.normalize_weights();
            Ok(())
        })
    }

    // ---- slots and attachments

    pub fn add_slot(&mut self, bone: &BoneId) -> Result<SlotId, OpError> {
        self.mutate(|rig| {
            bone_index(rig, bone).ok_or(OpError::Unknown)?;
            let id = SlotId::new(fresh_id("s", |s| rig.slots.iter().any(|x| x.id.as_str() == s)));
            let name = unique_name("slot", rig.slots.iter().map(|s| s.name.as_str()));
            rig.slots.push(Slot {
                id: id.clone(),
                name,
                bone: bone.clone(),
                attachment: None,
                color: orbit_anim::Rgba::new(1.0, 1.0, 1.0, 1.0),
            });
            Ok(id)
        })
    }

    pub fn rename_slot(&mut self, id: &SlotId, name: &str) -> Result<(), OpError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(OpError::EmptyName);
        }
        self.mutate(|rig| {
            rig.slots.iter_mut().find(|s| &s.id == id).ok_or(OpError::Unknown)?.name = name.to_owned();
            Ok(())
        })
    }

    pub fn set_slot_bone(&mut self, id: &SlotId, bone: &BoneId) -> Result<(), OpError> {
        self.mutate(|rig| {
            bone_index(rig, bone).ok_or(OpError::Unknown)?;
            rig.slots.iter_mut().find(|s| &s.id == id).ok_or(OpError::Unknown)?.bone = bone.clone();
            Ok(())
        })
    }

    pub fn delete_slot(&mut self, id: &SlotId) -> Result<(), OpError> {
        self.mutate(|rig| {
            slot(rig, id).ok_or(OpError::Unknown)?;
            remove_slots(rig, &HashSet::from([id.clone()]));
            Ok(())
        })
    }

    /// Moves a slot in the draw order by `delta` places (positive = in front).
    pub fn move_slot(&mut self, id: &SlotId, delta: i32) -> Result<(), OpError> {
        self.mutate(|rig| {
            let i = rig.slots.iter().position(|s| &s.id == id).ok_or(OpError::Unknown)?;
            let j = (i as i64 + i64::from(delta)).clamp(0, rig.slots.len() as i64 - 1) as usize;
            let s = rig.slots.remove(i);
            rig.slots.insert(j, s);
            Ok(())
        })
    }

    /// Shows the picture `image` (a project asset path, `size` in rig units)
    /// in `slot` for `skin` (created when missing), as a rigid region
    /// centered on the slot's bone.
    pub fn attach_image(
        &mut self,
        slot_id: &SlotId,
        skin: &str,
        image: &str,
        size: [f32; 2],
    ) -> Result<AttachmentId, OpError> {
        self.mutate(|rig| {
            let si = rig.slots.iter().position(|s| &s.id == slot_id).ok_or(OpError::Unknown)?;
            let att_id = match rig.slots[si].attachment.clone() {
                Some(a) => a,
                None => {
                    let a = AttachmentId::new(fresh_id("a", |s| {
                        rig.skins.iter().any(|k| k.attachments.keys().any(|x| x.as_str() == s))
                    }));
                    rig.slots[si].attachment = Some(a.clone());
                    a
                }
            };
            let skin = skin_mut(rig, skin);
            let xform = match skin.attachments.get(&att_id) {
                Some(Attachment::Region(r)) => r.xform,
                _ => Xform::IDENTITY,
            };
            skin.attachments
                .insert(att_id.clone(), Attachment::Region(RegionAttachment { image: image.to_owned(), size, xform }));
            Ok(att_id)
        })
    }

    /// Removes the slot's picture from `skin` (and from the slot when no skin
    /// shows it any more).
    pub fn detach(&mut self, slot_id: &SlotId, skin: &str) -> Result<(), OpError> {
        self.mutate(|rig| {
            let att = slot(rig, slot_id).ok_or(OpError::Unknown)?.attachment.clone();
            let Some(att) = att else { return Ok(()) };
            if let Some(k) = rig.skins.iter_mut().find(|k| k.name == skin) {
                k.attachments.remove(&att);
            }
            if !rig.skins.iter().any(|k| k.attachments.contains_key(&att)) {
                for s in &mut rig.slots {
                    if s.attachment.as_ref() == Some(&att) {
                        s.attachment = None;
                    }
                }
            }
            Ok(())
        })
    }

    /// Edits the region of the slot's attachment in `skin`.
    pub fn set_region(&mut self, slot_id: &SlotId, skin: &str, size: [f32; 2], xform: Xform) -> Result<(), OpError> {
        self.mutate(|rig| {
            let att = slot(rig, slot_id).ok_or(OpError::Unknown)?.attachment.clone().ok_or(OpError::Unknown)?;
            let k = rig.skins.iter_mut().find(|k| k.name == skin).ok_or(OpError::Unknown)?;
            match k.attachments.get_mut(&att) {
                Some(Attachment::Region(r)) => {
                    r.size = size;
                    r.xform = xform;
                    Ok(())
                }
                _ => Err(OpError::Unknown),
            }
        })
    }

    // ---- skins

    pub fn add_skin(&mut self, name: &str) -> Result<String, OpError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(OpError::EmptyName);
        }
        self.mutate(|rig| {
            let name = unique_name(name, rig.skins.iter().map(|s| s.name.as_str()));
            rig.skins.push(Skin { name: name.clone(), attachments: Default::default() });
            Ok(name)
        })
    }

    pub fn duplicate_skin(&mut self, from: &str) -> Result<String, OpError> {
        self.mutate(|rig| {
            let src = rig.skins.iter().find(|s| s.name == from).ok_or(OpError::Unknown)?.clone();
            let name = unique_name(&format!("{from} copy"), rig.skins.iter().map(|s| s.name.as_str()));
            rig.skins.push(Skin { name: name.clone(), attachments: src.attachments });
            Ok(name)
        })
    }

    pub fn rename_skin(&mut self, from: &str, to: &str) -> Result<(), OpError> {
        let to = to.trim();
        if to.is_empty() {
            return Err(OpError::EmptyName);
        }
        self.mutate(|rig| {
            if from != to && rig.skins.iter().any(|s| s.name == to) {
                return Err(OpError::Unknown);
            }
            rig.skins.iter_mut().find(|s| s.name == from).ok_or(OpError::Unknown)?.name = to.to_owned();
            Ok(())
        })
    }

    pub fn delete_skin(&mut self, name: &str) -> Result<(), OpError> {
        self.mutate(|rig| {
            if name == DEFAULT_SKIN || rig.skins.len() <= 1 {
                return Err(OpError::KeepSkin);
            }
            let n = rig.skins.len();
            rig.skins.retain(|s| s.name != name);
            if rig.skins.len() == n {
                return Err(OpError::Unknown);
            }
            Ok(())
        })
    }

    // ---- IK

    /// 1 bone (aims at the target) or 2 (`[parent, child]`).
    pub fn add_ik(&mut self, bones: Vec<BoneId>, target: &BoneId) -> Result<(), OpError> {
        self.mutate(|rig| {
            if !(1..=2).contains(&bones.len()) || bone_index(rig, target).is_none() || bones.contains(target) {
                return Err(OpError::BadIk);
            }
            for b in &bones {
                bone_index(rig, b).ok_or(OpError::BadIk)?;
            }
            if bones.len() == 2 && bone(rig, &bones[1]).and_then(|b| b.parent.as_ref()) != Some(&bones[0]) {
                return Err(OpError::BadIk);
            }
            rig.ik.push(IkConstraint { bones, target: target.clone(), bend_positive: false, mix: 1.0 });
            Ok(())
        })
    }

    pub fn update_ik(&mut self, index: usize, mix: f32, bend_positive: bool) -> Result<(), OpError> {
        self.mutate(|rig| {
            let c = rig.ik.get_mut(index).ok_or(OpError::Unknown)?;
            c.mix = mix.clamp(0.0, 1.0);
            c.bend_positive = bend_positive;
            Ok(())
        })
    }

    pub fn delete_ik(&mut self, index: usize) -> Result<(), OpError> {
        self.mutate(|rig| {
            if index >= rig.ik.len() {
                return Err(OpError::Unknown);
            }
            rig.ik.remove(index);
            Ok(())
        })
    }
}

impl Default for RigDoc {
    fn default() -> Self {
        Self::new()
    }
}

fn skin_mut<'a>(rig: &'a mut Rig, name: &str) -> &'a mut Skin {
    let i = match rig.skins.iter().position(|s| s.name == name) {
        Some(i) => i,
        None => {
            let name = if rig.skins.is_empty() { DEFAULT_SKIN } else { name };
            rig.skins.push(Skin { name: name.to_owned(), attachments: Default::default() });
            rig.skins.len() - 1
        }
    };
    &mut rig.skins[i]
}

/// Removes slots, the attachments only they showed, and their clip tracks.
fn remove_slots(rig: &mut Rig, dead: &HashSet<SlotId>) {
    let used: Vec<AttachmentId> =
        rig.slots.iter().filter(|s| dead.contains(&s.id)).filter_map(|s| s.attachment.clone()).collect();
    rig.slots.retain(|s| !dead.contains(&s.id));
    for a in used {
        if !rig.slots.iter().any(|s| s.attachment.as_ref() == Some(&a)) {
            for k in &mut rig.skins {
                k.attachments.remove(&a);
            }
        }
    }
    purge_tracks(rig, &HashSet::new(), dead);
}

fn purge_tracks(rig: &mut Rig, bones: &HashSet<BoneId>, slots: &HashSet<SlotId>) {
    for clip in rig.clips.values_mut() {
        clip.tracks.retain(|t| match t {
            RigTrack::BoneRotate { bone, .. } | RigTrack::BoneTranslate { bone, .. } | RigTrack::BoneScale { bone, .. } => {
                !bones.contains(bone)
            }
            RigTrack::SlotAttachment { slot, .. } | RigTrack::SlotColor { slot, .. } | RigTrack::Deform { slot, .. } => {
                !slots.contains(slot)
            }
            RigTrack::DrawOrder { .. } => true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// root (0,0)->(100,0); child from (100,0) to (100,50).
    fn two_bones() -> (RigDoc, BoneId, BoneId) {
        let mut doc = RigDoc::new();
        let root = doc.add_bone(None, [0.0, 0.0], [100.0, 0.0]).unwrap();
        let child = doc.add_bone(Some(&root), [100.0, 0.0], [100.0, 50.0]).unwrap();
        (doc, root, child)
    }

    #[test]
    fn add_bone_computes_local_rest_from_world_drag() {
        let (doc, root, child) = two_bones();
        let r = bone(doc.rig(), &root).unwrap();
        assert!(near(r.length, 100.0) && near(r.rest.rotation, 0.0));
        let c = bone(doc.rig(), &child).unwrap();
        assert!(near(c.length, 50.0));
        assert!(near(c.rest.rotation, 90.0), "{}", c.rest.rotation);
        assert!(near(c.rest.x, 100.0) && near(c.rest.y, 0.0));
        assert_eq!(c.parent.as_ref(), Some(&root));
        assert!(diagnostics(doc.rig()).iter().all(|x| x.severity != Severity::Error));
    }

    #[test]
    fn child_of_rotated_parent_lands_where_dragged() {
        let mut doc = RigDoc::new();
        let root = doc.add_bone(None, [10.0, 10.0], [10.0, 110.0]).unwrap(); // pointing down
        let child = doc.add_bone(Some(&root), [10.0, 110.0], [60.0, 110.0]).unwrap();
        let w = world_matrices(doc.rig(), &rest_locals(doc.rig()));
        let i = bone_index(doc.rig(), &child).unwrap();
        let c = bone(doc.rig(), &child).unwrap();
        let head = w[i].origin();
        let tip = w[i].apply([c.length, 0.0]);
        assert!(near(head[0], 10.0) && near(head[1], 110.0));
        assert!(near(tip[0], 60.0) && near(tip[1], 110.0), "{tip:?}");
    }

    #[test]
    fn rename_rejects_empty_and_undo_restores() {
        let (mut doc, root, _) = two_bones();
        assert_eq!(doc.rename_bone(&root, "  "), Err(OpError::EmptyName));
        doc.rename_bone(&root, "spine").unwrap();
        assert_eq!(bone(doc.rig(), &root).unwrap().name, "spine");
        assert!(doc.undo());
        assert_eq!(bone(doc.rig(), &root).unwrap().name, "bone");
        assert!(doc.redo());
        assert_eq!(bone(doc.rig(), &root).unwrap().name, "spine");
    }

    #[test]
    fn names_are_made_unique() {
        let (doc, root, child) = two_bones();
        assert_eq!(bone(doc.rig(), &root).unwrap().name, "bone");
        assert_eq!(bone(doc.rig(), &child).unwrap().name, "bone2");
    }

    #[test]
    fn reparent_keeps_world_and_refuses_cycles() {
        let (mut doc, root, child) = two_bones();
        let before = world_matrices(doc.rig(), &rest_locals(doc.rig()));
        let ci = bone_index(doc.rig(), &child).unwrap();
        let head_before = before[ci].origin();
        assert_eq!(doc.reparent(&root, Some(&child)), Err(OpError::Cycle));
        assert_eq!(doc.reparent(&root, Some(&root)), Err(OpError::Cycle));
        doc.reparent(&child, None).unwrap();
        let after = world_matrices(doc.rig(), &rest_locals(doc.rig()));
        let ci = bone_index(doc.rig(), &child).unwrap();
        assert!(near(after[ci].origin()[0], head_before[0]) && near(after[ci].origin()[1], head_before[1]));
        assert!(near(after[ci].x_angle(), 90.0));
        assert!(bone(doc.rig(), &child).unwrap().parent.is_none());
    }

    #[test]
    fn reparent_reorders_parents_before_children() {
        let mut doc = RigDoc::new();
        let a = doc.add_bone(None, [0.0, 0.0], [10.0, 0.0]).unwrap();
        let b = doc.add_bone(None, [0.0, 20.0], [10.0, 20.0]).unwrap();
        let c = doc.add_bone(Some(&b), [10.0, 20.0], [20.0, 20.0]).unwrap();
        // a (index 0) goes under c (index 2): a must now come after c.
        doc.reparent(&a, Some(&c)).unwrap();
        let rig = doc.rig();
        assert!(bone_index(rig, &a).unwrap() > bone_index(rig, &c).unwrap());
        assert!(rig.validate().iter().all(|i| i.severity != Severity::Error));
    }

    #[test]
    fn delete_bone_removes_subtree_slots_and_ik() {
        let (mut doc, root, child) = two_bones();
        let target = doc.add_bone(None, [200.0, 0.0], [220.0, 0.0]).unwrap();
        let slot = doc.add_slot(&child).unwrap();
        doc.attach_image(&slot, DEFAULT_SKIN, "a.png", [10.0, 10.0]).unwrap();
        doc.add_ik(vec![root.clone(), child.clone()], &target).unwrap();
        doc.delete_bone(&root).unwrap();
        let rig = doc.rig();
        assert_eq!(rig.bones.len(), 1);
        assert!(rig.slots.is_empty() && rig.ik.is_empty());
        assert!(rig.skins[0].attachments.is_empty());
        doc.undo();
        assert_eq!(doc.rig().bones.len(), 3);
        assert_eq!(doc.rig().ik.len(), 1);
    }

    #[test]
    fn attach_image_creates_default_skin_and_region() {
        let (mut doc, root, _) = two_bones();
        let s = doc.add_slot(&root).unwrap();
        let att = doc.attach_image(&s, DEFAULT_SKIN, "img/head.png", [64.0, 32.0]).unwrap();
        let rig = doc.rig();
        assert_eq!(rig.skins.len(), 1);
        assert_eq!(rig.skins[0].name, DEFAULT_SKIN);
        assert_eq!(rig.slots[0].attachment.as_ref(), Some(&att));
        assert_eq!(rig.skins[0].attachments[&att].image(), "img/head.png");
        // Another skin overrides the same slot with another picture.
        let outfit = doc.add_skin("outfit").unwrap();
        let att2 = doc.attach_image(&s, &outfit, "img/hat.png", [10.0, 10.0]).unwrap();
        assert_eq!(att, att2);
        assert_eq!(doc.rig().skins[1].attachments[&att].image(), "img/hat.png");
        assert_eq!(doc.rig().skins[0].attachments[&att].image(), "img/head.png");
        doc.detach(&s, &outfit).unwrap();
        assert!(doc.rig().slots[0].attachment.is_some(), "still shown by the default skin");
        doc.detach(&s, DEFAULT_SKIN).unwrap();
        assert!(doc.rig().slots[0].attachment.is_none());
    }

    #[test]
    fn draw_order_moves_and_clamps() {
        let (mut doc, root, child) = two_bones();
        let a = doc.add_slot(&root).unwrap();
        let b = doc.add_slot(&child).unwrap();
        let c = doc.add_slot(&root).unwrap();
        doc.move_slot(&c, -5).unwrap();
        let order: Vec<_> = doc.rig().slots.iter().map(|s| s.id.clone()).collect();
        assert_eq!(order, vec![c.clone(), a.clone(), b.clone()]);
        doc.move_slot(&c, 1).unwrap();
        let order: Vec<_> = doc.rig().slots.iter().map(|s| s.id.clone()).collect();
        assert_eq!(order, vec![a, c, b]);
    }

    #[test]
    fn skins_rules() {
        let (mut doc, root, _) = two_bones();
        let s = doc.add_slot(&root).unwrap();
        doc.attach_image(&s, DEFAULT_SKIN, "x.png", [1.0, 1.0]).unwrap();
        assert_eq!(doc.delete_skin(DEFAULT_SKIN), Err(OpError::KeepSkin));
        let copy = doc.duplicate_skin(DEFAULT_SKIN).unwrap();
        assert_eq!(doc.rig().skins[1].attachments.len(), 1);
        doc.rename_skin(&copy, "winter").unwrap();
        assert!(doc.rename_skin("winter", DEFAULT_SKIN).is_err());
        doc.delete_skin("winter").unwrap();
        assert_eq!(doc.rig().skins.len(), 1);
    }

    #[test]
    fn ik_validation() {
        let (mut doc, root, child) = two_bones();
        let t = doc.add_bone(None, [300.0, 0.0], [310.0, 0.0]).unwrap();
        assert_eq!(doc.add_ik(vec![], &t), Err(OpError::BadIk));
        assert_eq!(doc.add_ik(vec![child.clone(), root.clone()], &t), Err(OpError::BadIk), "not a chain");
        assert_eq!(doc.add_ik(vec![root.clone()], &root), Err(OpError::BadIk), "target in chain");
        doc.add_ik(vec![root.clone(), child.clone()], &t).unwrap();
        doc.update_ik(0, 2.0, true).unwrap();
        assert!(near(doc.rig().ik[0].mix, 1.0) && doc.rig().ik[0].bend_positive);
        assert!(doc.rig().validate().iter().all(|i| i.severity != Severity::Error));
        doc.delete_ik(0).unwrap();
        assert!(doc.rig().ik.is_empty());
    }

    #[test]
    fn gesture_is_one_undo_step() {
        let (mut doc, root, _) = two_bones();
        let base = doc.rig().clone();
        doc.begin_gesture();
        for i in 1..=10 {
            doc.set_rest(&root, Xform { x: i as f32, ..Xform::IDENTITY }).unwrap();
        }
        doc.end_gesture();
        assert!(near(bone(doc.rig(), &root).unwrap().rest.x, 10.0));
        doc.undo();
        assert_eq!(doc.rig(), &base);
        // A gesture that changed nothing leaves no history entry.
        let depth = doc.undo.len();
        doc.begin_gesture();
        doc.end_gesture();
        assert_eq!(doc.undo.len(), depth);
    }

    #[test]
    fn refused_operation_changes_nothing() {
        let (mut doc, root, _) = two_bones();
        let rev = doc.revision;
        let depth = doc.undo.len();
        assert!(doc.reparent(&root, Some(&root)).is_err());
        assert_eq!(doc.revision, rev);
        assert_eq!(doc.undo.len(), depth);
    }

    #[test]
    fn dirty_follows_saved_state() {
        let (mut doc, root, _) = two_bones();
        assert!(doc.dirty());
        let dir = std::env::temp_dir().join(format!("orbit_rig_doc_{}", fresh_id("t", |_| false)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hero.rig.ron");
        doc.save_as(&path).unwrap();
        assert!(!doc.dirty());
        doc.rename_bone(&root, "hip").unwrap();
        assert!(doc.dirty());
        doc.undo();
        assert!(!doc.dirty());
        let loaded = RigDoc::load(&path).unwrap();
        assert_eq!(loaded.rig().bones.len(), 2);
        assert_eq!(loaded.rig(), doc.rig());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn diagnostics_cover_budget_and_editor_checks() {
        let mut doc = RigDoc::new();
        let mut prev = None;
        for i in 0..(WEB_MAX_BONES + 2) {
            let y = i as f32 * 10.0;
            prev = Some(doc.add_bone(prev.as_ref(), [0.0, y], [0.0, y + 10.0]).unwrap());
        }
        let root = doc.rig().bones[0].id.clone();
        let _ = doc.add_slot(&root).unwrap();
        let diags = diagnostics(doc.rig());
        assert!(diags.iter().any(|x| x.key == "rig.diag.web_bones"));
        assert!(diags.iter().any(|x| x.key == "rig.diag.slot_empty"));
        assert!(diags.iter().all(|x| x.severity == Severity::Warning));
    }

    #[test]
    fn decompose_inverts_matrix() {
        let x = Xform { x: 3.0, y: -4.0, rotation: 35.0, sx: 2.0, sy: 0.5 };
        let y = decompose(&x.matrix());
        assert!(near(y.x, 3.0) && near(y.y, -4.0) && near(y.rotation, 35.0) && near(y.sx, 2.0) && near(y.sy, 0.5));
    }
}
