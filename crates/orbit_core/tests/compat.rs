//! Frozen stories of every older schema version (`examples/fixtures/compat/vN/`)
//! must keep loading: migrate to the current version, validate without errors,
//! and round-trip. Never delete a fixture (docs/IMPLEMENTATION_PLAN.md §1.3).

use orbit_core::{CURRENT_SCHEMA_VERSION, Story, validate};

macro_rules! frozen {
    ($($v:literal),*) => {
        [$(($v, include_str!(concat!("../../../examples/fixtures/compat/v", $v, "/story.ron")))),*]
    };
}

const FROZEN: [(u32, &str); 11] = frozen!(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11);

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

#[test]
fn a_v8_story_gains_empty_curves_and_linear_transitions() {
    let story = Story::from_ron_unchecked(include_str!(
        "../../../examples/fixtures/compat/v8/story.ron"
    ))
    .unwrap();
    assert!(story.curves.is_empty());
    let mut seen = 0;
    for node in story.scenes.iter().flat_map(|s| &s.nodes) {
        use orbit_core::NodeKind::*;
        if let Background { transition: Some(t), .. }
        | Show { transition: Some(t), .. }
        | Hide { transition: Some(t), .. } = &node.kind
        {
            assert_eq!(t.ease, None);
            seen += 1;
        }
    }
    assert!(seen > 0, "the fixture must exercise transitions");
}

#[test]
fn named_curves_are_validated() {
    use orbit_anim::Curve;
    use orbit_core::{Transition, ValidationError, ValidationWarning};
    let mut story = Story::from_ron_unchecked(include_str!(
        "../../../examples/fixtures/compat/v8/story.ron"
    ))
    .unwrap();
    story.curves.insert("snappy".into(), Curve::bezier(0.2, 0.0, 0.0, 1.0));
    story.curves.insert("alias".into(), Curve::Named("snappy".into()));
    story.curves.insert("bad".into(), Curve::bezier(f32::NAN, 0.0, 0.0, 1.0));
    let t = Transition::dissolve(0.5).with_ease(Curve::Named("missing".into()));
    for node in story.scenes.iter_mut().flat_map(|s| &mut s.nodes) {
        if let orbit_core::NodeKind::Background { transition, .. } = &mut node.kind {
            *transition = Some(t.clone());
        }
    }
    let report = validate(&story);
    assert!(report.errors.iter().any(|e| matches!(e, ValidationError::NamedCurveChain(n) if n == "alias")));
    assert!(report.errors.iter().any(|e| matches!(e, ValidationError::InvalidCurve { .. })));
    assert!(report.warnings.iter().any(|w| matches!(w, ValidationWarning::UnknownCurve { name, .. } if name == "missing")));
    // Curves survive a RON round trip.
    story.curves.remove("alias");
    story.curves.remove("bad");
    let again = Story::from_ron_unchecked(&story.to_ron().unwrap()).unwrap();
    assert_eq!(again.curves, story.curves);
}
