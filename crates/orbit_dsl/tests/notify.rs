//! In-game notifications (`notify`) in the text form.

use orbit_core::{AssetKind, Node, NodeId, NodeKind, Notify, Story};
use orbit_dsl::{Scope, parse, print_scene};

fn load() -> Story {
    let path = format!("{}/../../examples/stories/test_minimal.ron", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Story::from_ron_unchecked(&src).unwrap()
}

#[test]
fn notify_nodes_are_printed_and_parsed_back_exactly() {
    let mut story = load();
    let image = story.assets.assets.iter().find(|a| a.kind == AssetKind::Image).map(|a| a.id);
    let plain = story.strings.add("Saved [gold] coins");
    let full = story.strings.add("{b}Quest{/b} \"done\"");
    let nodes = &mut story.scenes[0].nodes;
    nodes.insert(
        0,
        Node {
            id: NodeId::new(),
            kind: NodeKind::Notify(Notify { text_id: plain, icon: None, sound: None, duration: None, style: None }),
        },
    );
    nodes.insert(
        1,
        Node {
            id: NodeId::new(),
            kind: NodeKind::Notify(Notify {
                text_id: full,
                icon: image,
                sound: None,
                duration: Some(4.5),
                style: Some("achievement".into()),
            }),
        },
    );
    let scene = story.scenes[0].id;
    let text = print_scene(&story, scene).unwrap();
    assert!(text.contains("notify \"Saved [gold] coins\""), "{text}");
    assert!(text.contains(" style achievement duration 4.5"), "{text}");
    if image.is_some() {
        assert!(text.contains("\" icon "), "{text}");
    }
    let parsed = parse(&text, &story, Scope::Scene(scene)).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(parsed, story);
}

#[test]
fn notify_can_be_written_by_hand_and_bad_options_are_refused() {
    let story = load();
    let scene = story.scenes[0].id;
    let text = print_scene(&story, scene).unwrap();
    let (head, rest) = text.split_once('\n').unwrap();
    let hand = format!("{head}\n    notify \"Hello\" duration 2 style \"my style\"\n{rest}");
    let parsed = parse(&hand, &story, Scope::Scene(scene)).unwrap_or_else(|e| panic!("{e:?}"));
    let NodeKind::Notify(n) = &parsed.scenes[0].nodes[0].kind else {
        panic!("the notify comes first");
    };
    assert_eq!(parsed.text(n.text_id), Some("Hello"));
    assert_eq!(n.duration, Some(2.0));
    assert_eq!(n.style.as_deref(), Some("my style"));

    for bad in ["notify", "notify \"x\" colour red", "notify \"x\" duration 1 duration 2", "notify \"x\" duration -1"] {
        let src = format!("{head}\n    {bad}\n{rest}");
        assert!(parse(&src, &story, Scope::Scene(scene)).is_err(), "{bad}");
    }
}
