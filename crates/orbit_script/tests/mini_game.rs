//! The scripted mini-game fixture (`examples/stories/test_script.ron` and
//! its module `examples/stories/scripts/dice.rhai`, M8-05): it validates
//! without warnings, plays to an end deterministically, and its progress
//! survives saves and rollback after scripts ran.

use std::collections::BTreeMap;
use std::sync::Arc;

use orbit_core::{CURRENT_SCHEMA_VERSION, Story, Value, validate};
use orbit_save::{CaptureInfo, SaveData};
use orbit_script::ScriptEngine;
use orbit_vm::{StepResult, Vm};

const STORY: &str = include_str!("../../../examples/stories/test_script.ron");
const DICE: &str = include_str!("../../../examples/stories/scripts/dice.rhai");

fn modules() -> BTreeMap<String, String> {
    BTreeMap::from([("dice".to_owned(), DICE.to_owned())])
}

fn story() -> Arc<Story> {
    Arc::new(Story::from_ron(STORY).unwrap())
}

fn vm_with_seed(seed: u64) -> Vm {
    let story = story();
    let entry = story.entry().unwrap();
    let mut vm = Vm::with_seed(story, entry, seed).unwrap();
    vm.set_script_host(Some(ScriptEngine::shared(modules())));
    vm
}

/// Plays always picking the first visible option; returns the number of
/// lines seen.
fn play(vm: &mut Vm) -> usize {
    let mut lines = 0;
    for _ in 0..500 {
        match vm.step() {
            StepResult::End => return lines,
            StepResult::Say { .. } => lines += 1,
            StepResult::Choice { .. } => vm.choose(0).unwrap(),
            StepResult::Error(e) => panic!("{e}"),
            _ => {}
        }
    }
    panic!("the duel did not end");
}

#[test]
fn the_fixture_is_clean() {
    let story = story();
    assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION);
    let report = validate(&story);
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert!(report.warnings.is_empty(), "{:#?}", report.warnings);
    let engine = ScriptEngine::new(modules());
    for source in orbit_script::script_sources(&story) {
        let expression = !source.contains("import") && !source.contains("branch(");
        let problems = engine.check(source, expression);
        assert!(problems.is_empty(), "{source}: {problems:?}");
    }
    assert!(engine.check_module(DICE).is_empty());
}

#[test]
fn the_duel_plays_to_an_end_the_same_way_for_a_seed() {
    let mut a = vm_with_seed(7);
    play(&mut a);
    let warnings = a.take_warnings();
    assert!(
        warnings.iter().all(|w| w.contains("history:")),
        "only the finale's log: {warnings:?}"
    );
    let round = a.variable("round").cloned();
    assert!(matches!(round, Some(Value::Int(1..=3))), "{round:?}");
    let Some(Value::List(_, history)) = a.variable("history") else {
        panic!("history is a list");
    };
    assert_eq!(Some(&Value::Int(history.len() as i64)), a.variable("round"));
    let mut b = vm_with_seed(7);
    play(&mut b);
    assert_eq!(a.snapshot(), b.snapshot(), "same seed, same game");
    // Another seed plays another game (the dice come from the VM's RNG).
    let differs = (8..20).any(|seed| {
        let mut c = vm_with_seed(seed);
        play(&mut c);
        c.variables() != a.variables()
    });
    assert!(differs);
}

#[test]
fn saves_and_rollback_after_a_script_restore_the_exact_state() {
    let mut vm = vm_with_seed(3);
    // Up to the first choice: the round script ran (round 1, a roll).
    loop {
        match vm.step() {
            StepResult::Choice { .. } => break,
            StepResult::End | StepResult::Error(_) => panic!("no choice"),
            _ => {}
        }
    }
    assert_eq!(vm.variable("round"), Some(&Value::Int(1)));
    let save = SaveData::capture(&vm, CaptureInfo::new(0, 0)).encode().unwrap();
    vm.choose(0).unwrap();
    // The bet script ran: gold changed.
    vm.step();
    let after_bet = vm.snapshot();
    assert_ne!(after_bet.vars.get("gold"), Some(&Value::Int(10)));

    // Loading the save and making the same choice replays the same dice.
    let mut loaded = vm_with_seed(999);
    SaveData::decode(&save).unwrap().apply_to(&mut loaded).unwrap();
    assert_eq!(loaded.variable("round"), Some(&Value::Int(1)));
    assert_eq!(loaded.variable("gold"), Some(&Value::Int(10)));
    loaded.step();
    loaded.choose(0).unwrap();
    loaded.step();
    assert_eq!(loaded.snapshot().vars, after_bet.vars);
    assert_eq!(loaded.snapshot().rng, after_bet.rng);

    // Rolling back over the bet undoes its writes and its draws.
    assert!(vm.can_rollback());
    vm.rollback(1).unwrap();
    assert_eq!(vm.variable("gold"), Some(&Value::Int(10)));
    vm.choose(0).unwrap();
    vm.step();
    assert_eq!(vm.snapshot().vars, after_bet.vars);
}

/// Compatibility (docs/IMPLEMENTATION_PLAN.md §1.3): the frozen v14 story
/// (before scripts) migrates, validates, plays with a script engine
/// attached (nothing changes for it) and saves / reloads.
#[test]
fn a_frozen_v14_story_plays_and_saves_with_scripting_on() {
    let source = include_str!("../../../examples/fixtures/compat/v14/story.ron");
    let story = Arc::new(Story::from_ron_unchecked(source).unwrap());
    assert_eq!(story.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(validate(&story).errors.is_empty());
    assert!(!orbit_script::story_uses_scripts(&story));
    let mut vm = Vm::from_start(story.clone()).unwrap();
    vm.set_script_host(Some(ScriptEngine::shared(BTreeMap::new())));
    let mut saved = None;
    for _ in 0..200 {
        match vm.step() {
            StepResult::End => break,
            StepResult::Say { .. } if saved.is_none() => {
                saved = Some(SaveData::capture(&vm, CaptureInfo::new(0, 0)).encode().unwrap());
            }
            StepResult::Choice { .. } => vm.choose(0).unwrap(),
            StepResult::Error(e) => panic!("{e}"),
            _ => {}
        }
    }
    let mut again = Vm::from_start(story).unwrap();
    SaveData::decode(&saved.expect("a line was shown"))
        .unwrap()
        .apply_to(&mut again)
        .unwrap();
    assert!(again.position().is_some());
}

#[test]
fn a_player_without_scripts_skips_them_with_warnings() {
    let story = story();
    let mut vm = Vm::from_start(story).unwrap();
    for _ in 0..50 {
        match vm.step() {
            StepResult::End => break,
            StepResult::Choice { .. } => vm.choose(0).unwrap(),
            StepResult::Error(e) => panic!("{e}"),
            _ => {}
        }
    }
    assert!(vm.take_warnings().iter().any(|w| w.contains("cannot run scripts")));
}
