//! Test fixtures for 3D scenes in stories: a skinned glTF model written to
//! disk and a 3D scene placing it in front of its camera.

use std::path::Path;

use orbit_scene::{CameraDef, Light, Node3d, Node3dKind, PrefabRef, Projection, Scene3d};

/// Writes `<dir>/<name>.gltf` (+ `<name>.bin`): a red 2×2 quad at the
/// origin facing +Z, **skinned** to one joint (`JOINTS_0` / `WEIGHTS_0`,
/// a skin with inverse bind matrices), like a rigged character.
pub fn write_skinned_gltf(dir: &Path, name: &str) {
    let mut bin: Vec<u8> = Vec::new();
    let f = |bin: &mut Vec<u8>, v: &[f32]| v.iter().for_each(|x| bin.extend(x.to_le_bytes()));
    // positions @0 (48)
    f(&mut bin, &[-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0]);
    // normals @48 (48)
    for _ in 0..4 {
        f(&mut bin, &[0.0, 0.0, 1.0]);
    }
    // joints @96 (16)
    bin.extend([0u8; 16]);
    // weights @112 (64)
    for _ in 0..4 {
        f(&mut bin, &[1.0, 0.0, 0.0, 0.0]);
    }
    // inverse bind matrix @176 (64): identity
    f(&mut bin, &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    // indices @240 (12)
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend(i.to_le_bytes());
    }
    assert_eq!(bin.len(), 252);
    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0, 1] }}],
  "nodes": [
    {{ "name": "body", "mesh": 0, "skin": 0 }},
    {{ "name": "root" }}
  ],
  "skins": [{{ "joints": [1], "inverseBindMatrices": 4 }}],
  "materials": [{{ "name": "red", "pbrMetallicRoughness": {{ "baseColorFactor": [1, 0, 0, 1], "metallicFactor": 0, "roughnessFactor": 1 }}, "emissiveFactor": [1, 0, 0] }}],
  "meshes": [{{ "name": "body", "primitives": [{{
    "attributes": {{ "POSITION": 0, "NORMAL": 1, "JOINTS_0": 2, "WEIGHTS_0": 3 }},
    "indices": 5, "material": 0 }}] }}],
  "buffers": [{{ "uri": "{name}.bin", "byteLength": 252 }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 48 }},
    {{ "buffer": 0, "byteOffset": 48, "byteLength": 48 }},
    {{ "buffer": 0, "byteOffset": 96, "byteLength": 16 }},
    {{ "buffer": 0, "byteOffset": 112, "byteLength": 64 }},
    {{ "buffer": 0, "byteOffset": 176, "byteLength": 64 }},
    {{ "buffer": 0, "byteOffset": 240, "byteLength": 12 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0] }},
    {{ "bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3" }},
    {{ "bufferView": 2, "componentType": 5121, "count": 4, "type": "VEC4" }},
    {{ "bufferView": 3, "componentType": 5126, "count": 4, "type": "VEC4" }},
    {{ "bufferView": 4, "componentType": 5126, "count": 1, "type": "MAT4" }},
    {{ "bufferView": 5, "componentType": 5123, "count": 6, "type": "SCALAR" }}
  ]
}}"#
    );
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{name}.bin")), bin).unwrap();
    std::fs::write(dir.join(format!("{name}.gltf")), json).unwrap();
}

/// A 3D scene: a sun, a camera `main` 4 m in front of the origin and the
/// model `model` (a prefab) at the origin.
pub fn scene_with_model(model: &str) -> Scene3d {
    let mut s = Scene3d::new("Hero room");
    s.environment.background = orbit_scene::Background::Color([0.0, 0.0, 0.5]);
    let mut sun = Node3d::new(
        "Sun",
        None,
        Node3dKind::Light(Light::Directional {
            color: [1.0, 1.0, 1.0],
            intensity: 3.0,
            shadows: false,
        }),
    );
    sun.transform.translation = [0.0, 4.0, 4.0];
    s.nodes.push(sun);
    let mut cam = Node3d::new(
        "Camera",
        None,
        Node3dKind::Camera(CameraDef {
            name: "main".into(),
            projection: Projection::Perspective { fov_y: 0.9 },
            near: 0.1,
            far: 100.0,
        }),
    );
    cam.transform.translation = [0.0, 0.0, 4.0];
    s.nodes.push(cam);
    s.nodes
        .push(Node3d::new(model, None, Node3dKind::Prefab(PrefabRef(model.into()))));
    s
}

/// Writes `scene` to `path` and its model `model` to `models/` next to it.
pub fn write_scene_with_models(path: &Path, scene: &Scene3d, model: &str) {
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(path, orbit_scene::to_ron(scene).unwrap()).unwrap();
    write_skinned_gltf(&dir.join("models"), model);
}
