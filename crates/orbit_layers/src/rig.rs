//! Layered document -> rig: one slot and one alpha-traced mesh per raster
//! layer at its exact canvas position, draw order from the layer stack, an
//! optional bone per group, hidden layers kept as slots with no active
//! attachment (alternate parts: "eyes closed", "mouth open"...).
//!
//! Rig space: pixels, the origin is the canvas bottom-center, Y points down
//! (the same as a picture imported with `rig_create`).

use orbit_anim::rig::{BoneId, Rig};
use orbit_project::rigmesh::workflow::Picture;
use orbit_project::rigops::{self, BoneSpec};

use crate::{LayeredDoc, trim};

#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// One bone per group (named after it, parented like the groups), the
    /// layers of a group attached to its bone. Otherwise everything is on the
    /// first root bone.
    pub groups_as_bones: bool,
    /// Keep hidden layers (as slots with no active attachment).
    pub include_hidden: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self { groups_as_bones: false, include_hidden: true }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    /// Slot names created, bottom to top.
    pub slots: Vec<String>,
    /// Of those, the ones imported hidden.
    pub hidden: Vec<String>,
    /// Bone names created (including `root` when the rig had no bone).
    pub bones: Vec<String>,
    pub warnings: Vec<String>,
}

fn clean(name: &str) -> String {
    let n: String = name.chars().filter(|c| !c.is_control()).collect();
    let n = n.trim();
    if n.is_empty() { "layer".to_owned() } else { n.to_owned() }
}

/// Adds the layers of `doc` to `rig`. `save` writes one layer's PNG (given
/// its name, size and RGBA pixels, already trimmed) and returns the path the
/// rig stores for it. The rig is modified even on error: work on a copy.
pub fn import_into_rig(
    rig: &mut Rig,
    doc: &LayeredDoc,
    opts: &ImportOptions,
    save: &mut dyn FnMut(&str, u32, u32, &[u8]) -> Result<String, String>,
) -> Result<ImportReport, String> {
    let op = |e: rigops::OpError| e.message;
    let mut report = ImportReport { warnings: doc.warnings.clone(), ..ImportReport::default() };
    // Layers to import, trimmed to their pixels.
    let mut layers = Vec::new();
    for l in &doc.layers {
        let visible = doc.is_visible(l);
        if !visible && !opts.include_hidden {
            continue;
        }
        let mut l = l.clone();
        if !trim(&mut l) {
            report.warnings.push(format!("layer `{}` is fully transparent: skipped", l.name));
            continue;
        }
        layers.push((l, visible));
    }
    if layers.is_empty() {
        return Err("the file has no raster layer to import".into());
    }
    let root_bone: BoneId = match rig.bones.iter().find(|b| b.parent.is_none()) {
        Some(b) => b.id.clone(),
        None => {
            let len = (doc.height as f32 * 0.08).clamp(10.0, 80.0);
            let id = rigops::bone_add(
                rig,
                None,
                &BoneSpec { name: Some("root".into()), rotation: Some(-90.0), length: Some(len), ..BoneSpec::default() },
            )
            .map_err(op)?;
            report.bones.push("root".into());
            id
        }
    };
    let to_rig = |x: f32, y: f32| [x - doc.width as f32 * 0.5, y - doc.height as f32];
    // One bone per group that has an imported layer, parents first.
    let mut group_bone: Vec<Option<String>> = vec![None; doc.groups.len()];
    if opts.groups_as_bones {
        for g in 0..doc.groups.len() {
            let inside: Vec<_> = layers.iter().filter(|(l, _)| l.groups.contains(&g)).collect();
            if inside.is_empty() {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (l, _) in &inside {
                x0 = x0.min(l.x as f32);
                y0 = y0.min(l.y as f32);
                x1 = x1.max((l.x + l.width as i32) as f32);
                y1 = y1.max((l.y + l.height as i32) as f32);
            }
            let head = to_rig((x0 + x1) * 0.5, (y0 + y1) * 0.5);
            let parent = doc.groups[g].parent.and_then(|p| group_bone[p].clone()).unwrap_or_else(|| root_bone.to_string());
            let name = rigops::unique_name(&clean(&doc.groups[g].name), rig.bones.iter().map(|b| b.name.as_str()));
            let spec = BoneSpec {
                name: Some(name.clone()),
                x: Some(head[0]),
                y: Some(head[1]),
                rotation: Some(0.0),
                length: Some(((x1 - x0).min(y1 - y0) * 0.25).clamp(8.0, 60.0)),
                world: true,
                ..BoneSpec::default()
            };
            rigops::bone_add(rig, Some(&parent), &spec).map_err(op)?;
            report.bones.push(name.clone());
            group_bone[g] = Some(name);
        }
    }
    for (l, visible) in layers {
        let path = save(&l.name, l.width, l.height, &l.rgba)?;
        let pic = Picture { path, width: l.width, height: l.height, rgba: l.rgba };
        let bone = l.groups.iter().rev().find_map(|g| group_bone[*g].clone());
        let center = to_rig(l.x as f32 + l.width as f32 * 0.5, l.y as f32 + l.height as f32 * 0.5);
        let name = rigops::unique_name(&clean(&l.name), rig.slots.iter().map(|s| s.name.as_str()));
        let bone_key = bone.unwrap_or_else(|| root_bone.to_string());
        let (slot, _att) = rigops::add_image_part(rig, &pic, Some(&bone_key), Some(&name), Some(center), None).map_err(op)?;
        let s = rig.slots.iter_mut().find(|s| s.id == slot).ok_or("slot vanished")?;
        s.color.a = l.opacity;
        if !visible {
            // The mesh stays in the skin: a clip key or the editor can show it.
            s.attachment = None;
            report.hidden.push(name.clone());
        }
        report.slots.push(name);
    }
    Ok(report)
}
