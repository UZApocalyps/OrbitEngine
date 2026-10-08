//! Rigged glTF import: skeleton, weights, animation clips, bone nodes.

use orbit_scene::import::import_gltf;
use orbit_scene::skin::skin_mesh;
use orbit_scene::{Node3dKind, omesh};

fn f32s(out: &mut Vec<u8>, v: &[f32]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

/// A triangle skinned to `hips` (two vertices) and `head` (the top one),
/// with a one second `nod` animation turning the head 90 degrees about Z.
fn rigged_glb() -> Vec<u8> {
    let mut bin = Vec::new();
    f32s(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 2.0, 0.0]); // 0..36
    for i in [0u16, 1, 2, 0] {
        bin.extend_from_slice(&i.to_le_bytes()); // 36..44
    }
    for j in [[0u16, 0, 0, 0], [0, 0, 0, 0], [1, 0, 0, 0]] {
        for x in j {
            bin.extend_from_slice(&x.to_le_bytes()); // 44..68
        }
    }
    for _ in 0..3 {
        f32s(&mut bin, &[1.0, 0.0, 0.0, 0.0]); // 68..116
    }
    let id = [
        1.0, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ];
    f32s(&mut bin, &id); // 116..180
    let mut down = id;
    down[13] = -1.0; // inverse of translate(0, 1, 0)
    f32s(&mut bin, &down); // 180..244
    f32s(&mut bin, &[0.0, 1.0]); // 244..252
    let h = std::f32::consts::FRAC_PI_4;
    f32s(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, h.sin(), h.cos()]); // 252..284
    let json = format!(
        r#"{{
 "asset": {{"version": "2.0"}},
 "scene": 0,
 "scenes": [{{"nodes": [0]}}],
 "nodes": [
  {{"name": "rig", "children": [1, 3]}},
  {{"name": "hips", "children": [2]}},
  {{"name": "head", "translation": [0, 1, 0]}},
  {{"name": "body", "mesh": 0, "skin": 0}}
 ],
 "skins": [{{"joints": [1, 2], "inverseBindMatrices": 4}}],
 "meshes": [{{"name": "body", "primitives": [{{"attributes": {{"POSITION": 0, "JOINTS_0": 2, "WEIGHTS_0": 3}}, "indices": 1}}]}}],
 "animations": [{{"name": "nod", "channels": [{{"sampler": 0, "target": {{"node": 2, "path": "rotation"}}}}],
                  "samplers": [{{"input": 5, "output": 6}}]}}],
 "buffers": [{{"byteLength": {len}}}],
 "bufferViews": [
  {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
  {{"buffer": 0, "byteOffset": 36, "byteLength": 6}},
  {{"buffer": 0, "byteOffset": 44, "byteLength": 24}},
  {{"buffer": 0, "byteOffset": 68, "byteLength": 48}},
  {{"buffer": 0, "byteOffset": 116, "byteLength": 128}},
  {{"buffer": 0, "byteOffset": 244, "byteLength": 8}},
  {{"buffer": 0, "byteOffset": 252, "byteLength": 32}}
 ],
 "accessors": [
  {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,2,0]}},
  {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}},
  {{"bufferView": 2, "componentType": 5123, "count": 3, "type": "VEC4"}},
  {{"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"}},
  {{"bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4"}},
  {{"bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]}},
  {{"bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC4"}}
 ]
}}"#,
        len = bin.len(),
    );
    let mut json = json.into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

#[test]
fn skins_and_animations_are_imported() {
    let imp = import_gltf("hero", &rigged_glb()).unwrap();
    assert!(
        !imp.warnings
            .iter()
            .any(|w| w.contains("skin") || w.contains("animation")),
        "{:?}",
        imp.warnings
    );
    // Bones are not scene nodes; the skinned mesh node is.
    let names: Vec<&str> = imp.prefab.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["rig", "body"]);
    assert!(matches!(imp.prefab.nodes[1].kind, Node3dKind::Mesh { .. }));

    let mesh = &imp.meshes[0];
    let skel = mesh.skin.as_ref().expect("skeleton");
    let joints: Vec<(&str, Option<usize>)> = skel
        .joints
        .iter()
        .map(|j| (j.name.as_str(), j.parent))
        .collect();
    assert_eq!(joints, [("hips", None), ("head", Some(0))]);
    assert_eq!(mesh.primitives[0].joints[2], [1, 0, 0, 0]);
    assert_eq!(mesh.clips.len(), 1);
    assert_eq!(mesh.clips[0].name, "nod");
    assert_eq!(mesh.clips[0].duration, 1.0);

    // Rest pose: unchanged; end of `nod`: the top vertex turned about the head.
    let rest = skin_mesh(mesh, &Default::default(), "r".into());
    assert!((rest.primitives[0].positions[2][1] - 2.0).abs() < 1e-5);
    let pose = mesh.clips[0].sample(skel, 1.0);
    let posed = skin_mesh(mesh, &pose, "p".into());
    let p = posed.primitives[0].positions[2];
    assert!(
        (p[0] + 1.0).abs() < 1e-4 && (p[1] - 1.0).abs() < 1e-4,
        "{p:?}"
    );
    assert_eq!(posed.primitives[0].positions[0], [0.0, 0.0, 0.0]);

    // The pak format keeps everything.
    assert_eq!(&omesh::decode("body", &omesh::encode(mesh)).unwrap(), mesh);
}
