//! The `Notify` node: non-blocking, transient (never in the saved state),
//! with the `[var]` values of the moment it ran.

use orbit_core::{Node, NodeId, NodeKind, Notify, Scene, SceneId, Story, Value, VarDecl};

use crate::{PresentationEvent, StepResult, Vm};

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

/// "a", notify "Gold: [gold]", gold = gold + 1, "b".
fn story() -> Story {
    let mut story = Story::new("toasts");
    story.variables.push(VarDecl::game("gold", Value::Int(3)));
    let a = story.strings.add("a");
    let b = story.strings.add("b");
    let text_id = story.strings.add("Gold: [gold]");
    story.scenes = vec![Scene {
        id: SceneId::new(),
        name: "s".into(),
        nodes: vec![
            node(NodeKind::Say { speaker: None, text_id: a, voice: None }),
            node(NodeKind::Notify(Notify {
                text_id,
                icon: None,
                sound: None,
                duration: Some(2.0),
                style: Some("achievement".into()),
            })),
            node(NodeKind::SetVar {
                var: "gold".into(),
                value: GOLD_PLUS_ONE.clone(),
            }),
            node(NodeKind::Say { speaker: None, text_id: b, voice: None }),
        ],
    }];
    story
}

static GOLD_PLUS_ONE: std::sync::LazyLock<orbit_core::Expr> =
    std::sync::LazyLock::new(|| orbit_core::Expr::parse("gold + 1").expect("expression"));

fn notifies(vm: &mut Vm) -> Vec<(Notify, Vec<(String, String)>)> {
    vm.take_events()
        .into_iter()
        .filter_map(|e| match e {
            PresentationEvent::Notify { notify, vars, .. } => Some((notify, vars)),
            _ => None,
        })
        .collect()
}

fn say_text(vm: &Vm, result: StepResult) -> String {
    match result {
        StepResult::Say { string_id, .. } => vm.text(string_id).unwrap_or_default().to_owned(),
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn notify_does_not_block_and_snapshots_its_variables() {
    let mut vm = Vm::from_start(story()).unwrap();
    let first = vm.step();
    assert_eq!(say_text(&vm, first), "a");
    assert!(notifies(&mut vm).is_empty());
    // The notify runs and the story reaches "b" in the same step.
    let second = vm.step();
    assert_eq!(say_text(&vm, second), "b");
    let got = notifies(&mut vm);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0.style.as_deref(), Some("achievement"));
    assert_eq!(got[0].1, vec![("gold".to_owned(), "3".to_owned())], "value before the SetVar");
    assert_eq!(vm.variable("gold"), Some(&Value::Int(4)));
}

#[test]
fn a_save_holds_no_toast_and_loading_does_not_replay_it() {
    let story = std::sync::Arc::new(story());
    let mut vm = Vm::from_start(story.clone()).unwrap();
    vm.step();
    vm.step();
    let _ = vm.take_events();
    let state = vm.snapshot();
    let text = format!("{state:?}");
    assert!(!text.contains("achievement"), "no notification in the saved state");
    let mut loaded = Vm::from_start(story).unwrap();
    loaded.restore(state).unwrap();
    loaded.step();
    assert!(notifies(&mut loaded).is_empty(), "a load does not show it again");
}
