//! Frozen stories of every older schema version (`examples/fixtures/compat/vN/`)
//! must keep loading: migrate to the current version, validate without errors,
//! and round-trip. Never delete a fixture (docs/IMPLEMENTATION_PLAN.md §1.3).

use orbit_core::{CURRENT_SCHEMA_VERSION, Story, validate};

macro_rules! frozen {
    ($($v:literal),*) => {
        [$(($v, include_str!(concat!("../../../examples/fixtures/compat/v", $v, "/story.ron")))),*]
    };
}

const FROZEN: [(u32, &str); 7] = frozen!(1, 2, 3, 4, 5, 6, 7);

#[test]
fn every_frozen_story_migrates_validates_and_roundtrips() {
    for (version, source) in FROZEN {
        let story = Story::from_ron_unchecked(source)
            .unwrap_or_else(|e| panic!("v{version} fixture must load: {e}"));
        assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION, "v{version}");
        let report = validate(&story);
        assert!(report.errors.is_empty(), "v{version}: {:#?}", report.errors);
        let again = Story::from_ron(&story.to_ron().unwrap())
            .unwrap_or_else(|e| panic!("v{version} must round-trip: {e}"));
        assert_eq!(story, again, "v{version}");
    }
}
