//! Saves with parallax layers and particles (M5A-07 / M5A-08): format 6
//! carries the background layers; particles need no extra data (they are
//! recomputed from the running-animation list). Format-5 saves still load.

use std::sync::Arc;

use orbit_core::{AssetKind, BgLayer, Emitter, Node, NodeId, NodeKind, ParticlePreset, Story};
use orbit_vm::{StepResult, Vm};

use crate::*;

const FIXTURE: &str = include_str!("../../../examples/stories/test_minimal.ron");
/// Written by format 5 (before parallax layers existed), frozen:
/// `examples/fixtures/compat/save_v5/save.orbs`, never regenerate.
const SAVE_V5: &[u8] = include_bytes!("../../../examples/fixtures/compat/save_v5/save.orbs");

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

fn minimal() -> Story {
    Story::from_ron(FIXTURE).expect("test_minimal.ron must load")
}

/// The minimal story with parallax layers and a rain emitter in front of
/// its first line, plus the id of the `Particles` node.
fn staged() -> (Arc<Story>, NodeId) {
    let mut story = minimal();
    let image = story
        .assets
        .assets
        .iter()
        .find(|a| a.kind == AssetKind::Image)
        .map(|a| a.id)
        .expect("the fixture has an image");
    let emitter = NodeId::new();
    let (scene, at) = story.locate(story.entry().unwrap()).unwrap();
    let nodes = &mut story.scenes[scene].nodes;
    // The story's own first background becomes the layer stack.
    let bg = nodes
        .iter_mut()
        .find(|n| matches!(n.kind, NodeKind::Background { .. }))
        .expect("the fixture sets a background");
    bg.kind = NodeKind::BackgroundLayers {
        layers: vec![BgLayer::new(image, 0.0), BgLayer { asset: image, depth: 0.6, repeat_x: true }],
        transition: None,
    };
    nodes.insert(
        at,
        Node { id: emitter, kind: NodeKind::Particles(Emitter::preset(ParticlePreset::Rain)) },
    );
    (Arc::new(story), emitter)
}

fn particles(vm: &Vm) -> Vec<orbit_core::Particle> {
    let mut out = Vec::new();
    for e in vm.stage_fx().emitters {
        e.sample(&mut out);
    }
    out
}

#[test]
fn a_format_5_save_loads_with_a_plain_background() {
    assert_eq!(&SAVE_V5[4..8], &5u32.to_le_bytes());
    let story = Arc::new(minimal());
    let decoded = SaveData::decode(SAVE_V5).expect("the format-5 save decodes");
    assert_eq!(decoded.meta.format_version, 5);
    assert!(decoded.vm_state.scene.bg_layers.is_empty());
    assert!(decoded.vm_state.scene.background.is_some());
    assert_eq!(check_compat(&story, &decoded), Compat::Compatible);
    let mut vm = Vm::from_start(story).unwrap();
    assert!(decoded.apply_to(&mut vm).unwrap(), "with its history");
    let again = SaveData::capture(&vm, info(9));
    assert_eq!(again.meta.format_version, FORMAT_VERSION);
    let back = SaveData::decode(&again.encode().unwrap()).unwrap();
    assert_eq!(back.vm_state, again.vm_state);
}

#[test]
fn a_format_5_body_rebuilt_from_current_types_decodes_back() {
    let (story, _) = staged();
    let mut vm = Vm::from_start(story).unwrap();
    play(&mut vm, 3);
    let save = SaveData::capture(&vm, info(7));
    let mut meta = save.meta.clone();
    meta.format_version = 5;
    let meta_bytes = postcard::to_stdvec(&meta).unwrap();
    let body = crate::legacy_v5::encode_body_for_tests(&save.vm_state, save.history.as_ref());
    let mut bytes = b"ORBS".to_vec();
    bytes.extend_from_slice(&5u32.to_le_bytes());
    bytes.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&meta_bytes);
    bytes.extend_from_slice(&body);
    let decoded = SaveData::decode(&bytes).expect("v5 save must decode");
    // Everything but the layers (a v5 save has none) is the same, and the
    // running emitter (an animation entry) survives.
    assert!(decoded.vm_state.scene.bg_layers.is_empty());
    assert_eq!(decoded.vm_state.active_anims, save.vm_state.active_anims);
    assert_eq!(decoded.vm_state.scene.background, save.vm_state.scene.background);
}

#[test]
fn layers_and_particles_survive_a_save_and_a_load_exactly() {
    let (story, emitter) = staged();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    play(&mut vm, 2);
    vm.advance_clock(1.3);
    let save = SaveData::capture(&vm, info(1));
    assert_eq!(save.vm_state.scene.bg_layers.len(), 2);
    assert!(save.vm_state.active_anims.iter().any(|a| a.node == emitter));
    let decoded = SaveData::decode(&save.encode().unwrap()).unwrap();
    assert_eq!(decoded.vm_state, save.vm_state);

    let mut loaded = Vm::from_start(story).unwrap();
    assert!(decoded.apply_to(&mut loaded).unwrap());
    assert_eq!(loaded.snapshot().scene.bg_layers, vm.snapshot().scene.bg_layers);
    let (a, b) = (particles(&vm), particles(&loaded));
    assert!(!a.is_empty());
    assert!(a == b, "the same particles, bit for bit");
    // And they keep matching as time goes on.
    vm.advance_clock(0.4);
    loaded.advance_clock(0.4);
    assert!(particles(&vm) == particles(&loaded));
}

#[test]
fn rolling_back_in_the_middle_of_particles_restores_the_same_frame() {
    let (story, _) = staged();
    let mut vm = Vm::from_start(story).unwrap();
    play(&mut vm, 1);
    vm.advance_clock(0.7);
    play(&mut vm, 1);
    let before = particles(&vm);
    let layers = vm.snapshot().scene.bg_layers.clone();
    assert!(!before.is_empty());
    vm.advance_clock(5.0);
    play(&mut vm, 1);
    assert!(particles(&vm) != before);
    // Back to the second line: clock and animations come from its checkpoint.
    assert!(vm.rollback(1).is_some());
    assert_eq!(vm.snapshot().scene.bg_layers, layers);
    assert!(
        particles(&vm) == before,
        "recomputed from (seed, time), not stored"
    );
}
