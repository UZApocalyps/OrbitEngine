//! `Hotspots` nodes: a choice drawn as zones over the background.

use std::sync::Arc;

use orbit_core::{
    AssetEntry, AssetId, AssetKind, Expr, Hotspot, HotspotMap, HotspotShape, Node, NodeId,
    NodeKind, Scene, SceneId, Story, Value,
};

use crate::{Pending, PresentationEvent, StepResult, Vm, VmState};

struct Fixture {
    story: Story,
    map: NodeId,
    door: NodeId,
    window: NodeId,
    bg: AssetId,
}

fn fixture() -> Fixture {
    let mut story = Story::new("hotspots");
    let bg = AssetId::new();
    story.assets.assets.push(AssetEntry { id: bg, kind: AssetKind::Image, path: "street.png".into() });
    let (map, door, window) = (NodeId::new(), NodeId::new(), NodeId::new());
    let spot = |story: &mut Story, text: &str, target, condition, enabled| Hotspot {
        name: text.into(),
        text_id: story.strings.add(text),
        shape: HotspotShape::Rect { x: 0.0, y: 0.0, w: 0.5, h: 0.5 },
        target,
        condition,
        enabled,
        style: None,
        hover_image: None,
        hover_sound: None,
        actions: Vec::new(),
    };
    let hotspots = vec![
        spot(&mut story, "Door", door, None, Some(Expr::Var("has_key".into()))),
        spot(&mut story, "Secret", window, Some(Expr::Var("found".into())), None),
        spot(&mut story, "Window", window, None, None),
    ];
    let said = |story: &mut Story, text: &str| NodeKind::Say {
        speaker: None,
        text_id: story.strings.add(text),
        voice: None,
    };
    let in_house = said(&mut story, "in the house");
    let at_window = said(&mut story, "at the window");
    let nodes = vec![
        Node { id: map, kind: NodeKind::Hotspots(HotspotMap { background: Some(bg), hotspots, ..Default::default() }) },
        Node { id: door, kind: NodeKind::Label { name: "door".into() } },
        Node { id: NodeId::new(), kind: in_house },
        Node { id: NodeId::new(), kind: NodeKind::End },
        Node { id: window, kind: NodeKind::Label { name: "window".into() } },
        Node { id: NodeId::new(), kind: at_window },
    ];
    story.scenes.push(Scene { id: SceneId::new(), name: "street".into(), nodes });
    Fixture { story, map, door, window, bg }
}

fn vm(story: Story, vars: &[(&str, bool)]) -> Vm {
    let mut vm = Vm::from_start(Arc::new(story)).unwrap();
    let mut state = vm.snapshot();
    for (name, v) in vars {
        state.vars.insert((*name).into(), Value::Bool(*v));
    }
    vm.restore(state).unwrap();
    vm
}

fn said(vm: &mut Vm) -> String {
    match vm.step() {
        StepResult::Say { string_id, .. } => vm.text(string_id).unwrap().to_owned(),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn a_map_waits_like_a_choice_and_shows_its_background() {
    let f = fixture();
    let mut vm = vm(f.story, &[("has_key", false), ("found", false)]);
    let StepResult::Choice { node, options } = vm.step() else { panic!("expected the map") };
    assert_eq!(node, f.map);
    // The door is disabled (no key), the secret hidden: only the window.
    assert_eq!(options.len(), 1);
    assert!(vm.take_events().contains(&PresentationEvent::Background { asset: f.bg, transition: None }));
    let view = vm.hotspot_choice().expect("waiting on the map");
    assert_eq!(view.active, vec![2]);
    assert_eq!(view.disabled, vec![0], "the door is shown dimmed");
    // Stepping again changes nothing.
    assert!(matches!(vm.step(), StepResult::Choice { .. }));
    vm.choose(0).unwrap();
    assert_eq!(said(&mut vm), "at the window");
    assert!(vm.hotspot_choice().is_none());
    let _ = (f.door, f.window);
}

#[test]
fn conditions_filter_the_zones() {
    let f = fixture();
    let mut vm = vm(f.story, &[("has_key", true), ("found", true)]);
    assert!(matches!(vm.step(), StepResult::Choice { .. }));
    let view = vm.hotspot_choice().unwrap();
    assert_eq!(view.active, vec![0, 1, 2]);
    assert!(view.disabled.is_empty());
    vm.choose(0).unwrap();
    assert_eq!(said(&mut vm), "in the house");
}

#[test]
fn a_save_taken_on_the_map_comes_back_waiting_on_it() {
    let f = fixture();
    let story = f.story.clone();
    let mut vm1 = vm(f.story, &[("has_key", true), ("found", false)]);
    assert!(matches!(vm1.step(), StepResult::Choice { .. }));
    let saved = ron::to_string(&vm1.snapshot()).unwrap();

    let mut vm2 = Vm::from_start(Arc::new(story)).unwrap();
    let state: VmState = ron::from_str(&saved).unwrap();
    assert!(matches!(state.pending, Pending::Choice { node, .. } if node == f.map));
    vm2.restore(state).unwrap();
    assert_eq!(vm2.snapshot().scene.background, Some(f.bg));
    assert_eq!(vm2.hotspot_choice().unwrap().active, vec![0, 2]);
    let StepResult::Choice { options, .. } = vm2.step() else { panic!("still waiting") };
    assert_eq!(options.len(), 2);
    vm2.choose(1).unwrap();
    assert_eq!(said(&mut vm2), "at the window");
}

#[test]
fn a_pending_index_past_the_zones_is_refused() {
    let f = fixture();
    let story = f.story.clone();
    let mut vm1 = vm(f.story, &[]);
    let _ = vm1.step();
    let mut state = vm1.snapshot();
    state.pending = Pending::Choice { node: f.map, visible: vec![7] };
    let mut vm2 = Vm::from_start(Arc::new(story)).unwrap();
    assert!(vm2.restore(state).is_err());
}
