//! `index.json`: a source's catalog (schema 1, see `docs/plugin-store.md`).
//!
//! Parsing is lenient where it can be: unknown fields are ignored, an entry
//! (plugin, version or build) that breaks a rule is skipped with a warning and
//! the rest of the catalog stays usable. A `schema_version` greater than
//! [`SCHEMA_VERSION`] is refused (the catalog needs a newer editor).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::version::{Version, compare};

/// The catalog schema this editor reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// Largest catalog accepted.
pub const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
/// Largest plugin package accepted.
pub const MAX_PACKAGE_BYTES: u64 = 1 << 30;

/// A text, either the same for every language or per language (`en`, `fr`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Localized {
    Text(String),
    Map(BTreeMap<String, String>),
}

impl Default for Localized {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl Localized {
    /// The text in `lang`, else English, else any.
    pub fn get(&self, lang: &str) -> &str {
        match self {
            Self::Text(t) => t,
            Self::Map(m) => m
                .get(lang)
                .or_else(|| m.get("en"))
                .or_else(|| m.values().next())
                .map_or("", String::as_str),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(t) => t.trim().is_empty(),
            Self::Map(m) => m.values().all(|v| v.trim().is_empty()),
        }
    }

    /// Every text (search).
    pub fn all(&self) -> Vec<&str> {
        match self {
            Self::Text(t) => vec![t.as_str()],
            Self::Map(m) => m.values().map(String::as_str).collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default)]
    pub description: Localized,
}

/// One downloadable package, for one editor build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    pub editor_version: String,
    /// The editor build id (`crate::fingerprint_id`).
    pub fingerprint: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub profile: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// Ed25519 signature (hex) of `signing::build_message`, or `null`.
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogVersion {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<String>,
    #[serde(default)]
    pub changelog: Localized,
    #[serde(default)]
    pub builds: Vec<Build>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPlugin {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Localized,
    #[serde(default)]
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub versions: Vec<CatalogVersion>,
}

impl CatalogPlugin {
    /// The newest version (any editor).
    pub fn latest(&self) -> Option<&CatalogVersion> {
        self.versions
            .iter()
            .max_by(|a, b| compare(&a.version, &b.version))
    }

    /// Whether `query` (lowercase words) matches the name, id, author, tags
    /// or description.
    pub fn matches(&self, query: &str) -> bool {
        let mut haystack = format!(
            "{} {} {} {}",
            self.name,
            self.id,
            self.author,
            self.tags.join(" ")
        );
        for t in self.description.all() {
            haystack.push(' ');
            haystack.push_str(t);
        }
        let haystack = haystack.to_lowercase();
        query
            .split_whitespace()
            .all(|w| haystack.contains(&w.to_lowercase()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub source: SourceInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<String>,
    #[serde(default)]
    pub plugins: Vec<CatalogPlugin>,
}

impl Catalog {
    pub fn new(name: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            source: SourceInfo {
                name: name.to_owned(),
                ..SourceInfo::default()
            },
            generated: None,
            plugins: Vec::new(),
        }
    }

    pub fn plugin(&self, id: &str) -> Option<&CatalogPlugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    /// Pretty JSON (what `cargo xtask plugin-index` writes).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Every tag, sorted.
    pub fn tags(&self) -> BTreeSet<String> {
        self.plugins
            .iter()
            .flat_map(|p| p.tags.iter().cloned())
            .collect()
    }
}

/// Why a catalog cannot be used at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    /// Not JSON, or not a catalog object.
    Invalid(String),
    /// A newer schema than this editor reads.
    TooNew(u32),
    TooLarge,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "invalid catalog: {e}"),
            Self::TooNew(v) => write!(
                f,
                "catalog schema {v} needs a newer editor (this one reads {SCHEMA_VERSION})"
            ),
            Self::TooLarge => write!(
                f,
                "catalog larger than {} MB",
                MAX_CATALOG_BYTES / (1024 * 1024)
            ),
        }
    }
}

/// A usable catalog and what was skipped in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCatalog {
    pub catalog: Catalog,
    pub warnings: Vec<String>,
}

/// Plugin ids: lowercase dotted names (`acme.hello`), as the plugin host
/// requires.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `https://`, `http://`, `file://` or a relative path (no other scheme).
pub fn valid_url(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() || url.chars().any(|c| c.is_control() || c == ' ') {
        return false;
    }
    if let Some((scheme, _)) = url.split_once("://") {
        return matches!(scheme, "https" | "http" | "file");
    }
    // A relative reference: no scheme before the first `/`.
    !url.split('/').next().unwrap_or_default().contains(':')
}

/// Ids starting with this are only accepted from the official source.
pub const RESERVED_PREFIX: &str = "orbit.";

fn check_build(b: &mut Build) -> Result<(), String> {
    b.fingerprint = b.fingerprint.trim().to_ascii_lowercase();
    b.sha256 = b.sha256.trim().to_ascii_lowercase();
    if let Some(s) = &mut b.signature {
        *s = s.trim().to_ascii_lowercase();
    }
    if !is_hex(&b.fingerprint, 32) {
        return Err(format!("bad editor build id `{}`", b.fingerprint));
    }
    if !is_hex(&b.sha256, 64) {
        return Err("bad sha256".into());
    }
    if b.signature.as_deref().is_some_and(|s| !is_hex(s, 128)) {
        return Err("bad signature".into());
    }
    if b.size == 0 || b.size > MAX_PACKAGE_BYTES {
        return Err(format!("bad size {}", b.size));
    }
    if !valid_url(&b.url) {
        return Err(format!("bad url `{}`", b.url));
    }
    Ok(())
}

fn check_plugin(
    p: &mut CatalogPlugin,
    official: bool,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    if !valid_id(&p.id) {
        return Err(format!("invalid id `{}`", p.id));
    }
    if !official && p.id.starts_with(RESERVED_PREFIX) {
        return Err(format!(
            "`{}`: ids `{RESERVED_PREFIX}*` are reserved to the official source",
            p.id
        ));
    }
    if p.name.trim().is_empty() {
        return Err(format!("`{}`: no name", p.id));
    }
    for t in &mut p.tags {
        *t = t.trim().to_lowercase();
    }
    p.tags.retain(|t| !t.is_empty());
    p.tags.sort();
    p.tags.dedup();
    let mut seen = BTreeSet::new();
    let id = p.id.clone();
    p.versions.retain_mut(|v| {
        if Version::parse(&v.version).is_none() {
            warnings.push(format!("{id}: invalid version `{}` skipped", v.version));
            return false;
        }
        if !seen.insert(v.version.clone()) {
            warnings.push(format!(
                "{id}: version {} listed twice, the second skipped",
                v.version
            ));
            return false;
        }
        let version = v.version.clone();
        v.builds.retain_mut(|b| match check_build(b) {
            Ok(()) => true,
            Err(e) => {
                warnings.push(format!("{id} {version}: build skipped ({e})"));
                false
            }
        });
        true
    });
    // Newest first.
    p.versions.sort_by(|a, b| compare(&b.version, &a.version));
    Ok(())
}

/// Parses and validates a catalog. `official`: it comes from the official
/// source (`orbit.*` ids allowed).
pub fn parse(text: &str, official: bool) -> Result<ParsedCatalog, CatalogError> {
    if text.len() > MAX_CATALOG_BYTES {
        return Err(CatalogError::TooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| CatalogError::Invalid(e.to_string()))?;
    let obj = value
        .as_object()
        .ok_or_else(|| CatalogError::Invalid("not a JSON object".into()))?;
    let schema = obj
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| CatalogError::Invalid("no schema_version".into()))?;
    if schema > SCHEMA_VERSION as u64 {
        return Err(CatalogError::TooNew(schema.min(u32::MAX as u64) as u32));
    }
    let source: SourceInfo = obj
        .get("source")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| CatalogError::Invalid(format!("source: {e}")))?
        .ok_or_else(|| CatalogError::Invalid("no source".into()))?;
    let generated = obj
        .get("generated")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let mut warnings = Vec::new();
    let mut plugins: Vec<CatalogPlugin> = Vec::new();
    let entries = match obj.get("plugins") {
        None => Vec::new(),
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(_) => return Err(CatalogError::Invalid("`plugins` is not a list".into())),
    };
    for (i, entry) in entries.into_iter().enumerate() {
        let mut plugin: CatalogPlugin = match serde_json::from_value(entry) {
            Ok(p) => p,
            Err(e) => {
                warnings.push(format!("plugin #{}: skipped ({e})", i + 1));
                continue;
            }
        };
        if let Err(e) = check_plugin(&mut plugin, official, &mut warnings) {
            warnings.push(format!("plugin #{}: skipped ({e})", i + 1));
            continue;
        }
        if plugins.iter().any(|p| p.id == plugin.id) {
            warnings.push(format!("{}: listed twice, the second skipped", plugin.id));
            continue;
        }
        plugins.push(plugin);
    }
    Ok(ParsedCatalog {
        catalog: Catalog {
            schema_version: schema as u32,
            source,
            generated,
            plugins,
        },
        warnings,
    })
}

/// What a plugin offers to one editor build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability<'a> {
    /// The newest version that has a build for this editor.
    Available {
        version: &'a CatalogVersion,
        build: &'a Build,
        /// A newer version exists, but not for this editor.
        newer_elsewhere: Option<&'a CatalogVersion>,
    },
    /// No version has a build for this editor.
    NoBuild { latest: Option<&'a CatalogVersion> },
}

impl<'a> Availability<'a> {
    pub fn build(&self) -> Option<(&'a CatalogVersion, &'a Build)> {
        match *self {
            Self::Available { version, build, .. } => Some((version, build)),
            Self::NoBuild { .. } => None,
        }
    }
}

/// Picks, for `plugin`, the newest version with a build whose editor build id
/// equals `fingerprint`.
pub fn select<'a>(plugin: &'a CatalogPlugin, fingerprint: &str) -> Availability<'a> {
    let latest = plugin.latest();
    let best = plugin
        .versions
        .iter()
        .filter_map(|v| {
            v.builds
                .iter()
                .find(|b| b.fingerprint == fingerprint)
                .map(|b| (v, b))
        })
        .max_by(|a, b| compare(&a.0.version, &b.0.version));
    match best {
        Some((version, build)) => Availability::Available {
            version,
            build,
            newer_elsewhere: latest.filter(|l| compare(&l.version, &version.version).is_gt()),
        },
        None => Availability::NoBuild { latest },
    }
}

/// The editor versions that have a build of `plugin` (sorted, newest first).
pub fn editor_versions(plugin: &CatalogPlugin) -> Vec<String> {
    let mut out: Vec<String> = plugin
        .versions
        .iter()
        .flat_map(|v| v.builds.iter().map(|b| b.editor_version.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    out.sort_by(|a, b| compare(b, a));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "0123456789abcdef0123456789abcdef";
    const OTHER: &str = "ffffffffffffffffffffffffffffffff";

    fn build(fp: &str, editor: &str) -> serde_json::Value {
        serde_json::json!({
            "editor_version": editor, "fingerprint": fp, "target": "x86_64-pc-windows-msvc",
            "profile": "release", "url": "https://example.org/a.orbitplugin",
            "sha256": "AB".repeat(32), "size": 10, "signature": null
        })
    }

    fn sample() -> String {
        serde_json::json!({
            "schema_version": 1,
            "source": { "name": "Test", "description": { "en": "Hi", "fr": "Salut" } },
            "future_field": true,
            "plugins": [
                {
                    "id": "acme.hello", "name": "Hello", "description": { "en": "Says hello", "fr": "Dit bonjour" },
                    "author": "ACME", "tags": ["Tools", "tools", " ui "],
                    "versions": [
                        { "version": "0.1.0", "changelog": "first", "builds": [build(FP, "0.4.0")] },
                        { "version": "0.3.0", "builds": [build(OTHER, "0.5.0")] },
                        { "version": "0.2.0", "builds": [build(FP, "0.4.0"), build("nothex", "0.4.0")] },
                        { "version": "bad", "builds": [] },
                        { "version": "0.1.0", "builds": [] }
                    ]
                },
                { "id": "Bad Id", "name": "X" },
                { "id": "orbit.fake", "name": "Fake" },
                { "id": "acme.hello", "name": "Twice" },
                { "name": "no id" },
                { "id": "acme.other", "name": "Other", "description": "plain text",
                  "versions": [ { "version": "1.0.0", "builds": [build(OTHER, "0.5.0")] } ] }
            ]
        })
        .to_string()
    }

    #[test]
    fn catalogs_parse_and_skip_bad_entries() {
        let parsed = parse(&sample(), false).unwrap();
        let c = &parsed.catalog;
        assert_eq!(c.source.name, "Test");
        assert_eq!(c.source.description.get("fr"), "Salut");
        let ids: Vec<&str> = c.plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["acme.hello", "acme.other"]);
        let hello = c.plugin("acme.hello").unwrap();
        assert_eq!(hello.tags, ["tools", "ui"]);
        let versions: Vec<&str> = hello.versions.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(versions, ["0.3.0", "0.2.0", "0.1.0"]);
        // Lowercased hashes, the bad build dropped.
        assert_eq!(hello.versions[1].builds.len(), 1);
        assert_eq!(hello.versions[1].builds[0].sha256, "ab".repeat(32));
        assert_eq!(hello.description.get("de"), "Says hello");
        assert_eq!(
            c.plugin("acme.other").unwrap().description.get("fr"),
            "plain text"
        );
        let w = parsed.warnings.join("\n");
        for expected in [
            "Bad Id",
            "reserved",
            "listed twice",
            "invalid version",
            "build skipped",
        ] {
            assert!(w.contains(expected), "missing warning {expected}: {w}");
        }
        // The official source may use `orbit.*`.
        let official = parse(&sample(), true).unwrap();
        assert!(official.catalog.plugin("orbit.fake").is_some());
        // Round trip of what xtask writes.
        let again = parse(&c.to_json(), false).unwrap();
        assert_eq!(&again.catalog, c);
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    }

    #[test]
    fn broken_or_newer_catalogs_are_refused() {
        assert!(matches!(
            parse("not json", false),
            Err(CatalogError::Invalid(_))
        ));
        assert!(matches!(parse("[]", false), Err(CatalogError::Invalid(_))));
        assert!(matches!(
            parse(r#"{"source":{"name":"x"}}"#, false),
            Err(CatalogError::Invalid(_))
        ));
        assert!(matches!(
            parse(r#"{"schema_version":1}"#, false),
            Err(CatalogError::Invalid(_))
        ));
        assert_eq!(
            parse(r#"{"schema_version":2,"source":{"name":"x"}}"#, false),
            Err(CatalogError::TooNew(2))
        );
        let empty = parse(
            r#"{"schema_version":1,"source":{"name":"x"},"plugins":[]}"#,
            false,
        )
        .unwrap();
        assert!(empty.catalog.plugins.is_empty());
        let big = " ".repeat(MAX_CATALOG_BYTES + 1);
        assert_eq!(parse(&big, false), Err(CatalogError::TooLarge));
    }

    #[test]
    fn the_build_for_this_editor_is_chosen() {
        let c = parse(&sample(), false).unwrap().catalog;
        let hello = c.plugin("acme.hello").unwrap();
        match select(hello, FP) {
            Availability::Available {
                version,
                build,
                newer_elsewhere,
            } => {
                assert_eq!(version.version, "0.2.0");
                assert_eq!(build.fingerprint, FP);
                assert_eq!(newer_elsewhere.map(|v| v.version.as_str()), Some("0.3.0"));
            }
            other => panic!("{other:?}"),
        }
        match select(hello, OTHER) {
            Availability::Available {
                version,
                newer_elsewhere,
                ..
            } => {
                assert_eq!(version.version, "0.3.0");
                assert_eq!(newer_elsewhere, None);
            }
            other => panic!("{other:?}"),
        }
        let other = c.plugin("acme.other").unwrap();
        assert!(
            matches!(select(other, FP), Availability::NoBuild { latest: Some(v) } if v.version == "1.0.0")
        );
        assert_eq!(editor_versions(hello), ["0.5.0", "0.4.0"]);
        assert!(hello.matches("hello acme"));
        assert!(hello.matches("BONJOUR"));
        assert!(!hello.matches("goodbye"));
    }

    #[test]
    fn urls_and_ids_are_checked() {
        for ok in [
            "https://x/y",
            "http://127.0.0.1:1/a",
            "file:///E:/a",
            "builds/a.orbitplugin",
            "../a",
        ] {
            assert!(valid_url(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "ftp://x",
            "",
            "a b",
            "data:text/plain,x",
        ] {
            assert!(!valid_url(bad), "{bad}");
        }
        assert!(valid_id("acme.hello_2"));
        assert!(!valid_id("acme..x"));
        assert!(!valid_id("Acme.x"));
    }
}
