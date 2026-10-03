//! Scene replay (M7-04) and seen scenes / shown images tracking (M7-03).

use std::collections::BTreeMap;

use orbit_core::{
    AssetEntry, AssetId, AssetKind, Expr, Node, NodeId, NodeKind, Scene, SceneId, Story, Value,
    VarDecl,
};

use crate::{StepResult, Vm, VmError};

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

fn set(var: &str, value: Expr) -> Node {
    node(NodeKind::SetVar {
        var: var.into(),
        value,
    })
}

struct Fixture {
    story: Story,
    intro: SceneId,
    middle: SceneId,
    shared: SceneId,
    outro: SceneId,
    bg: AssetId,
}

/// `intro`: gold = 5, "a", jump `middle`. `middle`: background, "b"
/// (gold), gold = gold + 10, wins = wins + 1 (persistent), call `shared`,
/// "c", jump `outro`. `shared`: "s". `outro`: "z".
fn fixture() -> Fixture {
    let mut story = Story::new("replay");
    story.variables = vec![
        VarDecl::game("gold", Value::Int(0)),
        VarDecl::persistent("wins", Value::Int(0)),
    ];
    let bg = AssetId::new();
    story.assets.assets.push(AssetEntry {
        id: bg,
        kind: AssetKind::Image,
        path: "bg/cg1.png".into(),
    });
    let (intro, middle, shared, outro) = (SceneId::new(), SceneId::new(), SceneId::new(), SceneId::new());
    let a = say(&mut story, "a");
    let b = say(&mut story, "b");
    let c = say(&mut story, "c");
    let s = say(&mut story, "s");
    let s_id = s.id;
    let z = say(&mut story, "z");
    let add = |var: &str, n: i64| {
        set(var, Expr::binary(orbit_core::BinOp::Add, Expr::var(var), Expr::lit(n)))
    };
    story.scenes = vec![
        Scene {
            id: intro,
            name: "intro".into(),
            nodes: vec![
                set("gold", Expr::lit(5i64)),
                a,
                node(NodeKind::JumpScene { scene: middle, label: None }),
            ],
        },
        Scene {
            id: middle,
            name: "middle".into(),
            nodes: vec![
                node(NodeKind::Background { asset: bg, transition: None }),
                b,
                add("gold", 10),
                add("wins", 1),
                node(NodeKind::Call { label: s_id }),
                c,
                node(NodeKind::JumpScene { scene: outro, label: None }),
            ],
        },
        Scene {
            id: shared,
            name: "shared".into(),
            nodes: vec![s, node(NodeKind::Return)],
        },
        Scene {
            id: outro,
            name: "outro".into(),
            nodes: vec![z],
        },
    ];
    Fixture { story, intro, middle, shared, outro, bg }
}

fn text(vm: &Vm, r: &StepResult) -> String {
    match r {
        StepResult::Say { string_id, .. } => vm.text(*string_id).unwrap_or_default().to_owned(),
        StepResult::End => "<end>".into(),
        other => format!("{other:?}"),
    }
}

fn run(vm: &mut Vm) -> Vec<String> {
    let mut out = Vec::new();
    for _ in 0..20 {
        let r = vm.step();
        out.push(text(vm, &r));
        if matches!(r, StepResult::End | StepResult::Error(_)) {
            break;
        }
    }
    out
}

#[test]
fn a_playthrough_records_seen_scenes_with_their_first_variables_and_images() {
    let f = fixture();
    let mut vm = Vm::from_start(f.story.clone()).unwrap();
    assert!(!vm.is_replay());
    assert_eq!(run(&mut vm), ["a", "b", "s", "c", "z", "<end>"]);
    let visits = vm.scene_visits();
    assert_eq!(visits.len(), 4);
    assert_eq!(visits[&f.intro]["gold"], Value::Int(0));
    // `middle` was entered with the value `intro` set.
    assert_eq!(visits[&f.middle]["gold"], Value::Int(5));
    assert!(visits.contains_key(&f.shared));
    assert!(vm.shown_images().contains(&f.bg));
    // A second entry keeps the first snapshot.
    let mut again = Vm::from_start(f.story.clone()).unwrap();
    again.set_scene_visits(visits.clone());
    again.state.vars.insert("gold".into(), Value::Int(99));
    run(&mut again);
    assert_eq!(again.scene_visits()[&f.intro]["gold"], Value::Int(0));
}

#[test]
fn a_replay_runs_one_scene_on_isolated_variables_and_records_nothing() {
    let f = fixture();
    let mut snapshot = BTreeMap::new();
    snapshot.insert("gold".to_owned(), Value::Int(5));
    let mut persistent = BTreeMap::new();
    persistent.insert("wins".to_owned(), Value::Int(3));
    let mut vm = Vm::replay(f.story.clone(), f.middle, Some(&snapshot), &persistent).unwrap();
    assert!(vm.is_replay());
    assert_eq!(vm.replay_scene(), Some(f.middle));
    assert_eq!(vm.variable("gold"), Some(&Value::Int(5)));
    // The called scene runs; jumping to `outro` ends the replay.
    assert_eq!(run(&mut vm), ["b", "s", "c", "<end>"]);
    assert!(vm.is_finished());
    assert_eq!(vm.variable("gold"), Some(&Value::Int(15)));
    // The caller's maps are untouched: the VM works on copies.
    assert_eq!(snapshot["gold"], Value::Int(5));
    assert_eq!(persistent["wins"], Value::Int(3));
    assert_eq!(vm.persistent_vars()["wins"], Value::Int(4));
    // Nothing is unlocked or marked seen by a replay.
    assert!(vm.scene_visits().is_empty());
    assert!(vm.shown_images().is_empty());
}

#[test]
fn a_replay_without_snapshot_uses_the_declared_initial_values() {
    let f = fixture();
    // A wrongly typed recorded value falls back to the initial one too.
    let mut snapshot = BTreeMap::new();
    snapshot.insert("gold".to_owned(), Value::from("oops"));
    for vars in [None, Some(&snapshot)] {
        let mut vm = Vm::replay(f.story.clone(), f.middle, vars, &BTreeMap::new()).unwrap();
        assert_eq!(vm.variable("gold"), Some(&Value::Int(0)));
        assert_eq!(vm.variable("wins"), Some(&Value::Int(0)));
        assert_eq!(run(&mut vm)[0], "b");
    }
}

#[test]
fn a_replay_ends_at_the_end_of_its_scene_and_refuses_unknown_scenes() {
    let f = fixture();
    let mut vm = Vm::replay(f.story.clone(), f.outro, None, &BTreeMap::new()).unwrap();
    assert_eq!(run(&mut vm), ["z", "<end>"]);
    let missing = SceneId::new();
    assert!(matches!(
        Vm::replay(f.story.clone(), missing, None, &BTreeMap::new()),
        Err(VmError::UnknownScene(id)) if id == missing
    ));
}

#[test]
fn a_goto_out_of_the_replayed_scene_ends_the_replay() {
    let f = fixture();
    let mut vm = Vm::replay(f.story.clone(), f.middle, None, &BTreeMap::new()).unwrap();
    let first = vm.step();
    assert_eq!(text(&vm, &first), "b");
    vm.goto_scene(f.intro).unwrap();
    assert_eq!(run(&mut vm), ["<end>"]);
}

