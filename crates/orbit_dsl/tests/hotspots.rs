//! Hotspot maps (point & click navigation) in the text form.

use orbit_core::{
    AssetKind, Color, Expr, Hotspot, HotspotMap, HotspotShape, HotspotStyle, Node, NodeId,
    NodeKind, Story,
};
use orbit_dsl::{Scope, parse, print_scene};

fn load() -> Story {
    let path = format!("{}/../../examples/stories/test_minimal.ron", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Story::from_ron_unchecked(&src).unwrap()
}

fn with_map(story: &mut Story) -> NodeId {
    let image = story
        .assets
        .assets
        .iter()
        .find(|a| a.kind == AssetKind::Image)
        .map(|a| a.id)
        .unwrap();
    let label = NodeId::new();
    let door = story.strings.add("The door");
    let window = story.strings.add("A \"window\"");
    let tree = story.strings.add("");
    let style = HotspotStyle {
        color: Color { r: 10, g: 200, b: 255 },
        glow_width: 7.5,
        hand_cursor: false,
        ..Default::default()
    };
    let spot = |text_id, shape| Hotspot {
        name: String::new(),
        text_id,
        shape,
        target: label,
        condition: None,
        enabled: None,
        style: None,
        hover_image: None,
        hover_sound: None,
    };
    let map = HotspotMap {
        background: Some(image),
        hotspots: vec![
            Hotspot {
                name: "door".into(),
                condition: Some(Expr::Var("has_key".into())),
                enabled: Some(Expr::Lit(orbit_core::Value::Bool(true))),
                hover_image: Some(image),
                style: Some(HotspotStyle { pulse_speed: 0.0, ..Default::default() }),
                ..spot(door, HotspotShape::Rect { x: 0.1, y: 0.2, w: 0.25, h: 0.5 })
            },
            spot(window, HotspotShape::Ellipse { cx: 0.7, cy: 0.3, rx: 0.1, ry: 0.05 }),
            spot(
                tree,
                HotspotShape::Polygon { points: vec![(0.5, 0.9), (0.6, 0.5), (0.7, 0.9)] },
            ),
        ],
        style,
    };
    let nodes = &mut story.scenes[0].nodes;
    nodes.insert(0, Node { id: NodeId::new(), kind: NodeKind::Hotspots(map) });
    nodes.insert(1, Node { id: label, kind: NodeKind::Label { name: "outside".into() } });
    label
}

#[test]
fn hotspot_maps_are_printed_and_parsed_back_exactly() {
    let mut story = load();
    with_map(&mut story);
    let scene = story.scenes[0].id;
    let text = print_scene(&story, scene).unwrap();
    assert!(text.contains("hotspots bg "), "{text}");
    assert!(text.contains("color \"#0ac8ff\" glow 7.5 no_hand"), "{text}");
    assert!(text.contains("spot rect (0.1, 0.2, 0.25, 0.5) \"The door\" -> outside name door"), "{text}");
    assert!(text.contains("style pulse 0.0 enabled true if has_key"), "{text}");
    assert!(text.contains("spot poly (0.5, 0.9) (0.6, 0.5) (0.7, 0.9) \"\" -> outside"), "{text}");
    let parsed = parse(&text, &story, Scope::Scene(scene)).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(parsed, story);
}

#[test]
fn a_map_can_be_written_by_hand() {
    let story = load();
    let scene = story.scenes[0].id;
    let text = print_scene(&story, scene).unwrap();
    let (head, rest) = text.split_once('\n').unwrap();
    let hand = format!(
        "{head}\n    hotspots idle 0.0\n        spot ellipse (0.5, 0.5, 0.1, 0.1) \"Well\" -> here if visits > 1\n    label here\n{rest}"
    );
    let parsed = parse(&hand, &story, Scope::Scene(scene)).unwrap_or_else(|e| panic!("{e:?}"));
    let NodeKind::Hotspots(map) = &parsed.scenes[0].nodes[0].kind else {
        panic!("the map comes first");
    };
    assert_eq!(map.style.idle_hint, 0.0);
    assert_eq!(map.hotspots.len(), 1);
    assert_eq!(parsed.text(map.hotspots[0].text_id), Some("Well"));
    assert!(map.hotspots[0].condition.is_some());
    assert_eq!(map.hotspots[0].target, parsed.scenes[0].nodes[1].id);

    let bad = format!("{head}\n    spot rect (0, 0, 1, 1) \"x\" -> here\n    label here\n{rest}");
    assert!(parse(&bad, &story, Scope::Scene(scene)).is_err());
    let two_points = format!("{head}\n    hotspots\n        spot poly (0, 0) (1, 1) \"x\" -> here\n    label here\n{rest}");
    assert!(parse(&two_points, &story, Scope::Scene(scene)).is_err());
}
