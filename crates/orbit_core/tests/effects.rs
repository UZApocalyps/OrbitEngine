//! Effect and Timeline nodes: the frozen v9 story, validation, and RON.

use orbit_core::{
    AnimLoop, AnimProp, AnimTarget, CharacterId, Effect, Key, NodeKind, Story, StoryTrack, Track,
    TrackData, ValidationError, ValidationWarning, Vec2, validate,
};

const V9: &str = include_str!("../../../examples/fixtures/compat/v9/story.ron");

fn story() -> Story {
    Story::from_ron(V9).expect("the frozen v9 story loads and validates")
}

fn first_timeline(story: &mut Story) -> &mut NodeKind {
    &mut story
        .scenes
        .iter_mut()
        .flat_map(|s| &mut s.nodes)
        .find(|n| matches!(n.kind, NodeKind::Timeline { .. }))
        .unwrap()
        .kind
}

#[test]
fn the_frozen_v9_story_has_no_error_and_no_warning() {
    let s = story();
    let report = validate(&s);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let nodes: Vec<&NodeKind> = s.scenes.iter().flat_map(|s| s.nodes.iter().map(|n| &n.kind)).collect();
    assert_eq!(nodes.iter().filter(|k| matches!(k, NodeKind::Effect(_))).count(), 5);
    assert_eq!(nodes.iter().filter(|k| matches!(k, NodeKind::Timeline { .. })).count(), 2);
}

#[test]
fn effects_and_timelines_round_trip_through_ron() {
    let s = story();
    let again = Story::from_ron(&s.to_ron().unwrap()).unwrap();
    assert_eq!(again, s);
    // A key without easing is written without it (the file stays terse).
    assert!(!s.to_ron().unwrap().contains("ease: Preset(Linear)"));
}

#[test]
fn an_effect_compiles_to_the_same_tracks_as_its_timeline() {
    let e = Effect::Move {
        target: AnimTarget::Background,
        dx: 0.1,
        dy: 0.2,
        duration: 2.0,
        ease: None,
        wait: true,
    };
    let NodeKind::Timeline { duration, looping, wait, tracks } = e.to_timeline() else {
        panic!("a timeline");
    };
    assert_eq!((duration, looping, wait), (2.0, AnimLoop::Once, true));
    assert_eq!(tracks, e.tracks());
    assert_eq!(e.to_clip().tracks.len(), 1);
}

#[test]
fn malformed_animations_are_refused() {
    let mut s = story();
    let ghost = CharacterId::new();
    let bad = |s: &Story| validate(s).errors;

    // Unknown character.
    if let NodeKind::Timeline { tracks, .. } = first_timeline(&mut s) {
        tracks.push(StoryTrack {
            target: AnimTarget::Character(ghost),
            prop: AnimProp::Opacity,
            data: TrackData::Scalar(Track::new(vec![Key::new(0.0, 1.0)])),
        });
    }
    assert!(bad(&s).iter().any(|e| matches!(e, ValidationError::UnknownCharacter { character, .. } if *character == ghost)));

    // A property that does not fit its target, a wrong value type, unsorted keys.
    let mut s = story();
    if let NodeKind::Timeline { tracks, .. } = first_timeline(&mut s) {
        tracks.clear();
        tracks.push(StoryTrack {
            target: AnimTarget::Screen,
            prop: AnimProp::Offset,
            data: TrackData::Vec2(Track::new(vec![Key::new(0.0, Vec2::new(0.0, 0.0))])),
        });
        tracks.push(StoryTrack {
            target: AnimTarget::Background,
            prop: AnimProp::Opacity,
            data: TrackData::Vec2(Track::new(vec![Key::new(0.0, Vec2::new(0.0, 0.0))])),
        });
        tracks.push(StoryTrack {
            target: AnimTarget::Background,
            prop: AnimProp::Offset,
            data: TrackData::Vec2(Track::new(vec![
                Key::new(1.0, Vec2::new(0.0, 0.0)),
                Key::new(0.5, Vec2::new(0.0, 0.0)),
            ])),
        });
    }
    let errors = bad(&s);
    let messages: Vec<String> = errors
        .iter()
        .filter_map(|e| match e {
            ValidationError::InvalidTimeline { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(messages.len(), 3, "{messages:?}");

    // Waiting for an endless timeline.
    let mut s = story();
    if let NodeKind::Timeline { looping, wait, .. } = first_timeline(&mut s) {
        *looping = AnimLoop::Forever;
        *wait = true;
    }
    assert!(bad(&s).iter().any(|e| matches!(e, ValidationError::BlockingInfiniteTimeline(_))));

    // A key after the end only warns; a non-finite value or amount is an error.
    let mut s = story();
    if let NodeKind::Timeline { duration, .. } = first_timeline(&mut s) {
        *duration = 1.0;
    }
    let report = validate(&s);
    assert!(report.errors.is_empty());
    assert!(report.warnings.iter().any(|w| matches!(w, ValidationWarning::TimelineKeyAfterEnd(_))));
    let mut s = story();
    for n in s.scenes.iter_mut().flat_map(|s| &mut s.nodes) {
        if let NodeKind::Effect(Effect::Zoom { factor, .. }) = &mut n.kind {
            *factor = 0.0;
        }
        if let NodeKind::Effect(Effect::Tint { strength, .. }) = &mut n.kind {
            *strength = 2.0;
        }
    }
    let errors = bad(&s);
    assert_eq!(
        errors.iter().filter(|e| matches!(e, ValidationError::InvalidTimeline { .. })).count(),
        2
    );
}
