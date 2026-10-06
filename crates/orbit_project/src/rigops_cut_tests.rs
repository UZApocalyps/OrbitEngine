//! Tests of [`super::cut_part`] ("cut to new part").

use super::*;
use crate::rigmesh::pixels::{Mask, polygon_mask};

/// A 40×30 opaque face (skin) with a dark strand across it.
fn face() -> Picture {
    let (w, h) = (40u32, 30u32);
    let strand = polygon_mask(w, h, &[[10.0, 0.0], [16.0, 0.0], [24.0, 30.0], [18.0, 30.0]]);
    let rgba = (0..w * h)
        .flat_map(|i| if strand.bits[i as usize] != 0 { [30, 20, 10, 255] } else { [230, 190, 170, 255] })
        .collect();
    Picture { path: "images/face.png".into(), width: w, height: h, rgba }
}

fn strand_mask() -> Mask {
    polygon_mask(40, 30, &[[9.0, 0.0], [17.0, 0.0], [25.0, 30.0], [17.0, 30.0]])
}

fn map_of(rig: &Rig, att: &AttachmentId) -> Affine {
    match &rig.skins[0].attachments[att] {
        Attachment::Mesh(m) => meshgen::uv_map(m).unwrap().inverse().unwrap(),
        Attachment::Region(_) => unreachable!(),
    }
}

#[test]
fn cut_makes_an_aligned_part_in_front_and_fills_the_hole() {
    let pic = face();
    let (mut rig, _, slot) = rig_from_picture(&pic).unwrap();
    bone_add(&mut rig, Some("root"), &BoneSpec { name: Some("strand".into()), ..BoneSpec::default() }).unwrap();
    let spec = CutSpec {
        slot: slot.as_str(),
        new_image: "images/face_strand.png",
        bone: Some("strand"),
        fill_hole: true,
        ..CutSpec::default()
    };
    let mask = strand_mask();
    let r = cut_part(&mut rig, &pic, &mask, &spec).unwrap();
    // Draw order: just in front of the source, on the chosen bone.
    assert_eq!(rig.slots.len(), 2);
    assert_eq!(rig.slots[1].id, r.slot);
    assert_eq!(rig.slots[1].name, "face_cut");
    assert_eq!(rig.bones.iter().find(|b| b.id == rig.slots[1].bone).unwrap().name, "strand");
    // Pixel exact: every selected pixel copied, the rest transparent.
    let [x0, y0] = r.offset;
    for y in 0..r.part.height {
        for x in 0..r.part.width {
            let d = ((y * r.part.width + x) * 4) as usize;
            let s = (((y + y0) * pic.width + x + x0) * 4) as usize;
            let want = if mask.get(x + x0, y + y0) { &pic.rgba[s..s + 4] } else { &[0, 0, 0, 0][..] };
            assert_eq!(&r.part.rgba[d..d + 4], want);
        }
    }
    // Same place: the part's center lands on the same pixel of the source.
    let src_map = map_of(&rig, rig.slots[0].attachment.as_ref().unwrap());
    let p = map_of(&rig, &r.attachment).apply([0.5, 0.5]);
    let q = src_map.apply([
        (x0 as f32 + r.part.width as f32 * 0.5) / pic.width as f32,
        (y0 as f32 + r.part.height as f32 * 0.5) / pic.height as f32,
    ]);
    assert!((p[0] - q[0]).abs() < 0.01 && (p[1] - q[1]).abs() < 0.01, "{p:?} vs {q:?}");
    // The hole is skin again, the rest untouched; an opaque face keeps its mesh.
    let filled = r.source.unwrap();
    for i in 0..(pic.width * pic.height) as usize {
        if mask.bits[i] == 0 {
            assert_eq!(filled[i * 4..i * 4 + 4], pic.rgba[i * 4..i * 4 + 4]);
        } else {
            assert!(filled[i * 4].abs_diff(230) < 4, "{:?}", &filled[i * 4..i * 4 + 4]);
        }
    }
    assert_eq!(r.remeshed, 0);
}

#[test]
fn cut_below_without_fill_keeps_the_source_and_refuses_bad_selections() {
    let pic = face();
    let (mut rig, _, slot) = rig_from_picture(&pic).unwrap();
    let spec = CutSpec { slot: slot.as_str(), new_image: "images/x.png", below: true, ..CutSpec::default() };
    let r = cut_part(&mut rig, &pic, &strand_mask(), &spec).unwrap();
    assert_eq!(rig.slots[0].id, r.slot);
    assert!(r.source.is_none());
    let before = rig.clone();
    let mut copy = rig.clone();
    assert_eq!(cut_part(&mut copy, &pic, &Mask::new(40, 30), &spec).unwrap_err().kind, OpErrorKind::Refused);
    assert_eq!(cut_part(&mut copy, &pic, &Mask::new(4, 3), &spec).unwrap_err().kind, OpErrorKind::Invalid);
    let other = Picture { path: "images/other.png".into(), ..pic.clone() };
    assert!(cut_part(&mut copy, &other, &strand_mask(), &spec).is_err());
    assert_eq!(copy, before);
}

#[test]
fn a_hole_on_the_silhouette_retraces_the_source_mesh() {
    // A strand sticking out over transparency: cut + fill shrinks the
    // source's silhouette, its mesh is traced again.
    let (w, h) = (40u32, 30u32);
    let body = polygon_mask(w, h, &[[5.0, 10.0], [35.0, 10.0], [35.0, 30.0], [5.0, 30.0]]);
    let strand = polygon_mask(w, h, &[[18.0, 0.0], [22.0, 0.0], [22.0, 12.0], [18.0, 12.0]]);
    let rgba = (0..(w * h) as usize)
        .flat_map(|i| if body.bits[i] != 0 || strand.bits[i] != 0 { [100, 50, 20, 255] } else { [0, 0, 0, 0] })
        .collect();
    let pic = Picture { path: "images/hair.png".into(), width: w, height: h, rgba };
    let (mut rig, _, slot) = rig_from_picture(&pic).unwrap();
    let mut tip = strand.clone();
    tip.subtract(&body);
    let spec = CutSpec { slot: slot.as_str(), new_image: "images/tip.png", fill_hole: true, ..CutSpec::default() };
    let r = cut_part(&mut rig, &pic, &tip, &spec).unwrap();
    assert_eq!(r.remeshed, 1);
    let src = r.source.unwrap();
    assert!(src[(20 * 4 + 3) as usize] < 128, "the top of the hole fades out");
}
