//! The 3D content of a pak (ADR 0014, addendum "Blobs 3D dans le pak").
//!
//! Container entries, not `GamePack` fields:
//!
//! * [`INDEX_ENTRY`] `3d/index.ron`: an [`Assets3dIndex`] (name -> entry of
//!   every mesh, prefab and texture shipped);
//! * `3d/meshes/<n>.omesh` ([`crate::omesh`]), `3d/prefabs/<n>.prefab.ron`
//!   ([`crate::prefab_from_ron`]), `3d/textures/<n>.ktx2` (RGBA8 +
//!   Zstandard, [`crate::ktx2`]).
//!
//! Entry files are numbered (`<n>`), never named after the asset: asset
//! names are free text (`model/mesh#2`). Procedural shapes are not shipped,
//! players generate them ([`crate::shapes`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Pak entry of the index.
pub const INDEX_ENTRY: &str = "3d/index.ron";
/// Current `assets3d_version`.
pub const ASSETS3D_VERSION: u32 = 1;

/// What the `3d/` entries of a pak hold.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Assets3dIndex {
    pub assets3d_version: u32,
    /// Mesh name -> `.omesh` entry.
    #[serde(default)]
    pub meshes: BTreeMap<String, String>,
    /// Prefab name -> `.prefab.ron` entry.
    #[serde(default)]
    pub prefabs: BTreeMap<String, String>,
    /// Texture name -> `.ktx2` entry.
    #[serde(default)]
    pub textures: BTreeMap<String, String>,
}

impl Assets3dIndex {
    pub fn new() -> Self {
        Self {
            assets3d_version: ASSETS3D_VERSION,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty() && self.prefabs.is_empty() && self.textures.is_empty()
    }

    pub fn to_ron(&self) -> Result<String, String> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| e.to_string())
    }

    /// Parses an index; a newer `assets3d_version` is refused (fields are
    /// only ever added, with defaults, so older versions always parse).
    pub fn from_ron(text: &str) -> Result<Self, String> {
        let index: Self = ron::from_str(text).map_err(|e| format!("3d/index.ron: {e}"))?;
        if index.assets3d_version == 0 || index.assets3d_version > ASSETS3D_VERSION {
            return Err(format!(
                "3D assets version {} is not supported (this build reads up to {ASSETS3D_VERSION})",
                index.assets3d_version
            ));
        }
        Ok(index)
    }

    /// Every entry path, for the loader.
    pub fn entries(&self) -> impl Iterator<Item = &str> {
        self.meshes
            .values()
            .chain(self.prefabs.values())
            .chain(self.textures.values())
            .map(String::as_str)
    }
}

/// Entry path of the `n`-th mesh / prefab / texture.
pub fn mesh_entry(n: usize) -> String {
    format!("3d/meshes/{n}.omesh")
}

pub fn prefab_entry(n: usize) -> String {
    format!("3d/prefabs/{n}.prefab.ron")
}

pub fn texture_entry(n: usize) -> String {
    format!("3d/textures/{n}.ktx2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_refuses_newer_versions() {
        let mut index = Assets3dIndex::new();
        index.meshes.insert("chair/seat".into(), mesh_entry(0));
        index.textures.insert("wood".into(), texture_entry(0));
        let text = index.to_ron().unwrap();
        assert_eq!(Assets3dIndex::from_ron(&text).unwrap(), index);
        assert_eq!(index.entries().count(), 2);
        assert!(Assets3dIndex::from_ron("(assets3d_version: 2)").is_err());
        assert!(Assets3dIndex::from_ron("(assets3d_version: 1)").unwrap().is_empty());
    }
}
