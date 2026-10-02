//! File-system side of projects: atomic writes, project folders, templates
//! and crash recovery.
//!
//! The editor never writes project files in place: every write goes to a
//! temporary file in the same folder, is flushed, then renamed over the
//! target (`rename` replaces an existing file on Windows and Linux), so a
//! crash leaves either the old or the new file, never half of one.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use orbit_core::{Node, NodeId, NodeKind, Scene, SceneId, Story};
use orbit_project::{
    DEFAULT_STORY_FILE, MANIFEST_FILE, ProjectManifest, ProjectStorage, STATE_DIR, load_manifest,
};
use serde::{Deserialize, Serialize};

/// Writes `contents` to `path` atomically (temp file + fsync + rename),
/// creating the parent folder if needed.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let tmp = dir.join(format!(".{}.tmp", name.to_string_lossy()));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// [`ProjectStorage`] on a folder of the file system.
#[derive(Clone, Debug)]
pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, rel: &str) -> io::Result<PathBuf> {
        let rel = Path::new(rel);
        let escapes = rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
        if escapes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path leaves the project folder",
            ));
        }
        Ok(self.root.join(rel))
    }
}

impl ProjectStorage for FsStorage {
    fn read_to_string(&self, path: &str) -> io::Result<String> {
        fs::read_to_string(self.resolve(path)?)
    }

    fn write_atomic(&self, path: &str, contents: &str) -> io::Result<()> {
        write_atomic(&self.resolve(path)?, contents.as_bytes())
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).is_ok_and(|p| p.exists())
    }

    fn remove(&self, path: &str) -> io::Result<()> {
        match fs::remove_file(self.resolve(path)?) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// Where a project lives on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectLocation {
    pub root: PathBuf,
    /// Story file relative to `root`.
    pub story_file: String,
    /// `None` for a bare story file opened without a project folder.
    pub manifest: Option<ProjectManifest>,
}

impl ProjectLocation {
    pub fn storage(&self) -> FsStorage {
        FsStorage::new(&self.root)
    }

    pub fn story_path(&self) -> PathBuf {
        self.root.join(&self.story_file)
    }

    /// Opens a project folder, its `orbit.project.ron`, a folder holding a
    /// `story.ron`, or a bare story `.ron` file.
    pub fn resolve(path: &Path) -> Result<Self, String> {
        let from_manifest = |root: &Path| -> Result<Self, String> {
            let storage = FsStorage::new(root);
            let manifest = load_manifest(&storage).map_err(|e| e.to_string())?;
            Ok(Self {
                root: root.to_owned(),
                story_file: manifest.story.clone(),
                manifest: Some(manifest),
            })
        };
        if path.is_dir() {
            if path.join(MANIFEST_FILE).is_file() {
                return from_manifest(path);
            }
            if path.join(DEFAULT_STORY_FILE).is_file() {
                return Ok(Self {
                    root: path.to_owned(),
                    story_file: DEFAULT_STORY_FILE.to_owned(),
                    manifest: None,
                });
            }
            return Err(format!(
                "{} ne contient ni {MANIFEST_FILE} ni {DEFAULT_STORY_FILE}",
                path.display()
            ));
        }
        if !path.is_file() {
            return Err(format!("{} n'existe pas", path.display()));
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file_name == MANIFEST_FILE {
            return from_manifest(parent);
        }
        // A story file that a manifest of its folder declares as its story.
        if parent.join(MANIFEST_FILE).is_file()
            && let Ok(found) = from_manifest(parent)
            && found.story_file == file_name
        {
            return Ok(found);
        }
        Ok(Self {
            root: parent.to_owned(),
            story_file: file_name,
            manifest: None,
        })
    }

    /// Folder for recovery files: inside the project when it has a manifest
    /// (`.orbit/recovery`), else in the user config folder, so that opening a
    /// bare story file never litters its folder.
    pub fn recovery_dir(&self, config_dir: Option<&Path>) -> PathBuf {
        match (&self.manifest, config_dir) {
            (None, Some(config)) => config
                .join("recovery")
                .join(format!("{:016x}", path_hash(&self.story_path()))),
            _ => self.root.join(STATE_DIR).join("recovery"),
        }
    }
}

/// Stable FNV-1a hash of a path, to name per-file state folders.
fn path_hash(path: &Path) -> u64 {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for b in canonical.to_string_lossy().bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A folder name from a project name: no characters Windows forbids.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    cleaned.trim_matches(|c| c == '.' || c == ' ').to_owned()
}

/// The story of a freshly created project: one scene, two lines.
pub fn minimal_story(name: &str) -> Story {
    let mut story = Story::new(name);
    let first = story.strings.add(format!("Bienvenue dans « {name} »."));
    let second = story
        .strings
        .add("Sélectionnez cette réplique dans l'explorateur pour la modifier.");
    let say = |text_id| Node {
        id: NodeId::new(),
        kind: NodeKind::Say {
            speaker: None,
            text_id,
        },
    };
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "Introduction".to_owned(),
        nodes: vec![say(first), say(second)],
    });
    story
}

/// Creates `parent/<folder_name(name)>` with a manifest, a minimal story and
/// an `assets/` folder. Refuses a non-empty existing folder. Returns the
/// project folder.
pub fn create_project(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    let folder = folder_name(name);
    if folder.is_empty() {
        return Err("le nom du projet est vide".to_owned());
    }
    let root = parent.join(&folder);
    if root.exists() && root.read_dir().map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{} existe déjà et n'est pas vide", root.display()));
    }
    let io = |e: io::Error| format!("{}: {e}", root.display());
    fs::create_dir_all(root.join("assets")).map_err(io)?;
    let storage = FsStorage::new(&root);
    let manifest = ProjectManifest::new(name);
    orbit_project::save_manifest(&storage, &manifest).map_err(|e| e.to_string())?;
    orbit_project::save_story(&storage, &manifest.story, &minimal_story(name))
        .map_err(|e| e.to_string())?;
    write_atomic(
        &root.join(".gitignore"),
        format!("{STATE_DIR}/\n").as_bytes(),
    )
    .map_err(io)?;
    Ok(root)
}

// ---------------------------------------------------------------- recovery

const RECOVERY_STORY: &str = "story.recovery.ron";
const RECOVERY_META: &str = "recovery.ron";

/// What a recovery folder says about the snapshot it holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryMeta {
    /// Story file the snapshot belongs to.
    pub story_file: String,
    /// Unix time, milliseconds.
    pub saved_at_ms: u64,
}

/// Crash recovery files of one project: a snapshot of the unsaved story,
/// written periodically, never over the project's own files. Removed on a
/// clean save, close or discard; its presence at the next launch means the
/// previous session did not end cleanly.
#[derive(Clone, Debug)]
pub struct Recovery {
    dir: PathBuf,
}

impl Recovery {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn write(&self, story: &Story, meta: &RecoveryMeta) -> Result<(), String> {
        let text = story.to_ron().map_err(|e| e.to_string())?;
        write_atomic(&self.dir.join(RECOVERY_STORY), text.as_bytes()).map_err(|e| e.to_string())?;
        let meta = ron::ser::to_string(meta).map_err(|e| e.to_string())?;
        // The meta file is written last: without it the snapshot is ignored.
        write_atomic(&self.dir.join(RECOVERY_META), meta.as_bytes()).map_err(|e| e.to_string())
    }

    /// The pending snapshot, if a previous session left one.
    pub fn pending(&self) -> Option<RecoveryMeta> {
        let meta = fs::read_to_string(self.dir.join(RECOVERY_META)).ok()?;
        let meta: RecoveryMeta = ron::from_str(&meta).ok()?;
        self.dir.join(RECOVERY_STORY).is_file().then_some(meta)
    }

    pub fn load(&self) -> Result<Story, String> {
        let text = fs::read_to_string(self.dir.join(RECOVERY_STORY)).map_err(|e| e.to_string())?;
        Story::from_ron_unchecked(&text).map_err(|e| e.to_string())
    }

    pub fn clear(&self) {
        for name in [RECOVERY_META, RECOVERY_STORY] {
            let _ = fs::remove_file(self.dir.join(name));
        }
        // Only succeeds when empty; harmless otherwise.
        let _ = fs::remove_dir(&self.dir);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A temporary folder removed on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "orbit-editor-test-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TempDir;
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = TempDir::new("atomic");
        let file = dir.path().join("sub").join("a.txt");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.txt"]);
    }

    #[test]
    fn storage_refuses_paths_outside_the_project() {
        let dir = TempDir::new("escape");
        let storage = FsStorage::new(dir.path());
        assert!(storage.write_atomic("../evil.txt", "x").is_err());
        assert!(storage.read_to_string("/etc/passwd").is_err());
        assert!(storage.remove("missing.txt").is_ok());
    }

    #[test]
    fn created_project_is_valid_and_resolvable() {
        let dir = TempDir::new("create");
        let root = create_project(dir.path(), "Mon jeu: v1").unwrap();
        assert_eq!(root.file_name().unwrap(), "Mon jeu- v1");

        let location = ProjectLocation::resolve(&root).unwrap();
        assert_eq!(location.story_file, "story.ron");
        assert_eq!(location.manifest.as_ref().unwrap().name, "Mon jeu: v1");
        // Same project whichever path is given.
        assert_eq!(
            ProjectLocation::resolve(&root.join(MANIFEST_FILE)).unwrap(),
            location
        );
        assert_eq!(
            ProjectLocation::resolve(&root.join("story.ron")).unwrap(),
            location
        );

        let story = orbit_project::load_story(&location.storage(), &location.story_file).unwrap();
        let report = orbit_core::validate(&story);
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        // A second project in the same place is refused, an empty name too.
        assert!(create_project(dir.path(), "Mon jeu: v1").is_err());
        assert!(create_project(dir.path(), "  ").is_err());
    }

    #[test]
    fn bare_story_file_has_no_manifest() {
        let dir = TempDir::new("bare");
        let file = dir.path().join("demo.ron");
        fs::write(&file, Story::new("t").to_ron().unwrap()).unwrap();
        let location = ProjectLocation::resolve(&file).unwrap();
        assert!(location.manifest.is_none());
        assert_eq!(location.story_file, "demo.ron");
        assert_eq!(location.root, dir.path());
        // Recovery goes to the config folder, not next to the file.
        let config = TempDir::new("cfg");
        let rec = location.recovery_dir(Some(config.path()));
        assert!(rec.starts_with(config.path()));
        let rec = location.recovery_dir(None);
        assert!(rec.starts_with(dir.path()));

        assert!(ProjectLocation::resolve(&dir.path().join("nope.ron")).is_err());
        assert!(ProjectLocation::resolve(&dir.path().join("x")).is_err());
    }

    #[test]
    fn recovery_round_trip_and_clear() {
        let dir = TempDir::new("recovery");
        let recovery = Recovery::new(dir.path().join(".orbit").join("recovery"));
        assert!(recovery.pending().is_none());
        let story = minimal_story("t");
        let meta = RecoveryMeta {
            story_file: "story.ron".into(),
            saved_at_ms: 42,
        };
        recovery.write(&story, &meta).unwrap();
        assert_eq!(recovery.pending(), Some(meta));
        assert_eq!(recovery.load().unwrap(), story);
        recovery.clear();
        assert!(recovery.pending().is_none());
    }

    #[test]
    fn snapshot_without_meta_is_ignored() {
        let dir = TempDir::new("partial");
        let recovery = Recovery::new(dir.path().to_owned());
        fs::write(dir.path().join(RECOVERY_STORY), "x").unwrap();
        assert!(recovery.pending().is_none());
    }
}
