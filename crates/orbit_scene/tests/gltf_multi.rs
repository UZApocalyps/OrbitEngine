//! Multi-file glTF import (external `.bin` and images, percent-encoded URIs,
//! KHR_materials_pbrSpecularGlossiness), like a Sketchfab `.gltf` export.

use image::ImageEncoder;
use orbit_scene::import::{
    external_files, import_gltf, import_gltf_file, import_gltf_in, relocate_gltf,
    safe_relative_uri,
};
use orbit_scene::validate_prefab;

/// Writes `scene.gltf`, an external `scene.bin`, `textures/base color.png`
/// (space: percent-encoded URI) and a spec-gloss material (extension
/// required) into `dir`.
fn write_multi_file(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("textures")).unwrap();
    let mut bin = Vec::new();
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in v {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    for i in [0u16, 1, 2, 0] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    std::fs::write(dir.join("scene.bin"), &bin).unwrap();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[0, 255, 0, 255, 0, 0, 255, 255], 2, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    std::fs::write(dir.join("textures/base color.png"), &png).unwrap();
    let json = format!(
        r#"{{
 "asset": {{"version": "2.0"}},
 "extensionsUsed": ["KHR_materials_pbrSpecularGlossiness"],
 "extensionsRequired": ["KHR_materials_pbrSpecularGlossiness"],
 "scene": 0,
 "scenes": [{{"nodes": [0]}}],
 "nodes": [{{"name": "body", "mesh": 0}}],
 "meshes": [{{"name": "tri", "primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1, "material": 0}}]}}],
 "materials": [{{
   "name": "skin",
   "extensions": {{"KHR_materials_pbrSpecularGlossiness": {{
     "diffuseFactor": [1, 0.5, 0.5, 1], "diffuseTexture": {{"index": 0}},
     "specularFactor": [1, 1, 1], "glossinessFactor": 0.25
   }}}}
 }}],
 "textures": [{{"source": 0}}],
 "images": [{{"uri": "textures/base%20color.png"}}],
 "buffers": [{{"uri": "scene.bin", "byteLength": {len}}}],
 "bufferViews": [
  {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
  {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}
 ],
 "accessors": [
  {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]}},
  {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
 ]
}}"#,
        len = bin.len()
    );
    let path = dir.join("scene.gltf");
    std::fs::write(&path, json).unwrap();
    path
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("orbit_gltf_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_multi_file_spec_gloss_model_imports_from_its_folder() {
    let dir = temp_dir("multi");
    let path = write_multi_file(&dir);
    let bytes = std::fs::read(&path).unwrap();
    // From bytes alone the external files cannot be found: clear message.
    let err = import_gltf("scene", &bytes).unwrap_err();
    assert!(err.contains("external file"), "{err}");
    assert_eq!(
        external_files(&bytes).unwrap(),
        vec!["scene.bin".to_owned(), "textures/base color.png".to_owned()]
    );

    let imp = import_gltf_file(&path).unwrap();
    assert_eq!(imp.meshes.len(), 1);
    assert_eq!(imp.textures.len(), 1);
    assert_eq!((imp.textures[0].width, imp.textures[0].height), (2, 1));
    let m = &imp.prefab.materials[0];
    assert_eq!(m.base_color, [1.0, 0.5, 0.5, 1.0]);
    assert_eq!(m.metallic, 0.0);
    assert!((m.roughness - 0.75).abs() < 1e-6);
    assert!(m.base_color_texture.is_some());
    assert!(
        imp.warnings.iter().any(|w| w.contains("specular-glossiness")),
        "{:?}",
        imp.warnings
    );
    assert!(validate_prefab(&imp.prefab).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_external_file_is_a_clear_error() {
    let dir = temp_dir("missing");
    let path = write_multi_file(&dir);
    std::fs::remove_file(dir.join("scene.bin")).unwrap();
    let err = import_gltf_file(&path).unwrap_err();
    assert!(err.contains("missing file `scene.bin`"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn relocated_models_keep_importing_and_never_escape_their_folder() {
    let dir = temp_dir("relocate");
    let path = write_multi_file(&dir.join("src"));
    let (json, moved) = relocate_gltf(&std::fs::read(&path).unwrap(), "girl.files").unwrap();
    assert_eq!(moved[0], ("scene.bin".to_owned(), "girl.files/scene.bin".to_owned()));
    // Copy like the editor does: models/girl.gltf + models/girl.files/...
    let models = dir.join("models");
    for (from, to) in &moved {
        let to = models.join(to);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(dir.join("src").join(from), to).unwrap();
    }
    let imp = import_gltf_in("girl", &json, &models).unwrap();
    assert_eq!(imp.textures.len(), 1);

    assert_eq!(safe_relative_uri("a/./b%20c.png").unwrap(), "a/b c.png");
    assert_eq!(safe_relative_uri("a/../b.bin").unwrap(), "b.bin");
    for bad in ["../x.bin", "a/../../x", "/etc/passwd", "C:/x.bin", "file:///x", "a\\b", "%2e%2e/x"] {
        assert!(safe_relative_uri(bad).is_err(), "{bad}");
    }
    // A model pointing outside its folder is refused before reading.
    let evil = String::from_utf8(json)
        .unwrap()
        .replace("girl.files/scene.bin", "../secret.bin");
    let err = import_gltf_in("x", evil.as_bytes(), &models).unwrap_err();
    assert!(err.contains("outside"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Checks a real model when `ORBIT_GLTF_CHECK` points at a `.gltf` / `.glb`
/// (not in the repository): prints its counts and warnings.
#[test]
fn optional_real_model_check() {
    let Ok(path) = std::env::var("ORBIT_GLTF_CHECK") else {
        return;
    };
    let imp = import_gltf_file(std::path::Path::new(&path)).unwrap();
    let triangles: usize = imp
        .meshes
        .iter()
        .flat_map(|m| &m.primitives)
        .map(|p| p.indices.len() / 3)
        .sum();
    println!(
        "meshes {} / materials {} / textures {} / nodes {} / triangles {triangles}",
        imp.meshes.len(),
        imp.prefab.materials.len(),
        imp.textures.len(),
        imp.prefab.nodes.len()
    );
    for w in &imp.warnings {
        println!("warning: {w}");
    }
    assert!(validate_prefab(&imp.prefab).is_empty());
}
