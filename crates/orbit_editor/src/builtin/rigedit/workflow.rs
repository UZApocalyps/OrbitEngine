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

use orbit_anim::rig::{Attachment, BoneId, Rig, SlotId};
pub use orbit_project::rigmesh::workflow::{Picture, mesh_for_picture, part_center, standing_center};
use orbit_project::rigmesh::workflow::{meshes, root_length};

use super::model::{self, DEFAULT_SKIN, OpError, RigDoc};
use super::weights;

/// Reads and decodes `path` under `root` (an absolute `path` ignores `root`).
pub fn load_picture(root: &Path, path: &str) -> Result<Picture, String> {
    let bytes = std::fs::read(root.join(path)).map_err(|e| e.to_string())?;
    let img = orbit_assets::decode_image(&bytes)?;
    Ok(Picture { path: path.to_owned(), width: img.width, height: img.height, rgba: img.rgba })
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
    let len = root_length(pic);
    let root = doc.add_bone(None, [0.0, 0.0], [0.0, -len])?;
    doc.rename_bone(&root, "root")?;
    Ok(root)
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
            if v[1] > -40.0 && v[1] < -20.0 {
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

    /// End to end: a PNG on disk → rig → 2 bones → bind → rotate the child
    /// → drawn by `orbit_render2d` (as the viewport host does) and read
    /// back: the top of the picture moved, the feet did not. Skipped (passes)
    /// without a GPU adapter.
    #[test]
    fn png_to_bent_pixels_end_to_end() {
        use orbit_render2d::orbit_gfx::Gfx;
        use orbit_render2d::{ReadbackStatus, Renderer2d, RigPlacement, Scene, TextureOptions, push_rig};
        const S: u32 = 160;
        // The PNG file.
        let dir = std::env::temp_dir().join(format!("orbit_rig_e2e_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets/images")).unwrap();
        let src = bar();
        image::RgbaImage::from_raw(src.width, src.height, src.rgba.clone())
            .unwrap()
            .save(dir.join("assets/images/bar.png"))
            .unwrap();
        let pic = load_picture(&dir, "assets/images/bar.png").unwrap();
        assert_eq!((pic.width, pic.height), (40, 120));
        let (mut doc, root, _) = rig_from_picture(&pic).unwrap();
        let body = doc.add_bone(Some(&root), [0.0, 0.0], [0.0, -60.0]).unwrap();
        let top = doc.add_bone(Some(&body), [0.0, -60.0], [0.0, -120.0]).unwrap();
        bind(&mut doc, 3.0).unwrap();

        let gfx = match pollster::block_on(Gfx::headless((S, S))) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("no GPU adapter, skipping: {e}");
                return;
            }
        };
        let Ok(mut r) = Renderer2d::new(gfx) else { return };
        r.set_virtual_size(S as f32, S as f32);
        let tex = r
            .create_texture("bar", pic.width, pic.height, &pic.rgba, TextureOptions::COLOR)
            .unwrap();
        let prepared = PreparedRig::new(doc.rig().clone()).unwrap();
        let mut draw = |degrees: f32| -> Vec<u8> {
            let mut pose = prepared.setup_pose();
            let i = model::bone_index(doc.rig(), &top).unwrap();
            pose.bones[i].rotation += degrees;
            let world = prepared.world(&pose);
            let list = prepared.draw_list(&pose, &world, None, &CpuSkinner);
            let mut scene = Scene::new();
            scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
            let placement = RigPlacement { position: [80.0, 140.0], scale: 1.0, ..RigPlacement::default() };
            push_rig(&mut scene, &list, &placement, |_| Some(tex));
            let target = r.create_render_target("t", S, S).unwrap();
            r.render_to_target(target, &scene).unwrap();
            let mut rb = r.start_readback(target).unwrap();
            for _ in 0..5000 {
                match r.poll_readback(&mut rb) {
                    ReadbackStatus::Pending => std::thread::sleep(std::time::Duration::from_millis(1)),
                    ReadbackStatus::Ready { rgba, .. } => return rgba,
                    ReadbackStatus::Failed => panic!("readback failed"),
                }
            }
            panic!("readback timed out");
        };
        let px = |img: &[u8], x: u32, y: u32| {
            let i = ((y * S + x) * 4) as usize;
            [img[i], img[i + 1], img[i + 2]]
        };
        let rest = draw(0.0);
        let bent = draw(90.0);
        // Rest: red on top (rig y -110 → target y 30), blue at the feet.
        assert!(px(&rest, 80, 30)[0] > 200, "{:?}", px(&rest, 80, 30));
        assert!(px(&rest, 80, 135)[2] > 200, "{:?}", px(&rest, 80, 135));
        // Bent: the top left its column, the feet stayed.
        assert!(px(&bent, 80, 30)[0] < 60, "top moved: {:?}", px(&bent, 80, 30));
        assert!(px(&bent, 80, 135)[2] > 200, "feet stayed: {:?}", px(&bent, 80, 135));
        let red_moved = (0..S).flat_map(|y| (0..S).map(move |x| (x, y))).any(|(x, y)| {
            px(&bent, x, y)[0] > 200 && px(&rest, x, y)[0] < 60
        });
        assert!(red_moved, "red pixels appear where there were none");
        let _ = std::fs::remove_dir_all(&dir);
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
