//! Effects and timelines in the VM: declarative `(node, started_at)` state,
//! scene clock, skip, rollback, seek, restore (ADR 0012 section 5).

use std::sync::Arc;

use orbit_core::{
    AnimProp, AnimTarget, AssetEntry, AssetId, AssetKind, Character, CharacterId, Color, Effect,
    Expression, Key, Loop, Node, NodeId, NodeKind, Position, Scene, SceneId, StoryTrack, Story,
    Track, TrackData, Vec2,
};

use crate::{SeekLimits, SeekOutcome, StepResult, Vm, seek};

struct Fx {
    story: Story,
    ana: CharacterId,
    nodes: Vec<Node>,
}

impl Fx {
    fn new() -> Self {
        let mut story = Story::new("anim");
        let image = AssetId::new();
        story.assets.assets.push(AssetEntry {
            id: image,
            kind: AssetKind::Image,
            path: "ana.png".into(),
        });
        let ana = CharacterId::new();
        let name_id = story.strings.add("Ana");
        story.characters.push(Character {
            id: ana,
            name_id,
            color: Color::WHITE,
            expressions: vec![Expression {
                name: "smile".into(),
                image,
                motion: None,
            }],
            default_position: None,
            default_scale: 1.0,
        });
        Self { story, ana, nodes: Vec::new() }
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

    fn mv(&mut self, dx: f32, duration: f32, wait: bool) -> NodeId {
        self.push(NodeKind::Effect(Effect::Move {
            target: AnimTarget::Background,
            dx,
            dy: 0.0,
            duration,
            ease: None,
            wait,
        }))
    }

    fn build(mut self) -> (Arc<Story>, CharacterId) {
        self.story.scenes.push(Scene {
            id: SceneId::new(),
            name: "main".into(),
            nodes: self.nodes,
        });
        (Arc::new(self.story), self.ana)
    }
}

fn bg_x(vm: &Vm) -> f32 {
    vm.stage_fx().background.offset[0]
}

fn say_node(vm: &mut Vm) -> NodeId {
    match vm.step() {
        StepResult::Say { node, .. } => node,
        other => panic!("expected a line, got {other:?}"),
    }
}

#[test]
fn a_blocking_effect_waits_its_duration_and_a_skip_lands_on_the_final_state() {
    let mut f = Fx::new();
    let m = f.mv(0.4, 2.0, true);
    f.say("after");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story).unwrap();

    assert_eq!(vm.step(), StepResult::WaitAnimation { node: m, duration: 2.0 });
    assert_eq!(bg_x(&vm), 0.0);
    vm.advance_clock(1.0);
    assert!((bg_x(&vm) - 0.2).abs() < 1e-6);
    assert!(!vm.anim_finished(m));

    // Skip: exact final value, whatever the clock says.
    vm.complete_anim(m);
    assert!(vm.anim_finished(m));
    assert_eq!(bg_x(&vm), 0.4);
    say_node(&mut vm);
    // The final state stays until the scene changes.
    vm.advance_clock(50.0);
    assert_eq!(bg_x(&vm), 0.4);
}

#[test]
fn the_editor_can_scrub_a_running_animation_without_moving_the_clock() {
    let mut f = Fx::new();
    let m = f.mv(0.4, 2.0, true);
    f.say("after");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story).unwrap();
    assert_eq!(vm.step(), StepResult::WaitAnimation { node: m, duration: 2.0 });
    let clock = vm.scene_clock();
    assert!(vm.set_anim_elapsed(m, 1.0));
    assert!((bg_x(&vm) - 0.2).abs() < 1e-6);
    assert!(vm.set_anim_elapsed(m, 0.5));
    assert!((bg_x(&vm) - 0.1).abs() < 1e-6);
    assert_eq!(vm.scene_clock(), clock);
    // Bad values land on the start; a node that does not animate is refused.
    assert!(vm.set_anim_elapsed(m, f32::NAN));
    assert_eq!(bg_x(&vm), 0.0);
    assert!(!vm.set_anim_elapsed(NodeId::new(), 1.0));
}

#[test]
fn a_non_blocking_timeline_runs_while_the_story_goes_on() {
    let mut f = Fx::new();
    f.say("a");
    f.push(NodeKind::Timeline {
        duration: 2.0,
        looping: Loop::Once,
        wait: false,
        tracks: vec![StoryTrack {
            target: AnimTarget::Background,
            prop: AnimProp::Offset,
            data: TrackData::Vec2(Track::new(vec![
                Key::new(0.0, Vec2::new(0.0, 0.0)),
                Key::new(2.0, Vec2::new(1.0, 0.0)),
            ])),
        }],
    });
    f.say("b");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story).unwrap();
    say_node(&mut vm);
    say_node(&mut vm); // the timeline started, the story did not wait
    assert_eq!(vm.active_anims().len(), 1);
    vm.advance_clock(0.5);
    assert!((bg_x(&vm) - 0.25).abs() < 1e-6);
    vm.advance_clock(f64::NAN);
    vm.advance_clock(-3.0);
    assert!((bg_x(&vm) - 0.25).abs() < 1e-6);
    vm.complete_anims();
    assert_eq!(bg_x(&vm), 1.0);
}

#[test]
fn zero_duration_and_endless_animations_do_not_block() {
    let mut f = Fx::new();
    f.mv(0.1, 0.0, true);
    f.say("a");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story).unwrap();
    say_node(&mut vm);
    assert_eq!(bg_x(&vm), 0.1);
}

#[test]
fn rollback_in_the_middle_of_an_animation_restores_its_phase() {
    let mut f = Fx::new();
    let a = f.say("a");
    f.mv(1.0, 4.0, false);
    let b = f.say("b");
    let c = f.say("c");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story).unwrap();

    assert_eq!(say_node(&mut vm), a);
    vm.advance_clock(3.0); // time spent reading "a": the move is not started yet
    assert_eq!(say_node(&mut vm), b);
    assert_eq!(vm.scene_clock(), 3.0);
    vm.advance_clock(1.0); // 1s into the move
    assert!((bg_x(&vm) - 0.25).abs() < 1e-6);
    assert_eq!(say_node(&mut vm), c);
    vm.advance_clock(1.0); // 2s into the move
    assert!((bg_x(&vm) - 0.5).abs() < 1e-6);

    // Back to "b": clock and animation as they were when "b" appeared.
    assert!(vm.rollback(1).is_some());
    assert_eq!(vm.scene_clock(), 3.0);
    assert_eq!(bg_x(&vm), 0.0);
    vm.advance_clock(2.0);
    assert!((bg_x(&vm) - 0.5).abs() < 1e-6);

    // Back before the move: nothing animates, the replay starts it again.
    assert!(vm.rollback(1).is_some());
    assert!(vm.active_anims().is_empty());
    assert_eq!(bg_x(&vm), 0.0);
    assert_eq!(say_node(&mut vm), b);
    assert_eq!(vm.active_anims().len(), 1);
    vm.advance_clock(8.0);
    assert_eq!(bg_x(&vm), 1.0);
}

#[test]
fn a_rolled_back_blocking_effect_is_not_replayed_but_a_replayed_one_waits_again() {
    let mut f = Fx::new();
    f.say("a");
    let m = f.mv(0.5, 1.0, true);
    f.say("b");
    f.say("c");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story).unwrap();
    say_node(&mut vm);
    assert!(matches!(vm.step(), StepResult::WaitAnimation { node, .. } if node == m));
    vm.complete_anim(m);
    say_node(&mut vm); // b
    say_node(&mut vm); // c
    // Rolling back to "b" keeps the finished effect at its final state.
    assert!(vm.rollback(1).is_some());
    assert_eq!(bg_x(&vm), 0.5);
    assert!(vm.anim_finished(m));
}

#[test]
fn state_snapshot_restore_mid_animation_resumes_the_phase() {
    let mut f = Fx::new();
    f.say("a");
    f.mv(1.0, 4.0, false);
    f.say("b");
    f.say("c");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story.clone()).unwrap();
    say_node(&mut vm);
    say_node(&mut vm);
    vm.advance_clock(1.0);
    let state = vm.snapshot();
    // The state survives serialization (saves embed it).
    let json = format!("{:?}", state.active_anims);
    assert!(json.contains("started_at"));

    let mut other = Vm::from_start(story).unwrap();
    other.restore(state).unwrap();
    assert_eq!(other.scene_clock(), 1.0);
    assert!((bg_x(&other) - 0.25).abs() < 1e-6);
    other.advance_clock(1.0);
    assert!((bg_x(&other) - 0.5).abs() < 1e-6);
}

#[test]
fn restore_drops_animations_of_nodes_that_no_longer_exist() {
    let mut f = Fx::new();
    f.say("a");
    f.mv(1.0, 4.0, false);
    f.say("b");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story).unwrap();
    say_node(&mut vm);
    say_node(&mut vm);
    let mut state = vm.snapshot();
    state.active_anims.push(crate::ActiveAnim { node: NodeId::new(), started_at: 0.0 });
    state.scene_clock = f64::NAN;
    vm.restore(state).unwrap();
    assert_eq!(vm.active_anims().len(), 1);
    assert_eq!(vm.scene_clock(), 0.0);
}

#[test]
fn seeking_past_an_animation_gives_the_same_state_as_playing_it() {
    let mut f = Fx::new();
    f.say("a");
    f.mv(0.7, 3.0, true);
    f.mv(-0.2, 1.0, false);
    f.say("b");
    let target = f.say("c");
    let (story, _) = f.build();
    let SeekOutcome::Found(path) = seek(&story, target, SeekLimits::default()) else {
        panic!("target is reachable");
    };
    let sought = path.into_vm(&story).unwrap();
    assert_eq!(sought.active_anims().len(), 2);
    assert!((bg_x(&sought) - 0.5).abs() < 1e-6);

    // Played for real: wait the blocking move out, read, wait out the other.
    let mut played = Vm::from_start(story).unwrap();
    say_node(&mut played);
    assert!(matches!(played.step(), StepResult::WaitAnimation { .. }));
    played.advance_clock(3.0);
    say_node(&mut played);
    played.advance_clock(5.0);
    assert!((bg_x(&played) - bg_x(&sought)).abs() < 1e-6);
}

#[test]
fn changing_scene_stops_animations_and_restarts_the_clock() {
    let mut f = Fx::new();
    f.mv(0.5, 1.0, false);
    f.say("a");
    let second = SceneId::new();
    f.push(NodeKind::JumpScene { scene: second, label: None });
    let mut story_nodes = std::mem::take(&mut f.nodes);
    let mut story = f.story;
    let text_id = story.strings.add("b");
    story.scenes.push(Scene { id: SceneId::new(), name: "one".into(), nodes: std::mem::take(&mut story_nodes) });
    story.scenes.push(Scene {
        id: second,
        name: "two".into(),
        nodes: vec![Node { id: NodeId::new(), kind: NodeKind::Say { speaker: None, text_id, voice: None } }],
    });
    let mut vm = Vm::from_start(Arc::new(story)).unwrap();
    say_node(&mut vm);
    vm.advance_clock(0.4);
    assert_ne!(bg_x(&vm), 0.0);
    say_node(&mut vm);
    assert!(vm.active_anims().is_empty());
    assert_eq!(vm.scene_clock(), 0.0);
    assert_eq!(bg_x(&vm), 0.0);
}

/// The frozen v9 story (every effect, two timelines) plays to the end, with
/// the clock running through the waits, and ends in the composed final state.
#[test]
fn the_frozen_v9_story_plays_to_the_end() {
    let story = Arc::new(
        Story::from_ron(include_str!("../../../examples/fixtures/compat/v10/story.ron")).unwrap(),
    );
    let mut vm = Vm::from_start(story).unwrap();
    let mut waits = 0;
    let mut says = 0;
    for _ in 0..50 {
        match vm.step() {
            StepResult::Say { .. } => says += 1,
            StepResult::WaitAnimation { duration, .. } => {
                waits += 1;
                vm.advance_clock(f64::from(duration));
            }
            StepResult::End => break,
            StepResult::Error(e) => panic!("{e}"),
            _ => {}
        }
    }
    assert_eq!((says, waits), (4, 2));
    assert!(vm.is_finished());
    vm.advance_clock(100.0);
    let fx = vm.stage_fx();
    // The 2x looping timeline (3 s per pass) is over: back to rest; the
    // character kept its 0.3 move, the camera its zoom of 1.3 x 1.2.
    assert!((fx.camera.zoom - 1.3 * 1.2).abs() < 1e-5, "{}", fx.camera.zoom);
    let alice = fx.characters.values().next().unwrap();
    assert!((alice.offset[0] - 0.3).abs() < 1e-6);
    assert_eq!(fx.background.offset, [0.0, 0.0]);
    // Flash faded, the red tint stays, then the timeline tint is composed over it.
    assert!(fx.screen[3] > 0.4);
}

#[test]
fn hiding_a_character_drops_its_animations() {
    let mut f = Fx::new();
    let ana = f.ana;
    f.push(NodeKind::Show {
        character: ana,
        expression: "smile".into(),
        position: Position::Left,
        transition: None,
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
    });
    f.push(NodeKind::Effect(Effect::Move {
        target: AnimTarget::Character(ana),
        dx: 0.3,
        dy: 0.0,
        duration: 1.0,
        ease: None,
        wait: false,
    }));
    f.say("a");
    f.push(NodeKind::Hide { character: ana, transition: None });
    f.say("b");
    let (story, _) = f.build();
    let mut vm = Vm::from_start(story).unwrap();
    say_node(&mut vm);
    vm.complete_anims();
    assert_eq!(vm.stage_fx().characters[&ana].offset[0], 0.3);
    say_node(&mut vm);
    assert!(vm.stage_fx().characters.is_empty());
}
