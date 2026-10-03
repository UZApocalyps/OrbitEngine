//! Effects, timelines and transition easing in the text form.

use orbit_core::{AnimTarget, Curve, Ease, Effect, NodeKind, Story, TransitionKind};
use orbit_dsl::{Scope, parse, print_scene};

fn load(name: &str) -> Story {
    let path = format!(
        "{}/../../examples/stories/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Story::from_ron_unchecked(&src).unwrap()
}

/// The scene text without ids, and the first line split off.
fn bare(story: &Story) -> (String, String) {
    let text = print_scene(story, story.scenes[0].id).unwrap();
    let text: String = text
        .lines()
        .map(|l| match l.rsplit_once(" #@") {
            Some((body, _)) if !l.trim_start().starts_with("scene") => format!("{body}\n"),
            _ => format!("{l}\n"),
        })
        .collect();
    let (head, rest) = text.split_once('\n').unwrap();
    (head.to_owned(), rest.to_owned())
}

#[test]
fn effects_timelines_and_transition_easing_are_scripted_by_hand() {
    let mut story = load("test_minimal.ron");
    story
        .curves
        .insert("snappy".into(), Curve::bezier(0.2, 0.0, 0.1, 1.4));
    let scene = story.scenes[0].id;
    let (head, rest) = bare(&story);
    let extra = r##"    shake camera 0.02 over 0.5 ease OutQuad
    move Alice by (0.25, -0.1) over 1.5 ease bezier(0.2, 0.0, 0.1, 1.4) wait
    zoom 1.5 over 1.0 ease "snappy"
    flash "#ffffff" over 0.3
    tint screen "#ff0000" 0.4 over 2.0
    timeline over 3.0 loop 2
        track background offset 0.0 (0.0, 0.0), 1.5 (0.3, 0.0) ease InOutQuad, 3.0 (0.0, 0.0)
        track Alice opacity 0.0 1.0, 3.0 0.5
        track screen tint 0.0 rgba(0.0, 0.0, 0.0, 0.0), 3.0 rgba(1.0, 0.0, 0.0, 0.5)
"##;
    let text = format!("{head}\n{extra}{rest}");
    let parsed = parse(&text, &story, Scope::Scene(scene)).unwrap_or_else(|e| panic!("{e:?}"));
    let nodes = &parsed.scene(scene).unwrap().nodes;
    assert!(matches!(
        &nodes[0].kind,
        NodeKind::Effect(Effect::Shake {
            target: AnimTarget::Camera,
            ease: Some(Curve::Preset(Ease::OutQuad)),
            ..
        })
    ));
    assert!(matches!(
        &nodes[1].kind,
        NodeKind::Effect(Effect::Move { wait: true, .. })
    ));
    assert!(matches!(
        &nodes[2].kind,
        NodeKind::Effect(Effect::Zoom { ease: Some(Curve::Named(n)), .. }) if n == "snappy"
    ));
    assert!(matches!(&nodes[5].kind, NodeKind::Timeline { tracks, .. } if tracks.len() == 3));
    // Canonical text parses back to the same story.
    let printed = print_scene(&parsed, scene).unwrap();
    assert_eq!(parse(&printed, &parsed, Scope::Scene(scene)).unwrap(), parsed);
}

#[test]
fn transition_ease_is_no_longer_dropped() {
    let mut story = load("test_minimal.ron");
    let mut eased = 0;
    for n in story.scenes.iter_mut().flat_map(|s| &mut s.nodes) {
        if let NodeKind::Background { transition, .. } = &mut n.kind {
            let mut t = transition.clone().unwrap_or(orbit_core::Transition::dissolve(0.5));
            t.ease = Some(Curve::Preset(Ease::InOutSine));
            *transition = Some(t);
            eased += 1;
        }
    }
    assert!(eased > 0);
    let text = orbit_dsl::print_story(&story);
    assert!(text.contains("ease InOutSine"), "{text}");
    let back = parse(&text, &story, Scope::Story).unwrap();
    assert_eq!(back, story);
    let t = back
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .find_map(|n| match &n.kind {
            NodeKind::Background { transition: Some(t), .. } => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(t.ease, Some(Curve::Preset(Ease::InOutSine)));
    assert!(matches!(
        t.kind,
        TransitionKind::Dissolve | TransitionKind::Fade(_)
    ));
}

#[test]
fn bad_effect_lines_are_reported_with_a_position() {
    let story = load("test_minimal.ron");
    let scene = story.scenes[0].id;
    let (head, rest) = bare(&story);
    for bad in [
        "    move background over 1.0",
        "    shake camera 0.1 over 1.0 ease Nope",
        "    track camera zoom 0.0 1.0",
        "    timeline over 1.0\n        \"x\" -> nowhere",
        "    tint screen \"red\" 0.5 over 1.0",
        "    move NobodyHere by (1.0, 0.0) over 1.0",
    ] {
        let text = format!("{head}\n{bad}\n{rest}");
        let err = parse(&text, &story, Scope::Scene(scene)).expect_err(bad);
        assert!(
            err.iter().all(|d| d.line >= 1 && d.column >= 1),
            "{bad}: {err:?}"
        );
    }
}
