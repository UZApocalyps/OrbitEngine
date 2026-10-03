use std::collections::BTreeMap;

use super::*;
use crate::lerp::{Angle, Rgba, Step, Vec2};
use crate::track::{Key, Track};
use crate::{Curve, Loop};

fn close(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

fn bone(id: &str, parent: Option<&str>, x: f32, y: f32, length: f32) -> Bone {
    Bone {
        id: BoneId::new(id),
        name: id.to_string(),
        parent: parent.map(BoneId::new),
        rest: Xform::translate(x, y),
        length,
    }
}

/// root at (100, 100), arm along +X (10 units), forearm 10 more.
fn arm_rig() -> Rig {
    let mut attachments = BTreeMap::new();
    attachments.insert(
        AttachmentId::new("strip"),
        Attachment::Mesh(MeshAttachment {
            image: "arm.png".into(),
            // Bind space (rig space, setup pose).
            vertices: vec![[100.0, 100.0], [110.0, 100.0], [120.0, 100.0]],
            uvs: vec![[0.0, 0.0], [0.5, 0.0], [1.0, 0.0]],
            triangles: vec![[0, 1, 2]],
            weights: vec![
                vec![(BoneId::new("arm"), 1.0)],
                vec![(BoneId::new("arm"), 0.5), (BoneId::new("fore"), 0.5)],
                vec![(BoneId::new("fore"), 1.0)],
            ],
        }),
    );
    attachments.insert(
        AttachmentId::new("hand"),
        Attachment::Region(RegionAttachment { image: "hand.png".into(), size: [4.0, 2.0], xform: Xform::translate(10.0, 0.0) }),
    );
    Rig {
        bones: vec![
            bone("root", None, 100.0, 100.0, 0.0),
            bone("arm", Some("root"), 0.0, 0.0, 10.0),
            bone("fore", Some("arm"), 10.0, 0.0, 10.0),
            bone("goal", Some("root"), 20.0, 0.0, 0.0),
        ],
        slots: vec![
            Slot {
                id: SlotId::new("s_arm"),
                name: "arm".into(),
                bone: BoneId::new("arm"),
                attachment: Some(AttachmentId::new("strip")),
                color: Rgba::new(1.0, 1.0, 1.0, 1.0),
            },
            Slot {
                id: SlotId::new("s_hand"),
                name: "hand".into(),
                bone: BoneId::new("fore"),
                attachment: Some(AttachmentId::new("hand")),
                color: Rgba::new(1.0, 1.0, 1.0, 1.0),
            },
        ],
        skins: vec![Skin { name: "default".into(), attachments }],
        ..Rig::default()
    }
}

fn rotate_clip(bone: &str, from: f32, to: f32, duration: f32, looping: Loop) -> RigClip {
    let mut c = RigClip::new(duration, looping);
    c.tracks.push(RigTrack::BoneRotate {
        bone: BoneId::new(bone),
        track: Track::new(vec![Key::new(0.0, Angle(from)), Key::new(duration, Angle(to))]),
    });
    c
}

#[test]
fn affine_compose_and_inverse() {
    let m = Xform { x: 3.0, y: -2.0, rotation: 30.0, sx: 2.0, sy: 0.5 }.matrix();
    let n = Xform { x: -1.0, y: 4.0, rotation: -75.0, sx: 1.5, sy: 1.5 }.matrix();
    let p = [7.0, -3.0];
    assert!(close(m.mul(&n).apply(p), m.apply(n.apply(p))));
    let inv = m.inverse().unwrap();
    assert!(close(inv.apply(m.apply(p)), p));
    assert!(Xform { sx: 0.0, ..Xform::IDENTITY }.matrix().inverse().is_none());
    // 90° clockwise (Y down): +X goes to +Y.
    assert!(close(Xform { rotation: 90.0, ..Xform::IDENTITY }.matrix().apply([1.0, 0.0]), [0.0, 1.0]));
}

#[test]
fn world_matrices_follow_the_hierarchy() {
    let rig = PreparedRig::new(arm_rig()).unwrap();
    let mut pose = rig.setup_pose();
    let w = rig.world(&pose);
    assert!(close(w.bones[2].origin(), [110.0, 100.0]));
    pose.bones[1].rotation = 90.0;
    let w = rig.world(&pose);
    assert!(close(w.bones[2].origin(), [100.0, 110.0]));
    assert!((w.bones[2].x_angle() - 90.0).abs() < 1e-3);
    pose.bones[0].sx = 2.0;
    pose.bones[0].sy = 2.0;
    let w = rig.world(&pose);
    assert!(close(w.bones[2].origin(), [100.0, 120.0]));
}

#[test]
fn skinning_is_identity_in_setup_pose_and_blends_weights() {
    let rig = PreparedRig::new(arm_rig()).unwrap();
    let pose = rig.setup_pose();
    let world = rig.world(&pose);
    let list = rig.draw_list(&pose, &world, None, &CpuSkinner);
    assert_eq!(list.positions, vec![[100.0, 100.0], [110.0, 100.0], [120.0, 100.0]]);

    // Bend the forearm 90°: vertex 2 (fore only) goes to (110, 110); vertex 1
    // sits on the joint so both bones agree; vertex 0 does not move.
    let mut pose = rig.setup_pose();
    pose.bones[2].rotation = 90.0;
    let world = rig.world(&pose);
    let list = rig.draw_list(&pose, &world, None, &CpuSkinner);
    assert!(close(list.positions[0], [100.0, 100.0]));
    assert!(close(list.positions[1], [110.0, 100.0]));
    assert!(close(list.positions[2], [110.0, 110.0]));

    // A half/half vertex off the joint lands halfway between both bones'
    // opinions (linear blend skinning).
    let mut rig2 = arm_rig();
    if let Attachment::Mesh(m) = rig2.skins[0].attachments.get_mut(&AttachmentId::new("strip")).unwrap() {
        m.vertices[1] = [115.0, 100.0];
    }
    let rig2 = PreparedRig::new(rig2).unwrap();
    let world = rig2.world(&pose);
    let list = rig2.draw_list(&pose, &world, None, &CpuSkinner);
    // arm alone: (115, 100); fore alone: (110, 105); average:
    assert!(close(list.positions[1], [112.5, 102.5]));
}

#[test]
fn rigid_mesh_and_region_follow_their_bone() {
    let mut r = arm_rig();
    if let Attachment::Mesh(m) = r.skins[0].attachments.get_mut(&AttachmentId::new("strip")).unwrap() {
        m.weights.clear();
    }
    let rig = PreparedRig::new(r).unwrap();
    let mut pose = rig.setup_pose();
    pose.bones[1].rotation = 90.0;
    let world = rig.world(&pose);
    let list = rig.draw_list(&pose, &world, None, &CpuSkinner);
    // Rigid on "arm" (pivot at 100,100).
    assert!(close(list.positions[2], [100.0, 120.0]));
    // Region on "fore": 10 units along the forearm (which now points down).
    let RigDrawItem::Region { transform, size, image, .. } = &list.items[1] else { panic!() };
    assert_eq!(*image, "hand.png");
    assert_eq!(*size, [4.0, 2.0]);
    assert!(close(transform.origin(), [100.0, 120.0]));
}

#[test]
fn deform_offsets_are_added_before_skinning() {
    let rig = PreparedRig::new(arm_rig()).unwrap();
    let mut pose = rig.setup_pose();
    pose.deforms.insert((0, AttachmentId::new("strip")), vec![[0.0, 0.0], [0.0, 0.0], [0.0, 3.0]]);
    pose.bones[2].rotation = 90.0;
    let world = rig.world(&pose);
    let list = rig.draw_list(&pose, &world, None, &CpuSkinner);
    // (120, 103) in bind space, rotated 90° around (110, 100) → (107, 110).
    assert!(close(list.positions[2], [107.0, 110.0]));
}

#[test]
fn clip_sampling_loops_and_is_relative_to_setup() {
    let mut r = arm_rig();
    r.bones[1].rest.rotation = 10.0;
    r.clips.insert("wave".into(), rotate_clip("arm", 0.0, 40.0, 2.0, Loop::Forever));
    let rig = PreparedRig::new(r).unwrap();
    assert!((rig.sample("wave", 1.0).bones[1].rotation - 30.0).abs() < 1e-4);
    assert!((rig.sample("wave", 3.0).bones[1].rotation - 30.0).abs() < 1e-4);
    // Unknown clip: setup pose.
    assert_eq!(rig.sample("nope", 1.0), rig.setup_pose());
}

#[test]
fn slot_tracks_switch_attachment_color_and_order() {
    let mut r = arm_rig();
    let mut c = RigClip::new(1.0, Loop::Once);
    c.tracks.push(RigTrack::SlotAttachment {
        slot: SlotId::new("s_hand"),
        track: Track::new(vec![Key::new(0.0, Step(Some(AttachmentId::new("hand")))), Key::new(0.5, Step(None))]),
    });
    c.tracks.push(RigTrack::SlotColor {
        slot: SlotId::new("s_arm"),
        track: Track::new(vec![Key::new(0.0, Rgba::new(1.0, 1.0, 1.0, 1.0)), Key::new(1.0, Rgba::new(0.0, 0.0, 0.0, 1.0))]),
    });
    c.tracks.push(RigTrack::DrawOrder { track: Track::new(vec![Key::new(0.0, Step(vec![SlotId::new("s_hand")]))]) });
    r.clips.insert("c".into(), c);
    let rig = PreparedRig::new(r).unwrap();
    let p = rig.sample("c", 0.25);
    assert_eq!(p.draw_order, vec![1, 0]);
    assert!(p.slots[1].attachment.is_some());
    assert!((p.slots[0].color.r - 0.75).abs() < 1e-4);
    let p = rig.sample("c", 0.75);
    assert!(p.slots[1].attachment.is_none());
    let w = rig.world(&p);
    assert_eq!(rig.draw_list(&p, &w, None, &CpuSkinner).items.len(), 1);
}

#[test]
fn cross_fade_blends_two_clips() {
    let mut r = arm_rig();
    r.clips.insert("a".into(), rotate_clip("arm", 0.0, 0.0, 1.0, Loop::Forever));
    r.clips.insert("b".into(), rotate_clip("arm", 80.0, 80.0, 1.0, Loop::Forever));
    let rig = PreparedRig::new(r).unwrap();
    let mut state = ClipBlend::new("a", 0.0);
    state.blend = 0.4;
    state.switch("b", 10.0);
    let rot = |t: f32| rig.sample_blend(&state, t).bones[1].rotation;
    assert!((rot(10.0) - 0.0).abs() < 1e-4);
    assert!((rot(10.2) - 40.0).abs() < 1e-3);
    assert!((rot(10.4) - 80.0).abs() < 1e-4);
    assert!((rot(50.0) - 80.0).abs() < 1e-4);
    // Switching to the same clip changes nothing.
    let before = state.clone();
    state.switch("b", 12.0);
    assert_eq!(state, before);
    // Angles blend on the shortest arc.
    let mut a = rig.setup_pose();
    let mut b = rig.setup_pose();
    a.bones[1].rotation = 170.0;
    b.bones[1].rotation = -170.0;
    assert!((RigPose::blend(&a, &b, 0.5).bones[1].rotation.rem_euclid(360.0) - 180.0).abs() < 1e-3);
}

fn tip(rig: &PreparedRig, world: &WorldPose, bone: usize) -> [f32; 2] {
    world.bones[bone].apply([rig.rig().bones[bone].length, 0.0])
}

#[test]
fn ik_one_bone_aims_at_the_target() {
    let mut r = arm_rig();
    r.ik.push(IkConstraint { bones: vec![BoneId::new("arm")], target: BoneId::new("goal"), bend_positive: false, mix: 1.0 });
    let rig = PreparedRig::new(r).unwrap();
    let mut pose = rig.setup_pose();
    pose.bones[3].x = 0.0;
    pose.bones[3].y = 30.0; // straight down from the shoulder
    let w = rig.world(&pose);
    assert!((w.bones[1].x_angle() - 90.0).abs() < 1e-3);
}

#[test]
fn ik_two_bones_reach_the_target_both_ways() {
    for bend in [false, true] {
        let mut r = arm_rig();
        r.ik.push(IkConstraint {
            bones: vec![BoneId::new("arm"), BoneId::new("fore")],
            target: BoneId::new("goal"),
            bend_positive: bend,
            mix: 1.0,
        });
        let rig = PreparedRig::new(r).unwrap();
        let mut pose = rig.setup_pose();
        pose.bones[3].x = 8.0;
        pose.bones[3].y = 9.0;
        let w = rig.world(&pose);
        let t = tip(&rig, &w, 2);
        assert!(close(t, [108.0, 109.0]), "bend {bend}: tip {t:?}");
        // The elbow is on opposite sides depending on the bend direction.
        let elbow = w.bones[2].origin();
        let cross = 8.0 * (elbow[1] - 100.0) - 9.0 * (elbow[0] - 100.0);
        assert_eq!(cross > 0.0, !bend, "bend {bend}: elbow {elbow:?}");
    }
    // Out of reach: the chain straightens towards the target.
    let mut r = arm_rig();
    r.ik.push(IkConstraint { bones: vec![BoneId::new("arm"), BoneId::new("fore")], target: BoneId::new("goal"), bend_positive: true, mix: 1.0 });
    let rig = PreparedRig::new(r).unwrap();
    let mut pose = rig.setup_pose();
    pose.bones[3].x = 0.0;
    pose.bones[3].y = 100.0;
    let w = rig.world(&pose);
    assert!(close(tip(&rig, &w, 2), [100.0, 120.0]));
    // mix 0: no effect.
    let mut r = arm_rig();
    r.ik.push(IkConstraint { bones: vec![BoneId::new("arm"), BoneId::new("fore")], target: BoneId::new("goal"), bend_positive: true, mix: 0.0 });
    let rig = PreparedRig::new(r).unwrap();
    let mut pose = rig.setup_pose();
    pose.bones[3].y = 50.0;
    assert!(close(tip(&rig, &rig.world(&pose), 2), [120.0, 100.0]));
}

#[test]
fn validation_reports_structure_errors() {
    let kinds = |r: &Rig| r.validate().into_iter().map(|i| i.kind).collect::<Vec<_>>();
    let mut r = arm_rig();
    r.bones.swap(1, 2);
    assert!(kinds(&r).contains(&RigIssueKind::ParentAfterChild { bone: "fore".into(), parent: "arm".into() }));
    assert!(PreparedRig::new(r).is_err());

    let mut r = arm_rig();
    r.bones[1].parent = Some(BoneId::new("ghost"));
    r.slots[0].bone = BoneId::new("ghost");
    r.slots[1].attachment = Some(AttachmentId::new("nothing"));
    let k = kinds(&r);
    assert!(k.contains(&RigIssueKind::UnknownParent { bone: "arm".into(), parent: "ghost".into() }));
    assert!(k.contains(&RigIssueKind::SlotUnknownBone { slot: "s_arm".into(), bone: "ghost".into() }));
    assert!(k.contains(&RigIssueKind::UnknownAttachment { slot: "s_hand".into(), attachment: "nothing".into() }));

    let mut r = arm_rig();
    if let Attachment::Mesh(m) = r.skins[0].attachments.get_mut(&AttachmentId::new("strip")).unwrap() {
        m.triangles.push([0, 1, 7]);
        m.uvs.pop();
    }
    r.ik.push(IkConstraint { bones: vec![BoneId::new("root"), BoneId::new("fore")], target: BoneId::new("goal"), bend_positive: false, mix: 1.0 });
    let k = kinds(&r);
    assert!(k.contains(&RigIssueKind::TriangleOutOfRange { attachment: "strip".into(), triangle: 1 }));
    assert!(k.contains(&RigIssueKind::MeshLengths { attachment: "strip".into() }));
    assert!(k.contains(&RigIssueKind::IkNotAChain(0)));
    assert!(arm_rig().validate().is_empty());
}

#[test]
fn weights_are_normalized_with_a_warning() {
    let mut r = arm_rig();
    if let Attachment::Mesh(m) = r.skins[0].attachments.get_mut(&AttachmentId::new("strip")).unwrap() {
        m.weights[1] = vec![(BoneId::new("arm"), 2.0), (BoneId::new("fore"), 2.0)];
        m.weights[2] = vec![
            (BoneId::new("fore"), 0.5),
            (BoneId::new("arm"), 0.2),
            (BoneId::new("root"), 0.1),
            (BoneId::new("goal"), 0.1),
            (BoneId::new("root"), 0.05),
        ];
    }
    let issues = r.validate();
    assert!(issues.iter().all(|i| i.severity == Severity::Warning));
    assert!(issues.iter().any(|i| matches!(i.kind, RigIssueKind::WeightsNotNormalized { vertex: 1, .. })));
    assert!(issues.iter().any(|i| matches!(i.kind, RigIssueKind::TooManyInfluences { vertex: 2, .. })));
    let prepared = PreparedRig::new(r.clone()).unwrap();
    assert_eq!(prepared.warnings.len(), 2);
    // Prepared weights are normalized: setup pose still skins to identity.
    let pose = prepared.setup_pose();
    let list = prepared.draw_list(&pose, &prepared.world(&pose), None, &CpuSkinner);
    assert!(close(list.positions[1], [110.0, 100.0]));
    assert_eq!(r.normalize_weights(), 2);
    assert!(r.validate().is_empty());
    let Attachment::Mesh(m) = &r.skins[0].attachments[&AttachmentId::new("strip")] else { panic!() };
    assert_eq!(m.weights[2].len(), 4);
    let sum: f32 = m.weights[2].iter().map(|w| w.1).sum();
    assert!((sum - 1.0).abs() < 1e-5);
}

#[test]
fn web_budget_is_a_warning() {
    let mut r = arm_rig();
    for i in 0..70 {
        r.bones.push(bone(&format!("b{i}"), Some("root"), 0.0, 0.0, 1.0));
    }
    if let Attachment::Mesh(m) = r.skins[0].attachments.get_mut(&AttachmentId::new("strip")).unwrap() {
        m.vertices.resize(4100, [0.0, 0.0]);
        m.uvs.resize(4100, [0.0, 0.0]);
        m.weights.resize(4100, vec![(BoneId::new("root"), 1.0)]);
    }
    let k: Vec<_> = r.validate().into_iter().map(|i| (i.severity, i.kind)).collect();
    assert!(k.contains(&(Severity::Warning, RigIssueKind::WebTooManyBones(74))));
    assert!(k.contains(&(Severity::Warning, RigIssueKind::WebTooManyVertices(4100))));
    assert!(PreparedRig::new(r).is_ok());
}

#[test]
fn ron_round_trip_and_versions() {
    let mut r = arm_rig();
    r.clips.insert("wave".into(), rotate_clip("arm", 0.0, 40.0, 2.0, Loop::PingPong));
    r.clips.get_mut("wave").unwrap().tracks.push(RigTrack::BoneTranslate {
        bone: BoneId::new("root"),
        track: Track::new(vec![Key::with_ease(0.0, Vec2::new(0.0, 0.0), Curve::default()), Key::new(1.0, Vec2::new(0.0, -4.0))]),
    });
    let text = save_rig(&r).unwrap();
    let loaded = load_rig(&text).unwrap();
    assert_eq!(loaded.rig, r);
    assert_eq!(loaded.migrated_from, None);

    // A missing rig_version reads as v1.
    let minimal = "(bones: [(id: \"r\", name: \"root\")])";
    assert_eq!(rig_text_version(minimal).unwrap(), 1);
    let l = load_rig(minimal).unwrap();
    assert_eq!(l.rig.rig_version, RIG_VERSION);
    assert_eq!(l.rig.bones[0].rest, Xform::IDENTITY);

    assert!(matches!(load_rig("(rig_version: 999)"), Err(RigLoadError::TooNew { found: 999, .. })));
    assert!(matches!(load_rig("not ron at all ("), Err(RigLoadError::Parse(_))));
}
