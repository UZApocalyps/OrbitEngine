//! The editor plugin store (see `docs/plugin-store.md`):
//!
//! * [`catalog`]: the `index.json` format (schema 1), its validation (a bad
//!   entry is skipped, not the whole catalog) and the choice of the build made
//!   for this editor ([`catalog::select`]);
//! * [`signing`]: the signed build message, the official public key, the
//!   pluggable [`signing::SignatureVerifier`] (Ed25519 from `orbit_sign`);
//! * [`sources`]: the catalog sources (`sources.ron`), the built-in official
//!   one, trust levels, GitHub URL resolution;
//! * [`fetch`]: catalogs fetched with `ETag` / `Last-Modified`, cached, with
//!   an offline fallback;
//! * [`download`]: a build downloaded, its size, SHA-256 and signature
//!   checked before the plugin installer sees it;
//! * [`installs`]: which source each plugin came from, and update detection.
//!
//! Nothing here loads code: the downloaded package goes through
//! `orbit_plugin_host::PluginManager::install` (format, descriptor probe in a
//! child process, fingerprint) like a package picked from disk.

pub mod catalog;
pub mod download;
pub mod fetch;
pub mod game;
pub mod installs;
pub mod signing;
pub mod sources;
pub mod version;

use std::path::{Path, PathBuf};

pub use catalog::{
    Availability, Build, Catalog, CatalogPlugin, CatalogVersion, Localized, ParsedCatalog, select,
};
pub use signing::{OFFICIAL_PUBLIC_KEY, SignatureVerifier};
pub use sources::{OFFICIAL_ID, OFFICIAL_URL, SourceEntry, Sources, Trust};

/// The store's folder in the editor config folder.
pub const STORE_DIR: &str = "plugin_store";

/// Where the store keeps its files (`<config>/plugin_store/`).
#[derive(Clone, Debug)]
pub struct StorePaths {
    pub root: PathBuf,
}

impl StorePaths {
    pub fn for_config_dir(config_dir: &Path) -> Self {
        Self {
            root: config_dir.join(STORE_DIR),
        }
    }

    pub fn sources_file(&self) -> PathBuf {
        self.root.join("sources.ron")
    }

    pub fn installs_file(&self) -> PathBuf {
        self.root.join("installs.ron")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }
}

/// The editor build id: the first 16 bytes (hex) of the SHA-256 of the
/// fingerprint key (`orbit_plugin_host::EditorFacts::cache_key`). Catalog
/// builds are matched on it.
pub fn fingerprint_id(cache_key: &str) -> String {
    let digest = orbit_sign::sha256::digest(cache_key.as_bytes());
    orbit_sign::hex(&digest[..16])
}

/// The fingerprint key from a fingerprint in JSON (`orbit-editor
/// --probe-plugin` / `--plugin-abi`), in the order of `EditorFacts::cache_key`.
pub fn fingerprint_key_from_json(f: &serde_json::Value) -> Option<String> {
    let text = |k: &str| f.get(k).and_then(|v| v.as_str()).map(str::to_owned);
    let abi = f.get("abi_version")?.as_u64()?;
    Some(format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        text("api_version")?,
        abi,
        text("rustc")?,
        text("egui")?,
        text("target")?,
        text("profile")?,
        text("api_hash")?,
        text("layout_hash")?,
        text("type_hash")?
    ))
}

/// Seconds since 1970.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Writes `text` through a temporary file and a rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::path::{Path, PathBuf};

    /// A fresh folder under the target directory (never the system temp drive).
    pub fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/test-tmp/orbit_plugin_store_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_id_is_the_same_from_the_editor_facts_and_from_json() {
        let facts = orbit_plugin_host::EditorFacts::current();
        let json = serde_json::to_value(orbit_editor_api::native::Fingerprint::current()).unwrap();
        assert_eq!(
            fingerprint_key_from_json(&json).as_deref(),
            Some(facts.cache_key().as_str())
        );
        let id = fingerprint_id(&facts.cache_key());
        assert_eq!(id.len(), 32);
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(fingerprint_id("other"), id);
    }
}
