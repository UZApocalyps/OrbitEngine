//! The French translation of the `test_minimal.ron` fixture.

use orbit_core::{Story, StringId};
use orbit_i18n::{Localizer, Translation, coverage};

const STORY: &str = include_str!("../../../examples/stories/test_minimal.ron");
const FR: &str = include_str!("../../../examples/stories/i18n/test_minimal.fr.ron");

fn id(n: u32) -> StringId {
    format!("01JBX0T{n:019}").parse().unwrap()
}

// Fixture: depends on exact string ids and the one deliberately untranslated string.
#[test]
fn french_translation_loads_and_falls_back() {
    let story = Story::from_ron(STORY).unwrap();
    let fr = Translation::from_ron(FR).expect("test_minimal.fr.ron must parse");
    assert_eq!(fr.lang, "fr");

    // Only string 11 is left in English; nothing is stale.
    let report = coverage(&story, &fr);
    assert!(report.orphans.is_empty(), "{report}");
    assert_eq!(report.missing, vec![id(11)]);
    assert_eq!(report.total, 20);

    let mut loc = Localizer::from_story(&story);
    loc.add_translation(fr).unwrap();
    assert_eq!(loc.text(id(4)), "Another night, another contract.");
    assert!(loc.set_language("fr-FR"));
    assert_eq!(loc.language(), "fr");
    assert_eq!(loc.text(id(4)), "Encore une nuit, encore un contrat.");
    // Untranslated strings use the source text.
    assert_eq!(loc.text(id(11)), "The end.");
    // Every string of the story resolves to something non-empty.
    for string in story.strings.entries.keys() {
        assert!(!loc.text(*string).is_empty());
    }
}
