//! The self-contained test fixtures (`examples/stories/test_*.ron`).

use orbit_core::{
    AssetKind, CURRENT_SCHEMA_VERSION, CoreError, Story, ValidationError, validate,
};

const MINIMAL: &str = include_str!("../../../examples/stories/test_minimal.ron");

#[test]
fn fixtures_load_and_validate_without_warnings() {
    for (name, src) in [
        ("test_minimal", include_str!("../../../examples/stories/test_minimal.ron")),
        ("test_features", include_str!("../../../examples/stories/test_features.ron")),
    ] {
        let story = Story::from_ron(src).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION, "{name}");
        let report = validate(&story);
        assert!(report.errors.is_empty(), "{name}: {:#?}", report.errors);
        assert!(report.warnings.is_empty(), "{name}: {:#?}", report.warnings);
        assert!(
            story.assets.assets.iter().all(|a| a.path.starts_with("fixtures/assets/")),
            "{name} must only use fixture assets"
        );
    }
}

#[test]
fn features_fixture_has_animated_expressions() {
    let story = Story::from_ron(include_str!("../../../examples/stories/test_features.ron")).unwrap();
    let animated = story
        .characters
        .iter()
        .flat_map(|c| &c.expressions)
        .filter(|e| e.animation().is_some())
        .count();
    assert_eq!(animated, 2);
}

// Fixture: depends on the exact asset list of test_minimal.ron.
#[test]
fn minimal_manifest_points_to_fixture_assets() {
    let story = Story::from_ron(MINIMAL).unwrap();
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
            ("fixtures/assets/alice_happy.png", AssetKind::Image),
            ("fixtures/assets/bg_room.png", AssetKind::Image),
            ("fixtures/assets/bob_sad.png", AssetKind::Image),
            ("fixtures/assets/music_loop.wav", AssetKind::Music),
            ("fixtures/assets/sfx_knock.wav", AssetKind::Sound),
        ]
    );
}

// Fixture: depends on a specific Jump node id of test_minimal.ron.
#[test]
fn broken_target_in_minimal_is_detected() {
    // Retarget the first Jump to an id that does not exist.
    let broken = MINIMAL.replacen(
        "Jump(target: \"01JBX0N0000000000000000032",
        "Jump(target: \"01JBX0N0000000000000000099",
        1,
    );
    assert_ne!(broken, MINIMAL, "test fixture must actually change");
    match Story::from_ron(&broken) {
        Err(CoreError::Invalid(errs)) => assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::UnknownTarget { .. }))
        ),
        other => panic!("expected validation error, got {other:?}"),
    }
}
