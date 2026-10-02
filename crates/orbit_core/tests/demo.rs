//! Loads the example story shipped in `examples/stories/demo.ron`.

use orbit_core::{
    AssetKind, CURRENT_SCHEMA_VERSION, CoreError, NodeKind, Story, ValidationError, validate,
};

const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

#[test]
fn demo_story_loads_and_validates_without_warnings() {
    let story = Story::from_ron(DEMO).expect("demo.ron must be valid");
    assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(story.title, "Demo");
    assert_eq!(story.scenes.len(), 1);
    let report = validate(&story);
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert!(report.warnings.is_empty(), "{:#?}", report.warnings);

    let nodes = || story.scenes.iter().flat_map(|s| &s.nodes);
    let count = |f: fn(&NodeKind) -> bool| nodes().filter(|n| f(&n.kind)).count();
    assert!(count(|k| matches!(k, NodeKind::Say { .. })) >= 20);
    assert_eq!(count(|k| matches!(k, NodeKind::Choice { .. })), 1);
    assert!(count(|k| matches!(k, NodeKind::PlayMusic { .. })) >= 1);
    assert!(count(|k| matches!(k, NodeKind::PlaySound { .. })) >= 1);
    assert!(count(|k| matches!(k, NodeKind::Call { .. })) >= 1);
    assert_eq!(story.characters.len(), 2);
    assert!(story.characters.iter().all(|c| c.expressions.len() >= 2));
}

#[test]
fn demo_manifest_points_to_assets_demo() {
    let story = Story::from_ron(DEMO).unwrap();
    let mut files: Vec<_> = story
        .assets
        .assets
        .iter()
        .map(|a| (a.path.as_str(), a.kind))
        .collect();
    files.sort_by_key(|f| f.0);
    assert_eq!(
        files,
        [
            ("assets/demo/alice_happy.png", AssetKind::Image),
            ("assets/demo/alice_neutral.png", AssetKind::Image),
            ("assets/demo/bg_room.png", AssetKind::Image),
            ("assets/demo/bg_street.png", AssetKind::Image),
            ("assets/demo/bob_angry.png", AssetKind::Image),
            ("assets/demo/bob_neutral.png", AssetKind::Image),
            ("assets/demo/music_theme.wav", AssetKind::Music),
            ("assets/demo/sfx_door.wav", AssetKind::Sound),
        ]
    );
}

#[test]
fn demo_story_roundtrips_through_ron() {
    let story = Story::from_ron(DEMO).unwrap();
    let again = Story::from_ron(&story.to_ron().unwrap()).unwrap();
    assert_eq!(story, again);
}

#[test]
fn broken_target_in_demo_is_detected() {
    // Retarget the first Jump to an id that does not exist.
    let broken = DEMO.replacen(
        "Jump(target: \"01JBX0N00000000000000000",
        "Jump(target: \"01JBX0Z00000000000000000",
        1,
    );
    assert_ne!(broken, DEMO, "test fixture must actually change");
    match Story::from_ron(&broken) {
        Err(CoreError::Invalid(errs)) => assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::UnknownTarget { .. }))
        ),
        other => panic!("expected validation error, got {other:?}"),
    }
}
