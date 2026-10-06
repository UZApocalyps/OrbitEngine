//! Rig level operations of the rig editor and the MCP `rig_*` tools:
//! duplicating a rig, copying clips from another rig, recentering the
//! drawing on the rig origin. Pure functions on rigs (no disk, no UI), so the
//! editor can run them off the UI thread; only [`duplicate_files`] touches
//! the disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orbit_anim::rig::{Bounds, origin_offset};

use super::*;

// ------------------------------------------------------------ recenter

/// What [`recenter`] did.
#[derive(Clone, Debug, PartialEq)]
pub struct RecenterReport {
    /// Bounds of the visible drawing before the move.
    pub bounds: Bounds,
    /// What was added to every root bone and mesh vertex.
    pub shift: [f32; 2],
}

/// Moves the drawing so its real bottom-center sits on the rig origin
/// `(0, 0)`: root bones are shifted (their children and the regions follow)
/// and so are the mesh vertices of every skin (they are stored in rig space,
/// in the setup pose). Clips are relative to the setup pose and stay valid.
/// `skin`: the active skin the bounds are measured on (None: the default
/// one); `all_skins`: measure the pictures of every skin.
pub fn recenter(rig: &mut Rig, skin: Option<&str>, all_skins: bool) -> OpResult<RecenterReport> {
    if let Some(name) = skin {
        find_skin(rig, name)?;
    }
    let Some(bounds) = rig.rest_bounds(skin, all_skins) else {
        return refused("no visible picture: nothing to center");
    };
    let ([cx, bottom], _) = origin_offset(&bounds);
    let shift = [-cx, -bottom];
    if shift != [0.0, 0.0] {
        let ids: HashSet<BoneId> = rig.bones.iter().map(|b| b.id.clone()).collect();
        for b in &mut rig.bones {
            if b.parent.as_ref().is_none_or(|p| !ids.contains(p)) {
                b.rest.x += shift[0];
                b.rest.y += shift[1];
            }
        }
        for s in &mut rig.skins {
            for a in s.attachments.values_mut() {
                if let Attachment::Mesh(m) = a {
                    for v in &mut m.vertices {
                        v[0] += shift[0];
                        v[1] += shift[1];
                    }
                }
            }
        }
    }
    Ok(RecenterReport { bounds, shift })
}

// ---------------------------------------------------------- copy clips

/// Options of [`copy_clips`].
#[derive(Clone, Debug, Default)]
pub struct CopyClipsSpec {
    /// Clips to copy (empty: all of them).
    pub clips: Vec<String>,
    /// Match bones and slots by their position in the hierarchy instead of
    /// by name.
    pub by_hierarchy: bool,
    /// Rescale translation keys by the ratio of the matched bones' lengths.
    pub rescale: bool,
}

/// What [`copy_clips`] did and could not do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CopyClipsReport {
    /// `(name in the source, name in this rig)` of each copied clip; a name
    /// already taken gets a number.
    pub copied: Vec<(String, String)>,
    pub tracks_copied: usize,
    pub tracks_dropped: usize,
    /// Source bones / slots animated by the copied clips with no match here.
    pub unmatched_bones: Vec<String>,
    pub unmatched_slots: Vec<String>,
    /// Attachments (`slot/attachment`) a copied key points to that this rig
    /// does not have.
    pub unmatched_attachments: Vec<String>,
    /// Deform tracks dropped because the meshes differ (`slot/attachment`).
    pub dropped_deform: Vec<String>,
}

/// Hierarchy position of each bone: the sibling indices from the root.
fn bone_paths(rig: &Rig) -> Vec<Vec<usize>> {
    let index: BTreeMap<&str, usize> = rig.bones.iter().enumerate().map(|(i, b)| (b.id.as_str(), i)).collect();
    let mut children: BTreeMap<Option<usize>, usize> = BTreeMap::new();
    let mut paths: Vec<Vec<usize>> = Vec::with_capacity(rig.bones.len());
    for b in &rig.bones {
        let parent = b.parent.as_ref().and_then(|p| index.get(p.as_str()).copied()).filter(|&p| p < paths.len());
        let n = children.entry(parent).or_insert(0);
        let mut path = parent.map(|p| paths[p].clone()).unwrap_or_default();
        path.push(*n);
        *n += 1;
        paths.push(path);
    }
    paths
}

/// For each bone of `src`, the matching bone of `dest`.
pub fn match_bones(src: &Rig, dest: &Rig, by_hierarchy: bool) -> Vec<Option<usize>> {
    if by_hierarchy {
        let (sp, dp) = (bone_paths(src), bone_paths(dest));
        sp.iter().map(|p| dp.iter().position(|q| q == p)).collect()
    } else {
        src.bones.iter().map(|b| dest.bones.iter().position(|d| d.name == b.name)).collect()
    }
}

/// For each slot of `src`, the matching slot of `dest`: by name, or (by
/// hierarchy) the same rank on the matching bone.
pub fn match_slots(src: &Rig, dest: &Rig, bones: &[Option<usize>], by_hierarchy: bool) -> Vec<Option<usize>> {
    if !by_hierarchy {
        return src.slots.iter().map(|s| dest.slots.iter().position(|d| d.name == s.name)).collect();
    }
    let src_bone = |id: &BoneId| src.bones.iter().position(|b| &b.id == id);
    src.slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let rank = src.slots[..i].iter().filter(|o| o.bone == s.bone).count();
            let db = bones.get(src_bone(&s.bone)?).copied().flatten()?;
            dest.slots.iter().enumerate().filter(|(_, d)| d.bone == dest.bones[db].id).nth(rank).map(|(j, _)| j)
        })
        .collect()
}

fn image_stem(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}

fn find_attachment<'a>(rig: &'a Rig, id: &AttachmentId) -> Option<&'a Attachment> {
    rig.skins.iter().find_map(|s| s.attachments.get(id))
}

/// The attachment of `dest` standing for `id` of `src`: the same id, else a
/// picture with the same file name.
fn match_attachment(src: &Rig, dest: &Rig, id: &AttachmentId) -> Option<AttachmentId> {
    if find_attachment(dest, id).is_some() {
        return Some(id.clone());
    }
    let stem = image_stem(find_attachment(src, id)?.image());
    dest.skins
        .iter()
        .flat_map(|s| s.attachments.iter())
        .find(|(_, a)| image_stem(a.image()) == stem)
        .map(|(k, _)| k.clone())
}

fn scale_translate(t: &Track<Vec2>, k: f32) -> Track<Vec2> {
    let mut out = t.clone();
    for key in &mut out.keys {
        key.value = Vec2 { x: key.value.x * k, y: key.value.y * k };
    }
    out
}

/// Copies clips of `src` into `dest`, mapping tracks to bones, slots and
/// attachments of `dest` (see [`CopyClipsSpec`]). Translation keys are
/// rescaled by the ratio of the matched bones' lengths when asked; deform
/// keys are kept only when the two meshes have the same vertex count.
/// Whatever has no match is dropped and listed in the report. Refused when
/// a named clip does not exist in `src`.
pub fn copy_clips(dest: &mut Rig, src: &Rig, spec: &CopyClipsSpec) -> OpResult<CopyClipsReport> {
    let names: Vec<String> = if spec.clips.is_empty() { src.clips.keys().cloned().collect() } else { spec.clips.clone() };
    for n in &names {
        if !src.clips.contains_key(n) {
            return not_found(format!("the source rig has no clip `{n}`"));
        }
    }
    if names.is_empty() {
        return refused("the source rig has no clip");
    }
    let bones = match_bones(src, dest, spec.by_hierarchy);
    let slots = match_slots(src, dest, &bones, spec.by_hierarchy);
    let bone_of = |id: &BoneId| src.bones.iter().position(|b| &b.id == id);
    let slot_of = |id: &SlotId| src.slots.iter().position(|s| &s.id == id);
    let mut report = CopyClipsReport::default();
    let (mut ub, mut us, mut ua, mut ud) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    let mut new_clips: Vec<(String, RigClip)> = Vec::new();
    for name in &names {
        let clip = &src.clips[name];
        let mut out = RigClip::new(clip.duration, clip.looping);
        for track in &clip.tracks {
            // Bone tracks: the matched bone (and the length ratio).
            let mapped_bone = |id: &BoneId, ub: &mut BTreeSet<String>| -> Option<(BoneId, f32)> {
                let si = bone_of(id)?;
                match bones[si] {
                    Some(di) => {
                        let (a, b) = (src.bones[si].length, dest.bones[di].length);
                        let k = if spec.rescale && a > 1e-6 && b > 1e-6 { b / a } else { 1.0 };
                        Some((dest.bones[di].id.clone(), k))
                    }
                    None => {
                        ub.insert(src.bones[si].name.clone());
                        None
                    }
                }
            };
            let mapped_slot = |id: &SlotId, us: &mut BTreeSet<String>| -> Option<(usize, usize)> {
                let si = slot_of(id)?;
                match slots[si] {
                    Some(di) => Some((si, di)),
                    None => {
                        us.insert(src.slots[si].name.clone());
                        None
                    }
                }
            };
            let new = match track {
                RigTrack::BoneRotate { bone, track } => {
                    mapped_bone(bone, &mut ub).map(|(b, _)| RigTrack::BoneRotate { bone: b, track: track.clone() })
                }
                RigTrack::BoneScale { bone, track } => {
                    mapped_bone(bone, &mut ub).map(|(b, _)| RigTrack::BoneScale { bone: b, track: track.clone() })
                }
                RigTrack::BoneTranslate { bone, track } => mapped_bone(bone, &mut ub)
                    .map(|(b, k)| RigTrack::BoneTranslate { bone: b, track: scale_translate(track, k) }),
                RigTrack::SlotColor { slot, track } => {
                    mapped_slot(slot, &mut us).map(|(_, d)| RigTrack::SlotColor { slot: dest.slots[d].id.clone(), track: track.clone() })
                }
                RigTrack::SlotAttachment { slot, track } => mapped_slot(slot, &mut us).and_then(|(s, d)| {
                    let mut keys = Vec::new();
                    for key in &track.keys {
                        let value = match &key.value.0 {
                            None => None,
                            Some(a) => match match_attachment(src, dest, a) {
                                Some(m) => Some(m),
                                None => {
                                    ua.insert(format!("{}/{}", src.slots[s].name, a));
                                    continue;
                                }
                            },
                        };
                        let mut k = key.clone();
                        k.value = Step(value);
                        keys.push(k);
                    }
                    (!keys.is_empty()).then(|| RigTrack::SlotAttachment {
                        slot: dest.slots[d].id.clone(),
                        track: Track { keys },
                    })
                }),
                RigTrack::Deform { slot, attachment, track } => mapped_slot(slot, &mut us).and_then(|(s, d)| {
                    let label = format!("{}/{}", src.slots[s].name, attachment);
                    let Some(da) = match_attachment(src, dest, attachment) else {
                        ua.insert(label);
                        return None;
                    };
                    let count = |rig: &Rig, id: &AttachmentId| match find_attachment(rig, id) {
                        Some(Attachment::Mesh(m)) => Some(m.vertices.len()),
                        _ => None,
                    };
                    if count(src, attachment).is_none() || count(src, attachment) != count(dest, &da) {
                        ud.insert(label);
                        return None;
                    }
                    Some(RigTrack::Deform { slot: dest.slots[d].id.clone(), attachment: da, track: track.clone() })
                }),
                RigTrack::DrawOrder { track } => {
                    let mut keys = Vec::new();
                    for key in &track.keys {
                        let order: Vec<SlotId> = key
                            .value
                            .0
                            .iter()
                            .filter_map(|id| mapped_slot(id, &mut us).map(|(_, d)| dest.slots[d].id.clone()))
                            .collect();
                        let mut k = key.clone();
                        k.value = Step(order);
                        keys.push(k);
                    }
                    Some(RigTrack::DrawOrder { track: Track { keys } })
                }
            };
            match new {
                Some(t) => {
                    out.tracks.push(t);
                    report.tracks_copied += 1;
                }
                None => report.tracks_dropped += 1,
            }
        }
        new_clips.push((name.clone(), out));
    }
    for (src_name, clip) in new_clips {
        let taken: Vec<String> = dest.clips.keys().cloned().collect();
        let final_name = unique_name(&src_name, taken.iter().map(String::as_str));
        dest.clips.insert(final_name.clone(), clip);
        report.copied.push((src_name, final_name));
    }
    report.unmatched_bones = ub.into_iter().collect();
    report.unmatched_slots = us.into_iter().collect();
    report.unmatched_attachments = ua.into_iter().collect();
    report.dropped_deform = ud.into_iter().collect();
    Ok(report)
}

// ----------------------------------------------------------- duplicate

/// `name` as a file name stem (letters, digits, `-`, `_`).
pub fn file_stem_of(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    s.trim_matches('_').to_owned()
}

/// The copy of `rig` that points to its own pictures: every picture path is
/// replaced through `rename`. Returns the distinct `(old, new)` pairs.
pub fn retarget_pictures(rig: &mut Rig, mut rename: impl FnMut(&str) -> String) -> Vec<(String, String)> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for s in &mut rig.skins {
        for a in s.attachments.values_mut() {
            let image = match a {
                Attachment::Mesh(m) => &mut m.image,
                Attachment::Region(r) => &mut r.image,
            };
            let new = map.entry(image.clone()).or_insert_with(|| rename(image)).clone();
            *image = new;
        }
    }
    map.into_iter().collect()
}

/// What [`duplicate_files`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicated {
    /// Path of the new rig (relative to the assets root).
    pub rig: String,
    /// New pictures `(old, new)` (empty when the pictures are shared).
    pub pictures: Vec<(String, String)>,
}

/// Writes a copy of `rig` as `<RIG_DIR>/<name>.rig.ron` under `root`
/// (refused when the file exists). With `copy_pictures`, every picture is
/// copied into `images/<name>/` and the new rig points to the copies; else
/// both rigs share the pictures. Nothing is left behind when it fails.
pub fn duplicate_files(root: &Path, rig: &Rig, name: &str, copy_pictures: bool) -> OpResult<Duplicated> {
    let stem = file_stem_of(name);
    if stem.is_empty() {
        return invalid("the new rig needs a name");
    }
    let rel = format!("{}/{stem}.rig.ron", crate::rigs::RIG_DIR);
    if root.join(&rel).exists() {
        return refused(format!("`{rel}` already exists: pick another name"));
    }
    let mut copy = rig.clone();
    let mut pictures = Vec::new();
    let mut written: Vec<std::path::PathBuf> = Vec::new();
    let io = |e: std::io::Error| OpError { kind: OpErrorKind::Refused, message: e.to_string() };
    let result = (|| -> OpResult<()> {
        if copy_pictures {
            let mut used: BTreeSet<String> = BTreeSet::new();
            let mut failure: Option<OpError> = None;
            pictures = retarget_pictures(&mut copy, |old| {
                let file = old.rsplit('/').next().unwrap_or(old);
                let (base, ext) = file.rsplit_once('.').map_or((file, ""), |(b, e)| (b, e));
                let mut n = 1;
                let new = loop {
                    let f = match (n, ext.is_empty()) {
                        (1, true) => base.to_owned(),
                        (1, false) => format!("{base}.{ext}"),
                        (_, true) => format!("{base}_{n}"),
                        (_, false) => format!("{base}_{n}.{ext}"),
                    };
                    let p = format!("images/{stem}/{f}");
                    if !used.contains(&p) && !root.join(&p).exists() {
                        break p;
                    }
                    n += 1;
                };
                used.insert(new.clone());
                let (from, to) = (root.join(old), root.join(&new));
                let done = to.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::copy(&from, &to).map(|_| ()));
                match done {
                    Ok(()) => written.push(to),
                    Err(e) if failure.is_none() => {
                        failure = Some(OpError { kind: OpErrorKind::NotFound, message: format!("cannot copy picture `{old}`: {e}") });
                    }
                    Err(_) => {}
                }
                new
            });
            if let Some(e) = failure {
                return Err(e);
            }
        }
        let text = orbit_anim::rig::save_rig(&copy).map_err(|e| OpError { kind: OpErrorKind::Invalid, message: e })?;
        let abs = root.join(&rel);
        if let Some(dir) = abs.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        std::fs::write(&abs, text).map_err(io)?;
        Ok(())
    })();
    if let Err(e) = result {
        for p in written {
            let _ = std::fs::remove_file(p);
        }
        return Err(e);
    }
    Ok(Duplicated { rig: rel, pictures })
}

#[cfg(test)]
#[path = "rigops_copy_tests.rs"]
mod tests;
