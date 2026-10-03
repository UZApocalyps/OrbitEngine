//! Rigged characters (M5B-07, schema v14): a rig clip is declarative (the
//! expression plus the scene clock), so saves need no new data and keep
//! format 6; a save made in the middle of a clip loads on the same
//! expression and a rollback goes back to the previous one.

use std::sync::Arc;

use orbit_core::Story;
use orbit_vm::{StepResult, Vm};

use crate::*;

const RIGGED: &str = include_str!("../../../examples/stories/test_rigged.ron");

fn story() -> Arc<Story> {
    Arc::new(Story::from_ron(RIGGED).expect("test_rigged.ron must load"))
}

/// Runs until the `n`-th line is displayed.
fn play_to_line(vm: &mut Vm, n: usize) {
    let mut seen = 0;
    for _ in 0..1_000 {
        match vm.step() {
            StepResult::Say { .. } => {
                seen += 1;
                if seen == n {
                    return;
                }
            }
            StepResult::Error(e) => panic!("{e}"),
            StepResult::End => panic!("the story ended early"),
            _ => {}
        }
    }
    panic!("story did not progress");
}

fn expression(vm: &Vm) -> String {
    vm.scene().characters[0].expression.clone()
}

#[test]
fn a_save_in_the_middle_of_a_clip_loads_on_the_same_expression_and_format_stays_6() {
    assert_eq!(FORMAT_VERSION, 6, "rigs add nothing to the save format");
    let story = story();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play_to_line(&mut vm, 2);
    assert_eq!(expression(&vm), "wave");
    // Time passes on the scene clock while the line waits.
    vm.advance_clock(0.4);
    let save = SaveData::capture(&vm, CaptureInfo::new(1, 1));
    let decoded = SaveData::decode(&save.encode().unwrap()).unwrap();
    assert_eq!(decoded.vm_state, save.vm_state);
    assert_eq!(decoded.meta.format_version, 6);
    assert_eq!(check_compat(&story, &decoded), Compat::Compatible);

    let mut loaded = Vm::from_start(story).unwrap();
    assert!(decoded.apply_to(&mut loaded).unwrap());
    assert_eq!(expression(&loaded), "wave");
    // Nothing about the clip is stored: the runtime derives the pose from
    // the expression and its clock, which restarts with the loaded game.
    assert_eq!(loaded.snapshot().scene.characters, vm.snapshot().scene.characters);
}

#[test]
fn a_rollback_goes_back_to_the_previous_expression() {
    let story = story();
    let mut vm = Vm::from_start(story).unwrap();
    play_to_line(&mut vm, 2);
    assert_eq!(expression(&vm), "wave");
    vm.rollback(1).expect("a line to go back to");
    assert_eq!(expression(&vm), "idle");
    // And forward again replays the change (the next line is line 2).
    play_to_line(&mut vm, 1);
    assert_eq!(expression(&vm), "wave");
}
