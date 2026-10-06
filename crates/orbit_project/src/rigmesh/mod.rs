//! Rig meshes from pictures and skin weights (ADR 0013, M5B-04 / M5B-05):
//! alpha outline tracing, triangulation (`spade`), manual mesh edits, weight
//! painting and automatic weights. Pure functions, no GUI and no GPU: the
//! editor's rig tools and the MCP `rig_mesh_auto` tool share them.

use orbit_anim::rig::{Affine, BoneId, Rig, Xform};

pub mod meshgen;
pub mod weights;
pub mod workflow;

/// World matrices of `locals` (one per bone, bone order = parents first).
pub(crate) fn world_matrices(rig: &Rig, locals: &[Xform]) -> Vec<Affine> {
    let index: std::collections::HashMap<&str, usize> =
        rig.bones.iter().enumerate().map(|(i, b)| (b.id.as_str(), i)).collect();
    let mut out: Vec<Affine> = Vec::with_capacity(rig.bones.len());
    for (i, b) in rig.bones.iter().enumerate() {
        let m = locals.get(i).copied().unwrap_or(b.rest).matrix();
        let w = match b.parent.as_ref().and_then(|p| index.get(p.as_str())) {
            Some(&p) if p < out.len() => out[p].mul(&m),
            _ => m,
        };
        out.push(w);
    }
    out
}

/// The rest transform of every bone.
pub(crate) fn rest_locals(rig: &Rig) -> Vec<Xform> {
    rig.bones.iter().map(|b| b.rest).collect()
}

pub(crate) fn bone_index(rig: &Rig, id: &BoneId) -> Option<usize> {
    rig.bones.iter().position(|b| &b.id == id)
}
