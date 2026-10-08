//! MCP tools for 3D scenes (M6-14): list, read, create, edit, replace and
//! validate the `.scene3d.ron` assets of the project (nodes, transforms,
//! lights, cameras, materials, environment), as `orbit_scene` types.
//!
//! A 3D scene is an asset of the manifest (`AssetKind::Scene3d`) whose file
//! is not part of the story, so there is no `orbit_project` command for its
//! content: like the `ui/` tools, these tools read and write the file
//! directly. Every write is validated first (`orbit_scene::validate`): an
//! edit that would leave an *error* in the scene is refused and nothing is
//! written (an edit is applied as a whole or not at all). Creating a scene
//! declares its asset through the command API, as one undo entry.
//!
//! Rendering needs a GPU: the picture of a scene (`capture_scene3d`) is a
//! live tool of the editor (see `live_tools`); the headless server has no
//! renderer and only reports the structure.

use std::path::{Path, PathBuf};

use orbit_core::{AssetId, AssetKind, Node3dId, Story};
use orbit_scene::{
    AlphaMode, Background, CameraDef, Environment, Fog, Light, MaterialDef, MaterialRef, MeshRef,
    Node3d, Node3dKind, PrefabRef, Projection, Scene3d, Severity, Shape, TextureRef, TextureSlot,
    Transform, shapes::ShapeParams,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::convert::find_asset;
use crate::error::ToolError;
use crate::tools::ToolCx;

fn out(value: Json) -> Result<String, ToolError> {
    Ok(serde_json::to_string_pretty(&value).unwrap_or_default())
}

// ------------------------------------------------------------------ files

/// A scene asset resolved to its entry and file.
struct SceneFile {
    id: AssetId,
    rel: String,
    abs: PathBuf,
}

fn resolve(cx: &ToolCx<'_>, key: &str) -> Result<SceneFile, ToolError> {
    let story = cx.project.story();
    // `room` also names `room.scene3d.ron` (its stem is `room.scene3d`).
    let id = find_asset(story, key, Some(AssetKind::Scene3d))
        .or_else(|first| {
            find_asset(story, &format!("{key}.scene3d"), Some(AssetKind::Scene3d))
                .map_err(|_| first)
        })?;
    let rel = story
        .assets
        .get(id)
        .map(|a| a.path.clone())
        .ok_or_else(|| ToolError::not_found(format!("no 3D scene `{key}`")))?;
    let abs = cx
        .sandbox
        .resolve(&rel)
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    Ok(SceneFile { id, rel, abs })
}

fn load(file: &SceneFile) -> Result<Scene3d, ToolError> {
    let text = std::fs::read_to_string(&file.abs).map_err(|e| {
        ToolError::new(
            crate::ErrorKind::Io,
            format!("cannot read `{}`: {e}", file.rel),
        )
    })?;
    orbit_scene::from_ron(&text)
        .map_err(|e| ToolError::invalid(format!("`{}` is not a valid 3D scene: {e}", file.rel)))
}

/// Writes `scene` (temporary file then rename).
fn store(abs: &Path, scene: &Scene3d) -> Result<(), ToolError> {
    let io = |e: std::io::Error| ToolError::new(crate::ErrorKind::Io, e.to_string());
    let text = orbit_scene::to_ron(scene).map_err(ToolError::invalid)?;
    if let Some(dir) = abs.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = abs.with_extension("ron.tmp");
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::rename(&tmp, abs).map_err(io)
}

// ------------------------------------------------------------ validation

/// Issues of the scene as JSON (structural validation plus the references
/// that can be checked against the files next to the scene).
pub fn issues_json(scene: &Scene3d, dir: Option<&Path>) -> Vec<Json> {
    let mut issues: Vec<Json> = orbit_scene::validate(scene)
        .iter()
        .map(|i| {
            json!({
                "severity": if i.severity == Severity::Error { "error" } else { "warning" },
                "path": i.path, "message": i.message,
            })
        })
        .collect();
    if scene.cameras().next().is_none() {
        issues.push(json!({
            "severity": "warning", "path": "",
            "message": "the scene has no camera: a story cannot show it (add a Camera node)",
        }));
    }
    if let Some(dir) = dir {
        let known = model_stems(dir);
        let warn = |path: &str, message: String| {
            json!({"severity": "warning", "path": path, "message": message})
        };
        for n in &scene.nodes {
            if let Node3dKind::Prefab(p) = &n.kind
                && !known.contains(&p.0)
            {
                issues.push(warn(
                    &n.name,
                    format!("prefab `{}`: no model `{}.glb/.gltf` in `models/` next to the scene", p.0, p.0),
                ));
            }
        }
        for m in &scene.materials {
            for (slot, t) in material_slots(m) {
                if let Some(s) = t
                    && !known.contains(&s.texture.0)
                {
                    issues.push(warn(
                        &m.name,
                        format!(
                            "{slot} texture `{}`: no image `{}.png/.jpg` in `models/` next to the scene (ignore if it comes from a glTF model)",
                            s.texture.0, s.texture.0
                        ),
                    ));
                }
            }
        }
        if let Background::Sky(t) = &scene.environment.background
            && !known.contains(&t.0)
        {
            issues.push(warn(
                "environment",
                format!("sky texture `{}`: no image `{}.png/.jpg` in `models/` next to the scene", t.0, t.0),
            ));
        }
    }
    issues
}

fn material_slots(m: &MaterialDef) -> [(&'static str, &Option<TextureSlot>); 4] {
    [
        ("base_color", &m.base_color_texture),
        ("orm", &m.orm_texture),
        ("normal", &m.normal_texture),
        ("emissive", &m.emissive_texture),
    ]
}

/// File stems of the models and images of `<dir>/models`.
fn model_stems(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir.join("models")) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?.to_ascii_lowercase();
            matches!(ext.as_str(), "glb" | "gltf" | "png" | "jpg" | "jpeg")
                .then(|| p.file_stem()?.to_str().map(str::to_owned))
                .flatten()
        })
        .collect()
}

fn errors_of(issues: &[Json]) -> Vec<String> {
    issues
        .iter()
        .filter(|i| i["severity"] == "error")
        .map(|i| format!("{}: {}", i["path"].as_str().unwrap_or(""), i["message"].as_str().unwrap_or("")))
        .collect()
}

// ----------------------------------------------------------------- views

fn f(x: f32) -> Json {
    json!((f64::from(x) * 1e6).round() / 1e6)
}

fn node_json(scene: &Scene3d, n: &Node3d) -> Json {
    let kind = match &n.kind {
        Node3dKind::Empty => json!({"type": "empty"}),
        Node3dKind::Mesh { mesh, materials } => json!({
            "type": "mesh", "mesh": mesh.0,
            "materials": materials.iter().map(|m| &m.0).collect::<Vec<_>>(),
        }),
        Node3dKind::Prefab(p) => json!({"type": "prefab", "prefab": p.0}),
        Node3dKind::Shape { shape, material } => {
            let mut v = serde_json::to_value(shape.params()).unwrap_or_default();
            if let Some(map) = v.as_object_mut() {
                map.retain(|_, x| !x.is_null());
                map.insert("type".into(), json!("shape"));
                map.insert("shape".into(), json!(shape.kind()));
                map.insert("material".into(), json!(material.as_ref().map(|m| &m.0)));
            }
            v
        }
        Node3dKind::Light(Light::Directional { color, intensity, shadows }) => json!({
            "type": "directional_light", "color": color.map(f), "intensity": f(*intensity), "shadows": shadows,
        }),
        Node3dKind::Light(Light::Point { color, intensity, range }) => json!({
            "type": "point_light", "color": color.map(f), "intensity": f(*intensity), "range": range,
        }),
        Node3dKind::Light(Light::Spot { color, intensity, range, inner_angle, outer_angle }) => json!({
            "type": "spot_light", "color": color.map(f), "intensity": f(*intensity), "range": range,
            "inner_angle_deg": f(inner_angle.to_degrees()), "outer_angle_deg": f(outer_angle.to_degrees()),
        }),
        Node3dKind::Camera(c) => camera_json(c),
    };
    let parent = n
        .parent
        .and_then(|p| scene.nodes.iter().find(|x| x.id == p))
        .map(|p| p.name.clone());
    json!({
        "id": n.id.to_string(), "name": n.name, "parent": parent, "visible": n.visible,
        "translation": n.transform.translation.map(f),
        "rotation_euler_deg": euler_deg(n.transform.rotation).map(f),
        "rotation": n.transform.rotation.map(f),
        "scale": n.transform.scale.map(f),
        "kind": kind,
    })
}

fn camera_json(c: &CameraDef) -> Json {
    let mut v = json!({"type": "camera", "camera_name": c.name, "near": f(c.near), "far": f(c.far)});
    match c.projection {
        Projection::Perspective { fov_y } => v["fov_y_deg"] = f(fov_y.to_degrees()),
        Projection::Orthographic { height } => v["orthographic_height"] = f(height),
    }
    v
}

fn material_json(m: &MaterialDef) -> Json {
    let tex = |t: &Option<TextureSlot>| t.as_ref().map(|s| s.texture.0.clone());
    json!({
        "name": m.name, "unlit": m.unlit, "base_color": m.base_color.map(f),
        "metallic": f(m.metallic), "roughness": f(m.roughness),
        "emissive": m.emissive.map(f), "emissive_strength": f(m.emissive_strength),
        "normal_scale": f(m.normal_scale), "occlusion_strength": f(m.occlusion_strength),
        "alpha_mode": match m.alpha_mode { AlphaMode::Opaque => "opaque", AlphaMode::Mask => "mask", AlphaMode::Blend => "blend" },
        "alpha_cutoff": f(m.alpha_cutoff), "double_sided": m.double_sided,
        "base_color_texture": tex(&m.base_color_texture), "orm_texture": tex(&m.orm_texture),
        "normal_texture": tex(&m.normal_texture), "emissive_texture": tex(&m.emissive_texture),
    })
}

fn environment_json(e: &Environment) -> Json {
    let fog = match &e.fog {
        None => json!({"kind": "none"}),
        Some(Fog::Linear { color, start, end }) => {
            json!({"kind": "linear", "color": color.map(f), "start": f(*start), "end": f(*end)})
        }
        Some(Fog::Exponential { color, density }) => {
            json!({"kind": "exponential", "color": color.map(f), "density": f(*density)})
        }
    };
    let background = match &e.background {
        Background::Color(c) => json!({"color": c.map(f)}),
        Background::Sky(t) => json!({"sky_texture": t.0}),
    };
    json!({
        "background": background, "ambient_color": e.ambient_color.map(f),
        "ambient_intensity": f(e.ambient_intensity), "fog": fog,
    })
}

fn scene_json(scene: &Scene3d) -> Json {
    let rigs: Vec<Json> = scene
        .rigs
        .iter()
        .map(|r| {
            let node = scene.nodes.iter().find(|n| n.id == r.node).map(|n| n.name.clone());
            json!({
                "node": node,
                "posed_bones": r.pose.joints.iter().map(|j| json!({
                    "joint": j.joint,
                    "rotation_euler_deg": euler_deg(j.transform.rotation).map(f),
                    "translation": j.transform.translation.map(f),
                })).collect::<Vec<_>>(),
                "saved_poses": r.poses.iter().map(|p| &p.name).collect::<Vec<_>>(),
                "clips": r.clips.iter().map(|c| json!({"name": c.name, "keys": c.keys.len(), "looping": c.looping})).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "name": scene.name,
        "rigs": rigs,
        "nodes": scene.nodes.iter().map(|n| node_json(scene, n)).collect::<Vec<_>>(),
        "cameras": scene.cameras().map(|(n, c)| json!({"node": n.name, "camera_name": c.name})).collect::<Vec<_>>(),
        "materials": scene.materials.iter().map(material_json).collect::<Vec<_>>(),
        "environment": environment_json(&scene.environment),
    })
}

// --------------------------------------------------------- math helpers

/// Quaternion `[x, y, z, w]` of Euler angles in degrees (the editor's
/// convention, see the 3D editor inspector).
pub fn quat_from_euler_deg(e: [f32; 3]) -> [f32; 4] {
    let (sx, cx) = (e[0].to_radians() * 0.5).sin_cos();
    let (sy, cy) = (e[1].to_radians() * 0.5).sin_cos();
    let (sz, cz) = (e[2].to_radians() * 0.5).sin_cos();
    normalize([
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ])
}

fn normalize(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-9 || !n.is_finite() {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        q.map(|x| x / n)
    }
}

/// Inverse of [`quat_from_euler_deg`] (degrees), for display.
pub fn euler_deg(q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = normalize(q);
    let sinr = 2.0 * (w * x + y * z);
    let cosr = 1.0 - 2.0 * (x * x + y * y);
    let sinp = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
    let siny = 2.0 * (w * z + x * y);
    let cosy = 1.0 - 2.0 * (y * y + z * z);
    [
        sinr.atan2(cosr).to_degrees(),
        sinp.asin().to_degrees(),
        siny.atan2(cosy).to_degrees(),
    ]
}

// ------------------------------------------------------------------ args

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneArg {
    /// The 3D scene: asset id, manifest path, file name or name without
    /// extension (see `list_scenes3d`).
    pub scene: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateArgs {
    /// Name of the scene (also the default file name).
    pub name: String,
    /// Manifest path of the new file (default `assets/3d/<name>.scene3d.ron`).
    /// It must end with `.ron`, lie under `assets/` and not exist yet.
    pub path: Option<String>,
    /// Start with a sun (directional light) and a camera named `main`
    /// (default true). With false the scene is empty.
    pub with_defaults: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteArgs {
    /// The 3D scene (asset id, path, file name or name).
    pub scene: String,
    /// The whole scene as RON (`Scene3d`, as returned by `read_scene3d`'s
    /// `ron`). Replaces the file; refused when it has errors.
    pub ron: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    /// The 3D scene (asset id, path, file name or name).
    pub scene: String,
    /// The edits, applied in order. All of them or none: if one fails or the
    /// result has errors, nothing is written.
    pub ops: Vec<Op>,
}

/// A node of the scene, by id (26-character ULID) or by name (must be
/// unique).
type NodeRef = String;

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeSpec {
    /// A transform-only node (a group).
    Empty,
    /// An imported mesh with one material name per primitive.
    Mesh {
        mesh: String,
        #[serde(default)]
        materials: Vec<String>,
    },
    /// An instance of an imported glTF model (its file stem).
    Prefab { prefab: String },
    /// A procedural primitive, generated by the engine (no file needed).
    /// Omitted parameters keep the kind's default (about 1 m).
    Shape {
        /// `cube`, `sphere`, `plane`, `cylinder`, `cone`, `capsule` or `torus`.
        shape: String,
        /// Cube: x, y, z sizes. Plane: x and z sizes (`[x, 0, z]`).
        size: Option<[f32; 3]>,
        /// Sphere, cylinder, cone, capsule; torus: distance to the tube centre.
        radius: Option<f32>,
        /// Cylinder, cone, capsule (total height).
        height: Option<f32>,
        /// Torus tube radius.
        tube: Option<f32>,
        /// Divisions around the axis (3..=256).
        segments: Option<u32>,
        /// Sphere / capsule divisions from pole to pole.
        rings: Option<u32>,
        /// Torus divisions around the tube.
        sides: Option<u32>,
        /// Plane grid divisions.
        subdivisions: Option<u32>,
        /// Material name of the scene; omit for the default white.
        material: Option<String>,
    },
    DirectionalLight {
        /// Linear RGB, default white.
        color: Option<[f32; 3]>,
        /// Default 3.
        intensity: Option<f32>,
        /// Casts shadows (default true).
        shadows: Option<bool>,
    },
    PointLight {
        color: Option<[f32; 3]>,
        /// Default 10.
        intensity: Option<f32>,
        /// Influence radius; omit for unlimited.
        range: Option<f32>,
    },
    SpotLight {
        color: Option<[f32; 3]>,
        /// Default 10.
        intensity: Option<f32>,
        range: Option<f32>,
        /// Default 20.
        inner_angle_deg: Option<f32>,
        /// Default 35.
        outer_angle_deg: Option<f32>,
    },
    /// A named camera: the unit stories use (`show_scene3d camera`).
    Camera {
        /// Unique name in the scene (default: the node name).
        camera_name: Option<String>,
        /// Vertical field of view, perspective (default 50).
        fov_y_deg: Option<f32>,
        /// Set for an orthographic camera (height of the view).
        orthographic_height: Option<f32>,
        near: Option<f32>,
        far: Option<f32>,
    },
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct FogSpec {
    /// `none`, `linear` or `exponential`.
    pub kind: String,
    pub color: Option<[f32; 3]>,
    /// Linear fog start distance.
    pub start: Option<f32>,
    /// Linear fog end distance.
    pub end: Option<f32>,
    /// Exponential fog density.
    pub density: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Add a node (its id is returned).
    AddNode {
        name: String,
        /// Parent node (id or name); omit for the root.
        parent: Option<NodeRef>,
        kind: NodeSpec,
        translation: Option<[f32; 3]>,
        /// Euler angles in degrees (X, Y, Z).
        rotation_euler_deg: Option<[f32; 3]>,
        /// Quaternion `[x, y, z, w]` (alternative to the Euler angles).
        rotation: Option<[f32; 4]>,
        scale: Option<[f32; 3]>,
        visible: Option<bool>,
    },
    /// Remove a node and its descendants.
    RemoveNode { node: NodeRef },
    RenameNode { node: NodeRef, name: String },
    /// Move a node under another one (omit `parent` for the root); its local
    /// transform is kept.
    SetParent { node: NodeRef, parent: Option<NodeRef> },
    SetVisible { node: NodeRef, visible: bool },
    /// Change the local transform (only the given fields).
    SetTransform {
        node: NodeRef,
        translation: Option<[f32; 3]>,
        rotation_euler_deg: Option<[f32; 3]>,
        rotation: Option<[f32; 4]>,
        scale: Option<[f32; 3]>,
    },
    /// Change a light (only the given fields; the node must be a light).
    SetLight {
        node: NodeRef,
        color: Option<[f32; 3]>,
        intensity: Option<f32>,
        shadows: Option<bool>,
        range: Option<f32>,
        inner_angle_deg: Option<f32>,
        outer_angle_deg: Option<f32>,
    },
    /// Change a camera (only the given fields; the node must be a camera).
    SetCamera {
        node: NodeRef,
        camera_name: Option<String>,
        fov_y_deg: Option<f32>,
        orthographic_height: Option<f32>,
        near: Option<f32>,
        far: Option<f32>,
    },
    /// Set the materials of a mesh node, one name per primitive.
    SetMeshMaterials { node: NodeRef, materials: Vec<String> },
    /// Change a shape node (only the given fields; the node must be a
    /// shape). A new `shape` kind starts from that kind's defaults. A
    /// parameter the kind does not use is an error. `material`: a material
    /// name, or an empty string for the default white.
    SetShape {
        node: NodeRef,
        shape: Option<String>,
        size: Option<[f32; 3]>,
        radius: Option<f32>,
        height: Option<f32>,
        tube: Option<f32>,
        segments: Option<u32>,
        rings: Option<u32>,
        sides: Option<u32>,
        subdivisions: Option<u32>,
        material: Option<String>,
    },
    /// Create or change a material (only the given fields). Texture fields
    /// take a texture name, or an empty string to clear the slot.
    SetMaterial {
        name: String,
        unlit: Option<bool>,
        base_color: Option<[f32; 4]>,
        metallic: Option<f32>,
        roughness: Option<f32>,
        emissive: Option<[f32; 3]>,
        emissive_strength: Option<f32>,
        normal_scale: Option<f32>,
        occlusion_strength: Option<f32>,
        /// `opaque`, `mask` or `blend`.
        alpha_mode: Option<String>,
        alpha_cutoff: Option<f32>,
        double_sided: Option<bool>,
        base_color_texture: Option<String>,
        orm_texture: Option<String>,
        normal_texture: Option<String>,
        emissive_texture: Option<String>,
    },
    /// Remove a material (meshes using it keep the name: validation reports
    /// them).
    RemoveMaterial { name: String },
    /// Change the environment (only the given fields).
    SetEnvironment {
        /// Plain background colour (linear RGB).
        background_color: Option<[f32; 3]>,
        /// Skybox texture name (cubemap or equirectangular image).
        sky_texture: Option<String>,
        ambient_color: Option<[f32; 3]>,
        ambient_intensity: Option<f32>,
        fog: Option<FogSpec>,
    },
    /// Pose one bone of a rigged character (a prefab instance or mesh node
    /// whose glTF has a skin). Only the given fields change; the others keep
    /// the current pose (the bone's rest transform the first time). Bone
    /// names: `read_scene3d` (`rigs`) or the model's glTF joints.
    SetBonePose {
        node: NodeRef,
        joint: String,
        /// Euler angles in degrees, relative to the parent bone.
        rotation_euler_deg: Option<[f32; 3]>,
        rotation: Option<[f32; 4]>,
        translation: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
    },
    /// Back to the rest pose: one bone, or the whole character without
    /// `joint`.
    ResetPose { node: NodeRef, joint: Option<String> },
    /// Store the current pose of a character under a name (replaces one of
    /// the same name).
    SavePose { node: NodeRef, name: String },
    /// Make a saved pose the current pose.
    ApplyPose { node: NodeRef, name: String },
    RemovePose { node: NodeRef, name: String },
}

// ------------------------------------------------------------------- ops

fn find_node(scene: &Scene3d, key: &str) -> Result<usize, ToolError> {
    if key.len() == 26
        && let Ok(id) = key.parse::<Node3dId>()
        && let Some(i) = scene.nodes.iter().position(|n| n.id == id)
    {
        return Ok(i);
    }
    let named: Vec<usize> = scene
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.name == key)
        .map(|(i, _)| i)
        .collect();
    match named.as_slice() {
        [one] => Ok(*one),
        [] => Err(ToolError::not_found(format!(
            "no node `{key}` in the scene (nodes: {})",
            scene
                .nodes
                .iter()
                .map(|n| format!("`{}`", n.name))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        many => Err(ToolError::invalid(format!(
            "node name `{key}` is ambiguous ({} nodes): use the id",
            many.len()
        ))),
    }
}

fn descendants(scene: &Scene3d, root: Node3dId) -> Vec<Node3dId> {
    let mut found = vec![root];
    let mut i = 0;
    while i < found.len() {
        let parent = found[i];
        for n in &scene.nodes {
            if n.parent == Some(parent) && !found.contains(&n.id) {
                found.push(n.id);
            }
        }
        i += 1;
    }
    found
}

fn finite_all(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

fn check_finite(what: &str, v: &[f32]) -> Result<(), ToolError> {
    if finite_all(v) {
        Ok(())
    } else {
        Err(ToolError::invalid(format!("{what} must be finite numbers")))
    }
}

fn apply_transform(
    t: &mut Transform,
    translation: Option<[f32; 3]>,
    euler: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    scale: Option<[f32; 3]>,
) -> Result<(), ToolError> {
    if euler.is_some() && rotation.is_some() {
        return Err(ToolError::invalid(
            "give either `rotation_euler_deg` or `rotation`, not both",
        ));
    }
    if let Some(v) = translation {
        check_finite("translation", &v)?;
        t.translation = v;
    }
    if let Some(e) = euler {
        check_finite("rotation_euler_deg", &e)?;
        t.rotation = quat_from_euler_deg(e);
    }
    if let Some(q) = rotation {
        check_finite("rotation", &q)?;
        t.rotation = normalize(q);
    }
    if let Some(s) = scale {
        check_finite("scale", &s)?;
        t.scale = s;
    }
    Ok(())
}

fn camera_from(
    base: CameraDef,
    name: Option<String>,
    fov: Option<f32>,
    ortho: Option<f32>,
    near: Option<f32>,
    far: Option<f32>,
) -> Result<CameraDef, ToolError> {
    let mut c = base;
    if let Some(n) = name {
        if n.trim().is_empty() {
            return Err(ToolError::invalid("camera_name must not be empty"));
        }
        c.name = n;
    }
    if fov.is_some() && ortho.is_some() {
        return Err(ToolError::invalid(
            "give either `fov_y_deg` (perspective) or `orthographic_height`, not both",
        ));
    }
    if let Some(f) = fov {
        check_finite("fov_y_deg", &[f])?;
        if !(1.0..=170.0).contains(&f) {
            return Err(ToolError::invalid("fov_y_deg must be between 1 and 170"));
        }
        c.projection = Projection::Perspective {
            fov_y: f.to_radians(),
        };
    }
    if let Some(h) = ortho {
        check_finite("orthographic_height", &[h])?;
        if h <= 0.0 {
            return Err(ToolError::invalid("orthographic_height must be positive"));
        }
        c.projection = Projection::Orthographic { height: h };
    }
    if let Some(n) = near {
        check_finite("near", &[n])?;
        c.near = n;
    }
    if let Some(x) = far {
        check_finite("far", &[x])?;
        c.far = x;
    }
    if c.near <= 0.0 || c.far <= c.near {
        return Err(ToolError::invalid("camera needs 0 < near < far"));
    }
    Ok(c)
}

fn default_camera() -> CameraDef {
    CameraDef {
        name: "main".into(),
        projection: Projection::Perspective {
            fov_y: 50f32.to_radians(),
        },
        near: 0.1,
        far: 200.0,
    }
}

fn shape_kind(kind: &str) -> Result<Shape, ToolError> {
    Shape::default_of(kind).ok_or_else(|| {
        ToolError::invalid(format!(
            "unknown shape `{kind}` (expected one of: {})",
            orbit_scene::shapes::SHAPE_KINDS.join(", ")
        ))
    })
}

fn build_kind(spec: NodeSpec, node_name: &str) -> Result<Node3dKind, ToolError> {
    Ok(match spec {
        NodeSpec::Empty => Node3dKind::Empty,
        NodeSpec::Mesh { mesh, materials } => Node3dKind::Mesh {
            mesh: MeshRef(mesh),
            materials: materials.into_iter().map(MaterialRef).collect(),
        },
        NodeSpec::Prefab { prefab } => Node3dKind::Prefab(PrefabRef(prefab)),
        NodeSpec::Shape {
            shape,
            size,
            radius,
            height,
            tube,
            segments,
            rings,
            sides,
            subdivisions,
            material,
        } => {
            let mut s = shape_kind(&shape)?;
            let params = ShapeParams { size, radius, height, tube, segments, rings, sides, subdivisions };
            s.apply(&params)
                .map_err(|e| ToolError::invalid(format!("node `{node_name}`: {e}")))?;
            Node3dKind::Shape {
                shape: s,
                material: material.filter(|m| !m.is_empty()).map(MaterialRef),
            }
        }
        NodeSpec::DirectionalLight {
            color,
            intensity,
            shadows,
        } => Node3dKind::Light(Light::Directional {
            color: color.unwrap_or([1.0; 3]),
            intensity: intensity.unwrap_or(3.0),
            shadows: shadows.unwrap_or(true),
        }),
        NodeSpec::PointLight {
            color,
            intensity,
            range,
        } => Node3dKind::Light(Light::Point {
            color: color.unwrap_or([1.0; 3]),
            intensity: intensity.unwrap_or(10.0),
            range,
        }),
        NodeSpec::SpotLight {
            color,
            intensity,
            range,
            inner_angle_deg,
            outer_angle_deg,
        } => Node3dKind::Light(Light::Spot {
            color: color.unwrap_or([1.0; 3]),
            intensity: intensity.unwrap_or(10.0),
            range,
            inner_angle: inner_angle_deg.unwrap_or(20.0).to_radians(),
            outer_angle: outer_angle_deg.unwrap_or(35.0).to_radians(),
        }),
        NodeSpec::Camera {
            camera_name,
            fov_y_deg,
            orthographic_height,
            near,
            far,
        } => {
            let mut base = default_camera();
            base.name = node_name.to_owned();
            Node3dKind::Camera(camera_from(
                base,
                camera_name,
                fov_y_deg,
                orthographic_height,
                near,
                far,
            )?)
        }
    })
}

fn texture_slot(slot: &mut Option<TextureSlot>, name: Option<String>) {
    match name {
        None => {}
        Some(n) if n.is_empty() => *slot = None,
        Some(n) => match slot {
            Some(s) => s.texture = TextureRef(n),
            None => *slot = Some(TextureSlot::new(TextureRef(n))),
        },
    }
}

/// Applies one edit. Returns the id of the node it created, if any.
#[cfg(test)]
fn apply(scene: &mut Scene3d, op: Op) -> Result<Option<(String, String)>, ToolError> {
    apply_in(scene, op, None)
}

/// The skeleton posed by node `i`: the first skinned mesh of its model,
/// imported from `<dir>/models`.
fn skeleton_of(
    scene: &Scene3d,
    i: usize,
    dir: Option<&Path>,
) -> Result<orbit_scene::skin::Skeleton, ToolError> {
    let node = &scene.nodes[i];
    let models = dir
        .map(|d| d.join("models"))
        .ok_or_else(|| ToolError::invalid("the models folder of the scene is unknown"))?;
    let candidates: Vec<PathBuf> = match &node.kind {
        Node3dKind::Prefab(p) => ["glb", "gltf"]
            .iter()
            .map(|e| models.join(format!("{}.{e}", p.0)))
            .filter(|p| p.is_file())
            .collect(),
        Node3dKind::Mesh { .. } => model_stems(dir.unwrap_or(Path::new(".")))
            .into_iter()
            .flat_map(|s| ["glb", "gltf"].map(|e| models.join(format!("{s}.{e}"))))
            .filter(|p| p.is_file())
            .collect(),
        _ => Vec::new(),
    };
    let want_mesh = match &node.kind {
        Node3dKind::Mesh { mesh, .. } => Some(mesh.0.clone()),
        _ => None,
    };
    for path in candidates {
        let Ok(imp) = orbit_scene::import::import_gltf_file(&path) else {
            continue;
        };
        if let Some(skel) = imp
            .meshes
            .into_iter()
            .filter(|m| want_mesh.as_ref().is_none_or(|w| &m.name == w))
            .find_map(|m| m.skin)
        {
            return Ok(skel);
        }
    }
    Err(ToolError::invalid(format!(
        "node `{}` has no rigged (skinned) model",
        node.name
    )))
}

fn apply_in(
    scene: &mut Scene3d,
    op: Op,
    dir: Option<&Path>,
) -> Result<Option<(String, String)>, ToolError> {
    match op {
        Op::SetBonePose {
            node,
            joint,
            rotation_euler_deg,
            rotation,
            translation,
            scale,
        } => {
            let i = find_node(scene, &node)?;
            let id = scene.nodes[i].id;
            let base = match scene.rig(id).and_then(|r| r.pose.get(&joint)) {
                Some(t) => *t,
                None => {
                    let skel = skeleton_of(scene, i, dir)?;
                    let j = skel.index_of(&joint).ok_or_else(|| {
                        let names: Vec<&str> = skel.joints.iter().map(|j| j.name.as_str()).collect();
                        ToolError::not_found(format!(
                            "no bone `{joint}` (bones: {})",
                            names.join(", ")
                        ))
                    })?;
                    skel.joints[j].rest
                }
            };
            let mut t = base;
            apply_transform(&mut t, translation, rotation_euler_deg, rotation, scale)?;
            scene.rig_mut(id).pose.set(&joint, t);
        }
        Op::ResetPose { node, joint } => {
            let id = scene.nodes[find_node(scene, &node)?].id;
            let rig = scene.rig_mut(id);
            match joint {
                Some(j) => rig.pose.reset(&j),
                None => rig.pose = Default::default(),
            }
        }
        Op::SavePose { node, name } => {
            if name.trim().is_empty() {
                return Err(ToolError::invalid("a pose needs a name"));
            }
            let id = scene.nodes[find_node(scene, &node)?].id;
            let rig = scene.rig_mut(id);
            let pose = rig.pose.clone();
            match rig.poses.iter_mut().find(|p| p.name == name) {
                Some(p) => p.pose = pose,
                None => rig.poses.push(orbit_scene::NamedPose { name, pose }),
            }
        }
        Op::ApplyPose { node, name } => {
            let id = scene.nodes[find_node(scene, &node)?].id;
            let rig = scene.rig_mut(id);
            let pose = rig
                .poses
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.pose.clone())
                .ok_or_else(|| ToolError::not_found(format!("no saved pose `{name}`")))?;
            rig.pose = pose;
        }
        Op::RemovePose { node, name } => {
            let id = scene.nodes[find_node(scene, &node)?].id;
            let rig = scene.rig_mut(id);
            let before = rig.poses.len();
            rig.poses.retain(|p| p.name != name);
            if rig.poses.len() == before {
                return Err(ToolError::not_found(format!("no saved pose `{name}`")));
            }
        }
        op => return apply_scene_op(scene, op),
    }
    // Rigs of removed nodes and empty rigs are not kept.
    scene.rigs.retain(|r| !r.pose.is_rest() || !r.poses.is_empty() || !r.clips.is_empty());
    Ok(None)
}

fn apply_scene_op(scene: &mut Scene3d, op: Op) -> Result<Option<(String, String)>, ToolError> {
    match op {
        Op::SetBonePose { .. }
        | Op::ResetPose { .. }
        | Op::SavePose { .. }
        | Op::ApplyPose { .. }
        | Op::RemovePose { .. } => unreachable!("pose ops are handled by apply_in"),
        Op::AddNode {
            name,
            parent,
            kind,
            translation,
            rotation_euler_deg,
            rotation,
            scale,
            visible,
        } => {
            if name.trim().is_empty() {
                return Err(ToolError::invalid("a node needs a name"));
            }
            let parent = match parent {
                Some(p) => Some(scene.nodes[find_node(scene, &p)?].id),
                None => None,
            };
            let mut node = Node3d::new(name.clone(), parent, build_kind(kind, &name)?);
            apply_transform(&mut node.transform, translation, rotation_euler_deg, rotation, scale)?;
            node.visible = visible.unwrap_or(true);
            let id = node.id.to_string();
            scene.nodes.push(node);
            return Ok(Some((id, name)));
        }
        Op::RemoveNode { node } => {
            let id = scene.nodes[find_node(scene, &node)?].id;
            let gone = descendants(scene, id);
            scene.nodes.retain(|n| !gone.contains(&n.id));
        }
        Op::RenameNode { node, name } => {
            if name.trim().is_empty() {
                return Err(ToolError::invalid("a node needs a name"));
            }
            let i = find_node(scene, &node)?;
            scene.nodes[i].name = name;
        }
        Op::SetParent { node, parent } => {
            let i = find_node(scene, &node)?;
            let new_parent = match parent {
                Some(p) => Some(scene.nodes[find_node(scene, &p)?].id),
                None => None,
            };
            if let Some(p) = new_parent
                && descendants(scene, scene.nodes[i].id).contains(&p)
            {
                return Err(ToolError::refused(
                    "a node cannot become a child of itself or of one of its descendants",
                ));
            }
            scene.nodes[i].parent = new_parent;
        }
        Op::SetVisible { node, visible } => {
            let i = find_node(scene, &node)?;
            scene.nodes[i].visible = visible;
        }
        Op::SetTransform {
            node,
            translation,
            rotation_euler_deg,
            rotation,
            scale,
        } => {
            let i = find_node(scene, &node)?;
            apply_transform(
                &mut scene.nodes[i].transform,
                translation,
                rotation_euler_deg,
                rotation,
                scale,
            )?;
        }
        Op::SetLight {
            node,
            color,
            intensity,
            shadows,
            range,
            inner_angle_deg,
            outer_angle_deg,
        } => {
            let i = find_node(scene, &node)?;
            let name = scene.nodes[i].name.clone();
            let Node3dKind::Light(light) = &mut scene.nodes[i].kind else {
                return Err(ToolError::invalid(format!("node `{name}` is not a light")));
            };
            for v in [intensity, range, inner_angle_deg, outer_angle_deg].into_iter().flatten() {
                check_finite("light value", &[v])?;
            }
            if let Some(c) = color {
                check_finite("color", &c)?;
            }
            match light {
                Light::Directional {
                    color: c,
                    intensity: i,
                    shadows: s,
                } => {
                    if range.is_some() || inner_angle_deg.is_some() || outer_angle_deg.is_some() {
                        return Err(ToolError::invalid(
                            "a directional light has no range or cone angles",
                        ));
                    }
                    *c = color.unwrap_or(*c);
                    *i = intensity.unwrap_or(*i);
                    *s = shadows.unwrap_or(*s);
                }
                Light::Point {
                    color: c,
                    intensity: i,
                    range: r,
                } => {
                    if shadows.is_some() || inner_angle_deg.is_some() || outer_angle_deg.is_some() {
                        return Err(ToolError::invalid(
                            "a point light has no shadows flag or cone angles",
                        ));
                    }
                    *c = color.unwrap_or(*c);
                    *i = intensity.unwrap_or(*i);
                    *r = range.or(*r);
                }
                Light::Spot {
                    color: c,
                    intensity: i,
                    range: r,
                    inner_angle,
                    outer_angle,
                } => {
                    if shadows.is_some() {
                        return Err(ToolError::invalid("spot light shadows are not editable"));
                    }
                    *c = color.unwrap_or(*c);
                    *i = intensity.unwrap_or(*i);
                    *r = range.or(*r);
                    if let Some(a) = inner_angle_deg {
                        *inner_angle = a.to_radians();
                    }
                    if let Some(a) = outer_angle_deg {
                        *outer_angle = a.to_radians();
                    }
                }
            }
        }
        Op::SetCamera {
            node,
            camera_name,
            fov_y_deg,
            orthographic_height,
            near,
            far,
        } => {
            let i = find_node(scene, &node)?;
            let name = scene.nodes[i].name.clone();
            let Node3dKind::Camera(c) = &scene.nodes[i].kind else {
                return Err(ToolError::invalid(format!("node `{name}` is not a camera")));
            };
            let new = camera_from(c.clone(), camera_name, fov_y_deg, orthographic_height, near, far)?;
            scene.nodes[i].kind = Node3dKind::Camera(new);
        }
        Op::SetMeshMaterials { node, materials } => {
            let i = find_node(scene, &node)?;
            let name = scene.nodes[i].name.clone();
            let Node3dKind::Mesh { materials: m, .. } = &mut scene.nodes[i].kind else {
                return Err(ToolError::invalid(format!("node `{name}` is not a mesh")));
            };
            *m = materials.into_iter().map(MaterialRef).collect();
        }
        Op::SetShape {
            node,
            shape,
            size,
            radius,
            height,
            tube,
            segments,
            rings,
            sides,
            subdivisions,
            material,
        } => {
            let i = find_node(scene, &node)?;
            let name = scene.nodes[i].name.clone();
            let Node3dKind::Shape { shape: s, material: m } = &mut scene.nodes[i].kind else {
                return Err(ToolError::invalid(format!("node `{name}` is not a shape")));
            };
            if let Some(kind) = shape
                && kind != s.kind()
            {
                *s = shape_kind(&kind)?;
            }
            let params = ShapeParams { size, radius, height, tube, segments, rings, sides, subdivisions };
            s.apply(&params)
                .map_err(|e| ToolError::invalid(format!("node `{name}`: {e}")))?;
            if let Some(material) = material {
                *m = (!material.is_empty()).then_some(MaterialRef(material));
            }
        }
        Op::SetMaterial {
            name,
            unlit,
            base_color,
            metallic,
            roughness,
            emissive,
            emissive_strength,
            normal_scale,
            occlusion_strength,
            alpha_mode,
            alpha_cutoff,
            double_sided,
            base_color_texture,
            orm_texture,
            normal_texture,
            emissive_texture,
        } => {
            if name.trim().is_empty() {
                return Err(ToolError::invalid("a material needs a name"));
            }
            let i = match scene.materials.iter().position(|m| m.name == name) {
                Some(i) => i,
                None => {
                    scene.materials.push(MaterialDef::new(name));
                    scene.materials.len() - 1
                }
            };
            let m = &mut scene.materials[i];
            if let Some(c) = base_color {
                check_finite("base_color", &c)?;
                m.base_color = c;
            }
            if let Some(c) = emissive {
                check_finite("emissive", &c)?;
                m.emissive = c;
            }
            for (v, dst) in [
                (metallic, &mut m.metallic),
                (roughness, &mut m.roughness),
                (emissive_strength, &mut m.emissive_strength),
                (normal_scale, &mut m.normal_scale),
                (occlusion_strength, &mut m.occlusion_strength),
                (alpha_cutoff, &mut m.alpha_cutoff),
            ] {
                if let Some(v) = v {
                    check_finite("material value", &[v])?;
                    *dst = v;
                }
            }
            if let Some(u) = unlit {
                m.unlit = u;
            }
            if let Some(d) = double_sided {
                m.double_sided = d;
            }
            if let Some(mode) = alpha_mode {
                m.alpha_mode = match mode.as_str() {
                    "opaque" => AlphaMode::Opaque,
                    "mask" => AlphaMode::Mask,
                    "blend" => AlphaMode::Blend,
                    other => {
                        return Err(ToolError::invalid(format!(
                            "alpha_mode `{other}`: use opaque, mask or blend"
                        )));
                    }
                };
            }
            texture_slot(&mut m.base_color_texture, base_color_texture);
            texture_slot(&mut m.orm_texture, orm_texture);
            texture_slot(&mut m.normal_texture, normal_texture);
            texture_slot(&mut m.emissive_texture, emissive_texture);
        }
        Op::RemoveMaterial { name } => {
            let before = scene.materials.len();
            scene.materials.retain(|m| m.name != name);
            if scene.materials.len() == before {
                return Err(ToolError::not_found(format!("no material `{name}`")));
            }
        }
        Op::SetEnvironment {
            background_color,
            sky_texture,
            ambient_color,
            ambient_intensity,
            fog,
        } => {
            let e = &mut scene.environment;
            if background_color.is_some() && sky_texture.as_deref().is_some_and(|s| !s.is_empty()) {
                return Err(ToolError::invalid(
                    "give either `background_color` or `sky_texture`, not both",
                ));
            }
            if let Some(c) = background_color {
                check_finite("background_color", &c)?;
                e.background = Background::Color(c);
            }
            match sky_texture.as_deref() {
                Some("") => {
                    if matches!(e.background, Background::Sky(_)) {
                        e.background = Background::Color([0.12, 0.13, 0.16]);
                    }
                }
                Some(t) => e.background = Background::Sky(TextureRef(t.to_owned())),
                None => {}
            }
            if let Some(c) = ambient_color {
                check_finite("ambient_color", &c)?;
                e.ambient_color = c;
            }
            if let Some(i) = ambient_intensity {
                check_finite("ambient_intensity", &[i])?;
                e.ambient_intensity = i;
            }
            if let Some(spec) = fog {
                let color = spec.color.unwrap_or([0.6, 0.65, 0.7]);
                check_finite("fog color", &color)?;
                e.fog = match spec.kind.as_str() {
                    "none" => None,
                    "linear" => Some(Fog::Linear {
                        color,
                        start: spec.start.unwrap_or(5.0),
                        end: spec.end.unwrap_or(50.0),
                    }),
                    "exponential" => Some(Fog::Exponential {
                        color,
                        density: spec.density.unwrap_or(0.02),
                    }),
                    other => {
                        return Err(ToolError::invalid(format!(
                            "fog kind `{other}`: use none, linear or exponential"
                        )));
                    }
                };
            }
        }
    }
    Ok(None)
}

/// Applies `ops` to `scene`, validates, and returns the nodes created and
/// the warnings, or an error that leaves the scene untouched by the caller
/// (who only writes on success).
pub fn apply_ops(
    scene: &mut Scene3d,
    ops: Vec<Op>,
    dir: Option<&Path>,
) -> Result<(Vec<Json>, Vec<Json>), ToolError> {
    if ops.is_empty() {
        return Err(ToolError::invalid("`ops` is empty"));
    }
    let mut created = Vec::new();
    for (n, op) in ops.into_iter().enumerate() {
        match apply_in(scene, op, dir) {
            Ok(Some((id, name))) => created.push(json!({"id": id, "name": name})),
            Ok(None) => {}
            Err(e) => {
                return Err(ToolError::new(e.kind, format!("op {n}: {}", e.message)));
            }
        }
    }
    // Poses of removed nodes go with them.
    let ids: std::collections::HashSet<Node3dId> = scene.nodes.iter().map(|n| n.id).collect();
    scene.rigs.retain(|r| ids.contains(&r.node));
    let issues = issues_json(scene, dir);
    let errors = errors_of(&issues);
    if !errors.is_empty() {
        return Err(ToolError::invalid(format!(
            "refused, the scene would have errors (nothing written): {}",
            errors.join("; ")
        )));
    }
    Ok((created, issues))
}

// ----------------------------------------------------------------- tools

pub fn list_scenes3d(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let story = cx.project.story();
    let mut scenes = Vec::new();
    for asset in story.assets.assets.iter().filter(|a| a.kind == AssetKind::Scene3d) {
        let abs = cx.sandbox.resolve(&asset.path).ok();
        let loaded = abs
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| orbit_scene::from_ron(&t));
        let entry = match loaded {
            Some(Ok(scene)) => {
                let issues = issues_json(&scene, abs.as_deref().and_then(Path::parent));
                let errors = issues.iter().filter(|i| i["severity"] == "error").count();
                json!({
                    "id": asset.id.to_string(), "path": asset.path, "name": scene.name,
                    "nodes": scene.nodes.len(), "materials": scene.materials.len(),
                    "cameras": scene.cameras().map(|(_, c)| c.name.clone()).collect::<Vec<_>>(),
                    "errors": errors, "warnings": issues.len() - errors,
                })
            }
            Some(Err(e)) => json!({"id": asset.id.to_string(), "path": asset.path, "error": e}),
            None => json!({"id": asset.id.to_string(), "path": asset.path, "error": "file not found"}),
        };
        scenes.push(entry);
    }
    out(json!({
        "scenes": scenes,
        "undeclared_files": undeclared_scene_files(cx, story),
        "hint": "read_scene3d shows a scene, edit_scene3d changes it, create_scene3d adds one.",
    }))
}

/// `.scene3d.ron` files under `assets/` that no asset declares.
fn undeclared_scene_files(cx: &ToolCx<'_>, story: &Story) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![cx.sandbox.root().join("assets")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if stack.len() < 256 {
                    stack.push(p);
                }
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".scene3d.ron"))
                && let Some(rel) = cx.sandbox.relative(&p)
                && !story.assets.assets.iter().any(|a| a.path == rel)
            {
                found.push(rel);
            }
        }
    }
    found.sort();
    found
}

pub fn read_scene3d(cx: &mut ToolCx<'_>, a: SceneArg) -> Result<String, ToolError> {
    let file = resolve(cx, &a.scene)?;
    let scene = load(&file)?;
    let issues = issues_json(&scene, file.abs.parent());
    out(json!({
        "id": file.id.to_string(), "path": file.rel,
        "scene": scene_json(&scene),
        "issues": issues,
        "ron": orbit_scene::to_ron(&scene).map_err(ToolError::invalid)?,
    }))
}

pub fn validate_scene3d(cx: &mut ToolCx<'_>, a: SceneArg) -> Result<String, ToolError> {
    let file = resolve(cx, &a.scene)?;
    let scene = load(&file)?;
    let issues = issues_json(&scene, file.abs.parent());
    let story = cx.project.story();
    let mut story_issues = Vec::new();
    let cameras: Vec<String> = scene.cameras().map(|(_, c)| c.name.clone()).collect();
    for s in &story.scenes {
        let mut shown = false;
        for n in &s.nodes {
            match &n.kind {
                orbit_core::NodeKind::ShowScene3d { scene: id, camera, .. } if *id == file.id => {
                    shown = true;
                    if let Some(c) = camera
                        && !c.is_empty()
                        && !cameras.contains(c)
                    {
                        story_issues.push(json!({
                            "severity": "error", "node": n.id.to_string(),
                            "message": format!("show_scene3d names camera `{c}`, which the scene does not have (cameras: {})", cameras.join(", ")),
                        }));
                    }
                }
                orbit_core::NodeKind::ShowScene3d { .. } | orbit_core::NodeKind::HideScene3d { .. } => {
                    shown = false;
                }
                orbit_core::NodeKind::Camera3d { camera, .. } if shown && !cameras.contains(camera) => {
                    story_issues.push(json!({
                        "severity": "error", "node": n.id.to_string(),
                        "message": format!("camera3d names camera `{camera}`, which the scene does not have (cameras: {})", cameras.join(", ")),
                    }));
                }
                _ => {}
            }
        }
    }
    let errors = issues.iter().chain(&story_issues).filter(|i| i["severity"] == "error").count();
    out(json!({"scene": file.rel, "ok": errors == 0, "issues": issues, "story_issues": story_issues}))
}

pub fn create_scene3d(cx: &mut ToolCx<'_>, a: CreateArgs) -> Result<String, ToolError> {
    let name = a.name.trim();
    if name.is_empty() {
        return Err(ToolError::invalid("the scene needs a name"));
    }
    let rel = match a.path {
        Some(p) => p.replace('\\', "/"),
        None => {
            let stem: String = name
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
                .collect();
            format!("assets/3d/{stem}.scene3d.ron")
        }
    };
    if !orbit_project::ops::is_valid_asset_path(&rel)
        || !rel.starts_with("assets/")
        || !rel.ends_with(".ron")
    {
        return Err(ToolError::invalid(format!(
            "`{rel}` must be a relative path under assets/ ending with .ron"
        )));
    }
    let abs = cx
        .sandbox
        .resolve(&rel)
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    if abs.exists() {
        return Err(ToolError::refused(format!("`{rel}` already exists")));
    }
    let mut scene = Scene3d::new(name);
    if a.with_defaults.unwrap_or(true) {
        let mut sun = Node3d::new(
            "Sun",
            None,
            Node3dKind::Light(Light::Directional {
                color: [1.0; 3],
                intensity: 3.0,
                shadows: true,
            }),
        );
        sun.transform.rotation = quat_from_euler_deg([-50.0, 30.0, 0.0]);
        sun.transform.translation = [0.0, 4.0, 0.0];
        let mut cam = Node3d::new("Camera", None, Node3dKind::Camera(default_camera()));
        cam.transform.translation = [0.0, 1.5, 5.0];
        scene.nodes.push(sun);
        scene.nodes.push(cam);
        scene.environment.background = Background::Color([0.12, 0.13, 0.16]);
        scene.environment.ambient_intensity = 0.3;
    }
    store(&abs, &scene)?;
    let id = match orbit_project::ops::add_asset(cx.project, AssetKind::Scene3d, &rel) {
        Ok(id) => id,
        Err(e) => {
            let _ = std::fs::remove_file(&abs);
            return Err(e.into());
        }
    };
    out(json!({
        "id": id.to_string(), "path": rel, "name": scene.name,
        "nodes": scene.nodes.iter().map(|n| json!({"id": n.id.to_string(), "name": n.name})).collect::<Vec<_>>(),
        "next": "edit_scene3d adds nodes, lights, cameras and materials; show_scene3d plays it in a story.",
    }))
}

pub fn edit_scene3d(cx: &mut ToolCx<'_>, a: EditArgs) -> Result<String, ToolError> {
    let file = resolve(cx, &a.scene)?;
    let mut scene = load(&file)?;
    let (created, issues) = apply_ops(&mut scene, a.ops, file.abs.parent())?;
    store(&file.abs, &scene)?;
    out(json!({
        "id": file.id.to_string(), "path": file.rel, "created_nodes": created,
        "nodes": scene.nodes.len(), "issues": issues,
        "note": "The scene file was rewritten; a change in the 3D editor must be saved and reopened to see it.",
    }))
}

pub fn write_scene3d(cx: &mut ToolCx<'_>, a: WriteArgs) -> Result<String, ToolError> {
    let file = resolve(cx, &a.scene)?;
    let scene = orbit_scene::from_ron(&a.ron)
        .map_err(|e| ToolError::invalid(format!("scene RON: {e}")))?;
    let issues = issues_json(&scene, file.abs.parent());
    let errors = errors_of(&issues);
    if !errors.is_empty() {
        return Err(ToolError::invalid(format!(
            "refused, the scene has errors (nothing written): {}",
            errors.join("; ")
        )));
    }
    store(&file.abs, &scene)?;
    out(json!({"id": file.id.to_string(), "path": file.rel, "nodes": scene.nodes.len(), "issues": issues}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene3d {
        let mut s = Scene3d::new("Room");
        s.nodes.push(Node3d::new("Camera", None, Node3dKind::Camera(default_camera())));
        s
    }

    fn op(v: Json) -> Op {
        serde_json::from_value(v).expect("op")
    }

    #[test]
    fn euler_round_trips() {
        for e in [[0.0, 0.0, 0.0], [-50.0, 30.0, 0.0], [10.0, -70.0, 120.0]] {
            let back = euler_deg(quat_from_euler_deg(e));
            let q2 = quat_from_euler_deg(back);
            let q1 = quat_from_euler_deg(e);
            let dot: f32 = q1.iter().zip(&q2).map(|(a, b)| a * b).sum();
            assert!(dot.abs() > 0.9999, "{e:?} -> {back:?}");
        }
    }

    #[test]
    fn edits_apply_in_order_and_report_created_nodes() {
        let mut s = scene();
        let (created, issues) = apply_ops(
            &mut s,
            vec![
                op(json!({"op": "add_node", "name": "Sun", "kind": {"type": "directional_light", "intensity": 5.0},
                          "rotation_euler_deg": [-45.0, 20.0, 0.0]})),
                op(json!({"op": "add_node", "name": "Lamp", "parent": "Sun",
                          "kind": {"type": "point_light", "range": 8.0}, "translation": [0.0, 1.0, 0.0]})),
                op(json!({"op": "set_light", "node": "Lamp", "intensity": 25.0})),
                op(json!({"op": "set_camera", "node": "Camera", "fov_y_deg": 35.0, "camera_name": "wide"})),
                op(json!({"op": "set_material", "name": "Wall", "base_color": [0.8, 0.7, 0.6, 1.0], "roughness": 0.9})),
                op(json!({"op": "set_environment", "ambient_intensity": 0.4, "fog": {"kind": "linear", "start": 2.0, "end": 30.0}})),
            ],
            None,
        )
        .unwrap();
        assert_eq!(created.len(), 2);
        assert!(issues.iter().all(|i| i["severity"] != "error"));
        let lamp = s.nodes.iter().find(|n| n.name == "Lamp").unwrap();
        assert!(matches!(lamp.kind, Node3dKind::Light(Light::Point { intensity, .. }) if intensity == 25.0));
        assert!(lamp.parent.is_some());
        assert_eq!(s.cameras().next().unwrap().1.name, "wide");
        assert_eq!(s.materials[0].name, "Wall");
        assert!(matches!(s.environment.fog, Some(Fog::Linear { .. })));
    }

    #[test]
    fn errors_name_the_failing_op() {
        let mut s = scene();
        let e = apply_ops(
            &mut s,
            vec![
                op(json!({"op": "set_visible", "node": "Camera", "visible": false})),
                op(json!({"op": "set_light", "node": "Camera", "intensity": 2.0})),
            ],
            None,
        )
        .unwrap_err();
        assert!(e.message.starts_with("op 1:"), "{}", e.message);
        assert!(e.message.contains("not a light"));
    }

    #[test]
    fn invalid_results_are_refused() {
        let mut s = scene();
        // A second camera with the same name: validation error.
        let e = apply_ops(
            &mut s,
            vec![op(json!({"op": "add_node", "name": "Cam2",
                           "kind": {"type": "camera", "camera_name": "main"}}))],
            None,
        )
        .unwrap_err();
        assert!(e.message.contains("duplicate camera name"), "{}", e.message);
    }

    #[test]
    fn nodes_resolve_by_id_and_names_must_be_unique() {
        let mut s = scene();
        apply_ops(
            &mut s,
            vec![
                op(json!({"op": "add_node", "name": "Box", "kind": {"type": "empty"}})),
                op(json!({"op": "add_node", "name": "Box", "kind": {"type": "empty"}})),
            ],
            None,
        )
        .unwrap();
        let ambiguous = apply(&mut s, op(json!({"op": "remove_node", "node": "Box"}))).unwrap_err();
        assert!(ambiguous.message.contains("ambiguous"));
        let id = s.nodes[1].id.to_string();
        apply(&mut s, op(json!({"op": "remove_node", "node": id}))).unwrap();
        assert_eq!(s.nodes.len(), 2);
    }

    #[test]
    fn remove_takes_descendants_and_reparent_refuses_cycles() {
        let mut s = scene();
        apply_ops(
            &mut s,
            vec![
                op(json!({"op": "add_node", "name": "A", "kind": {"type": "empty"}})),
                op(json!({"op": "add_node", "name": "B", "parent": "A", "kind": {"type": "empty"}})),
            ],
            None,
        )
        .unwrap();
        let cycle = apply(&mut s, op(json!({"op": "set_parent", "node": "A", "parent": "B"}))).unwrap_err();
        assert_eq!(cycle.kind, crate::ErrorKind::Refused);
        apply(&mut s, op(json!({"op": "remove_node", "node": "A"}))).unwrap();
        assert_eq!(s.nodes.len(), 1);
    }

    #[test]
    fn material_textures_set_and_clear() {
        let mut s = scene();
        apply_ops(
            &mut s,
            vec![op(json!({"op": "set_material", "name": "M", "base_color_texture": "wall", "alpha_mode": "mask"}))],
            None,
        )
        .unwrap();
        assert_eq!(s.materials[0].base_color_texture.as_ref().unwrap().texture.0, "wall");
        assert_eq!(s.materials[0].alpha_mode, AlphaMode::Mask);
        apply(&mut s, op(json!({"op": "set_material", "name": "M", "base_color_texture": ""}))).unwrap();
        assert!(s.materials[0].base_color_texture.is_none());
        assert!(apply(&mut s, op(json!({"op": "set_material", "name": "M", "alpha_mode": "x"}))).is_err());
    }

    #[test]
    fn unknown_fields_and_empty_ops_are_rejected() {
        assert!(serde_json::from_value::<Op>(json!({"op": "set_visible", "node": "a", "visible": true, "x": 1})).is_err());
        assert!(apply_ops(&mut scene(), Vec::new(), None).is_err());
    }

    #[test]
    fn sky_and_color_are_exclusive() {
        let mut s = scene();
        assert!(apply(&mut s, op(json!({"op": "set_environment", "background_color": [0,0,0], "sky_texture": "sky"}))).is_err());
        apply(&mut s, op(json!({"op": "set_environment", "sky_texture": "sky"}))).unwrap();
        assert!(matches!(s.environment.background, Background::Sky(_)));
        apply(&mut s, op(json!({"op": "set_environment", "sky_texture": ""}))).unwrap();
        assert!(matches!(s.environment.background, Background::Color(_)));
    }

    #[test]
    fn pose_ops_pose_save_apply_and_reset_bones() {
        let mut s = scene();
        let hero = Node3d::new("Hero", None, Node3dKind::Prefab(PrefabRef("hero".into())));
        let id = hero.id;
        s.nodes.push(hero);
        // A bone already posed: only the given fields change.
        s.rig_mut(id).pose.set(
            "arm",
            Transform {
                translation: [0.0, 1.0, 0.0],
                ..Transform::IDENTITY
            },
        );
        apply_ops(
            &mut s,
            vec![
                op(json!({"op": "set_bone_pose", "node": "Hero", "joint": "arm", "rotation_euler_deg": [0.0, 0.0, 90.0]})),
                op(json!({"op": "save_pose", "node": "Hero", "name": "wave"})),
                op(json!({"op": "reset_pose", "node": "Hero"})),
            ],
            None,
        )
        .unwrap();
        let rig = s.rig(id).unwrap();
        assert!(rig.pose.is_rest());
        let saved = rig.poses[0].pose.get("arm").unwrap();
        assert_eq!(saved.translation, [0.0, 1.0, 0.0]);
        assert!((euler_deg(saved.rotation)[2] - 90.0).abs() < 1e-3);
        apply(&mut s, op(json!({"op": "apply_pose", "node": "Hero", "name": "wave"}))).unwrap();
        assert!(s.rig(id).unwrap().pose.get("arm").is_some());
        assert!(scene_json(&s)["rigs"][0]["saved_poses"][0] == "wave");
        // Unknown bones need the model: a clear error, nothing applied.
        let err = apply(&mut s, op(json!({"op": "set_bone_pose", "node": "Hero", "joint": "leg", "rotation": [0.0, 0.0, 0.0, 1.0]})))
            .unwrap_err();
        assert!(err.message.contains("models"), "{}", err.message);
        assert!(apply(&mut s, op(json!({"op": "remove_pose", "node": "Hero", "name": "nope"}))).is_err());
        apply(&mut s, op(json!({"op": "remove_pose", "node": "Hero", "name": "wave"}))).unwrap();
        // Removing the node drops its pose data.
        apply_ops(&mut s, vec![op(json!({"op": "remove_node", "node": "Hero"}))], None).unwrap();
        assert!(s.rigs.is_empty());
    }
}
