//! Edit operations on a 2D [`Rig`] as pure functions (bones, slots, skins,
//! IK, clips and their keys, "bind to bones", pictures as parts).
//!
//! The MCP `rig_*` tools are built on them: each operation works on a
//! `&mut Rig` the caller has cloned, so a refused operation changes nothing,
//! and the caller validates the result before writing it. The rig editor's
//! own document keeps its operations in the editor crate (they carry undo);
//! the mesh / weight / picture logic is shared through [`crate::rigmesh`].
//!
//! Bones, slots, skins and clips are named by the caller with an id or a
//! name; every lookup says which one it could not find.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use orbit_anim::rig::{
    Affine, Attachment, AttachmentId, Bone, BoneId, Deform, IkConstraint, Rig, RigClip, RigTrack, Skin, Slot, SlotId, Xform,
};
use orbit_anim::{Angle, Curve, Key, Loop, Rgba, Step, Track, Vec2};

use crate::rigmesh::workflow::{self, Picture};
use crate::rigmesh::{meshgen, weights};

/// Name of the fallback skin.
pub const DEFAULT_SKIN: &str = "default";

/// Why an operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpErrorKind {
    /// A bone, slot, skin, clip or attachment that does not exist.
    NotFound,
    /// The arguments make no sense (empty name, NaN, wrong value shape).
    Invalid,
    /// The rig would be left in a state the model refuses (cycle, duplicate).
    Refused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpError {
    pub kind: OpErrorKind,
    pub message: String,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type OpResult<T> = Result<T, OpError>;

fn not_found<T>(m: impl Into<String>) -> OpResult<T> {
    Err(OpError { kind: OpErrorKind::NotFound, message: m.into() })
}

fn invalid<T>(m: impl Into<String>) -> OpResult<T> {
    Err(OpError { kind: OpErrorKind::Invalid, message: m.into() })
}

fn refused<T>(m: impl Into<String>) -> OpResult<T> {
    Err(OpError { kind: OpErrorKind::Refused, message: m.into() })
}

// ---------------------------------------------------------------- helpers

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
    (2..).map(|i| format!("{base}{i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_owned())
}

fn finite(name: &str, v: f32) -> OpResult<f32> {
    if v.is_finite() { Ok(v) } else { invalid(format!("`{name}` must be a finite number")) }
}

fn clean_name(what: &str, name: &str) -> OpResult<String> {
    let n = name.trim();
    if n.is_empty() { invalid(format!("the {what} name cannot be empty")) } else { Ok(n.to_owned()) }
}

/// World matrices of the rest pose.
fn rest_world(rig: &Rig) -> Vec<Affine> {
    let locals: Vec<Xform> = rig.bones.iter().map(|b| b.rest).collect();
    crate::rigmesh::world_matrices(rig, &locals)
}

/// Splits a (shear free) matrix into a local transform.
fn decompose(m: &Affine) -> Xform {
    let sx = m.a.hypot(m.b);
    if sx < 1e-9 {
        return Xform { x: m.tx, y: m.ty, rotation: 0.0, sx: 0.0, sy: 1.0 };
    }
    Xform { x: m.tx, y: m.ty, rotation: m.b.atan2(m.a).to_degrees(), sx, sy: m.det() / sx }
}

/// The bone with this id or this name.
pub fn find_bone(rig: &Rig, key: &str) -> OpResult<usize> {
    if let Some(i) = rig.bones.iter().position(|b| b.id.as_str() == key) {
        return Ok(i);
    }
    let named: Vec<usize> = rig.bones.iter().enumerate().filter(|(_, b)| b.name == key).map(|(i, _)| i).collect();
    match named.as_slice() {
        [i] => Ok(*i),
        [] => not_found(format!(
            "no bone `{key}` (bones: {})",
            rig.bones.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", ")
        )),
        _ => invalid(format!("several bones are named `{key}`: use the id")),
    }
}

/// The slot with this id or this name.
pub fn find_slot(rig: &Rig, key: &str) -> OpResult<usize> {
    if let Some(i) = rig.slots.iter().position(|s| s.id.as_str() == key) {
        return Ok(i);
    }
    let named: Vec<usize> = rig.slots.iter().enumerate().filter(|(_, s)| s.name == key).map(|(i, _)| i).collect();
    match named.as_slice() {
        [i] => Ok(*i),
        [] => not_found(format!(
            "no slot `{key}` (slots: {})",
            rig.slots.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
        )),
        _ => invalid(format!("several slots are named `{key}`: use the id")),
    }
}

fn find_skin(rig: &Rig, name: &str) -> OpResult<usize> {
    rig.skins.iter().position(|s| s.name == name).map_or_else(
        || not_found(format!("no skin `{name}` (skins: {})", rig.skins.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))),
        Ok,
    )
}

fn attachment_exists(rig: &Rig, id: &AttachmentId) -> bool {
    rig.skins.iter().any(|k| k.attachments.contains_key(id))
}

/// `id` and everything under it.
fn subtree(rig: &Rig, id: &BoneId) -> HashSet<BoneId> {
    let mut set = HashSet::new();
    if !rig.bones.iter().any(|b| &b.id == id) {
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
            if b.parent.as_ref().is_none_or(|p| placed.contains(p)) {
                placed.insert(b.id.clone());
                out.push(b);
            } else {
                keep.push(b);
            }
        }
        rest = keep;
        if rest.len() == before {
            for mut b in rest.drain(..) {
                b.parent = None;
                out.push(b);
            }
        }
    }
    rig.bones = out;
}

// ------------------------------------------------------------------ bones

/// A bone's setup pose. Positions and rotations are relative to the parent
/// (`world: false`) or in rig space (`world: true`); the origin is the
/// character's bottom-center, Y points down, rotation is in degrees
/// clockwise on screen. Missing fields keep their value (new bone: 0, 0,
/// unrotated, scale 1, length 50).
#[derive(Clone, Debug, Default)]
pub struct BoneSpec {
    pub name: Option<String>,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub rotation: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    pub length: Option<f32>,
    pub world: bool,
}

fn check_spec(spec: &BoneSpec) -> OpResult<()> {
    for (n, v) in [
        ("x", spec.x),
        ("y", spec.y),
        ("rotation", spec.rotation),
        ("scale_x", spec.scale_x),
        ("scale_y", spec.scale_y),
        ("length", spec.length),
    ] {
        if let Some(v) = v {
            finite(n, v)?;
        }
    }
    if spec.length.is_some_and(|l| l < 0.0) {
        return invalid("`length` cannot be negative");
    }
    Ok(())
}

/// Sets the rest pose of bone `i` from `spec` (see [`BoneSpec`]).
fn apply_pose(rig: &mut Rig, i: usize, spec: &BoneSpec) {
    let mut rest = rig.bones[i].rest;
    if spec.world {
        let world = rest_world(rig);
        let mut w = decompose(&world[i]);
        if let Some(x) = spec.x {
            w.x = x;
        }
        if let Some(y) = spec.y {
            w.y = y;
        }
        if let Some(r) = spec.rotation {
            w.rotation = r;
        }
        let parent_world = rig.bones[i]
            .parent
            .as_ref()
            .and_then(|p| rig.bones.iter().position(|b| &b.id == p))
            .map_or(Affine::IDENTITY, |pi| world[pi]);
        let local = parent_world.inverse().unwrap_or(Affine::IDENTITY).mul(&w.matrix());
        let d = decompose(&local);
        rest.x = d.x;
        rest.y = d.y;
        rest.rotation = d.rotation;
    } else {
        if let Some(x) = spec.x {
            rest.x = x;
        }
        if let Some(y) = spec.y {
            rest.y = y;
        }
        if let Some(r) = spec.rotation {
            rest.rotation = r;
        }
    }
    if let Some(s) = spec.scale_x {
        rest.sx = s;
    }
    if let Some(s) = spec.scale_y {
        rest.sy = s;
    }
    let b = &mut rig.bones[i];
    b.rest = rest;
    if let Some(l) = spec.length {
        b.length = l;
    }
}

fn check_bone_name(rig: &Rig, name: &str, except: Option<usize>) -> OpResult<String> {
    let name = clean_name("bone", name)?;
    if rig.bones.iter().enumerate().any(|(i, b)| Some(i) != except && b.name == name) {
        return refused(format!("a bone is already named `{name}`"));
    }
    Ok(name)
}

/// Adds a bone under `parent` (id or name; `None`: a root bone).
pub fn bone_add(rig: &mut Rig, parent: Option<&str>, spec: &BoneSpec) -> OpResult<BoneId> {
    check_spec(spec)?;
    let parent_idx = match parent {
        Some(p) => Some(find_bone(rig, p)?),
        None => None,
    };
    let name = match &spec.name {
        Some(n) => check_bone_name(rig, n, None)?,
        None => unique_name("bone", rig.bones.iter().map(|b| b.name.as_str())),
    };
    let id = BoneId::new(fresh_id("b", |s| rig.bones.iter().any(|b| b.id.as_str() == s)));
    rig.bones.push(Bone {
        id: id.clone(),
        name,
        parent: parent_idx.map(|i| rig.bones[i].id.clone()),
        rest: Xform::IDENTITY,
        length: 50.0,
    });
    let i = rig.bones.len() - 1;
    apply_pose(rig, i, spec);
    Ok(id)
}

/// Changes a bone: name, setup pose, length, and its parent (`parent`:
/// `Some(None)` makes it a root, `Some(Some(key))` moves it under `key`
/// keeping its place in the world, see [`bone_reparent`]).
pub fn bone_update(rig: &mut Rig, key: &str, parent: Option<Option<&str>>, spec: &BoneSpec) -> OpResult<BoneId> {
    check_spec(spec)?;
    let mut i = find_bone(rig, key)?;
    let id = rig.bones[i].id.clone();
    if let Some(p) = parent {
        bone_reparent(rig, key, p)?;
        i = find_bone(rig, id.as_str())?;
    }
    if let Some(n) = &spec.name {
        let name = check_bone_name(rig, n, Some(i))?;
        rig.bones[i].name = name;
    }
    apply_pose(rig, i, spec);
    Ok(id)
}

/// Moves a bone (and its subtree) under `parent` (`None`: root) keeping its
/// place in the world. A descendant of the bone (or the bone itself) is
/// refused.
pub fn bone_reparent(rig: &mut Rig, key: &str, parent: Option<&str>) -> OpResult<BoneId> {
    let i = find_bone(rig, key)?;
    let id = rig.bones[i].id.clone();
    let pi = match parent {
        Some(p) => Some(find_bone(rig, p)?),
        None => None,
    };
    if let Some(pi) = pi
        && subtree(rig, &id).contains(&rig.bones[pi].id)
    {
        return refused(format!("`{}` is under `{}` (or is it): that would make a loop", rig.bones[pi].name, rig.bones[i].name));
    }
    let new_parent = pi.map(|p| rig.bones[p].id.clone());
    if rig.bones[i].parent == new_parent {
        return Ok(id);
    }
    let world = rest_world(rig);
    let parent_world = pi.map_or(Affine::IDENTITY, |p| world[p]);
    let local = parent_world.inverse().unwrap_or(Affine::IDENTITY).mul(&world[i]);
    let b = &mut rig.bones[i];
    b.rest = decompose(&local);
    b.parent = new_parent;
    fix_order(rig);
    Ok(id)
}

/// What a bone deletion took with it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deleted {
    pub bones: Vec<String>,
    pub slots: Vec<String>,
    pub ik: usize,
    pub tracks: usize,
}

/// Deletes a bone and its descendants, the slots that rode on them and
/// whatever referred to them (IK, clip tracks, mesh weights).
pub fn bone_delete(rig: &mut Rig, key: &str) -> OpResult<Deleted> {
    let i = find_bone(rig, key)?;
    let id = rig.bones[i].id.clone();
    let heir = rig.bones[i].parent.clone();
    let gone = subtree(rig, &id);
    let mut out = Deleted {
        bones: rig.bones.iter().filter(|b| gone.contains(&b.id)).map(|b| b.name.clone()).collect(),
        ..Deleted::default()
    };
    rig.bones.retain(|b| !gone.contains(&b.id));
    let dead_slots: HashSet<SlotId> = rig.slots.iter().filter(|s| gone.contains(&s.bone)).map(|s| s.id.clone()).collect();
    out.slots = rig.slots.iter().filter(|s| dead_slots.contains(&s.id)).map(|s| s.name.clone()).collect();
    let tracks_before: usize = rig.clips.values().map(|c| c.tracks.len()).sum();
    remove_slots(rig, &dead_slots);
    let ik_before = rig.ik.len();
    rig.ik.retain(|c| !c.bones.iter().any(|b| gone.contains(b)) && !gone.contains(&c.target));
    out.ik = ik_before - rig.ik.len();
    purge_bone_tracks(rig, &gone);
    out.tracks = tracks_before - rig.clips.values().map(|c| c.tracks.len()).sum::<usize>();
    // Mesh weights: drop the influences of the deleted bones; a vertex left
    // with none follows the parent of the deleted bone (or the first bone).
    let fallback = heir.or_else(|| rig.bones.first().map(|b| b.id.clone()));
    for skin in &mut rig.skins {
        for att in skin.attachments.values_mut() {
            if let Attachment::Mesh(m) = att {
                for w in &mut m.weights {
                    w.retain(|(b, _)| !gone.contains(b));
                    if w.is_empty()
                        && let Some(f) = &fallback
                    {
                        w.push((f.clone(), 1.0));
                    }
                }
                if rig.bones.is_empty() {
                    m.weights.clear();
                }
            }
        }
    }
    rig.normalize_weights();
    Ok(out)
}

fn purge_bone_tracks(rig: &mut Rig, bones: &HashSet<BoneId>) {
    for clip in rig.clips.values_mut() {
        clip.tracks.retain(|t| match t {
            RigTrack::BoneRotate { bone, .. } | RigTrack::BoneTranslate { bone, .. } | RigTrack::BoneScale { bone, .. } => {
                !bones.contains(bone)
            }
            _ => true,
        });
    }
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
    for clip in rig.clips.values_mut() {
        clip.tracks.retain(|t| match t {
            RigTrack::SlotAttachment { slot, .. } | RigTrack::SlotColor { slot, .. } | RigTrack::Deform { slot, .. } => {
                !dead.contains(slot)
            }
            _ => true,
        });
        for t in &mut clip.tracks {
            if let RigTrack::DrawOrder { track } = t {
                for k in &mut track.keys {
                    k.value.0.retain(|s| !dead.contains(s));
                }
            }
        }
    }
}

// ------------------------------------------------------- parts and binding

/// Adds a picture as a new slot (drawn in front of the others) with its
/// alpha-traced mesh, on `bone` (default: the first root bone). Returns the
/// slot and its attachment id. The mesh is unweighted (rigid on its bone)
/// until [`bind`].
pub fn add_image_part(
    rig: &mut Rig,
    pic: &Picture,
    bone: Option<&str>,
    name: Option<&str>,
    center: Option<[f32; 2]>,
    skin: Option<&str>,
) -> OpResult<(SlotId, AttachmentId)> {
    let bi = match bone {
        Some(b) => find_bone(rig, b)?,
        None => rig
            .bones
            .iter()
            .position(|b| b.parent.is_none())
            .map_or_else(|| refused("the rig has no bone: add one first (rig_bone_add)"), Ok)?,
    };
    if let Some(c) = center
        && !(c[0].is_finite() && c[1].is_finite())
    {
        return invalid("`center` must be finite numbers");
    }
    let skin_name = skin.unwrap_or(DEFAULT_SKIN).to_owned();
    if skin.is_some() && !rig.skins.iter().any(|k| k.name == skin_name) {
        return not_found(format!("no skin `{skin_name}`"));
    }
    let slot_name = match name {
        Some(n) => {
            let n = clean_name("slot", n)?;
            if rig.slots.iter().any(|s| s.name == n) {
                return refused(format!("a slot is already named `{n}`"));
            }
            n
        }
        None => unique_name(&pic.stem(), rig.slots.iter().map(|s| s.name.as_str())),
    };
    let center = center.unwrap_or_else(|| workflow::part_center(rig, pic, workflow::standing_center(pic)));
    let mesh = workflow::mesh_for_picture(pic, center);
    let slot_id = SlotId::new(fresh_id("s", |s| rig.slots.iter().any(|x| x.id.as_str() == s)));
    let att_id = AttachmentId::new(fresh_id("a", |s| rig.skins.iter().any(|k| k.attachments.keys().any(|x| x.as_str() == s))));
    if rig.skins.is_empty() {
        rig.skins.push(Skin { name: DEFAULT_SKIN.to_owned(), attachments: BTreeMap::new() });
    }
    let skin_name = if rig.skins.iter().any(|k| k.name == skin_name) { skin_name } else { rig.skins[0].name.clone() };
    let si = find_skin(rig, &skin_name)?;
    rig.skins[si].attachments.insert(att_id.clone(), Attachment::Mesh(mesh));
    rig.slots.push(Slot {
        id: slot_id.clone(),
        name: slot_name,
        bone: rig.bones[bi].id.clone(),
        attachment: Some(att_id.clone()),
        color: Rgba::new(1.0, 1.0, 1.0, 1.0),
    });
    Ok((slot_id, att_id))
}

/// A new rig from one picture: a `root` bone at the feet pointing up, a slot
/// named after the file and its mesh, the picture standing on the origin.
pub fn rig_from_picture(pic: &Picture) -> OpResult<(Rig, BoneId, SlotId)> {
    let mut rig = Rig::default();
    let len = workflow::root_length(pic);
    let root = bone_add(
        &mut rig,
        None,
        &BoneSpec { name: Some("root".into()), rotation: Some(-90.0), length: Some(len), ..BoneSpec::default() },
    )?;
    let (slot, _) = add_image_part(&mut rig, pic, None, None, Some(workflow::standing_center(pic)), None)?;
    Ok((rig, root, slot))
}

/// "Bind to bones": automatic weights for every mesh of every skin from the
/// current bones (distance to bone segments, smoothed). Returns how many
/// meshes and vertices were bound.
pub fn bind(rig: &mut Rig, falloff: Option<f32>) -> OpResult<(usize, usize)> {
    if rig.bones.is_empty() {
        return refused("the rig has no bone to bind to (rig_bone_add)");
    }
    let falloff = finite("falloff", falloff.unwrap_or(3.0))?;
    let snapshot = rig.clone();
    let (mut meshes, mut vertices) = (0, 0);
    for skin in &mut rig.skins {
        for att in skin.attachments.values_mut() {
            if let Attachment::Mesh(m) = att {
                m.weights = weights::bind_mesh(&snapshot, m, falloff);
                meshes += 1;
                vertices += m.vertices.len();
            }
        }
    }
    if meshes == 0 {
        return refused("the rig has no mesh to bind (rig_add_image_part makes one, rig_mesh_auto converts a region)");
    }
    Ok((meshes, vertices))
}

/// Picture pixels to rig space for a region attachment in the setup pose
/// (used by callers that regenerate meshes).
pub fn region_to_rig(rig: &Rig, slot: &Slot, region_xform: &Xform, size: [f32; 2]) -> Option<Affine> {
    let world = rest_world(rig);
    let bi = rig.bones.iter().position(|b| b.id == slot.bone)?;
    Some(meshgen::region_uv_to_rig(&world[bi].mul(&region_xform.matrix()), size))
}

// ------------------------------------------------------------------ slots

/// Changes of a slot; missing fields keep their value.
#[derive(Clone, Debug, Default)]
pub struct SlotPatch {
    pub name: Option<String>,
    /// Attachment shown in the setup pose (`""`: none).
    pub attachment: Option<String>,
    pub color: Option<[f32; 4]>,
    pub bone: Option<String>,
    /// New position in the draw order (0 = behind, last = in front).
    pub draw_order: Option<usize>,
}

pub fn slot_update(rig: &mut Rig, key: &str, patch: &SlotPatch) -> OpResult<SlotId> {
    let i = find_slot(rig, key)?;
    let id = rig.slots[i].id.clone();
    if let Some(n) = &patch.name {
        let n = clean_name("slot", n)?;
        if rig.slots.iter().enumerate().any(|(j, s)| j != i && s.name == n) {
            return refused(format!("a slot is already named `{n}`"));
        }
        rig.slots[i].name = n;
    }
    if let Some(a) = &patch.attachment {
        if a.is_empty() {
            rig.slots[i].attachment = None;
        } else {
            let aid = AttachmentId::new(a.clone());
            if !attachment_exists(rig, &aid) {
                let known: Vec<String> =
                    rig.skins.iter().flat_map(|k| k.attachments.keys().map(ToString::to_string)).collect::<HashSet<_>>().into_iter().collect();
                return not_found(format!("no attachment `{a}` in any skin (attachments: {})", known.join(", ")));
            }
            rig.slots[i].attachment = Some(aid);
        }
    }
    if let Some(c) = patch.color {
        rig.slots[i].color = color_of(c)?;
    }
    if let Some(b) = &patch.bone {
        let bi = find_bone(rig, b)?;
        rig.slots[i].bone = rig.bones[bi].id.clone();
    }
    if let Some(pos) = patch.draw_order {
        if pos >= rig.slots.len() {
            return invalid(format!("`draw_order` is 0 to {} (0 = behind)", rig.slots.len() - 1));
        }
        let s = rig.slots.remove(i);
        rig.slots.insert(pos, s);
    }
    Ok(id)
}

fn color_of(c: [f32; 4]) -> OpResult<Rgba> {
    if c.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
        return invalid("a color is [r, g, b, a], each from 0 to 1");
    }
    Ok(Rgba::new(c[0], c[1], c[2], c[3]))
}

// ------------------------------------------------------------------ skins

pub fn skin_add(rig: &mut Rig, name: &str) -> OpResult<String> {
    let name = clean_name("skin", name)?;
    if rig.skins.iter().any(|s| s.name == name) {
        return refused(format!("a skin is already named `{name}`"));
    }
    rig.skins.push(Skin { name: name.clone(), attachments: BTreeMap::new() });
    Ok(name)
}

pub fn skin_duplicate(rig: &mut Rig, from: &str, name: Option<&str>) -> OpResult<String> {
    let si = find_skin(rig, from)?;
    let name = match name {
        Some(n) => {
            let n = clean_name("skin", n)?;
            if rig.skins.iter().any(|s| s.name == n) {
                return refused(format!("a skin is already named `{n}`"));
            }
            n
        }
        None => unique_name(&format!("{from} copy"), rig.skins.iter().map(|s| s.name.as_str())),
    };
    let attachments = rig.skins[si].attachments.clone();
    rig.skins.push(Skin { name: name.clone(), attachments });
    Ok(name)
}

pub fn skin_delete(rig: &mut Rig, name: &str) -> OpResult<()> {
    let si = find_skin(rig, name)?;
    if name == DEFAULT_SKIN {
        return refused("the `default` skin cannot be deleted (the others fall back on it)");
    }
    if rig.skins.len() <= 1 {
        return refused("the last skin cannot be deleted");
    }
    rig.skins.remove(si);
    Ok(())
}

// --------------------------------------------------------------------- IK

/// Creates the IK constraint on this chain (1 bone: aims at the target; 2
/// bones `[parent, child]`: bends the pair), or updates the one that already
/// has the same chain. Returns its index.
pub fn ik_set(rig: &mut Rig, bones: &[String], target: &str, bend_positive: Option<bool>, mix: Option<f32>) -> OpResult<usize> {
    if !(1..=2).contains(&bones.len()) {
        return invalid("an IK chain is 1 bone (aims at the target) or 2 bones [parent, child]");
    }
    let chain: Vec<BoneId> = bones
        .iter()
        .map(|b| find_bone(rig, b).map(|i| rig.bones[i].id.clone()))
        .collect::<OpResult<_>>()?;
    let target = rig.bones[find_bone(rig, target)?].id.clone();
    if chain.contains(&target) {
        return refused("the IK target cannot be part of the chain");
    }
    if chain.len() == 2 {
        let child = rig.bones.iter().find(|b| b.id == chain[1]);
        if child.and_then(|b| b.parent.as_ref()) != Some(&chain[0]) {
            return refused("in a 2-bone chain the second bone must be a child of the first");
        }
    }
    if let Some(m) = mix {
        finite("mix", m)?;
    }
    let existing = rig.ik.iter().position(|c| c.bones == chain);
    let index = match existing {
        Some(i) => {
            rig.ik[i].target = target;
            i
        }
        None => {
            rig.ik.push(IkConstraint { bones: chain, target, bend_positive: false, mix: 1.0 });
            rig.ik.len() - 1
        }
    };
    if let Some(b) = bend_positive {
        rig.ik[index].bend_positive = b;
    }
    if let Some(m) = mix {
        rig.ik[index].mix = m.clamp(0.0, 1.0);
    }
    Ok(index)
}

/// Removes an IK constraint, by index or by its chain.
pub fn ik_remove(rig: &mut Rig, index: Option<usize>, bones: Option<&[String]>) -> OpResult<usize> {
    let i = match (index, bones) {
        (Some(i), None) => i,
        (None, Some(b)) => {
            let chain: Vec<BoneId> = b
                .iter()
                .map(|k| find_bone(rig, k).map(|i| rig.bones[i].id.clone()))
                .collect::<OpResult<_>>()?;
            match rig.ik.iter().position(|c| c.bones == chain) {
                Some(i) => i,
                None => return not_found("no IK constraint on that chain (rig_read lists them)"),
            }
        }
        _ => return invalid("give either `index` or `bones`"),
    };
    if i >= rig.ik.len() {
        return not_found(format!("no IK constraint {i} (the rig has {})", rig.ik.len()));
    }
    rig.ik.remove(i);
    Ok(i)
}

// ------------------------------------------------------------------ clips

fn check_duration(d: f32) -> OpResult<f32> {
    finite("duration", d)?;
    if d <= 0.0 || d > 3600.0 {
        return invalid("`duration` is in seconds, above 0 and at most 3600");
    }
    Ok(d)
}

pub fn clip_create(rig: &mut Rig, name: &str, duration: f32, looping: Loop) -> OpResult<()> {
    let name = clean_name("clip", name)?;
    let duration = check_duration(duration)?;
    if rig.clips.contains_key(&name) {
        return refused(format!("the rig already has a clip `{name}`"));
    }
    rig.clips.insert(name, RigClip::new(duration, looping));
    Ok(())
}

fn clip_mut<'a>(rig: &'a mut Rig, name: &str) -> OpResult<&'a mut RigClip> {
    let known = rig.clips.keys().cloned().collect::<Vec<_>>().join(", ");
    match rig.clips.get_mut(name) {
        Some(c) => Ok(c),
        None => not_found(format!("no clip `{name}` (clips: {known})")),
    }
}

/// Changes a clip's duration and / or loop mode. Shortening below the last
/// key is refused (the keys would never play).
pub fn clip_update(rig: &mut Rig, name: &str, duration: Option<f32>, looping: Option<Loop>) -> OpResult<()> {
    let clip = clip_mut(rig, name)?;
    if let Some(d) = duration {
        let d = check_duration(d)?;
        let last = clip.tracks.iter().map(RigTrack::end_time).fold(0.0_f32, f32::max);
        if d + 1e-4 < last {
            return refused(format!("the clip has a key at {last}s: delete it (rig_clip_key_delete) before shortening to {d}s"));
        }
        clip.duration = d;
    }
    if let Some(l) = looping {
        clip.looping = l;
    }
    Ok(())
}

pub fn clip_delete(rig: &mut Rig, name: &str) -> OpResult<()> {
    clip_mut(rig, name)?;
    rig.clips.remove(name);
    Ok(())
}

/// The kind of a clip track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    BoneRotate,
    BoneTranslate,
    BoneScale,
    SlotAttachment,
    SlotColor,
    Deform,
    DrawOrder,
}

impl TrackKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "bone_rotate" => Self::BoneRotate,
            "bone_translate" => Self::BoneTranslate,
            "bone_scale" => Self::BoneScale,
            "slot_attachment" => Self::SlotAttachment,
            "slot_color" => Self::SlotColor,
            "deform" => Self::Deform,
            "draw_order" => Self::DrawOrder,
            _ => return None,
        })
    }

    pub fn is_bone(self) -> bool {
        matches!(self, Self::BoneRotate | Self::BoneTranslate | Self::BoneScale)
    }
}

/// The value of a key.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyValue {
    /// Degrees, added to the setup rotation.
    Angle(f32),
    /// Translation added to the setup position, or scale factor.
    Vec2([f32; 2]),
    /// Attachment id shown (`None`: hidden).
    Attachment(Option<String>),
    Color([f32; 4]),
    /// One `[dx, dy]` offset per vertex of the mesh.
    Deform(Vec<[f32; 2]>),
    /// Slot keys, behind to front.
    Order(Vec<String>),
}

/// Where a key goes.
#[derive(Clone, Debug)]
pub struct KeyTarget<'a> {
    pub clip: &'a str,
    pub kind: TrackKind,
    /// Bone or slot (id or name); not used by `DrawOrder`.
    pub target: Option<&'a str>,
    /// Deform only: the mesh attachment (default: the slot's own).
    pub attachment: Option<&'a str>,
}

/// What a key operation did.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyReport {
    pub clip: String,
    pub track: usize,
    pub keys: usize,
    /// A key at that time existed and was replaced / removed.
    pub existing: bool,
}

enum Resolved {
    Bone(BoneId),
    Slot(SlotId),
    Deform(SlotId, AttachmentId),
    Order,
}

fn resolve(rig: &Rig, t: &KeyTarget<'_>) -> OpResult<Resolved> {
    let need = || t.target.ok_or_else(|| OpError { kind: OpErrorKind::Invalid, message: "`target` (a bone or a slot) is required for this track".into() });
    Ok(match t.kind {
        k if k.is_bone() => Resolved::Bone(rig.bones[find_bone(rig, need()?)?].id.clone()),
        TrackKind::SlotAttachment | TrackKind::SlotColor => Resolved::Slot(rig.slots[find_slot(rig, need()?)?].id.clone()),
        TrackKind::Deform => {
            let slot = &rig.slots[find_slot(rig, need()?)?];
            let att = match t.attachment {
                Some(a) => AttachmentId::new(a),
                None => slot.attachment.clone().ok_or_else(|| OpError {
                    kind: OpErrorKind::Invalid,
                    message: format!("slot `{}` shows no attachment: pass `attachment`", slot.name),
                })?,
            };
            if !rig.skins.iter().any(|k| matches!(k.attachments.get(&att), Some(Attachment::Mesh(_)))) {
                return refused(format!("`{att}` is not a mesh (deform needs a mesh: rig_mesh_auto / rig_add_image_part)"));
            }
            Resolved::Deform(slot.id.clone(), att)
        }
        _ => Resolved::Order,
    })
}

/// Index of the track of this kind and target in the clip, if any.
fn find_track(clip: &RigClip, kind: TrackKind, r: &Resolved) -> Option<usize> {
    clip.tracks.iter().position(|t| match (kind, t, r) {
        (TrackKind::BoneRotate, RigTrack::BoneRotate { bone, .. }, Resolved::Bone(b)) => bone == b,
        (TrackKind::BoneTranslate, RigTrack::BoneTranslate { bone, .. }, Resolved::Bone(b)) => bone == b,
        (TrackKind::BoneScale, RigTrack::BoneScale { bone, .. }, Resolved::Bone(b)) => bone == b,
        (TrackKind::SlotAttachment, RigTrack::SlotAttachment { slot, .. }, Resolved::Slot(s)) => slot == s,
        (TrackKind::SlotColor, RigTrack::SlotColor { slot, .. }, Resolved::Slot(s)) => slot == s,
        (TrackKind::Deform, RigTrack::Deform { slot, attachment, .. }, Resolved::Deform(s, a)) => slot == s && attachment == a,
        (TrackKind::DrawOrder, RigTrack::DrawOrder { .. }, Resolved::Order) => true,
        _ => false,
    })
}

const TIME_EPS: f32 = 1e-3;

fn upsert<T>(track: &mut Track<T>, t: f32, value: T, ease: Option<Curve>) -> bool {
    if let Some(k) = track.keys.iter_mut().find(|k| (k.t - t).abs() < TIME_EPS) {
        k.value = value;
        if let Some(e) = ease {
            k.ease = e;
        }
        return true;
    }
    let at = track.keys.partition_point(|k| k.t < t);
    track.keys.insert(at, Key { t, value, ease: ease.unwrap_or_default() });
    false
}

fn remove_key<T>(track: &mut Track<T>, t: f32) -> bool {
    match track.keys.iter().position(|k| (k.t - t).abs() < TIME_EPS) {
        Some(i) => {
            track.keys.remove(i);
            true
        }
        None => false,
    }
}

/// Sets (creates or replaces) the key at `time` of a track; the track is
/// created when the clip has none for that target. `ease` shapes the segment
/// from this key to the next.
pub fn clip_key_set(rig: &mut Rig, at: &KeyTarget<'_>, time: f32, value: &KeyValue, ease: Option<Curve>) -> OpResult<KeyReport> {
    finite("time", time)?;
    if time < 0.0 {
        return invalid("`time` cannot be negative");
    }
    let resolved = resolve(rig, at)?;
    let duration = clip_mut(rig, at.clip)?.duration;
    if time > duration + TIME_EPS {
        return refused(format!("time {time}s is past the end of the clip ({duration}s): lengthen it first (rig_clip_update)"));
    }
    // Value-specific checks need the whole rig.
    let converted: RigTrack = match (at.kind, &resolved, value) {
        (TrackKind::BoneRotate, Resolved::Bone(b), KeyValue::Angle(a)) => {
            finite("value", *a)?;
            RigTrack::BoneRotate { bone: b.clone(), track: Track::new(vec![Key::new(time, Angle(*a))]) }
        }
        (TrackKind::BoneTranslate, Resolved::Bone(b), KeyValue::Vec2(v)) => {
            v.iter().try_for_each(|x| finite("value", *x).map(|_| ()))?;
            RigTrack::BoneTranslate { bone: b.clone(), track: Track::new(vec![Key::new(time, Vec2 { x: v[0], y: v[1] })]) }
        }
        (TrackKind::BoneScale, Resolved::Bone(b), KeyValue::Vec2(v)) => {
            v.iter().try_for_each(|x| finite("value", *x).map(|_| ()))?;
            RigTrack::BoneScale { bone: b.clone(), track: Track::new(vec![Key::new(time, Vec2 { x: v[0], y: v[1] })]) }
        }
        (TrackKind::SlotAttachment, Resolved::Slot(s), KeyValue::Attachment(a)) => {
            let id = match a {
                Some(a) if !a.is_empty() => {
                    let id = AttachmentId::new(a.clone());
                    if !attachment_exists(rig, &id) {
                        return not_found(format!("no attachment `{a}` in any skin"));
                    }
                    Some(id)
                }
                _ => None,
            };
            RigTrack::SlotAttachment { slot: s.clone(), track: Track::new(vec![Key::new(time, Step(id))]) }
        }
        (TrackKind::SlotColor, Resolved::Slot(s), KeyValue::Color(c)) => {
            RigTrack::SlotColor { slot: s.clone(), track: Track::new(vec![Key::new(time, color_of(*c)?)]) }
        }
        (TrackKind::Deform, Resolved::Deform(s, a), KeyValue::Deform(d)) => {
            if d.iter().flatten().any(|v| !v.is_finite()) {
                return invalid("deform offsets must be finite numbers");
            }
            for k in &rig.skins {
                if let Some(Attachment::Mesh(m)) = k.attachments.get(a)
                    && m.vertices.len() != d.len()
                {
                    return refused(format!(
                        "mesh `{a}` has {} vertices but the deform has {} offsets (rig_read shows the count)",
                        m.vertices.len(),
                        d.len()
                    ));
                }
            }
            RigTrack::Deform { slot: s.clone(), attachment: a.clone(), track: Track::new(vec![Key::new(time, Deform(d.clone()))]) }
        }
        (TrackKind::DrawOrder, Resolved::Order, KeyValue::Order(names)) => {
            let mut ids = Vec::with_capacity(names.len());
            for n in names {
                let id = rig.slots[find_slot(rig, n)?].id.clone();
                if ids.contains(&id) {
                    return invalid(format!("slot `{n}` is listed twice"));
                }
                ids.push(id);
            }
            RigTrack::DrawOrder { track: Track::new(vec![Key::new(time, Step(ids))]) }
        }
        _ => {
            return invalid(format!(
                "the value does not fit a {:?} track (rotate: a number; translate / scale: [x, y]; slot_attachment: an attachment id or null; slot_color: [r, g, b, a]; deform: [[dx, dy], ...]; draw_order: [slot, ...])",
                at.kind
            ));
        }
    };
    let clip = clip_mut(rig, at.clip)?;
    let index = match find_track(clip, at.kind, &resolved) {
        Some(i) => i,
        None => {
            clip.tracks.push(match &converted {
                RigTrack::BoneRotate { bone, .. } => RigTrack::BoneRotate { bone: bone.clone(), track: Track::default() },
                RigTrack::BoneTranslate { bone, .. } => RigTrack::BoneTranslate { bone: bone.clone(), track: Track::default() },
                RigTrack::BoneScale { bone, .. } => RigTrack::BoneScale { bone: bone.clone(), track: Track::default() },
                RigTrack::SlotAttachment { slot, .. } => RigTrack::SlotAttachment { slot: slot.clone(), track: Track::default() },
                RigTrack::SlotColor { slot, .. } => RigTrack::SlotColor { slot: slot.clone(), track: Track::default() },
                RigTrack::Deform { slot, attachment, .. } => {
                    RigTrack::Deform { slot: slot.clone(), attachment: attachment.clone(), track: Track::default() }
                }
                RigTrack::DrawOrder { .. } => RigTrack::DrawOrder { track: Track::default() },
            });
            clip.tracks.len() - 1
        }
    };
    let mut replaced = false;
    let keys = match (&mut clip.tracks[index], converted) {
        (RigTrack::BoneRotate { track, .. }, RigTrack::BoneRotate { track: n, .. }) => {
            replaced = upsert(track, time, n.keys[0].value, ease);
            track.keys.len()
        }
        (RigTrack::BoneTranslate { track, .. }, RigTrack::BoneTranslate { track: n, .. })
        | (RigTrack::BoneScale { track, .. }, RigTrack::BoneScale { track: n, .. }) => {
            replaced = upsert(track, time, n.keys[0].value, ease);
            track.keys.len()
        }
        (RigTrack::SlotAttachment { track, .. }, RigTrack::SlotAttachment { track: mut n, .. }) => {
            replaced = upsert(track, time, n.keys.remove(0).value, ease);
            track.keys.len()
        }
        (RigTrack::SlotColor { track, .. }, RigTrack::SlotColor { track: n, .. }) => {
            replaced = upsert(track, time, n.keys[0].value, ease);
            track.keys.len()
        }
        (RigTrack::Deform { track, .. }, RigTrack::Deform { track: mut n, .. }) => {
            replaced = upsert(track, time, n.keys.remove(0).value, ease);
            track.keys.len()
        }
        (RigTrack::DrawOrder { track }, RigTrack::DrawOrder { track: mut n }) => {
            replaced = upsert(track, time, n.keys.remove(0).value, ease);
            track.keys.len()
        }
        _ => 0,
    };
    Ok(KeyReport { clip: at.clip.to_owned(), track: index, keys, existing: replaced })
}

/// Deletes the key at `time` (within a millisecond); an emptied track is
/// removed too.
pub fn clip_key_delete(rig: &mut Rig, at: &KeyTarget<'_>, time: f32) -> OpResult<KeyReport> {
    finite("time", time)?;
    let resolved = resolve(rig, at)?;
    let clip = clip_mut(rig, at.clip)?;
    let Some(index) = find_track(clip, at.kind, &resolved) else {
        return not_found("the clip has no such track (rig_read lists the clips; rig_clip_key_set creates tracks)");
    };
    let (found, left) = match &mut clip.tracks[index] {
        RigTrack::BoneRotate { track, .. } => (remove_key(track, time), track.keys.len()),
        RigTrack::BoneTranslate { track, .. } | RigTrack::BoneScale { track, .. } => (remove_key(track, time), track.keys.len()),
        RigTrack::SlotAttachment { track, .. } => (remove_key(track, time), track.keys.len()),
        RigTrack::SlotColor { track, .. } => (remove_key(track, time), track.keys.len()),
        RigTrack::Deform { track, .. } => (remove_key(track, time), track.keys.len()),
        RigTrack::DrawOrder { track } => (remove_key(track, time), track.keys.len()),
    };
    if !found {
        return not_found(format!("no key at {time}s on that track"));
    }
    if left == 0 {
        clip.tracks.remove(index);
    }
    Ok(KeyReport { clip: at.clip.to_owned(), track: index, keys: left, existing: true })
}

/// A named easing preset (`InOutQuad`, `in_out_quad`, `out-back`...).
pub fn ease_by_name(name: &str) -> Option<Curve> {
    let norm: String = name.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_ascii_lowercase();
    orbit_anim::Ease::ALL.iter().find(|e| e.name().to_ascii_lowercase() == norm).map(|e| Curve::Preset(*e))
}
