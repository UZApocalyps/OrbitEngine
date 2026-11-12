//! Game plugins in a project: `plugins/<dir>/`, one folder per crate (the
//! layout `orbit_export::plugins` builds the player from).
//!
//! A folder installed by the store holds an install record,
//! [`RECORD_FILE`] (store id, version, source, and the SHA-256 of every file
//! as installed). It lets the store tell a pristine install (updated or
//! removed freely) from one the user changed (never overwritten silently:
//! refused, or replaced only on request after a backup into
//! `.orbit/plugin-backups/`). A crate added by hand (no record) is never
//! touched unless the user asks to replace it, with the same backup.
//!
//! An install is staged in `.orbit/plugin-staging/` (same volume: the final
//! step is a rename) and the previous folder is only removed once the new
//! one is in place; any failure puts the previous folder back.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::PackageSource;
use super::catalog::engine_in_range;
use super::package::{Package, PackageError, read_cargo_toml, rewrite_engine_paths};
use crate::catalog::RESERVED_PREFIX;
use crate::sources::Trust;

/// Folder of the plugin crates, relative to the project root (the
/// exporter's `orbit_export::plugins::PLUGINS_DIR`).
pub const PLUGINS_DIR: &str = "plugins";
/// The install record in a plugin folder.
pub const RECORD_FILE: &str = ".orbit-store.json";
/// Staging folder of installs, relative to the project root.
pub const STAGING_DIR: &str = ".orbit/plugin-staging";
/// Where replaced or removed folders that had local changes are kept.
pub const BACKUP_DIR: &str = ".orbit/plugin-backups";
/// The record format.
pub const RECORD_FORMAT: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordSource {
    pub id: String,
    pub name: String,
    pub url: String,
    pub trust: Trust,
}

/// `plugins/<dir>/.orbit-store.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRecord {
    pub format: u32,
    pub id: String,
    pub plugin_ids: Vec<String>,
    pub name: String,
    pub version: String,
    pub source: RecordSource,
    /// SHA-256 of the package installed.
    pub package_sha256: String,
    pub api_version: u32,
    pub min_engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_engine: Option<String>,
    /// The engine folder its engine dependencies were pointed at (`None`:
    /// left as packaged, the export will fail until it is reinstalled with
    /// the engine sources known).
    #[serde(default)]
    pub engine_path: Option<String>,
    #[serde(default)]
    pub build_script: bool,
    #[serde(default)]
    pub proc_macro: bool,
    /// Every file as installed (path with `/`) and its SHA-256.
    pub files: BTreeMap<String, String>,
}

/// A crate in the project's `plugins/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledGame {
    /// Folder name.
    pub dir_name: String,
    pub dir: PathBuf,
    /// `[package] name`, when `Cargo.toml` reads.
    pub crate_name: Option<String>,
    pub version: Option<String>,
    /// `[package.metadata.orbit] plugins`.
    pub plugin_ids: Vec<String>,
    /// Installed by the store.
    pub record: Option<InstallRecord>,
    /// Why `Cargo.toml` or the record could not be read.
    pub error: Option<String>,
}

impl InstalledGame {
    /// The store id of a store install.
    pub fn store_id(&self) -> Option<&str> {
        self.record.as_ref().map(|r| r.id.as_str())
    }

    pub fn provides(&self, id: &str) -> bool {
        self.plugin_ids.iter().any(|p| p == id)
            || self
                .record
                .as_ref()
                .is_some_and(|r| r.plugin_ids.iter().any(|p| p == id))
    }
}

/// The crates of `plugins/` (folders holding a `Cargo.toml`), sorted by
/// folder name.
pub fn scan(project_root: &Path) -> Vec<InstalledGame> {
    let dir = project_root.join(PLUGINS_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || !path.join("Cargo.toml").is_file() {
            continue;
        }
        let mut game = InstalledGame {
            dir_name: entry.file_name().to_string_lossy().into_owned(),
            dir: path.clone(),
            crate_name: None,
            version: None,
            plugin_ids: Vec::new(),
            record: None,
            error: None,
        };
        match std::fs::read_to_string(path.join("Cargo.toml"))
            .map_err(|e| e.to_string())
            .and_then(|t| read_cargo_toml(&t).map_err(|e| e.to_string()))
        {
            Ok(info) => {
                game.crate_name = Some(info.name);
                game.version = Some(info.version);
                game.plugin_ids = info.plugin_ids;
            }
            Err(e) => game.error = Some(e),
        }
        if let Ok(text) = std::fs::read_to_string(path.join(RECORD_FILE)) {
            match serde_json::from_str::<InstallRecord>(&text) {
                Ok(r) if r.format <= RECORD_FORMAT => game.record = Some(r),
                Ok(r) => {
                    game.error = Some(format!("install record format {} is too new", r.format))
                }
                Err(e) => game.error = Some(format!("{RECORD_FILE}: {e}")),
            }
        }
        out.push(game);
    }
    out.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
    out
}

/// A difference between an installed folder and its record.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Modified(String),
    Added(String),
    Removed(String),
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Modified(p) => write!(f, "~ {p}"),
            Self::Added(p) => write!(f, "+ {p}"),
            Self::Removed(p) => write!(f, "- {p}"),
        }
    }
}

fn walk(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, Option<String>>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let at_root = dir == root;
        let name = entry.file_name();
        if at_root && (name == RECORD_FILE || name == "target" || name == "Cargo.lock") {
            continue;
        }
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            // Never followed; a change in itself.
            out.insert(rel, None);
        } else if meta.is_dir() {
            walk(root, &path, out)?;
        } else {
            let hash = orbit_sign::sha256::file_hex(&path, &|| false)?
                .map(|(h, _)| h)
                .unwrap_or_default();
            out.insert(rel, Some(hash));
        }
    }
    Ok(())
}

/// What changed in `dir` since it was installed (`target/`, `Cargo.lock`
/// and the record are ignored: building the crate is not a change).
pub fn local_changes(dir: &Path, record: &InstallRecord) -> std::io::Result<Vec<Change>> {
    let mut found = BTreeMap::new();
    walk(dir, dir, &mut found)?;
    let mut changes = Vec::new();
    for (path, hash) in &record.files {
        match found.get(path) {
            None => changes.push(Change::Removed(path.clone())),
            Some(h) if h.as_ref() != Some(hash) => changes.push(Change::Modified(path.clone())),
            Some(_) => {}
        }
    }
    for path in found.keys() {
        if !record.files.contains_key(path) {
            changes.push(Change::Added(path.clone()));
        }
    }
    changes.sort();
    Ok(changes)
}

/// Why an install or an uninstall would overwrite something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// The folder exists and was not installed by the store.
    Unmanaged(PathBuf),
    /// Installed by the store, then changed.
    Modified { dir: PathBuf, changes: Vec<Change> },
    /// Another crate of the project (`dir`) provides the plugin id `id`.
    SameId { dir: PathBuf, id: String },
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unmanaged(d) => write!(
                f,
                "{} exists and was not installed by the store",
                d.display()
            ),
            Self::Modified { dir, changes } => write!(
                f,
                "{} was changed since it was installed ({} file(s))",
                dir.display(),
                changes.len()
            ),
            Self::SameId { dir, id } => write!(f, "{} already provides `{id}`", dir.display()),
        }
    }
}

/// What to do with a [`Conflict`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnConflict {
    /// Refuse (the default: never overwrite user code silently).
    #[default]
    Refuse,
    /// Move the conflicting folder to `.orbit/plugin-backups/` and go on.
    Backup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallError {
    Package(PackageError),
    /// The package is not what the catalog announced.
    Mismatch(String),
    /// It does not build with this engine.
    Incompatible(String),
    /// It provides an `orbit.*` id and does not come from the official
    /// source.
    Reserved(String),
    Conflict(Conflict),
    NotInstalled(String),
    Io(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Package(e) => write!(f, "{e}"),
            Self::Mismatch(e) => write!(f, "the package does not match the catalog: {e}"),
            Self::Incompatible(e) => write!(f, "{e}"),
            Self::Reserved(id) => write!(
                f,
                "`{id}`: ids `{RESERVED_PREFIX}*` are reserved to the official source"
            ),
            Self::Conflict(c) => write!(f, "{c}"),
            Self::NotInstalled(id) => {
                write!(f, "`{id}` is not installed by the store in this project")
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

fn io_err(what: &Path) -> impl Fn(std::io::Error) -> InstallError + '_ {
    move |e| InstallError::Io(format!("{}: {e}", what.display()))
}

/// What the catalog announced (checked against the package).
#[derive(Clone, Debug)]
pub struct Expected {
    pub id: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct InstallOptions<'a> {
    /// The engine (editor) version.
    pub engine_version: &'a str,
    /// `orbit_plugin_api::API_VERSION`.
    pub api_version: u32,
    /// The engine sources (`ORBIT_ENGINE_SRC`): the engine dependencies are
    /// pointed there. `None`: left as packaged.
    pub engine_root: Option<&'a Path>,
    pub on_conflict: OnConflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallOutcome {
    pub id: String,
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    /// The version replaced (an update).
    pub previous: Option<String>,
    /// Folders moved aside because of a conflict.
    pub backups: Vec<PathBuf>,
    /// Plugin ids the project now has from this crate.
    pub plugin_ids: Vec<String>,
    /// Plugin ids the previous version provided and this one does not.
    pub removed_ids: Vec<String>,
    /// The engine folder the dependencies point at (`None`: not rewritten).
    pub engine_path: Option<PathBuf>,
    pub build_script: bool,
    pub proc_macro: bool,
}

/// What an install of `id` would conflict with, before downloading
/// anything (the target folder name is only known from the package, so
/// [`install`] checks again).
pub fn precheck(project_root: &Path, id: &str) -> Option<Conflict> {
    let games = scan(project_root);
    if let Some(g) = games.iter().find(|g| g.store_id() == Some(id)) {
        let record = g.record.as_ref()?;
        let changes = local_changes(&g.dir, record).unwrap_or_default();
        return (!changes.is_empty()).then(|| Conflict::Modified {
            dir: g.dir.clone(),
            changes,
        });
    }
    games
        .iter()
        .find(|g| g.record.is_none() && g.provides(id))
        .map(|g| Conflict::SameId {
            dir: g.dir.clone(),
            id: id.to_owned(),
        })
}

fn unique_dir(base: &Path, name: &str) -> PathBuf {
    let stamp = crate::now_secs();
    let mut n = 0u32;
    loop {
        let candidate = if n == 0 {
            base.join(format!("{name}-{stamp}"))
        } else {
            base.join(format!("{name}-{stamp}-{n}"))
        };
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Moves `from` to `to` (creating `to`'s parent).
fn move_dir(from: &Path, to: &Path) -> Result<(), InstallError> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    std::fs::rename(from, to).map_err(io_err(from))
}

/// Writes the package files into `dir` (fresh) and returns the record
/// files (hashes as written).
fn write_files(
    dir: &Path,
    package: &Package,
    engine_root: Option<&Path>,
) -> Result<BTreeMap<String, String>, InstallError> {
    let mut hashes = BTreeMap::new();
    for (path, data) in &package.files {
        // `path` went through `safe_path`: relative, no `..`, `/` only.
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        let data = match (path.as_str(), engine_root) {
            ("Cargo.toml", Some(engine)) => {
                rewrite_engine_paths(&String::from_utf8_lossy(data), engine).into_bytes()
            }
            _ => data.clone(),
        };
        std::fs::write(&target, &data).map_err(io_err(&target))?;
        hashes.insert(path.clone(), orbit_sign::sha256::digest_hex(&data));
    }
    Ok(hashes)
}

/// Installs (or updates) the package `bytes` from `source` in the project.
pub fn install(
    project_root: &Path,
    bytes: &[u8],
    source: &PackageSource,
    expected: Option<&Expected>,
    opts: &InstallOptions<'_>,
) -> Result<InstallOutcome, InstallError> {
    let package = Package::read(bytes).map_err(InstallError::Package)?;
    let m = &package.manifest;
    let sha256 = orbit_sign::sha256::digest_hex(bytes);
    if let Some(e) = expected {
        if e.sha256 != sha256 {
            return Err(InstallError::Mismatch("SHA-256".into()));
        }
        if e.id != m.id || e.version != m.version {
            return Err(InstallError::Mismatch(format!(
                "{} {} instead of {} {}",
                m.id, m.version, e.id, e.version
            )));
        }
    }
    if !source.official
        && let Some(id) = m
            .plugin_ids
            .iter()
            .find(|id| id.starts_with(RESERVED_PREFIX))
    {
        return Err(InstallError::Reserved(id.clone()));
    }
    if m.api_version != opts.api_version {
        return Err(InstallError::Incompatible(format!(
            "{} {} is written for plugin API {} (this engine: {})",
            m.name, m.version, m.api_version, opts.api_version
        )));
    }
    if !engine_in_range(opts.engine_version, &m.min_engine, m.max_engine.as_deref()) {
        return Err(InstallError::Incompatible(format!(
            "{} {} builds with engine {}{} (this engine: {})",
            m.name,
            m.version,
            m.min_engine,
            m.max_engine
                .as_deref()
                .map(|v| format!(" to {v} (excluded)"))
                .unwrap_or_else(|| " or newer".into()),
            opts.engine_version
        )));
    }

    // What is in the way.
    let plugins = project_root.join(PLUGINS_DIR);
    let target = plugins.join(&m.dir);
    let games = scan(project_root);
    let existing = games.iter().find(|g| g.store_id() == Some(m.id.as_str()));
    let mut conflicts: Vec<(PathBuf, Conflict)> = Vec::new();
    let mut pristine: Option<PathBuf> = None;
    if let Some(g) = existing {
        let record = g.record.as_ref().expect("store install");
        let changes = local_changes(&g.dir, record).map_err(io_err(&g.dir))?;
        if changes.is_empty() {
            pristine = Some(g.dir.clone());
        } else {
            conflicts.push((
                g.dir.clone(),
                Conflict::Modified {
                    dir: g.dir.clone(),
                    changes,
                },
            ));
        }
    }
    for g in &games {
        if existing.is_some_and(|e| e.dir == g.dir) {
            continue;
        }
        if let Some(id) = m.plugin_ids.iter().find(|id| g.provides(id)) {
            conflicts.push((
                g.dir.clone(),
                Conflict::SameId {
                    dir: g.dir.clone(),
                    id: id.clone(),
                },
            ));
        }
    }
    let target_taken = std::fs::symlink_metadata(&target).is_ok()
        && pristine.as_deref() != Some(target.as_path())
        && !conflicts.iter().any(|(d, _)| *d == target);
    if target_taken {
        conflicts.push((target.clone(), Conflict::Unmanaged(target.clone())));
    }
    if opts.on_conflict == OnConflict::Refuse
        && let Some((_, c)) = conflicts.first()
    {
        return Err(InstallError::Conflict(c.clone()));
    }

    // Stage.
    let staging_root = project_root.join(STAGING_DIR);
    let stage = unique_dir(&staging_root, &m.dir);
    std::fs::create_dir_all(&stage).map_err(io_err(&stage))?;
    let staged = (|| {
        let files = write_files(&stage, &package, opts.engine_root)?;
        let record = InstallRecord {
            format: RECORD_FORMAT,
            id: m.id.clone(),
            plugin_ids: m.plugin_ids.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            source: RecordSource {
                id: source.id.clone(),
                name: source.name.clone(),
                url: source.url.clone(),
                trust: source.trust,
            },
            package_sha256: sha256.clone(),
            api_version: m.api_version,
            min_engine: m.min_engine.clone(),
            max_engine: m.max_engine.clone(),
            engine_path: opts.engine_root.map(|p| p.display().to_string()),
            build_script: m.build_script,
            proc_macro: m.proc_macro,
            files,
        };
        let text =
            serde_json::to_string_pretty(&record).map_err(|e| InstallError::Io(e.to_string()))?;
        let path = stage.join(RECORD_FILE);
        std::fs::write(&path, text + "\n").map_err(io_err(&path))
    })();
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }

    // Move what is in the way, then the new folder in place.
    let backup_root = project_root.join(BACKUP_DIR);
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut backups = Vec::new();
    let trash = pristine.as_ref().map(|_| unique_dir(&staging_root, "old"));
    let mut plan: Vec<(PathBuf, PathBuf)> = Vec::new();
    if let (Some(dir), Some(trash)) = (&pristine, &trash) {
        plan.push((dir.clone(), trash.clone()));
    }
    let mut seen = BTreeSet::new();
    for (dir, _) in &conflicts {
        if seen.insert(dir.clone()) {
            let name = dir
                .file_name()
                .map_or_else(|| "plugin".into(), |n| n.to_string_lossy().into_owned());
            let to = unique_dir(&backup_root, &name);
            backups.push(to.clone());
            plan.push((dir.clone(), to));
        }
    }
    let restore = |moved: &[(PathBuf, PathBuf)]| {
        for (from, to) in moved.iter().rev() {
            let _ = std::fs::rename(to, from);
        }
    };
    for (from, to) in &plan {
        if let Err(e) = move_dir(from, to) {
            restore(&moved);
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
        moved.push((from.clone(), to.clone()));
    }
    if let Err(e) = std::fs::create_dir_all(&plugins)
        .map_err(io_err(&plugins))
        .and_then(|()| std::fs::rename(&stage, &target).map_err(io_err(&target)))
    {
        restore(&moved);
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    if let Some(trash) = &trash {
        let _ = std::fs::remove_dir_all(trash);
    }
    let _ = std::fs::remove_dir(&staging_root);
    let previous_ids: Vec<String> = existing
        .and_then(|g| g.record.as_ref())
        .map(|r| r.plugin_ids.clone())
        .unwrap_or_default();
    Ok(InstallOutcome {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        dir: target,
        previous: existing
            .and_then(|g| g.record.as_ref())
            .map(|r| r.version.clone()),
        backups,
        plugin_ids: m.plugin_ids.clone(),
        removed_ids: previous_ids
            .into_iter()
            .filter(|id| !m.plugin_ids.contains(id))
            .collect(),
        engine_path: opts.engine_root.map(Path::to_path_buf),
        build_script: m.build_script,
        proc_macro: m.proc_macro,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UninstallOutcome {
    pub name: String,
    pub dir: PathBuf,
    pub plugin_ids: Vec<String>,
    /// Where the folder went, when it had local changes.
    pub backup: Option<PathBuf>,
}

/// Removes the store install of `id`. A folder with local changes is
/// refused, or moved to `.orbit/plugin-backups/` with [`OnConflict::Backup`];
/// a pristine one is deleted (the store can install it again).
pub fn uninstall(
    project_root: &Path,
    id: &str,
    on_conflict: OnConflict,
) -> Result<UninstallOutcome, InstallError> {
    let games = scan(project_root);
    let game = games
        .iter()
        .find(|g| g.store_id() == Some(id))
        .ok_or_else(|| InstallError::NotInstalled(id.to_owned()))?;
    let record = game.record.as_ref().expect("store install");
    let changes = local_changes(&game.dir, record).map_err(io_err(&game.dir))?;
    let backup = if changes.is_empty() {
        std::fs::remove_dir_all(&game.dir).map_err(io_err(&game.dir))?;
        None
    } else if on_conflict == OnConflict::Backup {
        let to = unique_dir(&project_root.join(BACKUP_DIR), &game.dir_name);
        move_dir(&game.dir, &to)?;
        Some(to)
    } else {
        return Err(InstallError::Conflict(Conflict::Modified {
            dir: game.dir.clone(),
            changes,
        }));
    };
    Ok(UninstallOutcome {
        name: record.name.clone(),
        dir: game.dir.clone(),
        plugin_ids: record.plugin_ids.clone(),
        backup,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::package::tests::{crate_dir, options};
    use crate::game::package::{MANIFEST_FILE, pack};
    use crate::test_util::scratch;

    fn source(official: bool) -> PackageSource {
        PackageSource {
            id: "src-1".into(),
            name: "Test".into(),
            url: "file:///catalog/index.json".into(),
            trust: Trust::Trusted,
            key: None,
            official,
        }
    }

    fn opts(engine_root: Option<&Path>, on_conflict: OnConflict) -> InstallOptions<'_> {
        InstallOptions {
            engine_version: "0.4.0",
            api_version: 1,
            engine_root,
            on_conflict,
        }
    }

    fn package_version(dir: &Path, version: &str) -> Vec<u8> {
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        let cargo = cargo.replacen(
            "version = \"1.2.0\"",
            &format!("version = \"{version}\""),
            1,
        );
        std::fs::write(dir.join("Cargo.toml"), cargo).unwrap();
        let bytes = pack(dir, &options()).unwrap().bytes;
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            cargo.replacen(
                &format!("version = \"{version}\""),
                "version = \"1.2.0\"",
                1,
            ),
        )
        .unwrap();
        bytes
    }

    #[test]
    fn install_update_uninstall_in_a_project() {
        let root = scratch("game_project");
        let src = crate_dir("game_project_src");
        let v1 = pack(&src, &options()).unwrap().bytes;
        let engine = Path::new("E:/Engine");
        let out = install(
            &root,
            &v1,
            &source(false),
            None,
            &opts(Some(engine), OnConflict::Refuse),
        )
        .unwrap();
        assert_eq!(out.dir, root.join("plugins/acme-dice"));
        assert_eq!(out.previous, None);
        assert_eq!(out.plugin_ids, ["acme.dice", "acme.extra"]);
        let cargo = std::fs::read_to_string(out.dir.join("Cargo.toml")).unwrap();
        assert!(
            cargo.contains("E:/Engine/crates/orbit_plugin_api"),
            "{cargo}"
        );
        assert!(out.dir.join("src/nested/mod.rs").is_file());
        assert!(!out.dir.join(MANIFEST_FILE).exists());
        assert!(!root.join(STAGING_DIR).exists(), "staging cleaned");
        let games = scan(&root);
        assert_eq!(games.len(), 1);
        let record = games[0].record.clone().unwrap();
        assert_eq!(
            (record.id.as_str(), record.version.as_str()),
            ("acme.dice", "1.2.0")
        );
        assert_eq!(record.engine_path.as_deref(), Some("E:/Engine"));
        assert!(local_changes(&out.dir, &record).unwrap().is_empty());
        // Building it is not a change.
        std::fs::create_dir_all(out.dir.join("target/release")).unwrap();
        std::fs::write(out.dir.join("target/release/x"), "x").unwrap();
        std::fs::write(out.dir.join("Cargo.lock"), "x").unwrap();
        assert!(local_changes(&out.dir, &record).unwrap().is_empty());
        assert_eq!(precheck(&root, "acme.dice"), None);

        // Update: pristine, replaced.
        let v2 = package_version(&src, "1.3.0");
        let out = install(
            &root,
            &v2,
            &source(false),
            None,
            &opts(None, OnConflict::Refuse),
        )
        .unwrap();
        assert_eq!(out.previous.as_deref(), Some("1.2.0"));
        assert!(out.backups.is_empty());
        let record = scan(&root)[0].record.clone().unwrap();
        assert_eq!(record.version, "1.3.0");
        assert_eq!(record.engine_path, None);
        assert!(!out.dir.join("target").exists(), "the old folder is gone");

        // Local change: refused, reported, then replaced with a backup.
        std::fs::write(out.dir.join("src/lib.rs"), "// mine\n").unwrap();
        std::fs::write(out.dir.join("src/new.rs"), "// mine too\n").unwrap();
        std::fs::remove_file(out.dir.join("README.md")).unwrap();
        let changes = local_changes(&out.dir, &record).unwrap();
        assert_eq!(
            changes,
            [
                Change::Modified("src/lib.rs".into()),
                Change::Added("src/new.rs".into()),
                Change::Removed("README.md".into())
            ]
        );
        assert!(matches!(
            precheck(&root, "acme.dice"),
            Some(Conflict::Modified { .. })
        ));
        let err = install(
            &root,
            &v1,
            &source(false),
            None,
            &opts(None, OnConflict::Refuse),
        )
        .unwrap_err();
        assert!(
            matches!(err, InstallError::Conflict(Conflict::Modified { .. })),
            "{err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(out.dir.join("src/lib.rs")).unwrap(),
            "// mine\n"
        );
        assert!(matches!(
            uninstall(&root, "acme.dice", OnConflict::Refuse),
            Err(InstallError::Conflict(_))
        ));
        let out = install(
            &root,
            &v1,
            &source(false),
            None,
            &opts(None, OnConflict::Backup),
        )
        .unwrap();
        assert_eq!(out.backups.len(), 1);
        assert_eq!(
            std::fs::read_to_string(out.backups[0].join("src/lib.rs")).unwrap(),
            "// mine\n"
        );
        assert!(out.backups[0].starts_with(root.join(BACKUP_DIR)));
        assert_eq!(scan(&root)[0].record.as_ref().unwrap().version, "1.2.0");

        // Uninstall: pristine, deleted.
        let gone = uninstall(&root, "acme.dice", OnConflict::Refuse).unwrap();
        assert_eq!(gone.backup, None);
        assert_eq!(gone.plugin_ids, ["acme.dice", "acme.extra"]);
        assert!(scan(&root).is_empty());
        assert!(matches!(
            uninstall(&root, "acme.dice", OnConflict::Refuse),
            Err(InstallError::NotInstalled(_))
        ));
    }

    #[test]
    fn hand_made_crates_are_never_overwritten_silently() {
        let root = scratch("game_project_manual");
        let src = crate_dir("game_project_manual_src");
        let bytes = pack(&src, &options()).unwrap().bytes;
        // A crate the user wrote in the target folder.
        let mine = root.join("plugins/acme-dice");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join("Cargo.toml"), "[package]\nname = \"mine\"\nversion = \"0.1.0\"\n[package.metadata.orbit]\nplugins = [\"me.mine\"]\n").unwrap();
        let err = install(
            &root,
            &bytes,
            &source(false),
            None,
            &opts(None, OnConflict::Refuse),
        )
        .unwrap_err();
        assert!(
            matches!(err, InstallError::Conflict(Conflict::Unmanaged(_))),
            "{err:?}"
        );
        // Another folder providing the same plugin id.
        std::fs::remove_dir_all(&mine).unwrap();
        let other = root.join("plugins/my-dice");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("Cargo.toml"), "[package]\nname = \"my-dice\"\nversion = \"0.1.0\"\n[package.metadata.orbit]\nplugins = [\"acme.dice\"]\n").unwrap();
        assert!(matches!(
            precheck(&root, "acme.dice"),
            Some(Conflict::SameId { .. })
        ));
        let err = install(
            &root,
            &bytes,
            &source(false),
            None,
            &opts(None, OnConflict::Refuse),
        )
        .unwrap_err();
        assert!(
            matches!(err, InstallError::Conflict(Conflict::SameId { .. })),
            "{err:?}"
        );
        assert!(other.join("Cargo.toml").is_file());
        // Asked for: kept as a backup.
        let out = install(
            &root,
            &bytes,
            &source(false),
            None,
            &opts(None, OnConflict::Backup),
        )
        .unwrap();
        assert!(out.backups[0].join("Cargo.toml").is_file());
        assert!(!other.exists());
        // A hand-made crate is not a store install.
        assert!(matches!(
            uninstall(&root, "me.mine", OnConflict::Backup),
            Err(InstallError::NotInstalled(_))
        ));
    }

    #[test]
    fn packages_are_checked_before_anything_is_written() {
        let root = scratch("game_project_checks");
        let src = crate_dir("game_project_checks_src");
        let bytes = pack(&src, &options()).unwrap().bytes;
        let sha = orbit_sign::sha256::digest_hex(&bytes);
        let expect = |id: &str, version: &str, sha: &str| Expected {
            id: id.into(),
            version: version.into(),
            sha256: sha.into(),
        };
        let run = |expected: Option<&Expected>, o: InstallOptions<'_>| {
            install(&root, &bytes, &source(false), expected, &o)
        };
        assert!(matches!(
            run(
                Some(&expect("acme.dice", "1.2.0", &"0".repeat(64))),
                opts(None, OnConflict::Refuse)
            ),
            Err(InstallError::Mismatch(_))
        ));
        assert!(matches!(
            run(
                Some(&expect("acme.other", "1.2.0", &sha)),
                opts(None, OnConflict::Refuse)
            ),
            Err(InstallError::Mismatch(_))
        ));
        let mut newer_api = opts(None, OnConflict::Refuse);
        newer_api.api_version = 2;
        assert!(matches!(
            run(None, newer_api),
            Err(InstallError::Incompatible(_))
        ));
        let mut newer_engine = opts(None, OnConflict::Refuse);
        newer_engine.engine_version = "0.5.0";
        let err = run(None, newer_engine).unwrap_err();
        assert!(err.to_string().contains("0.5.0 (excluded)"), "{err}");
        assert!(matches!(
            install(
                &root,
                b"garbage",
                &source(false),
                None,
                &opts(None, OnConflict::Refuse)
            ),
            Err(InstallError::Package(_))
        ));
        assert!(!root.join(PLUGINS_DIR).exists(), "nothing written");
        // `orbit.*` ids only from the official source.
        let cargo = std::fs::read_to_string(src.join("Cargo.toml")).unwrap();
        std::fs::write(
            src.join("Cargo.toml"),
            cargo.replace("\"acme.extra\"", "\"orbit.extra\""),
        )
        .unwrap();
        let reserved = pack(&src, &options()).unwrap().bytes;
        assert!(matches!(
            install(&root, &reserved, &source(false), None, &opts(None, OnConflict::Refuse)),
            Err(InstallError::Reserved(id)) if id == "orbit.extra"
        ));
        assert!(
            install(
                &root,
                &reserved,
                &source(true),
                None,
                &opts(None, OnConflict::Refuse)
            )
            .is_ok()
        );
        assert!(
            run(
                Some(&expect("acme.dice", "1.2.0", &sha)),
                opts(None, OnConflict::Backup)
            )
            .is_ok()
        );
    }
}
