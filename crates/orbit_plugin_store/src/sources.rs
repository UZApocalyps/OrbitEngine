//! Catalog sources (`<config>/plugin_store/sources.ron`): the built-in
//! official one (always present, can be disabled, not removed nor changed)
//! and the ones the user adds by URL, with an optional public key.

use serde::{Deserialize, Serialize};

use crate::signing::{official_key, parse_public_key};
use crate::{StorePaths, write_atomic};

/// Id of the built-in official source.
pub const OFFICIAL_ID: &str = "official";
/// The official catalog: `index.json` on the `main` branch of the official
/// plugins repository.
pub const OFFICIAL_URL: &str =
    "https://raw.githubusercontent.com/UZApocalyps/OrbitEnginePlugins/main/index.json";
pub const OFFICIAL_NAME: &str = "OrbitEngine";
/// The official repository (homepage).
pub const OFFICIAL_REPO: &str = "https://github.com/UZApocalyps/OrbitEnginePlugins";

/// How far a source's builds are trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Trust {
    /// The built-in source: builds signed by the official key embedded in the
    /// editor.
    Official,
    /// Added with a public key: builds signed by that key.
    Trusted,
    /// Added without a key: only the SHA-256 of the catalog is checked.
    Unsigned,
}

impl Trust {
    pub fn key(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Trusted => "trusted",
            Self::Unsigned => "unsigned",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntry {
    pub id: String,
    pub name: String,
    /// The `index.json` URL (resolved when added).
    pub url: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Ed25519 public key (hex) of a user-added source.
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub builtin: bool,
}

fn yes() -> bool {
    true
}

impl SourceEntry {
    pub fn official() -> Self {
        Self {
            id: OFFICIAL_ID.to_owned(),
            name: OFFICIAL_NAME.to_owned(),
            url: OFFICIAL_URL.to_owned(),
            enabled: true,
            public_key: None,
            builtin: true,
        }
    }

    pub fn trust(&self) -> Trust {
        if self.builtin {
            Trust::Official
        } else if self
            .public_key
            .as_deref()
            .and_then(parse_public_key)
            .is_some()
        {
            Trust::Trusted
        } else {
            Trust::Unsigned
        }
    }

    /// The key its builds must be signed with.
    pub fn key(&self) -> Option<[u8; 32]> {
        if self.builtin {
            official_key()
        } else {
            self.public_key.as_deref().and_then(parse_public_key)
        }
    }
}

/// Why a source cannot be added or changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    BadUrl(String),
    BadKey,
    Duplicate(String),
    /// The official source cannot be removed.
    Builtin,
    Unknown(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadUrl(u) => write!(f, "not a catalog URL: {u}"),
            Self::BadKey => write!(f, "the public key must be 64 hexadecimal characters"),
            Self::Duplicate(name) => write!(f, "this catalog is already a source ({name})"),
            Self::Builtin => write!(f, "the official source cannot be removed"),
            Self::Unknown(id) => write!(f, "no source `{id}`"),
        }
    }
}

/// Turns what the user typed into a catalog URL:
///
/// * `https://github.com/owner/repo[.git][/]` → the raw `index.json` of
///   `main`;
/// * `https://github.com/owner/repo/tree/<branch>[/dir]` → `<dir>/index.json`
///   of that branch;
/// * `https://github.com/owner/repo/blob/<branch>/<path>` → that raw file;
/// * any other `http(s)://` or `file://` URL as is (`/index.json` appended
///   when it does not end in `.json`);
/// * a local path (absolute) → its `file://` URL.
pub fn resolve_catalog_url(input: &str) -> Result<String, SourceError> {
    let input = input.trim();
    let bad = || SourceError::BadUrl(input.to_owned());
    if input.is_empty()
        || input.chars().any(|c| c.is_control() || c == ' ') && !looks_like_path(input)
    {
        return Err(bad());
    }
    let github = input
        .strip_prefix("https://github.com/")
        .or_else(|| input.strip_prefix("http://github.com/"))
        .or_else(|| input.strip_prefix("https://www.github.com/"));
    if let Some(rest) = github {
        let rest = rest
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .trim_end_matches('/');
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() < 2 {
            return Err(bad());
        }
        let owner = parts[0];
        let repo = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
        let raw = format!("https://raw.githubusercontent.com/{owner}/{repo}");
        return match parts.get(2).copied() {
            None => Ok(format!("{raw}/main/index.json")),
            Some("tree") if parts.len() >= 4 => {
                let dir = parts[4..].join("/");
                let dir = if dir.is_empty() {
                    String::new()
                } else {
                    format!("{dir}/")
                };
                Ok(format!("{raw}/{}/{dir}index.json", parts[3]))
            }
            Some("blob" | "raw") if parts.len() >= 5 => {
                Ok(format!("{raw}/{}", parts[3..].join("/")))
            }
            _ => Err(bad()),
        };
    }
    if let Some((scheme, rest)) = input.split_once("://") {
        if !matches!(scheme, "https" | "http" | "file") || rest.is_empty() {
            return Err(bad());
        }
        let path = input.split(['?', '#']).next().unwrap_or(input);
        if path.to_ascii_lowercase().ends_with(".json") {
            return Ok(input.to_owned());
        }
        return Ok(format!("{}/index.json", input.trim_end_matches('/')));
    }
    if looks_like_path(input) {
        let path = std::path::Path::new(input);
        let path = if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            path.to_path_buf()
        } else {
            path.join("index.json")
        };
        return Ok(orbit_http::file_url(&path));
    }
    Err(bad())
}

fn looks_like_path(input: &str) -> bool {
    let b = input.as_bytes();
    input.starts_with('/')
        || input.starts_with("\\\\")
        || (b.len() > 2 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

/// A stable id for a user-added source.
fn source_id(url: &str) -> String {
    format!(
        "src-{}",
        &orbit_sign::sha256::digest_hex(url.as_bytes())[..12]
    )
}

/// A name from a catalog URL (`owner/repo` for GitHub).
pub fn default_name(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://raw.githubusercontent.com/") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 2 {
            return format!("{}/{}", parts[0], parts[1]);
        }
    }
    url.split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .filter(|h| !h.is_empty())
        .unwrap_or(url)
        .to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sources {
    pub sources: Vec<SourceEntry>,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            sources: vec![SourceEntry::official()],
        }
    }
}

impl Sources {
    /// Reads the sources; a missing or broken file gives the default list. The
    /// official source is always first and keeps its URL (only `enabled` is
    /// taken from the file).
    pub fn load(paths: &StorePaths) -> Self {
        let mut list: Sources = std::fs::read_to_string(paths.sources_file())
            .ok()
            .and_then(|t| ron::from_str(&t).ok())
            .unwrap_or_default();
        let enabled = list
            .sources
            .iter()
            .find(|s| s.builtin || s.id == OFFICIAL_ID)
            .is_none_or(|s| s.enabled);
        list.sources.retain(|s| !s.builtin && s.id != OFFICIAL_ID);
        let mut official = SourceEntry::official();
        official.enabled = enabled;
        list.sources.insert(0, official);
        let mut seen = std::collections::BTreeSet::new();
        list.sources.retain(|s| seen.insert(s.url.clone()));
        list
    }

    pub fn save(&self, paths: &StorePaths) -> std::io::Result<()> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(std::io::Error::other)?;
        write_atomic(&paths.sources_file(), text.as_bytes())
    }

    pub fn get(&self, id: &str) -> Option<&SourceEntry> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn enabled(&self) -> impl Iterator<Item = &SourceEntry> {
        self.sources.iter().filter(|s| s.enabled)
    }

    /// Adds a source from what the user typed (see [`resolve_catalog_url`]).
    /// `name` empty: derived from the URL. `public_key` empty: unsigned.
    pub fn add(
        &mut self,
        input: &str,
        name: &str,
        public_key: &str,
    ) -> Result<SourceEntry, SourceError> {
        let url = resolve_catalog_url(input)?;
        let public_key = match public_key.trim() {
            "" => None,
            k => Some(
                parse_public_key(k)
                    .map(|k| orbit_sign::hex(&k))
                    .ok_or(SourceError::BadKey)?,
            ),
        };
        if let Some(existing) = self.sources.iter().find(|s| s.url == url) {
            return Err(SourceError::Duplicate(existing.name.clone()));
        }
        let entry = SourceEntry {
            id: source_id(&url),
            name: if name.trim().is_empty() {
                default_name(&url)
            } else {
                name.trim().to_owned()
            },
            url,
            enabled: true,
            public_key,
            builtin: false,
        };
        self.sources.push(entry.clone());
        Ok(entry)
    }

    pub fn remove(&mut self, id: &str) -> Result<SourceEntry, SourceError> {
        let i = self
            .sources
            .iter()
            .position(|s| s.id == id)
            .ok_or_else(|| SourceError::Unknown(id.to_owned()))?;
        if self.sources[i].builtin {
            return Err(SourceError::Builtin);
        }
        Ok(self.sources.remove(i))
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), SourceError> {
        let s = self
            .sources
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| SourceError::Unknown(id.to_owned()))?;
        s.enabled = enabled;
        Ok(())
    }

    /// Sets (or clears, with an empty text) the key of a user-added source.
    pub fn set_key(&mut self, id: &str, public_key: &str) -> Result<(), SourceError> {
        let key = match public_key.trim() {
            "" => None,
            k => Some(
                parse_public_key(k)
                    .map(|k| orbit_sign::hex(&k))
                    .ok_or(SourceError::BadKey)?,
            ),
        };
        let s = self
            .sources
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| SourceError::Unknown(id.to_owned()))?;
        if s.builtin {
            return Err(SourceError::Builtin);
        }
        s.public_key = key;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::scratch;

    #[test]
    fn github_and_direct_urls_resolve_to_a_catalog() {
        let r = |s: &str| resolve_catalog_url(s).unwrap();
        assert_eq!(
            r("https://github.com/UZApocalyps/OrbitEnginePlugins"),
            OFFICIAL_URL
        );
        assert_eq!(
            r("https://github.com/UZApocalyps/OrbitEnginePlugins.git"),
            OFFICIAL_URL
        );
        assert_eq!(
            r("  https://github.com/UZApocalyps/OrbitEnginePlugins/  "),
            OFFICIAL_URL
        );
        assert_eq!(
            r("https://github.com/acme/cat/tree/dev"),
            "https://raw.githubusercontent.com/acme/cat/dev/index.json"
        );
        assert_eq!(
            r("https://github.com/acme/cat/tree/dev/catalog"),
            "https://raw.githubusercontent.com/acme/cat/dev/catalog/index.json"
        );
        assert_eq!(
            r("https://github.com/acme/cat/blob/main/store/plugins.json"),
            "https://raw.githubusercontent.com/acme/cat/main/store/plugins.json"
        );
        assert_eq!(
            r("https://example.org/plugins/index.json"),
            "https://example.org/plugins/index.json"
        );
        assert_eq!(
            r("https://example.org/plugins/"),
            "https://example.org/plugins/index.json"
        );
        assert_eq!(
            r("file:///E:/catalog/index.json"),
            "file:///E:/catalog/index.json"
        );
        assert!(r("E:\\my catalog").starts_with("file:///E:/my%20catalog/index.json"));
        for bad in [
            "",
            "github.com/x",
            "https://github.com/onlyowner",
            "ftp://x/index.json",
            "javascript:alert(1)",
        ] {
            assert!(resolve_catalog_url(bad).is_err(), "{bad}");
        }
        assert_eq!(default_name(OFFICIAL_URL), "UZApocalyps/OrbitEnginePlugins");
        assert_eq!(
            default_name("https://plugins.example.org/index.json"),
            "plugins.example.org"
        );
    }

    #[test]
    fn sources_persist_and_the_official_one_stays() {
        let dir = scratch("sources");
        let paths = StorePaths {
            root: dir.join("plugin_store"),
        };
        let mut s = Sources::load(&paths);
        assert_eq!(s.sources.len(), 1);
        assert_eq!(s.sources[0].trust(), Trust::Official);
        assert_eq!(s.remove(OFFICIAL_ID), Err(SourceError::Builtin));
        assert_eq!(
            s.set_key(OFFICIAL_ID, &"ab".repeat(32)),
            Err(SourceError::Builtin)
        );

        let key = crate::signing::public_key_hex(&[5u8; 32]);
        let trusted = s
            .add("https://github.com/acme/cat", "", &key.to_uppercase())
            .unwrap();
        assert_eq!(trusted.name, "acme/cat");
        assert_eq!(trusted.trust(), Trust::Trusted);
        assert_eq!(trusted.public_key.as_deref(), Some(key.as_str()));
        let unsigned = s
            .add("https://example.org/index.json", "Example", "")
            .unwrap();
        assert_eq!(unsigned.trust(), Trust::Unsigned);
        assert!(matches!(
            s.add("https://github.com/acme/cat.git", "", ""),
            Err(SourceError::Duplicate(_))
        ));
        assert_eq!(
            s.add("https://x.org/index.json", "", "nothex").unwrap_err(),
            SourceError::BadKey
        );
        assert!(matches!(
            s.add(OFFICIAL_URL, "", ""),
            Err(SourceError::Duplicate(_))
        ));
        s.set_enabled(OFFICIAL_ID, false).unwrap();
        s.save(&paths).unwrap();

        let mut again = Sources::load(&paths);
        assert_eq!(again, s);
        assert!(!again.sources[0].enabled);
        assert_eq!(again.enabled().count(), 2);
        again.remove(&unsigned.id).unwrap();
        assert_eq!(again.sources.len(), 2);

        // A tampered file cannot move or re-key the official source.
        let tampered = r#"(sources: [
            (id: "evil", name: "Evil", url: "https://evil/index.json", enabled: true, public_key: None, builtin: false),
            (id: "official", name: "Fake", url: "https://evil/other.json", enabled: true, public_key: Some("00"), builtin: true),
        ])"#;
        std::fs::write(paths.sources_file(), tampered).unwrap();
        let loaded = Sources::load(&paths);
        assert_eq!(loaded.sources[0], SourceEntry::official());
        assert_eq!(loaded.sources[1].id, "evil");
        assert_eq!(loaded.sources.len(), 2);
        // A broken file: the defaults.
        std::fs::write(paths.sources_file(), "garbage").unwrap();
        assert_eq!(Sources::load(&paths), Sources::default());
    }
}
