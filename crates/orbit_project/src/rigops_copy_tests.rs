//! Tests of duplicate, copy clips and recenter.

use super::*;
use orbit_anim::rig::{MeshAttachment, RegionAttachment};
use orbit_anim::rig::{RigIssueKind, Severity};

fn bone(rig: &mut Rig, parent: Option<&str>, name: &str, x: f32, length: f32) -> BoneId {
    let spec = BoneSpec { name: Some(name.into()), x: Some(x), length: Some(length), ..BoneSpec::default() };
    bone_add(rig, parent, &spec).unwrap()
}

fn region(rig: &mut Rig, slot: &str, bone: &BoneId, image: &str, size: [f32; 2], at: [f32; 2]) -> (SlotId, AttachmentId) {
    let sid = SlotId::new(format!("s_{slot}"));
    let aid = AttachmentId::new(format!("a_{slot}"));
    if rig.skins.is_empty() {
        rig.skins.push(Skin { name: DEFAULT_SKIN.into(), attachments: BTreeMap::new() });
    }
    rig.skins[0].attachments.insert(
        aid.clone(),
        Attachment::Region(RegionAttachment { image: image.into(), size, xform: Xform::translate(at[0], at[1]) }),
    );
    rig.slots.push(Slot { id: sid.clone(), name: slot.into(), bone: bone.clone(), attachment: Some(aid.clone()), color: Rgba::new(1.0, 1.0, 1.0, 1.0) });
    (sid, aid)
}

fn key<T>(t: f32, value: T) -> Key<T> {
    Key::new(t, value)
}

#[test]
fn bounds_come_from_real_visible_pixels_and_recenter_puts_the_feet_on_the_origin() {
    let mut rig = Rig::default();
    let root = bone(&mut rig, None, "root", 0.0, 10.0);
    // A 40x100 body whose center is at (30, -20): spans x 10..50, y -70..30.
    region(&mut rig, "body", &root, "images/body.png", [40.0, 100.0], [30.0, -20.0]);
    // A hidden slot far away must not count.
    let (hidden, _) = region(&mut rig, "ghost", &root, "images/ghost.png", [40.0, 40.0], [500.0, 500.0]);
    rig.slots.iter_mut().find(|s| s.id == hidden).unwrap().attachment = None;
    let b = rig.rest_bounds(None, false).unwrap();
    assert_eq!(b, [10.0, -70.0, 50.0, 30.0]);
    let r = recenter(&mut rig, None, false).unwrap();
    assert_eq!(r.shift, [-30.0, -30.0]);
    assert_eq!(rig.rest_bounds(None, false).unwrap(), [-20.0, -100.0, 20.0, 0.0]);
    assert_eq!((rig.bones[0].rest.x, rig.bones[0].rest.y), (-30.0, -30.0));
    // Second time: nothing to do.
    assert_eq!(recenter(&mut rig, None, false).unwrap().shift, [0.0, 0.0]);
}

#[test]
fn recenter_moves_meshes_with_the_root_and_follows_the_chosen_skin() {
    let mut rig = Rig::default();
    let root = bone(&mut rig, None, "root", 5.0, 10.0);
    let mesh = |dx: f32| MeshAttachment {
        image: "images/m.png".into(),
        vertices: vec![[dx, 0.0], [dx + 10.0, 0.0], [dx + 10.0, 20.0]],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
        triangles: vec![[0, 1, 2]],
        weights: Vec::new(),
    };
    let aid = AttachmentId::new("m");
    rig.skins.push(Skin { name: "default".into(), attachments: BTreeMap::from([(aid.clone(), Attachment::Mesh(mesh(0.0)))]) });
    rig.skins.push(Skin { name: "wide".into(), attachments: BTreeMap::from([(aid.clone(), Attachment::Mesh(mesh(100.0)))]) });
    rig.slots.push(Slot { id: SlotId::new("s"), name: "s".into(), bone: root, attachment: Some(aid), color: Rgba::new(1.0, 1.0, 1.0, 1.0) });
    // Active skin "wide": measured on its own mesh (x 100..110, y 0..20).
    let mut a = rig.clone();
    assert_eq!(recenter(&mut a, Some("wide"), false).unwrap().shift, [-105.0, -20.0]);
    // Every mesh of every skin moved by the same amount.
    let Attachment::Mesh(m) = &a.skins[0].attachments[&AttachmentId::new("m")] else { unreachable!() };
    assert_eq!(m.vertices[0], [-105.0, -20.0]);
    // All skins: bounds 0..110.
    let mut b = rig.clone();
    assert_eq!(recenter(&mut b, None, true).unwrap().shift, [-55.0, -20.0]);
    assert!(recenter(&mut rig, Some("nope"), false).is_err());
}

#[test]
fn recenter_refuses_a_rig_with_nothing_visible() {
    let mut rig = Rig::default();
    bone(&mut rig, None, "root", 0.0, 10.0);
    assert!(recenter(&mut rig, None, false).is_err());
}

#[test]
fn the_origin_warning_fires_when_far_from_the_drawing_only() {
    let mut rig = Rig::default();
    let root = bone(&mut rig, None, "root", 0.0, 10.0);
    // Feet on the origin, centered: fine.
    region(&mut rig, "body", &root, "images/body.png", [40.0, 100.0], [0.0, -50.0]);
    let far = |r: &Rig| r.validate().iter().any(|i| matches!(i.kind, RigIssueKind::OriginFarFromDrawing { .. }));
    assert!(!far(&rig));
    // Slightly off (within 25% of the width, 10% of the height): still fine.
    rig.bones[0].rest.x = 8.0;
    rig.bones[0].rest.y = -5.0;
    assert!(!far(&rig));
    // Origin well to the side.
    rig.bones[0].rest.x = 15.0;
    rig.bones[0].rest.y = 0.0;
    assert!(far(&rig));
    // Origin at the hips (above the bottom).
    rig.bones[0].rest.x = 0.0;
    rig.bones[0].rest.y = -50.0;
    let w = rig.validate().into_iter().find(|i| matches!(i.kind, RigIssueKind::OriginFarFromDrawing { .. })).unwrap();
    assert_eq!(w.severity, Severity::Warning);
    // Recentering fixes it.
    recenter(&mut rig, None, false).unwrap();
    assert!(!far(&rig));
}

// ---------------------------------------------------------- copy clips

fn walker(names: [&str; 3], lengths: [f32; 3]) -> Rig {
    let mut rig = Rig::default();
    let root = bone(&mut rig, None, names[0], 0.0, lengths[0]);
    let a = bone(&mut rig, Some(&root.0), names[1], 10.0, lengths[1]);
    bone(&mut rig, Some(&root.0), names[2], -10.0, lengths[2]);
    region(&mut rig, &format!("{}_pic", names[1]), &a, &format!("images/{}.png", names[1]), [10.0, 10.0], [0.0, 0.0]);
    rig
}

fn clip_with(rig: &mut Rig, bones: &[&str]) {
    let mut c = RigClip::new(1.0, Loop::Forever);
    for n in bones {
        let id = rig.bones.iter().find(|b| b.name == *n).unwrap().id.clone();
        c.tracks.push(RigTrack::BoneTranslate { bone: id.clone(), track: Track::new(vec![key(0.0, Vec2 { x: 0.0, y: 0.0 }), key(1.0, Vec2 { x: 10.0, y: 20.0 })]) });
        c.tracks.push(RigTrack::BoneRotate { bone: id, track: Track::new(vec![key(0.0, Angle(0.0)), key(1.0, Angle(30.0))]) });
    }
    rig.clips.insert("walk".into(), c);
}

#[test]
fn clips_map_by_name_and_report_what_is_missing() {
    let mut src = walker(["root", "arm", "tail"], [10.0, 20.0, 10.0]);
    clip_with(&mut src, &["root", "arm", "tail"]);
    let mut dest = walker(["root", "arm", "wing"], [10.0, 20.0, 10.0]);
    let r = copy_clips(&mut dest, &src, &CopyClipsSpec::default()).unwrap();
    assert_eq!(r.copied, vec![("walk".to_owned(), "walk".to_owned())]);
    assert_eq!(r.unmatched_bones, vec!["tail".to_owned()]);
    assert_eq!((r.tracks_copied, r.tracks_dropped), (4, 2));
    assert_eq!(dest.clips["walk"].tracks.len(), 4);
    // Targets are the destination's ids.
    for t in &dest.clips["walk"].tracks {
        let (RigTrack::BoneRotate { bone, .. } | RigTrack::BoneTranslate { bone, .. }) = t else { panic!() };
        assert!(dest.bones.iter().any(|b| &b.id == bone));
    }
    // A second copy does not overwrite.
    let r = copy_clips(&mut dest, &src, &CopyClipsSpec::default()).unwrap();
    assert_eq!(r.copied[0].1, "walk2");
    assert_eq!(dest.clips.len(), 2);
}

#[test]
fn clips_map_by_hierarchy_when_names_differ() {
    let mut src = walker(["root", "arm", "tail"], [10.0, 20.0, 10.0]);
    clip_with(&mut src, &["arm", "tail"]);
    let mut dest = walker(["hips", "bras", "queue"], [10.0, 20.0, 10.0]);
    let by_name = copy_clips(&mut dest.clone(), &src, &CopyClipsSpec::default()).unwrap();
    assert_eq!(by_name.tracks_copied, 0);
    assert_eq!(by_name.unmatched_bones, vec!["arm".to_owned(), "tail".to_owned()]);
    let r = copy_clips(&mut dest, &src, &CopyClipsSpec { by_hierarchy: true, ..Default::default() }).unwrap();
    assert_eq!((r.tracks_copied, r.tracks_dropped), (4, 0));
    let id_of = |n: &str| dest.bones.iter().find(|b| b.name == n).unwrap().id.clone();
    let RigTrack::BoneRotate { bone, .. } = &dest.clips["walk"].tracks[1] else { panic!() };
    assert_eq!(bone, &id_of("bras"));
    let RigTrack::BoneRotate { bone, .. } = &dest.clips["walk"].tracks[3] else { panic!() };
    assert_eq!(bone, &id_of("queue"));
}

#[test]
fn translation_is_rescaled_by_the_bone_length_ratio_only_when_asked() {
    let mut src = walker(["root", "arm", "tail"], [10.0, 20.0, 10.0]);
    clip_with(&mut src, &["arm"]);
    let translation = |rig: &Rig, clip: &str| match &rig.clips[clip].tracks[0] {
        RigTrack::BoneTranslate { track, .. } => track.keys[1].value,
        _ => panic!(),
    };
    let mut plain = walker(["root", "arm", "tail"], [10.0, 40.0, 10.0]);
    copy_clips(&mut plain, &src, &CopyClipsSpec::default()).unwrap();
    assert_eq!(translation(&plain, "walk"), Vec2 { x: 10.0, y: 20.0 });
    let mut scaled = walker(["root", "arm", "tail"], [10.0, 40.0, 10.0]);
    copy_clips(&mut scaled, &src, &CopyClipsSpec { rescale: true, ..Default::default() }).unwrap();
    assert_eq!(translation(&scaled, "walk"), Vec2 { x: 20.0, y: 40.0 });
    // A zero length leaves the keys alone (no division).
    let mut zero = walker(["root", "arm", "tail"], [10.0, 0.0, 10.0]);
    copy_clips(&mut zero, &src, &CopyClipsSpec { rescale: true, ..Default::default() }).unwrap();
    assert_eq!(translation(&zero, "walk"), Vec2 { x: 10.0, y: 20.0 });
    // Rotations are never rescaled.
    let RigTrack::BoneRotate { track, .. } = &scaled.clips["walk"].tracks[1] else { panic!() };
    assert_eq!(track.keys[1].value, Angle(30.0));
}

fn mesh_rig(vertices: usize) -> (Rig, SlotId, AttachmentId) {
    let mut rig = Rig::default();
    let root = bone(&mut rig, None, "root", 0.0, 10.0);
    let mesh = MeshAttachment {
        image: "images/skirt.png".into(),
        vertices: (0..vertices).map(|i| [i as f32, 0.0]).collect(),
        uvs: (0..vertices).map(|_| [0.0, 0.0]).collect(),
        triangles: Vec::new(),
        weights: Vec::new(),
    };
    let aid = AttachmentId::new("skirt_a");
    rig.skins.push(Skin { name: "default".into(), attachments: BTreeMap::from([(aid.clone(), Attachment::Mesh(mesh))]) });
    let sid = SlotId::new("skirt_s");
    rig.slots.push(Slot { id: sid.clone(), name: "skirt".into(), bone: root, attachment: Some(aid.clone()), color: Rgba::new(1.0, 1.0, 1.0, 1.0) });
    (rig, sid, aid)
}

#[test]
fn deform_keys_survive_only_between_meshes_with_the_same_vertex_count() {
    let (mut src, sid, aid) = mesh_rig(4);
    let mut c = RigClip::new(1.0, Loop::Once);
    c.tracks.push(RigTrack::Deform { slot: sid, attachment: aid, track: Track::new(vec![key(0.0, Deform(vec![[1.0, 1.0]; 4]))]) });
    src.clips.insert("sway".into(), c);
    let (mut same, ..) = mesh_rig(4);
    let r = copy_clips(&mut same, &src, &CopyClipsSpec::default()).unwrap();
    assert_eq!((r.tracks_copied, r.dropped_deform.len()), (1, 0));
    let (mut other, ..) = mesh_rig(6);
    let r = copy_clips(&mut other, &src, &CopyClipsSpec::default()).unwrap();
    assert_eq!((r.tracks_copied, r.tracks_dropped), (0, 1));
    assert_eq!(r.dropped_deform, vec!["skirt/skirt_a".to_owned()]);
    assert!(other.clips["sway"].tracks.is_empty());
}

#[test]
fn attachment_keys_map_by_id_or_picture_name_and_unknown_ones_are_reported() {
    let (mut src, sid, _) = mesh_rig(4);
    let Attachment::Mesh(m) = src.skins[0].attachments.get_mut(&AttachmentId::new("skirt_a")).unwrap() else { unreachable!() };
    let copy = m.clone();
    src.skins[0].attachments.insert(AttachmentId::new("other_a"), Attachment::Mesh(MeshAttachment { image: "images/other.png".into(), ..copy.clone() }));
    src.skins[0].attachments.insert(AttachmentId::new("lost_a"), Attachment::Mesh(MeshAttachment { image: "images/lost.png".into(), ..copy }));
    let step = |a: &str| Step(Some(AttachmentId::new(a)));
    let mut c = RigClip::new(1.0, Loop::Once);
    c.tracks.push(RigTrack::SlotAttachment { slot: sid, track: Track::new(vec![key(0.0, step("skirt_a")), key(0.5, step("other_a")), key(0.8, step("lost_a"))]) });
    src.clips.insert("swap".into(), c);
    // Destination: different ids, but the pictures `skirt` and `other` exist.
    let (mut dest, ..) = mesh_rig(4);
    let Attachment::Mesh(m) = dest.skins[0].attachments.remove(&AttachmentId::new("skirt_a")).unwrap() else { unreachable!() };
    dest.skins[0].attachments.insert(AttachmentId::new("d1"), Attachment::Mesh(m.clone()));
    dest.skins[0].attachments.insert(AttachmentId::new("d2"), Attachment::Mesh(MeshAttachment { image: "images/other.png".into(), ..m }));
    dest.slots[0].attachment = Some(AttachmentId::new("d1"));
    let r = copy_clips(&mut dest, &src, &CopyClipsSpec::default()).unwrap();
    assert_eq!(r.unmatched_attachments, vec!["skirt/lost_a".to_owned()]);
    let RigTrack::SlotAttachment { track, .. } = &dest.clips["swap"].tracks[0] else { panic!() };
    let ids: Vec<_> = track.keys.iter().map(|k| k.value.0.clone().unwrap().0).collect();
    assert_eq!(ids, vec!["d1".to_owned(), "d2".to_owned()]);
    assert!(dest.validate().iter().all(|i| i.severity != Severity::Error));
}

#[test]
fn copying_an_unknown_clip_is_refused_and_changes_nothing() {
    let src = walker(["root", "arm", "tail"], [10.0, 20.0, 10.0]);
    let mut dest = src.clone();
    let before = dest.clone();
    let spec = CopyClipsSpec { clips: vec!["nope".into()], ..Default::default() };
    assert!(copy_clips(&mut dest, &src, &spec).is_err());
    assert_eq!(dest, before);
    assert!(copy_clips(&mut dest, &src, &CopyClipsSpec::default()).is_err(), "a source with no clip");
}

// ------------------------------------------------------------ duplicate

#[test]
fn duplicate_copies_the_rig_and_optionally_its_pictures() {
    let root = std::env::temp_dir().join(format!("orbit_dup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("images/a")).unwrap();
    std::fs::create_dir_all(root.join("images/b")).unwrap();
    std::fs::write(root.join("images/a/hand.png"), b"A").unwrap();
    std::fs::write(root.join("images/b/hand.png"), b"B").unwrap();
    let mut rig = Rig::default();
    let r = bone(&mut rig, None, "root", 0.0, 10.0);
    region(&mut rig, "one", &r, "images/a/hand.png", [1.0, 1.0], [0.0, 0.0]);
    region(&mut rig, "two", &r, "images/b/hand.png", [1.0, 1.0], [0.0, 0.0]);
    // Shared pictures.
    let d = duplicate_files(&root, &rig, "Hero copy!", false).unwrap();
    assert_eq!(d.rig, "assets/rigs/Hero_copy.rig.ron");
    assert!(d.pictures.is_empty());
    let text = std::fs::read_to_string(root.join(&d.rig)).unwrap();
    assert_eq!(orbit_anim::rig::load_rig(&text).unwrap().rig, rig);
    // Never overwrites.
    assert!(duplicate_files(&root, &rig, "Hero copy!", false).is_err());
    // Copied pictures: two files with the same name do not collide.
    let d = duplicate_files(&root, &rig, "hero2", true).unwrap();
    assert_eq!(d.pictures.len(), 2);
    let copy = orbit_anim::rig::load_rig(&std::fs::read_to_string(root.join(&d.rig)).unwrap()).unwrap().rig;
    let imgs = crate::rigs::rig_images(&copy);
    assert_eq!(imgs, vec!["images/hero2/hand.png".to_owned(), "images/hero2/hand_2.png".to_owned()]);
    assert_eq!(std::fs::read(root.join(&imgs[0])).unwrap(), b"A");
    assert_eq!(std::fs::read(root.join(&imgs[1])).unwrap(), b"B");
    // A missing picture fails cleanly: nothing is left.
    std::fs::remove_file(root.join("images/b/hand.png")).unwrap();
    assert!(duplicate_files(&root, &rig, "hero3", true).is_err());
    assert!(!root.join("assets/rigs/hero3.rig.ron").exists());
    assert!(!root.join("images/hero3/hand.png").exists());
    assert!(duplicate_files(&root, &rig, "  ", false).is_err());
    let _ = std::fs::remove_dir_all(&root);
}
