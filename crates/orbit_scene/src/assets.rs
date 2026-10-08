//! Imported mesh / texture asset data. GPU free and available without the
//! `gltf-import` feature, so wasm runtimes and `orbit_render3d` can use it.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrimitiveData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub tangents: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// Skinned meshes: up to four joint indices per vertex (into the
    /// mesh's [`Skeleton`](crate::skin::Skeleton)); empty otherwise.
    pub joints: Vec<[u16; 4]>,
    /// Weights matching `joints`.
    pub weights: Vec<[f32; 4]>,
}

/// One mesh asset; primitives line up with the node's `materials`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MeshAsset {
    pub name: String,
    pub primitives: Vec<PrimitiveData>,
    /// Skeleton of a skinned (rigged) mesh.
    pub skin: Option<crate::skin::Skeleton>,
    /// glTF animations targeting `skin`.
    pub clips: Vec<crate::skin::Clip>,
}

/// Decoded texture (RGBA8).
#[derive(Debug, Clone, PartialEq)]
pub struct TextureAsset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}
