//! The project to export: where it is, its manifest and its profiles.

use std::path::{Path, PathBuf};

use orbit_project::{
    DEFAULT_STORY_FILE, ExportProfile, MANIFEST_FILE, ProjectManifest,
};

use crate::error::{ExportError, Result};

/// A project folder (or a bare story file) ready to be compiled.
#[derive(Debug, Clone)]
pub struct ExportProject {
    /// The project folder: the asset paths of the story are relative to it.
    pub root: PathBuf,
    /// The story file, relative to `root`.
    pub story_file: String,
    pub manifest: Option<ProjectManifest>,
}

impl ExportProject {
    /// Opens `path`: a project folder (holding `orbit.project.ron` or
    /// `story.ron`), the manifest itself, or a story `.ron` file (its folder
    /// is then the project root).
    pub fn open(path: &Path) -> Result<Self> {
        let root_and_story = |root: &Path, manifest: Option<ProjectManifest>| {
            let story_file = manifest
                .as_ref()
                .map_or_else(|| DEFAULT_STORY_FILE.to_owned(), |m| m.story.clone());
            (root.to_owned(), story_file, manifest)
        };
        let (root, story_file, manifest) = if path.is_dir() {
            let manifest_path = path.join(MANIFEST_FILE);
            let manifest = if manifest_path.is_file() {
                Some(read_manifest(&manifest_path)?)
            } else {
                None
            };
            root_and_story(path, manifest)
        } else if path.file_name().is_some_and(|n| n == MANIFEST_FILE) {
            let root = path.parent().unwrap_or(Path::new("."));
            root_and_story(root, Some(read_manifest(path)?))
        } else if path.is_file() {
            let root = match path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => Path::new("."),
            };
            let story_file = path
                .file_name()
                .map_or_else(|| DEFAULT_STORY_FILE.to_owned(), |n| n.to_string_lossy().into_owned());
            (root.to_owned(), story_file, None)
        } else {
            return Err(ExportError::Project(format!(
                "`{}` is neither a project folder nor a story file",
                path.display()
            )));
        };
        let project = Self {
            root,
            story_file,
            manifest,
        };
        if !project.story_path().is_file() {
            return Err(ExportError::Project(format!(
                "the story `{}` does not exist",
                project.story_path().display()
            )));
        }
        Ok(project)
    }

    /// Uses `root` instead of the story's folder as the base of the asset
    /// paths (a bare story file whose manifest paths are relative to
    /// elsewhere, like the repository demo).
    pub fn with_asset_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = root.into();
        self
    }

    pub fn story_path(&self) -> PathBuf {
        self.root.join(&self.story_file)
    }

    /// Name of the project: the manifest's, else the story file's name.
    pub fn name(&self) -> String {
        match &self.manifest {
            Some(m) if !m.name.trim().is_empty() => m.name.clone(),
            _ => Path::new(&self.story_file)
                .file_stem()
                .map_or_else(|| "game".to_owned(), |s| s.to_string_lossy().into_owned()),
        }
    }

    /// Export profiles of the project (the implicit one when it defines none).
    pub fn profiles(&self) -> Vec<ExportProfile> {
        match &self.manifest {
            Some(m) if !m.export_profiles.is_empty() => m.export_profiles.clone(),
            _ => vec![ExportProfile::new("release", &self.name())],
        }
    }

    /// The profile called `name`, or the first one.
    pub fn profile(&self, name: Option<&str>) -> Result<ExportProfile> {
        let profiles = self.profiles();
        match name {
            None => Ok(profiles.into_iter().next().expect("at least one profile")),
            Some(name) => profiles.into_iter().find(|p| p.name == name).ok_or_else(|| {
                ExportError::Project(format!(
                    "the project has no export profile `{name}` (profiles: {})",
                    self.profiles()
                        .iter()
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }),
        }
    }
}

fn read_manifest(path: &Path) -> Result<ProjectManifest> {
    let text = std::fs::read_to_string(path).map_err(ExportError::io(path))?;
    ProjectManifest::from_ron(&text)
        .map_err(|e| ExportError::Project(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn opens_folders_manifests_and_bare_stories() {
        let dir = TempDir::new("project");
        std::fs::write(dir.path().join("story.ron"), "x").unwrap();
        // No manifest: the folder with story.ron works.
        let p = ExportProject::open(dir.path()).unwrap();
        assert!(p.manifest.is_none());
        assert_eq!(p.profiles().len(), 1);
        assert_eq!(p.profile(None).unwrap().name, "release");
        assert!(p.profile(Some("nope")).is_err());
        // With a manifest and profiles.
        let mut m = ProjectManifest::new("Mon jeu");
        m.export_profiles
            .push(ExportProfile::new("demo", "Mon jeu demo"));
        std::fs::write(dir.path().join(MANIFEST_FILE), m.to_ron().unwrap()).unwrap();
        let p = ExportProject::open(&dir.path().join(MANIFEST_FILE)).unwrap();
        assert_eq!(p.name(), "Mon jeu");
        assert_eq!(p.profile(Some("demo")).unwrap().game_name, "Mon jeu demo");
        // A bare story file.
        let bare = ExportProject::open(&dir.path().join("story.ron")).unwrap();
        assert_eq!(bare.story_file, "story.ron");
        assert!(ExportProject::open(&dir.path().join("missing")).is_err());
    }
}
