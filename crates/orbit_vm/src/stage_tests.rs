//! Background layers and particle emitters in the VM (M5A-07 / M5A-08):
//! declarative state, rollback, restore, seek, scene change.

use std::sync::Arc;

use orbit_core::{
    AssetEntry, AssetId, AssetKind, BgLayer, Emitter, Node, NodeId, NodeKind, ParticlePreset,
    Scene, SceneId, Story,
};

use crate::{PresentationEvent, SeekLimits, SeekOutcome, StepResult, Vm, seek};

struct Fx {
    story: Story,
    image: AssetId,
    nodes: Vec<Node>,
}

impl Fx {
    fn new() -> Self {
        let mut story = Story::new("stage");
        let image = AssetId::new();
        story.assets.assets.push(AssetEntry {
            id: image,
            kind: AssetKind::Image,
            path: "bg.png".into(),
        });
        Self { story, image, nodes: Vec::new() }
    }

    fn push(&mut self, kind: NodeKind) -> NodeId {
        let id = NodeId::new();
        self.nodes.push(Node { id, kind });
        id
    }

    fn say(&mut self, text: &str) -> NodeId {
        let text_id = self.story.strings.add(text);
        self.push(NodeKind::Say { speaker: None, text_id, voice: None })
    }

    fn emitter(&mut self, preset: ParticlePreset) -> NodeId {
        self.push(NodeKind::Particles(Emitter::preset(preset)))
    }

    fn build(mut self) -> Arc<Story> {
        self.story.scenes.push(Scene {
            id: SceneId::new(),
            name: "main".into(),
            nodes: self.nodes,
        });
        Arc::new(self.story)
    }
}

fn particle_count(vm: &Vm) -> usize {
    let mut out = Vec::new();
    for e in vm.stage_fx().emitters {
        e.sample(&mut out);
    }
    out.len()
}

fn say(vm: &mut Vm) {
    match vm.step() {
        StepResult::Say { .. } => {}
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn background_layers_set_the_scene_and_emit_one_event() {
    let mut f = Fx::new();
    let image = f.image;
    f.push(NodeKind::BackgroundLayers {
        layers: vec![BgLayer::new(image, 0.0), BgLayer::new(image, 0.5)],
        transition: None,
    });
    f.say("hi");
    f.push(NodeKind::Background { asset: image, transition: None });
    f.say("plain");
    let mut vm = Vm::from_start(f.build()).unwrap();
    say(&mut vm);
    assert_eq!(vm.scene().bg_layers.len(), 2);
    assert_eq!(vm.scene().background, Some(image));
    let events = vm.take_events();
    assert!(matches!(&events[0], PresentationEvent::BackgroundLayers { layers, .. } if layers.len() == 2));
    // A plain Background replaces the layers.
    say(&mut vm);
    assert!(vm.scene().bg_layers.is_empty());
}

#[test]
fn rollback_restores_the_layers_and_a_scene_event_for_them() {
    let mut f = Fx::new();
    let image = f.image;
    f.push(NodeKind::BackgroundLayers {
        layers: vec![BgLayer::new(image, 0.3)],
        transition: None,
    });
    f.say("a");
    f.push(NodeKind::Background { asset: image, transition: None });
    f.say("b");
    let mut vm = Vm::from_start(f.build()).unwrap();
    say(&mut vm);
    say(&mut vm);
    assert!(vm.scene().bg_layers.is_empty());
    let presentation = vm.rollback(1).expect("rolls back");
    assert_eq!(vm.scene().bg_layers.len(), 1);
    assert!(presentation
        .events
        .iter()
        .any(|e| matches!(e, PresentationEvent::BackgroundLayers { .. })));
}

#[test]
fn an_emitter_runs_until_stopped_then_dies_out() {
    let mut f = Fx::new();
    f.emitter(ParticlePreset::Snow);
    f.say("a");
    f.push(NodeKind::StopParticles { name: "snow".into() });
    f.say("b");
    let mut vm = Vm::from_start(f.build()).unwrap();
    say(&mut vm);
    assert!(particle_count(&vm) > 100, "prewarmed: already snowing");
    vm.advance_clock(30.0);
    let snowing = particle_count(&vm);
    assert!(snowing > 100);
    say(&mut vm);
    // The births stop now: what is alive falls out of the screen.
    let after_stop = particle_count(&vm);
    vm.advance_clock(20.0);
    assert_eq!(particle_count(&vm), 0, "{after_stop} particles lived out");
}

#[test]
fn an_emitter_with_the_same_name_replaces_the_running_one() {
    let mut f = Fx::new();
    f.emitter(ParticlePreset::Rain);
    f.emitter(ParticlePreset::Rain);
    f.say("a");
    let mut vm = Vm::from_start(f.build()).unwrap();
    say(&mut vm);
    assert_eq!(vm.stage_fx().emitters.len(), 1);
    assert_eq!(vm.active_anims().len(), 1);
}

#[test]
fn a_scene_change_stops_the_emitters() {
    let mut f = Fx::new();
    let next = SceneId::new();
    f.emitter(ParticlePreset::Rain);
    f.say("a");
    f.push(NodeKind::JumpScene { scene: next, label: None });
    let mut story = {
        let mut story = f.story;
        story.scenes.push(Scene { id: SceneId::new(), name: "one".into(), nodes: f.nodes });
        story
    };
    let end = NodeId::new();
    let text_id = story.strings.add("b");
    story.scenes.push(Scene {
        id: next,
        name: "two".into(),
        nodes: vec![
            Node { id: end, kind: NodeKind::Say { speaker: None, text_id, voice: None } },
            Node { id: NodeId::new(), kind: NodeKind::End },
        ],
    });
    let mut vm = Vm::from_start(Arc::new(story)).unwrap();
    say(&mut vm);
    assert!(particle_count(&vm) > 0);
    say(&mut vm);
    assert_eq!(particle_count(&vm), 0);
}

#[test]
fn restoring_a_snapshot_resumes_the_same_particles() {
    let mut f = Fx::new();
    f.emitter(ParticlePreset::Sparks);
    f.say("a");
    f.say("b");
    let story = f.build();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    say(&mut vm);
    vm.advance_clock(0.6);
    let snapshot = vm.snapshot();
    let mut a = Vec::new();
    for e in vm.stage_fx().emitters {
        e.sample(&mut a);
    }
    assert!(!a.is_empty());
    let mut other = Vm::from_start(story).unwrap();
    other.restore(snapshot).unwrap();
    let mut b = Vec::new();
    for e in other.stage_fx().emitters {
        e.sample(&mut b);
    }
    assert!(a == b);
}

#[test]
fn seeking_through_an_emitter_is_the_same_as_playing_through_it() {
    let mut f = Fx::new();
    f.emitter(ParticlePreset::Dust);
    f.say("a");
    let target = f.say("b");
    let story = f.build();
    let SeekOutcome::Found(path) = seek(&story, target, SeekLimits::default()) else {
        panic!("target is reachable");
    };
    let mut seeked = Vm::from_start(story.clone()).unwrap();
    seeked.restore(path.state.clone()).unwrap();
    let mut played = Vm::from_start(story).unwrap();
    say(&mut played);
    say(&mut played);
    assert_eq!(seeked.stage_fx().emitters.len(), 1);
    assert_eq!(played.stage_fx().emitters.len(), 1);
}
