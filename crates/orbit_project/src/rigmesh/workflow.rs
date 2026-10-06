//! The picture-to-rig workflow shared by the rig editor and the MCP `rig_*`
//! tools: a picture standing on the origin, its alpha-traced mesh, where an
//! extra part goes. Pure functions on [`Rig`]; decoding the picture is the
//! caller's business (this crate has no image codec).

use orbit_anim::rig::{Affine, Attachment, MeshAttachment, Rig, Xform};

use super::meshgen::{self, MeshParams};

/// A decoded picture and its path as the rig stores it.
#[derive(Clone, Debug)]
pub struct Picture {
    /// Project path (relative to the assets root, `/`) or absolute path.
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Picture {
    pub fn size(&self) -> [f32; 2] {
        [self.width.max(1) as f32, self.height.max(1) as f32]
    }

    /// File name without extension: the slot's name.
    pub fn stem(&self) -> String {
        let name = self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path);
        let stem = name.split('.').next().unwrap_or(name).trim();
        if stem.is_empty() { "picture".to_owned() } else { stem.to_owned() }
    }
}

/// Every mesh of the rig (any skin).
pub fn meshes(rig: &Rig) -> impl Iterator<Item = &MeshAttachment> {
    rig.skins.iter().flat_map(|s| s.attachments.values()).filter_map(|a| match a {
        Attachment::Mesh(m) => Some(m),
        Attachment::Region(_) => None,
    })
}

/// Rig-space center of a picture placed "standing on the origin": the rig
/// origin is the bottom-center of the character (what the game expects).
pub fn standing_center(pic: &Picture) -> [f32; 2] {
    [0.0, -pic.size()[1] * 0.5]
}

/// Length of the `root` bone of a rig made from `pic`.
pub fn root_length(pic: &Picture) -> f32 {
    (pic.size()[1] * 0.08).clamp(10.0, 80.0)
}

/// The mesh of `pic` traced from its alpha, at its pixel size, centered on
/// `center` (rig space). Falls back to the picture's rectangle when the
/// tracing fails.
pub fn mesh_for_picture(pic: &Picture, center: [f32; 2]) -> MeshAttachment {
    let placement = Xform::translate(center[0], center[1]).matrix();
    mesh_with_map(pic, &meshgen::region_uv_to_rig(&placement, pic.size()))
}

/// The mesh of `pic` traced from its alpha, its UVs (`0..1`) placed in rig
/// space by `uv_to_rig`. Falls back to the picture's rectangle when the
/// tracing fails (a blank picture: a quad to paint on).
pub fn mesh_with_map(pic: &Picture, uv_to_rig: &Affine) -> MeshAttachment {
    let uv_to_rig = *uv_to_rig;
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

/// Where a new part goes: aligned on the first picture when it has the same
/// pixel size (layers exported from one canvas), else centered on `fallback`.
pub fn part_center(rig: &Rig, pic: &Picture, fallback: [f32; 2]) -> [f32; 2] {
    let same = meshes(rig).find(|m| {
        meshgen::uv_map(m).and_then(|u| u.inverse()).is_some_and(|inv| {
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
