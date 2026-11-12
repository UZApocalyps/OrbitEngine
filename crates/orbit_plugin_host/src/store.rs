//! `plugins.ron` in the plugins folder: per plugin, whether it is enabled,
//! the checksum recorded at install, the cached probe result and the last
//! error that disabled it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orbit_editor_api::native::DescriptorInfo;
use serde::{Deserialize, Serialize};

/// File name of the store in the plugins folder.
pub const STORE_FILE: &str = "plugins.ron";

/// The descriptor read by a probe, valid for one editor build and one
/// library file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeCache {
    /// `EditorFacts::cache_key` + `|` + the library's SHA-256.
    pub key: String,
    pub info: DescriptorInfo,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRecord {
    pub enabled: bool,
    /// SHA-256 (hex) of the library, recorded at install.
    #[serde(default)]
    pub sha256: String,
    /// Size and modification time (seconds) seen at install: when both are
    /// unchanged the checksum is not computed again.
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub modified: u64,
    #[serde(default)]
    pub probe: Option<ProbeCache>,
    /// Why the editor disabled it (crash), shown until it is enabled again.
    #[serde(default)]
    pub error: Option<String>,
    /// Uninstalled while loaded: its folder is removed at the next start.
    #[serde(default)]
    pub uninstall: bool,
    /// A new version waits in `.staged/<id>` (installed while loaded).
    #[serde(default)]
    pub staged: bool,
    /// Installed from a package shipped with the editor (`bundled-plugins/`),
    /// not by hand nor from the store: only such a plugin is updated from
    /// the shipped package. Cleared by any other install.
    #[serde(default)]
    pub bundled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginStore {
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginRecord>,
    /// Ids of the shipped packages already installed (or found installed by
    /// hand) once: a shipped plugin the user uninstalled does not come back.
    /// Missing in files written by earlier editors; ignored (as an unknown
    /// field) by those editors reading a newer file.
    #[serde(default)]
    pub bundled_offered: BTreeSet<String>,
}

impl PluginStore {
    /// Reads the store of `root`; a missing or broken file is an empty store
    /// (every plugin disabled: the safe default).
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(root.join(STORE_FILE))
            .ok()
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the store (through a temporary file, then a rename).
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(std::io::Error::other)?;
        let tmp = root.join(format!("{STORE_FILE}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, root.join(STORE_FILE))
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.plugins.get(id).is_some_and(|r| r.enabled)
    }
}
