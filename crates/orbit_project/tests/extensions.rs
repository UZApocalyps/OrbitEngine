//! `.orbit` story extension helpers (TL-02).

use orbit_project::{ProjectManifest, default_story_in, is_story_file_name, orbit_story_name};

#[test]
fn story_names_and_legacy_conversion() {
    assert!(is_story_file_name("a.orbit") && is_story_file_name("a/b.ron"));
    assert!(!is_story_file_name("a.txt") && !is_story_file_name(".ron"));
    assert_eq!(
        orbit_story_name("story.ron").as_deref(),
        Some("story.orbit")
    );
    assert_eq!(
        orbit_story_name("s/demo.ron").as_deref(),
        Some("s/demo.orbit")
    );
    assert_eq!(orbit_story_name("story.orbit"), None);
    assert_eq!(orbit_story_name("a.fr.ron").as_deref(), Some("a.fr.orbit"));
    assert_eq!(default_story_in(|f| f == "story.ron"), Some("story.ron"));
    assert_eq!(
        default_story_in(|f| f == "story.ron" || f == "story.orbit"),
        Some("story.orbit")
    );
    assert_eq!(default_story_in(|f| f == "x.orbit"), None);
    // A new project points to `.orbit`; an old manifest keeps its `.ron`.
    assert_eq!(ProjectManifest::new("x").story, "story.orbit");
    let old = ProjectManifest::from_ron("(format: 1, name: \"x\", story: \"story.ron\")").unwrap();
    assert_eq!(old.story, "story.ron");
}
