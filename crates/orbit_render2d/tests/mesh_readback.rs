//! GPU readback tests of the mesh pipeline and rig drawing. Skipped (pass)
//! when no adapter is available (CI without GPU).

use std::collections::BTreeMap;

use orbit_anim::rig::{
    Attachment, AttachmentId, Bone, BoneId, CpuSkinner, MeshAttachment, PreparedRig, Rig, Skin, Slot, SlotId, Xform,
};
use orbit_render2d::orbit_gfx::Gfx;
use orbit_render2d::{
    Layer, Mesh2d, MeshVertex, ReadbackStatus, Renderer2d, RigPlacement, Scene, Sprite2d, TextureId, TextureOptions,
    push_rig,
};

const SIZE: u32 = 64;

fn renderer() -> Option<Renderer2d> {
    let gfx = match pollster::block_on(Gfx::headless((SIZE, SIZE))) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("no GPU adapter, skipping: {e}");
            return None;
        }
    };
    let mut r = Renderer2d::new(gfx).ok()?;
    r.set_virtual_size(SIZE as f32, SIZE as f32);
    Some(r)
}

fn solid(r: &mut Renderer2d, name: &str, rgba: [u8; 4]) -> TextureId {
    r.create_texture(name, 1, 1, &rgba, TextureOptions::COLOR).unwrap()
}

fn quad(tex: TextureId, x0: f32, y0: f32, x1: f32, y1: f32) -> Mesh2d {
    let v = |x, y, u, w| MeshVertex { position: [x, y], uv: [u, w], color: [1.0; 4] };
    Mesh2d::new(tex, vec![v(x0, y0, 0.0, 0.0), v(x1, y0, 1.0, 0.0), v(x0, y1, 0.0, 1.0), v(x1, y1, 1.0, 1.0)], vec![0, 1, 2, 2, 1, 3])
}

fn render(r: &mut Renderer2d, scene: &Scene) -> Vec<u8> {
    let target = r.create_render_target("t", SIZE, SIZE).unwrap();
    r.render_to_target(target, scene).unwrap();
    let mut rb = r.start_readback(target).unwrap();
    for _ in 0..5000 {
        match r.poll_readback(&mut rb) {
            ReadbackStatus::Pending => std::thread::sleep(std::time::Duration::from_millis(1)),
            ReadbackStatus::Ready { rgba, .. } => return rgba,
            ReadbackStatus::Failed => panic!("readback failed"),
        }
    }
    panic!("readback timed out");
}

fn px(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

fn is(c: [u8; 4], rgb: [bool; 3]) -> bool {
    (0..3).all(|k| if rgb[k] { c[k] > 200 } else { c[k] < 50 })
}

#[test]
fn meshes_draw_and_interleave_with_sprites() {
    let Some(mut r) = renderer() else { return };
    let red = solid(&mut r, "red", [255, 0, 0, 255]);
    let green = solid(&mut r, "green", [0, 255, 0, 255]);
    let blue = solid(&mut r, "blue", [0, 0, 255, 255]);
    let mut scene = Scene::new();
    scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
    scene.push_mesh(quad(red, 0.0, 0.0, 32.0, 64.0));
    scene.push(Sprite2d::new(green, [16.0, 0.0], [32.0, 32.0]).anchor([0.0, 0.0]));
    scene.push_mesh(quad(blue, 40.0, 0.0, 64.0, 16.0));
    // A mesh on a lower layer pushed last still goes under everything.
    scene.push_mesh(quad(blue, 0.0, 40.0, 64.0, 64.0).layer(Layer::BACKGROUND));
    let img = render(&mut r, &scene);
    assert_eq!(r.last_mesh_count(), 3);
    assert!(is(px(&img, 8, 20), [true, false, false]), "{:?}", px(&img, 8, 20));
    assert!(is(px(&img, 24, 8), [false, true, false]), "sprite over the first mesh");
    assert!(is(px(&img, 44, 4), [false, false, true]), "second mesh over the sprite");
    assert!(is(px(&img, 8, 50), [true, false, false]), "background mesh under the red one");
    assert!(is(px(&img, 56, 50), [false, false, true]));
    assert!(is(px(&img, 56, 30), [false, false, false]));
}

#[test]
fn posed_rig_mesh_is_skinned_on_screen() {
    let Some(mut r) = renderer() else { return };
    let red = solid(&mut r, "red", [255, 0, 0, 255]);
    let mut attachments = BTreeMap::new();
    attachments.insert(
        AttachmentId::new("strip"),
        Attachment::Mesh(MeshAttachment {
            image: "strip".into(),
            vertices: vec![[0.0, 0.0], [16.0, 0.0], [0.0, 4.0], [16.0, 4.0]],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            triangles: vec![[0, 1, 2], [2, 1, 3]],
            weights: vec![vec![(BoneId::new("arm"), 1.0)]; 4],
        }),
    );
    let rig = PreparedRig::new(Rig {
        bones: vec![Bone { id: BoneId::new("arm"), name: "arm".into(), parent: None, rest: Xform::IDENTITY, length: 16.0 }],
        slots: vec![Slot {
            id: SlotId::new("s"),
            name: "s".into(),
            bone: BoneId::new("arm"),
            attachment: Some(AttachmentId::new("strip")),
            color: orbit_anim::Rgba::new(1.0, 1.0, 1.0, 1.0),
        }],
        skins: vec![Skin { name: "default".into(), attachments }],
        ..Rig::default()
    })
    .unwrap();
    let mut pose = rig.setup_pose();
    pose.bones[0].rotation = 90.0;
    let world = rig.world(&pose);
    let list = rig.draw_list(&pose, &world, None, &CpuSkinner);
    let mut scene = Scene::new();
    scene.clear = Some([0.0, 0.0, 0.0, 1.0]);
    let placement = RigPlacement { position: [32.0, 16.0], scale: 2.0, ..Default::default() };
    assert_eq!(push_rig(&mut scene, &list, &placement, |_| Some(red)), 1);
    let img = render(&mut r, &scene);
    // Rotated 90° clockwise and scaled 2x: x in 24..32, y in 16..48.
    assert!(is(px(&img, 28, 40), [true, false, false]), "{:?}", px(&img, 28, 40));
    // Where the unposed strip would be: empty.
    assert!(is(px(&img, 50, 20), [false, false, false]));
}
