//! Gallery unlocks and seen scenes in the persistent data (M7-03, M7-04).

use std::collections::BTreeMap;
use std::sync::Arc;

use orbit_core::{
    AssetEntry, AssetId, AssetKind, Node, NodeId, NodeKind, Scene, SceneId, Story, Value, VarDecl,
};
use orbit_vm::{StepResult, Vm};

use crate::PersistentData;

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

/// `one`: background `cg.png`, "a", jump `two`. `two`: "b".
fn story() -> (Story, SceneId, SceneId) {
    let mut story = Story::new("unlocks");
    story.variables = vec![VarDecl::game("gold", Value::Int(1))];
    let bg = AssetId::new();
    story.assets.assets.push(AssetEntry {
        id: bg,
        kind: AssetKind::Image,
        path: "bg/cg.png".into(),
    });
    let (one, two) = (SceneId::new(), SceneId::new());
    let a = story.strings.add("a");
    let b = story.strings.add("b");
    story.scenes = vec![
        Scene {
            id: one,
            name: "one".into(),
            nodes: vec![
                node(NodeKind::Background { asset: bg, transition: None }),
                node(NodeKind::Say { speaker: None, text_id: a, voice: None }),
                node(NodeKind::JumpScene { scene: two, label: None }),
            ],
        },
        Scene {
            id: two,
            name: "two".into(),
            nodes: vec![node(NodeKind::Say { speaker: None, text_id: b, voice: None })],
        },
    ];
    (story, one, two)
}

fn play(vm: &mut Vm) {
    for _ in 0..10 {
        if matches!(vm.step(), StepResult::End | StepResult::Error(_)) {
            break;
        }
    }
}

#[test]
fn a_playthrough_unlocks_images_and_scenes_that_survive_a_round_trip() {
    let (story, one, two) = story();
    let story = Arc::new(story);
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm);
    let data = PersistentData::from_vm(&vm);
    assert!(data.is_unlocked("bg/cg.png"));
    assert!(data.has_seen(one) && data.has_seen(two));
    assert_eq!(data.seen_scenes[&one]["gold"], Value::Int(1));
    let back = PersistentData::from_ron(&data.to_ron().unwrap()).unwrap();
    assert_eq!(back, data);
    // Loaded into the next playthrough, then written back: nothing is lost.
    let mut next = Vm::from_start(story).unwrap();
    back.apply_to(&mut next);
    assert_eq!(PersistentData::from_vm(&next), data);
}

#[test]
fn a_replay_writes_nothing_back() {
    let (story, one, _) = story();
    let story = Arc::new(story);
    let empty = PersistentData::default();
    let mut vm = Vm::replay(story, one, None, &BTreeMap::new()).unwrap();
    empty.apply_to(&mut vm);
    play(&mut vm);
    assert!(vm.scene_visits().is_empty());
    assert!(vm.shown_images().is_empty());
}

#[test]
fn files_without_unlocks_still_load_and_stay_unchanged() {
    // A file written before M7-03 / M7-04.
    let old = PersistentData::from_ron(
        "(format_version: 1, vars: {\"plays\": Int(2)}, read_nodes: [])",
    )
    .unwrap();
    assert!(old.seen_scenes.is_empty() && old.unlocked_images.is_empty());
    // Nothing new is written while there is nothing to remember.
    let text = old.to_ron().unwrap();
    assert!(!text.contains("seen_scenes") && !text.contains("unlocked_images"));
}

#[test]
fn unlocks_of_vanished_scenes_and_images_are_dropped_on_load() {
    let (story, one, _) = story();
    let mut data = PersistentData::default();
    data.seen_scenes.insert(one, BTreeMap::new());
    data.seen_scenes.insert(SceneId::new(), BTreeMap::new());
    data.unlocked_images.insert("bg/cg.png".into());
    data.unlocked_images.insert("bg/deleted.png".into());
    let mut vm = Vm::from_start(story).unwrap();
    data.apply_to(&mut vm);
    assert_eq!(vm.scene_visits().len(), 1);
    assert_eq!(vm.shown_images().len(), 1);
}
