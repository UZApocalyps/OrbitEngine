//! Saves and animations (ADR 0012): format 4 carries the running animations;
//! formats 2 and 3 still load with none running.

use std::sync::Arc;

use orbit_core::{AnimTarget, Effect, Node, NodeId, NodeKind, Story};
use orbit_vm::{StepResult, Vm};

use crate::*;

const FIXTURE: &str = include_str!("../../../examples/stories/test_minimal.ron");

fn demo() -> Arc<Story> {
    Arc::new(Story::from_ron(FIXTURE).expect("test_minimal.ron must load"))
}

fn play(vm: &mut Vm, n: usize) {
    let mut seen = 0;
    for _ in 0..10_000 {
        match vm.step() {
            StepResult::Say { .. } => seen += 1,
            StepResult::Choice { options, .. } => {
                seen += 1;
                vm.choose(0.min(options.len() - 1)).unwrap();
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

/// A frozen save written by the format-2 build must still load and play on.
#[test]
fn version_2_saves_still_load_with_no_animation_running() {
    let bytes = include_bytes!("../../../examples/fixtures/compat/save_v2/save.orbs");
    let story = demo();
    assert_eq!(SaveData::decode_meta(bytes).unwrap().format_version, 2);
    let decoded = SaveData::decode(bytes).expect("v2 save must decode");
    assert!(decoded.vm_state.active_anims.is_empty());
    assert_eq!(decoded.vm_state.scene_clock, 0.0);
    assert!(decoded.history.as_ref().is_some_and(|h| !h.checkpoints.is_empty()));

    let mut vm = Vm::from_start(story).unwrap();
    assert!(decoded.apply_to(&mut vm).unwrap());
    assert!(matches!(vm.step(), StepResult::Say { .. } | StepResult::Choice { .. }));

    let again = SaveData::capture(&vm, info(9));
    assert_eq!(again.meta.format_version, FORMAT_VERSION);
    let back = SaveData::decode(&again.encode().unwrap()).unwrap();
    assert_eq!(back.vm_state, again.vm_state);
}

/// A format-3 body (3D scene, no animation) is rebuilt from the v3 layout.
#[test]
fn version_3_saves_still_load_with_no_animation_running() {
    let story = demo();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 8);
    let save = SaveData::capture(&vm, info(7));
    let mut meta = save.meta.clone();
    meta.format_version = 3;
    let meta_bytes = postcard::to_stdvec(&meta).unwrap();
    let body = crate::legacy_v3::encode_body_for_tests(&save.vm_state, save.history.as_ref());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ORBS");
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&meta_bytes);
    bytes.extend_from_slice(&body);
    let decoded = SaveData::decode(&bytes).expect("v3 save must decode");
    assert_eq!(decoded.vm_state, save.vm_state);
    assert_eq!(decoded.history, save.history);
    let mut loaded = Vm::from_start(story).unwrap();
    assert!(decoded.apply_to(&mut loaded).unwrap());
}

/// `demo()` with a non-blocking 2 s move of the background before the first line.
fn story_with_animation() -> (Arc<Story>, NodeId) {
    let mut story = (*demo()).clone();
    let id = NodeId::new();
    story.scenes[0].nodes.insert(
        0,
        Node {
            id,
            kind: NodeKind::Effect(Effect::Move {
                target: AnimTarget::Background,
                dx: 1.0,
                dy: 0.0,
                duration: 2.0,
                ease: None,
                wait: false,
            }),
        },
    );
    (Arc::new(story), id)
}

#[test]
fn saving_and_loading_in_the_middle_of_an_animation_resumes_it() {
    let (story, id) = story_with_animation();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 3);
    vm.advance_clock(0.5);
    let save = SaveData::capture(&vm, info(1));
    assert!(save.vm_state.active_anims.iter().any(|a| a.node == id));
    let decoded = SaveData::decode(&save.encode().unwrap()).unwrap();
    assert_eq!(decoded.vm_state, save.vm_state);

    let mut loaded = Vm::from_start(story.clone()).unwrap();
    assert!(decoded.apply_to(&mut loaded).unwrap());
    assert_eq!(loaded.scene_clock(), 0.5);
    assert_eq!(loaded.stage_fx(), vm.stage_fx());
    assert!((loaded.stage_fx().background.offset[0] - 0.25).abs() < 1e-6);
    loaded.advance_clock(0.5);
    assert!((loaded.stage_fx().background.offset[0] - 0.5).abs() < 1e-6);

    let mut loaded = Vm::from_start(story).unwrap();
    assert!(decoded.apply_to(&mut loaded).unwrap());
    assert!(loaded.rollback(1).is_some());
    assert!(loaded.scene_clock() <= 0.5);
}

#[test]
fn a_save_whose_animation_node_was_deleted_still_loads() {
    let (story, id) = story_with_animation();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 2);
    let save = SaveData::capture(&vm, info(1));
    let mut edited = (*story).clone();
    edited.scenes[0].nodes.retain(|n| n.id != id);
    let mut loaded = Vm::from_start(Arc::new(edited)).unwrap();
    save.apply_to(&mut loaded).expect("the save loads anyway");
    assert!(loaded.active_anims().is_empty());
    assert!(loaded.stage_fx().is_identity());
}
