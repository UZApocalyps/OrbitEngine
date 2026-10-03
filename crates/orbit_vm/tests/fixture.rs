//! Plays the feature fixture `test_features.ron` end to end (structure-dependent assertions).

use orbit_core::{Story, Value};
use orbit_vm::{PresentationEvent, StepResult, Vm};

const FIXTURE: &str = include_str!("../../../examples/stories/test_features.ron");

struct Run {
    lines: Vec<String>,
    events: Vec<PresentationEvent>,
}

/// Plays the fixture, answering each choice with the option whose text starts
/// with the next prefix of `script`. Returns the run and the finished VM.
fn play_from(mut vm: Vm, script: &[&str]) -> (Run, Vm) {
    let mut run = Run {
        lines: Vec::new(),
        events: Vec::new(),
    };
    let mut next = 0;
    for _ in 0..2000 {
        let result = vm.step();
        run.events.extend(vm.take_events());
        match result {
            StepResult::Say { string_id, .. } => {
                run.lines.push(vm.text(string_id).unwrap().to_owned());
            }
            StepResult::Choice { options, .. } => {
                let prefix = script.get(next).expect("script too short");
                next += 1;
                let texts: Vec<_> = options
                    .iter()
                    .map(|o| vm.text(o.string_id).unwrap().to_owned())
                    .collect();
                let i = texts
                    .iter()
                    .position(|t| t.starts_with(prefix))
                    .unwrap_or_else(|| panic!("no option {prefix:?} in {texts:?}"));
                vm.choose(i).unwrap();
            }
            StepResult::WaitTime(_)
            | StepResult::WaitTransition { .. }
            | StepResult::WaitAnimation { .. } => {}
            StepResult::End => return (run, vm),
            StepResult::Error(e) => panic!("VM error: {e}"),
        }
    }
    panic!("fixture did not finish");
}

fn play(script: &[&str]) -> (Run, Vm) {
    let story = Story::from_ron(FIXTURE).unwrap();
    play_from(Vm::from_start(story).unwrap(), script)
}

const YES: &[&str] = &["Yes."];
const NO: &[&str] = &["No."];

fn has_line(run: &Run, start: &str) -> bool {
    run.lines.iter().any(|l| l.starts_with(start))
}

// Fixture: depends on the exact branch structure (yes/no -> join -> end).
#[test]
fn every_branch_reaches_the_end_cleanly() {
    for (script, line) in [(YES, "Rich now."), (NO, "Still poor.")] {
        let (run, vm) = play(script);
        assert!(has_line(&run, line), "{:#?}", run.lines);
        assert!(run.lines.last().unwrap().starts_with("The end."));
        assert!(vm.is_finished());
        assert_eq!(vm.call_depth(), 0);
        assert!(vm.scene().music.is_none());
        assert!(vm.scene().background.is_some());
    }
}

// Fixture: exact variable values and the condition hiding the third option.
#[test]
fn variables_drive_the_branches_and_conditions_hide_options() {
    let (yes, vm) = play(YES);
    assert_eq!(vm.variable("gold"), Some(&Value::Int(10)));
    assert!(has_line(&yes, "Rich now."));
    assert!(!has_line(&yes, "Still poor."));

    let (no, vm) = play(NO);
    assert_eq!(vm.variable("gold"), Some(&Value::Int(0)));
    assert!(has_line(&no, "Still poor."));

    // At gold = 0 the conditional "gift" option is not offered.
    let story = Story::from_ron(FIXTURE).unwrap();
    let mut vm = Vm::from_start(story).unwrap();
    loop {
        match vm.step() {
            StepResult::Choice { options, .. } => {
                assert_eq!(options.len(), 2);
                break;
            }
            StepResult::End | StepResult::Error(_) => panic!("no choice reached"),
            _ => {}
        }
    }
}

// Fixture: relies on the precise presentation nodes (fade, dissolve, music, sound).
#[test]
fn fixture_emits_music_sound_fade_and_dissolve() {
    let (run, _) = play(YES);
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

// Fixture: the persistent variable `visits` is only incremented in the fixture's last scene.
#[test]
fn persistent_counters_survive_into_the_next_playthrough() {
    let (_, first) = play(YES);
    assert_eq!(first.variable("visits"), Some(&Value::Int(1)));
    // The counter lives in the persistent store, not in the saved state.
    assert!(!first.snapshot().vars.contains_key("visits"));

    let story = Story::from_ron(FIXTURE).unwrap();
    let mut second = Vm::from_start(story).unwrap();
    second.set_persistent_vars(first.persistent_vars());
    let (_, second) = play_from(second, NO);
    assert_eq!(second.variable("visits"), Some(&Value::Int(2)));
}
