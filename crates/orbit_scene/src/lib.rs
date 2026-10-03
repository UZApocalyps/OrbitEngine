//! GPU-free 3D scene data (ADR 0014, M6-01).
//!
//! [`Scene3d`] documents (`.scene3d.ron`, own `scene3d_version`), [`Prefab`]s,
//! PBR/unlit [`MaterialDef`]s, lights, cameras, transforms, [`validate`] and
//! the `scene3d_version` [`migrate`] skeleton. glTF import lives in
//! [`import`] (feature `gltf-import`, M6-02).

pub mod assets;
pub mod migrate;
pub mod quality;
mod validate;

#[cfg(feature = "ktx2")]
pub mod ktx2;

#[cfg(feature = "gltf-import")]
pub mod import;

use serde::{Deserialize, Serialize};

pub use orbit_core::Node3dId;
pub use assets::{MeshAsset, PrimitiveData, TextureAsset};
pub use validate::{Issue, Severity, validate, validate_prefab};

/// Current `scene3d_version`.
pub const SCENE3D_VERSION: u32 = 1;

/// Reference to a mesh asset (name key, resolved by the project).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MeshRef(pub String);
/// Reference to a material by name (scene or prefab local).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MaterialRef(pub String);
/// Reference to a texture asset by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TextureRef(pub String);
/// Reference to a prefab asset by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrefabRef(pub String);

/// Local transform: translation, rotation quaternion `[x, y, z, w]`, scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AlphaMode {
    #[default]
    Opaque,
    /// Alpha test against the material's `alpha_cutoff`.
    Mask,
    Blend,
}

/// Texture binding with optional UV transform (`KHR_texture_transform`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureSlot {
    pub texture: TextureRef,
    pub uv_set: u32,
    pub offset: [f32; 2],
    pub rotation: f32,
    pub scale: [f32; 2],
}

impl TextureSlot {
    pub fn new(texture: TextureRef) -> Self {
        Self {
            texture,
            uv_set: 0,
            offset: [0.0; 2],
            rotation: 0.0,
            scale: [1.0; 2],
        }
    }
}

/// PBR metallic-roughness (glTF aligned) or unlit material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialDef {
    pub name: String,
    pub unlit: bool,
    pub base_color: [f32; 4],
    pub base_color_texture: Option<TextureSlot>,
    pub metallic: f32,
    pub roughness: f32,
    /// Occlusion (R) / roughness (G) / metallic (B) texture.
    pub orm_texture: Option<TextureSlot>,
    pub normal_texture: Option<TextureSlot>,
    pub normal_scale: f32,
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    pub emissive_texture: Option<TextureSlot>,
    pub occlusion_strength: f32,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

impl MaterialDef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            unlit: false,
            base_color: [1.0; 4],
            base_color_texture: None,
            metallic: 1.0,
            roughness: 1.0,
            orm_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            emissive_texture: None,
            occlusion_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Light {
    Directional {
        color: [f32; 3],
        intensity: f32,
        shadows: bool,
    },
    Point {
        color: [f32; 3],
        intensity: f32,
        range: Option<f32>,
    },
    Spot {
        color: [f32; 3],
        intensity: f32,
        range: Option<f32>,
        inner_angle: f32,
        outer_angle: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Projection {
    /// Vertical field of view in radians.
    Perspective { fov_y: f32 },
    Orthographic { height: f32 },
}

/// Named camera; the unit used by stories (ADR 0014 section 5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraDef {
    pub name: String,
    pub projection: Projection,
    pub near: f32,
    pub far: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node3dKind {
    Empty,
    Mesh {
        mesh: MeshRef,
        /// One material per primitive/slot.
        materials: Vec<MaterialRef>,
    },
    Light(Light),
    Camera(CameraDef),
    /// Instance of a prefab asset.
    Prefab(PrefabRef),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node3d {
    pub id: Node3dId,
    pub name: String,
    pub parent: Option<Node3dId>,
    pub transform: Transform,
    /// Author override; preserved across glTF re-import.
    pub visible: bool,
    pub kind: Node3dKind,
}

impl Node3d {
    pub fn new(name: impl Into<String>, parent: Option<Node3dId>, kind: Node3dKind) -> Self {
        Self {
            id: Node3dId::new(),
            name: name.into(),
            parent,
            transform: Transform::IDENTITY,
            visible: true,
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Background {
    Color([f32; 3]),
    /// Cubemap or equirectangular texture.
    Sky(TextureRef),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Fog {
    Linear {
        color: [f32; 3],
        start: f32,
        end: f32,
    },
    Exponential {
        color: [f32; 3],
        density: f32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub background: Background,
    pub ambient_color: [f32; 3],
    pub ambient_intensity: f32,
    pub fog: Option<Fog>,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            background: Background::Color([0.0; 3]),
            ambient_color: [1.0; 3],
            ambient_intensity: 0.1,
            fog: None,
        }
    }
}

/// A reusable node tree (e.g. an imported glTF) with its materials.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Prefab {
    pub name: String,
    pub nodes: Vec<Node3d>,
    pub materials: Vec<MaterialDef>,
    /// Author-edited copies of imported materials (matched by name over
    /// `materials`); survive re-import.
    #[serde(default)]
    pub material_overrides: Vec<MaterialDef>,
}

/// A `.scene3d.ron` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene3d {
    pub scene3d_version: u32,
    pub id: orbit_core::SceneId,
    pub name: String,
    pub nodes: Vec<Node3d>,
    pub materials: Vec<MaterialDef>,
    pub environment: Environment,
}

impl Scene3d {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            scene3d_version: SCENE3D_VERSION,
            id: orbit_core::SceneId::new(),
            name: name.into(),
            nodes: Vec::new(),
            materials: Vec::new(),
            environment: Environment::default(),
        }
    }

    /// Named cameras (Camera nodes), in node order.
    pub fn cameras(&self) -> impl Iterator<Item = (&Node3d, &CameraDef)> {
        self.nodes.iter().filter_map(|n| match &n.kind {
            Node3dKind::Camera(c) => Some((n, c)),
            _ => None,
        })
    }
}

pub fn to_ron(scene: &Scene3d) -> Result<String, String> {
    ron::ser::to_string_pretty(scene, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())
}

/// Parses a `.scene3d.ron`, migrating older versions first.
pub fn from_ron(source: &str) -> Result<Scene3d, String> {
    let from = migrate::read_version(source)?;
    let text = migrate::migrate(from, source)?;
    ron::from_str(&text).map_err(|e| e.to_string())
}

pub fn prefab_to_ron(prefab: &Prefab) -> Result<String, String> {
    ron::ser::to_string_pretty(prefab, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())
}

pub fn prefab_from_ron(source: &str) -> Result<Prefab, String> {
    ron::from_str(source).map_err(|e| e.to_string())
}
