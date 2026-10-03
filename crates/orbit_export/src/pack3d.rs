//! The 3D content of the pak (ADR 0014, addendum "Blobs 3D dans le pak").
//!
//! For every shipped 3D scene `<dir>/<name>.scene3d.ron`, the models and
//! images of `<dir>/models/` are read (glTF with external files, PNG, JPEG,
//! KTX2) and **only what the scene uses** is written: meshes as `.omesh`,
//! prefabs as RON, textures as KTX2 RGBA8 + Zstandard, listed by
//! `3d/index.ron` (`orbit_scene::pack3d`). Procedural shapes are generated
//! by the player and not shipped.
//!
//! A game with such content is written with game pack version 4 (older
//! players refuse it by number instead of showing empty scenes); a 2D game,
//! or a 3D scene with only lights and cameras, keeps version 3 and gets no
//! `3d/` entry.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use orbit_core::{AssetKind, Story};
use orbit_scene::pack3d::{Assets3dIndex, INDEX_ENTRY, mesh_entry, prefab_entry, texture_entry};
use orbit_scene::{
    Background, MaterialDef, MeshAsset, Node3d, Node3dKind, Prefab, Scene3d, TextureAsset,
};

/// Nested prefab instances deeper than this are not followed (cycles).
const MAX_DEPTH: usize = 8;

/// Models and textures of one `models/` folder, fully decoded.
#[derive(Default)]
pub struct Library {
    pub meshes: HashMap<String, MeshAsset>,
    pub prefabs: HashMap<String, Prefab>,
    pub textures: HashMap<String, TextureAsset>,
}

/// Reads every model and image of `dir` (missing folder: empty library).
/// Returns the library and the files that could not be read.
pub fn load_library(dir: &Path) -> (Library, Vec<String>) {
    let mut lib = Library::default();
    let mut errors = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (lib, errors);
    };
    let mut files: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    for path in files {
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)
        else {
            continue;
        };
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model").to_owned();
        let result = match ext.as_str() {
            "glb" | "gltf" => orbit_scene::import::import_gltf_file(&path).map(|imp| {
                for m in imp.meshes {
                    lib.meshes.entry(m.name.clone()).or_insert(m);
                }
                for t in imp.textures {
                    lib.textures.entry(t.name.clone()).or_insert(t);
                }
                lib.prefabs.insert(imp.prefab.name.clone(), imp.prefab);
            }),
            "png" | "jpg" | "jpeg" | "ktx2" => decode_texture(&name, &path).map(|t| {
                // A standalone image wins over a glTF texture of the same
                // name, like in the editor (it is loaded after).
                lib.textures.insert(name.clone(), t);
            }),
            _ => continue,
        };
        if let Err(e) = result {
            errors.push(format!("{}: {e}", path.display()));
        }
    }
    (lib, errors)
}

fn decode_texture(name: &str, path: &Path) -> Result<TextureAsset, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if orbit_scene::ktx2::is_ktx2(&bytes) {
        return orbit_scene::ktx2::decode_rgba8(name, &bytes);
    }
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    Ok(TextureAsset {
        name: name.to_owned(),
        width: img.width(),
        height: img.height(),
        rgba8: img.to_rgba8().into_raw(),
    })
}

/// KTX2 RGBA8 (UNORM: the renderer picks sRGB or linear by use) with
/// Zstandard supercompression.
pub fn encode_texture(t: &TextureAsset) -> Vec<u8> {
    orbit_scene::ktx2::encode_simple(
        orbit_scene::ktx2::Format::R8G8B8A8_UNORM,
        t.width,
        t.height,
        &t.rgba8,
        Some(orbit_scene::ktx2::SupercompressionScheme::Zstandard),
    )
}

/// What a set of scenes references.
#[derive(Debug, Default, PartialEq)]
pub struct Usage {
    pub meshes: BTreeSet<String>,
    pub prefabs: BTreeSet<String>,
    pub textures: BTreeSet<String>,
    pub shapes: bool,
}

fn material_textures(m: &MaterialDef, out: &mut BTreeSet<String>) {
    for slot in [
        &m.base_color_texture,
        &m.orm_texture,
        &m.normal_texture,
        &m.emissive_texture,
    ]
    .into_iter()
    .flatten()
    {
        out.insert(slot.texture.0.clone());
    }
}

impl Usage {
    /// Adds what `scene` uses, following the prefabs found in `lib`.
    pub fn add_scene(&mut self, scene: &Scene3d, lib: &Library) {
        for m in &scene.materials {
            material_textures(m, &mut self.textures);
        }
        if let Background::Sky(t) = &scene.environment.background {
            self.textures.insert(t.0.clone());
        }
        self.add_nodes(&scene.nodes, lib, 0);
    }

    fn add_nodes(&mut self, nodes: &[Node3d], lib: &Library, depth: usize) {
        for node in nodes {
            match &node.kind {
                Node3dKind::Mesh { mesh, .. } => {
                    self.meshes.insert(mesh.0.clone());
                }
                Node3dKind::Shape { .. } => self.shapes = true,
                Node3dKind::Prefab(p) => {
                    if !self.prefabs.insert(p.0.clone()) || depth >= MAX_DEPTH {
                        continue;
                    }
                    if let Some(prefab) = lib.prefabs.get(&p.0) {
                        for m in prefab.materials.iter().chain(&prefab.material_overrides) {
                            material_textures(m, &mut self.textures);
                        }
                        self.add_nodes(&prefab.nodes, lib, depth + 1);
                    }
                }
                Node3dKind::Empty | Node3dKind::Light(_) | Node3dKind::Camera(_) => {}
            }
        }
    }
}

/// The `3d/` entries of a pak.
#[derive(Debug, Default)]
pub struct Content3d {
    /// `(entry path, bytes)`, index first. Empty: nothing to ship.
    pub entries: Vec<(String, Vec<u8>)>,
    /// Whether the game needs game pack version 4 (meshes, prefabs,
    /// textures or procedural shapes).
    pub needs_3d_pack: bool,
    pub warnings: Vec<String>,
}

/// Collects and encodes the 3D content used by the shipped scenes of
/// `story` (project at `root`).
pub fn build(root: &Path, story: &Story) -> Content3d {
    let mut out = Content3d::default();
    let mut usage = Usage::default();
    let mut merged = Library::default();
    let mut loaded: BTreeSet<std::path::PathBuf> = BTreeSet::new();
    for entry in story.assets.assets.iter().filter(|a| a.kind == AssetKind::Scene3d) {
        let file = root.join(&entry.path);
        let scene = match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|t| orbit_scene::from_ron(&t))
        {
            Ok(s) => s,
            Err(e) => {
                out.warnings.push(format!("3D scene `{}`: {e}", entry.path));
                continue;
            }
        };
        let models = file.parent().map(|d| d.join("models")).unwrap_or_default();
        if loaded.insert(models.clone()) {
            let (lib, errors) = load_library(&models);
            for e in errors {
                out.warnings.push(format!("3D scene `{}`: {e}", entry.path));
            }
            for (k, v) in lib.meshes {
                merged.meshes.entry(k).or_insert(v);
            }
            for (k, v) in lib.prefabs {
                merged.prefabs.entry(k).or_insert(v);
            }
            for (k, v) in lib.textures {
                merged.textures.entry(k).or_insert(v);
            }
        }
        usage.add_scene(&scene, &merged);
    }

    let mut index = Assets3dIndex::new();
    let mut blobs = Vec::new();
    let missing = |kind: &str, name: &str| format!("3D {kind} `{name}` is not in a models folder: not shipped");
    // Meshes used by the shipped prefabs too.
    let mut meshes: BTreeSet<String> = usage.meshes.clone();
    for p in &usage.prefabs {
        match merged.prefabs.get(p) {
            Some(prefab) => {
                let entry = prefab_entry(index.prefabs.len());
                match orbit_scene::prefab_to_ron(prefab) {
                    Ok(text) => {
                        blobs.push((entry.clone(), text.into_bytes()));
                        index.prefabs.insert(p.clone(), entry);
                    }
                    Err(e) => out.warnings.push(format!("3D prefab `{p}`: {e}")),
                }
                for n in &prefab.nodes {
                    if let Node3dKind::Mesh { mesh, .. } = &n.kind {
                        meshes.insert(mesh.0.clone());
                    }
                }
            }
            None => out.warnings.push(missing("model", p)),
        }
    }
    for name in &meshes {
        match merged.meshes.get(name) {
            Some(mesh) => {
                let entry = mesh_entry(index.meshes.len());
                blobs.push((entry.clone(), orbit_scene::omesh::encode(mesh)));
                index.meshes.insert(name.clone(), entry);
            }
            None => out.warnings.push(missing("mesh", name)),
        }
    }
    for name in &usage.textures {
        match merged.textures.get(name) {
            Some(t) => {
                let entry = texture_entry(index.textures.len());
                blobs.push((entry.clone(), encode_texture(t)));
                index.textures.insert(name.clone(), entry);
            }
            None => out.warnings.push(missing("texture", name)),
        }
    }

    out.needs_3d_pack = usage.shapes || !index.is_empty();
    if out.needs_3d_pack {
        match index.to_ron() {
            Ok(text) => out.entries.push((INDEX_ENTRY.to_owned(), text.into_bytes())),
            Err(e) => out.warnings.push(format!("3D index: {e}")),
        }
        out.entries.extend(blobs);
    }
    out
}

/// Test projects with a 3D scene (also used to regenerate the frozen pak).
#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use orbit_core::{AssetEntry, AssetId, AssetKind, Node, NodeId, NodeKind, Story};
    use orbit_pak::GamePack;
    use orbit_scene::pack3d::{Assets3dIndex, INDEX_ENTRY};
    use orbit_scene::{
        Background, CameraDef, MaterialDef, MaterialRef, Node3d, Node3dKind, PrefabRef, Projection,
        Scene3d, Shape,
    };

    use super::*;
    use crate::compile::compile;
    use crate::pakbuild::build_pak;
    use crate::report::Reporter;
    use crate::testing::{TempDir, fixture_project};

    pub(crate) const SCENE: &str = "assets/3d/room.scene3d.ron";

    /// `models/panel.gltf`: a 2x2 quad facing +Z (external `panel.bin`),
    /// unlit, textured with a 2x2 red PNG (`red.png`).
    fn write_panel(models: &Path) {
        std::fs::create_dir_all(models).unwrap();
        let mut bin = Vec::new();
        for v in [[-1.0f32, -1.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0]] {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        for uv in [[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]] {
            for c in uv {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        for i in [0u16, 1, 2, 0, 2, 3] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        std::fs::write(models.join("panel.bin"), &bin).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(models.join("red.png"), png.into_inner()).unwrap();
        let json = r#"{
 "asset": {"version": "2.0"},
 "extensionsUsed": ["KHR_materials_unlit"],
 "scene": 0, "scenes": [{"nodes": [0]}],
 "nodes": [{"name": "panel", "mesh": 0}],
 "meshes": [{"name": "panel", "primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "indices": 2, "material": 0}]}],
 "materials": [{"name": "red", "extensions": {"KHR_materials_unlit": {}},
   "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
 "textures": [{"source": 0}],
 "images": [{"uri": "red.png"}],
 "buffers": [{"uri": "panel.bin", "byteLength": 92}],
 "bufferViews": [
  {"buffer": 0, "byteOffset": 0, "byteLength": 48},
  {"buffer": 0, "byteOffset": 48, "byteLength": 32},
  {"buffer": 0, "byteOffset": 80, "byteLength": 12}
 ],
 "accessors": [
  {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1,-1,0], "max": [1,1,0]},
  {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC2"},
  {"bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR"}
 ]
}"#;
        std::fs::write(models.join("panel.gltf"), json).unwrap();
        // Not used by the scene: must not be shipped.
        std::fs::write(models.join("unused.png"), std::fs::read(models.join("red.png")).unwrap())
            .unwrap();
    }

    /// The scene: blue background, orthographic camera `main` (4 units
    /// high) at z = 5, the panel prefab at the centre and a green unlit cube
    /// (procedural) at x = 2.5.
    pub(crate) fn room() -> Scene3d {
        let mut scene = Scene3d::new("room");
        scene.environment.background = Background::Color([0.0, 0.0, 1.0]);
        let mut cam = Node3d::new(
            "cam",
            None,
            Node3dKind::Camera(CameraDef {
                name: "main".into(),
                projection: Projection::Orthographic { height: 4.0 },
                near: 0.1,
                far: 50.0,
            }),
        );
        cam.transform.translation = [0.0, 0.0, 5.0];
        scene.nodes.push(cam);
        scene.nodes.push(Node3d::new("panel", None, Node3dKind::Prefab(PrefabRef("panel".into()))));
        let mut green = MaterialDef::new("green");
        green.unlit = true;
        green.base_color = [0.0, 1.0, 0.0, 1.0];
        scene.materials.push(green);
        let mut cube = Node3d::new(
            "cube",
            None,
            Node3dKind::Shape {
                shape: Shape::Cube { size: [1.0; 3] },
                material: Some(MaterialRef("green".into())),
            },
        );
        cube.transform.translation = [2.5, 0.0, 0.0];
        scene.nodes.push(cube);
        scene
    }

    /// The test-minimal project with `room` shown at the start of its first
    /// scene.
    pub(crate) fn project_3d(dir: &Path) -> crate::project::ExportProject {
        fixture_project(dir);
        write_panel(&dir.join("assets/3d/models"));
        std::fs::write(dir.join(SCENE), orbit_scene::to_ron(&room()).unwrap()).unwrap();
        let path = dir.join("story.orbit");
        let mut story = Story::from_ron_unchecked(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let id = AssetId::new();
        story.assets.assets.push(AssetEntry {
            id,
            kind: AssetKind::Scene3d,
            path: SCENE.into(),
        });
        story.scenes[0].nodes.insert(
            0,
            Node {
                id: NodeId::new(),
                kind: NodeKind::ShowScene3d {
                    scene: id,
                    camera: Some("main".into()),
                    transition: None,
                },
            },
        );
        std::fs::write(&path, story.to_ron().unwrap()).unwrap();
        crate::project::ExportProject::open(dir).unwrap()
    }

    fn pak_of(project: &crate::project::ExportProject) -> (Vec<u8>, Vec<String>) {
        let profile = project.profile(None).unwrap();
        let compiled = compile(project, &profile).unwrap();
        let mut warnings = Vec::new();
        let content = build(&compiled.root, &compiled.pack.story);
        warnings.extend(content.warnings);
        let built = build_pak(&compiled, &profile, &mut Reporter::silent()).unwrap();
        (built.bytes, warnings)
    }

    #[test]
    fn a_3d_game_ships_its_used_models_and_a_version_4_pack() {
        let dir = TempDir::new("pak3d");
        let project = project_3d(dir.path());
        let (pak, warnings) = pak_of(&project);
        assert!(warnings.is_empty(), "{warnings:?}");
        let (_, index) = orbit_pak::container::parse_prefix(&pak).unwrap();
        let pack = index.read_from(&pak, "game.pack").unwrap();
        assert_eq!(GamePack::version_of(&pack), Some(4));
        assert!(GamePack::from_bytes(&pack).is_ok());

        let idx = Assets3dIndex::from_ron(
            &String::from_utf8(index.read_from(&pak, INDEX_ENTRY).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(idx.meshes.keys().collect::<Vec<_>>(), ["panel"]);
        assert_eq!(idx.prefabs.keys().collect::<Vec<_>>(), ["panel"]);
        // The glTF texture (unnamed: `texture0`) is shipped; the images
        // of the folder are not, the scene does not use them.
        assert_eq!(idx.textures.keys().collect::<Vec<_>>(), ["texture0"]);
        for entry in idx.entries() {
            assert!(index.contains(entry), "{entry}");
        }
        let mesh = orbit_scene::omesh::decode("panel", &index.read_from(&pak, &idx.meshes["panel"]).unwrap())
            .unwrap();
        assert_eq!(mesh.primitives[0].positions.len(), 4);
        let tex = orbit_scene::ktx2::decode_rgba8(
            "red",
            &index.read_from(&pak, &idx.textures["texture0"]).unwrap(),
        )
        .unwrap();
        assert_eq!(&tex.rgba8[..4], &[255, 0, 0, 255]);
        // Neither the glTF files nor the models folder are shipped raw.
        assert!(!index.entries().iter().any(|e| e.path.contains("models/")));
    }

    #[test]
    fn a_2d_game_keeps_version_3_and_no_3d_entries() {
        let dir = TempDir::new("pak2d_v3");
        let project = fixture_project(dir.path());
        let (pak, _) = pak_of(&project);
        let (_, index) = orbit_pak::container::parse_prefix(&pak).unwrap();
        let pack = index.read_from(&pak, "game.pack").unwrap();
        assert_eq!(GamePack::version_of(&pack), Some(3));
        assert!(!index.entries().iter().any(|e| e.path.starts_with("3d/")));
        assert_eq!(index.meta("quality"), None);
    }

    #[test]
    fn a_3d_scene_with_only_lights_stays_version_3() {
        let dir = TempDir::new("pak3d_lights");
        let project = project_3d(dir.path());
        let mut scene = room();
        scene.nodes.retain(|n| matches!(n.kind, Node3dKind::Camera(_)));
        std::fs::write(dir.path().join(SCENE), orbit_scene::to_ron(&scene).unwrap()).unwrap();
        let (pak, _) = pak_of(&project);
        let (_, index) = orbit_pak::container::parse_prefix(&pak).unwrap();
        let pack = index.read_from(&pak, "game.pack").unwrap();
        assert_eq!(GamePack::version_of(&pack), Some(3));
        assert!(!index.contains(INDEX_ENTRY));
    }

    #[test]
    fn missing_models_are_warnings() {
        let dir = TempDir::new("pak3d_missing");
        let project = project_3d(dir.path());
        std::fs::remove_file(dir.path().join("assets/3d/models/panel.gltf")).unwrap();
        let (pak, warnings) = pak_of(&project);
        assert!(warnings.iter().any(|w| w.contains("`panel`")), "{warnings:?}");
        // The shape still needs a version 4 pack.
        let (_, index) = orbit_pak::container::parse_prefix(&pak).unwrap();
        let pack = index.read_from(&pak, "game.pack").unwrap();
        assert_eq!(GamePack::version_of(&pack), Some(4));
    }

    /// Regenerates `examples/fixtures/compat/pak3d_v4/game.pak`, the frozen
    /// 3D pak read by `orbit_runtime` (`a_mesh_from_a_pak_is_drawn`): the
    /// export of [`project_3d`] stripped to the game pack, the scene and the
    /// `3d/` entries. Never regenerate once committed (plan §1.3).
    #[test]
    #[ignore = "writes the frozen fixture; run once by hand"]
    fn write_frozen_3d_pak() {
        let dir = TempDir::new("pak3d_fixture");
        let project = project_3d(dir.path());
        let (pak, _) = pak_of(&project);
        let (_, index) = orbit_pak::container::parse_prefix(&pak).unwrap();
        let mut writer = orbit_pak::PakWriter::new(orbit_pak::Compression::Lz4);
        for key in ["name", "id", "version", "quality"] {
            if let Some(v) = index.meta(key) {
                writer.meta(key, v);
            }
        }
        for entry in index.entries() {
            if entry.path == "game.pack" || entry.path == SCENE || entry.path.starts_with("3d/") {
                writer.add(&entry.path, &index.read_from(&pak, &entry.path).unwrap()).unwrap();
            }
        }
        let out = crate::testing::repo_root().join("examples/fixtures/compat/pak3d_v4");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("game.pak"), writer.to_bytes().0).unwrap();
    }
}
