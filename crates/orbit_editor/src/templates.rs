//! Project templates of the New Project dialog (M7-12).
//!
//! A template is plain data under `assets/templates/<id>/`: a thumbnail and,
//! except for the blank one, a small project (`project/story.orbit`,
//! `project/ui/`, `project/assets/`) embedded in the editor binary. Creating
//! a project from a template copies those files next to a fresh manifest and
//! renames the story after the project.

use std::path::{Path, PathBuf};

use orbit_editor_api::Translator;
use orbit_project::{DEFAULT_STORY_FILE, ProjectManifest};

use crate::storage::{FsStorage, create_project, write_atomic};

/// One template.
pub struct Template {
    /// Stable identifier.
    pub id: &'static str,
    /// Text keys of its name and description (`template.<id>.name` / `.desc`).
    pub name_key: &'static str,
    pub desc_key: &'static str,
    /// PNG thumbnail.
    pub thumbnail: &'static [u8],
    /// Project files, `(path relative to the project, bytes)`. Empty for the
    /// blank template (the editor's own minimal story is used).
    pub files: &'static [(&'static str, &'static [u8])],
}

macro_rules! tpl {
    ($dir:literal, $file:literal) => {
        (
            $file,
            include_bytes!(concat!(
                "../../../assets/templates/",
                $dir,
                "/project/",
                $file
            )) as &[u8],
        )
    };
}

/// Every template, in the order of the dialog (the first is the default).
pub const TEMPLATES: &[Template] = &[
    Template {
        id: "blank",
        name_key: "template.blank.name",
        desc_key: "template.blank.desc",
        thumbnail: include_bytes!("../../../assets/templates/blank/thumbnail.png"),
        files: &[],
    },
    Template {
        id: "visual_novel",
        name_key: "template.visual_novel.name",
        desc_key: "template.visual_novel.desc",
        thumbnail: include_bytes!("../../../assets/templates/visual_novel/thumbnail.png"),
        files: &[
            tpl!("visual_novel", "story.orbit"),
            tpl!("visual_novel", "assets/bg_dawn.png"),
            tpl!("visual_novel", "assets/bg_night.png"),
            tpl!("visual_novel", "assets/hero.png"),
            tpl!("visual_novel", "assets/friend.png"),
            tpl!("visual_novel", "ui/load.screen.ron"),
            tpl!("visual_novel", "ui/main_menu.screen.ron"),
            tpl!("visual_novel", "ui/pause.screen.ron"),
            tpl!("visual_novel", "ui/preferences.screen.ron"),
            tpl!("visual_novel", "ui/save.screen.ron"),
            tpl!("visual_novel", "ui/theme.ron"),
            tpl!("visual_novel", "ui/ui.ron"),
        ],
    },
];

/// The template called `id`.
pub fn find(id: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.id == id)
}

/// Creates `parent/<folder>` from the template `template_id` (see
/// [`create_project`] for the name and folder rules). Unknown id: blank.
pub fn create_project_from_template(
    parent: &Path,
    name: &str,
    template_id: &str,
    texts: &Translator,
) -> Result<PathBuf, String> {
    let root = create_project(parent, name, texts)?;
    let Some(template) = find(template_id).filter(|t| !t.files.is_empty()) else {
        // Blank: the minimal story, ended so that it validates without warning.
        let storage = FsStorage::new(&root);
        let file = ProjectManifest::new(name.trim()).story;
        let mut story = orbit_project::load_story(&storage, &file).map_err(|e| e.to_string())?;
        if let Some(scene) = story.scenes.last_mut() {
            scene.nodes.push(orbit_core::Node {
                id: orbit_core::NodeId::new(),
                kind: orbit_core::NodeKind::End,
            });
        }
        orbit_project::save_story(&storage, &file, &story).map_err(|e| e.to_string())?;
        return Ok(root);
    };
    let io = |e: std::io::Error| format!("{}: {e}", root.display());
    for (path, bytes) in template.files {
        if *path == DEFAULT_STORY_FILE {
            continue;
        }
        write_atomic(&root.join(path), bytes).map_err(io)?;
    }
    let text = template
        .files
        .iter()
        .find(|(p, _)| *p == DEFAULT_STORY_FILE)
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .ok_or_else(|| format!("template {}: no story", template.id))?;
    let mut story = orbit_core::Story::from_ron(&text).map_err(|e| e.to_string())?;
    story.title = name.trim().to_owned();
    let storage = FsStorage::new(&root);
    let manifest = ProjectManifest::new(name.trim());
    orbit_project::save_story(&storage, &manifest.story, &story).map_err(|e| e.to_string())?;
    Ok(root)
}
