//! The 3D models of an exported game (feature `3d`): the `3d/` entries of
//! the pak (ADR 0014, addendum "Blobs 3D dans le pak"), loaded the first
//! time a 3D scene is, then registered on the 3D renderer.
//!
//! Two stages, both asynchronous (HTTP range requests on the web): the
//! index `3d/index.ron`, then every mesh (`.omesh`), prefab (RON) and
//! texture (KTX2) it lists. A game run from its project folder has no index:
//! that is not an error (the library is just empty). A blob that cannot be
//! decoded (unknown `.omesh` version...) is reported and skipped; the scene
//! is drawn without it.

use std::sync::OnceLock;

use orbit_assets::{AssetError, AssetHandle, AssetLoader, AssetPoll};
use orbit_scene::pack3d::{Assets3dIndex, INDEX_ENTRY};
use orbit_scene::{MeshAsset, Prefab, TextureAsset};

/// Decoded 3D assets of the game.
#[derive(Debug, Default)]
pub struct Library3d {
    pub meshes: Vec<MeshAsset>,
    pub prefabs: Vec<Prefab>,
    pub textures: Vec<TextureAsset>,
}

impl Library3d {
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty() && self.prefabs.is_empty() && self.textures.is_empty()
    }
}

struct Parts {
    meshes: Vec<AssetHandle<MeshAsset>>,
    prefabs: Vec<AssetHandle<Prefab>>,
    textures: Vec<AssetHandle<TextureAsset>>,
}

/// The library being loaded.
pub(crate) struct LibraryLoad {
    loader: AssetLoader,
    index: AssetHandle<Assets3dIndex>,
    parts: OnceLock<Parts>,
}

impl LibraryLoad {
    pub(crate) fn start(loader: &AssetLoader) -> Self {
        let index = loader.load_with(INDEX_ENTRY, |_, bytes| {
            let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            Assets3dIndex::from_ron(&text)
        });
        Self {
            loader: loader.clone(),
            index,
            parts: OnceLock::new(),
        }
    }

    /// Starts the second stage once the index is there.
    fn parts(&self) -> Option<&Parts> {
        let AssetPoll::Ready(index) = self.index.poll() else {
            return None;
        };
        Some(self.parts.get_or_init(|| {
            let l = &self.loader;
            Parts {
                meshes: index
                    .meshes
                    .iter()
                    .map(|(name, entry)| {
                        let name = name.clone();
                        l.load_with(entry, move |_, b| orbit_scene::omesh::decode(&name, &b))
                    })
                    .collect(),
                prefabs: index
                    .prefabs
                    .values()
                    .map(|entry| {
                        l.load_with(entry, |_, b| {
                            orbit_scene::prefab_from_ron(&String::from_utf8_lossy(&b))
                        })
                    })
                    .collect(),
                textures: index
                    .textures
                    .iter()
                    .map(|(name, entry)| {
                        let name = name.clone();
                        l.load_with(entry, move |_, b| orbit_scene::ktx2::decode_rgba8(&name, &b))
                    })
                    .collect(),
            }
        }))
    }

    pub(crate) fn is_done(&self) -> bool {
        match self.index.poll() {
            AssetPoll::Loading => false,
            AssetPoll::Failed(_) => true,
            AssetPoll::Ready(_) => self.parts().is_some_and(|p| {
                fn done<T>(handles: &[AssetHandle<T>]) -> bool {
                    handles.iter().all(|h| !matches!(h.poll(), AssetPoll::Loading))
                }
                done(&p.meshes) && done(&p.prefabs) && done(&p.textures)
            }),
        }
    }

    /// The library (empty when the game has none) and the failures.
    pub(crate) fn finish(self) -> (Library3d, Vec<String>) {
        let mut lib = Library3d::default();
        let mut failures = Vec::new();
        match self.index.poll() {
            AssetPoll::Ready(_) => {}
            // No index: a game run from its folder, or without 3D models.
            AssetPoll::Failed(AssetError::Io { .. }) => {
                log::debug!("no {INDEX_ENTRY}: no packed 3D models");
                return (lib, failures);
            }
            AssetPoll::Failed(e) => {
                failures.push(format!("3D models: {e}"));
                return (lib, failures);
            }
            AssetPoll::Loading => return (lib, failures),
        }
        let Some(parts) = self.parts() else {
            return (lib, failures);
        };
        fn take<T: Clone>(handles: &[AssetHandle<T>], out: &mut Vec<T>, failures: &mut Vec<String>) {
            for h in handles {
                match h.poll() {
                    AssetPoll::Ready(v) => out.push((*v).clone()),
                    AssetPoll::Failed(e) => failures.push(format!("3D model: {e}")),
                    AssetPoll::Loading => {}
                }
            }
        }
        take(&parts.meshes, &mut lib.meshes, &mut failures);
        take(&parts.prefabs, &mut lib.prefabs, &mut failures);
        take(&parts.textures, &mut lib.textures, &mut failures);
        log::info!(
            "3D models: {} mesh(es), {} prefab(s), {} texture(s)",
            lib.meshes.len(),
            lib.prefabs.len(),
            lib.textures.len()
        );
        (lib, failures)
    }
}
