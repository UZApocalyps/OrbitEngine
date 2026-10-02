use orbit_core::{
    AssetEntry, AssetId, AssetKind, BinOp, Character, CharacterId, ChoiceOption, Color, Expr,
    Expression, Node, NodeId, NodeKind, Position, PositionDef, Scene, SceneId, Story, Transition,
    Value, VarDecl,
};

use crate::{
    ChoiceView, MAX_CALL_DEPTH, MAX_NODES_PER_STEP, Pending, PresentationEvent, StepResult, Vm,
    VmError, VmState,
};

/// Small story builder for tests.
struct B {
    story: Story,
    nodes: Vec<Node>,
}

impl B {
    fn new() -> Self {
        Self {
            story: Story::new("test"),
            nodes: Vec::new(),
        }
    }

    fn at(&mut self, id: NodeId, kind: NodeKind) -> NodeId {
        self.nodes.push(Node { id, kind });
        id
    }

    fn push(&mut self, kind: NodeKind) -> NodeId {
        self.at(NodeId::new(), kind)
    }

    fn say(&mut self, speaker: Option<CharacterId>, text: &str) -> NodeId {
        let text_id = self.story.strings.add(text);
        self.push(NodeKind::Say { speaker, text_id })
    }

    fn label(&mut self, id: NodeId, name: &str) -> NodeId {
        self.at(id, NodeKind::Label { name: name.into() })
    }

    fn asset(&mut self, kind: AssetKind, path: &str) -> AssetId {
        let id = AssetId::new();
        self.story.assets.assets.push(AssetEntry {
            id,
            kind,
            path: path.into(),
        });
        id
    }

    fn character(&mut self, name: &str, expressions: &[(&str, AssetId)]) -> CharacterId {
        let id = CharacterId::new();
        let name_id = self.story.strings.add(name);
        self.story.characters.push(Character {
            default_scale: 1.0,
            id,
            name_id,
            color: Color::WHITE,
            expressions: expressions
                .iter()
                .map(|(n, image)| Expression {
                    name: (*n).into(),
                    image: *image,
                    motion: None,
                })
                .collect(),
            default_position: None,
        });
        id
    }

    fn option(&mut self, text: &str, target: NodeId, condition: Option<Expr>) -> ChoiceOption {
        ChoiceOption {
            text_id: self.story.strings.add(text),
            target,
            condition,
        }
    }

    fn build(mut self) -> Story {
        self.story.scenes.push(Scene {
            id: SceneId::new(),
            name: "main".into(),
            nodes: self.nodes,
        });
        self.story
    }
}

/// Plays the story, answering choices with `pick(n_options)`, and returns a
/// textual trace (lines, choices, waits, events, end).
fn trace(vm: &mut Vm, pick: &mut dyn FnMut(usize) -> usize) -> Vec<String> {
    let mut out = Vec::new();
    for _ in 0..1000 {
        let result = tick(vm, pick, &mut out);
        if matches!(result, StepResult::End | StepResult::Error(_)) {
            return out;
        }
    }
    panic!("story did not end");
}

/// One `step()` (plus the answer to a choice), appending to the trace.
fn tick(vm: &mut Vm, pick: &mut dyn FnMut(usize) -> usize, out: &mut Vec<String>) -> StepResult {
    let result = vm.step();
    for e in vm.take_events() {
        out.push(format!("event {e:?}"));
    }
    match &result {
        StepResult::Say {
            speaker,
            string_id,
            already_read,
            ..
        } => {
            let who = speaker
                .and_then(|c| vm.character_name(c))
                .unwrap_or("-")
                .to_owned();
            out.push(format!(
                "say {who}: {}{}",
                vm.text(*string_id).unwrap(),
                if *already_read { " (read)" } else { "" }
            ));
        }
        StepResult::Choice { options, .. } => {
            let texts: Vec<_> = options
                .iter()
                .map(|o| vm.text(o.string_id).unwrap().to_owned())
                .collect();
            out.push(format!("choice {texts:?}"));
            let i = pick(options.len());
            vm.choose(i).unwrap();
            out.push(format!("chose {i}"));
        }
        StepResult::WaitTime(s) => out.push(format!("wait {s}")),
        StepResult::WaitTransition { duration } => out.push(format!("transition {duration}")),
        StepResult::End => out.push("end".into()),
        StepResult::Error(e) => out.push(format!("error {e}")),
    }
    result
}

fn first(_: usize) -> usize {
    0
}

fn vm_for(story: Story) -> Vm {
    Vm::from_start(story).unwrap()
}

#[test]
fn plays_a_linear_story_to_the_end() {
    let mut b = B::new();
    let ana = b.character("Ana", &[]);
    b.say(Some(ana), "Hello");
    b.say(None, "World");
    let mut vm = vm_for(b.build());
    assert_eq!(
        trace(&mut vm, &mut first),
        ["say Ana: Hello", "say -: World", "end"]
    );
    assert!(vm.is_finished());
    // End is sticky.
    assert_eq!(vm.step(), StepResult::End);
}

#[test]
fn say_result_carries_ids_and_read_flag() {
    let mut b = B::new();
    let ana = b.character("Ana", &[]);
    let say = b.say(Some(ana), "Hi");
    let mut vm = vm_for(b.build());
    let snap = vm.snapshot();
    let StepResult::Say {
        node,
        speaker,
        already_read,
        ..
    } = vm.step()
    else {
        panic!("expected Say");
    };
    assert_eq!((node, speaker, already_read), (say, Some(ana), false));
    assert!(vm.is_read(say));
    // Rolling back and replaying the line marks it as already read.
    vm.restore(snap).unwrap();
    let StepResult::Say { already_read, .. } = vm.step() else {
        panic!("expected Say");
    };
    assert!(already_read);
    assert_eq!(vm.read_nodes().len(), 1);
}

#[test]
fn choices_branch_and_rejoin() {
    let mut b = B::new();
    let (forest, river, join) = (NodeId::new(), NodeId::new(), NodeId::new());
    b.say(None, "Which way?");
    let o1 = b.option("Forest", forest, None);
    let o2 = b.option("River", river, None);
    b.push(NodeKind::Choice {
        options: vec![o1, o2],
    });
    b.label(forest, "forest");
    b.say(None, "Trees.");
    b.push(NodeKind::Jump { target: join });
    b.label(river, "river");
    b.say(None, "Water.");
    b.label(join, "join");
    b.say(None, "Village.");
    let story = b.build();

    let mut vm = vm_for(story.clone());
    assert_eq!(
        trace(&mut vm, &mut |_| 0),
        [
            "say -: Which way?",
            "choice [\"Forest\", \"River\"]",
            "chose 0",
            "say -: Trees.",
            "say -: Village.",
            "end"
        ]
    );
    let mut vm = vm_for(story);
    assert_eq!(
        trace(&mut vm, &mut |_| 1),
        [
            "say -: Which way?",
            "choice [\"Forest\", \"River\"]",
            "chose 1",
            "say -: Water.",
            "say -: Village.",
            "end"
        ]
    );
}

#[test]
fn choice_is_idempotent_until_answered_and_validates_the_index() {
    let mut b = B::new();
    let end = NodeId::new();
    let o = b.option("Only", end, None);
    b.push(NodeKind::Choice { options: vec![o] });
    b.label(end, "end");
    let mut vm = vm_for(b.build());

    assert_eq!(vm.choose(0), Err(VmError::NotAwaitingChoice));
    let first = vm.step();
    assert!(matches!(first, StepResult::Choice { .. }));
    assert_eq!(vm.step(), first);
    assert_eq!(
        vm.choose(1),
        Err(VmError::InvalidChoice {
            index: 1,
            available: 1
        })
    );
    // A failed choose leaves the choice pending.
    assert_eq!(vm.step(), first);
    vm.choose(0).unwrap();
    assert_eq!(vm.choose(0), Err(VmError::NotAwaitingChoice));
    assert_eq!(vm.step(), StepResult::End);
}

#[test]
fn conditional_options_are_filtered_and_indexed_among_visible() {
    let mut b = B::new();
    b.story
        .variables
        .push(VarDecl::game("key", Value::Bool(false)));
    let (a, c) = (NodeId::new(), NodeId::new());
    let o1 = b.option("Locked", a, Some(Expr::var("key")));
    let o2 = b.option("Open", c, None);
    b.push(NodeKind::Choice {
        options: vec![o1, o2],
    });
    b.label(a, "a");
    b.say(None, "A");
    b.push(NodeKind::End);
    b.label(c, "c");
    b.say(None, "C");
    let mut vm = vm_for(b.build());

    let StepResult::Choice { options, .. } = vm.step() else {
        panic!("expected Choice");
    };
    assert_eq!(options.len(), 1);
    assert_eq!(vm.text(options[0].string_id), Some("Open"));
    vm.choose(0).unwrap();
    // Visible index 0 is the *second* option.
    let StepResult::Say { string_id, .. } = vm.step() else {
        panic!("expected Say");
    };
    assert_eq!(vm.text(string_id), Some("C"));
}

#[test]
fn choice_with_no_visible_option_is_an_error() {
    let mut b = B::new();
    let end = NodeId::new();
    let o = b.option("Never", end, Some(Expr::lit(false)));
    let cid = b.push(NodeKind::Choice { options: vec![o] });
    b.label(end, "end");
    let mut vm = vm_for(b.build());
    assert_eq!(vm.step(), StepResult::Error(VmError::NoVisibleChoice(cid)));
    // Errors are sticky until restore().
    assert_eq!(vm.step(), StepResult::Error(VmError::NoVisibleChoice(cid)));
}

#[test]
fn variables_and_conditions() {
    let mut b = B::new();
    let (hi, lo, end) = (NodeId::new(), NodeId::new(), NodeId::new());
    b.story
        .variables
        .push(VarDecl::game("score", Value::Int(1)));
    b.push(NodeKind::SetVar {
        var: "score".into(),
        value: Expr::binary(BinOp::Add, Expr::var("score"), Expr::lit(2i64)),
    });
    b.push(NodeKind::If {
        condition: Expr::binary(BinOp::Gt, Expr::var("score"), Expr::lit(2i64)),
        then_target: hi,
        else_target: Some(lo),
    });
    b.label(hi, "hi");
    b.say(None, "high");
    b.push(NodeKind::Jump { target: end });
    b.label(lo, "lo");
    b.say(None, "low");
    b.label(end, "end");
    // `If` without else falls through.
    b.push(NodeKind::If {
        condition: Expr::lit(false),
        then_target: lo,
        else_target: None,
    });
    b.say(None, "done");
    let mut vm = vm_for(b.build());
    assert_eq!(
        trace(&mut vm, &mut first),
        ["say -: high", "say -: done", "end"]
    );
    assert_eq!(vm.variable("score"), Some(&Value::Int(3)));
}

#[test]
fn non_bool_condition_and_unknown_variable_are_errors() {
    let mut b = B::new();
    let l = NodeId::new();
    let cond = b.push(NodeKind::If {
        condition: Expr::lit(1i64),
        then_target: l,
        else_target: None,
    });
    b.label(l, "l");
    let mut vm = vm_for(b.build());
    assert_eq!(
        vm.step(),
        StepResult::Error(VmError::ConditionNotBool(cond))
    );

    let mut b = B::new();
    let l = NodeId::new();
    let set = b.push(NodeKind::SetVar {
        var: "x".into(),
        value: Expr::var("missing"),
    });
    b.label(l, "l");
    let mut vm = vm_for(b.build());
    assert!(matches!(
        vm.step(),
        StepResult::Error(VmError::Eval { node, .. }) if node == set
    ));
}

#[test]
fn call_and_return_nest() {
    let mut b = B::new();
    let (outer, inner) = (NodeId::new(), NodeId::new());
    b.say(None, "main 1");
    b.push(NodeKind::Call { label: outer });
    b.say(None, "main 2");
    b.push(NodeKind::End);
    b.label(outer, "outer");
    b.say(None, "outer 1");
    b.push(NodeKind::Call { label: inner });
    b.say(None, "outer 2");
    b.push(NodeKind::Return);
    b.label(inner, "inner");
    b.say(None, "inner");
    b.push(NodeKind::Return);
    let mut vm = vm_for(b.build());
    assert_eq!(
        trace(&mut vm, &mut first),
        [
            "say -: main 1",
            "say -: outer 1",
            "say -: inner",
            "say -: outer 2",
            "say -: main 2",
            "end"
        ]
    );
    assert_eq!(vm.call_depth(), 0);
}

#[test]
fn runaway_recursion_hits_the_depth_limit() {
    let mut b = B::new();
    let routine = NodeId::new();
    b.push(NodeKind::Jump { target: routine });
    b.label(routine, "r");
    b.say(None, "x");
    b.push(NodeKind::Call { label: routine });
    let mut vm = vm_for(b.build());
    // r -> Say, then Call r again (continuation None): infinite recursion
    // guarded by the call depth limit.
    let mut out = Vec::new();
    let mut result = StepResult::End;
    for _ in 0..(MAX_CALL_DEPTH + 10) {
        result = tick(&mut vm, &mut first, &mut out);
        if matches!(result, StepResult::Error(_)) {
            break;
        }
    }
    assert_eq!(
        result,
        StepResult::Error(VmError::StackOverflow(MAX_CALL_DEPTH))
    );
}

#[test]
fn return_without_call_and_missing_label_are_errors() {
    let mut b = B::new();
    let ret = b.push(NodeKind::Return);
    let mut vm = vm_for(b.build());
    assert_eq!(
        vm.step(),
        StepResult::Error(VmError::ReturnWithoutCall(ret))
    );

    let ghost = NodeId::new();
    let mut b = B::new();
    b.push(NodeKind::Call { label: ghost });
    let mut vm = vm_for(b.build());
    assert_eq!(vm.step(), StepResult::Error(VmError::UnknownNode(ghost)));

    let mut b = B::new();
    b.push(NodeKind::Jump { target: ghost });
    let mut vm = vm_for(b.build());
    assert_eq!(vm.step(), StepResult::Error(VmError::UnknownNode(ghost)));

    // Unknown choice target: choose() reports it and keeps the choice pending.
    let mut b = B::new();
    let o = b.option("x", ghost, None);
    b.push(NodeKind::Choice { options: vec![o] });
    let mut vm = vm_for(b.build());
    assert!(matches!(vm.step(), StepResult::Choice { .. }));
    assert_eq!(vm.choose(0), Err(VmError::UnknownNode(ghost)));
    assert!(matches!(vm.step(), StepResult::Choice { .. }));
}

#[test]
fn unknown_entry_and_empty_story_are_errors() {
    let story = B::new().build();
    assert!(matches!(
        Vm::from_start(story.clone()).err(),
        Some(VmError::EmptyStory)
    ));
    let ghost = NodeId::new();
    assert!(matches!(
        Vm::new(story, ghost).err(),
        Some(VmError::UnknownNode(id)) if id == ghost
    ));
}

#[test]
fn infinite_loop_without_blocking_node_is_stopped() {
    let mut b = B::new();
    let l = NodeId::new();
    b.label(l, "loop");
    b.push(NodeKind::Jump { target: l });
    let mut vm = vm_for(b.build());
    assert_eq!(
        vm.step(),
        StepResult::Error(VmError::StepLimit(MAX_NODES_PER_STEP))
    );
}

#[test]
fn presentation_events_and_scene_state() {
    let mut b = B::new();
    let bg = b.asset(AssetKind::Image, "bg.png");
    let img_a = b.asset(AssetKind::Image, "a_neutral.png");
    let img_b = b.asset(AssetKind::Image, "a_happy.png");
    let music = b.asset(AssetKind::Music, "m.ogg");
    let sfx = b.asset(AssetKind::Sound, "s.wav");
    let ana = b.character("Ana", &[("neutral", img_a), ("happy", img_b)]);
    b.story.positions.push(PositionDef {
        name: "far".into(),
        x: 0.1,
        y: 0.9,
    });
    let fade = Transition::fade(Color::BLACK, 1.0);
    b.push(NodeKind::Background {
        asset: bg,
        transition: Some(fade.clone()),
    });
    b.push(NodeKind::PlayMusic {
        asset: music,
        fade_in: 2.0,
        looping: true,
    });
    b.push(NodeKind::Show {
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
        character: ana,
        expression: "neutral".into(),
        position: Position::Left,
        transition: Some(Transition::dissolve(0.0)), // zero duration: no wait
    });
    b.push(NodeKind::PlaySound { asset: sfx });
    b.say(Some(ana), "Hi");
    b.push(NodeKind::Show {
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
        character: ana,
        expression: "happy".into(),
        position: Position::Custom("far".into()),
        transition: Some(Transition::dissolve(0.5)),
    });
    b.push(NodeKind::Wait { seconds: 1.5 });
    b.push(NodeKind::Hide {
        character: ana,
        transition: None,
    });
    // Hiding someone who is not on stage is a silent no-op.
    b.push(NodeKind::Hide {
        character: ana,
        transition: Some(fade.clone()),
    });
    b.push(NodeKind::StopMusic { fade_out: 1.0 });
    let mut vm = vm_for(b.build());

    // Background with a fade blocks.
    assert_eq!(vm.step(), StepResult::WaitTransition { duration: 1.0 });
    assert_eq!(
        vm.take_events(),
        [PresentationEvent::Background {
            asset: bg,
            transition: Some(fade)
        }]
    );
    assert_eq!(vm.scene().background, Some(bg));

    // Music, zero-length show, sound, then the line.
    assert!(matches!(vm.step(), StepResult::Say { .. }));
    let events = vm.take_events();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0],
        PresentationEvent::PlayMusic {
            asset: music,
            fade_in: 2.0,
            looping: true
        }
    );
    assert!(matches!(
        &events[1],
        PresentationEvent::Show { image, x, y, .. } if *image == img_a && (*x, *y) == (0.25, 1.0)
    ));
    assert_eq!(events[2], PresentationEvent::PlaySound { asset: sfx });
    assert_eq!(vm.scene().music.as_ref().map(|m| m.asset), Some(music));
    assert_eq!(vm.scene().characters.len(), 1);

    // Re-showing updates in place.
    assert_eq!(vm.step(), StepResult::WaitTransition { duration: 0.5 });
    assert!(matches!(
        &vm.take_events()[0],
        PresentationEvent::Show { image, x, y, .. } if *image == img_b && (*x, *y) == (0.1, 0.9)
    ));
    let on_stage = vm.scene().character(ana).unwrap();
    assert_eq!(on_stage.expression, "happy");
    assert_eq!(on_stage.position, Position::Custom("far".into()));
    assert_eq!(vm.scene().characters.len(), 1);

    assert_eq!(vm.step(), StepResult::WaitTime(1.5));
    assert_eq!(vm.step(), StepResult::End);
    let events = vm.take_events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(matches!(events[0], PresentationEvent::Hide { .. }));
    assert_eq!(events[1], PresentationEvent::StopMusic { fade_out: 1.0 });
    assert!(vm.scene().characters.is_empty());
    assert_eq!(vm.scene().music, None);
    assert_eq!(vm.scene().background, Some(bg));
}

#[test]
fn show_with_bad_references_is_an_error() {
    let mut b = B::new();
    let img = b.asset(AssetKind::Image, "a.png");
    let ana = b.character("Ana", &[("neutral", img)]);
    let ghost = CharacterId::new();
    let n1 = b.push(NodeKind::Show {
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
        character: ghost,
        expression: "x".into(),
        position: Position::Left,
        transition: None,
    });
    let story = b.build();
    assert_eq!(
        vm_for(story.clone()).step(),
        StepResult::Error(VmError::UnknownCharacter {
            node: n1,
            character: ghost
        })
    );

    for (expression, position) in [
        ("sad", Position::Left),
        ("neutral", Position::Custom("nowhere".into())),
    ] {
        let mut b = B::new();
        let img = b.asset(AssetKind::Image, "a.png");
        let _ = ana;
        let ana2 = b.character("Ana", &[("neutral", img)]);
        b.push(NodeKind::Show {
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
            character: ana2,
            expression: expression.into(),
            position,
            transition: None,
        });
        assert!(matches!(
            vm_for(b.build()).step(),
            StepResult::Error(VmError::UnknownExpression { .. } | VmError::UnknownPosition { .. })
        ));
    }
}

#[test]
fn restore_rejects_foreign_state() {
    let mut b = B::new();
    b.say(None, "x");
    let mut vm = vm_for(b.build());
    let mut state = vm.snapshot();
    let ghost = NodeId::new();
    state.pc = Some(ghost);
    assert_eq!(vm.restore(state.clone()), Err(VmError::UnknownNode(ghost)));
    state.pc = vm.snapshot().pc;
    state.pending = Pending::Choice {
        node: vm.snapshot().pc.unwrap(), // a Say, not a Choice
        visible: vec![0],
    };
    assert!(vm.restore(state).is_err());
    // Untouched.
    assert!(matches!(vm.step(), StepResult::Say { .. }));
}

/// A story exercising choices, variables, call/return, scene state.
fn complex_story() -> Story {
    let mut b = B::new();
    let bg = b.asset(AssetKind::Image, "bg.png");
    let img = b.asset(AssetKind::Image, "a.png");
    let ana = b.character("Ana", &[("neutral", img)]);
    let (left, right, join, routine) = (NodeId::new(), NodeId::new(), NodeId::new(), NodeId::new());
    b.story.variables.push(VarDecl::game("n", Value::Int(0)));
    b.push(NodeKind::Background {
        asset: bg,
        transition: Some(Transition::dissolve(1.0)),
    });
    b.push(NodeKind::Show {
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
        character: ana,
        expression: "neutral".into(),
        position: Position::Center,
        transition: None,
    });
    b.say(Some(ana), "Start");
    let o1 = b.option("Left", left, None);
    let o2 = b.option(
        "Right",
        right,
        Some(Expr::binary(BinOp::Ge, Expr::var("n"), Expr::lit(0i64))),
    );
    b.push(NodeKind::Choice {
        options: vec![o1, o2],
    });
    b.label(left, "left");
    b.push(NodeKind::SetVar {
        var: "n".into(),
        value: Expr::lit(10i64),
    });
    b.push(NodeKind::Call { label: routine });
    b.push(NodeKind::Jump { target: join });
    b.label(right, "right");
    b.push(NodeKind::SetVar {
        var: "n".into(),
        value: Expr::lit(20i64),
    });
    b.say(None, "Right path");
    b.label(join, "join");
    b.push(NodeKind::Wait { seconds: 0.5 });
    b.say(Some(ana), "Joined");
    b.push(NodeKind::Hide {
        character: ana,
        transition: None,
    });
    b.push(NodeKind::End);
    b.label(routine, "routine");
    b.say(None, "In routine");
    b.push(NodeKind::SetVar {
        var: "n".into(),
        value: Expr::binary(BinOp::Add, Expr::var("n"), Expr::lit(1i64)),
    });
    b.push(NodeKind::Return);
    b.build()
}

#[test]
fn snapshot_restore_then_advance_matches_uninterrupted_run() {
    let story = complex_story();
    for path in [0usize, 1] {
        // Reference run, recording a snapshot before every tick.
        let mut reference = Vm::from_start(story.clone()).unwrap();
        let mut snapshots: Vec<VmState> = Vec::new();
        let mut full = Vec::new();
        let mut prefix_len = Vec::new();
        loop {
            snapshots.push(reference.snapshot());
            prefix_len.push(full.len());
            let r = tick(&mut reference, &mut |_| path, &mut full);
            if matches!(r, StepResult::End | StepResult::Error(_)) {
                break;
            }
        }
        assert!(snapshots.len() > 5);
        let final_state = reference.snapshot();

        for (k, snap) in snapshots.iter().enumerate() {
            // Through serde, as a save file would do.
            let text = ron::to_string(snap).unwrap();
            let loaded: VmState = ron::from_str(&text).unwrap();
            assert_eq!(&loaded, snap);

            let mut vm = Vm::from_start(story.clone()).unwrap();
            // Make the fresh VM diverge before restoring.
            let _ = vm.step();
            vm.take_events();
            vm.restore(loaded).unwrap();
            let rest = trace(&mut vm, &mut |_| path);
            assert_eq!(
                rest,
                full[prefix_len[k]..],
                "path {path}, resumed from snapshot {k}"
            );
            assert_eq!(vm.snapshot(), final_state);
        }
    }
}

#[test]
fn state_changes_only_through_the_api() {
    // Two VMs fed the same inputs have identical states at every step, and
    // taking snapshots never perturbs a run.
    let story = complex_story();
    let mut a = Vm::from_start(story.clone()).unwrap();
    let mut b = Vm::from_start(story).unwrap();
    let (mut ta, mut tb) = (Vec::new(), Vec::new());
    loop {
        let _ = a.snapshot();
        let ra = tick(&mut a, &mut |_| 1, &mut ta);
        let rb = tick(&mut b, &mut |_| 1, &mut tb);
        assert_eq!(ra, rb);
        assert_eq!(a.snapshot(), b.snapshot());
        if matches!(ra, StepResult::End) {
            break;
        }
    }
    assert_eq!(ta, tb);
}

#[test]
fn rng_is_seeded_and_part_of_the_state() {
    let mut b = B::new();
    b.say(None, "x");
    let story = b.build();
    let entry = story.entry().unwrap();
    let mut a = Vm::with_seed(story.clone(), entry, 42).unwrap();
    let mut b2 = Vm::with_seed(story.clone(), entry, 42).unwrap();
    let mut c = Vm::with_seed(story, entry, 43).unwrap();
    let draws = |vm: &mut Vm| (0..4).map(|_| vm.next_random()).collect::<Vec<_>>();
    let da = draws(&mut a);
    assert_eq!(da, draws(&mut b2));
    assert_ne!(da, draws(&mut c));

    // The RNG position is saved: restoring replays the same draws.
    let mut vm = Vm::with_seed(a.story().clone(), entry, 7).unwrap();
    vm.next_random();
    let snap = vm.snapshot();
    let next = draws(&mut vm);
    vm.restore(snap).unwrap();
    assert_eq!(draws(&mut vm), next);
}

#[test]
fn choice_views_expose_string_ids() {
    let mut b = B::new();
    let end = NodeId::new();
    let o = b.option("Go", end, None);
    let sid = o.text_id;
    b.push(NodeKind::Choice { options: vec![o] });
    b.label(end, "end");
    let mut vm = vm_for(b.build());
    let StepResult::Choice { options, .. } = vm.step() else {
        panic!("expected Choice");
    };
    assert_eq!(options, vec![ChoiceView { string_id: sid }]);
}

// ------------------------------------------------------------------ rollback

use crate::{Interaction, RollbackHistory};

/// bg + character + `L1` + music + `L2` + choice (A/B) + branch line + join.
struct RbStory {
    story: Story,
    bg: AssetId,
    ana: CharacterId,
    l1: NodeId,
    l2: NodeId,
    choice: NodeId,
    a_line: NodeId,
    b_line: NodeId,
    join1: NodeId,
    join2: NodeId,
}

fn rb_story() -> RbStory {
    let mut b = B::new();
    let bg = b.asset(AssetKind::Image, "bg.png");
    let img = b.asset(AssetKind::Image, "ana.png");
    let music = b.asset(AssetKind::Music, "m.ogg");
    let ana = b.character("Ana", &[("smile", img)]);
    b.story.variables.push(VarDecl::game("v", Value::Int(0)));
    b.story
        .variables
        .push(VarDecl::persistent("plays", Value::Int(0)));
    b.push(NodeKind::Background {
        asset: bg,
        transition: Some(Transition::dissolve(0.5)),
    });
    b.push(NodeKind::Show {
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
        character: ana,
        expression: "smile".into(),
        position: Position::Left,
        transition: None,
    });
    let l1 = b.say(Some(ana), "L1");
    b.push(NodeKind::PlayMusic {
        asset: music,
        fade_in: 1.0,
        looping: true,
    });
    let l2 = b.say(None, "L2");
    let (la, lb, join) = (NodeId::new(), NodeId::new(), NodeId::new());
    let oa = b.option("A", la, None);
    let ob = b.option("B", lb, None);
    let choice = b.push(NodeKind::Choice {
        options: vec![oa, ob],
    });
    b.label(la, "a");
    b.push(NodeKind::SetVar {
        var: "v".into(),
        value: Expr::Lit(Value::Int(1)),
    });
    let a_line = b.say(None, "A line");
    b.push(NodeKind::Jump { target: join });
    b.label(lb, "b");
    b.push(NodeKind::SetVar {
        var: "v".into(),
        value: Expr::Lit(Value::Int(2)),
    });
    let b_line = b.say(None, "B line");
    b.label(join, "join");
    b.push(NodeKind::SetVar {
        var: "plays".into(),
        value: Expr::binary(BinOp::Add, Expr::var("plays"), Expr::Lit(Value::Int(1))),
    });
    let join1 = b.say(None, "Join1");
    let join2 = b.say(None, "Join2");
    b.push(NodeKind::End);
    RbStory {
        story: b.build(),
        bg,
        ana,
        l1,
        l2,
        choice,
        a_line,
        b_line,
        join1,
        join2,
    }
}

/// Steps until the next Say/Choice and returns its node.
fn next_interaction(vm: &mut Vm) -> NodeId {
    for _ in 0..100 {
        match vm.step() {
            StepResult::Say { node, .. } | StepResult::Choice { node, .. } => return node,
            StepResult::WaitTime(_) | StepResult::WaitTransition { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    panic!("no interaction");
}

fn shown_node(p: &crate::Presentation) -> NodeId {
    p.interaction.as_ref().expect("an interaction").node()
}

#[test]
fn rollback_restores_scene_without_replaying_and_replay_is_identical() {
    let s = rb_story();
    let mut vm = vm_for(s.story.clone());
    let mut nodes = Vec::new();
    let mut scenes = Vec::new();
    for _ in 0..3 {
        nodes.push(next_interaction(&mut vm));
        scenes.push(vm.scene().clone());
    }
    vm.choose(0).unwrap();
    for _ in 0..2 {
        nodes.push(next_interaction(&mut vm));
        scenes.push(vm.scene().clone());
    }
    assert_eq!(
        nodes,
        [s.l1, s.l2, s.choice, s.a_line, s.join1],
        "unexpected timeline"
    );
    assert_eq!(vm.variable("plays"), Some(&Value::Int(1)));

    // Back to L1: bg + Ana, but the music (started after L1) is gone, and no
    // event is pending: the host rebuilds from the scene.
    let p = vm.rollback(4).expect("rollback");
    assert_eq!(shown_node(&p), s.l1);
    assert_eq!(p.scene, scenes[0]);
    assert!(p.scene.music.is_none());
    assert_eq!(p.scene.background, Some(s.bg));
    assert!(p.scene.character(s.ana).is_some());
    assert!(vm.take_events().is_empty());
    assert!(p.events.iter().all(|e| !matches!(
        e,
        PresentationEvent::Background {
            transition: Some(_),
            ..
        }
    )));
    assert!(matches!(p.events[0], PresentationEvent::Background { .. }));
    assert!(!vm.can_rollback());
    assert!(vm.can_roll_forward());
    assert!(p.already_read);
    // Game variables are rolled back, persistent ones are not.
    assert_eq!(vm.variable("v"), Some(&Value::Int(0)));
    assert_eq!(vm.variable("plays"), Some(&Value::Int(1)));

    // Stepping forward replays exactly the same interactions and scenes.
    for i in 1..nodes.len() {
        let n = next_interaction(&mut vm);
        assert_eq!(n, nodes[i]);
        assert_eq!(vm.scene(), &scenes[i]);
        if matches!(vm.snapshot().pending, Pending::Choice { .. }) {
            vm.choose(0).unwrap(); // same choice as before
        }
    }
    assert!(!vm.can_roll_forward());
    // Rolling forward is the other way to get there.
    vm.rollback(3).unwrap();
    let p = vm.roll_forward(2).unwrap();
    assert_eq!(shown_node(&p), s.a_line);
    assert_eq!(vm.variable("v"), Some(&Value::Int(1)));
}

#[test]
fn different_choice_truncates_the_future_and_backlog() {
    let s = rb_story();
    let mut vm = vm_for(s.story);
    next_interaction(&mut vm); // L1
    next_interaction(&mut vm); // L2
    next_interaction(&mut vm); // choice
    vm.choose(0).unwrap();
    assert_eq!(next_interaction(&mut vm), s.a_line);
    assert_eq!(next_interaction(&mut vm), s.join1);

    // Back to the choice and re-pick the same option: the future is kept.
    let p = vm.rollback(2).unwrap();
    assert_eq!(shown_node(&p), s.choice);
    assert!(matches!(p.interaction, Some(Interaction::Choice { .. })));
    vm.choose(0).unwrap();
    assert!(vm.can_roll_forward());
    assert_eq!(next_interaction(&mut vm), s.a_line);

    // Back again, pick the other option: the A branch is gone.
    vm.rollback(1).unwrap();
    vm.choose(1).unwrap();
    assert!(!vm.can_roll_forward());
    assert_eq!(next_interaction(&mut vm), s.b_line);
    assert_eq!(vm.variable("v"), Some(&Value::Int(2)));
    let lines: Vec<NodeId> = vm.backlog().iter().map(|b| b.node).collect();
    assert_eq!(lines, [s.l1, s.l2, s.b_line]);
    // L1, L2, choice, B line.
    assert_eq!(vm.history_len(), 4);
}

#[test]
fn rollback_at_the_end_reshows_the_last_line() {
    let s = rb_story();
    let mut vm = vm_for(s.story);
    for _ in 0..3 {
        next_interaction(&mut vm);
    }
    vm.choose(1).unwrap();
    for _ in 0..3 {
        next_interaction(&mut vm);
    }
    assert!(matches!(vm.step(), StepResult::End));
    let p = vm.rollback(1).unwrap();
    assert_eq!(shown_node(&p), s.join2, "the last line is on screen at End");
    let p = vm.rollback(1).unwrap();
    assert_eq!(shown_node(&p), s.join1);
    // Stepping again goes through Join2 and reaches the end again.
    assert_eq!(next_interaction(&mut vm), s.join2);
    assert!(matches!(vm.step(), StepResult::End));
    assert!(vm.is_finished());
}

#[test]
fn read_set_survives_rollback() {
    let s = rb_story();
    let mut vm = vm_for(s.story);
    next_interaction(&mut vm);
    next_interaction(&mut vm);
    vm.rollback(1).unwrap();
    assert!(vm.is_read(s.l2));
    let StepResult::Say { already_read, .. } = vm.step() else {
        panic!("expected a line");
    };
    assert!(already_read);
}

#[test]
fn barrier_stops_rollback() {
    let s = rb_story();
    let mut vm = vm_for(s.story);
    next_interaction(&mut vm); // L1
    vm.checkpoint_barrier();
    next_interaction(&mut vm); // L2 (barrier)
    next_interaction(&mut vm); // choice
    vm.choose(0).unwrap();
    next_interaction(&mut vm); // A line
    assert!(vm.can_rollback());
    let p = vm.rollback(99).unwrap();
    assert_eq!(shown_node(&p), s.l2, "cannot go before the barrier");
    assert!(!vm.can_rollback());
    assert!(vm.rollback(1).is_none());
    // The backlog marks lines before the barrier as not rollbackable.
    let backlog = vm.backlog();
    assert_eq!(backlog[0].node, s.l1);
    assert_eq!(backlog[0].checkpoint, None);
    assert!(backlog[1].checkpoint.is_some());
}

#[test]
fn history_is_a_bounded_ring_and_zero_disables_rollback() {
    let s = rb_story();
    let mut vm = vm_for(s.story.clone());
    vm.set_history_capacity(3);
    for _ in 0..3 {
        next_interaction(&mut vm);
    }
    vm.choose(0).unwrap();
    for _ in 0..2 {
        next_interaction(&mut vm);
    }
    assert_eq!(vm.history_len(), 3);
    let p = vm.rollback(99).unwrap();
    assert_eq!(shown_node(&p), s.choice.min(s.choice));
    // The backlog is longer than the ring: old lines are not rollbackable.
    let backlog = vm.backlog();
    assert_eq!(backlog.len(), 2, "L1 and L2 (lines up to the choice)");
    assert!(backlog.iter().all(|b| b.checkpoint.is_none()));

    let mut vm = vm_for(s.story);
    vm.set_history_capacity(0);
    next_interaction(&mut vm);
    next_interaction(&mut vm);
    assert!(!vm.can_rollback());
    assert!(vm.rollback(1).is_none());
    assert_eq!(vm.backlog().len(), 2, "backlog works without checkpoints");
}

#[test]
fn rollback_to_checkpoint_from_the_backlog() {
    let s = rb_story();
    let mut vm = vm_for(s.story);
    for _ in 0..3 {
        next_interaction(&mut vm);
    }
    vm.choose(1).unwrap();
    next_interaction(&mut vm);
    let backlog = vm.backlog();
    assert_eq!(backlog.len(), 3);
    let seq = backlog[1].checkpoint.expect("rollbackable");
    let p = vm.rollback_to(seq).unwrap();
    assert_eq!(shown_node(&p), s.l2);
    assert_eq!(p.checkpoint, Some(seq));
    // Rolled-forward lines come back to the backlog; the rest is hidden.
    assert_eq!(vm.backlog().len(), 2);
    assert!(vm.rollback_to(9999).is_none());
}

#[test]
fn persistent_variables_are_separate_from_the_state() {
    let s = rb_story();
    let mut vm = vm_for(s.story);
    assert_eq!(vm.persistent_vars().get("plays"), Some(&Value::Int(0)));
    assert!(!vm.snapshot().vars.contains_key("plays"));
    assert!(vm.snapshot().vars.contains_key("v"));

    // The host store wins; wrongly typed or unknown entries are ignored.
    let mut store = std::collections::BTreeMap::new();
    store.insert("plays".to_owned(), Value::Int(7));
    store.insert("ghost".to_owned(), Value::Int(1));
    vm.set_persistent_vars(&store);
    assert_eq!(vm.variable("plays"), Some(&Value::Int(7)));
    assert!(!vm.persistent_vars().contains_key("ghost"));
    store.insert("plays".to_owned(), Value::Bool(true));
    vm.set_persistent_vars(&store);
    assert_eq!(vm.variable("plays"), Some(&Value::Int(0)));

    // SetVar writes the persistent store; snapshot/restore leave it alone.
    store.insert("plays".to_owned(), Value::Int(7));
    vm.set_persistent_vars(&store);
    let snap = vm.snapshot();
    for _ in 0..3 {
        next_interaction(&mut vm);
    }
    vm.choose(0).unwrap();
    next_interaction(&mut vm);
    next_interaction(&mut vm); // Join1 (plays incremented before it)
    assert_eq!(vm.variable("plays"), Some(&Value::Int(8)));
    vm.restore(snap).unwrap();
    assert_eq!(vm.variable("plays"), Some(&Value::Int(8)));
    assert!(!vm.snapshot().vars.contains_key("plays"));
}

#[test]
fn history_roundtrips_through_serialization_and_restores() {
    let s = rb_story();
    let mut vm = vm_for(s.story.clone());
    for _ in 0..3 {
        next_interaction(&mut vm);
    }
    vm.choose(0).unwrap();
    next_interaction(&mut vm);
    next_interaction(&mut vm);

    let state = vm.snapshot();
    let history = vm.history_snapshot(100, 100);
    let text = ron::to_string(&(&state, &history)).unwrap();
    let (state2, history2): (VmState, RollbackHistory) = ron::from_str(&text).unwrap();
    assert_eq!(state, state2);
    assert_eq!(history, history2);

    let mut loaded = vm_for(s.story.clone());
    loaded.restore_with_history(state2, history2).unwrap();
    assert_eq!(loaded.presentation(), vm.presentation());
    assert_eq!(loaded.backlog(), vm.backlog());
    // The loaded VM can roll back exactly like the original.
    let a = vm.rollback(1).unwrap();
    let b = loaded.rollback(1).unwrap();
    assert_eq!(a, b);
    assert_eq!(shown_node(&b), s.a_line);

    // A bounded snapshot keeps only the most recent entries.
    let small = vm.history_snapshot(2, 1);
    assert_eq!(small.checkpoints.len(), 2);
    assert_eq!(small.backlog.len(), 1);

    // Without history, a plain restore still knows what to show.
    let mut plain = vm_for(s.story);
    plain.restore(state).unwrap();
    assert_eq!(shown_node(&plain.presentation()), s.join1);
    assert!(!plain.can_rollback());
}

#[test]
fn restore_reconciles_a_state_from_an_older_story() {
    let s = rb_story();
    let mut old = vm_for(s.story.clone());
    next_interaction(&mut old);
    let old_state = old.snapshot();

    let mut newer = s.story.clone();
    newer.variables.push(VarDecl::game("fresh", Value::Int(5)));
    newer.assets.assets.retain(|a| a.id != s.bg); // background removed
    let mut vm = vm_for(newer);
    vm.restore(old_state).unwrap();
    assert_eq!(vm.variable("fresh"), Some(&Value::Int(5)));
    assert_eq!(vm.scene().background, None);
    assert!(vm.scene().character(s.ana).is_some());
    // And a state pointing to a vanished node is refused.
    let mut broken = old.snapshot();
    broken.pc = Some(NodeId::new());
    assert!(matches!(vm.restore(broken), Err(VmError::UnknownNode(_))));
    let _ = (s.l2, s.b_line, s.join2);
}

// ------------------------------------------------ randomized action sequences

/// Loop-heavy story used by the fuzz test.
fn fuzz_story() -> Story {
    let mut b = B::new();
    b.story.variables.push(VarDecl::game("n", Value::Int(0)));
    b.story
        .variables
        .push(VarDecl::persistent("p", Value::Int(0)));
    let (top, mid, fin) = (NodeId::new(), NodeId::new(), NodeId::new());
    b.label(top, "top");
    b.push(NodeKind::SetVar {
        var: "n".into(),
        value: Expr::binary(BinOp::Add, Expr::var("n"), Expr::Lit(Value::Int(1))),
    });
    b.push(NodeKind::SetVar {
        var: "p".into(),
        value: Expr::binary(BinOp::Add, Expr::var("p"), Expr::Lit(Value::Int(1))),
    });
    b.say(None, "top");
    let again = b.option(
        "again",
        top,
        Some(Expr::binary(
            BinOp::Lt,
            Expr::var("n"),
            Expr::Lit(Value::Int(5)),
        )),
    );
    let next = b.option("next", mid, None);
    b.push(NodeKind::Choice {
        options: vec![again, next],
    });
    b.label(mid, "mid");
    b.say(None, "mid 1");
    b.say(None, "mid 2");
    let end = b.option("end", fin, None);
    let back = b.option("back", top, None);
    b.push(NodeKind::Choice {
        options: vec![end, back],
    });
    b.label(fin, "fin");
    b.say(None, "fin");
    b.push(NodeKind::End);
    b.build()
}

/// Reference model of the rollback timeline.
#[derive(Default)]
struct Model {
    /// (node, chosen option) per interaction.
    timeline: Vec<(NodeId, Option<usize>)>,
    pos: usize,
}

impl Model {
    fn interaction(&mut self, node: NodeId) {
        if self.timeline.is_empty() {
            self.timeline.push((node, None));
            self.pos = 0;
        } else if self.timeline.get(self.pos + 1).map(|t| t.0) == Some(node) {
            self.pos += 1;
        } else {
            self.timeline.truncate(self.pos + 1);
            self.timeline.push((node, None));
            self.pos = self.timeline.len() - 1;
        }
    }

    fn chose(&mut self, index: usize) {
        if self.timeline[self.pos].1 != Some(index) {
            self.timeline.truncate(self.pos + 1);
            self.timeline[self.pos].1 = Some(index);
        }
    }
}

#[test]
fn random_action_sequences_keep_invariants() {
    let story = std::sync::Arc::new(fuzz_story());
    for seed in 0..300u64 {
        let mut rng = crate::Rng::new(seed);
        let mut vm = Vm::from_start(story.clone()).unwrap();
        let mut model = Model::default();
        let mut ahead = false;
        let mut saves: Vec<(VmState, RollbackHistory)> = Vec::new();
        for _ in 0..150 {
            let op = rng.next_u64() % 8;
            let repeated = matches!(vm.snapshot().pending, Pending::Choice { .. });
            match op {
                0..=3 => match vm.step() {
                    // Asking again while a choice is pending changes nothing.
                    StepResult::Choice { .. } if repeated => {}
                    StepResult::Say { node, .. } | StepResult::Choice { node, .. } => {
                        model.interaction(node);
                        ahead = false;
                    }
                    StepResult::End => ahead = true,
                    other => panic!("seed {seed}: {other:?}"),
                },
                4 => {
                    if let Pending::Choice { visible, .. } = vm.snapshot().pending {
                        let i = (rng.next_u64() as usize) % visible.len();
                        vm.choose(i).unwrap();
                        model.chose(i);
                        ahead = true;
                    } else {
                        assert!(vm.choose(0).is_err());
                    }
                }
                5 => {
                    let k = 1 + (rng.next_u64() % 4) as usize;
                    let eff = if ahead { k - 1 } else { k };
                    let want = model.pos.saturating_sub(eff);
                    let moved = want != model.pos || (ahead && !model.timeline.is_empty());
                    match vm.rollback(k) {
                        Some(p) => {
                            assert!(moved, "seed {seed}: unexpected rollback");
                            model.pos = want;
                            assert_eq!(shown_node(&p), model.timeline[want].0);
                            ahead = false;
                        }
                        None => assert!(!moved, "seed {seed}: rollback refused"),
                    }
                }
                6 => {
                    let k = 1 + (rng.next_u64() % 3) as usize;
                    let want = (model.pos + k).min(model.timeline.len().saturating_sub(1));
                    match vm.roll_forward(k) {
                        Some(p) => {
                            assert_ne!(want, model.pos);
                            model.pos = want;
                            assert_eq!(shown_node(&p), model.timeline[want].0);
                            ahead = false;
                        }
                        None => assert!(want == model.pos || model.timeline.is_empty()),
                    }
                }
                _ => {
                    if saves.is_empty() || rng.next_u64().is_multiple_of(2) {
                        saves.push((vm.snapshot(), vm.history_snapshot(50, 50)));
                    } else {
                        let (state, history) = &saves[(rng.next_u64() as usize) % saves.len()];
                        vm.restore_with_history(state.clone(), history.clone())
                            .unwrap();
                        // The model restarts from the loaded timeline.
                        model = Model::default();
                        for c in &history.checkpoints {
                            model
                                .timeline
                                .push((c.state.shown.as_ref().unwrap().node(), c.chosen));
                        }
                        model.pos = model.timeline.len().saturating_sub(1);
                        ahead = history
                            .checkpoints
                            .last()
                            .is_some_and(|c| c.state != *state);
                    }
                }
            }
            assert!(vm.history_len() <= vm.history_capacity());
            assert!(vm.backlog().len() <= 500);
            // A rollback is possible exactly when the model says so.
            let possible = model.pos > 0 || (ahead && !model.timeline.is_empty());
            assert_eq!(vm.can_rollback(), possible, "seed {seed}");
            // Persistent variables are never part of the state.
            assert!(!vm.snapshot().vars.contains_key("p"));
        }
    }
}

#[test]
fn custom_node_emits_a_generic_event_and_falls_through() {
    use orbit_core::CustomData;

    let mut b = B::new();
    b.push(NodeKind::Custom {
        plugin_id: "acme.anim".into(),
        kind: "play_clip".into(),
        version: 1,
        data: CustomData::Str("walk".into()),
    });
    b.say(None, "after");
    let mut vm = vm_for(b.build());

    let StepResult::Say { string_id, .. } = vm.step() else {
        panic!("expected Say");
    };
    assert_eq!(vm.text(string_id), Some("after"));
    assert_eq!(
        vm.take_events(),
        vec![PresentationEvent::Custom {
            plugin_id: "acme.anim".into(),
            kind: "play_clip".into(),
            data: CustomData::Str("walk".into()),
        }]
    );
}

// ------------------------------------------------------------ scene jumps

/// Scene `a`: say "a1", say "a2". Scene `b`: say "b1", label, say "b2".
/// Returns the story, the ids of `a` and `b`, and the label of `b`.
fn two_scenes() -> (Story, SceneId, SceneId, NodeId) {
    let mut b = B::new();
    let scene = |b: &mut B, name: &str, build: &dyn Fn(&mut B) -> NodeId| {
        let before = std::mem::take(&mut b.nodes);
        let label = build(b);
        let nodes = std::mem::replace(&mut b.nodes, before);
        let id = SceneId::new();
        b.story.scenes.push(Scene {
            id,
            name: name.into(),
            nodes,
        });
        (id, label)
    };
    let (a, _) = scene(&mut b, "a", &|b| {
        b.say(None, "a1");
        b.say(None, "a2")
    });
    let (sb, label) = scene(&mut b, "b", &|b| {
        b.say(None, "b1");
        let l = b.label(NodeId::new(), "mid");
        b.say(None, "b2");
        l
    });
    (b.story, a, sb, label)
}

fn say_trace(story: Story) -> Vec<String> {
    let mut vm = vm_for(story);
    trace(&mut vm, &mut first)
}

#[test]
fn a_scene_never_falls_into_the_next_one() {
    let (story, ..) = two_scenes();
    let out = say_trace(story);
    let text = out.join("|");
    assert!(text.contains("a1") && text.contains("a2"), "{text}");
    assert!(!text.contains("b1"), "{text}");
    assert!(text.ends_with("end"), "{text}");
}

#[test]
fn jump_scene_enters_the_scene_at_its_start_or_at_a_label() {
    let (mut story, _, sb, label) = two_scenes();
    story.scenes[0].nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::JumpScene {
            scene: sb,
            label: None,
        },
    });
    let text = say_trace(story.clone()).join("|");
    for want in ["a1", "a2", "b1", "b2"] {
        assert!(text.contains(want), "{want} missing from {text}");
    }
    assert!(text.find("a2").unwrap() < text.find("b1").unwrap());
    assert!(text.ends_with("end"));

    // With a label: skips b1.
    story.scenes[0].nodes.pop();
    story.scenes[0].nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::JumpScene {
            scene: sb,
            label: Some(label),
        },
    });
    let text = say_trace(story).join("|");
    assert!(text.contains("b2") && !text.contains("b1"), "{text}");
}

#[test]
fn jump_scene_to_an_unknown_scene_is_an_error_and_to_an_empty_one_ends() {
    let (mut story, ..) = two_scenes();
    story.scenes[0].nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::JumpScene {
            scene: SceneId::new(),
            label: None,
        },
    });
    let mut vm = vm_for(story.clone());
    let out = trace(&mut vm, &mut first);
    assert!(out.last().unwrap().contains("unknown scene"), "{out:?}");

    let empty = SceneId::new();
    story.scenes.push(Scene {
        id: empty,
        name: "vide".into(),
        nodes: Vec::new(),
    });
    let last = story.scenes[0].nodes.len() - 1;
    story.scenes[0].nodes[last].kind = NodeKind::JumpScene {
        scene: empty,
        label: None,
    };
    let text = say_trace(story).join("|");
    assert!(text.contains("a2") && !text.contains("b1"), "{text}");
    assert!(text.ends_with("end"), "{text}");
}

#[test]
fn rollback_and_seek_cross_scene_jumps() {
    use std::sync::Arc;

    let (mut story, _, sb, _) = two_scenes();
    story.scenes[0].nodes.push(Node {
        id: NodeId::new(),
        kind: NodeKind::JumpScene {
            scene: sb,
            label: None,
        },
    });
    let b1 = story.scenes[1].nodes[0].id;
    let b2 = story.scenes[1].nodes.last().unwrap().id;
    let story = Arc::new(story);

    // Seek: "play from here" on a line of the second scene.
    let crate::SeekOutcome::Found(path) = crate::seek(&story, b2, crate::SeekLimits::default())
    else {
        panic!("reachable through the jump");
    };
    assert!(path.choices.is_empty());
    let vm = path.into_vm(&story).unwrap();
    assert_eq!(vm.position(), Some(b2));

    // Rollback across the jump: a1, a2, b1, b2, then back to b1.
    let mut vm = Vm::from_start(story.clone()).unwrap();
    for _ in 0..3 {
        assert!(matches!(vm.step(), StepResult::Say { .. }));
    }
    let StepResult::Say { node, .. } = vm.step() else {
        panic!("b2")
    };
    assert_eq!(node, b2);
    assert!(vm.rollback(1).is_some());
    assert!(vm.can_roll_forward());
    let StepResult::Say { node, .. } = vm.step() else {
        panic!("a line")
    };
    assert_eq!(node, b2, "replays the line the player rolled back from");
    let _ = b1;
}

/// Scale and mirroring: effective scale resolution, in-place update by a
/// second `Show`, and restoration by rollback, snapshot restore and the
/// "scene as events" rebuild.
#[test]
fn show_scale_and_flip_are_tracked_and_restored() {
    let mut b = B::new();
    let img = b.asset(AssetKind::Image, "a.png");
    let ana = b.character("Ana", &[("neutral", img)]);
    b.story
        .characters
        .iter_mut()
        .find(|c| c.id == ana)
        .unwrap()
        .default_scale = 0.5;
    let show = |position: Position, scale, scale_y, flip_x, flip_y| NodeKind::Show {
        character: ana,
        expression: "neutral".into(),
        position,
        transition: None,
        scale,
        scale_y,
        flip_x,
        flip_y,
    };
    // 1: default scale (0.5), no mirror. 2: override + mirror X + free
    // position. 3: non uniform stretch + mirror Y.
    b.push(show(Position::Left, None, None, false, false));
    b.say(Some(ana), "one");
    b.push(show(
        Position::Free { x: 0.3, y: 0.8 },
        Some(1.5),
        None,
        true,
        false,
    ));
    b.say(Some(ana), "two");
    b.push(show(Position::Center, Some(2.0), Some(0.5), false, true));
    b.say(Some(ana), "three");
    let story = b.build();
    let mut vm = vm_for(story.clone());

    assert!(matches!(vm.step(), StepResult::Say { .. }));
    let events = vm.take_events();
    assert!(matches!(
        &events[0],
        PresentationEvent::Show { scale, flip_x: false, flip_y: false, .. } if *scale == [0.5, 0.5]
    ));
    assert_eq!(vm.scene().character(ana).unwrap().scale, [0.5, 0.5]);

    assert!(matches!(vm.step(), StepResult::Say { .. }));
    let events = vm.take_events();
    assert!(matches!(
        &events[0],
        PresentationEvent::Show { x, y, scale, flip_x: true, .. }
            if (*x, *y) == (0.3, 0.8) && *scale == [1.5, 1.5]
    ));
    let c = vm.scene().character(ana).unwrap().clone();
    assert_eq!(c.position, Position::Free { x: 0.3, y: 0.8 });
    assert!(c.flip_x && !c.flip_y);
    assert_eq!(vm.scene().characters.len(), 1, "updated in place");

    assert!(matches!(vm.step(), StepResult::Say { .. }));
    vm.take_events();
    let c = vm.scene().character(ana).unwrap();
    assert_eq!(c.scale, [2.0, 0.5]);
    assert!(!c.flip_x && c.flip_y);

    // Rollback to line two brings back its scale, mirror and free position,
    // both in the scene and in the rebuild events.
    let p = vm.rollback(1).unwrap();
    let c = p.scene.character(ana).unwrap();
    assert_eq!(c.scale, [1.5, 1.5]);
    assert!(c.flip_x);
    assert!(matches!(
        p.events.last(),
        Some(PresentationEvent::Show { x, y, scale, flip_x: true, .. })
            if (*x, *y) == (0.3, 0.8) && *scale == [1.5, 1.5]
    ));

    // A saved state restores the same look.
    let state = vm.snapshot();
    let mut other = vm_for(story);
    other.restore(state).unwrap();
    assert_eq!(other.scene().character(ana).unwrap().scale, [1.5, 1.5]);
}

// ------------------------------------------------- no-code variable ops

fn varop_story() -> Story {
    let mut b = B::new();
    b.story.variables.push(VarDecl::game("gold", Value::Int(10)));
    b.story.variables.push(VarDecl::game("flag", Value::Bool(false)));
    b.story
        .variables
        .push(VarDecl::persistent("plays", Value::Int(0)));
    b.say(None, "one");
    b.say(None, "two");
    b.say(None, "three");
    b.build()
}

#[test]
fn ui_var_op_survives_rollback_of_the_current_screen_and_is_cut_from_the_future() {
    use orbit_core::{OpKind, Operand, VarOp};
    let mut vm = Vm::from_start(varop_story()).unwrap();
    vm.step();
    vm.step();
    // On screen: line two. A UI button adds 5 gold.
    let add = VarOp::new("gold", OpKind::Add, Operand::Lit(Value::Int(5)));
    vm.apply_var_op(&add).unwrap();
    assert_eq!(vm.variable("gold"), Some(&Value::Int(15)));
    vm.step(); // line three
    assert_eq!(vm.variable("gold"), Some(&Value::Int(15)));
    // Back to line two: the change made while it was on screen is kept.
    vm.rollback(1).unwrap();
    assert_eq!(vm.variable("gold"), Some(&Value::Int(15)));
    // Back to line one: before the change.
    vm.rollback(1).unwrap();
    assert_eq!(vm.variable("gold"), Some(&Value::Int(10)));
    // Operating from the past cuts the known future.
    vm.apply_var_op(&VarOp::toggle("flag")).unwrap();
    assert!(!vm.can_roll_forward());
    assert_eq!(vm.variable("flag"), Some(&Value::Bool(true)));
    // The state (what a save embeds) carries the change.
    assert_eq!(vm.snapshot().vars.get("flag"), Some(&Value::Bool(true)));
}

#[test]
fn ui_var_op_on_persistent_and_invalid_targets() {
    use orbit_core::{OpError, OpKind, Operand, VarOp};
    let mut vm = Vm::from_start(varop_story()).unwrap();
    vm.step();
    let add = VarOp::new("plays", OpKind::Add, Operand::Lit(Value::Int(1)));
    vm.apply_var_op(&add).unwrap();
    assert_eq!(vm.persistent_vars().get("plays"), Some(&Value::Int(1)));
    assert!(!vm.snapshot().vars.contains_key("plays"));
    // Errors change nothing and leave a warning.
    let div0 = VarOp::new("gold", OpKind::Div, Operand::Lit(Value::Int(0)));
    assert_eq!(vm.apply_var_op(&div0), Err(OpError::DivisionByZero));
    assert!(vm.apply_var_op(&VarOp::set("ghost", 1i64)).is_err());
    assert!(vm.apply_var_op(&VarOp::set("gold", "text")).is_err());
    assert_eq!(vm.variable("gold"), Some(&Value::Int(10)));
    assert_eq!(vm.take_warnings().len(), 3);
    assert!(vm.take_warnings().is_empty());
}

#[test]
fn ui_condition_reads_variables() {
    use orbit_core::{Cmp, Condition};
    let mut vm = Vm::from_start(varop_story()).unwrap();
    assert!(vm.eval_condition(&Condition::cmp("gold", Cmp::Ge, 10i64)));
    assert!(!vm.eval_condition(&Condition::is_true("flag")));
    assert!(!vm.eval_condition(&Condition::is_true("ghost")));
    assert_eq!(vm.take_warnings().len(), 1);
}

#[test]
fn set_var_node_is_a_noop_on_division_by_zero_and_keeps_int_type() {
    let mut b = B::new();
    b.story.variables.push(VarDecl::game("gold", Value::Int(10)));
    b.push(NodeKind::SetVar {
        var: "gold".into(),
        value: Expr::parse("gold / 0").unwrap(),
    });
    b.push(NodeKind::SetVar {
        var: "gold".into(),
        value: Expr::parse("gold * 1.26").unwrap(),
    });
    b.say(None, "end");
    let mut vm = Vm::from_start(b.build()).unwrap();
    assert!(matches!(vm.step(), StepResult::Say { .. }));
    assert_eq!(vm.variable("gold"), Some(&Value::Int(13)));
    assert_eq!(vm.take_warnings().len(), 1);
}
