//! `manifest.ron`: what a plugin package says about itself, next to its
//! library. A bare library installed from a `.dll` / `.so` / `.dylib` gets
//! one written from its descriptor.
//!
//! ```ron
//! (
//!     id: "acme.hello",
//!     name: "Hello",
//!     version: "0.1.0",
//!     description: "Adds a window and a menu action.",
//!     author: "ACME",
//!     library: "orbit_hello_plugin.dll",
//!     min_editor: Some("0.3.1"),
//!     max_editor: Some("0.3.1"),
//! )
//! ```

use serde::{Deserialize, Serialize};

/// File name of the manifest in a package and in an installed plugin folder.
pub const MANIFEST_FILE: &str = "manifest.ron";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// File name of the library in the package (no folder).
    pub library: String,
    /// Oldest editor version the plugin accepts (inclusive).
    #[serde(default)]
    pub min_editor: Option<String>,
    /// Newest editor version the plugin accepts (inclusive; missing parts
    /// match anything: `"0.3"` accepts every 0.3.x).
    #[serde(default)]
    pub max_editor: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
}

/// Why the editor version is outside a manifest's range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeError {
    TooOld { min: String },
    TooNew { max: String },
    /// A bound is not a version.
    Invalid(String),
}

/// `major.minor.patch` (a pre-release / build suffix is ignored); missing
/// parts are `fill`.
fn parse_version(text: &str, fill: u64) -> Option<[u64; 3]> {
    let core = text.trim().split(['-', '+']).next()?;
    let mut out = [fill; 3];
    for (i, part) in core.split('.').enumerate() {
        if i >= 3 {
            return None;
        }
        out[i] = if part == "*" || part == "x" { fill } else { part.parse().ok()? };
    }
    Some(out)
}

impl PackageManifest {
    /// Parses a manifest (`Some(...)` can be omitted around optional fields).
    pub fn parse(text: &str) -> Result<Self, String> {
        let options = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        let manifest: Self = options.from_str(text).map_err(|e| e.to_string())?;
        if manifest.library.contains(['/', '\\']) || manifest.library.is_empty() {
            return Err(format!("`library` must be a file name, not `{}`", manifest.library));
        }
        Ok(manifest)
    }

    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).unwrap_or_default()
    }

    /// Whether `editor` (a version) is inside the manifest's range.
    pub fn check_editor(&self, editor: &str) -> Result<(), RangeError> {
        let Some(current) = parse_version(editor, 0) else {
            return Err(RangeError::Invalid(editor.to_owned()));
        };
        if let Some(min) = &self.min_editor {
            let v = parse_version(min, 0).ok_or_else(|| RangeError::Invalid(min.clone()))?;
            if current < v {
                return Err(RangeError::TooOld { min: min.clone() });
            }
        }
        if let Some(max) = &self.max_editor {
            let v = parse_version(max, u64::MAX).ok_or_else(|| RangeError::Invalid(max.clone()))?;
            if current > v {
                return Err(RangeError::TooNew { max: max.clone() });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"(
        id: "acme.hello",
        name: "Hello",
        version: "0.1.0",
        description: "Adds a window.",
        author: "ACME",
        library: "hello.dll",
        min_editor: Some("0.3.0"),
        max_editor: "0.3",
    )"#;

    #[test]
    fn manifests_parse_with_or_without_optional_fields() {
        let m = PackageManifest::parse(FULL).unwrap();
        assert_eq!(m.id, "acme.hello");
        assert_eq!(m.library, "hello.dll");
        assert_eq!(m.min_editor.as_deref(), Some("0.3.0"));
        assert_eq!(m.max_editor.as_deref(), Some("0.3"));
        let m = PackageManifest::parse(r#"(id: "a.b", name: "B", version: "1.0.0", library: "b.so")"#).unwrap();
        assert_eq!(m.description, "");
        assert_eq!(m.min_editor, None);
        // Round trip.
        assert_eq!(PackageManifest::parse(&m.to_ron()).unwrap(), m);
    }

    #[test]
    fn bad_manifests_are_refused() {
        assert!(PackageManifest::parse("(id: \"a.b\")").is_err());
        assert!(PackageManifest::parse("not ron").is_err());
        assert!(PackageManifest::parse(r#"(id: "a.b", name: "B", version: "1", library: "../evil.dll")"#).is_err());
        assert!(PackageManifest::parse(r#"(id: "a.b", name: "B", version: "1", library: "dir\\x.dll")"#).is_err());
    }

    #[test]
    fn editor_range_is_inclusive_and_partial_max_matches_a_series() {
        let m = PackageManifest::parse(FULL).unwrap();
        assert_eq!(m.check_editor("0.3.0"), Ok(()));
        assert_eq!(m.check_editor("0.3.9"), Ok(()));
        assert_eq!(m.check_editor("0.3.1-dev"), Ok(()));
        assert_eq!(m.check_editor("0.2.9"), Err(RangeError::TooOld { min: "0.3.0".into() }));
        assert_eq!(m.check_editor("0.4.0"), Err(RangeError::TooNew { max: "0.3".into() }));
        let open = PackageManifest { min_editor: None, max_editor: None, ..m.clone() };
        assert_eq!(open.check_editor("9.9.9"), Ok(()));
        let bad = PackageManifest { min_editor: Some("soon".into()), ..m };
        assert!(matches!(bad.check_editor("0.3.1"), Err(RangeError::Invalid(_))));
    }
}
