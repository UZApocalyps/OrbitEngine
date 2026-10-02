//! A project folder with the legacy `story.ron` still opens (TL-02).

use std::fs;

use orbit_core::Story;
use orbit_mcp::HeadlessHost;

#[test]
fn a_legacy_ron_folder_opens_without_creating_a_second_story() {
    let dir = std::env::temp_dir().join(format!("orbit_mcp_legacy_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let text = Story::new("Old").to_ron().unwrap();
    fs::write(dir.join("story.ron"), &text).unwrap();
    let host = HeadlessHost::open(&dir).unwrap();
    assert_eq!(host.story_file(), "story.ron");
    assert!(!dir.join("story.orbit").exists());

    fs::write(dir.join("b.orbit"), &text).unwrap();
    let host = HeadlessHost::open(&dir.join("b.orbit")).unwrap();
    assert_eq!(host.story_file(), "b.orbit");
    let _ = fs::remove_dir_all(&dir);
}
