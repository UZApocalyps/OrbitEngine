//! Screens opened by the story (UI-14, ADR 0016): waiting and non-waiting
//! screens, results, parameters, rollback, skip-like replays and restores.

use orbit_core::{
    Node, NodeId, NodeKind, Operand, ScalarType, Scene, SceneId, ScreenParam, Story, Value,
    VarDecl,
};

use crate::{Interaction, Pending, StepResult, Vm, VmError};

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

fn say(story: &mut Story, text: &str) -> Node {
    let text_id = story.strings.add(text);
    node(NodeKind::Say {
        speaker: None,
        text_id,
    })
}

fn open(screen: &str, wait: bool, result: Option<&str>, params: Vec<ScreenParam>) -> Node {
    node(NodeKind::OpenScreen {
        screen: screen.into(),
        wait,
        result: result.map(Into::into),
        params,
    })
}

fn param(name: &str, value: Operand) -> ScreenParam {
    ScreenParam {
        name: name.into(),
        value,
    }
}

/// `hud` (no wait) -> "one" -> `inventory` (wait, result `picked`, params)
/// -> "two" -> close `hud` -> "three".
fn story() -> Story {
    let mut story = Story::new("screens");
    story.variables = vec![
        VarDecl::game("picked", Value::from("")),
        VarDecl::game("gold", Value::Int(7)),
        VarDecl::game(
            "bag",
            Value::list(ScalarType::Str, [Value::from("key"), Value::from("map")]),
        ),
    ];
    let one = say(&mut story, "one");
    let two = say(&mut story, "two");
    let three = say(&mut story, "three");
    let nodes = vec![
        open("hud", false, None, vec![]),
        one,
        open(
            "inventory",
            true,
            Some("picked"),
            vec![
                param("title", Operand::Lit(Value::from("Bag"))),
                param("gold", Operand::Var("gold".into())),
            ],
        ),
        two,
        node(NodeKind::CloseScreen {
            screen: "hud".into(),
        }),
        three,
    ];
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "main".into(),
        nodes,
    });
    story
}

fn text(vm: &Vm, r: &StepResult) -> String {
    match r {
        StepResult::Say { string_id, .. } => vm.text(*string_id).unwrap().to_owned(),
        other => format!("{other:?}"),
    }
}

fn screens(vm: &Vm) -> Vec<(&str, bool)> {
    vm.open_screens()
        .iter()
        .map(|s| (s.screen.as_str(), s.waiting))
        .collect()
}

#[test]
fn a_waiting_screen_blocks_until_closed_and_stores_its_result() {
    let mut vm = Vm::from_start(story()).unwrap();
    let r = vm.step();
    assert_eq!(text(&vm, &r), "one");
    assert_eq!(screens(&vm), vec![("hud", false)], "the HUD does not block");

    let r = vm.step();
    let StepResult::Screen { screen, node } = &r else {
        panic!("expected a screen, got {r:?}");
    };
    assert_eq!(screen, "inventory");
    assert_eq!(vm.snapshot().pending, Pending::Screen { node: *node });
    assert!(matches!(vm.snapshot().shown, Some(Interaction::Screen { .. })));
    // Parameters are evaluated when the node runs.
    let inv = &vm.open_screens()[1];
    assert_eq!(
        inv.params,
        vec![
            ("title".to_owned(), Value::from("Bag")),
            ("gold".to_owned(), Value::Int(7)),
        ]
    );
    // Stepping again does not move on.
    assert_eq!(vm.step(), r);
    assert_eq!(vm.step(), r);

    assert_eq!(vm.close_screen("inventory", Some(Value::from("map"))), Ok(true));
    assert_eq!(vm.variable("picked"), Some(&Value::from("map")));
    let r = vm.step();
    assert_eq!(text(&vm, &r), "two");
    assert_eq!(screens(&vm), vec![("hud", false)]);
    let r = vm.step();
    assert_eq!(text(&vm, &r), "three");
    assert!(screens(&vm).is_empty(), "CloseScreen closed the HUD");
}

#[test]
fn closing_without_a_value_or_with_a_wrong_type_keeps_the_variable() {
    let mut vm = Vm::from_start(story()).unwrap();
    vm.step();
    vm.step();
    assert_eq!(vm.close_screen("inventory", Some(Value::Bool(true))), Ok(true));
    assert_eq!(vm.variable("picked"), Some(&Value::from("")));
    assert!(vm.take_warnings().iter().any(|w| w.contains("result ignored")));

    let mut vm = Vm::from_start(story()).unwrap();
    vm.step();
    vm.step();
    assert_eq!(vm.close_screen("inventory", None), Ok(true));
    assert_eq!(vm.variable("picked"), Some(&Value::from("")));
    // A number is converted to the declared type (int -> int here).
    assert_eq!(
        vm.close_screen("nope", None),
        Err(VmError::ScreenNotOpen("nope".into()))
    );
}

#[test]
fn rollback_reopens_the_waiting_screen_and_removes_later_ones() {
    let mut vm = Vm::from_start(story()).unwrap();
    vm.step(); // one
    vm.step(); // inventory
    vm.close_screen("inventory", Some(Value::from("key"))).unwrap();
    vm.step(); // two
    assert_eq!(screens(&vm), vec![("hud", false)]);

    // One step back: the screen is open again, waiting, in its initial
    // state, and the result is undone.
    let p = vm.rollback(1).unwrap();
    assert!(matches!(p.interaction, Some(Interaction::Screen { ref screen, .. }) if screen == "inventory"));
    assert_eq!(screens(&vm), vec![("hud", false), ("inventory", true)]);
    assert_eq!(vm.variable("picked"), Some(&Value::from("")));
    assert!(matches!(vm.step(), StepResult::Screen { .. }));

    // Closing with another value goes on with it.
    vm.close_screen("inventory", Some(Value::from("map"))).unwrap();
    let r = vm.step();
    assert_eq!(text(&vm, &r), "two");
    assert_eq!(vm.variable("picked"), Some(&Value::from("map")));

    // Back to the first line: before the inventory, only the HUD is open.
    vm.rollback(2).unwrap();
    assert_eq!(screens(&vm), vec![("hud", false)]);
}

#[test]
fn closing_a_non_waiting_screen_is_kept_by_the_checkpoint_on_screen() {
    let mut vm = Vm::from_start(story()).unwrap();
    vm.step(); // one, HUD open
    assert_eq!(vm.close_screen("hud", None), Ok(false));
    assert!(screens(&vm).is_empty());
    // Rolling back to the line on screen keeps the HUD closed...
    vm.step(); // inventory
    vm.rollback(1).unwrap();
    assert_eq!(screens(&vm), Vec::<(&str, bool)>::new());
}

#[test]
fn snapshot_and_restore_keep_the_waiting_screen() {
    let story = story();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    vm.step();
    vm.step();
    let state = vm.snapshot();
    let mut other = Vm::from_start(story).unwrap();
    other.restore(state.clone()).unwrap();
    assert_eq!(other.open_screens(), vm.open_screens());
    assert!(matches!(other.step(), StepResult::Screen { .. }));
    other.close_screen("inventory", Some(Value::from("key"))).unwrap();
    let r = other.step();
    assert_eq!(text(&other, &r), "two");

    // A state whose waiting screen points nowhere is refused.
    let mut broken = state;
    broken.open_screens.clear();
    let mut fresh = Vm::from_start(story_for_restore()).unwrap();
    assert!(fresh.restore(broken).is_err());
}

fn story_for_restore() -> Story {
    story()
}

#[test]
fn reopening_a_screen_replaces_it_and_scene_jumps_drop_waiting_screens() {
    let mut story = Story::new("re");
    let line = say(&mut story, "line");
    let second = SceneId::new();
    let end_line = say(&mut story, "end");
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "a".into(),
        nodes: vec![
            open("hud", false, None, vec![param("n", Operand::Lit(Value::Int(1)))]),
            open("hud", false, None, vec![param("n", Operand::Lit(Value::Int(2)))]),
            line,
            open("menu", true, None, vec![]),
        ],
    });
    story.scenes.push(Scene {
        id: second,
        name: "b".into(),
        nodes: vec![end_line],
    });
    let mut vm = Vm::from_start(story).unwrap();
    vm.step();
    assert_eq!(vm.open_screens().len(), 1);
    assert_eq!(vm.open_screens()[0].params[0].1, Value::Int(2));
    assert!(matches!(vm.step(), StepResult::Screen { .. }));
    vm.goto_scene(second).unwrap();
    assert_eq!(screens(&vm), vec![("hud", false)], "the waiting menu is dropped");
    let r = vm.step();
    assert_eq!(text(&vm, &r), "end");
}

#[test]
fn an_unknown_parameter_variable_is_an_error() {
    let mut story = Story::new("bad");
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "a".into(),
        nodes: vec![open("x", false, None, vec![param("p", Operand::Var("ghost".into()))])],
    });
    let mut vm = Vm::from_start(story).unwrap();
    assert!(matches!(vm.step(), StepResult::Error(VmError::Eval { .. })));
}

#[test]
fn seek_passes_waiting_screens() {
    let story = std::sync::Arc::new(story());
    let target = story.scenes[0].nodes[5].id; // "three"
    match crate::seek(&story, target, crate::SeekLimits::default()) {
        crate::SeekOutcome::Found(path) => {
            let vm = path.into_vm(&story).unwrap();
            assert_eq!(vm.position(), Some(target));
            assert_eq!(vm.variable("picked"), Some(&Value::from("")));
        }
        other => panic!("{other:?}"),
    }
}
