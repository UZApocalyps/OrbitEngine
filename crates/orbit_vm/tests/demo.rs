//! Plays the demo story shipped in `examples/stories/demo.ron` end to end.

use orbit_core::Story;
use orbit_vm::{PresentationEvent, StepResult, Vm};

const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

struct Run {
    lines: Vec<String>,
    events: Vec<PresentationEvent>,
}

fn play(choice: usize) -> (Run, Vm) {
    let story = Story::from_ron(DEMO).unwrap();
    let mut vm = Vm::from_start(story).unwrap();
    let mut run = Run {
        lines: Vec::new(),
        events: Vec::new(),
    };
    for _ in 0..500 {
        let result = vm.step();
        run.events.extend(vm.take_events());
        match result {
            StepResult::Say { string_id, .. } => {
                run.lines.push(vm.text(string_id).unwrap().to_owned());
            }
            StepResult::Choice { .. } => vm.choose(choice).unwrap(),
            StepResult::WaitTime(_) | StepResult::WaitTransition { .. } => {}
            StepResult::End => return (run, vm),
            StepResult::Error(e) => panic!("VM error: {e}"),
        }
    }
    panic!("demo did not finish");
}

#[test]
fn both_branches_play_at_least_twenty_lines_and_rejoin() {
    for choice in [0, 1] {
        let (run, vm) = play(choice);
        assert!(
            run.lines.len() >= 20,
            "branch {choice}: {}",
            run.lines.len()
        );
        assert!(run.lines.iter().any(|l| l.starts_with("The next morning")));
        assert!(run.lines.last().unwrap().contains("fades away"));
        assert!(vm.is_finished());
        assert_eq!(vm.call_depth(), 0);
        // Music is stopped at the end; background is the street.
        assert!(vm.scene().music.is_none());
        assert!(vm.scene().background.is_some());
    }
}

#[test]
fn variable_decides_one_line() {
    let (help, vm_help) = play(0);
    let (refuse, vm_refuse) = play(1);
    assert!(help.lines.iter().any(|l| l.starts_with("Thanks again")));
    assert!(!refuse.lines.iter().any(|l| l.starts_with("Thanks again")));
    assert!(
        refuse
            .lines
            .iter()
            .any(|l| l.starts_with("I am still annoyed"))
    );
    assert_eq!(
        vm_help.variable("helped"),
        Some(&orbit_core::Value::Bool(true))
    );
    assert_eq!(
        vm_refuse.variable("helped"),
        Some(&orbit_core::Value::Bool(false))
    );
}

#[test]
fn demo_emits_music_sound_fade_and_dissolve() {
    let (run, _) = play(0);
    let has = |f: fn(&PresentationEvent) -> bool| run.events.iter().any(f);
    assert!(has(|e| matches!(
        e,
        PresentationEvent::PlayMusic { looping: true, .. }
    )));
    assert!(has(|e| matches!(e, PresentationEvent::StopMusic { .. })));
    assert!(has(|e| matches!(e, PresentationEvent::PlaySound { .. })));
    assert!(has(|e| matches!(
        e,
        PresentationEvent::Background {
            transition: Some(t),
            ..
        } if matches!(t.kind, orbit_core::TransitionKind::Fade(_))
    )));
    assert!(has(|e| matches!(
        e,
        PresentationEvent::Show {
            transition: Some(t),
            ..
        } if matches!(t.kind, orbit_core::TransitionKind::Dissolve)
    )));
}

#[test]
fn persistent_counter_survives_into_the_next_playthrough() {
    use orbit_core::Value;

    let (_, first) = play(0);
    // The counter lives in the persistent store, not in the saved state.
    assert_eq!(first.variable("plays"), Some(&Value::Int(1)));
    assert!(!first.snapshot().vars.contains_key("plays"));

    // The host carries it to a new game: the story greets a returning player.
    let story = Story::from_ron(DEMO).unwrap();
    let mut second = Vm::from_start(story).unwrap();
    second.set_persistent_vars(first.persistent_vars());
    let mut lines = Vec::new();
    for _ in 0..4 {
        if let StepResult::Say { string_id, .. } = second.step() {
            lines.push(second.text(string_id).unwrap().to_owned());
        }
    }
    assert!(
        lines.iter().any(|l| l.starts_with("Welcome back")),
        "{lines:?}"
    );
}
