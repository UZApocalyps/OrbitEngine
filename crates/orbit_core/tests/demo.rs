//! Global properties of the example story shipped in `examples/stories/demo.ron`.
//! Structure-dependent tests live in `fixture.rs` on the test fixtures.

use orbit_core::{CURRENT_SCHEMA_VERSION, Story, validate};

const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

// Demo: global property only (loads, validates with no error and no warning).
#[test]
fn demo_still_validates() {
    let story = Story::from_ron(DEMO).expect("demo.ron must be valid");
    assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION);
    let report = validate(&story);
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert!(report.warnings.is_empty(), "{:#?}", report.warnings);
}

// Demo: global property (RON round trip is lossless).
#[test]
fn demo_story_roundtrips_through_ron() {
    let story = Story::from_ron(DEMO).unwrap();
    let again = Story::from_ron(&story.to_ron().unwrap()).unwrap();
    assert_eq!(story, again);
}
