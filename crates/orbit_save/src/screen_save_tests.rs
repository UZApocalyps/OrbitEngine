//! Saves and screens opened by the story (UI-14, ADR 0016): format 5 keeps
//! the open screens (and the one the story waits for) with the rollback
//! history; format-4 saves load with no screen open.

use std::sync::Arc;

use orbit_core::{Node, NodeId, NodeKind, Operand, Scene, SceneId, ScreenParam, Story, Value, VarDecl};
use orbit_vm::{Interaction, StepResult, Vm};

use crate::*;

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

fn phone_story() -> Story {
    let mut story = Story::new("phone");
    story.variables = vec![VarDecl::game("called", Value::from(""))];
    let hello = story.strings.add("hello");
    let after = story.strings.add("after");
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "main".into(),
        nodes: vec![
            node(NodeKind::OpenScreen {
                screen: "hud".into(),
                wait: false,
                result: None,
                params: vec![],
            }),
            node(NodeKind::Say {
                speaker: None,
                text_id: hello,
                voice: None,
            }),
            node(NodeKind::OpenScreen {
                screen: "phone".into(),
                wait: true,
                result: Some("called".into()),
                params: vec![ScreenParam {
                    name: "contact".into(),
                    value: Operand::Lit(Value::from("Aiko")),
                }],
            }),
            node(NodeKind::Say {
                speaker: None,
                text_id: after,
                voice: None,
            }),
            node(NodeKind::End),
        ],
    });
    story
}

#[test]
fn a_save_made_while_a_screen_is_open_reopens_it() {
    let story = Arc::new(phone_story());
    assert!(orbit_core::validate(&story).errors.is_empty());
    let mut vm = Vm::from_start(story.clone()).unwrap();
    vm.step();
    assert!(matches!(vm.step(), StepResult::Screen { .. }));
    let save = SaveData::capture(&vm, CaptureInfo::new(1, 1));
    assert_eq!(save.meta.format_version, FORMAT_VERSION);
    let back = SaveData::decode(&save.encode().unwrap()).unwrap();
    assert_eq!(back.vm_state, save.vm_state);

    let mut loaded = Vm::from_start(story.clone()).unwrap();
    assert!(back.apply_to(&mut loaded).unwrap());
    assert_eq!(loaded.open_screens(), vm.open_screens());
    assert_eq!(loaded.open_screens()[1].params[0].1, Value::from("Aiko"));
    assert!(matches!(
        loaded.presentation().interaction,
        Some(Interaction::Screen { ref screen, .. }) if screen == "phone"
    ));
    // The story still waits for the phone, then goes on with its result.
    assert!(matches!(loaded.step(), StepResult::Screen { .. }));
    assert_eq!(loaded.close_screen("phone", Some(Value::from("Aiko"))), Ok(true));
    assert!(matches!(loaded.step(), StepResult::Say { .. }));
    assert_eq!(loaded.variable("called"), Some(&Value::from("Aiko")));
    // Rolling back after the load reopens the phone, result undone.
    loaded.rollback(1).unwrap();
    assert_eq!(loaded.open_screens().len(), 2);
    assert_eq!(loaded.variable("called"), Some(&Value::from("")));
    loaded.rollback(1).unwrap();
    assert_eq!(loaded.open_screens().len(), 1, "only the HUD before the phone");
}

#[test]
fn a_format_4_save_loads_with_no_screen_open() {
    // The frozen format-4 save of the minimal story (written before UI-14).
    let bytes: &[u8] = include_bytes!("../../../examples/fixtures/compat/save_v4/save.orbs");
    let decoded = SaveData::decode(bytes).unwrap();
    assert_eq!(decoded.meta.format_version, 4);
    assert!(decoded.vm_state.open_screens.is_empty());
    assert!(
        decoded
            .history
            .as_ref()
            .is_some_and(|h| h.checkpoints.iter().all(|c| c.state.open_screens.is_empty()))
    );
}

#[test]
fn a_save_from_a_newer_format_is_refused_cleanly() {
    let story = Arc::new(phone_story());
    let vm = Vm::from_start(story).unwrap();
    let mut bytes = SaveData::capture(&vm, CaptureInfo::new(1, 1)).encode().unwrap();
    bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
    assert!(matches!(SaveData::decode(&bytes), Err(SaveError::Incompatible(_))));
}
