//! Zone actions of `Hotspots` nodes: variables, sounds, notifications and
//! flow (stay, jump, call / return) run when a zone is activated.

use std::sync::Arc;

use orbit_core::{
    AssetEntry, AssetId, AssetKind, Expr, Hotspot, HotspotAction, HotspotMap, HotspotShape, Node,
    NodeId, NodeKind, Notify, OpKind, Operand, Scene, SceneId, Story, UnOp, Value, VarDecl,
};

use crate::{PresentationEvent, StepResult, Vm};

struct Room {
    story: Story,
    map: NodeId,
    snd: AssetId,
}

/// A room: a key to pick up (stay), a door locked until then (an action,
/// then its target), a painting to look at (call, back to the map).
fn room() -> Room {
    let mut story = Story::new("room");
    let snd = AssetId::new();
    story.assets.assets.push(AssetEntry { id: snd, kind: AssetKind::Sound, path: "pick.ogg".into() });
    story.variables.push(VarDecl::game("has_key", Value::Bool(false)));
    story.variables.push(VarDecl::game("coins", Value::Int(0)));
    let (map, out, look, start) = (NodeId::new(), NodeId::new(), NodeId::new(), NodeId::new());
    let rect = HotspotShape::Rect { x: 0.0, y: 0.0, w: 0.2, h: 0.2 };
    let mut key = Hotspot::new(story.strings.add("Key"), rect.clone(), start);
    key.condition = Some(Expr::Unary(UnOp::Not, Box::new(Expr::Var("has_key".into()))));
    let picked = story.strings.add("Picked up the key ([coins] coins)");
    key.actions = vec![
        HotspotAction::SetVar { var: "has_key".into(), op: OpKind::Toggle, value: Operand::Lit(Value::Bool(true)) },
        HotspotAction::SetVar { var: "coins".into(), op: OpKind::Add, value: Operand::Lit(Value::Int(5)) },
        HotspotAction::PlaySound { asset: snd },
        HotspotAction::Notify(Notify { text_id: picked, icon: None, sound: None, duration: None, style: None }),
        HotspotAction::Stay,
        // Never runs: after the first flow action.
        HotspotAction::SetVar { var: "coins".into(), op: OpKind::Set, value: Operand::Lit(Value::Int(99)) },
    ];
    let mut door = Hotspot::new(story.strings.add("Door"), rect.clone(), out);
    door.enabled = Some(Expr::Var("has_key".into()));
    door.actions = vec![HotspotAction::SetVar {
        var: "coins".into(),
        op: OpKind::Sub,
        value: Operand::Lit(Value::Int(1)),
    }];
    let mut painting = Hotspot::new(story.strings.add("Painting"), rect, start);
    painting.actions = vec![HotspotAction::Call { label: look }];
    let say = |story: &mut Story, text: &str| NodeKind::Say {
        speaker: None,
        text_id: story.strings.add(text),
        voice: None,
    };
    let (outside, nice) = (say(&mut story, "outside"), say(&mut story, "a nice painting"));
    let nodes = vec![
        Node { id: start, kind: NodeKind::Label { name: "start".into() } },
        Node {
            id: map,
            kind: NodeKind::Hotspots(HotspotMap { hotspots: vec![key, door, painting], ..Default::default() }),
        },
        Node { id: out, kind: NodeKind::Label { name: "out".into() } },
        Node { id: NodeId::new(), kind: outside },
        Node { id: NodeId::new(), kind: NodeKind::End },
        Node { id: look, kind: NodeKind::Label { name: "look".into() } },
        Node { id: NodeId::new(), kind: nice },
        Node { id: NodeId::new(), kind: NodeKind::Return },
    ];
    story.scenes.push(Scene { id: SceneId::new(), name: "room".into(), nodes });
    Room { story, map, snd }
}

fn on_map(vm: &mut Vm, map: NodeId) -> Vec<usize> {
    let StepResult::Choice { node, .. } = vm.step() else { panic!("expected the map") };
    assert_eq!(node, map);
    vm.hotspot_choice().unwrap().active
}

fn said(vm: &mut Vm) -> String {
    match vm.step() {
        StepResult::Say { string_id, .. } => vm.text(string_id).unwrap().to_owned(),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn set_var_actions_then_stay_show_the_map_again() {
    let r = room();
    let mut vm = Vm::from_start(Arc::new(r.story)).unwrap();
    assert_eq!(on_map(&mut vm, r.map), vec![0, 2], "key shown, door disabled");
    assert_eq!(vm.hotspot_choice().unwrap().disabled, vec![1]);
    vm.take_events();
    vm.choose(0).unwrap();
    assert_eq!(vm.variable("has_key"), Some(&Value::Bool(true)));
    assert_eq!(vm.variable("coins"), Some(&Value::Int(5)), "actions after the flow one never run");
    let events = vm.take_events();
    assert!(events.contains(&PresentationEvent::PlaySound { asset: r.snd }));
    assert!(events.iter().any(|e| matches!(e,
        PresentationEvent::Notify { node, vars, .. }
            if *node == r.map && vars == &[("coins".to_owned(), "5".to_owned())])));
    // Stay: the same map, the key gone, the door open.
    assert_eq!(on_map(&mut vm, r.map), vec![1, 2]);
    // The door: its action runs, then the story jumps to its target.
    vm.choose(0).unwrap();
    assert_eq!(vm.variable("coins"), Some(&Value::Int(4)));
    assert_eq!(said(&mut vm), "outside");
}

#[test]
fn a_call_action_comes_back_to_the_map() {
    let r = room();
    let mut vm = Vm::from_start(Arc::new(r.story)).unwrap();
    on_map(&mut vm, r.map);
    vm.choose(1).unwrap(); // the painting (second clickable zone)
    assert_eq!(said(&mut vm), "a nice painting");
    assert_eq!(on_map(&mut vm, r.map), vec![0, 2], "back on the map after Return");
}

#[test]
fn a_save_after_actions_restores_mid_map() {
    let r = room();
    let story = r.story.clone();
    let mut vm1 = Vm::from_start(Arc::new(r.story)).unwrap();
    on_map(&mut vm1, r.map);
    vm1.choose(0).unwrap();
    on_map(&mut vm1, r.map);
    let saved = ron::to_string(&vm1.snapshot()).unwrap();
    let mut vm2 = Vm::from_start(Arc::new(story)).unwrap();
    vm2.restore(ron::from_str(&saved).unwrap()).unwrap();
    assert_eq!(vm2.variable("has_key"), Some(&Value::Bool(true)));
    assert_eq!(vm2.hotspot_choice().unwrap().active, vec![1, 2]);
    vm2.choose(0).unwrap();
    assert_eq!(said(&mut vm2), "outside");
}

#[test]
fn a_jump_action_replaces_the_target_and_bad_flows_are_refused() {
    let mut r = room();
    let NodeKind::Hotspots(map) = &mut r.story.scenes[0].nodes[1].kind else { unreachable!() };
    let out = map.hotspots[1].target;
    map.hotspots[2].actions = vec![HotspotAction::Jump { label: out }];
    map.hotspots[0].actions = vec![
        HotspotAction::SetVar { var: "coins".into(), op: OpKind::Set, value: Operand::Lit(Value::Int(7)) },
        HotspotAction::Return,
    ];
    let mut vm = Vm::from_start(Arc::new(r.story)).unwrap();
    on_map(&mut vm, r.map);
    // Return without a call: refused, nothing changes.
    assert!(vm.choose(0).is_err());
    assert_eq!(vm.variable("coins"), Some(&Value::Int(0)));
    assert!(vm.hotspot_choice().is_some());
    vm.choose(1).unwrap();
    assert_eq!(said(&mut vm), "outside");
}
