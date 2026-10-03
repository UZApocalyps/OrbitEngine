//! Background layers and particles in the text form (M5A-07 / M5A-08).

use orbit_core::{AssetKind, Emitter, Node, NodeId, NodeKind, ParticlePreset, Story};
use orbit_dsl::{Scope, parse, print_scene};

fn load() -> Story {
    let path = format!("{}/../../examples/stories/test_minimal.ron", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Story::from_ron_unchecked(&src).unwrap()
}

#[test]
fn layers_and_emitters_are_printed_and_parsed_back_exactly() {
    let mut story = load();
    let image = story
        .assets
        .assets
        .iter()
        .find(|a| a.kind == AssetKind::Image)
        .map(|a| a.id)
        .unwrap();
    let mut custom = Emitter::preset(ParticlePreset::Custom);
    custom.name = "my \"fx\"".into();
    custom.sprite = Some(image);
    custom.front = false;
    custom.prewarm = false;
    custom.seed = 99;
    let nodes = &mut story.scenes[0].nodes;
    for (i, kind) in [
        NodeKind::BackgroundLayers {
            layers: vec![
                orbit_core::BgLayer::new(image, 0.0),
                orbit_core::BgLayer { asset: image, depth: -0.25, repeat_x: true },
            ],
            transition: Some(orbit_core::Transition::dissolve(0.5)),
        },
        NodeKind::Particles(Emitter::preset(ParticlePreset::Rain)),
        NodeKind::Particles(Emitter::preset(ParticlePreset::Sparks)),
        NodeKind::Particles(custom),
        NodeKind::StopParticles { name: "rain".into() },
        NodeKind::StopParticles { name: String::new() },
    ]
    .into_iter()
    .enumerate()
    {
        nodes.insert(i, Node { id: NodeId::new(), kind });
    }
    let scene = story.scenes[0].id;
    let text = print_scene(&story, scene).unwrap();
    assert!(text.contains("bg_layers with dissolve(0.5)"), "{text}");
    assert!(text.contains("layer ") && text.contains("depth -0.25 repeat_x"));
    assert!(text.contains("stop particles \"rain\"") && text.contains("stop particles #@"));
    let parsed = parse(&text, &story, Scope::Scene(scene)).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(parsed, story);
}

#[test]
fn a_preset_line_can_be_written_by_hand_and_overridden() {
    let story = load();
    let scene = story.scenes[0].id;
    let text = print_scene(&story, scene).unwrap();
    let (head, rest) = text.split_once('\n').unwrap();
    let hand = format!("{head}\n    particles \"rain\" rain rate 100.0 color \"#ffffff\" cold\n{rest}");
    let parsed = parse(&hand, &story, Scope::Scene(scene)).unwrap_or_else(|e| panic!("{e:?}"));
    let NodeKind::Particles(e) = &parsed.scenes[0].nodes[0].kind else {
        panic!("a particles node comes first");
    };
    let base = Emitter::preset(ParticlePreset::Rain);
    assert_eq!(e.rate, 100.0);
    assert!(!e.prewarm);
    assert_eq!(e.color, e.color_end, "a lone color is both ends");
    assert_eq!(e.lifetime, base.lifetime, "the rest comes from the preset");
}

#[test]
fn mistakes_are_reported_on_their_line() {
    let story = load();
    let scene = story.scenes[0].id;
    let text = print_scene(&story, scene).unwrap();
    let (head, rest) = text.split_once('\n').unwrap();
    for bad in [
        "particles \"x\" hail",
        "particles \"x\" rain bogus 3.0",
        "layer \"a.png\" depth 1.0",
    ] {
        let hand = format!("{head}\n    {bad}\n{rest}");
        let err = parse(&hand, &story, Scope::Scene(scene)).unwrap_err();
        assert!(!err.is_empty(), "{bad}");
    }
}
