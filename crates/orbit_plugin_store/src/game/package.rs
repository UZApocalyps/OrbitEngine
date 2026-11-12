//! Game plugin packages (`.orbitsrc`): a zip holding a plugin crate's
//! sources and `orbit-game-plugin.json` ([`PackageManifest`]).
//!
//! Reading ([`Package::read`]) checks everything before a single file is
//! written: size limits (package, each file, total, file count), entry paths
//! (relative, no `..`, no drive or root, no backslash, no Windows device
//! name, no duplicate up to case), no link or special entry, and the files
//! are exactly those the manifest lists, each with its SHA-256. The package
//! SHA-256 is what the catalog signature covers, so a valid signature
//! vouches for the manifest and every file.
//!
//! Packing ([`pack`], `cargo xtask game-plugin pack`) reads the crate's
//! `Cargo.toml` (name, version, `[package.metadata.orbit] plugins`), drops a
//! `[lints] workspace = true` table, refuses any other workspace
//! inheritance and any path dependency that is not an engine crate
//! (`…/crates/orbit_*`), and skips `target/`, `Cargo.lock` and hidden files.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::catalog::valid_id;
use crate::version::Version;

/// File extension of a package.
pub const PACKAGE_EXTENSION: &str = "orbitsrc";
/// The manifest inside a package.
pub const MANIFEST_FILE: &str = "orbit-game-plugin.json";
/// The package format this editor reads and writes.
pub const PACKAGE_FORMAT: u32 = 1;
/// Largest package (compressed).
pub const MAX_PACKAGE_BYTES: u64 = 16 << 20;
/// Largest total of the unpacked files.
pub const MAX_UNPACKED_BYTES: u64 = 64 << 20;
/// Largest single file.
pub const MAX_FILE_BYTES: u64 = 16 << 20;
/// Most files in a package.
pub const MAX_FILES: usize = 2000;
/// Names a package may not contain at its root (the installer writes them).
pub const RESERVED_NAMES: &[&str] = &[MANIFEST_FILE, super::project::RECORD_FILE];

/// `orbit-game-plugin.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    pub format: u32,
    /// The store id (one of `plugin_ids`).
    pub id: String,
    /// Every plugin id the crate provides (`[package.metadata.orbit] plugins`).
    pub plugin_ids: Vec<String>,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// `[package] name` of the crate.
    pub crate_name: String,
    /// Folder name in the project's `plugins/`.
    pub dir: String,
    /// `orbit_plugin_api::API_VERSION` the crate is written against.
    pub api_version: u32,
    pub min_engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_engine: Option<String>,
    /// Engine crates it depends on by path (`orbit_plugin_api`, …): the
    /// installer points them at the engine sources.
    #[serde(default)]
    pub engine_crates: Vec<String>,
    /// Has a build script (`build.rs`): code run when it compiles.
    #[serde(default)]
    pub build_script: bool,
    /// Is a procedural macro crate: code run when it compiles.
    #[serde(default)]
    pub proc_macro: bool,
    /// Every file (path with `/`) and its SHA-256 (lowercase hex).
    pub files: BTreeMap<String, String>,
}

/// Why a package (or a crate to pack) is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageError {
    TooLarge(String),
    /// An entry path that could escape or confuse the install folder.
    UnsafePath(String),
    Invalid(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(what) => write!(f, "package too large: {what}"),
            Self::UnsafePath(p) => write!(f, "unsafe path in the package: `{p}`"),
            Self::Invalid(e) => write!(f, "invalid package: {e}"),
        }
    }
}

fn invalid(e: impl Into<String>) -> PackageError {
    PackageError::Invalid(e.into())
}

/// Folder names: lowercase letters, digits, `_` and `-`, 64 characters at
/// most.
pub fn valid_dir_name(dir: &str) -> bool {
    !dir.is_empty()
        && dir.len() <= 64
        && dir
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && !dir.starts_with('-')
}

const WINDOWS_DEVICES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// An entry path the installer may write: relative, `/`-separated, every
/// component a plain name that means the same on Windows, macOS and Linux.
pub fn safe_path(name: &str) -> Result<String, PackageError> {
    let unsafe_path = || PackageError::UnsafePath(name.chars().take(200).collect());
    if name.is_empty() || name.len() > 240 || name.starts_with('/') || name.contains('\\') {
        return Err(unsafe_path());
    }
    let mut parts = Vec::new();
    for part in name.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || part.ends_with(' ')
            || part
                .chars()
                .any(|c| c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
            || WINDOWS_DEVICES.contains(&stem.as_str())
        {
            return Err(unsafe_path());
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

/// A package's content, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub manifest: PackageManifest,
    /// The crate files (manifest excluded), sorted by path.
    pub files: Vec<(String, Vec<u8>)>,
}

/// Unix file type bits of a symbolic link.
const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;

impl Package {
    /// Reads and checks a package (see the module documentation). Nothing
    /// is written anywhere.
    pub fn read(bytes: &[u8]) -> Result<Self, PackageError> {
        if bytes.len() as u64 > MAX_PACKAGE_BYTES {
            return Err(PackageError::TooLarge(format!("{} bytes", bytes.len())));
        }
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| invalid(format!("not a zip archive ({e})")))?;
        if zip.len() > MAX_FILES + 1 {
            return Err(PackageError::TooLarge(format!("{} entries", zip.len())));
        }
        let mut total = 0u64;
        let mut seen_lower = BTreeSet::new();
        let mut manifest_bytes = None;
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut dirs = BTreeSet::new();
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| invalid(format!("entry #{i}: {e}")))?;
            let raw = entry.name().to_owned();
            let kind = entry.unix_mode().map(|m| m & S_IFMT);
            if entry.is_symlink() || kind == Some(S_IFLNK) {
                return Err(PackageError::UnsafePath(format!("{raw} (link)")));
            }
            if let Some(k) = kind
                && k != 0
                && k != S_IFREG
                && k != S_IFDIR
            {
                return Err(PackageError::UnsafePath(format!("{raw} (special file)")));
            }
            if entry.is_dir() {
                let path = safe_path(raw.trim_end_matches('/'))?;
                dirs.insert(path);
                continue;
            }
            let path = safe_path(&raw)?;
            if !seen_lower.insert(path.to_lowercase()) {
                return Err(invalid(format!("`{path}` is listed twice")));
            }
            let declared = entry.size();
            if declared > MAX_FILE_BYTES {
                return Err(PackageError::TooLarge(format!(
                    "`{path}`: {declared} bytes"
                )));
            }
            let mut data = Vec::with_capacity(declared.min(MAX_FILE_BYTES) as usize);
            (&mut entry)
                .take(MAX_FILE_BYTES + 1)
                .read_to_end(&mut data)
                .map_err(|e| invalid(format!("`{path}`: {e}")))?;
            if data.len() as u64 != declared {
                return Err(invalid(format!("`{path}`: size differs from its entry")));
            }
            total += declared;
            if total > MAX_UNPACKED_BYTES {
                return Err(PackageError::TooLarge(format!(
                    "more than {MAX_UNPACKED_BYTES} bytes unpacked"
                )));
            }
            if path == MANIFEST_FILE {
                manifest_bytes = Some(data);
            } else {
                files.insert(path, data);
            }
        }
        // A file that is also a folder of another entry.
        for path in files.keys() {
            let mut prefix = String::new();
            for part in path.split('/') {
                if !prefix.is_empty() && files.contains_key(&prefix) {
                    return Err(invalid(format!("`{prefix}` is both a file and a folder")));
                }
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
            }
            if dirs.contains(path) {
                return Err(invalid(format!("`{path}` is both a file and a folder")));
            }
        }
        let manifest_bytes =
            manifest_bytes.ok_or_else(|| invalid(format!("no {MANIFEST_FILE}")))?;
        let manifest: PackageManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| invalid(format!("{MANIFEST_FILE}: {e}")))?;
        check_manifest(&manifest)?;
        for name in RESERVED_NAMES {
            if files.contains_key(*name) {
                return Err(invalid(format!("`{name}` is reserved")));
            }
        }
        if !files.contains_key("Cargo.toml") {
            return Err(invalid("no Cargo.toml at the root"));
        }
        let listed: BTreeSet<&String> = manifest.files.keys().collect();
        let present: BTreeSet<&String> = files.keys().collect();
        if let Some(extra) = present.difference(&listed).next() {
            return Err(invalid(format!("`{extra}` is not listed in the manifest")));
        }
        if let Some(missing) = listed.difference(&present).next() {
            return Err(invalid(format!("`{missing}` is listed but missing")));
        }
        for (path, data) in &files {
            if orbit_sign::sha256::digest_hex(data) != manifest.files[path] {
                return Err(invalid(format!(
                    "`{path}`: SHA-256 differs from the manifest"
                )));
            }
        }
        Ok(Self {
            manifest,
            files: files.into_iter().collect(),
        })
    }
}

fn check_manifest(m: &PackageManifest) -> Result<(), PackageError> {
    if m.format == 0 || m.format > PACKAGE_FORMAT {
        return Err(invalid(format!(
            "package format {} (this editor reads {PACKAGE_FORMAT})",
            m.format
        )));
    }
    if m.plugin_ids.is_empty() || m.plugin_ids.iter().any(|id| !valid_id(id)) {
        return Err(invalid("bad plugin ids"));
    }
    if !m.plugin_ids.contains(&m.id) {
        return Err(invalid(format!("`{}` is not one of the plugin ids", m.id)));
    }
    if Version::parse(&m.version).is_none() {
        return Err(invalid(format!("bad version `{}`", m.version)));
    }
    if Version::parse(&m.min_engine).is_none()
        || m.max_engine
            .as_deref()
            .is_some_and(|v| Version::parse(v).is_none())
    {
        return Err(invalid("bad engine range"));
    }
    if !valid_dir_name(&m.dir) {
        return Err(invalid(format!("bad folder name `{}`", m.dir)));
    }
    if m.name.trim().is_empty() || m.crate_name.trim().is_empty() || m.api_version == 0 {
        return Err(invalid("incomplete manifest"));
    }
    for (path, hash) in &m.files {
        if safe_path(path)? != *path {
            return Err(PackageError::UnsafePath(path.clone()));
        }
        if hash.len() != 64 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(invalid(format!("`{path}`: bad SHA-256")));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ Cargo.toml

/// The bits of a plugin crate's `Cargo.toml` the store uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrateInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub authors: Vec<String>,
    /// `[package.metadata.orbit] plugins`.
    pub plugin_ids: Vec<String>,
    /// `[package.metadata.orbit] name` (display name).
    pub display_name: Option<String>,
    /// `[package] build = "…"`.
    pub build: bool,
    /// `[lib] proc-macro = true`.
    pub proc_macro: bool,
}

/// `line` without its comment (a `#` outside a string).
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '#' => return &line[..i],
                _ => {}
            },
        }
    }
    line
}

fn unquote(value: &str) -> String {
    let v = value.trim();
    v.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(v)
        .to_owned()
}

fn string_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(unquote)
        .filter(|s| !s.is_empty())
        .collect()
}

fn section_of(line: &str) -> Option<String> {
    let l = line.trim();
    l.starts_with('[')
        .then(|| l.trim_matches(|c| c == '[' || c == ']').trim().to_owned())
}

/// Reads `[package]`, `[lib]` and `[package.metadata.orbit]` (one key per
/// line, lists on one line, like the exporter).
pub fn read_cargo_toml(text: &str) -> Result<CrateInfo, PackageError> {
    let mut info = CrateInfo::default();
    let mut ids = None;
    let mut section = String::new();
    let mut depth = 0i64;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        // The continuation lines of a multi-line array or inline table.
        if depth > 0 {
            depth += depth_change(line);
            continue;
        }
        if let Some(s) = section_of(line) {
            section = s;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        depth = depth_change(value).max(0);
        if depth > 0 && section == "package.metadata.orbit" {
            return Err(invalid(format!(
                "Cargo.toml: write [package.metadata.orbit] {key} on one line (the exporter reads it so)"
            )));
        }
        match (section.as_str(), key) {
            ("package", "name") => info.name = unquote(value),
            ("package", "version") => info.version = unquote(value),
            ("package", "description") => info.description = unquote(value),
            ("package", "license") => info.license = Some(unquote(value)),
            ("package", "homepage") => info.homepage = Some(unquote(value)),
            ("package", "repository") if info.homepage.is_none() => {
                info.homepage = Some(unquote(value))
            }
            ("package", "authors") => info.authors = string_list(value),
            ("package", "build") => info.build = value != "false",
            ("lib", "proc-macro") | ("lib", "proc_macro") => info.proc_macro = value == "true",
            ("package.metadata.orbit", "plugins") => ids = Some(string_list(value)),
            ("package.metadata.orbit", "name") => info.display_name = Some(unquote(value)),
            _ => {}
        }
    }
    if info.name.is_empty() {
        return Err(invalid("Cargo.toml has no `[package] name`"));
    }
    if Version::parse(&info.version).is_none() {
        return Err(invalid(format!(
            "Cargo.toml: `[package] version` must be written out (found `{}`)",
            info.version
        )));
    }
    info.plugin_ids = ids.ok_or_else(|| {
        invalid(
            "Cargo.toml does not declare its plugin ids: add `[package.metadata.orbit]` \
             `plugins = [\"your.plugin.id\"]`",
        )
    })?;
    Ok(info)
}

fn is_dependency_section(section: &str) -> bool {
    let kinds = ["dependencies", "dev-dependencies", "build-dependencies"];
    section
        .split('.')
        .any(|part| kinds.contains(&part.trim().trim_matches('\'').trim_matches('"')))
}

/// Keys whose string value is a file path the store checks.
const PATH_KEYS: &[&str] = &["path", "build"];

/// The path-like values of a line: (key, start, end) byte ranges of the
/// string contents of `path = "…"` / `build = "…"`, bare, quoted (`"path"`)
/// or dotted (`foo.path`).
fn path_values(line: &str) -> Vec<(&'static str, usize, usize)> {
    let bytes = line.as_bytes();
    let key_start = |i: usize| {
        let before = line[..i].trim_end_matches([' ', '\t']);
        before.is_empty() || before.ends_with(['{', ',', '.'])
    };
    // After a key ending at `j`: optional blanks, `=`, blanks, a quote.
    let value_at = |mut j: usize| -> Option<(usize, usize)> {
        while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') {
            j += 1;
        }
        if bytes.get(j) != Some(&b'=') {
            return None;
        }
        j += 1;
        while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') {
            j += 1;
        }
        let q = *bytes.get(j)?;
        if q != b'"' && q != b'\'' {
            return None;
        }
        let len = line[j + 1..].bytes().position(|c| c == q)?;
        Some((j + 1, j + 1 + len))
    };
    let mut out = Vec::new();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q == b'"' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if !line.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let mut matched = None;
        for key in PATH_KEYS {
            if !key_start(i) {
                break;
            }
            let bare = line[i..].starts_with(key)
                && !line[i + key.len()..]
                    .starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '-');
            if bare && let Some(range) = value_at(i + key.len()) {
                matched = Some((*key, range));
                break;
            }
            for q in ['"', '\''] {
                let quoted = format!("{q}{key}{q}");
                if line[i..].starts_with(&quoted)
                    && let Some(range) = value_at(i + quoted.len())
                {
                    matched = Some((*key, range));
                    break;
                }
            }
            if matched.is_some() {
                break;
            }
        }
        if let Some((key, (start, end))) = matched {
            out.push((key, start, end));
            i = end + 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = Some(b);
        }
        i += 1;
    }
    out
}

/// The engine crate a path dependency points at (`…/crates/orbit_x`).
pub fn engine_crate_of(path: &str) -> Option<String> {
    if path.contains('\\') {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let [.., parent, last] = parts.as_slice() else {
        return None;
    };
    (*parent == "crates"
        && last.starts_with("orbit_")
        && last
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
    .then(|| (*last).to_owned())
}

/// A file path inside the crate (`[lib] path`, `build = "…"`): relative,
/// without `..`.
fn inside_crate(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|p| p != "..")
}

/// A path value found by [`scan_toml`].
#[derive(Clone, Debug, PartialEq, Eq)]
struct PathRef {
    /// Physical line (`split_inclusive('\n')` index).
    line: usize,
    start: usize,
    end: usize,
    /// The engine crate it points at (dependency paths only).
    engine: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct TomlScan {
    /// Table headers: (line, table name).
    tables: Vec<(usize, String)>,
    paths: Vec<PathRef>,
}

/// Bracket depth change of `code` (strings skipped).
fn depth_change(code: &str) -> i64 {
    let mut depth = 0i64;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in code.chars() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                _ => {}
            },
        }
    }
    depth
}

/// The key of a `key = value` line (text before the first `=` outside a
/// string), quotes and blanks removed.
fn line_key(code: &str) -> Option<String> {
    let mut quote: Option<char> = None;
    for (i, c) in code.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '=' => {
                    return Some(code[..i].replace(['"', '\'', ' ', '\t'], ""));
                }
                _ => {}
            },
        }
    }
    None
}

/// Scans a `Cargo.toml` with the line reader of this module and refuses
/// every form it cannot read for sure, so that a path dependency is never
/// missed nor rewritten wrongly:
///
/// * multi-line strings (`"""`, `'''`), unbalanced brackets;
/// * `[patch]`, `[replace]` and `[workspace]` tables, `package.workspace`,
///   and any `workspace = true` inheritance (a `[lints]` table holding only
///   that is dropped by [`normalize_cargo_toml`] before);
/// * escapes (`\`) in a path, a dependency path that is not an engine crate
///   (`…/crates/orbit_*`), a crate file path (`[lib] path`, `build`) that
///   leaves the crate.
///
/// Multi-line inline tables and arrays are followed: their lines belong to
/// the key that opened them, so `foo = {\n path = "…" }` under
/// `[dependencies]` is a dependency path.
fn scan_toml(text: &str) -> Result<TomlScan, PackageError> {
    let mut scan = TomlScan::default();
    let mut section = String::new();
    let mut depth = 0i64;
    let mut owner = String::new();
    for (n, line) in text.split_inclusive('\n').enumerate() {
        let code = strip_comment(line);
        let trimmed = code.trim();
        let refuse = |why: &str| {
            invalid(format!(
                "Cargo.toml line {}: {why} (`{}`)",
                n + 1,
                line.trim()
            ))
        };
        if code.contains("\"\"\"") || code.contains("'''") {
            return Err(refuse(
                "multi-line strings are not supported, write the value on one line",
            ));
        }
        let context = if depth > 0 {
            owner.clone()
        } else if trimmed.starts_with('[') {
            if depth_change(trimmed) != 0 {
                return Err(refuse("unbalanced brackets"));
            }
            section = trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split('.')
                .map(|p| p.trim())
                .collect::<Vec<_>>()
                .join(".");
            let first = section
                .split('.')
                .next()
                .unwrap_or_default()
                .trim_matches(['"', '\''])
                .to_owned();
            if matches!(first.as_str(), "patch" | "replace" | "workspace") {
                return Err(refuse(&format!(
                    "a `[{first}]` table has no effect in a dependency and is not packaged"
                )));
            }
            scan.tables.push((n, section.clone()));
            continue;
        } else if let Some(key) = line_key(code) {
            if section.is_empty() {
                key
            } else {
                format!("{section}.{key}")
            }
        } else {
            section.clone()
        };
        let compact = code.replace([' ', '\t', '"', '\''], "");
        if compact.contains("workspace=true") {
            return Err(refuse(
                "inherits from a workspace: write the value out, the package is built outside your workspace",
            ));
        }
        let ctx_key = context.trim_matches(['"', '\'']).to_owned();
        if ctx_key == "package.workspace"
            || (section == "package" && line_key(code).as_deref() == Some("workspace"))
        {
            return Err(refuse("`package.workspace` points outside the package"));
        }
        for (key, start, end) in path_values(code) {
            let value = &code[start..end];
            if value.contains('\\') {
                return Err(refuse("escapes in a path are not supported"));
            }
            let dependency = key == "path" && is_dependency_section(&context);
            let engine = if dependency {
                match engine_crate_of(value) {
                    Some(name) => Some(name),
                    None => {
                        return Err(refuse(&format!(
                            "path dependency `{value}` is not an engine crate (`…/crates/orbit_*`) \
                             and would not be in the package"
                        )));
                    }
                }
            } else {
                if !inside_crate(value) {
                    return Err(refuse(&format!("`{value}` points outside the crate")));
                }
                None
            };
            scan.paths.push(PathRef {
                line: n,
                start,
                end,
                engine,
            });
        }
        let before = depth;
        let value_part = match code.find('=') {
            Some(i) if before == 0 => &code[i + 1..],
            _ => code,
        };
        depth += depth_change(value_part);
        if depth < 0 {
            return Err(refuse("unbalanced brackets"));
        }
        if before == 0 && depth > 0 {
            owner = context;
        }
    }
    if depth != 0 {
        return Err(invalid(
            "Cargo.toml: unbalanced brackets at the end of the file",
        ));
    }
    Ok(scan)
}

/// The `Cargo.toml` as packaged: a `[lints]` table that only says
/// `workspace = true` removed, then [`scan_toml`]'s checks. Returns the text
/// and the engine crates depended on.
pub fn normalize_cargo_toml(text: &str) -> Result<(String, Vec<String>), PackageError> {
    // Drop a `[lints]` table that only inherits the workspace lints.
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut keep = vec![true; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if strip_comment(lines[i]).trim() == "[lints]" {
            let end = (i + 1..lines.len())
                .find(|&j| strip_comment(lines[j]).trim().starts_with('['))
                .unwrap_or(lines.len());
            let only_workspace = (i + 1..end)
                .map(|j| strip_comment(lines[j]).replace([' ', '\t', '\r', '\n'], ""))
                .filter(|l| !l.is_empty())
                .all(|l| l == "workspace=true");
            if only_workspace {
                keep[i..end].iter_mut().for_each(|k| *k = false);
            }
            i = end;
        } else {
            i += 1;
        }
    }
    let mut out = String::with_capacity(text.len());
    for (line, keep) in lines.iter().zip(keep) {
        if keep {
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    let scan = scan_toml(&out)?;
    let engine: BTreeSet<String> = scan.paths.into_iter().filter_map(|p| p.engine).collect();
    Ok((out, engine.into_iter().collect()))
}

/// The installer's check of a packaged `Cargo.toml`: [`scan_toml`]'s rules
/// (a package made by hand could hold anything) and the facts of the
/// manifest (crate name, version, plugin ids are what the exporter will
/// read). Returns the engine crates it depends on.
pub fn check_packaged_cargo_toml(
    text: &str,
    m: &PackageManifest,
) -> Result<Vec<String>, PackageError> {
    let scan = scan_toml(text)?;
    let info = read_cargo_toml(text)?;
    if info.name != m.crate_name || info.version != m.version || info.plugin_ids != m.plugin_ids {
        return Err(invalid(format!(
            "Cargo.toml says {} {} providing {:?}, the manifest {} {} providing {:?}",
            info.name, info.version, info.plugin_ids, m.crate_name, m.version, m.plugin_ids
        )));
    }
    let engine: BTreeSet<String> = scan.paths.into_iter().filter_map(|p| p.engine).collect();
    Ok(engine.into_iter().collect())
}

fn toml_path(p: &Path) -> String {
    let s = p.display().to_string();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    s.replace('\\', "/")
}

/// Points the engine path dependencies of a `Cargo.toml` at `engine_root`
/// (the folder holding `crates/orbit_*`). Other bytes are unchanged; a file
/// [`scan_toml`] refuses is refused here too, never half rewritten.
pub fn rewrite_engine_paths(text: &str, engine_root: &Path) -> Result<String, PackageError> {
    let scan = scan_toml(text)?;
    let crates = engine_root.join("crates");
    let engine_dir = toml_path(&crates);
    if engine_dir.contains(['"', '\'', '\\']) {
        return Err(invalid(format!(
            "the engine folder `{engine_dir}` cannot be written in Cargo.toml"
        )));
    }
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    // Right to left within a line, so the earlier ranges stay valid.
    for p in scan.paths.iter().rev() {
        if let Some(name) = &p.engine {
            lines[p.line].replace_range(p.start..p.end, &format!("{engine_dir}/{name}"));
        }
    }
    Ok(lines.concat())
}

// ------------------------------------------------------------------ packing

/// What `pack` needs besides the crate.
#[derive(Clone, Debug)]
pub struct PackOptions {
    /// `orbit_plugin_api::API_VERSION` of the engine the crate builds with.
    pub api_version: u32,
    pub min_engine: String,
    pub max_engine: Option<String>,
    /// Store id (default: the first plugin id).
    pub id: Option<String>,
    /// Display name (default: `[package.metadata.orbit] name`, else the
    /// crate name).
    pub name: Option<String>,
}

/// A packed crate.
#[derive(Clone, Debug)]
pub struct Packed {
    pub manifest: PackageManifest,
    pub bytes: Vec<u8>,
}

fn collect_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, Vec<u8>)>,
    total: &mut u64,
) -> Result<(), PackageError> {
    let io = |e: std::io::Error| invalid(format!("{}: {e}", dir.display()));
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(io)?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let at_root = dir == root;
        if name.starts_with('.') || (at_root && (name == "target" || name == "Cargo.lock")) {
            continue;
        }
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(io)?;
        let rel = path
            .strip_prefix(root)
            .map_err(|_| invalid("bad path"))?
            .components()
            .map(|c| match c {
                Component::Normal(s) => s
                    .to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("non UTF-8 file name")),
                _ => Err(invalid("bad path")),
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("/");
        if meta.file_type().is_symlink() {
            return Err(invalid(format!(
                "`{rel}` is a link: links are not packaged"
            )));
        }
        if meta.is_dir() {
            collect_files(root, &path, out, total)?;
            continue;
        }
        let rel = safe_path(&rel)?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(PackageError::TooLarge(format!("`{rel}`")));
        }
        *total += meta.len();
        if *total > MAX_UNPACKED_BYTES || out.len() >= MAX_FILES {
            return Err(PackageError::TooLarge("the crate folder".into()));
        }
        out.push((rel, std::fs::read(&path).map_err(io)?));
    }
    Ok(())
}

/// Packs the crate in `crate_dir` (deterministic: same files, same bytes).
pub fn pack(crate_dir: &Path, opts: &PackOptions) -> Result<Packed, PackageError> {
    let cargo = std::fs::read_to_string(crate_dir.join("Cargo.toml"))
        .map_err(|e| invalid(format!("{}: {e}", crate_dir.join("Cargo.toml").display())))?;
    let info = read_cargo_toml(&cargo)?;
    let (cargo, engine_crates) = normalize_cargo_toml(&cargo)?;
    let mut files = Vec::new();
    collect_files(crate_dir, crate_dir, &mut files, &mut 0)?;
    for (path, data) in &mut files {
        if path == "Cargo.toml" {
            *data = cargo.clone().into_bytes();
        }
        if RESERVED_NAMES.contains(&path.as_str()) {
            return Err(invalid(format!("`{path}` is a reserved name")));
        }
    }
    let id = opts
        .id
        .clone()
        .unwrap_or_else(|| info.plugin_ids.first().cloned().unwrap_or_default());
    let dir = info.name.to_ascii_lowercase();
    let manifest = PackageManifest {
        format: PACKAGE_FORMAT,
        id,
        plugin_ids: info.plugin_ids.clone(),
        name: opts
            .name
            .clone()
            .or(info.display_name.clone())
            .unwrap_or_else(|| info.name.clone()),
        version: info.version.clone(),
        description: info.description.clone(),
        author: info.authors.join(", "),
        license: info.license.clone(),
        homepage: info.homepage.clone(),
        crate_name: info.name.clone(),
        dir,
        api_version: opts.api_version,
        min_engine: opts.min_engine.clone(),
        max_engine: opts.max_engine.clone(),
        engine_crates,
        build_script: info.build || files.iter().any(|(p, _)| p == "build.rs"),
        proc_macro: info.proc_macro,
        files: files
            .iter()
            .map(|(p, d)| (p.clone(), orbit_sign::sha256::digest_hex(d)))
            .collect(),
    };
    check_manifest(&manifest)?;
    let zip_err = |e: zip::result::ZipError| invalid(e.to_string());
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    zip.start_file(MANIFEST_FILE, options).map_err(zip_err)?;
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| invalid(e.to_string()))? + "\n";
    zip.write_all(text.as_bytes())
        .map_err(|e| invalid(e.to_string()))?;
    for (path, data) in &files {
        zip.start_file(path.as_str(), options).map_err(zip_err)?;
        zip.write_all(data).map_err(|e| invalid(e.to_string()))?;
    }
    let bytes = zip.finish().map_err(zip_err)?.into_inner();
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(PackageError::TooLarge(format!("{} bytes", bytes.len())));
    }
    Ok(Packed { manifest, bytes })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_util::scratch;

    pub const CARGO: &str = r#"[package]
name = "acme-dice"   # the crate
version = "1.2.0"
edition = "2024"
description = "Rolls # dice"
authors = ["ACME <a@b.c>"]

[package.metadata.orbit]
plugins = ["acme.dice", "acme.extra"]
name = "Dice"

[dependencies]
orbit_plugin_api = { path = "../../../crates/orbit_plugin_api" }
orbit_core = { version = "0.4", path = "../../../crates/orbit_core", features = ["path"] }
log = "0.4"

[dev-dependencies.orbit_vm]
path = "../../../crates/orbit_vm"

[lints]
workspace = true
"#;

    /// A crate folder with a few files and junk that is not packaged.
    pub fn crate_dir(name: &str) -> std::path::PathBuf {
        let dir = scratch(name);
        std::fs::create_dir_all(dir.join("src/nested")).unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), CARGO).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn orbit_plugins() {}\n").unwrap();
        std::fs::write(dir.join("src/nested/mod.rs"), "// nested\n").unwrap();
        std::fs::write(dir.join("README.md"), "# Dice\n").unwrap();
        std::fs::write(dir.join("Cargo.lock"), "lock").unwrap();
        std::fs::write(dir.join("target/debug/junk"), "junk").unwrap();
        std::fs::write(dir.join(".git/config"), "git").unwrap();
        dir
    }

    pub fn options() -> PackOptions {
        PackOptions {
            api_version: 1,
            min_engine: "0.4.0".into(),
            max_engine: Some("0.5.0".into()),
            id: None,
            name: None,
        }
    }

    #[test]
    fn cargo_manifests_are_read_and_normalized() {
        let info = read_cargo_toml(CARGO).unwrap();
        assert_eq!(info.name, "acme-dice");
        assert_eq!(info.version, "1.2.0");
        assert_eq!(info.description, "Rolls # dice");
        assert_eq!(info.plugin_ids, ["acme.dice", "acme.extra"]);
        assert_eq!(info.display_name.as_deref(), Some("Dice"));
        assert_eq!(info.authors, ["ACME <a@b.c>"]);
        let (text, engine) = normalize_cargo_toml(CARGO).unwrap();
        assert_eq!(engine, ["orbit_core", "orbit_plugin_api", "orbit_vm"]);
        assert!(!text.contains("[lints]") && !text.contains("workspace"));
        assert!(text.contains("log = \"0.4\""));
        for bad in [
            "[package]\nname = \"x\"\nversion.workspace = true\n",
            "[dependencies]\nserde = { workspace = true }\n",
            "[dependencies]\nhelper = { path = \"../helper\" }\n",
            "[dependencies.helper]\npath = \"../helper\"\n",
            "[lints]\nworkspace = true\nclippy = {}\n",
        ] {
            assert!(normalize_cargo_toml(bad).is_err(), "{bad}");
        }
        // `path` in a feature name or a string is not a path key.
        assert!(
            normalize_cargo_toml(
                "[dependencies]\na = { version = \"1\", features = [\"path = 'x'\"] }\n"
            )
            .is_ok()
        );
        assert!(read_cargo_toml("[package]\nname = \"x\"\nversion = \"1.0.0\"\n").is_err());
        assert!(read_cargo_toml("[package]\nname = \"x\"\nversion.workspace = true\n").is_err());
    }

    #[test]
    fn engine_paths_are_rewritten() {
        let (text, _) = normalize_cargo_toml(CARGO).unwrap();
        let out = rewrite_engine_paths(&text, Path::new(r"E:\Engine")).unwrap();
        assert!(
            out.contains("orbit_plugin_api = { path = \"E:/Engine/crates/orbit_plugin_api\" }"),
            "{out}"
        );
        assert!(
            out.contains("path = \"E:/Engine/crates/orbit_core\", features = [\"path\"]"),
            "{out}"
        );
        assert!(
            out.contains("path = \"E:/Engine/crates/orbit_vm\""),
            "{out}"
        );
        assert!(out.contains("name = \"acme-dice\"   # the crate"));
        assert_eq!(
            engine_crate_of("../../crates/orbit_core"),
            Some("orbit_core".into())
        );
        assert_eq!(engine_crate_of("crates/orbit_core/../evil"), None);
        assert_eq!(engine_crate_of("../helper"), None);
    }

    #[test]
    fn paths_are_checked() {
        for ok in ["Cargo.toml", "src/lib.rs", "assets/a b.png", "src/.hidden"] {
            assert_eq!(safe_path(ok).unwrap(), ok);
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a/./b",
            "a//b",
            "C:/x",
            "c:x",
            r"a\b",
            "src/con.rs",
            "NUL",
            "aux.txt",
            "a/b.",
            "a/b ",
            "a\u{0}b",
            "a/b?",
            "x/",
        ] {
            assert!(safe_path(bad).is_err(), "{bad:?}");
        }
        assert!(valid_dir_name("acme-dice_2"));
        assert!(
            !valid_dir_name("Acme")
                && !valid_dir_name("-x")
                && !valid_dir_name("a/b")
                && !valid_dir_name("..")
        );
    }

    #[test]
    fn crates_pack_and_read_back() {
        let dir = crate_dir("pack");
        let packed = pack(&dir, &options()).unwrap();
        let m = &packed.manifest;
        assert_eq!(
            (m.id.as_str(), m.dir.as_str(), m.name.as_str()),
            ("acme.dice", "acme-dice", "Dice")
        );
        let files: Vec<&str> = m.files.keys().map(String::as_str).collect();
        assert_eq!(
            files,
            ["Cargo.toml", "README.md", "src/lib.rs", "src/nested/mod.rs"]
        );
        assert!(!m.build_script && !m.proc_macro);
        assert_eq!(m.engine_crates.len(), 3);
        // Deterministic.
        assert_eq!(pack(&dir, &options()).unwrap().bytes, packed.bytes);
        let back = Package::read(&packed.bytes).unwrap();
        assert_eq!(&back.manifest, m);
        let cargo = &back
            .files
            .iter()
            .find(|(p, _)| p == "Cargo.toml")
            .unwrap()
            .1;
        assert!(!String::from_utf8_lossy(cargo).contains("[lints]"));
        // A build script is reported.
        std::fs::write(dir.join("build.rs"), "fn main() {}").unwrap();
        assert!(pack(&dir, &options()).unwrap().manifest.build_script);
        // Another store id must be one of the plugin ids.
        let mut o = options();
        o.id = Some("acme.extra".into());
        assert_eq!(pack(&dir, &o).unwrap().manifest.id, "acme.extra");
        o.id = Some("acme.other".into());
        assert!(pack(&dir, &o).is_err());
    }

    /// A zip with raw entries (name, data, unix permissions).
    pub fn raw_zip(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data, mode) in entries {
            let mut options = zip::write::SimpleFileOptions::default();
            if let Some(m) = mode {
                options = options.unix_permissions(*m);
            }
            if name.ends_with('/') {
                zip.add_directory(*name, options).unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
                zip.write_all(data).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    fn manifest_for(files: &[(&str, &[u8])]) -> Vec<u8> {
        let dir = crate_dir("manifest_for");
        let mut m = pack(&dir, &options()).unwrap().manifest;
        m.files = files
            .iter()
            .map(|(p, d)| ((*p).to_owned(), orbit_sign::sha256::digest_hex(d)))
            .collect();
        serde_json::to_vec(&m).unwrap()
    }

    #[test]
    fn hostile_packages_are_refused() {
        let cargo: &[u8] = b"[package]\nname = \"x\"\n";
        let ok_manifest = manifest_for(&[("Cargo.toml", cargo)]);
        // The baseline is accepted.
        let good = raw_zip(&[
            (MANIFEST_FILE, &ok_manifest, None),
            ("Cargo.toml", cargo, None),
        ]);
        assert!(Package::read(&good).is_ok());
        let with = |name: &str, data: &[u8], mode: Option<u32>| {
            let manifest = manifest_for(&[("Cargo.toml", cargo), (name, data)]);
            raw_zip(&[
                (MANIFEST_FILE, &manifest, None),
                ("Cargo.toml", cargo, None),
                (name, data, mode),
            ])
        };
        // Zip slip, absolute paths, drives, backslashes.
        for name in [
            "../evil.rs",
            "src/../../evil.rs",
            "/abs.rs",
            "C:/x.rs",
            r"src\x.rs",
            "src/aux.rs",
        ] {
            let raw = raw_zip(&[
                (MANIFEST_FILE, &ok_manifest, None),
                ("Cargo.toml", cargo, None),
                (name, b"x", None),
            ]);
            assert!(
                matches!(Package::read(&raw), Err(PackageError::UnsafePath(_))),
                "{name}: {:?}",
                Package::read(&raw)
            );
        }
        // A symbolic link entry (listed in the manifest, so only the link
        // check can refuse it).
        let manifest = manifest_for(&[("Cargo.toml", cargo), ("src/link", b"/etc/passwd")]);
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        zip.start_file(MANIFEST_FILE, o).unwrap();
        zip.write_all(&manifest).unwrap();
        zip.start_file("Cargo.toml", o).unwrap();
        zip.write_all(cargo).unwrap();
        zip.add_symlink("src/link", "/etc/passwd", o).unwrap();
        let link = zip.finish().unwrap().into_inner();
        assert!(
            matches!(Package::read(&link), Err(PackageError::UnsafePath(p)) if p.contains("link"))
        );
        // A file not listed, a listed file missing, a changed file.
        let extra = raw_zip(&[
            (MANIFEST_FILE, &ok_manifest, None),
            ("Cargo.toml", cargo, None),
            ("src/x.rs", b"x", None),
        ]);
        assert!(
            Package::read(&extra)
                .unwrap_err()
                .to_string()
                .contains("not listed")
        );
        let missing = raw_zip(&[
            (
                MANIFEST_FILE,
                &manifest_for(&[("Cargo.toml", cargo), ("a.rs", b"a")]),
                None,
            ),
            ("Cargo.toml", cargo, None),
        ]);
        assert!(
            Package::read(&missing)
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
        let changed = raw_zip(&[
            (MANIFEST_FILE, &ok_manifest, None),
            ("Cargo.toml", b"tampered", None),
        ]);
        assert!(
            Package::read(&changed)
                .unwrap_err()
                .to_string()
                .contains("SHA-256")
        );
        // Duplicates up to case, a file that is also a folder.
        let dup = with("cargo.toml", cargo, None);
        assert!(
            Package::read(&dup)
                .unwrap_err()
                .to_string()
                .contains("twice")
        );
        let both = raw_zip(&[
            (
                MANIFEST_FILE,
                &manifest_for(&[("Cargo.toml", cargo), ("src", b"a"), ("src/a.rs", b"a")]),
                None,
            ),
            ("Cargo.toml", cargo, None),
            ("src", b"a", None),
            ("src/a.rs", b"a", None),
        ]);
        assert!(
            Package::read(&both)
                .unwrap_err()
                .to_string()
                .contains("file and a folder")
        );
        // Reserved names, no manifest, not a zip, too large.
        let reserved = with(super::super::project::RECORD_FILE, b"{}", None);
        assert!(
            Package::read(&reserved)
                .unwrap_err()
                .to_string()
                .contains("reserved")
        );
        let no_manifest = raw_zip(&[("Cargo.toml", cargo, None)]);
        assert!(Package::read(&no_manifest).is_err());
        assert!(Package::read(b"not a zip").is_err());
        let huge = vec![0u8; MAX_PACKAGE_BYTES as usize + 1];
        assert!(matches!(
            Package::read(&huge),
            Err(PackageError::TooLarge(_))
        ));
        // A zip bomb: a big file that compresses well.
        let big = vec![b'a'; (MAX_FILE_BYTES + 1) as usize];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let deflate = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("Cargo.toml", deflate).unwrap();
        zip.write_all(&big).unwrap();
        let bomb = zip.finish().unwrap().into_inner();
        assert!(bomb.len() < 1 << 20);
        assert!(matches!(
            Package::read(&bomb),
            Err(PackageError::TooLarge(_))
        ));
        // A newer package format.
        let mut newer: PackageManifest = serde_json::from_slice(&ok_manifest).unwrap();
        newer.format = PACKAGE_FORMAT + 1;
        let newer = raw_zip(&[
            (MANIFEST_FILE, &serde_json::to_vec(&newer).unwrap(), None),
            ("Cargo.toml", cargo, None),
        ]);
        assert!(
            Package::read(&newer)
                .unwrap_err()
                .to_string()
                .contains("format")
        );
    }

    /// Every TOML form the line reader cannot read for sure is refused at
    /// packing and at install: never a path dependency missed, never a
    /// wrong rewrite.
    #[test]
    fn cargo_forms_the_reader_cannot_follow_are_refused() {
        let head = "[package]\nname = \"x\"\nversion = \"1.0.0\"\n";
        let refused = [
            // Multi-line strings (a `path = "..."` inside would be missed or invented).
            "[package]\ndescription = \"\"\"\nline\n\"\"\"\n",
            "[package]\ndescription = '''\npath = \"../x\"\n'''\n",
            // Quoted, dotted and root-level dotted keys to a non-engine path.
            "[dependencies]\nhelper = { \"path\" = \"../helper\" }\n",
            "[dependencies]\nhelper = { 'path' = '../helper' }\n",
            "[dependencies]\nhelper.path = \"../helper\"\n",
            "dependencies.helper.path = \"../helper\"\n",
            "dependencies.helper = { path = \"../helper\" }\n",
            // Multi-line inline table (TOML 1.1) to a non-engine path.
            "[dependencies]\nhelper = {\n  version = \"1\",\n  path = \"../helper\",\n}\n",
            // Target-specific dependencies.
            "[target.'cfg(windows)'.dependencies]\nhelper = { path = \"../helper\" }\n",
            "[target.\"cfg(target_os = \\\"linux\\\")\".dev-dependencies.helper]\npath = \"../helper\"\n",
            // Tables without effect in a dependency, workspace pointers.
            "[patch.crates-io]\nserde = { path = \"../serde\" }\n",
            "[replace]\n\"serde:1.0.0\" = { path = \"../serde\" }\n",
            "[workspace]\nmembers = []\n",
            "[workspace.dependencies]\nx = \"1\"\n",
            "[package]\nworkspace = \"../..\"\n",
            "[dependencies]\nserde = { \"workspace\" = true }\n",
            // Escapes and paths leaving the crate.
            "[dependencies]\norbit_core = { path = \"..\\\\..\\\\crates\\\\orbit_core\" }\n",
            "[dependencies]\norbit_core = { path = \"\\u002e\\u002e/crates/orbit_core\" }\n",
            "[lib]\npath = \"../other/lib.rs\"\n",
            "[[bin]]\nname = \"b\"\npath = \"C:/elsewhere/main.rs\"\n",
            "[package]\nbuild = \"../build.rs\"\n",
            // Unbalanced brackets.
            "[dependencies]\nx = { version = \"1\"\n",
            "[dependencies\nx = \"1\"\n",
        ];
        for body in refused {
            let text = format!("{head}{body}");
            assert!(
                normalize_cargo_toml(&text).is_err(),
                "accepted at packing:\n{text}"
            );
            assert!(
                rewrite_engine_paths(&text, Path::new("E:/Engine")).is_err(),
                "rewritten:\n{text}"
            );
        }
        // A multi-line `plugins` list: refused, the exporter could not read it.
        let multi = format!("{head}[package.metadata.orbit]\nplugins = [\n  \"a.b\",\n]\n");
        assert!(
            read_cargo_toml(&multi)
                .unwrap_err()
                .to_string()
                .contains("one line")
        );

        // What is followed and rewritten right.
        let ok = format!(
            "{head}\
             [features]\nall = [\n  \"a\",\n  [\"nested\"],\n]\n\
             [dependencies]\n\
             orbit_core = {{\n  version = \"0.4\",\n  path = \"../../crates/orbit_core\",\n}}\n\
             orbit_vm.path = \"../../crates/orbit_vm\"\n\
             \"orbit_plugin_api\" = {{ 'path' = '../../crates/orbit_plugin_api' }}\n\
             [target.'cfg(windows)'.dependencies.orbit_anim]\npath = \"../../crates/orbit_anim\"  # comment\n\
             [lib]\npath = \"src/lib.rs\"\n\
             [package.metadata.orbit]\nplugins = [\"a.b\"]   # one line\n"
        );
        let (normalized, engine) = normalize_cargo_toml(&ok).unwrap();
        assert_eq!(normalized, ok);
        assert_eq!(
            engine,
            ["orbit_anim", "orbit_core", "orbit_plugin_api", "orbit_vm"]
        );
        let out = rewrite_engine_paths(&ok, Path::new("E:/Engine")).unwrap();
        for name in ["orbit_anim", "orbit_core", "orbit_plugin_api", "orbit_vm"] {
            assert!(
                out.contains(&format!("E:/Engine/crates/{name}")),
                "{name}:\n{out}"
            );
        }
        assert!(!out.contains("../../crates"), "{out}");
        assert!(out.contains("path = \"src/lib.rs\""));
        assert_eq!(out.lines().count(), ok.lines().count());
        assert_eq!(read_cargo_toml(&ok).unwrap().plugin_ids, ["a.b"]);
        // Non-ASCII text outside strings does not panic the reader.
        assert!(
            normalize_cargo_toml("[package]\nname = \"x\"\nversion = \"1.0.0\"\nclé = \"é\"\n")
                .is_ok()
        );
        // CRLF files.
        let crlf = ok.replace('\n', "\r\n");
        let out = rewrite_engine_paths(&crlf, Path::new("E:/Engine")).unwrap();
        assert!(out.contains("E:/Engine/crates/orbit_vm\"\r\n"), "{out}");
    }

    #[test]
    fn the_installer_checks_the_packaged_cargo_toml() {
        let dir = crate_dir("installer_cargo");
        let packed = pack(&dir, &options()).unwrap();
        let m = &packed.manifest;
        let (cargo, _) = normalize_cargo_toml(CARGO).unwrap();
        assert_eq!(
            check_packaged_cargo_toml(&cargo, m).unwrap(),
            ["orbit_core", "orbit_plugin_api", "orbit_vm"]
        );
        // The manifest and Cargo.toml disagree (the exporter reads Cargo.toml).
        let other_ids = cargo.replace("\"acme.extra\"", "\"acme.hidden\"");
        assert!(check_packaged_cargo_toml(&other_ids, m).is_err());
        let other_version = cargo.replace("version = \"1.2.0\"", "version = \"9.0.0\"");
        assert!(check_packaged_cargo_toml(&other_version, m).is_err());
        // A hand-made package with a form the packer refuses.
        let sneaky = format!("{cargo}[dependencies.helper]\n\"path\" = \"../helper\"\n");
        assert!(check_packaged_cargo_toml(&sneaky, m).is_err());
    }
}
