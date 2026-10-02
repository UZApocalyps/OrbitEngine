//! Global property of the demo translation.

use orbit_core::Story;
use orbit_i18n::{Translation, coverage};

const DEMO: &str = include_str!("../../../examples/stories/demo.ron");
const FR: &str = include_str!("../../../examples/stories/i18n/demo.fr.ron");

// Demo: global property only (the French translation parses, has no stale entry,
// and covers every string); no exact counts.
#[test]
fn french_demo_translation_is_complete() {
    let story = Story::from_ron(DEMO).unwrap();
    let fr = Translation::from_ron(FR).expect("demo.fr.ron must parse");
    assert_eq!(fr.lang, "fr");
    let report = coverage(&story, &fr);
    assert!(report.orphans.is_empty(), "{report}");
    assert!(report.missing.is_empty(), "{report}");
}
