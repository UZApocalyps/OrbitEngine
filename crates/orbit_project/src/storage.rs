//! Project files: the manifest and the storage abstraction.
//!
//! A project is a folder:
//!
//! ```text
//! my_game/
//!   orbit.project.ron   manifest (this module): name, story file, plugins
//!   story.ron           the story IR (orbit_core), canonical RON
//!   assets/ ...         files referenced by the story's asset manifest
//!   .orbit/             tool state, never part of the game (recovery files...)
//! ```
//!
//! Disk access goes through [`ProjectStorage`] so that this crate stays free
//! of any platform dependency (and compiles to wasm); the file system
//! implementation lives in the editor / CLI.

use std::io;

use orbit_core::{CoreError, Story};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::export::ExportProfile;

/// File name of the manifest at the project root.
pub const MANIFEST_FILE: &str = "orbit.project.ron";
/// Default story file of a new project (also what the player and
/// `orbit validate` look for in a project folder).
pub const DEFAULT_STORY_FILE: &str = "story.ron";
/// Folder (relative to the root) for tool state that is not game content.
pub const STATE_DIR: &str = ".orbit";
/// Current manifest format.
pub const MANIFEST_FORMAT: u32 = 1;

/// A plugin a project needs, with the version it was last saved with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRequirement {
    pub id: String,
    pub version: String,
}

/// Content of `orbit.project.ron`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub format: u32,
    pub name: String,
    /// Story file, relative to the project root.
    pub story: String,
    #[serde(default)]
    pub plugins: Vec<PluginRequirement>,
    /// Export profiles (see [`ExportProfile`]); none means one implicit
    /// profile derived from the project name.
    #[serde(default)]
    pub export_profiles: Vec<ExportProfile>,
}

impl ProjectManifest {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format: MANIFEST_FORMAT,
            name: name.into(),
            story: DEFAULT_STORY_FILE.to_owned(),
            plugins: Vec::new(),
            export_profiles: Vec::new(),
        }
    }

    /// The profile called `name`, or, when `None`, the first one; a project
    /// without profiles gets an implicit `release` profile named after it.
    pub fn profile(&self, name: Option<&str>) -> Option<ExportProfile> {
        match name {
            Some(name) => self.export_profiles.iter().find(|p| p.name == name).cloned(),
            None => Some(
                self.export_profiles
                    .first()
                    .cloned()
                    .unwrap_or_else(|| ExportProfile::new("release", &self.name)),
            ),
        }
    }

    pub fn to_ron(&self) -> Result<String, ProjectFileError> {
        let cfg = ron::ser::PrettyConfig::new().indentor("    ".to_owned());
        ron::ser::to_string_pretty(self, cfg).map_err(|e| ProjectFileError::Manifest(e.to_string()))
    }

    pub fn from_ron(source: &str) -> Result<Self, ProjectFileError> {
        let manifest: Self =
            ron::from_str(source).map_err(|e| ProjectFileError::Manifest(e.to_string()))?;
        if manifest.format > MANIFEST_FORMAT {
            return Err(ProjectFileError::Manifest(format!(
                "manifest format {} is newer than supported ({MANIFEST_FORMAT})",
                manifest.format
            )));
        }
        Ok(manifest)
    }
}

#[derive(Debug, Error)]
pub enum ProjectFileError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("invalid project manifest: {0}")]
    Manifest(String),
    #[error(transparent)]
    Story(#[from] CoreError),
}

/// Reads and writes project files by path relative to the project root.
pub trait ProjectStorage {
    fn read_to_string(&self, path: &str) -> io::Result<String>;
    /// Writes the whole file so that a crash never leaves it half written
    /// (temporary file in the same folder, flush, rename).
    fn write_atomic(&self, path: &str, contents: &str) -> io::Result<()>;
    fn exists(&self, path: &str) -> bool;
    /// Removes a file; a missing file is not an error.
    fn remove(&self, path: &str) -> io::Result<()>;
}

fn io_error(path: &str) -> impl FnOnce(io::Error) -> ProjectFileError + '_ {
    move |source| ProjectFileError::Io {
        path: path.to_owned(),
        source,
    }
}

/// Loads the manifest of a project folder.
pub fn load_manifest(storage: &dyn ProjectStorage) -> Result<ProjectManifest, ProjectFileError> {
    let text = storage
        .read_to_string(MANIFEST_FILE)
        .map_err(io_error(MANIFEST_FILE))?;
    ProjectManifest::from_ron(&text)
}

pub fn save_manifest(
    storage: &dyn ProjectStorage,
    manifest: &ProjectManifest,
) -> Result<(), ProjectFileError> {
    storage
        .write_atomic(MANIFEST_FILE, &manifest.to_ron()?)
        .map_err(io_error(MANIFEST_FILE))
}

/// Parses (migrating) a story file without semantic validation, so that the
/// editor can open and repair a broken story.
pub fn load_story(storage: &dyn ProjectStorage, file: &str) -> Result<Story, ProjectFileError> {
    let text = storage.read_to_string(file).map_err(io_error(file))?;
    Ok(Story::from_ron_unchecked(&text)?)
}

pub fn save_story(
    storage: &dyn ProjectStorage,
    file: &str,
    story: &Story,
) -> Result<(), ProjectFileError> {
    storage
        .write_atomic(file, &story.to_ron()?)
        .map_err(io_error(file))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct Memory(RefCell<HashMap<String, String>>);

    impl ProjectStorage for Memory {
        fn read_to_string(&self, path: &str) -> io::Result<String> {
            self.0
                .borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing"))
        }
        fn write_atomic(&self, path: &str, contents: &str) -> io::Result<()> {
            self.0.borrow_mut().insert(path.into(), contents.into());
            Ok(())
        }
        fn exists(&self, path: &str) -> bool {
            self.0.borrow().contains_key(path)
        }
        fn remove(&self, path: &str) -> io::Result<()> {
            self.0.borrow_mut().remove(path);
            Ok(())
        }
    }

    #[test]
    fn manifest_round_trips() {
        let storage = Memory::default();
        let mut manifest = ProjectManifest::new("Mon jeu");
        manifest.plugins.push(PluginRequirement {
            id: "acme.anim".into(),
            version: "1.0.0".into(),
        });
        save_manifest(&storage, &manifest).unwrap();
        assert_eq!(load_manifest(&storage).unwrap(), manifest);
    }

    #[test]
    fn export_profiles_round_trip_and_default() {
        let mut manifest = ProjectManifest::new("Mon jeu");
        assert_eq!(manifest.profile(None).unwrap().game_id, "mon-jeu");
        assert!(manifest.profile(Some("demo")).is_none());
        let mut demo = ExportProfile::new("demo", "Mon jeu (demo)");
        demo.version = "0.1.0".into();
        manifest.export_profiles.push(demo.clone());
        let back = ProjectManifest::from_ron(&manifest.to_ron().unwrap()).unwrap();
        assert_eq!(back, manifest);
        assert_eq!(back.profile(Some("demo")), Some(demo.clone()));
        assert_eq!(back.profile(None), Some(demo));
        // A manifest written before profiles existed still loads.
        let old = ProjectManifest::from_ron("(format: 1, name: \"x\", story: \"story.ron\")").unwrap();
        assert!(old.export_profiles.is_empty());
    }

    #[test]
    fn newer_manifest_is_refused_and_garbage_reported() {
        assert!(ProjectManifest::from_ron("(format: 99, name: \"x\", story: \"s.ron\")").is_err());
        assert!(matches!(
            ProjectManifest::from_ron("(((("),
            Err(ProjectFileError::Manifest(_))
        ));
        let storage = Memory::default();
        assert!(matches!(
            load_manifest(&storage),
            Err(ProjectFileError::Io { .. })
        ));
    }

    #[test]
    fn story_round_trips() {
        let storage = Memory::default();
        let story = Story::new("t");
        save_story(&storage, "story.ron", &story).unwrap();
        assert_eq!(load_story(&storage, "story.ron").unwrap(), story);
    }
}
