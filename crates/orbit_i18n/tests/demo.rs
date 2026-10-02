//! The French demo translation against `demo.ron`.

use orbit_core::{Story, StringId};
use orbit_i18n::{Localizer, Translation, coverage};

const DEMO: &str = include_str!("../../../examples/stories/demo.ron");
const FR: &str = include_str!("../../../examples/stories/i18n/demo.fr.ron");

fn id(n: u32) -> StringId {
    format!("01JBX0T{n:019}").parse().unwrap()
}

#[test]
fn french_demo_translation_loads_and_falls_back() {
    let story = Story::from_ron(DEMO).unwrap();
    let fr = Translation::from_ron(FR).expect("demo.fr.ron must parse");
    assert_eq!(fr.lang, "fr");

    // Only the two last strings of the story are left in English; nothing
    // is stale.
    let report = coverage(&story, &fr);
    assert!(report.orphans.is_empty(), "{report}");
    assert_eq!(report.missing, vec![id(86), id(87)]);
    assert_eq!(report.total, 87);

    let mut loc = Localizer::from_story(&story);
    loc.add_translation(fr).unwrap();
    assert_eq!(
        loc.text(id(4)),
        "Neo-Kyoto, 2087. Rain hammers the megatower blocks while neon bleeds across every wet window."
    );
    assert!(loc.set_language("fr-FR"));
    assert_eq!(loc.language(), "fr");
    assert_eq!(
        loc.text(id(4)),
        "Néo-Kyoto, 2087. La pluie martèle les mégatours et le néon coule sur chaque vitre mouillée."
    );
    // Untranslated strings use the source text.
    assert_eq!(loc.text(id(86)), "The music fades away.");
    // Every string of the story resolves to something non-empty.
    for string in story.strings.entries.keys() {
        assert!(!loc.text(*string).is_empty());
    }
}
