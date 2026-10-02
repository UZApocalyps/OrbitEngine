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
            id,
            name_id,
            color: Color::WHITE,
            expressions: expressions
                .iter()
                .map(|(n, image)| Expression {
                    name: (*n).into(),
                    image: *image,
                })
                .collect(),
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
    b.story.variables.push(VarDecl {
        name: "key".into(),
        initial: Value::Bool(false),
    });
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
    b.story.variables.push(VarDecl {
        name: "score".into(),
        initial: Value::Int(1),
    });
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
        transition: Some(fade),
    });
    b.push(NodeKind::PlayMusic {
        asset: music,
        fade_in: 2.0,
        looping: true,
    });
    b.push(NodeKind::Show {
        character: ana,
        expression: "neutral".into(),
        position: Position::Left,
        transition: Some(Transition::dissolve(0.0)), // zero duration: no wait
    });
    b.push(NodeKind::PlaySound { asset: sfx });
    b.say(Some(ana), "Hi");
    b.push(NodeKind::Show {
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
        transition: Some(fade),
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
    b.story.variables.push(VarDecl {
        name: "n".into(),
        initial: Value::Int(0),
    });
    b.push(NodeKind::Background {
        asset: bg,
        transition: Some(Transition::dissolve(1.0)),
    });
    b.push(NodeKind::Show {
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
