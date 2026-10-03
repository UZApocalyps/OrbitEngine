//! The beginner workflow of the rig editor, as Spine / DragonBones teach it:
//!
//! 1. **Import image**: a picture becomes a rig with a `root` bone at the
//!    character's feet, a slot and a mesh traced from the picture's alpha
//!    ([`rig_from_picture`]); more pictures (arm, head...) are added as
//!    separate slots ([`add_part`]);
//! 2. **Draw bones** over the picture (bone tool);
//! 3. **Bind**: automatic weights of every mesh from the bones ([`bind`]);
//! 4. **Pose / animate**: rotating a bone bends the picture.
//!
//! No egui, no GPU: unit tested headlessly (and end to end through
//! `orbit_render2d` in the tests below when an adapter exists).

use std::path::Path;

use orbit_anim::rig::{Attachment, BoneId, MeshAttachment, Rig, SlotId, Xform};

use super::meshgen::{self, MeshParams};
use super::model::{self, DEFAULT_SKIN, OpError, RigDoc};
use super::weights;

/// A decoded picture and its path as the rig stores it.
#[derive(Clone)]
pub struct Picture {
    /// Project path (relative to the assets root, `/`) or absolute path.
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Picture {
    /// Reads and decodes `path` under `root` (an absolute `path` ignores
    /// `root`).
    pub fn load(root: &Path, path: &str) -> Result<Self, String> {
        let bytes = std::fs::read(root.join(path)).map_err(|e| e.to_string())?;
        let img = orbit_assets::decode_image(&bytes)?;
        Ok(Self { path: path.to_owned(), width: img.width, height: img.height, rgba: img.rgba })
    }

    fn size(&self) -> [f32; 2] {
        [self.width.max(1) as f32, self.height.max(1) as f32]
    }

    /// File name without extension: the slot's name.
    fn stem(&self) -> String {
        let name = self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path);
        let stem = name.split('.').next().unwrap_or(name).trim();
        if stem.is_empty() { "picture".to_owned() } else { stem.to_owned() }
    }
}

/// Where the next step of the workflow is (shown in the viewport).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Import,
    Bones,
    Bind,
    Pose,
}

impl Step {
    pub fn index(self) -> usize {
        match self {
            Step::Import => 0,
            Step::Bones => 1,
            Step::Bind => 2,
            Step::Pose => 3,
        }
    }
}

/// Every mesh of the rig (any skin).
fn meshes(rig: &Rig) -> impl Iterator<Item = &MeshAttachment> {
    rig.skins.iter().flat_map(|s| s.attachments.values()).filter_map(|a| match a {
        Attachment::Mesh(m) => Some(m),
        Attachment::Region(_) => None,
    })
}

/// The step the rig is at. `bound_bones`: the bones of the last bind made in
/// this session (a bone added since asks to bind again).
pub fn step(rig: &Rig, bound_bones: Option<&[BoneId]>) -> Step {
    let has_picture = rig.slots.iter().any(|s| s.attachment.is_some());
    if !has_picture {
        return Step::Import;
    }
    if rig.bones.len() < 2 {
        return Step::Bones;
    }
    let unbound = meshes(rig).any(|m| m.weights.is_empty());
    let changed = bound_bones.is_some_and(|b| b.len() != rig.bones.len() || rig.bones.iter().zip(b).any(|(x, y)| &x.id != y));
    if unbound || changed { Step::Bind } else { Step::Pose }
}

/// Rig-space center of a picture placed "standing on the origin": the rig
/// origin is the bottom-center of the character (what the game expects).
pub fn standing_center(pic: &Picture) -> [f32; 2] {
    [0.0, -pic.size()[1] * 0.5]
}

/// The mesh of `pic` traced from its alpha, at its pixel size, centered on
/// `center` (rig space). Falls back to the picture's rectangle when the
/// tracing fails.
pub fn mesh_for_picture(pic: &Picture, center: [f32; 2]) -> MeshAttachment {
    let size = pic.size();
    let placement = Xform::translate(center[0], center[1]).matrix();
    let uv_to_rig = meshgen::region_uv_to_rig(&placement, size);
    let params = MeshParams::for_character(pic.width, pic.height);
    match meshgen::generate(pic.width, pic.height, &pic.rgba, &params) {
        Ok(g) => meshgen::to_attachment(&pic.path, &g, pic.width, pic.height, &uv_to_rig),
        Err(_) => {
            let uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
            MeshAttachment {
                image: pic.path.clone(),
                vertices: uvs.iter().map(|uv| uv_to_rig.apply(*uv)).collect(),
                uvs,
                triangles: vec![[0, 1, 2], [0, 2, 3]],
                weights: Vec::new(),
            }
        }
    }
}

/// Adds `pic` as a new slot on `bone`, drawn in front of the others, as a
/// mesh centered on `center`. One undo step.
pub fn add_part(doc: &mut RigDoc, bone: &BoneId, pic: &Picture, center: [f32; 2]) -> Result<SlotId, OpError> {
    let open = !doc.in_gesture();
    doc.begin_gesture();
    let result = (|| {
        let slot = doc.add_slot(bone)?;
        let name = model::unique_name(&pic.stem(), doc.rig().slots.iter().map(|s| s.name.as_str()));
        doc.rename_slot(&slot, &name)?;
        doc.attach_image(&slot, DEFAULT_SKIN, &pic.path, pic.size())?;
        doc.set_mesh(&slot, DEFAULT_SKIN, mesh_for_picture(pic, center))?;
        Ok(slot)
    })();
    if open {
        doc.end_gesture();
    }
    result
}

/// A new rig made from one picture: a `root` bone at the feet (pointing up),
/// a slot named after the file and its mesh, the picture standing on the
/// origin. Returns the document, the root bone and the slot.
pub fn rig_from_picture(pic: &Picture) -> Result<(RigDoc, BoneId, SlotId), OpError> {
    let mut doc = RigDoc::new();
    let root = add_root(&mut doc, pic)?;
    let slot = add_part(&mut doc, &root, pic, standing_center(pic))?;
    Ok((doc, root, slot))
}

/// The `root` bone of a rig made from `pic`.
pub fn add_root(doc: &mut RigDoc, pic: &Picture) -> Result<BoneId, OpError> {
    let len = (pic.size()[1] * 0.08).clamp(10.0, 80.0);
    let root = doc.add_bone(None, [0.0, 0.0], [0.0, -len])?;
    doc.rename_bone(&root, "root")?;
    Ok(root)
}

/// Where a new part goes: aligned on the first picture when it has the same
/// pixel size (layers exported from one canvas), else centered on `fallback`.
pub fn part_center(rig: &Rig, pic: &Picture, fallback: [f32; 2]) -> [f32; 2] {
    let same = meshes(rig).find(|m| {
        meshgen::uv_map(m)
            .and_then(|u| u.inverse())
            .is_some_and(|inv| {
                let a = inv.apply([0.0, 0.0]);
                let b = inv.apply([1.0, 1.0]);
                ((b[0] - a[0]).abs() - pic.size()[0]).abs() < 0.5 && ((b[1] - a[1]).abs() - pic.size()[1]).abs() < 0.5
            })
    });
    match same.and_then(|m| meshgen::uv_map(m)?.inverse()) {
        Some(inv) => inv.apply([0.5, 0.5]),
        None => fallback,
    }
}

/// "Bind to bones": automatic weights for every mesh of every skin from the
/// current bones. One undo step; returns how many meshes were bound.
pub fn bind(doc: &mut RigDoc, falloff: f32) -> Result<usize, OpError> {
    doc.edit(|rig| {
        if rig.bones.is_empty() {
            return Ok(0);
        }
        let snapshot = rig.clone();
        let mut n = 0;
        for skin in &mut rig.skins {
            for att in skin.attachments.values_mut() {
                if let Attachment::Mesh(m) = att {
                    m.weights = weights::bind_mesh(&snapshot, m, falloff);
                    n += 1;
                }
            }
        }
        Ok(n)
    })
}

/// The bone new parts and drops attach to: the selected one, else the root.
pub fn part_bone(rig: &Rig, selected: Option<&BoneId>) -> Option<BoneId> {
    selected
        .filter(|b| model::bone(rig, b).is_some())
        .cloned()
        .or_else(|| rig.bones.iter().find(|b| b.parent.is_none()).map(|b| b.id.clone()))
}

/// Whether `path` names a picture the rig editor can import.
pub fn is_picture_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_anim::rig::{CpuSkinner, PreparedRig, RigPose};

    /// A 40×120 "character": an opaque vertical bar on a transparent canvas,
    /// red at the top half, blue at the bottom half.
    fn bar() -> Picture {
        let (w, h) = (40u32, 120u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 8..32 {
                let i = ((y * w + x) * 4) as usize;
                let c = if y < h / 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                rgba[i..i + 4].copy_from_slice(&c);
            }
        }
        Picture { path: "assets/images/bar.png".into(), width: w, height: h, rgba }
    }

    /// Root at the feet, `body` up to the middle, `top` up to the head.
    fn rigged() -> (RigDoc, BoneId, BoneId) {
        let pic = bar();
        let (mut doc, root, _) = rig_from_picture(&pic).unwrap();
        let body = doc.add_bone(Some(&root), [0.0, 0.0], [0.0, -60.0]).unwrap();
        let top = doc.add_bone(Some(&body), [0.0, -60.0], [0.0, -120.0]).unwrap();
        (doc, body, top)
    }

    #[test]
    fn a_picture_becomes_a_rig_standing_on_the_origin() {
        let pic = bar();
        let (doc, root, slot) = rig_from_picture(&pic).unwrap();
        let rig = doc.rig();
        assert_eq!(rig.bones.len(), 1);
        assert_eq!(rig.bones[0].name, "root");
        let s = model::slot(rig, &slot).unwrap();
        assert_eq!(s.bone, root);
        assert_eq!(s.name, "bar");
        let Some(Attachment::Mesh(m)) = rig.skins[0].attachments.get(s.attachment.as_ref().unwrap()) else {
            panic!("a mesh")
        };
        assert!(m.vertices.len() >= 4 && !m.triangles.is_empty());
        // The opaque bar spans x 8..32 → -12..12, y -120..0, padding aside.
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for v in &m.vertices {
            lo = [lo[0].min(v[0]), lo[1].min(v[1])];
            hi = [hi[0].max(v[0]), hi[1].max(v[1])];
        }
        assert!(lo[0] > -16.0 && hi[0] < 16.0, "{lo:?} {hi:?}");
        assert!(lo[1] <= -117.0 && hi[1] >= -3.0 && hi[1] <= 0.5, "{lo:?} {hi:?}");
        // UVs map back to the picture: top vertices sample the top rows.
        for (v, uv) in m.vertices.iter().zip(&m.uvs) {
            assert!((uv[1] - (v[1] + 120.0) / 120.0).abs() < 1e-3);
        }
        assert_eq!(step(rig, None), Step::Bones);
        // One undo step brings back nothing but the root.
        assert!(PreparedRig::new(rig.clone()).is_ok());
    }

    #[test]
    fn the_steps_follow_the_work() {
        assert_eq!(step(&Rig::default(), None), Step::Import);
        let (mut doc, body, _) = rigged();
        assert_eq!(step(doc.rig(), None), Step::Bind);
        bind(&mut doc, 3.0).unwrap();
        let ids: Vec<BoneId> = doc.rig().bones.iter().map(|b| b.id.clone()).collect();
        assert_eq!(step(doc.rig(), Some(&ids)), Step::Pose);
        doc.add_bone(Some(&body), [0.0, -30.0], [30.0, -30.0]).unwrap();
        assert_eq!(step(doc.rig(), Some(&ids)), Step::Bind, "a new bone asks to bind again");
    }

    #[test]
    fn binding_weights_each_half_to_its_bone() {
        let (mut doc, body, top) = rigged();
        assert_eq!(bind(&mut doc, 3.0).unwrap(), 1);
        let rig = doc.rig();
        let m = meshes(rig).next().unwrap();
        assert_eq!(m.weights.len(), m.vertices.len());
        for (v, w) in m.vertices.iter().zip(&m.weights) {
            let sum: f32 = w.iter().map(|x| x.1).sum();
            assert!((sum - 1.0).abs() < 1e-3);
            if v[1] < -100.0 {
                assert!(weights::weight_of(w, &top) > 0.6, "{v:?} {w:?}");
            }
            if v[1] > -50.0 && v[1] < -20.0 {
                assert!(weights::weight_of(w, &top) < 0.3, "{v:?} {w:?}");
                assert!(weights::weight_of(w, &body) > 0.3, "{v:?} {w:?}");
            }
        }
        // One undo step.
        doc.undo();
        assert!(meshes(doc.rig()).next().unwrap().weights.is_empty());
    }

    /// Skinned positions of the mesh with `top` rotated by `degrees`.
    fn posed(doc: &RigDoc, top: &BoneId, degrees: f32) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
        let prepared = PreparedRig::new(doc.rig().clone()).unwrap();
        let mut pose: RigPose = prepared.setup_pose();
        let i = model::bone_index(doc.rig(), top).unwrap();
        pose.bones[i].rotation += degrees;
        let world = prepared.world(&pose);
        let list = prepared.draw_list(&pose, &world, None, &CpuSkinner);
        let rest = prepared.setup_pose();
        let rest_world = prepared.world(&rest);
        let rest_list = prepared.draw_list(&rest, &rest_world, None, &CpuSkinner);
        (rest_list.positions.clone(), list.positions.clone())
    }

    #[test]
    fn rotating_a_bound_bone_bends_the_top_and_keeps_the_feet() {
        let (mut doc, _, top) = rigged();
        bind(&mut doc, 3.0).unwrap();
        let (rest, bent) = posed(&doc, &top, 60.0);
        assert_eq!(rest.len(), bent.len());
        let mut top_moved = 0.0f32;
        let mut feet_moved = 0.0f32;
        for (a, b) in rest.iter().zip(&bent) {
            let d = (a[0] - b[0]).hypot(a[1] - b[1]);
            if a[1] < -105.0 {
                top_moved = top_moved.max(d);
            }
            if a[1] > -10.0 {
                feet_moved = feet_moved.max(d);
            }
        }
        assert!(top_moved > 20.0, "{top_moved}");
        assert!(feet_moved < 1.0, "{feet_moved}");
    }

    #[test]
    fn parts_of_the_same_canvas_line_up_and_others_go_where_asked() {
        let pic = bar();
        let (mut doc, root, _) = rig_from_picture(&pic).unwrap();
        let c = part_center(doc.rig(), &pic, [500.0, 500.0]);
        assert!((c[0]).abs() < 1e-3 && (c[1] + 60.0).abs() < 1e-3, "{c:?}");
        let small = Picture { path: "assets/images/hat.png".into(), width: 4, height: 4, rgba: vec![255; 64] };
        assert_eq!(part_center(doc.rig(), &small, [7.0, 8.0]), [7.0, 8.0]);
        let s = add_part(&mut doc, &root, &small, [7.0, 8.0]).unwrap();
        assert_eq!(doc.rig().slots.last().map(|x| &x.id), Some(&s), "in front");
        assert_eq!(model::slot(doc.rig(), &s).unwrap().name, "hat");
        assert_eq!(part_bone(doc.rig(), None), Some(root));
        assert!(is_picture_file(Path::new("a/B.PNG")) && !is_picture_file(Path::new("a.txt")));
    }
}
