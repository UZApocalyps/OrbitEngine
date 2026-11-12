//! Where each installed plugin came from (`installs.ron`), and update
//! detection.
//!
//! An installed plugin is only updated from the source it was installed
//! from; a plugin installed from a file (no record) only from the official
//! source. A third-party catalog listing the same id with a higher version
//! therefore never shows up as an "update".

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, select};
use crate::sources::{OFFICIAL_ID, SourceEntry};
use crate::version::is_newer;
use crate::{StorePaths, write_atomic};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub source: String,
    pub version: String,
    #[serde(default)]
    pub fingerprint: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origins {
    #[serde(default)]
    pub plugins: BTreeMap<String, Origin>,
}

impl Origins {
    pub fn load(paths: &StorePaths) -> Self {
        std::fs::read_to_string(paths.installs_file())
            .ok()
            .and_then(|t| ron::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &StorePaths) -> std::io::Result<()> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(std::io::Error::other)?;
        write_atomic(&paths.installs_file(), text.as_bytes())
    }

    /// The source updates of `id` come from.
    pub fn update_source(&self, id: &str) -> &str {
        self.plugins
            .get(id)
            .map_or(OFFICIAL_ID, |o| o.source.as_str())
    }

    /// Records an install from the store.
    pub fn record(&mut self, id: &str, source: &str, version: &str, fingerprint: &str) {
        self.plugins.insert(
            id.to_owned(),
            Origin {
                source: source.to_owned(),
                version: version.to_owned(),
                fingerprint: fingerprint.to_owned(),
            },
        );
    }

    /// Forgets the plugins no longer installed (ids of the installed ones).
    pub fn retain_installed(&mut self, installed: &[String]) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|id, _| installed.contains(id));
        before != self.plugins.len()
    }

    /// Forgets `id` (installed from a file: updates come from the official
    /// source again).
    pub fn forget(&mut self, id: &str) -> bool {
        self.plugins.remove(id).is_some()
    }
}

/// A newer build for this editor of an installed plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub id: String,
    pub installed: String,
    pub available: String,
    pub source: String,
}

/// The updates of `installed` (id, version) in `catalogs` (enabled sources)
/// for the editor build `fingerprint`.
pub fn find_updates(
    installed: &[(String, String)],
    origins: &Origins,
    catalogs: &[(&SourceEntry, &Catalog)],
    fingerprint: &str,
) -> Vec<Update> {
    let mut out = Vec::new();
    for (id, version) in installed {
        let source = origins.update_source(id);
        let Some((_, catalog)) = catalogs.iter().find(|(s, _)| s.id == source) else {
            continue;
        };
        let Some(plugin) = catalog.plugin(id) else {
            continue;
        };
        if let Some((v, _)) = select(plugin, fingerprint).build()
            && is_newer(&v.version, version)
        {
            out.push(Update {
                id: id.clone(),
                installed: version.clone(),
                available: v.version.clone(),
                source: source.to_owned(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::parse;
    use crate::sources::Sources;
    use crate::test_util::scratch;

    const FP: &str = "0123456789abcdef0123456789abcdef";

    fn catalog(id: &str, versions: &[(&str, &str)]) -> Catalog {
        let versions: Vec<_> = versions
            .iter()
            .map(|(v, fp)| {
                serde_json::json!({ "version": v, "builds": [{
                    "editor_version": "0.4.0", "fingerprint": fp, "url": "a.orbitplugin",
                    "sha256": "ab".repeat(32), "size": 1 }] })
            })
            .collect();
        let json = serde_json::json!({ "schema_version": 1, "source": { "name": "x" },
            "plugins": [{ "id": id, "name": "P", "versions": versions }] });
        parse(&json.to_string(), true).unwrap().catalog
    }

    #[test]
    fn updates_come_from_the_install_source_only() {
        let mut sources = Sources::default();
        let third = sources
            .add("https://example.org/index.json", "Third", "")
            .unwrap();
        let official = sources.sources[0].clone();
        let other_fp = "f".repeat(32);
        let official_cat = catalog("acme.hello", &[("1.1.0", FP), ("1.2.0", &other_fp)]);
        let third_cat = catalog("acme.hello", &[("9.0.0", FP)]);
        let catalogs = [(&official, &official_cat), (&third, &third_cat)];
        let installed = vec![("acme.hello".to_owned(), "1.0.0".to_owned())];

        // Installed from a file: the official source only (1.2.0 has no build
        // for this editor, the third-party 9.0.0 is ignored).
        let mut origins = Origins::default();
        let updates = find_updates(&installed, &origins, &catalogs, FP);
        assert_eq!(
            updates,
            [Update {
                id: "acme.hello".into(),
                installed: "1.0.0".into(),
                available: "1.1.0".into(),
                source: OFFICIAL_ID.into()
            }]
        );
        // Installed from the third-party source: its versions.
        origins.record("acme.hello", &third.id, "1.0.0", FP);
        let updates = find_updates(&installed, &origins, &catalogs, FP);
        assert_eq!(updates[0].available, "9.0.0");
        // Up to date.
        let current = vec![("acme.hello".to_owned(), "9.0.0".to_owned())];
        assert!(find_updates(&current, &origins, &catalogs, FP).is_empty());
        // Source disabled (not in the list): nothing.
        assert!(find_updates(&installed, &origins, &catalogs[..1], FP).is_empty());
    }

    #[test]
    fn origins_persist_and_follow_the_installed_plugins() {
        let paths = StorePaths {
            root: scratch("installs"),
        };
        let mut o = Origins::load(&paths);
        o.record("acme.a", "src-1", "1.0.0", FP);
        o.record("acme.b", "official", "2.0.0", FP);
        o.save(&paths).unwrap();
        let mut again = Origins::load(&paths);
        assert_eq!(again, o);
        // acme.a uninstalled.
        assert!(again.retain_installed(&["acme.b".into()]));
        assert!(!again.retain_installed(&["acme.b".into()]));
        assert_eq!(again.update_source("acme.b"), "official");
        // acme.b then installed from a file.
        assert!(again.forget("acme.b"));
        assert!(again.plugins.is_empty());
        assert_eq!(again.update_source("acme.b"), OFFICIAL_ID);
    }
}
