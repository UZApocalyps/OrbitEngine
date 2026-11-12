//! Clip editing model of the rig editor (M5B-06): clip list operations, the
//! typed tracks of a [`RigClip`] seen as rows of keys (so the shared timeline
//! widgets and key edits of `timeline_model` apply to them), and the
//! *recording* of poses into keys (auto-key). Pure and headless: no egui, no
//! document, unit tested. The panel commits each result through the
//! document, so every gesture is one undo step.

use std::collections::BTreeSet;

use orbit_anim::rig::{
    Attachment, AttachmentId, Bone, BoneId, ClipBlend, Deform, Rig, RigClip, RigTrack, SlotId, Xform,
};
use orbit_anim::{Angle, Curve, Lerp, Loop, Rgba, Step, Track, Vec2};

use super::model::{OpError, unique_name};
use crate::builtin::timeline_model::{KeyRef, MIN_GAP, move_in_track, put_in_track, round_time};

/// Length of a new clip, seconds.
pub const DEFAULT_DURATION: f32 = 1.0;
const EPS: f32 = 1e-4;

// ---------------------------------------------------------------- clip list

pub fn add_clip(rig: &mut Rig, base: &str) -> Result<String, OpError> {
    let base = if base.trim().is_empty() { "clip" } else { base.trim() };
    let name = unique_name(base, rig.clips.keys().map(String::as_str));
    rig.clips.insert(name.clone(), RigClip::new(DEFAULT_DURATION, Loop::Forever));
    Ok(name)
}

pub fn rename_clip(rig: &mut Rig, from: &str, to: &str) -> Result<(), OpError> {
    let to = to.trim();
    if to.is_empty() {
        return Err(OpError::EmptyName);
    }
    if from == to {
        return Ok(());
    }
    if rig.clips.contains_key(to) {
        return Err(OpError::Exists);
    }
    let clip = rig.clips.remove(from).ok_or(OpError::Unknown)?;
    rig.clips.insert(to.to_owned(), clip);
    Ok(())
}

pub fn duplicate_clip(rig: &mut Rig, from: &str) -> Result<String, OpError> {
    let clip = rig.clips.get(from).ok_or(OpError::Unknown)?.clone();
    let name = unique_name(&format!("{from} copy"), rig.clips.keys().map(String::as_str));
    rig.clips.insert(name.clone(), clip);
    Ok(name)
}

pub fn delete_clip(rig: &mut Rig, name: &str) -> Result<(), OpError> {
    rig.clips.remove(name).map(|_| ()).ok_or(OpError::Unknown)
}

fn clip_mut<'a>(rig: &'a mut Rig, name: &str) -> Result<&'a mut RigClip, OpError> {
    rig.clips.get_mut(name).ok_or(OpError::Unknown)
}

/// Keys past the new end are kept (they just stop playing), so shortening
/// then lengthening again loses nothing.
pub fn set_duration(rig: &mut Rig, name: &str, duration: f32) -> Result<(), OpError> {
    clip_mut(rig, name)?.duration = if duration.is_finite() { duration.max(0.0) } else { 0.0 };
    Ok(())
}

pub fn set_loop(rig: &mut Rig, name: &str, looping: Loop) -> Result<(), OpError> {
    clip_mut(rig, name)?.looping = looping;
    Ok(())
}

// ------------------------------------------------------------ track kinds

/// What a track animates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Rotate,
    Translate,
    Scale,
    Attachment,
    Color,
    Deform,
    DrawOrder,
}

/// The object a track is aimed at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Bone(BoneId),
    Slot(SlotId),
    Deform(SlotId, AttachmentId),
    DrawOrder,
}

pub fn track_kind(t: &RigTrack) -> (Channel, Target) {
    match t {
        RigTrack::BoneRotate { bone, .. } => (Channel::Rotate, Target::Bone(bone.clone())),
        RigTrack::BoneTranslate { bone, .. } => (Channel::Translate, Target::Bone(bone.clone())),
        RigTrack::BoneScale { bone, .. } => (Channel::Scale, Target::Bone(bone.clone())),
        RigTrack::SlotAttachment { slot, .. } => (Channel::Attachment, Target::Slot(slot.clone())),
        RigTrack::SlotColor { slot, .. } => (Channel::Color, Target::Slot(slot.clone())),
        RigTrack::Deform { slot, attachment, .. } => (Channel::Deform, Target::Deform(slot.clone(), attachment.clone())),
        RigTrack::DrawOrder { .. } => (Channel::DrawOrder, Target::DrawOrder),
    }
}

pub fn find_track(clip: &RigClip, channel: Channel, target: &Target) -> Option<usize> {
    clip.tracks.iter().position(|t| track_kind(t) == (channel, target.clone()))
}

/// An empty track for `channel` aimed at `target`; `None` when they do not
/// go together.
pub fn empty_track(channel: Channel, target: &Target) -> Option<RigTrack> {
    Some(match (channel, target) {
        (Channel::Rotate, Target::Bone(b)) => RigTrack::BoneRotate { bone: b.clone(), track: Track::default() },
        (Channel::Translate, Target::Bone(b)) => RigTrack::BoneTranslate { bone: b.clone(), track: Track::default() },
        (Channel::Scale, Target::Bone(b)) => RigTrack::BoneScale { bone: b.clone(), track: Track::default() },
        (Channel::Attachment, Target::Slot(s)) => RigTrack::SlotAttachment { slot: s.clone(), track: Track::default() },
        (Channel::Color, Target::Slot(s)) => RigTrack::SlotColor { slot: s.clone(), track: Track::default() },
        (Channel::Deform, Target::Deform(s, a)) => {
            RigTrack::Deform { slot: s.clone(), attachment: a.clone(), track: Track::default() }
        }
        (Channel::DrawOrder, Target::DrawOrder) => RigTrack::DrawOrder { track: Track::default() },
        _ => return None,
    })
}

/// The setup-pose value of a track, as a key at `t` (what "add track" puts
/// first, so adding a track changes nothing visible).
fn setup_key(rig: &Rig, track: &mut RigTrack, t: f32) {
    let t = round_time(t);
    match track {
        RigTrack::BoneRotate { track, .. } => {
            put_in_track(track, t, Angle(0.0), None);
        }
        RigTrack::BoneTranslate { track, .. } => {
            put_in_track(track, t, Vec2::new(0.0, 0.0), None);
        }
        RigTrack::BoneScale { track, .. } => {
            put_in_track(track, t, Vec2::new(1.0, 1.0), None);
        }
        RigTrack::SlotAttachment { slot, track } => {
            let a = rig.slots.iter().find(|s| &s.id == slot).and_then(|s| s.attachment.clone());
            put_in_track(track, t, Step(a), None);
        }
        RigTrack::SlotColor { slot, track } => {
            let c = rig.slots.iter().find(|s| &s.id == slot).map_or(Rgba::new(1.0, 1.0, 1.0, 1.0), |s| s.color);
            put_in_track(track, t, c, None);
        }
        RigTrack::Deform { slot, attachment, track } => {
            let n = mesh_vertex_count(rig, slot, attachment);
            put_in_track(track, t, Deform(vec![[0.0, 0.0]; n]), None);
        }
        RigTrack::DrawOrder { track } => {
            put_in_track(track, t, Step(rig.slots.iter().map(|s| s.id.clone()).collect()), None);
        }
    }
}

/// Vertex count of the mesh `attachment` (in any skin) of the slot; 0 when it
/// is not a mesh.
pub fn mesh_vertex_count(rig: &Rig, _slot: &SlotId, attachment: &AttachmentId) -> usize {
    rig.skins
        .iter()
        .find_map(|s| match s.attachments.get(attachment) {
            Some(Attachment::Mesh(m)) => Some(m.vertices.len()),
            _ => None,
        })
        .unwrap_or(0)
}

/// Adds a track (with its first key at `t` holding the setup value). An
/// existing track is returned as is. `None` when the target does not exist
/// or does not fit the channel.
pub fn add_track(rig: &mut Rig, name: &str, channel: Channel, target: &Target, t: f32) -> Option<usize> {
    let valid = match target {
        Target::Bone(b) => rig.bones.iter().any(|x| &x.id == b),
        Target::Slot(s) => rig.slots.iter().any(|x| &x.id == s),
        Target::Deform(s, a) => rig.slots.iter().any(|x| &x.id == s) && mesh_vertex_count(rig, s, a) > 0,
        Target::DrawOrder => true,
    };
    if !valid {
        return None;
    }
    if let Some(i) = rig.clips.get(name).and_then(|c| find_track(c, channel, target)) {
        return Some(i);
    }
    let mut track = empty_track(channel, target)?;
    setup_key(rig, &mut track, t);
    let clip = rig.clips.get_mut(name)?;
    clip.tracks.push(track);
    Some(clip.tracks.len() - 1)
}

// ---------------------------------------------------------- tracks as rows

macro_rules! each {
    ($t:expr, $tr:ident => $body:expr) => {
        match $t {
            RigTrack::BoneRotate { track: $tr, .. } => $body,
            RigTrack::BoneTranslate { track: $tr, .. } => $body,
            RigTrack::BoneScale { track: $tr, .. } => $body,
            RigTrack::SlotAttachment { track: $tr, .. } => $body,
            RigTrack::SlotColor { track: $tr, .. } => $body,
            RigTrack::Deform { track: $tr, .. } => $body,
            RigTrack::DrawOrder { track: $tr, .. } => $body,
        }
    };
}

pub fn key_times(t: &RigTrack) -> Vec<f32> {
    each!(t, tr => tr.keys.iter().map(|k| k.t).collect())
}

pub fn key_count(t: &RigTrack) -> usize {
    each!(t, tr => tr.keys.len())
}

pub fn key_ease(t: &RigTrack, i: usize) -> Option<Curve> {
    each!(t, tr => tr.keys.get(i).map(|k| k.ease.clone()))
}

pub fn set_key_ease(t: &mut RigTrack, i: usize, ease: Curve) {
    each!(t, tr => if let Some(k) = tr.keys.get_mut(i) { k.ease = ease })
}

pub fn remove_key(t: &mut RigTrack, i: usize) {
    each!(t, tr => if i < tr.keys.len() { tr.keys.remove(i); })
}

/// `animated[w]`: the value changes between key `w` and `w + 1`.
pub fn segments_animated(t: &RigTrack) -> Vec<bool> {
    each!(t, tr => tr.keys.windows(2).map(|w| w[0].value != w[1].value).collect())
}

pub fn key_at(t: &RigTrack, time: f32) -> Option<usize> {
    key_times(t).iter().position(|&k| (k - time).abs() < MIN_GAP)
}

/// Rows of key times, one per track (what the shared widgets draw and pick).
pub fn rows(clip: &RigClip) -> Vec<Vec<f32>> {
    clip.tracks.iter().map(key_times).collect()
}

/// Time of the last key of any track (0 without keys).
pub fn last_key_time(clip: &RigClip) -> f32 {
    clip.tracks.iter().map(RigTrack::end_time).fold(0.0, f32::max)
}

/// Adds a key at `t` holding the value the track has there (the curve is
/// unchanged). An existing key is reported as is.
pub fn add_key_at<'a>(t: &mut RigTrack, time: f32, resolve: impl Fn(&str) -> Option<&'a Curve>) -> Option<usize> {
    let time = round_time(time);
    if let Some(i) = key_at(t, time) {
        return Some(i);
    }
    each!(t, tr => {
        let v = tr.sample_with(time, resolve)?;
        Some(put_in_track(tr, time, v, None))
    })
}

/// A key's value as the inspector edits it.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyValue {
    Angle(f32),
    Vec2(f32, f32),
    Color(Rgba),
    Attachment(Option<AttachmentId>),
    /// Read only: the number of vertex offsets.
    Deform(usize),
    /// Read only: the slot order, first = behind.
    Order(Vec<SlotId>),
}

pub fn key_value(t: &RigTrack, i: usize) -> Option<KeyValue> {
    Some(match t {
        RigTrack::BoneRotate { track, .. } => KeyValue::Angle(track.keys.get(i)?.value.0),
        RigTrack::BoneTranslate { track, .. } | RigTrack::BoneScale { track, .. } => {
            let v = track.keys.get(i)?.value;
            KeyValue::Vec2(v.x, v.y)
        }
        RigTrack::SlotAttachment { track, .. } => KeyValue::Attachment(track.keys.get(i)?.value.0.clone()),
        RigTrack::SlotColor { track, .. } => KeyValue::Color(track.keys.get(i)?.value),
        RigTrack::Deform { track, .. } => KeyValue::Deform(track.keys.get(i)?.value.0.len()),
        RigTrack::DrawOrder { track } => KeyValue::Order(track.keys.get(i)?.value.0.clone()),
    })
}

/// Sets the value of key `i`; a value of the wrong kind (or a read only one)
/// is ignored.
pub fn set_key_value(t: &mut RigTrack, i: usize, v: KeyValue) -> bool {
    match (t, v) {
        (RigTrack::BoneRotate { track, .. }, KeyValue::Angle(a)) => track.keys.get_mut(i).map(|k| k.value = Angle(a)).is_some(),
        (RigTrack::BoneTranslate { track, .. } | RigTrack::BoneScale { track, .. }, KeyValue::Vec2(x, y)) => {
            track.keys.get_mut(i).map(|k| k.value = Vec2::new(x, y)).is_some()
        }
        (RigTrack::SlotAttachment { track, .. }, KeyValue::Attachment(a)) => {
            track.keys.get_mut(i).map(|k| k.value = Step(a)).is_some()
        }
        (RigTrack::SlotColor { track, .. }, KeyValue::Color(c)) => track.keys.get_mut(i).map(|k| k.value = c).is_some(),
        _ => false,
    }
}

/// Moves the selected keys by `dt` seconds (clamped so none goes before 0).
/// A selected key landing on an unselected one replaces it. Returns the new
/// selection.
pub fn move_keys(clip: &mut RigClip, sel: &BTreeSet<KeyRef>, dt: f32) -> BTreeSet<KeyRef> {
    let min_t = sel
        .iter()
        .filter_map(|&(ti, ki)| key_times(clip.tracks.get(ti)?).get(ki).copied())
        .fold(f32::INFINITY, f32::min);
    let dt = if min_t.is_finite() { dt.max(-min_t) } else { 0.0 };
    let mut out = BTreeSet::new();
    for (ti, tr) in clip.tracks.iter_mut().enumerate() {
        let picked: BTreeSet<usize> = sel.iter().filter(|&&(t, _)| t == ti).map(|&(_, k)| k).collect();
        if picked.is_empty() {
            continue;
        }
        for k in each!(tr, t => move_in_track(t, &picked, dt)) {
            out.insert((ti, k));
        }
    }
    out
}

/// Deletes the selected keys (a track left empty is kept: it still shows
/// where the bone or slot is animated). Returns how many were removed.
pub fn delete_keys(clip: &mut RigClip, sel: &BTreeSet<KeyRef>) -> usize {
    let mut n = 0;
    for &(ti, ki) in sel.iter().rev() {
        if let Some(tr) = clip.tracks.get_mut(ti)
            && ki < key_count(tr)
        {
            remove_key(tr, ki);
            n += 1;
        }
    }
    n
}

// -------------------------------------------------------------- recording

fn rot_ref(t: &RigTrack) -> Option<&Track<Angle>> {
    if let RigTrack::BoneRotate { track, .. } = t { Some(track) } else { None }
}
fn rot_mut(t: &mut RigTrack) -> Option<&mut Track<Angle>> {
    if let RigTrack::BoneRotate { track, .. } = t { Some(track) } else { None }
}
fn vec_ref(t: &RigTrack) -> Option<&Track<Vec2>> {
    match t {
        RigTrack::BoneTranslate { track, .. } | RigTrack::BoneScale { track, .. } => Some(track),
        _ => None,
    }
}
fn vec_mut(t: &mut RigTrack) -> Option<&mut Track<Vec2>> {
    match t {
        RigTrack::BoneTranslate { track, .. } | RigTrack::BoneScale { track, .. } => Some(track),
        _ => None,
    }
}
fn att_ref(t: &RigTrack) -> Option<&Track<Step<Option<AttachmentId>>>> {
    if let RigTrack::SlotAttachment { track, .. } = t { Some(track) } else { None }
}
fn att_mut(t: &mut RigTrack) -> Option<&mut Track<Step<Option<AttachmentId>>>> {
    if let RigTrack::SlotAttachment { track, .. } = t { Some(track) } else { None }
}
fn col_ref(t: &RigTrack) -> Option<&Track<Rgba>> {
    if let RigTrack::SlotColor { track, .. } = t { Some(track) } else { None }
}
fn col_mut(t: &mut RigTrack) -> Option<&mut Track<Rgba>> {
    if let RigTrack::SlotColor { track, .. } = t { Some(track) } else { None }
}
fn def_ref(t: &RigTrack) -> Option<&Track<Deform>> {
    if let RigTrack::Deform { track, .. } = t { Some(track) } else { None }
}
fn def_mut(t: &mut RigTrack) -> Option<&mut Track<Deform>> {
    if let RigTrack::Deform { track, .. } = t { Some(track) } else { None }
}
fn ord_ref(t: &RigTrack) -> Option<&Track<Step<Vec<SlotId>>>> {
    if let RigTrack::DrawOrder { track } = t { Some(track) } else { None }
}
fn ord_mut(t: &mut RigTrack) -> Option<&mut Track<Step<Vec<SlotId>>>> {
    if let RigTrack::DrawOrder { track } = t { Some(track) } else { None }
}

/// The shared core of every recorder: puts `want` at `t` unless the track
/// already gives that value there (no track is created for a no-op). A key
/// already at `t` is always updated. Returns whether the clip changed.
#[allow(clippy::too_many_arguments)]
fn record<'a, T: Lerp + Clone>(
    clip: &mut RigClip,
    kind: (Channel, Target),
    get: fn(&RigTrack) -> Option<&Track<T>>,
    get_mut: fn(&mut RigTrack) -> Option<&mut Track<T>>,
    base: T,
    want: T,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
    same: impl Fn(&T, &T) -> bool,
) -> bool {
    let Some(blank) = empty_track(kind.0, &kind.1) else { return false };
    let t = round_time(t);
    let idx = find_track(clip, kind.0, &kind.1);
    let existing = idx.and_then(|i| get(&clip.tracks[i]));
    let on_key = existing.and_then(|tr| tr.keys.iter().find(|k| (k.t - t).abs() < MIN_GAP));
    match on_key {
        Some(k) if same(&k.value, &want) => return false,
        Some(_) => {}
        None => {
            let cur = existing.and_then(|tr| tr.sample_with(t, resolve)).unwrap_or(base);
            if same(&cur, &want) {
                return false;
            }
        }
    }
    let i = idx.unwrap_or_else(|| {
        clip.tracks.push(blank);
        clip.tracks.len() - 1
    });
    let Some(tr) = get_mut(&mut clip.tracks[i]) else { return false };
    put_in_track(tr, t, want, None);
    true
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

/// `a` shifted by multiples of 360 to be as close as possible to `reference`
/// (so a recorded rotation never takes the long way round).
pub fn nearest_angle(reference: f32, a: f32) -> f32 {
    let mut d = (a - reference) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    reference + d
}

/// Records the local transform `local` of `bone` at `t` (clip values are
/// relative to the rest pose: rotation and translation added, scale
/// multiplying). Only the channels that differ from what the clip already
/// gives are keyed, so posing a rotation does not key translation or scale.
pub fn record_bone<'a>(
    clip: &mut RigClip,
    bone: &Bone,
    local: &Xform,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve> + Copy,
) -> bool {
    let rest = bone.rest;
    let id = bone.id.clone();
    let target = Target::Bone(id);
    let t = round_time(t);
    let mut changed = false;

    let cur_rot = find_track(clip, Channel::Rotate, &target)
        .and_then(|i| rot_ref(&clip.tracks[i]))
        .and_then(|tr| tr.sample_with(t, resolve))
        .map_or(0.0, |a| a.0);
    let want_rot = nearest_angle(cur_rot, local.rotation - rest.rotation);
    changed |= record(
        clip,
        (Channel::Rotate, target.clone()),
        rot_ref,
        rot_mut,
        Angle(0.0),
        Angle(want_rot),
        t,
        resolve,
        |a, b| near(a.0, b.0),
    );
    changed |= record(
        clip,
        (Channel::Translate, target.clone()),
        vec_ref,
        vec_mut,
        Vec2::new(0.0, 0.0),
        Vec2::new(local.x - rest.x, local.y - rest.y),
        t,
        resolve,
        |a, b| near(a.x, b.x) && near(a.y, b.y),
    );
    let ratio = |v: f32, r: f32| if r.abs() > 1e-6 { v / r } else { v };
    changed |= record(
        clip,
        (Channel::Scale, target),
        vec_ref,
        vec_mut,
        Vec2::new(1.0, 1.0),
        Vec2::new(ratio(local.sx, rest.sx), ratio(local.sy, rest.sy)),
        t,
        resolve,
        |a, b| near(a.x, b.x) && near(a.y, b.y),
    );
    changed
}

pub fn record_attachment<'a>(
    clip: &mut RigClip,
    rig: &Rig,
    slot: &SlotId,
    attachment: Option<AttachmentId>,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
) -> bool {
    let base = rig.slots.iter().find(|s| &s.id == slot).and_then(|s| s.attachment.clone());
    record(
        clip,
        (Channel::Attachment, Target::Slot(slot.clone())),
        att_ref,
        att_mut,
        Step(base),
        Step(attachment),
        t,
        resolve,
        |a, b| a == b,
    )
}

pub fn record_color<'a>(
    clip: &mut RigClip,
    rig: &Rig,
    slot: &SlotId,
    color: Rgba,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
) -> bool {
    let base = rig.slots.iter().find(|s| &s.id == slot).map_or(Rgba::new(1.0, 1.0, 1.0, 1.0), |s| s.color);
    record(
        clip,
        (Channel::Color, Target::Slot(slot.clone())),
        col_ref,
        col_mut,
        base,
        color,
        t,
        resolve,
        |a, b| near(a.r, b.r) && near(a.g, b.g) && near(a.b, b.b) && near(a.a, b.a),
    )
}

/// Records vertex offsets (bind space) of a mesh attachment.
pub fn record_deform<'a>(
    clip: &mut RigClip,
    slot: &SlotId,
    attachment: &AttachmentId,
    offsets: Vec<[f32; 2]>,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
) -> bool {
    let n = offsets.len();
    record(
        clip,
        (Channel::Deform, Target::Deform(slot.clone(), attachment.clone())),
        def_ref,
        def_mut,
        Deform(vec![[0.0, 0.0]; n]),
        Deform(offsets),
        t,
        resolve,
        |a, b| {
            let n = a.0.len().max(b.0.len());
            let get = |d: &Deform, i: usize| d.0.get(i).copied().unwrap_or([0.0, 0.0]);
            (0..n).all(|i| near(get(a, i)[0], get(b, i)[0]) && near(get(a, i)[1], get(b, i)[1]))
        },
    )
}

pub fn record_draw_order<'a>(
    clip: &mut RigClip,
    rig: &Rig,
    order: Vec<SlotId>,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
) -> bool {
    let base: Vec<SlotId> = rig.slots.iter().map(|s| s.id.clone()).collect();
    record(
        clip,
        (Channel::DrawOrder, Target::DrawOrder),
        ord_ref,
        ord_mut,
        Step(base),
        Step(order),
        t,
        resolve,
        |a, b| a == b,
    )
}

/// The value a track gives at `time` (named curves through `resolve`).
pub fn sample_track<'a>(t: &RigTrack, time: f32, resolve: impl Fn(&str) -> Option<&'a Curve>) -> Option<KeyValue> {
    Some(match t {
        RigTrack::BoneRotate { track, .. } => KeyValue::Angle(track.sample_with(time, resolve)?.0),
        RigTrack::BoneTranslate { track, .. } | RigTrack::BoneScale { track, .. } => {
            let v = track.sample_with(time, resolve)?;
            KeyValue::Vec2(v.x, v.y)
        }
        RigTrack::SlotAttachment { track, .. } => KeyValue::Attachment(track.sample_with(time, resolve)?.0),
        RigTrack::SlotColor { track, .. } => KeyValue::Color(track.sample_with(time, resolve)?),
        RigTrack::Deform { track, .. } => KeyValue::Deform(track.sample_with(time, resolve)?.0.len()),
        RigTrack::DrawOrder { track } => KeyValue::Order(track.sample_with(time, resolve)?.0),
    })
}

/// The offsets of a deform track at `time`, padded with zeros to `n` vertices.
pub fn sample_deform<'a>(
    clip: &RigClip,
    slot: &SlotId,
    attachment: &AttachmentId,
    time: f32,
    n: usize,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
) -> Vec<[f32; 2]> {
    let mut out = find_track(clip, Channel::Deform, &Target::Deform(slot.clone(), attachment.clone()))
        .and_then(|i| def_ref(&clip.tracks[i]))
        .and_then(|tr| tr.sample_with(time, resolve))
        .map(|d| d.0)
        .unwrap_or_default();
    out.resize(n, [0.0, 0.0]);
    out
}

/// Records a value edited in the inspector at `t` (rotation and translation
/// relative to the rest pose, scale as a ratio). Deform offsets go through
/// [`record_deform`].
pub fn record_value<'a>(
    clip: &mut RigClip,
    rig: &Rig,
    channel: Channel,
    target: &Target,
    value: &KeyValue,
    t: f32,
    resolve: impl Fn(&str) -> Option<&'a Curve>,
) -> bool {
    let kind = (channel, target.clone());
    match (channel, target, value) {
        (Channel::Rotate, Target::Bone(_), KeyValue::Angle(a)) => {
            record(clip, kind, rot_ref, rot_mut, Angle(0.0), Angle(*a), t, resolve, |x, y| near(x.0, y.0))
        }
        (Channel::Translate, Target::Bone(_), KeyValue::Vec2(x, y)) => {
            record(clip, kind, vec_ref, vec_mut, Vec2::new(0.0, 0.0), Vec2::new(*x, *y), t, resolve, |a, b| {
                near(a.x, b.x) && near(a.y, b.y)
            })
        }
        (Channel::Scale, Target::Bone(_), KeyValue::Vec2(x, y)) => {
            record(clip, kind, vec_ref, vec_mut, Vec2::new(1.0, 1.0), Vec2::new(*x, *y), t, resolve, |a, b| {
                near(a.x, b.x) && near(a.y, b.y)
            })
        }
        (Channel::Attachment, Target::Slot(s), KeyValue::Attachment(a)) => record_attachment(clip, rig, s, a.clone(), t, resolve),
        (Channel::Color, Target::Slot(s), KeyValue::Color(c)) => record_color(clip, rig, s, *c, t, resolve),
        (Channel::DrawOrder, Target::DrawOrder, KeyValue::Order(o)) => record_draw_order(clip, rig, o.clone(), t, resolve),
        _ => false,
    }
}

// ------------------------------------------------------------- cross-fade

/// A cross-fade preview state: `from` plays alone until clock `hold`, then
/// `to` fades in over `blend` seconds. Clock 0 starts `from`.
pub fn crossfade(from: &str, to: &str, blend: f32, hold: f32) -> ClipBlend {
    let mut b = ClipBlend::new(from, 0.0);
    b.blend = blend.max(0.0);
    b.switch(to, hold.max(0.0));
    b
}

/// Clock length of a cross-fade preview: the hold, the fade and a short tail
/// of the destination clip.
pub fn crossfade_length(hold: f32, blend: f32) -> f32 {
    hold.max(0.0) + blend.max(0.0) + 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_anim::rig::{PreparedRig, Skin, Slot};

    fn no_curves(_: &str) -> Option<&'static Curve> {
        None
    }

    fn bone(id: &str, parent: Option<&str>, rest: Xform) -> Bone {
        Bone { id: BoneId::new(id), name: id.to_owned(), parent: parent.map(BoneId::new), rest, length: 50.0 }
    }

    fn rig() -> Rig {
        let mut r = Rig::default();
        r.bones.push(bone("root", None, Xform::IDENTITY));
        r.bones.push(bone("arm", Some("root"), Xform { x: 10.0, y: 0.0, rotation: 30.0, sx: 2.0, sy: 1.0 }));
        r.slots.push(Slot { id: SlotId::new("s1"), name: "s1".into(), bone: BoneId::new("arm"), attachment: None, color: Rgba::new(1.0, 1.0, 1.0, 1.0) });
        r.slots.push(Slot { id: SlotId::new("s2"), name: "s2".into(), bone: BoneId::new("root"), attachment: None, color: Rgba::new(1.0, 1.0, 1.0, 1.0) });
        r.skins.push(Skin { name: "default".into(), attachments: Default::default() });
        r
    }

    #[test]
    fn clip_list_operations_and_names() {
        let mut r = rig();
        let a = add_clip(&mut r, "walk").unwrap();
        assert_eq!(a, "walk");
        assert_eq!(add_clip(&mut r, "walk").unwrap(), "walk2");
        assert_eq!(add_clip(&mut r, "  ").unwrap(), "clip");
        assert_eq!(r.clips["walk"].duration, DEFAULT_DURATION);
        assert_eq!(rename_clip(&mut r, "walk", "walk2"), Err(OpError::Exists));
        assert_eq!(rename_clip(&mut r, "walk", " "), Err(OpError::EmptyName));
        assert_eq!(rename_clip(&mut r, "nope", "x"), Err(OpError::Unknown));
        rename_clip(&mut r, "walk", "run").unwrap();
        assert!(r.clips.contains_key("run") && !r.clips.contains_key("walk"));
        set_duration(&mut r, "run", 2.5).unwrap();
        set_loop(&mut r, "run", Loop::PingPong).unwrap();
        let copy = duplicate_clip(&mut r, "run").unwrap();
        assert_eq!(copy, "run copy");
        assert_eq!(r.clips[&copy], r.clips["run"]);
        set_duration(&mut r, "run", f32::NAN).unwrap();
        assert_eq!(r.clips["run"].duration, 0.0);
        set_duration(&mut r, "run", -3.0).unwrap();
        assert_eq!(r.clips["run"].duration, 0.0);
        delete_clip(&mut r, "run").unwrap();
        assert_eq!(delete_clip(&mut r, "run"), Err(OpError::Unknown));
    }

    #[test]
    fn recording_a_bone_keys_only_the_changed_channels_relative_to_rest() {
        let r = rig();
        let arm = r.bones[1].clone();
        let mut clip = RigClip::new(1.0, Loop::Once);
        // Rotated by 20 degrees from rest, moved by (5, -2), scale untouched.
        let local = Xform { x: 15.0, y: -2.0, rotation: 50.0, sx: 2.0, sy: 1.0 };
        assert!(record_bone(&mut clip, &arm, &local, 0.5, no_curves));
        assert_eq!(clip.tracks.len(), 2, "scale is unchanged: no scale track");
        let rot = find_track(&clip, Channel::Rotate, &Target::Bone(arm.id.clone())).unwrap();
        assert_eq!(key_value(&clip.tracks[rot], 0), Some(KeyValue::Angle(20.0)));
        let tr = find_track(&clip, Channel::Translate, &Target::Bone(arm.id.clone())).unwrap();
        assert_eq!(key_value(&clip.tracks[tr], 0), Some(KeyValue::Vec2(5.0, -2.0)));
        // The prepared rig reads it back as the same local transform.
        let mut full = r.clone();
        full.clips.insert("c".into(), clip.clone());
        let p = PreparedRig::new(full).unwrap();
        let pose = p.sample_clip_local_with(&clip, 0.5, no_curves);
        assert!((pose.bones[1].rotation - 50.0).abs() < 1e-3);
        assert!((pose.bones[1].x - 15.0).abs() < 1e-3 && (pose.bones[1].sx - 2.0).abs() < 1e-3);
        // Recording the same pose again changes nothing.
        assert!(!record_bone(&mut clip, &arm, &local, 0.5, no_curves));
        // Scale doubles relative to rest (rest sx = 2 -> local 4 -> ratio 2).
        let scaled = Xform { sx: 4.0, ..local };
        assert!(record_bone(&mut clip, &arm, &scaled, 0.5, no_curves));
        let sc = find_track(&clip, Channel::Scale, &Target::Bone(arm.id.clone())).unwrap();
        assert_eq!(key_value(&clip.tracks[sc], 0), Some(KeyValue::Vec2(2.0, 1.0)));
    }

    #[test]
    fn a_pose_equal_to_the_interpolation_adds_no_key_but_a_key_there_is_updated() {
        let r = rig();
        let arm = r.bones[1].clone();
        let mut clip = RigClip::new(1.0, Loop::Once);
        let at = |rot: f32| Xform { rotation: 30.0 + rot, ..arm.rest };
        record_bone(&mut clip, &arm, &at(0.0), 0.0, no_curves);
        assert!(clip.tracks.is_empty(), "rest pose over an empty clip is a no-op");
        record_bone(&mut clip, &arm, &at(40.0), 1.0, no_curves);
        // Keys exist at 1.0 only (0.0 held at the first key): the halfway
        // value equals the held value, so nothing is added.
        assert!(!record_bone(&mut clip, &arm, &at(40.0), 0.5, no_curves));
        assert!(record_bone(&mut clip, &arm, &at(10.0), 0.0, no_curves));
        let rot = find_track(&clip, Channel::Rotate, &Target::Bone(arm.id.clone())).unwrap();
        assert_eq!(key_times(&clip.tracks[rot]), vec![0.0, 1.0]);
        // Halfway is now 25, so posing 25 adds nothing; 30 adds a key.
        assert!(!record_bone(&mut clip, &arm, &at(25.0), 0.5, no_curves));
        assert!(record_bone(&mut clip, &arm, &at(30.0), 0.5, no_curves));
        assert_eq!(key_times(&clip.tracks[rot]), vec![0.0, 0.5, 1.0]);
        // Posing on an existing key rewrites it, keeping its easing.
        set_key_ease(&mut clip.tracks[rot], 1, Curve::Preset(orbit_anim::Ease::InQuad));
        assert!(record_bone(&mut clip, &arm, &at(35.0), 0.5, no_curves));
        assert_eq!(key_value(&clip.tracks[rot], 1), Some(KeyValue::Angle(35.0)));
        assert_eq!(key_ease(&clip.tracks[rot], 1), Some(Curve::Preset(orbit_anim::Ease::InQuad)));
    }

    #[test]
    fn recorded_rotation_takes_the_short_way() {
        assert!((nearest_angle(170.0, -170.0) - 190.0).abs() < 1e-4);
        assert!((nearest_angle(0.0, 350.0) + 10.0).abs() < 1e-4);
        assert!((nearest_angle(720.0, 10.0) - 730.0).abs() < 1e-4);
    }

    #[test]
    fn slot_and_draw_order_recording() {
        let r = rig();
        let s1 = SlotId::new("s1");
        let mut clip = RigClip::new(1.0, Loop::Once);
        let blue = Rgba::new(0.0, 0.0, 1.0, 1.0);
        assert!(!record_color(&mut clip, &r, &s1, Rgba::new(1.0, 1.0, 1.0, 1.0), 0.0, no_curves));
        assert!(record_color(&mut clip, &r, &s1, blue, 0.5, no_curves));
        assert!(record_attachment(&mut clip, &r, &s1, Some(AttachmentId::new("hat")), 0.25, no_curves));
        assert!(!record_attachment(&mut clip, &r, &s1, Some(AttachmentId::new("hat")), 0.0, no_curves), "held before the first key");
        assert!(record_draw_order(&mut clip, &r, vec![SlotId::new("s2"), s1.clone()], 0.75, no_curves));
        assert!(!record_draw_order(&mut clip, &r, vec![SlotId::new("s2"), s1.clone()], 0.0, no_curves));
        assert!(record_deform(&mut clip, &s1, &AttachmentId::new("m"), vec![[1.0, 0.0], [0.0, 0.0]], 0.5, no_curves));
        assert!(!record_deform(&mut clip, &s1, &AttachmentId::new("m"), vec![[1.0, 0.0], [0.0, 0.0]], 0.0, no_curves));
        assert_eq!(clip.tracks.len(), 4);
        let mut full = r.clone();
        full.clips.insert("c".into(), clip.clone());
        let p = PreparedRig::new(full).unwrap();
        let pose = p.sample_clip_local_with(&clip, 0.75, no_curves);
        assert_eq!(pose.draw_order, vec![1, 0]);
        assert_eq!(pose.slots[0].attachment, Some(AttachmentId::new("hat")));
        assert_eq!(pose.slots[0].color, blue);
        assert_eq!(pose.deforms.get(&(0, AttachmentId::new("m"))).map(Vec::len), Some(2));
    }

    #[test]
    fn add_track_starts_from_the_setup_pose_and_validates_targets() {
        let mut r = rig();
        add_clip(&mut r, "c").unwrap();
        let arm = Target::Bone(BoneId::new("arm"));
        let i = add_track(&mut r, "c", Channel::Scale, &arm, 0.2).unwrap();
        assert_eq!(key_value(&r.clips["c"].tracks[i], 0), Some(KeyValue::Vec2(1.0, 1.0)));
        assert_eq!(key_times(&r.clips["c"].tracks[i]), vec![0.2]);
        assert_eq!(add_track(&mut r, "c", Channel::Scale, &arm, 0.9), Some(i), "no duplicate track");
        assert_eq!(add_track(&mut r, "c", Channel::Rotate, &Target::Bone(BoneId::new("nope")), 0.0), None);
        assert_eq!(add_track(&mut r, "c", Channel::Rotate, &Target::Slot(SlotId::new("s1")), 0.0), None);
        assert_eq!(add_track(&mut r, "c", Channel::Deform, &Target::Deform(SlotId::new("s1"), AttachmentId::new("m")), 0.0), None);
        let d = add_track(&mut r, "c", Channel::DrawOrder, &Target::DrawOrder, 0.0).unwrap();
        assert_eq!(key_value(&r.clips["c"].tracks[d], 0), Some(KeyValue::Order(vec![SlotId::new("s1"), SlotId::new("s2")])));
        r.clips.get_mut("c").unwrap().tracks.remove(d);
        assert!(validate_ok(&r));
    }

    fn validate_ok(r: &Rig) -> bool {
        r.clips.values().all(|c| c.tracks.iter().all(|t| t.validate_keys().is_ok()))
    }

    #[test]
    fn keys_move_delete_and_edit_across_typed_tracks() {
        let mut r = rig();
        add_clip(&mut r, "c").unwrap();
        let arm = Target::Bone(BoneId::new("arm"));
        add_track(&mut r, "c", Channel::Rotate, &arm, 0.0).unwrap();
        add_track(&mut r, "c", Channel::Translate, &arm, 0.0).unwrap();
        let clip = r.clips.get_mut("c").unwrap();
        add_key_at(&mut clip.tracks[0], 1.0, no_curves).unwrap();
        add_key_at(&mut clip.tracks[0], 2.0, no_curves).unwrap();
        // Adding at an existing time reuses the key.
        assert_eq!(add_key_at(&mut clip.tracks[0], 1.0004, no_curves), Some(1));
        assert_eq!(rows(clip), vec![vec![0.0, 1.0, 2.0], vec![0.0]]);
        let sel: BTreeSet<KeyRef> = [(0, 0), (0, 1), (1, 0)].into_iter().collect();
        let moved = move_keys(clip, &sel, -5.0);
        assert_eq!(moved, sel, "clamped at 0: the first key stays put and so does the group");
        let moved = move_keys(clip, &[(0, 0)].into_iter().collect(), 1.5);
        assert_eq!(rows(clip)[0], vec![1.0, 1.5, 2.0]);
        assert_eq!(moved, [(0, 1)].into_iter().collect());
        assert!(set_key_value(&mut clip.tracks[0], 1, KeyValue::Angle(45.0)));
        assert!(!set_key_value(&mut clip.tracks[0], 1, KeyValue::Vec2(1.0, 1.0)));
        assert!(!set_key_value(&mut clip.tracks[0], 9, KeyValue::Angle(1.0)));
        assert_eq!(segments_animated(&clip.tracks[0]), vec![true, true]);
        assert_eq!(last_key_time(clip), 2.0);
        let n = delete_keys(clip, &[(0, 0), (0, 2), (1, 0), (5, 0)].into_iter().collect());
        assert_eq!(n, 3);
        assert_eq!(rows(clip), vec![vec![1.5], vec![]]);
    }

    #[test]
    fn crossfade_state_blends_between_two_clips() {
        let mut r = rig();
        let mut a = RigClip::new(1.0, Loop::Forever);
        record_bone(&mut a, &r.bones[1].clone(), &Xform { rotation: 30.0 + 40.0, ..r.bones[1].rest }, 0.0, no_curves);
        let mut b = RigClip::new(1.0, Loop::Forever);
        record_bone(&mut b, &r.bones[1].clone(), &Xform { rotation: 30.0 - 40.0, ..r.bones[1].rest }, 0.0, no_curves);
        r.clips.insert("a".into(), a);
        r.clips.insert("b".into(), b);
        let p = PreparedRig::new(r).unwrap();
        let state = crossfade("a", "b", 0.4, 1.0);
        let at = |t: f32| p.sample_blend_with(&state, t, no_curves).bones[1].rotation;
        assert!((at(0.5) - 70.0).abs() < 1e-3, "only the first clip before the switch");
        assert!((at(1.2) - 30.0).abs() < 1e-3, "halfway through the fade");
        assert!((at(1.6) - (-10.0)).abs() < 1e-3, "only the second after the fade");
        assert_eq!(crossfade_length(1.0, 0.4), 1.9);
        // Same clip twice: no fade, no panic.
        let same = crossfade("a", "a", 0.4, 1.0);
        assert!(same.previous.is_none());
    }
}
