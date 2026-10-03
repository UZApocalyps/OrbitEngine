//! Saves and lists/maps (UI-13, ADR 0016). The new `Value` variants are
//! appended, so the postcard layout of existing data is unchanged and the
//! format stays 4: a frozen format-4 save (written before collections
//! existed) must still load, and a save holding collections must round-trip
//! with its rollback history.

use std::sync::Arc;

use orbit_core::{Expr, Node, NodeId, NodeKind, ScalarType, Scene, SceneId, Story, Value, VarDecl};
use orbit_vm::{StepResult, Vm};

use crate::*;

const MINIMAL: &str = include_str!("../../../examples/stories/test_minimal.ron");
/// The last story of schema v10, frozen (`examples/fixtures/compat/v10`).
const V10: &str = include_str!("../../../examples/fixtures/compat/v10/story.ron");
/// Written by the build of schema v10 / save format 4, before collections
/// existed: `examples/fixtures/compat/save_v4/save.orbs`, never regenerate.
const SAVE_V4: &[u8] = include_bytes!("../../../examples/fixtures/compat/save_v4/save.orbs");

fn play(vm: &mut Vm, n: usize) {
    let mut seen = 0;
    for _ in 0..10_000 {
        match vm.step() {
            StepResult::Say { .. } => seen += 1,
            StepResult::Choice { .. } => {
                seen += 1;
                vm.choose(0).unwrap();
            }
            StepResult::End => return,
            StepResult::Error(e) => panic!("{e}"),
            _ => {}
        }
        if seen >= n {
            return;
        }
    }
    panic!("story did not progress");
}

fn info(t: u64) -> CaptureInfo {
    CaptureInfo::new(t, t / 1000)
}

#[test]
fn a_frozen_format_4_save_still_loads_and_plays_on() {
    assert_eq!(&SAVE_V4[4..8], &4u32.to_le_bytes());
    let story = Arc::new(Story::from_ron(MINIMAL).unwrap());
    let decoded = SaveData::decode(SAVE_V4).expect("the format-4 save decodes");
    assert_eq!(decoded.meta.format_version, 4);
    assert_eq!(check_compat(&story, &decoded), Compat::Compatible);
    let mut vm = Vm::from_start(story.clone()).unwrap();
    assert!(decoded.apply_to(&mut vm).unwrap(), "with its history");
    // Same state as the run that wrote it.
    let mut fresh = Vm::from_start(story).unwrap();
    play(&mut fresh, 8);
    assert_eq!(vm.snapshot().vars, fresh.snapshot().vars);
    assert_eq!(vm.position(), fresh.position());
    play(&mut vm, 2);
    assert!(vm.rollback(1).is_some());
    let again = SaveData::capture(&vm, info(9));
    let back = SaveData::decode(&again.encode().unwrap()).unwrap();
    assert_eq!(back.vm_state, again.vm_state);
}

#[test]
fn a_frozen_v10_story_plays_saves_and_reloads() {
    let story = Arc::new(Story::from_ron(V10).expect("the v10 story migrates and validates"));
    assert_eq!(story.schema_version, orbit_core::CURRENT_SCHEMA_VERSION);
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 2);
    let save = SaveData::capture(&vm, info(3));
    let back = SaveData::decode(&save.encode().unwrap()).unwrap();
    let mut loaded = Vm::from_start(story).unwrap();
    back.apply_to(&mut loaded).unwrap();
    assert_eq!(loaded.snapshot(), vm.snapshot());
}

fn inventory_story() -> Story {
    let mut story = Story::new("inventory");
    story.variables = vec![
        VarDecl::game("bag", Value::empty_list(ScalarType::Str)),
        VarDecl::game("gold", Value::map(ScalarType::Int, [("alice", Value::Int(1))])),
        VarDecl::persistent("endings", Value::empty_map(ScalarType::Bool)),
    ];
    let mut nodes = Vec::new();
    for i in 0..5 {
        nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::SetVar {
                var: "bag".into(),
                value: Expr::parse(&format!("push(bag, \"item{i}\")")).unwrap(),
            },
        });
        nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::SetVar {
                var: "gold".into(),
                value: Expr::parse(&format!("insert(gold, \"k{i}\", {i})")).unwrap(),
            },
        });
        let text_id = story.strings.add(format!("line {i}"));
        nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id,
            },
        });
    }
    nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::SetVar {
            var: "endings".into(),
            value: Expr::parse("insert(endings, \"good\", true)").unwrap(),
        },
    });
    nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::End,
    });
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "main".into(),
        nodes,
    });
    story
}

#[test]
fn collections_round_trip_through_a_save_with_history() {
    let story = Arc::new(inventory_story());
    assert!(orbit_core::validate(&story).errors.is_empty());
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 4);
    let save = SaveData::capture(&vm, info(5));
    assert_eq!(save.meta.format_version, FORMAT_VERSION);
    let bytes = save.encode().unwrap();
    let back = SaveData::decode(&bytes).unwrap();
    assert_eq!(back.vm_state, save.vm_state);
    assert_eq!(back.history, save.history);
    let mut loaded = Vm::from_start(story).unwrap();
    assert!(back.apply_to(&mut loaded).unwrap());
    let Some(Value::List(ScalarType::Str, bag)) = loaded.variable("bag") else {
        panic!("bag is a list of text")
    };
    assert_eq!(bag.len(), 4);
    // Rolling back after loading restores the shorter list.
    loaded.rollback(1).unwrap();
    assert_eq!(loaded.variable("bag").and_then(Value::len), Some(3));
    assert_eq!(loaded.variable("gold").and_then(Value::len), Some(4));
}

#[test]
fn persistent_collections_round_trip_through_ron() {
    let story = Arc::new(inventory_story());
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 100);
    let data = PersistentData::from_vm(&vm);
    let text = data.to_ron().unwrap();
    let back = PersistentData::from_ron(&text).unwrap();
    assert_eq!(back, data);
    let mut next = Vm::from_start(story).unwrap();
    back.apply_to(&mut next);
    assert_eq!(
        next.variable("endings"),
        Some(&Value::map(ScalarType::Bool, [("good", Value::Bool(true))]))
    );
    // A persistent file written before collections (scalars only) still reads.
    let old = PersistentData::from_ron("(format_version: 1, vars: {\"plays\": Int(2)})").unwrap();
    assert_eq!(old.vars.get("plays"), Some(&Value::Int(2)));
}
