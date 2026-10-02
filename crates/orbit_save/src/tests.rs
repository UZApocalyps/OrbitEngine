use std::sync::Arc;

use orbit_core::{Node, NodeId, NodeKind, Story, Value, VarDecl};
use orbit_vm::{Pending, Rng, StepResult, Vm};

use crate::*;

const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

fn demo() -> Arc<Story> {
    Arc::new(Story::from_ron(DEMO).expect("demo.ron must load"))
}

fn manager(story: &Arc<Story>, storage: MemoryStorage) -> SaveManager {
    SaveManager::new(Box::new(storage), story.clone(), SaveConfig::default())
}

/// Plays `n` interactions, always picking option `pick`.
fn play(vm: &mut Vm, n: usize, pick: usize) {
    let mut seen = 0;
    for _ in 0..10_000 {
        match vm.step() {
            StepResult::Say { .. } => seen += 1,
            StepResult::Choice { options, .. } => {
                seen += 1;
                vm.choose(pick.min(options.len() - 1)).unwrap();
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
fn encode_decode_roundtrip_with_history_and_thumbnail() {
    let story = demo();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 20, 0);
    let mut capture = info(1_700_000_000_000);
    capture.thumbnail = Some(vec![0x89, b'P', b'N', b'G', 1, 2, 3]);
    let save = SaveData::capture(&vm, capture);
    assert_eq!(save.meta.format_version, FORMAT_VERSION);
    assert_eq!(save.meta.story_hash, story_hash(&story));
    assert_eq!(save.meta.label, "hub");
    assert!(!save.meta.preview.is_empty());
    assert!(save.meta.has_history);

    let bytes = save.encode().unwrap();
    assert_eq!(&bytes[..4], b"ORBS");
    let decoded = SaveData::decode(&bytes).unwrap();
    assert_eq!(decoded, save);
    // The cheap path reads the same metadata.
    assert_eq!(SaveData::decode_meta(&bytes).unwrap(), save.meta);

    // Loading into a fresh VM reproduces the state, scene and history.
    let mut loaded = Vm::from_start(story).unwrap();
    assert!(decoded.apply_to(&mut loaded).unwrap());
    // The read set is persistent data, not part of the save.
    loaded.set_read_nodes(vm.read_nodes().clone());
    assert_eq!(loaded.snapshot(), vm.snapshot());
    assert_eq!(loaded.presentation(), vm.presentation());
    assert_eq!(loaded.backlog(), vm.backlog());
    let a = vm.rollback(3).unwrap();
    let b = loaded.rollback(3).unwrap();
    assert_eq!(a, b);
    // And the story continues identically.
    for _ in 0..5 {
        let (x, y) = (vm.step(), loaded.step());
        assert_eq!(x, y);
        if matches!(x, StepResult::Choice { .. }) {
            vm.choose(0).unwrap();
            loaded.choose(0).unwrap();
        }
    }
}

#[test]
fn history_is_optional_and_bounded() {
    let story = demo();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 25, 0);
    let mut small = info(1);
    small.max_history = Some(5);
    let with_five = SaveData::capture(&vm, small);
    assert_eq!(with_five.history.as_ref().unwrap().checkpoints.len(), 5);
    let mut none = info(1);
    none.max_history = None;
    let without = SaveData::capture(&vm, none);
    assert!(without.history.is_none() && !without.meta.has_history);
    assert!(without.encode().unwrap().len() < with_five.encode().unwrap().len());

    let mut loaded = Vm::from_start(story).unwrap();
    assert!(!without.apply_to(&mut loaded).unwrap());
    assert!(!loaded.can_rollback());
    assert_eq!(
        loaded.presentation().interaction,
        vm.presentation().interaction
    );
}

#[test]
fn story_hash_ignores_texts_but_not_structure() {
    let story = demo();
    let h = story_hash(&story);
    let mut typo = (*story).clone();
    let first = *typo.strings.entries.keys().next().unwrap();
    typo.strings.set(first, "Corrected text");
    typo.title = "Another title".into();
    assert_eq!(story_hash(&typo), h);

    let mut extra = (*story).clone();
    extra.scenes[0].nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::End,
    });
    assert_ne!(story_hash(&extra), h);
    let mut retyped = (*story).clone();
    retyped.scenes[0].nodes[0].kind = NodeKind::End;
    assert_ne!(story_hash(&retyped), h);
}

#[test]
fn compat_with_a_modified_story() {
    let story = demo();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 12, 0);
    let save = SaveData::capture(&vm, info(5));

    // Same story, a text fixed: compatible.
    let mut typo = (*story).clone();
    let first = *typo.strings.entries.keys().next().unwrap();
    typo.strings.set(first, "Fixed");
    assert_eq!(check_compat(&typo, &save), Compat::Compatible);

    // A node added elsewhere and a new variable: loadable with warnings,
    // and the loaded VM has the new variable initialized.
    let mut grown = (*story).clone();
    grown.scenes[0].nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::End,
    });
    grown
        .variables
        .push(VarDecl::game("brand_new", Value::Int(9)));
    let Compat::CompatibleWithWarnings(warnings) = check_compat(&grown, &save) else {
        panic!("expected warnings");
    };
    assert!(
        warnings.iter().any(|w| w.contains("variable")),
        "{warnings:?}"
    );
    let mut vm2 = Vm::from_start(Arc::new(grown)).unwrap();
    save.apply_to(&mut vm2).unwrap();
    assert_eq!(vm2.variable("brand_new"), Some(&Value::Int(9)));
    assert!(matches!(vm2.step(), StepResult::Say { .. }));

    // The node the save points to was removed: incompatible, no panic.
    let target = save.vm_state.shown.as_ref().unwrap().node();
    let mut cut = (*story).clone();
    for scene in &mut cut.scenes {
        scene.nodes.retain(|n| n.id != target);
    }
    let Compat::Incompatible(reason) = check_compat(&cut, &save) else {
        panic!("expected incompatible");
    };
    assert!(reason.contains("no longer exists"), "{reason}");

    // A save from a future format version.
    let mut future = save.clone();
    future.meta.format_version = FORMAT_VERSION + 1;
    assert!(matches!(
        check_compat(&story, &future),
        Compat::Incompatible(_)
    ));
}

#[test]
fn a_changed_pending_choice_is_incompatible() {
    let story = demo();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    // Play up to the choice (the demo has exactly one).
    for _ in 0..200 {
        if matches!(vm.step(), StepResult::Choice { .. }) {
            break;
        }
    }
    assert!(matches!(vm.snapshot().pending, Pending::Choice { .. }));
    let save = SaveData::capture(&vm, info(1));
    let mut edited = (*story).clone();
    for node in &mut edited.scenes[0].nodes {
        if let NodeKind::Choice { options } = &mut node.kind {
            options.truncate(1);
        }
    }
    // A hash change (extra node) is needed for the structural check to run.
    edited.scenes[0].nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::End,
    });
    // The visible indexes {0, 1} no longer fit a one-option choice.
    assert!(matches!(
        check_compat(&edited, &save),
        Compat::Incompatible(_)
    ));
}

#[test]
fn slots_save_load_delete_and_list() {
    let story = demo();
    let storage = MemoryStorage::new();
    let mgr = manager(&story, storage.clone());
    let mut vm = Vm::from_start(story.clone()).unwrap();

    let listing = mgr.list().unwrap();
    assert_eq!(listing.len(), 1 + 3 + 20);
    assert!(listing.iter().all(|s| s.state == SlotState::Empty));
    assert_eq!(mgr.latest().unwrap(), None);
    assert!(matches!(
        mgr.load(SlotId::Manual(1)),
        Err(SaveError::NotFound(_))
    ));

    play(&mut vm, 5, 0);
    mgr.save(SlotId::Manual(3), &SaveData::capture(&vm, info(1000)))
        .unwrap();
    play(&mut vm, 5, 0);
    mgr.quicksave(&SaveData::capture(&vm, info(2000))).unwrap();

    let listing = mgr.list().unwrap();
    let occupied: Vec<_> = listing
        .iter()
        .filter(|s| s.state != SlotState::Empty)
        .map(|s| s.slot)
        .collect();
    assert_eq!(occupied, [SlotId::Quick, SlotId::Manual(3)]);
    let (latest, meta) = mgr.latest().unwrap().unwrap();
    assert_eq!((latest, meta.created_at_ms), (SlotId::Quick, 2000));

    let loaded = mgr.load(SlotId::Manual(3)).unwrap();
    assert_eq!(loaded.compat, Compat::Compatible);
    let mut vm2 = Vm::from_start(story.clone()).unwrap();
    loaded.data.apply_to(&mut vm2).unwrap();
    assert_ne!(vm2.snapshot(), vm.snapshot());

    // Out of range slots are refused.
    for bad in [SlotId::Manual(0), SlotId::Manual(21), SlotId::Auto(3)] {
        assert!(matches!(
            mgr.save(bad, &loaded.data),
            Err(SaveError::InvalidSlot(_))
        ));
        assert!(matches!(mgr.load(bad), Err(SaveError::InvalidSlot(_))));
    }

    mgr.delete(SlotId::Manual(3)).unwrap();
    mgr.delete(SlotId::Manual(3)).unwrap();
    assert!(matches!(
        mgr.load(SlotId::Manual(3)),
        Err(SaveError::NotFound(_))
    ));
    // A new manager on the same storage sees the remaining save (restart).
    assert!(manager(&story, storage).load(SlotId::Quick).is_ok());
}

#[test]
fn autosave_rotates() {
    let story = demo();
    let mgr = manager(&story, MemoryStorage::new());
    let mut vm = Vm::from_start(story).unwrap();
    for t in 1..=5u64 {
        play(&mut vm, 2, 0);
        mgr.autosave(&SaveData::capture(&vm, info(t))).unwrap();
    }
    let created = |slot| mgr.load(slot).unwrap().data.meta.created_at_ms;
    assert_eq!(created(SlotId::Auto(0)), 5);
    assert_eq!(created(SlotId::Auto(1)), 4);
    assert_eq!(created(SlotId::Auto(2)), 3);
    assert!(matches!(
        mgr.load(SlotId::Auto(3)),
        Err(SaveError::InvalidSlot(_))
    ));
}

#[test]
fn list_marks_incompatible_and_corrupt_slots_without_panicking() {
    let story = demo();
    let storage = MemoryStorage::new();
    let mgr = manager(&story, storage.clone());
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 6, 0);
    let save = SaveData::capture(&vm, info(10));
    mgr.save(SlotId::Manual(1), &save).unwrap();
    mgr.save(SlotId::Manual(2), &save).unwrap();
    storage.put_raw("saves/manual_003.sav", b"garbage".to_vec());
    let mut truncated = save.encode().unwrap();
    truncated.truncate(truncated.len() / 2);
    storage.put_raw("saves/manual_004.sav", truncated);

    // The story evolves: the node of the save disappears.
    let target = save.vm_state.shown.as_ref().unwrap().node();
    let mut cut = (*story).clone();
    cut.scenes[0].nodes.retain(|n| n.id != target);
    let evolved = SaveManager::new(Box::new(storage), Arc::new(cut), SaveConfig::default());
    let listing = evolved.list().unwrap();
    let state = |n| {
        listing
            .iter()
            .find(|s| s.slot == SlotId::Manual(n))
            .unwrap()
            .state
            .clone()
    };
    assert!(matches!(
        state(1),
        SlotState::Occupied {
            compat: Compat::Incompatible(_),
            ..
        }
    ));
    assert!(matches!(state(3), SlotState::Corrupt(_)));
    assert!(matches!(state(4), SlotState::Corrupt(_)));
    assert!(matches!(
        evolved.load(SlotId::Manual(2)),
        Err(SaveError::Incompatible(_))
    ));
    assert!(matches!(
        evolved.load(SlotId::Manual(3)),
        Err(SaveError::Corrupt(_))
    ));
    assert_eq!(evolved.latest().unwrap(), None);
    // The current story still lists them as fine.
    let ok = mgr.list().unwrap();
    assert!(matches!(
        ok[4].state,
        SlotState::Occupied {
            compat: Compat::Compatible,
            ..
        }
    ));
}

#[test]
fn decoding_arbitrary_damage_never_panics() {
    let story = demo();
    let mut vm = Vm::from_start(story).unwrap();
    play(&mut vm, 8, 0);
    let bytes = SaveData::capture(&vm, info(1)).encode().unwrap();
    // Every truncation.
    for n in 0..bytes.len() {
        let _ = SaveData::decode(&bytes[..n]);
        let _ = SaveData::decode_meta(&bytes[..n]);
    }
    // Random single and multiple byte corruptions.
    let mut rng = Rng::new(99);
    for _ in 0..500 {
        let mut b = bytes.clone();
        for _ in 0..1 + rng.next_u64() % 4 {
            let i = (rng.next_u64() as usize) % b.len();
            b[i] = rng.next_u64() as u8;
        }
        let _ = SaveData::decode(&b);
    }
    assert!(SaveData::decode(b"").is_err());
}

#[test]
fn quota_exceeded_keeps_the_previous_save() {
    let story = demo();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 4, 0);
    let first = SaveData::capture(&vm, info(1));
    let size = first.encode().unwrap().len();
    let storage = MemoryStorage::with_quota(size + 10);
    let mgr = manager(&story, storage.clone());
    mgr.save(SlotId::Manual(1), &first).unwrap();
    play(&mut vm, 20, 0);
    let bigger = SaveData::capture(&vm, info(2));
    assert_eq!(
        mgr.save(SlotId::Manual(1), &bigger),
        Err(SaveError::Storage(StorageError::QuotaExceeded))
    );
    assert_eq!(
        mgr.load(SlotId::Manual(1)).unwrap().data.meta.created_at_ms,
        1
    );
    // A second slot does not fit either, and nothing is half-written.
    assert!(matches!(
        mgr.save(SlotId::Manual(2), &first),
        Err(SaveError::Storage(StorageError::QuotaExceeded))
    ));
    assert!(matches!(
        mgr.load(SlotId::Manual(2)),
        Err(SaveError::NotFound(_))
    ));
    storage.set_fail_writes(true);
    assert!(matches!(
        mgr.save(SlotId::Manual(1), &first),
        Err(SaveError::Storage(StorageError::Io(_)))
    ));
}

#[test]
fn persistent_variables_and_read_lines_live_outside_saves() {
    let story = demo();
    let storage = MemoryStorage::new();
    let mgr = manager(&story, storage.clone());

    // First game: play to the end; the persistent counter goes up.
    let mut vm = Vm::from_start(story.clone()).unwrap();
    assert_eq!(vm.variable("plays"), Some(&Value::Int(0)));
    vm.set_persistent_vars(&mgr.load_persistent().unwrap().vars);
    play(&mut vm, 1000, 0);
    assert!(vm.is_finished());
    assert_eq!(vm.variable("plays"), Some(&Value::Int(1)));
    let persistent = PersistentData::from_vm(&vm);
    assert!(!persistent.read_nodes.is_empty());
    mgr.save_persistent(&persistent).unwrap();

    // A save made mid-game does not contain it, and loading it does not
    // touch the persistent value.
    let mut second = Vm::from_start(story.clone()).unwrap();
    mgr.load_persistent().unwrap().apply_to(&mut second);
    assert_eq!(second.variable("plays"), Some(&Value::Int(1)));
    assert!(second.read_nodes().len() > 10);
    play(&mut second, 5, 0);
    let save = SaveData::capture(&second, info(3));
    assert!(!save.vm_state.vars.contains_key("plays"));
    let mut third = Vm::from_start(story.clone()).unwrap();
    mgr.load_persistent().unwrap().apply_to(&mut third);
    save.apply_to(&mut third).unwrap();
    assert_eq!(third.variable("plays"), Some(&Value::Int(1)));
    // The demo shows a "welcome back" line to returning players.
    let mut text = Vec::new();
    let mut fresh = Vm::from_start(story.clone()).unwrap();
    mgr.load_persistent().unwrap().apply_to(&mut fresh);
    for _ in 0..3 {
        if let StepResult::Say { string_id, .. } = fresh.step() {
            text.push(fresh.text(string_id).unwrap().to_owned());
        }
    }
    assert!(
        text.iter().any(|t| t.starts_with("Welcome back")),
        "{text:?}"
    );

    // Stale entries (deleted node, retyped variable) are ignored.
    let mut stale = persistent.clone();
    stale.read_nodes.insert(NodeId::new());
    stale.vars.insert("plays".into(), Value::Bool(true));
    let mut vm4 = Vm::from_start(story).unwrap();
    stale.apply_to(&mut vm4);
    assert_eq!(vm4.variable("plays"), Some(&Value::Int(0)));
    assert_eq!(vm4.read_nodes().len(), persistent.read_nodes.len());

    // Damaged file: an error, and the default is still obtainable.
    storage.put_raw(PERSISTENT_KEY, b"((( not ron".to_vec());
    assert!(matches!(mgr.load_persistent(), Err(SaveError::Corrupt(_))));
}

#[test]
fn preferences_roundtrip_default_and_sanitize() {
    let story = demo();
    let storage = MemoryStorage::new();
    let mgr = manager(&story, storage.clone());
    assert_eq!(mgr.load_prefs().unwrap(), Preferences::default());

    let prefs = Preferences {
        volumes: BusVolumes {
            master: 0.5,
            music: 0.25,
            sfx: 1.0,
            voice: 0.0,
        },
        text_speed: 80.0,
        auto_delay: 1.0,
        skip_mode: SkipMode::All,
        fullscreen: true,
        language: Some("fr".into()),
    };
    mgr.save_prefs(&prefs).unwrap();
    assert_eq!(mgr.load_prefs().unwrap(), prefs);

    // Old files lack newer fields; hand-edited ones can hold nonsense.
    storage.put_raw(
        PREFS_KEY,
        b"(volumes: (master: 7.0, music: -1.0), text_speed: -3.0, fullscreen: true)".to_vec(),
    );
    let p = mgr.load_prefs().unwrap();
    assert_eq!(p.volumes.master, 1.0);
    assert_eq!(p.volumes.music, 0.0);
    assert_eq!(p.volumes.sfx, 1.0);
    assert_eq!(p.text_speed, 0.0);
    assert_eq!(p.skip_mode, SkipMode::ReadOnly);
    assert!(p.fullscreen);
    storage.put_raw(PREFS_KEY, b"nope".to_vec());
    assert!(matches!(mgr.load_prefs(), Err(SaveError::Corrupt(_))));
}

/// Random sequences of advance / choose / rollback / save / load on the demo.
#[test]
fn random_actions_with_saves_keep_invariants() {
    let story = demo();
    for seed in 0..60u64 {
        let mut rng = Rng::new(seed);
        let storage = MemoryStorage::new();
        let mgr = manager(&story, storage);
        let mut vm = Vm::from_start(story.clone()).unwrap();
        let mut saved = std::collections::HashMap::new();
        for _ in 0..120 {
            match rng.next_u64() % 9 {
                0..=3 => match vm.step() {
                    StepResult::Error(e) => panic!("seed {seed}: {e}"),
                    _ => {
                        vm.take_events();
                    }
                },
                4 => {
                    if let Pending::Choice { visible, .. } = vm.snapshot().pending {
                        vm.choose((rng.next_u64() as usize) % visible.len())
                            .unwrap();
                    }
                }
                5 => {
                    let _ = vm.rollback(1 + (rng.next_u64() % 5) as usize);
                }
                6 => {
                    let _ = vm.roll_forward(1 + (rng.next_u64() % 3) as usize);
                }
                7 => {
                    let slot = SlotId::Manual(1 + (rng.next_u64() % 4) as u32);
                    let mut capture = info(rng.next_u64() % 1_000_000);
                    capture.max_history = Some((rng.next_u64() % 60) as usize);
                    let data = SaveData::capture(&vm, capture);
                    mgr.save(slot, &data).unwrap();
                    saved.insert(slot, vm.snapshot());
                }
                _ => {
                    let slot = SlotId::Manual(1 + (rng.next_u64() % 4) as u32);
                    match mgr.load(slot) {
                        Ok(loaded) => {
                            assert!(loaded.compat.is_loadable());
                            loaded.data.apply_to(&mut vm).unwrap();
                            assert_eq!(vm.snapshot(), saved[&slot], "seed {seed}");
                        }
                        Err(SaveError::NotFound(_)) => assert!(!saved.contains_key(&slot)),
                        Err(e) => panic!("seed {seed}: {e}"),
                    }
                }
            }
            assert!(vm.history_len() <= vm.history_capacity());
            assert!(!vm.snapshot().vars.contains_key("plays"));
        }
    }
}
