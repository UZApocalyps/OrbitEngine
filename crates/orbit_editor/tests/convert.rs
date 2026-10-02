//! Opening and converting legacy `.ron` stories to `.orbit` (TL-02).

use std::fs;
use std::path::PathBuf;

use orbit_core::Story;
use orbit_editor::i18n::translator;
use orbit_editor::storage::{FsStorage, ProjectLocation};
use orbit_editor_api::Lang;
use orbit_project::{ProjectManifest, save_manifest};

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("orbit_conv_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_legacy_project_converts_to_orbit_and_still_resolves() {
    let root = dir("manifest");
    let t = translator(Lang::En);
    let mut manifest = ProjectManifest::new("Old");
    manifest.story = "story.ron".into();
    save_manifest(&FsStorage::new(&root), &manifest).unwrap();
    fs::write(root.join("story.ron"), Story::new("Old").to_ron().unwrap()).unwrap();
    fs::write(root.join("story.layout"), "()").unwrap();

    let location = ProjectLocation::resolve(&root, &t).unwrap();
    assert!(location.is_legacy_story());
    assert_eq!(location.convert_to_orbit().unwrap(), root);
    assert!(!root.join("story.ron").exists() && root.join("story.orbit").is_file());
    assert!(root.join("story.layout").is_file());
    let again = ProjectLocation::resolve(&root, &t).unwrap();
    assert_eq!(again.story_file, "story.orbit");
    assert!(!again.is_legacy_story());
    assert!(again.convert_to_orbit().is_err());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_bare_ron_file_converts_and_an_existing_target_is_refused() {
    let root = dir("bare");
    let t = translator(Lang::En);
    let file = root.join("demo.ron");
    fs::write(&file, Story::new("d").to_ron().unwrap()).unwrap();
    let location = ProjectLocation::resolve(&file, &t).unwrap();
    fs::write(root.join("demo.orbit"), "keep").unwrap();
    assert!(location.convert_to_orbit().is_err());
    assert!(file.is_file());
    fs::remove_file(root.join("demo.orbit")).unwrap();
    assert_eq!(
        location.convert_to_orbit().unwrap(),
        root.join("demo.orbit")
    );
    assert!(ProjectLocation::resolve(&root.join("demo.orbit"), &t).is_ok());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_folder_with_only_story_ron_resolves() {
    let root = dir("folder");
    let t = translator(Lang::En);
    fs::write(root.join("story.ron"), Story::new("x").to_ron().unwrap()).unwrap();
    assert_eq!(
        ProjectLocation::resolve(&root, &t).unwrap().story_file,
        "story.ron"
    );
    let _ = fs::remove_dir_all(&root);
}
