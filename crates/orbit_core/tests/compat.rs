//! Frozen stories of every older schema version (`examples/fixtures/compat/vN/`)
//! must keep loading: migrate to the current version, validate without errors,
//! and round-trip. Never delete a fixture (docs/IMPLEMENTATION_PLAN.md §1.3).

use orbit_core::{CURRENT_SCHEMA_VERSION, Story, validate};

macro_rules! frozen {
    ($($v:literal),*) => {
        [$(($v, include_str!(concat!("../../../examples/fixtures/compat/v", $v, "/story.ron")))),*]
    };
}

const FROZEN: [(u32, &str); 13] = frozen!(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13);

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

#[test]
fn a_v13_unit_rigged_expression_becomes_a_still_and_a_rigged_motion_round_trips() {
    use orbit_core::{AssetKind, LoopMode, Motion, RiggedMotion};
    let mut story = Story::from_ron_unchecked(include_str!(
        "../../../examples/fixtures/compat/v13/story.ron"
    ))
    .unwrap();
    let waving = story.characters[0]
        .expressions
        .iter()
        .find(|e| e.name == "waving")
        .expect("the fixture keeps the expression");
    assert_eq!(waving.motion, None, "the reserved Rigged had no effect: a still");
    assert_eq!(waving.assets(), vec![waving.image]);

    let rig = orbit_core::AssetId::new();
    story.assets.assets.push(orbit_core::AssetEntry {
        id: rig,
        kind: AssetKind::Rig,
        path: "assets/rigs/hero.rig.ron".into(),
    });
    let e = story.characters[0]
        .expressions
        .iter_mut()
        .find(|e| e.name == "waving")
        .unwrap();
    e.motion = Some(Motion::Rigged(RiggedMotion {
        rig,
        skin: Some("outfit".into()),
        clip: "wave".into(),
        loop_mode: LoopMode::PingPong,
    }));
    assert_eq!(e.assets(), vec![e.image, rig]);
    let text = story.to_ron().unwrap();
    let again = Story::from_ron(&text).unwrap();
    assert_eq!(story, again);
    assert!(validate(&story).errors.is_empty());
    // A rigged expression pointing at an image asset is an error.
    let image = story.characters[0].expressions[0].image;
    if let Some(Motion::Rigged(r)) = &mut story.characters[0].expressions[1].motion {
        r.rig = image;
    }
    assert!(
        validate(&story)
            .errors
            .iter()
            .any(|e| matches!(e, orbit_core::ValidationError::AssetKindMismatch { .. }))
    );
}

#[test]
fn unit_rigged_is_dropped_whatever_the_spacing_and_older_stories_still_load() {
    // implicit_some spelling, tight spacing, in an older document.
    let v8 = r#"(
    schema_version: 8,
    title: "Old",
    scenes: [],
    characters: [
        (id: "01JBX0C0000000000000000001", name_id: "01JBX0T0000000000000000001",
         color: (r: 1, g: 2, b: 3), expressions: [
            (name: "a", image: "01JBX0A0000000000000000001", motion:Some( Rigged ) ),
            (name: "b", image: "01JBX0A0000000000000000001", motion: Rigged,),
            (name: "c", image: "01JBX0A0000000000000000001"),
         ]),
    ],
)"#;
    let story = Story::from_ron_unchecked(v8).unwrap();
    assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(story.characters[0].expressions.iter().all(|e| e.motion.is_none()));
    assert_eq!(story.characters[0].expressions.len(), 3);
}
