//! Project templates (M7-12): each one creates a project that validates with
//! no error and no warning, has every asset on disk, a valid `ui/` folder,
//! and opens in the editor. Also: the manual covers both languages.

use std::path::{Path, PathBuf};

use egui::Context;
use orbit_core::{Story, check_asset_files, validate};
use orbit_editor::builtin::manual;
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor::i18n::translator;
use orbit_editor::templates::{TEMPLATES, create_project_from_template};
use orbit_editor_api::Lang;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-editor-tpl-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn load(root: &Path) -> Story {
    Story::from_ron_unchecked(&std::fs::read_to_string(root.join("story.orbit")).unwrap()).unwrap()
}

#[test]
fn every_template_validates_clean_and_opens() {
    for template in TEMPLATES {
        let dir = TempDir::new(template.id);
        let root =
            create_project_from_template(&dir.0, "My game", template.id, &translator(Lang::En))
                .unwrap_or_else(|e| panic!("{}: {e}", template.id));
        let story = load(&root);
        assert_eq!(story.title, "My game", "{}", template.id);
        let report = validate(&story);
        assert!(
            report.errors.is_empty(),
            "{}: {:?}",
            template.id,
            report.errors
        );
        assert!(
            report.warnings.is_empty(),
            "{}: {:?}",
            template.id,
            report.warnings
        );
        let missing = check_asset_files(&story, |p| root.join(p).is_file());
        assert!(missing.is_empty(), "{}: {missing:?}", template.id);
        let ui = root.join("ui");
        if ui.is_dir() {
            let bundle = orbit_ui::UiBundle::load_dir(&ui)
                .unwrap_or_else(|e| panic!("{}: ui: {e}", template.id));
            let issues = bundle.validate();
            assert!(issues.is_empty(), "{}: {issues:?}", template.id);
        }
        // Opens in the editor.
        let ctx = Context::default();
        let mut core = EditorCore::new(
            CoreOptions {
                config_dir: None,
                ..CoreOptions::with_language(Lang::En)
            },
            &ctx,
        );
        core.open_project(&root);
        assert_eq!(
            core.state.project.as_ref().map(|p| p.name.as_str()),
            Some("My game"),
            "{}",
            template.id
        );
    }
}

#[test]
fn visual_novel_template_has_the_promised_content() {
    let dir = TempDir::new("content");
    let root =
        create_project_from_template(&dir.0, "VN", "visual_novel", &translator(Lang::Fr)).unwrap();
    let story = load(&root);
    assert_eq!(story.scenes.len(), 2);
    assert!(story.characters.len() >= 2);
    assert!(
        story
            .scenes
            .iter()
            .flat_map(|s| &s.nodes)
            .any(|n| matches!(n.kind, orbit_core::NodeKind::Choice { .. }))
    );
    assert!(root.join("ui/main_menu.screen.ron").is_file());
    let ui = std::fs::read_to_string(root.join("ui/ui.ron")).unwrap();
    assert!(ui.contains("splashes"));
}

#[test]
fn template_texts_exist_in_both_languages() {
    for lang in Lang::ALL {
        let texts = translator(lang);
        for template in TEMPLATES {
            for key in [template.name_key, template.desc_key] {
                assert_ne!(texts.get(key), key, "{key} missing in {}", lang.code());
            }
        }
    }
}

#[test]
fn manual_is_complete_in_both_languages() {
    for lang in Lang::ALL {
        let chapters = manual::chapters(lang);
        assert_eq!(chapters.len(), 14);
        for (name, md) in chapters {
            let blocks = manual::parse(md);
            assert!(blocks.len() > 5, "{name} ({})", lang.code());
        }
    }
}
