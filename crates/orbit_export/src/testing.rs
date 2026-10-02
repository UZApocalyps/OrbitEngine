//! Helpers for the tests of this crate and of the tools built on it.

use std::path::{Path, PathBuf};

use crate::project::ExportProject;

/// A temporary directory removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("orbit_export_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temporary directory");
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

/// The root of the OrbitEngine repository (this crate is `crates/orbit_export`).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Copies the demo assets (`assets/demo`) into `dir/assets/demo`.
pub fn restore_demo_assets(dir: &Path) {
    let from = repo_root().join("assets/demo");
    let to = dir.join("assets/demo");
    std::fs::create_dir_all(&to).expect("assets dir");
    for entry in std::fs::read_dir(from).expect("repo demo assets") {
        let entry = entry.expect("entry");
        std::fs::copy(entry.path(), to.join(entry.file_name())).expect("copy asset");
    }
}

/// Builds a project of the demo story in `dir`: `orbit.project.ron`,
/// `story.ron`, its French translation and its assets.
pub fn demo_project(dir: &Path) -> ExportProject {
    let stories = repo_root().join("examples/stories");
    std::fs::copy(stories.join("demo.ron"), dir.join("story.ron")).expect("copy story");
    std::fs::create_dir_all(dir.join("i18n")).expect("i18n dir");
    std::fs::copy(
        stories.join("i18n/demo.fr.ron"),
        dir.join("i18n/story.fr.ron"),
    )
    .expect("copy translation");
    restore_demo_assets(dir);
    let manifest = orbit_project::ProjectManifest::new("Demo");
    std::fs::write(
        dir.join(orbit_project::MANIFEST_FILE),
        manifest.to_ron().expect("manifest"),
    )
    .expect("write manifest");
    ExportProject::open(dir).expect("demo project opens")
}

/// Like [`demo_project`], but built from the small dedicated test story
/// (`examples/stories/test_minimal.ron`), which does not change when the demo does.
pub fn fixture_project(dir: &Path) -> ExportProject {
    let stories = repo_root().join("examples/stories");
    std::fs::copy(stories.join("test_minimal.ron"), dir.join("story.ron")).expect("copy story");
    std::fs::create_dir_all(dir.join("i18n")).expect("i18n dir");
    std::fs::copy(
        stories.join("i18n/test_minimal.fr.ron"),
        dir.join("i18n/story.fr.ron"),
    )
    .expect("copy translation");
    restore_demo_assets(dir);
    let manifest = orbit_project::ProjectManifest::new("Test Minimal");
    std::fs::write(
        dir.join(orbit_project::MANIFEST_FILE),
        manifest.to_ron().expect("manifest"),
    )
    .expect("write manifest");
    ExportProject::open(dir).expect("fixture project opens")
}
