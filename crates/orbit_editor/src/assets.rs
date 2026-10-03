//! Assets of the open project, without any UI: importing files (copy into
//! the project, content validation by the registered importers, stable id,
//! manifest entry as one undo step), renaming / moving, the case lint and
//! the scan of the project's asset folders.
//!
//! Everything that writes goes through the project sandbox
//! ([`orbit_project::sandbox`]): destinations are confined to the project
//! folder, links resolved. Sources may live anywhere (they are only read).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use orbit_core::{AssetEntry, AssetId, AssetKind, Story};
use orbit_editor_api::Translator;
use orbit_plugin_api::ProjectRegistry;
use orbit_project::sandbox::Sandbox;
use orbit_project::{Command, Project, ProjectError, ops};

/// Largest file the importer reads (bytes).
pub const MAX_IMPORT_BYTES: u64 = 256 * 1024 * 1024;
/// Files taken from one dropped folder, at most.
const MAX_FOLDER_FILES: usize = 500;

/// Sub-folder of the project that holds assets of `kind`.
pub fn asset_dir(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Image => "assets/images",
        AssetKind::Music => "assets/music",
        AssetKind::Sound => "assets/sounds",
        AssetKind::Scene3d => "assets/3d",
        AssetKind::Font => "assets/fonts",
    }
}

/// Reasons given by the built-in importers, and their translation.
pub mod refusal {
    use orbit_editor_api::Translator;

    // The importers have no access to the editor's language (the plugin API is a
    // plain `fn(&[u8])`), so they refuse with these stable English reasons, which
    // are also what the MCP server reports to the AI. The editor shows them
    // through [`localize`].
    pub const ERR_NOT_IMAGE: &str = "not a PNG or JPEG image";
    pub const ERR_EMPTY_IMAGE: &str = "empty image";
    pub const ERR_EMPTY_AUDIO: &str = "empty audio";
    pub const ERR_FORMAT: (&str, &str) = ("image format ", " not supported (PNG or JPEG)");
    pub const ERR_IMAGE_UNREADABLE: (&str, &str) = ("unreadable image (", ")");
    pub const ERR_AUDIO_UNREADABLE: (&str, &str) = ("unreadable audio (", ")");
    pub const ERR_TOO_LARGE: (&str, &str) = ("image too large (", " px at most)");

    fn between<'a>(text: &'a str, (head, tail): (&str, &str)) -> Option<&'a str> {
        text.strip_prefix(head)?.strip_suffix(tail)
    }

    /// The reason an importer gave, in the editor's language. A reason that is
    /// not one of the built-in importers' (a plugin's) is returned as it is.
    pub fn localize(texts: &Translator, reason: &str) -> String {
        match reason {
            ERR_NOT_IMAGE => return texts.t("import.err.not_image"),
            ERR_EMPTY_IMAGE => return texts.t("import.err.image_empty"),
            ERR_EMPTY_AUDIO => return texts.t("import.err.audio_empty"),
            _ => {}
        }
        if let Some(format) = between(reason, ERR_FORMAT) {
            return texts.tf("import.err.image_format", &[("format", &format)]);
        }
        if let Some(error) = between(reason, ERR_IMAGE_UNREADABLE) {
            return texts.tf("import.err.image_unreadable", &[("error", &error)]);
        }
        if let Some(error) = between(reason, ERR_AUDIO_UNREADABLE) {
            return texts.tf("import.err.audio_unreadable", &[("error", &error)]);
        }
        if let Some(inner) = between(reason, ERR_TOO_LARGE)
            && let Some((dims, max)) = inner.split_once(" px, ")
            && let Some((width, height)) = dims.split_once('×')
        {
            return texts.tf(
                "import.err.image_too_large",
                &[("width", &width), ("height", &height), ("max", &max)],
            );
        }
        reason.to_owned()
    }
}

/// Why a file could not be imported; [`ImportError::message`] says it to the
/// user in the editor's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// No importer handles this extension.
    Unsupported {
        extension: String,
        accepted: String,
    },
    TooLarge(u64),
    Unreadable(String),
    /// The importer refused the content (not really a PNG, truncated...).
    Invalid(String),
    /// Writing in the project failed.
    Io(String),
}

impl ImportError {
    /// The reason, in the editor's language.
    pub fn message(&self, texts: &Translator) -> String {
        match self {
            ImportError::Unsupported {
                extension,
                accepted,
            } if extension.is_empty() => {
                texts.tf("import.err.unsupported_no_ext", &[("accepted", accepted)])
            }
            ImportError::Unsupported {
                extension,
                accepted,
            } => texts.tf(
                "import.err.unsupported",
                &[("extension", extension), ("accepted", accepted)],
            ),
            ImportError::TooLarge(n) => texts.tf(
                "import.err.too_large",
                &[
                    ("size", &(n / (1024 * 1024))),
                    ("limit", &(MAX_IMPORT_BYTES / (1024 * 1024))),
                ],
            ),
            ImportError::Unreadable(e) => texts.tf("import.err.unreadable", &[("error", e)]),
            ImportError::Invalid(e) => texts.tf(
                "import.err.invalid",
                &[("error", &refusal::localize(texts, e))],
            ),
            ImportError::Io(e) => texts.tf("import.err.io", &[("error", e)]),
        }
    }
}

/// One file that is now an asset of the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    pub id: AssetId,
    pub kind: AssetKind,
    /// Manifest path (relative, `/`).
    pub path: String,
    /// The file was already declared: nothing was added.
    pub existing: bool,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub imported: Vec<Imported>,
    pub failed: Vec<(PathBuf, ImportError)>,
}

impl ImportReport {
    /// Newly declared assets (not the ones that already existed).
    pub fn added(&self) -> usize {
        self.imported.iter().filter(|i| !i.existing).count()
    }
}

/// A file name safe on Windows, Linux and the web: letters, digits, `-`,
/// `_`, `.`; spaces become `_`; the extension is lowercased. Never empty.
pub fn sanitize_file_name(name: &str) -> String {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, Some(e)),
        _ => (name, None),
    };
    // Letters and digits stay; any run of other characters becomes one `_`
    // (a lone `-` is kept).
    let clean = |s: &str| -> String {
        let mut out = String::new();
        let mut run = String::new();
        let flush = |out: &mut String, run: &mut String| {
            if !run.is_empty() {
                out.push(if run == "-" { '-' } else { '_' });
                run.clear();
            }
        };
        for c in s.chars() {
            if c.is_alphanumeric() {
                flush(&mut out, &mut run);
                out.push(c);
            } else {
                run.push(c);
            }
        }
        flush(&mut out, &mut run);
        out.trim_matches(['-', '_']).to_owned()
    };
    let stem = clean(stem);
    let stem = if stem.is_empty() {
        "asset".to_owned()
    } else {
        stem
    };
    match ext
        .map(|e| clean(e).to_ascii_lowercase())
        .filter(|e| !e.is_empty())
    {
        Some(e) => format!("{stem}.{e}"),
        None => stem,
    }
}

/// Files directly under `dir` (names only), for collision checks.
fn dir_names(dir: &Path) -> HashSet<String> {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// A manifest path for a new file named `name` in `dir`, free on disk **and**
/// in the manifest, compared without regard to case (Windows would merge
/// `A.png` and `a.png`; Linux and the web would not).
pub(crate) fn free_path(root: &Path, dir: &str, name: &str, taken: &HashSet<String>) -> String {
    let on_disk = dir_names(&root.join(dir));
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_owned(), format!(".{e}")),
        None => (name.to_owned(), String::new()),
    };
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            name.to_owned()
        } else {
            format!("{stem}-{n}{ext}")
        };
        let rel = format!("{dir}/{candidate}");
        if !on_disk.contains(&candidate.to_lowercase()) && !taken.contains(&rel.to_lowercase()) {
            return rel;
        }
        n += 1;
    }
}

fn lower_ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Files to import for the dropped / picked `paths`: folders are expanded
/// (recursively, at most [`MAX_FOLDER_FILES`] files, dot-files skipped).
pub fn expand_sources(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = paths.iter().rev().cloned().collect();
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            let mut children: Vec<PathBuf> = fs::read_dir(&path)
                .map(|rd| rd.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            children.sort();
            children.retain(|c| {
                !c.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
            });
            stack.extend(children.into_iter().rev());
        } else if out.len() < MAX_FOLDER_FILES {
            out.push(path);
        }
    }
    out
}

/// What [`prepare`] decided for one file.
struct Prepared {
    entry: AssetEntry,
    existing: bool,
    /// The copy made in the project, removed again if the declaration fails.
    copied: Option<PathBuf>,
}

/// Validates and copies one file; does not touch the manifest.
fn prepare(
    root: &Path,
    sandbox: &Sandbox,
    registry: &ProjectRegistry,
    story: &Story,
    taken: &mut HashSet<String>,
    source: &Path,
    texts: &Translator,
) -> Result<Prepared, ImportError> {
    let meta = fs::metadata(source).map_err(|e| ImportError::Unreadable(e.to_string()))?;
    if !meta.is_file() {
        return Err(ImportError::Unreadable(texts.t("import.err.not_a_file")));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::TooLarge(meta.len()));
    }
    let ext = lower_ext(source);
    let Some(importer) = registry.importer_for(&ext) else {
        return Err(ImportError::Unsupported {
            extension: ext,
            accepted: registry.import_extensions().join(", "),
        });
    };
    let bytes = fs::read(source).map_err(|e| ImportError::Unreadable(e.to_string()))?;
    let imported = (importer.import)(&bytes).map_err(ImportError::Invalid)?;

    // Already inside the project: declared as it is when it lies under
    // `assets/`, copied otherwise.
    let inside = fs::canonicalize(source)
        .ok()
        .and_then(|abs| sandbox.relative(&abs));
    if let Some(rel) = &inside
        && let Some(existing) = story.assets.assets.iter().find(|a| &a.path == rel)
    {
        return Ok(Prepared {
            entry: existing.clone(),
            existing: true,
            copied: None,
        });
    }
    if let Some(rel) = inside.filter(|rel| rel.starts_with("assets/")) {
        taken.insert(rel.to_lowercase());
        return Ok(Prepared {
            entry: AssetEntry {
                id: AssetId::new(),
                kind: imported.kind,
                path: rel,
            },
            existing: false,
            copied: None,
        });
    }

    let file_name = sanitize_file_name(
        source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("asset"),
    );
    let rel = free_path(root, asset_dir(imported.kind), &file_name, taken);
    let dest = sandbox
        .resolve(&rel)
        .map_err(|e| ImportError::Io(e.to_string()))?;
    crate::storage::write_atomic(&dest, &imported.bytes)
        .map_err(|e| ImportError::Io(e.to_string()))?;
    taken.insert(rel.to_lowercase());
    Ok(Prepared {
        entry: AssetEntry {
            id: AssetId::new(),
            kind: imported.kind,
            path: rel,
        },
        existing: false,
        copied: Some(dest),
    })
}

/// Imports files into the project: each one is validated by the importer of
/// its extension (decoded: a renamed text file is refused), copied under
/// `assets/<type>/` with a safe, unique name, and declared in the manifest
/// under a new stable id. All the declarations are **one undo step**
/// ("Importer N ressource(s)"); a file that fails is reported and does not
/// stop the others.
pub fn import_files(
    project: &mut Project,
    root: &Path,
    registry: &ProjectRegistry,
    sources: &[PathBuf],
    texts: &Translator,
) -> ImportReport {
    let mut report = ImportReport::default();
    let sandbox = match Sandbox::new(root) {
        Ok(s) => s,
        Err(e) => {
            for s in sources {
                report
                    .failed
                    .push((s.clone(), ImportError::Io(e.to_string())));
            }
            return report;
        }
    };
    let mut taken: HashSet<String> = project
        .story()
        .assets
        .assets
        .iter()
        .map(|a| a.path.to_lowercase())
        .collect();
    let mut to_declare: Vec<AssetEntry> = Vec::new();
    let mut copied: Vec<PathBuf> = Vec::new();
    for source in expand_sources(sources) {
        let story = project.story();
        match prepare(root, &sandbox, registry, story, &mut taken, &source, texts) {
            Ok(p) => {
                if !p.existing {
                    copied.extend(p.copied);
                    to_declare.push(p.entry.clone());
                }
                report.imported.push(Imported {
                    id: p.entry.id,
                    kind: p.entry.kind,
                    path: p.entry.path,
                    existing: p.existing,
                });
            }
            Err(e) => report.failed.push((source, e)),
        }
    }
    if to_declare.is_empty() {
        return report;
    }
    let n = to_declare.len();
    let label = texts.tn("import.undo", n, &[]);
    let base = project.story().assets.assets.len();
    let result = project.transaction_labeled(&label, |tx| {
        for (i, entry) in to_declare.iter().enumerate() {
            tx.execute(Command::SetAsset {
                id: entry.id,
                entry: Some(entry.clone()),
                index: Some(base + i),
            })?;
        }
        Ok(())
    });
    if let Err(e) = result {
        // Nothing was declared: do not leave the copies behind.
        for file in copied {
            let _ = fs::remove_file(file);
        }
        for i in std::mem::take(&mut report.imported) {
            report
                .failed
                .push((PathBuf::from(&i.path), ImportError::Io(e.to_string())));
        }
    }
    report
}

// ------------------------------------------------------------- rename, move

/// A file move made by the editor: undoing / redoing the manifest change
/// must move the file back / forth too ([`reconcile_moves`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveJournal(pub Vec<(String, String)>);

fn exists_exact(root: &Path, rel: &str) -> bool {
    // `exists` follows the OS's case rules; compare against the listing.
    let mut dir = root.to_path_buf();
    for seg in rel.split('/') {
        let found = fs::read_dir(&dir)
            .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy() == seg))
            .unwrap_or(false);
        if !found {
            return false;
        }
        dir.push(seg);
    }
    true
}

/// Renames a file of the project and updates its manifest path (same id,
/// one undo step; undo moves the file back through [`reconcile_moves`]).
/// The extension cannot change; the new path must be free (case-insensitive).
pub fn move_asset(
    project: &mut Project,
    root: &Path,
    journal: &mut MoveJournal,
    id: AssetId,
    new_path: &str,
    texts: &Translator,
) -> Result<(), ProjectError> {
    let reject = |m: String| ProjectError::rejected(m);
    let entry = project
        .story()
        .assets
        .get(id)
        .cloned()
        .ok_or_else(|| reject(texts.t("asset.err.not_found")))?;
    if entry.path == new_path {
        return Ok(());
    }
    if !ops::is_valid_asset_path(new_path) {
        return Err(reject(
            texts.tf("asset.err.invalid_path", &[("path", &new_path)]),
        ));
    }
    let ext_of = |p: &str| {
        p.rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default()
    };
    if ext_of(&entry.path) != ext_of(new_path) {
        return Err(reject(texts.t("asset.err.extension")));
    }
    let sandbox = Sandbox::new(root).map_err(|e| reject(e.to_string()))?;
    let from = sandbox
        .resolve(&entry.path)
        .map_err(|e| reject(e.to_string()))?;
    let to = sandbox
        .resolve(new_path)
        .map_err(|e| reject(e.to_string()))?;
    let case_only = entry.path.to_lowercase() == new_path.to_lowercase();
    if !case_only && (to.exists() || exists_exact(root, new_path)) {
        return Err(reject(texts.tf("asset.err.exists", &[("path", &new_path)])));
    }
    // The manifest first: it validates the path and the collisions.
    ops::set_asset_path(project, id, new_path)?;
    if from.exists() {
        let moved = to
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::rename(&from, &to));
        if let Err(e) = moved {
            let _ = project.undo();
            return Err(reject(texts.tf("asset.err.move_failed", &[("error", &e)])));
        }
        journal.0.push((entry.path, new_path.to_owned()));
    }
    Ok(())
}

/// After an undo or a redo: puts the files back where the manifest says, for
/// every move of the journal. A file is only moved when the manifest points
/// at the other end and nothing is there yet.
pub fn reconcile_moves(root: &Path, story: &Story, journal: &MoveJournal) -> Vec<(String, String)> {
    let mut done = Vec::new();
    for (a, b) in &journal.0 {
        for (want, have) in [(a, b), (b, a)] {
            let declared = story.assets.assets.iter().any(|e| &e.path == want);
            let (want_abs, have_abs) = (root.join(want), root.join(have));
            if declared
                && !exists_exact(root, want)
                && have_abs.exists()
                && !story.assets.assets.iter().any(|e| &e.path == have)
                && (want_abs.parent().map_or(Ok(()), fs::create_dir_all)).is_ok()
                && fs::rename(&have_abs, &want_abs).is_ok()
            {
                done.push((have.clone(), want.clone()));
            }
        }
    }
    done
}

// --------------------------------------------------------------------- lint

/// A problem between the manifest and the files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetIssue {
    /// The file does not exist.
    Missing { asset: AssetId, path: String },
    /// The file exists under another case (`Bg.png` for `bg.png`): works on
    /// Windows, breaks on Linux and the web.
    CaseMismatch {
        asset: AssetId,
        path: String,
        actual: String,
    },
    /// Two assets whose paths differ only by case: one file on Windows.
    CaseCollision {
        asset: AssetId,
        path: String,
        other: String,
    },
    /// A 3D scene file is invalid or has a finding (`orbit_scene::validate`).
    Scene3d {
        asset: AssetId,
        path: String,
        error: bool,
        detail: String,
    },
    /// A story command names a camera the 3D scene does not have.
    CameraMissing { asset: AssetId, camera: String },
}

impl AssetIssue {
    pub fn asset(&self) -> AssetId {
        match self {
            AssetIssue::Missing { asset, .. }
            | AssetIssue::CaseMismatch { asset, .. }
            | AssetIssue::CaseCollision { asset, .. }
            | AssetIssue::Scene3d { asset, .. }
            | AssetIssue::CameraMissing { asset, .. } => *asset,
        }
    }

    pub fn is_error(&self) -> bool {
        match self {
            AssetIssue::Missing { .. } => true,
            AssetIssue::Scene3d { error, .. } => *error,
            _ => false,
        }
    }

    pub fn message(&self, texts: &Translator) -> String {
        match self {
            AssetIssue::Missing { path, .. } => texts.tf("asset.issue.missing", &[("path", path)]),
            AssetIssue::CaseMismatch { path, actual, .. } => texts.tf(
                "asset.issue.case_mismatch",
                &[("path", path), ("actual", actual)],
            ),
            AssetIssue::CaseCollision { path, other, .. } => texts.tf(
                "asset.issue.case_collision",
                &[("path", path), ("other", other)],
            ),
            AssetIssue::Scene3d { path, detail, .. } => texts.tf(
                "asset.issue.scene3d",
                &[("path", path), ("detail", detail)],
            ),
            AssetIssue::CameraMissing { camera, .. } => {
                texts.tf("asset.issue.camera_missing", &[("camera", camera)])
            }
        }
    }
}

/// Checks the manifest paths against the files under `root`: missing files
/// and case mismatches (the folder listing is compared byte for byte, so the
/// result does not depend on the OS's case rules), plus paths that differ
/// only by case.
pub fn lint_asset_files(root: &Path, story: &Story) -> Vec<AssetIssue> {
    let mut issues = Vec::new();
    // `read_dir("")` fails: an empty root is the current directory.
    let root = if root.as_os_str().is_empty() {
        Path::new(".")
    } else {
        root
    };
    let mut listings: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for asset in &story.assets.assets {
        let lower = asset.path.to_lowercase();
        if let Some(other) = seen.get(&lower) {
            issues.push(AssetIssue::CaseCollision {
                asset: asset.id,
                path: asset.path.clone(),
                other: other.clone(),
            });
        } else {
            seen.insert(lower, asset.path.clone());
        }
        let mut dir = root.to_path_buf();
        let mut actual: Vec<String> = Vec::new();
        let mut exact = true;
        let mut found = true;
        for seg in asset.path.split('/') {
            let names = listings.entry(dir.clone()).or_insert_with(|| {
                fs::read_dir(&dir)
                    .map(|rd| {
                        rd.flatten()
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default()
            });
            if names.iter().any(|n| n == seg) {
                actual.push(seg.to_owned());
            } else if let Some(n) = names
                .iter()
                .find(|n| n.to_lowercase() == seg.to_lowercase())
            {
                exact = false;
                actual.push(n.clone());
            } else {
                found = false;
                break;
            }
            dir.push(actual.last().map_or("", String::as_str));
        }
        if !found {
            issues.push(AssetIssue::Missing {
                asset: asset.id,
                path: asset.path.clone(),
            });
        } else if !exact {
            issues.push(AssetIssue::CaseMismatch {
                asset: asset.id,
                path: asset.path.clone(),
                actual: actual.join("/"),
            });
        }
    }
    issues.extend(lint_scene3d(root, story));
    issues
}

// --------------------------------------------------------------------- scan

/// A file found in the project's asset folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskFile {
    /// Path relative to the root, `/` separators.
    pub path: String,
    pub len: u64,
    pub modified: Option<SystemTime>,
}

/// Every file under `root/assets` (recursively, dot-files and the editor's
/// state folder skipped), sorted by path.
pub fn scan_assets(root: &Path) -> Vec<DiskFile> {
    let mut out = Vec::new();
    let mut stack = vec![(root.join("assets"), "assets".to_owned())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let child = format!("{rel}/{name}");
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push((entry.path(), child));
            } else if meta.is_file() {
                out.push(DiskFile {
                    path: child,
                    len: meta.len(),
                    modified: meta.modified().ok(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe_and_stable() {
        assert_eq!(
            sanitize_file_name("My Room (final).PNG"),
            "My_Room_final.png"
        );
        assert_eq!(sanitize_file_name("bg_room-01.png"), "bg_room-01.png");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_file_name("C:\\dir\\sprite one.jpg"),
            "sprite_one.jpg"
        );
        assert_eq!(sanitize_file_name("..."), "asset");
        assert_eq!(sanitize_file_name("été.wav"), "été.wav");
    }

    #[test]
    fn only_a_clean_relative_path_is_a_manifest_path() {
        for ok in ["assets/images/a.png", "a.png"] {
            assert!(ops::is_valid_asset_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/a.png",
            "../a.png",
            "a/../b.png",
            "a//b.png",
            "a\\b.png",
            "C:/a.png",
        ] {
            assert!(!ops::is_valid_asset_path(bad), "{bad}");
        }
    }
}

// ---------------------------------------------------------------- 3D scenes

/// Where a 3D scene goes in the project (see [`plan_scene3d_target`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene3dTarget {
    /// Manifest path (relative, `/`).
    pub rel: String,
    pub abs: PathBuf,
    /// The scene file already lies under `assets/`: only declared.
    pub in_place: bool,
}

/// Manifest path of `file` when it lies under `root/assets`, `None` otherwise.
pub fn asset_rel_path(root: &Path, file: &Path) -> Option<String> {
    let root = fs::canonicalize(root).ok()?;
    let abs = fs::canonicalize(file).ok().or_else(|| {
        // A file that does not exist yet: canonicalize its folder.
        let parent = fs::canonicalize(file.parent()?).ok()?;
        Some(parent.join(file.file_name()?))
    })?;
    let rel = abs.strip_prefix(&root).ok()?;
    let rel = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    rel.starts_with("assets/").then_some(rel)
}

/// Decides where "add to project" puts the 3D scene named `name`: where it
/// is when `current` is already under `assets/`, otherwise a free
/// `assets/3d/<name>.scene3d.ron`.
pub fn plan_scene3d_target(
    root: &Path,
    story: &Story,
    name: &str,
    current: Option<&Path>,
) -> Scene3dTarget {
    if let Some(current) = current
        && let Some(rel) = asset_rel_path(root, current)
    {
        return Scene3dTarget {
            rel,
            abs: current.to_owned(),
            in_place: true,
        };
    }
    let taken: HashSet<String> = story
        .assets
        .assets
        .iter()
        .map(|a| a.path.to_lowercase())
        .collect();
    let stem = sanitize_file_name(name);
    let stem = stem.rsplit_once('.').map_or(stem.as_str(), |(s, _)| s);
    // `free_path` numbers collisions before the last dot (`a.scene3d-2.ron`),
    // which keeps the `.scene3d.ron` suffix readable enough.
    let wanted = format!("{stem}.scene3d.ron");
    let rel = free_path(root, asset_dir(AssetKind::Scene3d), &wanted, &taken);
    let abs = root.join(&rel);
    Scene3dTarget {
        rel,
        abs,
        in_place: false,
    }
}

/// Camera names of the 3D scene asset `asset`, read from its file. `None`
/// when the asset is not a 3D scene or its file cannot be read.
pub fn scene3d_cameras(root: &Path, story: &Story, asset: AssetId) -> Option<Vec<String>> {
    let entry = story.assets.assets.iter().find(|a| a.id == asset)?;
    if entry.kind != AssetKind::Scene3d {
        return None;
    }
    let text = fs::read_to_string(root.join(&entry.path)).ok()?;
    let scene = orbit_scene::from_ron(&text).ok()?;
    Some(scene.cameras().map(|(_, c)| c.name.clone()).collect())
}

/// Findings about the 3D scene files of the manifest, for the Problems
/// panel: invalid files, and `ShowScene3d` / `Camera3d` naming a camera the
/// scene does not have. Files that are missing are reported by the file lint.
pub fn lint_scene3d(root: &Path, story: &Story) -> Vec<AssetIssue> {
    use orbit_core::NodeKind;
    let mut issues = Vec::new();
    let mut cameras: HashMap<AssetId, Option<Vec<String>>> = HashMap::new();
    for asset in story
        .assets
        .assets
        .iter()
        .filter(|a| a.kind == AssetKind::Scene3d)
    {
        let Ok(text) = fs::read_to_string(root.join(&asset.path)) else {
            cameras.insert(asset.id, None);
            continue;
        };
        match orbit_scene::from_ron(&text) {
            Ok(scene) => {
                for issue in orbit_scene::validate(&scene) {
                    issues.push(AssetIssue::Scene3d {
                        asset: asset.id,
                        path: asset.path.clone(),
                        error: issue.severity == orbit_scene::Severity::Error,
                        detail: format!("{}: {}", issue.path, issue.message),
                    });
                }
                cameras.insert(
                    asset.id,
                    Some(scene.cameras().map(|(_, c)| c.name.clone()).collect()),
                );
            }
            Err(e) => {
                issues.push(AssetIssue::Scene3d {
                    asset: asset.id,
                    path: asset.path.clone(),
                    error: true,
                    detail: e,
                });
                cameras.insert(asset.id, None);
            }
        }
    }
    // Commands in reading order; a `Camera3d` is checked against the scene
    // shown last (a straight-line approximation: branches are not followed).
    for scene in &story.scenes {
        let mut shown: Option<AssetId> = None;
        for node in &scene.nodes {
            match &node.kind {
                NodeKind::ShowScene3d {
                    scene: asset,
                    camera,
                    ..
                } => {
                    shown = Some(*asset);
                    if let Some(name) = camera {
                        push_camera_issue(&mut issues, &cameras, *asset, name, true);
                    }
                }
                NodeKind::HideScene3d { .. } => shown = None,
                NodeKind::Camera3d { camera, .. } => {
                    if let Some(asset) = shown {
                        push_camera_issue(&mut issues, &cameras, asset, camera, false);
                    }
                }
                _ => {}
            }
        }
    }
    issues
}

fn push_camera_issue(
    issues: &mut Vec<AssetIssue>,
    cameras: &HashMap<AssetId, Option<Vec<String>>>,
    asset: AssetId,
    name: &str,
    allow_empty: bool,
) {
    if allow_empty && name.trim().is_empty() {
        return;
    }
    if let Some(Some(list)) = cameras.get(&asset)
        && !list.iter().any(|c| c == name)
    {
        let issue = AssetIssue::CameraMissing {
            asset,
            camera: name.to_owned(),
        };
        if !issues.contains(&issue) {
            issues.push(issue);
        }
    }
}

/// The 3D scene on screen when node `id` runs, found by reading its scene
/// upwards for the last `ShowScene3d` (not cancelled by a `HideScene3d`).
pub fn shown_scene3d_before(story: &Story, id: orbit_core::NodeId) -> Option<AssetId> {
    use orbit_core::NodeKind;
    let scene = story
        .scenes
        .iter()
        .find(|s| s.nodes.iter().any(|n| n.id == id))?;
    let index = scene.nodes.iter().position(|n| n.id == id)?;
    for node in scene.nodes[..index].iter().rev() {
        match &node.kind {
            NodeKind::ShowScene3d { scene, .. } => return Some(*scene),
            NodeKind::HideScene3d { .. } => return None,
            _ => {}
        }
    }
    None
}
