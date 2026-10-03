//! Drawing a posed rig (ADR 0013 §5): meshes through the mesh pipeline,
//! rigid regions through the sprite batcher when their transform has no
//! shear (otherwise as a 4-vertex mesh), all in slot order on one layer/`z`.

use orbit_anim::rig::{Affine, RigDrawItem, RigDrawList};

use crate::sprite::{Layer, Mesh2d, MeshVertex, Scene, Sprite2d};
use crate::TextureId;

/// Where a rig lands on the stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RigPlacement {
    /// World position (virtual pixels) of the rig origin.
    pub position: [f32; 2],
    /// Uniform scale of rig units to virtual pixels.
    pub scale: f32,
    /// Vertical stretch on top of `scale` (1 = uniform).
    pub aspect: f32,
    /// Rotation (radians, clockwise on screen) around `position`.
    pub rotation: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub layer: Layer,
    pub z: f32,
    /// Linear RGBA multiplied with every part (fades, dimming).
    pub tint: [f32; 4],
}

impl Default for RigPlacement {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0],
            scale: 1.0,
            aspect: 1.0,
            rotation: 0.0,
            flip_x: false,
            flip_y: false,
            layer: Layer::CHARACTERS,
            z: 0.0,
            tint: [1.0; 4],
        }
    }
}

impl RigPlacement {
    /// Rig space → world.
    pub fn matrix(&self) -> Affine {
        let sx = if self.flip_x { -self.scale } else { self.scale };
        let sy = if self.flip_y { -self.scale } else { self.scale } * self.aspect;
        let (sin, cos) = self.rotation.sin_cos();
        Affine {
            a: cos * sx,
            b: sin * sx,
            c: -sin * sy,
            d: cos * sy,
            tx: self.position[0],
            ty: self.position[1],
        }
    }
}

fn mul_color(c: orbit_anim::Rgba, t: [f32; 4]) -> [f32; 4] {
    [c.r * t[0], c.g * t[1], c.b * t[2], c.a * t[3]]
}

/// Pushes the parts of `list` (from `PreparedRig::draw_list`) into `scene`,
/// back to front. `texture` maps an image reference to a loaded texture;
/// parts whose image is not loaded are skipped. Returns the number of parts
/// pushed.
pub fn push_rig(
    scene: &mut Scene,
    list: &RigDrawList<'_>,
    placement: &RigPlacement,
    mut texture: impl FnMut(&str) -> Option<TextureId>,
) -> usize {
    let stage = placement.matrix();
    let mut pushed = 0;
    for item in &list.items {
        match item {
            RigDrawItem::Region { image, size, transform, color, .. } => {
                let Some(tex) = texture(image) else { continue };
                let m = stage.mul(transform);
                let tint = mul_color(*color, placement.tint);
                let kx = libm_hypot(m.a, m.b);
                let ky = if kx > 0.0 { m.det() / kx } else { 0.0 };
                let shear = (m.a * m.c + m.b * m.d).abs();
                if kx > 0.0 && shear <= 1e-4 * kx * ky.abs().max(1e-6) {
                    scene.push(
                        Sprite2d::new(tex, [m.tx, m.ty], *size)
                            .anchor([0.5, 0.5])
                            .rotation(m.b.atan2(m.a))
                            .scale([kx, ky])
                            .layer(placement.layer)
                            .z(placement.z)
                            .tint(tint),
                    );
                } else {
                    let (w, h) = (size[0] * 0.5, size[1] * 0.5);
                    let corners = [([-w, -h], [0.0, 0.0]), ([w, -h], [1.0, 0.0]), ([-w, h], [0.0, 1.0]), ([w, h], [1.0, 1.0])];
                    let vertices = corners
                        .iter()
                        .map(|(p, uv)| MeshVertex { position: m.apply(*p), uv: *uv, color: tint })
                        .collect();
                    scene.push_mesh(
                        Mesh2d::new(tex, vertices, vec![0, 1, 2, 2, 1, 3]).layer(placement.layer).z(placement.z),
                    );
                }
                pushed += 1;
            }
            RigDrawItem::Mesh { image, range, uvs, triangles, color, .. } => {
                let Some(tex) = texture(image) else { continue };
                let tint = mul_color(*color, placement.tint);
                let positions = &list.positions[range.clone()];
                let vertices = positions
                    .iter()
                    .enumerate()
                    .map(|(i, p)| MeshVertex {
                        position: stage.apply(*p),
                        uv: uvs.get(i).copied().unwrap_or([0.0, 0.0]),
                        color: tint,
                    })
                    .collect();
                let indices = triangles.iter().flat_map(|t| t.iter().map(|&i| i as u32)).collect();
                scene.push_mesh(Mesh2d::new(tex, vertices, indices).layer(placement.layer).z(placement.z));
                pushed += 1;
            }
        }
    }
    pushed
}

fn libm_hypot(a: f32, b: f32) -> f32 {
    (a * a + b * b).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_anim::rig::{
        Attachment, AttachmentId, Bone, BoneId, CpuSkinner, MeshAttachment, PreparedRig, RegionAttachment, Rig, Skin,
        Slot, SlotId, Xform,
    };
    use std::collections::BTreeMap;

    fn rig() -> PreparedRig {
        let mut attachments = BTreeMap::new();
        attachments.insert(
            AttachmentId::new("body"),
            Attachment::Mesh(MeshAttachment {
                image: "body".into(),
                vertices: vec![[-5.0, -5.0], [5.0, -5.0], [-5.0, 5.0], [5.0, 5.0]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                triangles: vec![[0, 1, 2], [2, 1, 3]],
                weights: Vec::new(),
            }),
        );
        attachments.insert(
            AttachmentId::new("hat"),
            Attachment::Region(RegionAttachment { image: "hat".into(), size: [4.0, 2.0], xform: Xform::translate(0.0, -8.0) }),
        );
        let slot = |id: &str, att: &str| Slot {
            id: SlotId::new(id),
            name: id.into(),
            bone: BoneId::new("root"),
            attachment: Some(AttachmentId::new(att)),
            color: orbit_anim::Rgba::new(1.0, 1.0, 1.0, 1.0),
        };
        PreparedRig::new(Rig {
            bones: vec![Bone { id: BoneId::new("root"), name: "root".into(), parent: None, rest: Xform::IDENTITY, length: 0.0 }],
            slots: vec![slot("s_body", "body"), slot("s_hat", "hat")],
            skins: vec![Skin { name: "default".into(), attachments }],
            ..Rig::default()
        })
        .unwrap()
    }

    #[test]
    fn placement_scales_flips_and_orders_parts() {
        let rig = rig();
        let pose = rig.setup_pose();
        let world = rig.world(&pose);
        let list = rig.draw_list(&pose, &world, None, &CpuSkinner);
        let mut scene = Scene::new();
        let placement = RigPlacement { position: [100.0, 50.0], scale: 2.0, flip_x: true, ..Default::default() };
        let n = push_rig(&mut scene, &list, &placement, |name| Some(TextureId(if name == "body" { 1 } else { 2 })));
        assert_eq!(n, 2);
        let mesh = &scene.meshes()[0];
        // Flipped on X: the rig's left-top corner (-5, -5) lands right of the origin.
        assert_eq!(mesh.vertices[0].position, [110.0, 40.0]);
        assert_eq!(mesh.indices, vec![0, 1, 2, 2, 1, 3]);
        // The hat goes through the sprite batcher (no shear), mirrored.
        let hat = &scene.sprites()[0];
        assert_eq!(hat.position, [100.0, 34.0]);
        assert!((hat.scale[0].abs() - 2.0).abs() < 1e-5 && (hat.scale[1].abs() - 2.0).abs() < 1e-5);
        assert!(hat.scale[0] * hat.scale[1] < 0.0, "mirrored");
        // The hat (slot 1) was pushed after the body mesh.
        assert_eq!(scene.mesh_after(), &[0]);
        // Missing textures are skipped.
        let mut empty = Scene::new();
        assert_eq!(push_rig(&mut empty, &list, &placement, |_| None), 0);
    }
}
