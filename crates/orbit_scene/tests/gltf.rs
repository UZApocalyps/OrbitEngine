use orbit_scene::import::{import_gltf, node_paths, reimport};
use orbit_scene::{AlphaMode, Node3dKind, validate_prefab};

/// Builds a tiny GLB: one triangle mesh with a textured material, a node
/// hierarchy `root/arm`, plus an animation and a camera (both ignored).
fn tiny_glb(arm_name: &str) -> Vec<u8> {
    let mut bin = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in v {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    for i in [0u16, 1, 2, 0] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let png_offset = bin.len();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[255, 0, 0, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    bin.extend_from_slice(&png);
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let json = format!(
        r#"{{
 "asset": {{"version": "2.0"}},
 "scene": 0,
 "scenes": [{{"nodes": [0]}}],
 "nodes": [
  {{"name": "root", "children": [1, 2], "translation": [1, 2, 3]}},
  {{"name": "{arm_name}", "mesh": 0}},
  {{"name": "cam", "camera": 0}}
 ],
 "cameras": [{{"type": "perspective", "perspective": {{"yfov": 0.7, "znear": 0.1}}}}],
 "meshes": [{{"name": "tri", "primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1, "material": 0}}]}}],
 "materials": [{{
   "name": "red",
   "pbrMetallicRoughness": {{"baseColorFactor": [1, 0.5, 0.25, 1], "metallicFactor": 0.0, "roughnessFactor": 0.4,
                             "baseColorTexture": {{"index": 0}}}},
   "alphaMode": "MASK", "alphaCutoff": 0.3, "doubleSided": true
 }}],
 "textures": [{{"source": 0}}],
 "images": [{{"bufferView": 2, "mimeType": "image/png"}}],
 "animations": [{{"channels": [{{"sampler": 0, "target": {{"node": 1, "path": "translation"}}}}],
                  "samplers": [{{"input": 3, "output": 4}}]}}],
 "buffers": [{{"byteLength": {len}}}],
 "bufferViews": [
  {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
  {{"buffer": 0, "byteOffset": 36, "byteLength": 6}},
  {{"buffer": 0, "byteOffset": {png_offset}, "byteLength": {png_len}}},
  {{"buffer": 0, "byteOffset": 0, "byteLength": 4}},
  {{"buffer": 0, "byteOffset": 4, "byteLength": 12}}
 ],
 "accessors": [
  {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}},
  {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}},
  {{"bufferView": 3, "componentType": 5126, "count": 1, "type": "SCALAR", "min": [0], "max": [0]}},
  {{"bufferView": 4, "componentType": 5126, "count": 1, "type": "VEC3"}}
 ]
}}"#,
        len = bin.len(),
        png_len = png.len(),
    );
    // Animation sampler accessors are 2 and 3 in the array above.
    let json = json.replace(r#""input": 3, "output": 4"#, r#""input": 2, "output": 3"#);
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

use image::ImageEncoder;

#[test]
fn imports_meshes_materials_textures_hierarchy() {
    let imp = import_gltf("tiny", &tiny_glb("arm")).unwrap();
    assert_eq!(imp.meshes.len(), 1);
    assert_eq!(imp.meshes[0].primitives[0].positions.len(), 3);
    assert_eq!(imp.meshes[0].primitives[0].indices, vec![0, 1, 2]);
    assert_eq!(imp.textures.len(), 1);
    assert_eq!(imp.textures[0].rgba8, vec![255, 0, 0, 255]);

    let m = &imp.prefab.materials[0];
    assert_eq!(m.name, "red");
    assert_eq!(m.alpha_mode, AlphaMode::Mask);
    assert!(m.double_sided);
    assert_eq!(m.roughness, 0.4);
    assert!(m.base_color_texture.is_some());

    assert_eq!(node_paths(&imp.prefab.nodes), ["root", "root/arm", "root/cam"]);
    assert_eq!(imp.prefab.nodes[0].transform.translation, [1.0, 2.0, 3.0]);
    assert!(matches!(imp.prefab.nodes[1].kind, Node3dKind::Mesh { .. }));
    assert!(validate_prefab(&imp.prefab).is_empty());

    let w = imp.warnings.join("|");
    assert!(w.contains("animation"), "{w}");
    assert!(w.contains("camera"), "{w}");
}

#[test]
fn reimport_preserves_ids_and_overrides() {
    let first = import_gltf("tiny", &tiny_glb("arm")).unwrap().prefab;
    let mut edited = first.clone();
    edited.nodes[1].visible = false;
    let mut over = edited.materials[0].clone();
    over.roughness = 0.9;
    edited.material_overrides.push(over);

    let mut second = import_gltf("tiny", &tiny_glb("arm")).unwrap().prefab;
    assert_ne!(second.nodes[1].id, edited.nodes[1].id);
    assert_eq!(reimport(&edited, &mut second), 3);
    for (a, b) in second.nodes.iter().zip(&edited.nodes) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.parent, b.parent);
    }
    assert!(!second.nodes[1].visible);
    assert_eq!(second.material_overrides[0].roughness, 0.9);

    // A renamed node is a new node; its siblings keep their ids.
    let mut renamed = import_gltf("tiny", &tiny_glb("arm2")).unwrap().prefab;
    assert_eq!(reimport(&edited, &mut renamed), 2);
    assert_ne!(renamed.nodes[1].id, edited.nodes[1].id);
    assert_eq!(renamed.nodes[0].id, edited.nodes[0].id);
    assert_eq!(renamed.nodes[1].parent, Some(edited.nodes[0].id));
}

#[test]
fn prefab_ron_roundtrip_and_garbage_rejected() {
    let p = import_gltf("tiny", &tiny_glb("arm")).unwrap().prefab;
    let back = orbit_scene::prefab_from_ron(&orbit_scene::prefab_to_ron(&p).unwrap()).unwrap();
    assert_eq!(back, p);
    assert!(import_gltf("bad", b"not a gltf").is_err());
}
