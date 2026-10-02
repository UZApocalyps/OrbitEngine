//! Export projects open on `.orbit` and legacy `.ron` stories (TL-02).

use orbit_export::ExportProject;

#[test]
fn a_folder_opens_on_story_orbit_or_the_legacy_story_ron() {
    let dir = std::env::temp_dir().join(format!("orbit_export_ext_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("story.ron"), "x").unwrap();
    assert_eq!(ExportProject::open(&dir).unwrap().story_file, "story.ron");
    std::fs::write(dir.join("story.orbit"), "x").unwrap();
    assert_eq!(ExportProject::open(&dir).unwrap().story_file, "story.orbit");
    let bare = ExportProject::open(&dir.join("story.orbit")).unwrap();
    assert_eq!(bare.story_file, "story.orbit");
    let _ = std::fs::remove_dir_all(&dir);
}
