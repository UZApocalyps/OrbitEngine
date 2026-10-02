//! Plays the demo story shipped in `examples/stories/demo.ron` end to end.

use orbit_core::{Story, Value};
use orbit_vm::{PresentationEvent, StepResult, Vm};

const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

struct Run {
    lines: Vec<String>,
    events: Vec<PresentationEvent>,
}

/// Plays the demo, answering each choice with the option whose text starts
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
            StepResult::WaitTime(_) | StepResult::WaitTransition { .. } => {}
            StepResult::End => return (run, vm),
            StepResult::Error(e) => panic!("VM error: {e}"),
        }
    }
    panic!("demo did not finish");
}

fn play(script: &[&str]) -> (Run, Vm) {
    let story = Story::from_ron(DEMO).unwrap();
    play_from(Vm::from_start(story).unwrap(), script)
}

/// Take the job, buy the chip at the market, hack the terminal by listening to
/// the AI, then give the chip to Helix.
const CORP: &[&str] = &[
    "Take the job",
    "Visit the black market",
    "Buy the access chip",
    "Leave the market",
    "Break into",
    "Hack the terminal",
    "Listen to what",
    "Give the chip to Helix",
];
const GHOST: &[&str] = &[
    "Take the job",
    "Visit the black market",
    "Buy the access chip",
    "Leave the market",
    "Break into",
    "Hack the terminal",
    "Listen to what",
    "Free the AI",
];
const LOUD: &[&str] = &[
    "Take the job",
    "Visit the black market",
    "Buy the access chip",
    "Leave the market",
    "Break into",
    "Force the lock",
    "Run with Bob",
];

fn has_line(run: &Run, start: &str) -> bool {
    run.lines.iter().any(|l| l.starts_with(start))
}

#[test]
fn every_ending_is_reachable_and_the_story_finishes_cleanly() {
    let destroy: Vec<&str> = GHOST[..7]
        .iter()
        .copied()
        .chain(["Destroy the chip"])
        .collect();
    let endings: [(&[&str], &str); 5] = [
        (&["Refuse"], "ENDING 1"),
        (CORP, "ENDING 2"),
        (GHOST, "ENDING 3"),
        (&destroy, "ENDING 4"),
        (LOUD, "ENDING 5"),
    ];
    for (script, ending) in endings {
        let (run, vm) = play(script);
        assert!(has_line(&run, ending), "{ending}: {:#?}", run.lines);
        assert!(run.lines.last().unwrap().contains("OrbitEngine demo"));
        assert!(vm.is_finished());
        assert_eq!(vm.call_depth(), 0);
        // Music is stopped at the end.
        assert!(vm.scene().music.is_none());
        assert!(vm.scene().background.is_some());
    }
}

#[test]
fn the_story_is_long_and_variables_drive_the_branches() {
    let (corp, vm) = play(CORP);
    assert!(corp.lines.len() >= 30, "{}", corp.lines.len());
    // 100 + 50 advance - 60 chip + 200 from Helix.
    assert_eq!(vm.variable("credits"), Some(&Value::Int(290)));
    assert_eq!(vm.variable("has_chip"), Some(&Value::Bool(true)));
    assert_eq!(vm.variable("alarm"), Some(&Value::Bool(false)));
    assert!(has_line(&corp, "Clean work, Alice"));

    // Forcing the lock trips the alarm, which changes Bob's reaction.
    let (loud, vm) = play(LOUD);
    assert_eq!(vm.variable("alarm"), Some(&Value::Bool(true)));
    assert!(has_line(&loud, "You tripped every alarm"));
    assert!(!has_line(&loud, "Clean work, Alice"));
    // Float variable: the heat accumulated during the run.
    assert!(matches!(vm.variable("heat"), Some(Value::Float(h)) if *h > 1.0));
}

#[test]
fn choices_loop_back_to_the_hub_and_the_hacking_loop_can_fail() {
    // Rest twice at the hub (a loop), then walk the tower and fail the hack.
    let (run, vm) = play(&[
        "Take the job",
        "Visit the black market",
        "Buy the access chip",
        "Leave the market",
        "Take a moment to rest",
        "Take a moment to rest",
        "Break into",
        "Hack the terminal",
        "Brute-force",
        "Brute-force",
        "Brute-force",
        "Run with Bob",
    ]);
    assert!(has_line(&run, "Access denied"));
    assert!(has_line(&run, "The terminal locks"));
    assert_eq!(vm.variable("alarm"), Some(&Value::Bool(true)));
    assert_eq!(vm.variable("tries"), Some(&Value::Int(0)));
    // Call/Return: the status routine ran at each visit of the hub.
    assert!(has_line(&run, "My wallet") || has_line(&run, "At least my wallet"));
}

#[test]
fn demo_emits_music_sound_fade_and_dissolve() {
    let (run, _) = play(CORP);
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
fn persistent_counters_survive_into_the_next_playthrough() {
    let (_, first) = play(GHOST);
    // The counters live in the persistent store, not in the saved state.
    assert_eq!(first.variable("plays"), Some(&Value::Int(1)));
    assert_eq!(first.variable("ghosts"), Some(&Value::Int(1)));
    assert!(!first.snapshot().vars.contains_key("plays"));

    // The host carries them to a new game: the story greets a returning
    // player, and a hidden option opens up at the first choice.
    let story = Story::from_ron(DEMO).unwrap();
    let mut second = Vm::from_start(story).unwrap();
    second.set_persistent_vars(first.persistent_vars());
    let mut script = vec!["\"I heard the chip"];
    script.extend(&GHOST[1..]);
    let (run, _) = play_from(second, &script);
    assert!(has_line(&run, "Welcome back"), "{:?}", run.lines);
    assert!(has_line(&run, "Where did you hear that name"));
}
