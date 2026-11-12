//! The `game_plugins` array of a catalog (`index.json`, still schema 1).
//!
//! It is a separate top-level array, not a kind of `plugins` entry, so that
//! an editor that predates game plugins ignores it entirely (unknown fields
//! are ignored) instead of listing source packages as editor plugins with
//! "no build for your editor". Parsing follows the catalog rules: a bad
//! entry is skipped with a warning, a `schema_version` the editor does not
//! know refuses the whole catalog, and a broken `game_plugins` array never
//! makes the editor plugins of the same catalog unusable (it is parsed on
//! its own).
//!
//! Compatibility is not an editor build id: a game plugin is compiled at
//! export with the engine sources, so a version is offered when its
//! `api_version` equals `orbit_plugin_api::API_VERSION` and the engine
//! version is in `[min_engine, max_engine)`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::PackageSource;
use super::package::MAX_PACKAGE_BYTES;
use crate::StorePaths;
use crate::catalog::{
    Catalog, CatalogError, Localized, MAX_CATALOG_BYTES, RESERVED_PREFIX, SCHEMA_VERSION, valid_id,
    valid_url,
};
use crate::fetch::CacheMeta;
use crate::signing::{SignatureError, SignatureState, SignatureVerifier};
use crate::sources::{SourceEntry, Trust};
use crate::version::{Version, compare, is_newer};

/// The catalog field listing game plugins.
pub const FIELD: &str = "game_plugins";

/// One version of a game plugin: one source package (sources build on every
/// platform, so there is no per-target build).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameVersion {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released: Option<String>,
    #[serde(default)]
    pub changelog: Localized,
    /// The `orbit_plugin_api::API_VERSION` the crate is written against.
    pub api_version: u32,
    /// Oldest engine version it builds with (inclusive).
    pub min_engine: String,
    /// First engine version it no longer builds with (exclusive), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_engine: Option<String>,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// Ed25519 signature (hex) of [`package_message`], or `null`.
    #[serde(default)]
    pub signature: Option<String>,
}

impl GameVersion {
    /// Whether this version builds with `engine` (version text) and the
    /// plugin API `api_version`.
    pub fn compatible(&self, engine: &str, api_version: u32) -> bool {
        self.api_version == api_version
            && engine_in_range(engine, &self.min_engine, self.max_engine.as_deref())
    }
}

/// `min <= engine < max` (no `max`: no upper bound).
pub fn engine_in_range(engine: &str, min: &str, max: Option<&str>) -> bool {
    let (Some(e), Some(lo)) = (Version::parse(engine), Version::parse(min)) else {
        return false;
    };
    if e < lo {
        return false;
    }
    match max.map(Version::parse) {
        None => true,
        Some(Some(hi)) => e < hi,
        Some(None) => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamePlugin {
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
    pub versions: Vec<GameVersion>,
}

impl GamePlugin {
    /// The newest version (any engine).
    pub fn latest(&self) -> Option<&GameVersion> {
        self.versions
            .iter()
            .max_by(|a, b| compare(&a.version, &b.version))
    }

    /// Whether `query` (words) matches the name, id, author, tags or
    /// description.
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

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn check_version(v: &mut GameVersion) -> Result<(), String> {
    v.sha256 = v.sha256.trim().to_ascii_lowercase();
    if let Some(s) = &mut v.signature {
        *s = s.trim().to_ascii_lowercase();
    }
    if v.api_version == 0 {
        return Err("api_version 0".into());
    }
    let Some(min) = Version::parse(&v.min_engine) else {
        return Err(format!("bad min_engine `{}`", v.min_engine));
    };
    if let Some(max) = &v.max_engine {
        match Version::parse(max) {
            Some(max) if max > min => {}
            _ => return Err(format!("bad max_engine `{max}`")),
        }
    }
    if !is_hex(&v.sha256, 64) {
        return Err("bad sha256".into());
    }
    if v.signature.as_deref().is_some_and(|s| !is_hex(s, 128)) {
        return Err("bad signature".into());
    }
    if v.size == 0 || v.size > MAX_PACKAGE_BYTES {
        return Err(format!("bad size {}", v.size));
    }
    if !valid_url(&v.url) {
        return Err(format!("bad url `{}`", v.url));
    }
    Ok(())
}

fn check_plugin(
    p: &mut GamePlugin,
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
            warnings.push(format!(
                "game plugin {id}: invalid version `{}` skipped",
                v.version
            ));
            return false;
        }
        if !seen.insert(v.version.clone()) {
            warnings.push(format!(
                "game plugin {id}: version {} listed twice, the second skipped",
                v.version
            ));
            return false;
        }
        match check_version(v) {
            Ok(()) => true,
            Err(e) => {
                warnings.push(format!("game plugin {id} {}: skipped ({e})", v.version));
                false
            }
        }
    });
    p.versions.sort_by(|a, b| compare(&b.version, &a.version));
    Ok(())
}

/// The game plugins of a catalog text (an absent `game_plugins`: none).
/// `official`: the catalog comes from the official source.
pub fn parse_game_plugins(
    text: &str,
    official: bool,
) -> Result<(Vec<GamePlugin>, Vec<String>), CatalogError> {
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
    let mut warnings = Vec::new();
    let entries = match obj.get(FIELD) {
        None | Some(serde_json::Value::Null) => return Ok((Vec::new(), warnings)),
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(_) => {
            warnings.push(format!("`{FIELD}` is not a list: ignored"));
            return Ok((Vec::new(), warnings));
        }
    };
    let mut plugins: Vec<GamePlugin> = Vec::new();
    for (i, entry) in entries.into_iter().enumerate() {
        let mut plugin: GamePlugin = match serde_json::from_value(entry) {
            Ok(p) => p,
            Err(e) => {
                warnings.push(format!("game plugin #{}: skipped ({e})", i + 1));
                continue;
            }
        };
        if let Err(e) = check_plugin(&mut plugin, official, &mut warnings) {
            warnings.push(format!("game plugin #{}: skipped ({e})", i + 1));
            continue;
        }
        if plugins.iter().any(|p| p.id == plugin.id) {
            warnings.push(format!(
                "game plugin {}: listed twice, the second skipped",
                plugin.id
            ));
            continue;
        }
        plugins.push(plugin);
    }
    Ok((plugins, warnings))
}

/// The game plugins of the cached catalog of `source` (no network); `None`
/// when nothing usable is cached.
pub fn cached(paths: &StorePaths, source: &SourceEntry) -> Option<(Vec<GamePlugin>, Vec<String>)> {
    let dir = paths.cache_dir();
    let meta: CacheMeta =
        ron::from_str(&std::fs::read_to_string(dir.join(format!("{}.meta.ron", source.id))).ok()?)
            .ok()?;
    if meta.url != source.url {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(format!("{}.json", source.id))).ok()?;
    parse_game_plugins(&text, source.builtin).ok()
}

/// A catalog with its game plugins, as `cargo xtask plugin-index` and
/// `game-plugin index` write it (no `game_plugins` field when there are
/// none, so a catalog without game plugins is byte-identical to before).
pub fn catalog_json(catalog: &Catalog, games: &[GamePlugin]) -> String {
    #[derive(Serialize)]
    struct WithGames<'a> {
        #[serde(flatten)]
        catalog: &'a Catalog,
        game_plugins: &'a [GamePlugin],
    }
    if games.is_empty() {
        return catalog.to_json();
    }
    serde_json::to_string_pretty(&WithGames {
        catalog,
        game_plugins: games,
    })
    .unwrap_or_default()
        + "\n"
}

/// What a game plugin offers to this engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameAvailability<'a> {
    /// The newest version compatible with this engine.
    Available {
        version: &'a GameVersion,
        /// A newer version exists, not for this engine.
        newer_elsewhere: Option<&'a GameVersion>,
    },
    /// No version builds with this engine.
    Incompatible { latest: Option<&'a GameVersion> },
}

impl<'a> GameAvailability<'a> {
    pub fn version(&self) -> Option<&'a GameVersion> {
        match *self {
            Self::Available { version, .. } => Some(version),
            Self::Incompatible { .. } => None,
        }
    }
}

/// Picks the newest version of `plugin` that builds with `engine` (version)
/// and the plugin API `api_version`.
pub fn select_game<'a>(
    plugin: &'a GamePlugin,
    engine: &str,
    api_version: u32,
) -> GameAvailability<'a> {
    let latest = plugin.latest();
    let best = plugin
        .versions
        .iter()
        .filter(|v| v.compatible(engine, api_version))
        .max_by(|a, b| compare(&a.version, &b.version));
    match best {
        Some(version) => GameAvailability::Available {
            version,
            newer_elsewhere: latest.filter(|l| compare(&l.version, &version.version).is_gt()),
        },
        None => GameAvailability::Incompatible { latest },
    }
}

// ------------------------------------------------------------------ signatures

/// The message a game package signature covers (every line ends with
/// `\n`). Its first line differs from editor builds (`orbit-plugin-build/v1`),
/// so a signature of one kind is never valid for the other.
pub fn package_message(id: &str, version: &str, api_version: u32, sha256: &str) -> String {
    format!(
        "orbit-game-plugin/v1\nid={id}\nversion={version}\napi_version={api_version}\nsha256={}\n",
        sha256.to_ascii_lowercase()
    )
}

/// Signs a package (hex signature).
pub fn sign_package(
    seed: &[u8; 32],
    id: &str,
    version: &str,
    api_version: u32,
    sha256: &str,
) -> String {
    let message = package_message(id, version, api_version, sha256);
    orbit_sign::hex(&orbit_sign::ed25519::sign(seed, message.as_bytes()))
}

/// Checks the signature of `version` of `id` for `source` (same rules as
/// editor builds: Official and Trusted need a valid signature by their key,
/// Unsigned checks nothing but the SHA-256 and size).
pub fn check_package(
    source: &PackageSource,
    id: &str,
    version: &GameVersion,
    verifier: &dyn SignatureVerifier,
) -> Result<SignatureState, SignatureError> {
    if source.trust == Trust::Unsigned {
        return Ok(SignatureState::NotChecked);
    }
    let key = source.key.as_ref().ok_or(SignatureError::NoKey)?;
    let signature = version
        .signature
        .as_deref()
        .ok_or(SignatureError::Missing)?;
    let signature = orbit_sign::from_hex::<64>(signature).ok_or(SignatureError::Invalid)?;
    let message = package_message(id, &version.version, version.api_version, &version.sha256);
    if verifier.verify(key, message.as_bytes(), &signature) {
        Ok(SignatureState::Valid)
    } else {
        Err(SignatureError::Invalid)
    }
}

// ------------------------------------------------------------------ updates

/// A newer compatible version of a game plugin installed in a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameUpdate {
    pub id: String,
    pub installed: String,
    pub available: String,
    /// Source id.
    pub source: String,
}

/// The source a recorded install takes its updates from: the one it came
/// from (same id, or same URL when the project comes from another machine).
pub fn update_source<'a>(
    record_source_id: &str,
    record_source_url: &str,
    sources: impl IntoIterator<Item = &'a SourceEntry>,
) -> Option<&'a SourceEntry> {
    let mut by_url = None;
    for s in sources {
        if s.id == record_source_id {
            return Some(s);
        }
        if by_url.is_none() && s.url == record_source_url {
            by_url = Some(s);
        }
    }
    by_url
}

/// The updates of `installed` (id, version, source id, source URL: only
/// store installs; a crate added by hand is never "updated") in `catalogs`
/// (enabled sources).
pub fn find_game_updates(
    installed: &[(String, String, String, String)],
    catalogs: &[(&SourceEntry, &[GamePlugin])],
    engine: &str,
    api_version: u32,
) -> Vec<GameUpdate> {
    let mut out = Vec::new();
    for (id, version, source_id, source_url) in installed {
        let Some(source) = update_source(source_id, source_url, catalogs.iter().map(|(s, _)| *s))
        else {
            continue;
        };
        let Some((_, games)) = catalogs.iter().find(|(s, _)| s.id == source.id) else {
            continue;
        };
        let Some(plugin) = games.iter().find(|p| &p.id == id) else {
            continue;
        };
        if let Some(v) = select_game(plugin, engine, api_version).version()
            && is_newer(&v.version, version)
        {
            out.push(GameUpdate {
                id: id.clone(),
                installed: version.clone(),
                available: v.version.clone(),
                source: source.id.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::parse;
    use crate::signing::{Ed25519, public_key_hex};
    use crate::sources::Sources;

    fn version(v: &str, api: u32, min: &str, max: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "version": v, "api_version": api, "min_engine": min, "max_engine": max,
            "url": "packages/x.orbitsrc", "sha256": "AB".repeat(32), "size": 10, "signature": null
        })
    }

    fn sample() -> String {
        serde_json::json!({
            "schema_version": 1,
            "source": { "name": "Test" },
            "plugins": [ { "id": "acme.editor", "name": "Editor plugin", "versions": [] } ],
            "game_plugins": [
                { "id": "acme.dice", "name": "Dice", "description": { "en": "Rolls dice", "fr": "Lance des dés" },
                  "tags": ["Game", "game"], "future": 1,
                  "versions": [
                      version("1.0.0", 1, "0.4.0", Some("0.5.0")),
                      version("1.1.0", 1, "0.5.0", None),
                      version("2.0.0", 2, "0.6.0", None),
                      version("0.9.0", 1, "0.3.0", Some("0.3.0")),
                      version("bad", 1, "0.4.0", None),
                      version("1.0.0", 1, "0.4.0", None)
                  ] },
                { "id": "orbit.fake", "name": "Fake" },
                { "id": "acme.dice", "name": "Twice" },
                { "name": "no id" },
                { "id": "acme.cards", "name": "Cards", "versions": [ version("0.1.0", 1, "0.4.0", None) ] }
            ]
        })
        .to_string()
    }

    #[test]
    fn game_plugins_parse_and_skip_bad_entries() {
        let (games, warnings) = parse_game_plugins(&sample(), false).unwrap();
        let ids: Vec<&str> = games.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["acme.dice", "acme.cards"]);
        let dice = &games[0];
        assert_eq!(dice.tags, ["game"]);
        let versions: Vec<&str> = dice.versions.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(versions, ["2.0.0", "1.1.0", "1.0.0"]);
        assert_eq!(dice.versions[2].sha256, "ab".repeat(32));
        let w = warnings.join("\n");
        for expected in [
            "reserved",
            "listed twice",
            "invalid version",
            "bad max_engine",
            "missing field `id`",
        ] {
            assert!(w.contains(expected), "{expected}: {w}");
        }
        // The official source may use `orbit.*`.
        let (official, _) = parse_game_plugins(&sample(), true).unwrap();
        assert!(official.iter().any(|p| p.id == "orbit.fake"));
    }

    #[test]
    fn catalogs_stay_backward_and_forward_compatible() {
        // A catalog without the field: no game plugin, no warning.
        let old = r#"{"schema_version":1,"source":{"name":"x"},"plugins":[]}"#;
        assert_eq!(
            parse_game_plugins(old, false).unwrap(),
            (Vec::new(), Vec::new())
        );
        // The editor-plugin parser ignores `game_plugins` (what an editor
        // that predates it does): the catalog stays valid, no warning.
        let editor = parse(&sample(), false).unwrap();
        assert_eq!(editor.catalog.plugins.len(), 1);
        assert!(editor.warnings.is_empty(), "{:?}", editor.warnings);
        // A broken `game_plugins` is ignored with a warning, the editor
        // plugins of the catalog are untouched.
        let broken =
            r#"{"schema_version":1,"source":{"name":"x"},"plugins":[],"game_plugins":{"a":1}}"#;
        let (games, w) = parse_game_plugins(broken, false).unwrap();
        assert!(games.is_empty() && w.len() == 1);
        assert!(parse(broken, false).is_ok());
        // Same schema rules as the catalog.
        assert_eq!(
            parse_game_plugins(r#"{"schema_version":2,"source":{"name":"x"}}"#, false),
            Err(CatalogError::TooNew(2))
        );
        // Round trip of what xtask writes, game plugins kept after the
        // editor plugins; none: byte-identical to the plain catalog.
        let (games, _) = parse_game_plugins(&sample(), false).unwrap();
        let text = catalog_json(&editor.catalog, &games);
        assert_eq!(parse(&text, false).unwrap().catalog, editor.catalog);
        assert_eq!(
            parse_game_plugins(&text, false).unwrap(),
            (games, Vec::new())
        );
        assert!(text.find("\"plugins\"").unwrap() < text.find("\"game_plugins\"").unwrap());
        assert_eq!(catalog_json(&editor.catalog, &[]), editor.catalog.to_json());
    }

    #[test]
    fn the_version_for_this_engine_is_chosen() {
        let (games, _) = parse_game_plugins(&sample(), false).unwrap();
        let dice = &games[0];
        let pick = |engine: &str, api| select_game(dice, engine, api);
        assert_eq!(pick("0.4.0", 1).version().unwrap().version, "1.0.0");
        assert_eq!(pick("0.4.7", 1).version().unwrap().version, "1.0.0");
        assert_eq!(pick("0.5.0", 1).version().unwrap().version, "1.1.0");
        assert_eq!(pick("0.6.0", 2).version().unwrap().version, "2.0.0");
        assert!(matches!(
            pick("0.4.0", 1),
            GameAvailability::Available { newer_elsewhere: Some(v), .. } if v.version == "2.0.0"
        ));
        assert!(matches!(
            pick("0.3.0", 1),
            GameAvailability::Incompatible { latest: Some(v) } if v.version == "2.0.0"
        ));
        assert!(engine_in_range("0.5.0-beta", "0.4.0", Some("0.5.0")));
        assert!(!engine_in_range("garbage", "0.4.0", None));
        assert!(dice.matches("DÉS"));
        assert!(dice.matches("dice acme"));
    }

    #[test]
    fn package_signatures_are_checked_by_trust_and_kind() {
        let seed = [5u8; 32];
        let key = crate::signing::parse_public_key(&public_key_hex(&seed)).unwrap();
        let (games, _) = parse_game_plugins(&sample(), false).unwrap();
        let mut v = games[0].versions[2].clone();
        v.signature = Some(sign_package(&seed, "acme.dice", &v.version, 1, &v.sha256));
        let source = |trust, key| PackageSource {
            id: "s".into(),
            name: "S".into(),
            url: "file:///x".into(),
            trust,
            key,
            official: false,
        };
        let trusted = source(Trust::Trusted, Some(key));
        assert_eq!(
            check_package(&trusted, "acme.dice", &v, &Ed25519),
            Ok(SignatureState::Valid)
        );
        assert_eq!(
            check_package(&trusted, "acme.cards", &v, &Ed25519),
            Err(SignatureError::Invalid)
        );
        let mut other_api = v.clone();
        other_api.api_version = 2;
        assert_eq!(
            check_package(&trusted, "acme.dice", &other_api, &Ed25519),
            Err(SignatureError::Invalid)
        );
        // An editor build signature over the same id / version / hash is
        // not a package signature.
        let mut editor_sig = v.clone();
        editor_sig.signature = Some(crate::signing::sign_build(
            &seed,
            "acme.dice",
            &v.version,
            &"0".repeat(32),
            &v.sha256,
        ));
        assert_eq!(
            check_package(&trusted, "acme.dice", &editor_sig, &Ed25519),
            Err(SignatureError::Invalid)
        );
        let mut unsigned = v.clone();
        unsigned.signature = None;
        assert_eq!(
            check_package(&trusted, "acme.dice", &unsigned, &Ed25519),
            Err(SignatureError::Missing)
        );
        assert_eq!(
            check_package(&source(Trust::Official, None), "acme.dice", &v, &Ed25519),
            Err(SignatureError::NoKey)
        );
        assert_eq!(
            check_package(
                &source(Trust::Unsigned, None),
                "acme.dice",
                &unsigned,
                &Ed25519
            ),
            Ok(SignatureState::NotChecked)
        );
    }

    #[test]
    fn updates_come_from_the_install_source_only() {
        let mut sources = Sources::default();
        let third = sources
            .add("https://example.org/index.json", "Third", "")
            .unwrap();
        let official = sources.sources[0].clone();
        let (games, _) = parse_game_plugins(&sample(), false).unwrap();
        let catalogs: [(&SourceEntry, &[GamePlugin]); 2] =
            [(&official, &games), (&third, &games[..1])];
        let installed = vec![(
            "acme.dice".to_owned(),
            "0.9.0".to_owned(),
            third.id.clone(),
            third.url.clone(),
        )];
        let updates = find_game_updates(&installed, &catalogs, "0.4.0", 1);
        assert_eq!(updates.len(), 1);
        assert_eq!(
            (updates[0].available.as_str(), updates[0].source.as_str()),
            ("1.0.0", third.id.as_str())
        );
        // Another machine: the source has another id but the same URL.
        let moved = vec![(
            "acme.dice".to_owned(),
            "0.9.0".to_owned(),
            "src-elsewhere".to_owned(),
            third.url.clone(),
        )];
        assert_eq!(
            find_game_updates(&moved, &catalogs, "0.4.0", 1)[0].source,
            third.id
        );
        // Unknown source: nothing (no silent switch to another catalog).
        let unknown = vec![(
            "acme.dice".to_owned(),
            "0.9.0".to_owned(),
            "gone".to_owned(),
            "https://gone/index.json".to_owned(),
        )];
        assert!(find_game_updates(&unknown, &catalogs, "0.4.0", 1).is_empty());
        // Up to date.
        let current = vec![(
            "acme.dice".to_owned(),
            "1.0.0".to_owned(),
            third.id.clone(),
            third.url.clone(),
        )];
        assert!(find_game_updates(&current, &catalogs, "0.4.0", 1).is_empty());
    }
}
