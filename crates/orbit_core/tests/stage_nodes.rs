//! Schema v13: background layers and particle emitters (M5A-07 / M5A-08).

use orbit_core::{
    AssetId, AssetKind, BgLayer, Emitter, Node, NodeId, NodeKind, ParticlePreset,
    Scene, SceneId, Story, ValidationError, ValidationWarning, validate,
};

const V12: &str = include_str!("../../../examples/fixtures/compat/v12/story.ron");

#[test]
fn a_v12_story_gains_nothing_but_the_version_number() {
    // Schema v13 is purely additive: no new node appears in a migrated v12
    // document, and the nodes it had are still there.
    let story = Story::from_ron_unchecked(V12).unwrap();
    assert_eq!(story.schema_version, 13);
    let text = story.to_ron().unwrap();
    assert!(!text.contains("BackgroundLayers") && !text.contains("Particles"));
    assert!(text.contains("OpenScreen") && text.contains("CloseScreen"));
    assert!(validate(&story).errors.is_empty());
}

fn base() -> (Story, AssetId, AssetId) {
    let mut story = Story::from_ron_unchecked(V12).unwrap();
    let image = story
        .assets
        .assets
        .iter()
        .find(|e| e.kind == AssetKind::Image)
        .map(|e| e.id)
        .expect("the fixture has an image");
    let sound = AssetId::new();
    story.assets.assets.push(orbit_core::AssetEntry {
        id: sound,
        kind: AssetKind::Sound,
        path: "assets/x.ogg".into(),
    });
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "stage".into(),
        nodes: Vec::new(),
    });
    (story, image, sound)
}

fn push(story: &mut Story, kind: NodeKind) -> NodeId {
    let id = NodeId::new();
    story.scenes.last_mut().unwrap().nodes.push(Node { id, kind });
    id
}

#[test]
fn stage_nodes_roundtrip_through_ron() {
    let (mut story, image, _) = base();
    push(
        &mut story,
        NodeKind::BackgroundLayers {
            layers: vec![BgLayer::new(image, 0.0), BgLayer { asset: image, depth: 0.5, repeat_x: true }],
            transition: None,
        },
    );
    push(&mut story, NodeKind::Particles(Emitter::preset(ParticlePreset::Rain)));
    push(&mut story, NodeKind::StopParticles { name: "rain".into() });
    assert!(validate(&story).errors.is_empty(), "{:?}", validate(&story).errors);
    let again = Story::from_ron(&story.to_ron().unwrap()).unwrap();
    assert_eq!(story, again);
}

#[test]
fn bad_stage_nodes_are_reported() {
    let (mut story, image, sound) = base();
    let empty = push(&mut story, NodeKind::BackgroundLayers { layers: vec![], transition: None });
    let wrong = push(
        &mut story,
        NodeKind::BackgroundLayers { layers: vec![BgLayer::new(sound, 1.0)], transition: None },
    );
    let depth = push(
        &mut story,
        NodeKind::BackgroundLayers { layers: vec![BgLayer::new(image, f32::NAN)], transition: None },
    );
    let mut heavy = Emitter::preset(ParticlePreset::Snow);
    heavy.rate = 4000.0;
    let heavy = push(&mut story, NodeKind::Particles(heavy));
    let stop = push(&mut story, NodeKind::StopParticles { name: "nobody".into() });
    let report = validate(&story);
    let stage_errors: Vec<NodeId> = report
        .errors
        .iter()
        .filter_map(|e| match e {
            ValidationError::InvalidStageNode { node, .. } => Some(*node),
            _ => None,
        })
        .collect();
    for n in [empty, depth, heavy] {
        assert!(stage_errors.contains(&n), "{n}");
    }
    assert!(
        report.errors.iter().any(|e| matches!(e, ValidationError::AssetKindMismatch { node: Some(n), .. } if *n == wrong))
    );
    assert!(report.warnings.iter().any(
        |w| matches!(w, ValidationWarning::UnknownEmitter { node, .. } if *node == stop)
    ));
}
