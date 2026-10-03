//! Lists and maps in the VM (UI-13, ADR 0016): operations, conditions,
//! recoverable errors, persistent collections, rollback and its cost.

use std::collections::BTreeMap;
use std::sync::Arc;

use orbit_core::{
    ChoiceOption, Cmp, Condition, Expr, Node, NodeId, NodeKind, OpKind, Operand, ScalarType,
    Scene, SceneId, Story, Value, VarDecl, VarOp,
};

use crate::{StepResult, Vm};

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

fn say(story: &mut Story, text: &str) -> Node {
    let text_id = story.strings.add(text);
    node(NodeKind::Say {
        speaker: None,
        text_id,
        voice: None,
    })
}

fn set(var: &str, expr: &str) -> Node {
    node(NodeKind::SetVar {
        var: var.into(),
        value: Expr::parse(expr).unwrap(),
    })
}

fn story_with(vars: Vec<VarDecl>, build: impl FnOnce(&mut Story) -> Vec<Node>) -> Story {
    let mut story = Story::new("collections");
    story.variables = vars;
    let nodes = build(&mut story);
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "main".into(),
        nodes,
    });
    story
}

fn strs(items: &[&str]) -> Value {
    Value::list(ScalarType::Str, items.iter().map(|s| Value::from(*s)))
}

/// Steps until the next line or the end, answering choices with the first
/// visible option; returns the lines shown.
fn lines(vm: &mut Vm) -> Vec<String> {
    let mut out = Vec::new();
    for _ in 0..1000 {
        match vm.step() {
            StepResult::Say { string_id, .. } => {
                out.push(vm.text(string_id).unwrap_or("?").to_owned());
            }
            StepResult::Choice { .. } => vm.choose(0).unwrap(),
            StepResult::End => return out,
            StepResult::Error(e) => panic!("{e}"),
            _ => {}
        }
    }
    panic!("the story did not end");
}

#[test]
fn an_inventory_plays_with_operations_and_conditions() {
    let story = story_with(
        vec![
            VarDecl::game("bag", strs(&["sword"])),
            VarDecl::game("gold", Value::empty_map(ScalarType::Int)),
        ],
        |s| {
            let has_key = NodeId::new();
            let no_key = NodeId::new();
            let after = NodeId::new();
            let mut nodes = vec![
                set("bag", "push(bag, \"key\")"),
                set("gold", "insert(gold, \"alice\", 3)"),
                set("gold", "insert(gold, \"bob\", gold[\"alice\"] * 2)"),
                node(NodeKind::If {
                    condition: Condition::cmp("bag", Cmp::Contains, "key").to_expr(),
                    then_target: has_key,
                    else_target: Some(no_key),
                }),
                Node { id: has_key, kind: NodeKind::Label { name: "has".into() } },
            ];
            nodes.push(say(s, "has key"));
            nodes.push(node(NodeKind::Jump { target: after }));
            nodes.push(Node { id: no_key, kind: NodeKind::Label { name: "none".into() } });
            nodes.push(say(s, "no key"));
            nodes.push(Node { id: after, kind: NodeKind::Label { name: "after".into() } });
            // A choice whose first option is hidden unless bob has gold.
            let (o1, o2) = (s.strings.add("Pay bob"), s.strings.add("Leave"));
            let target = NodeId::new();
            nodes.push(node(NodeKind::Choice {
                options: vec![
                    ChoiceOption {
                        text_id: o1,
                        target,
                        condition: Some(Expr::parse("has_key(gold, \"carol\")").unwrap()),
                    },
                    ChoiceOption {
                        text_id: o2,
                        target,
                        condition: Some(Expr::parse("len(gold) == 2 && gold[\"bob\"] == 6").unwrap()),
                    },
                ],
            }));
            nodes.push(Node { id: target, kind: NodeKind::Label { name: "t".into() } });
            nodes.push(set("bag", "remove(bag, \"sword\")"));
            nodes.push(set("gold", "remove_key(gold, \"alice\")"));
            nodes.push(say(s, "done"));
            nodes.push(node(NodeKind::End));
            nodes
        },
    );
    assert!(orbit_core::validate(&story).errors.is_empty(), "{:#?}", orbit_core::validate(&story).errors);
    let mut vm = Vm::from_start(story).unwrap();
    assert_eq!(lines(&mut vm), ["has key", "done"]);
    assert_eq!(vm.variable("bag"), Some(&strs(&["key"])));
    assert_eq!(
        vm.variable("gold"),
        Some(&Value::map(ScalarType::Int, [("bob", Value::Int(6))]))
    );
    assert!(vm.take_warnings().is_empty());
}

#[test]
fn collection_errors_leave_the_variable_unchanged_with_a_warning() {
    let story = story_with(
        vec![
            VarDecl::game("bag", strs(&["a"])),
            VarDecl::game("m", Value::empty_map(ScalarType::Int)),
            VarDecl::game("n", Value::Int(1)),
        ],
        |s| {
            vec![
                set("bag", "remove_at(bag, 3)"),
                set("n", "m[\"ghost\"]"),
                set("n", "len(bag) + 1"),
                say(s, "end"),
                node(NodeKind::End),
            ]
        },
    );
    let mut vm = Vm::from_start(story).unwrap();
    assert_eq!(lines(&mut vm), ["end"]);
    assert_eq!(vm.variable("bag"), Some(&strs(&["a"])));
    assert_eq!(vm.variable("n"), Some(&Value::Int(2)));
    let warnings = vm.take_warnings();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("out of bounds"), "{warnings:?}");
    assert!(warnings[1].contains("ghost"), "{warnings:?}");
}

#[test]
fn the_size_limit_is_enforced_at_runtime() {
    let full = Value::list(
        ScalarType::Int,
        (0..orbit_core::MAX_COLLECTION_LEN as i64).map(Value::Int),
    );
    let story = story_with(vec![VarDecl::game("big", full.clone())], |s| {
        vec![set("big", "push(big, 1)"), say(s, "end"), node(NodeKind::End)]
    });
    let mut vm = Vm::from_start(story).unwrap();
    assert_eq!(lines(&mut vm), ["end"]);
    assert_eq!(vm.variable("big"), Some(&full));
    assert!(vm.take_warnings()[0].contains("too large"));
}

#[test]
fn empty_literals_take_the_declared_type() {
    let story = story_with(vec![VarDecl::game("bag", strs(&["a"]))], |s| {
        vec![set("bag", "[]"), say(s, "end"), node(NodeKind::End)]
    });
    let mut vm = Vm::from_start(story).unwrap();
    lines(&mut vm);
    assert_eq!(vm.variable("bag"), Some(&Value::empty_list(ScalarType::Str)));
}

#[test]
fn ui_operations_on_collections() {
    let story = story_with(
        vec![
            VarDecl::game("bag", strs(&[])),
            VarDecl::game("m", Value::empty_map(ScalarType::Float)),
        ],
        |s| vec![say(s, "a"), say(s, "b"), node(NodeKind::End)],
    );
    let mut vm = Vm::from_start(story).unwrap();
    assert!(matches!(vm.step(), StepResult::Say { .. }));
    let push = VarOp::new("bag", OpKind::Push, Operand::Lit("potion".into()));
    vm.apply_var_op(&push).unwrap();
    vm.apply_var_op(&push).unwrap();
    let set = VarOp::new("m", OpKind::MapSet, Operand::Lit(Value::Int(2)))
        .with_key(Operand::Lit("hp".into()));
    vm.apply_var_op(&set).unwrap();
    assert_eq!(vm.variable("bag"), Some(&strs(&["potion", "potion"])));
    assert_eq!(
        vm.variable("m"),
        Some(&Value::map(ScalarType::Float, [("hp", Value::Float(2.0))]))
    );
    let bad = VarOp::new("bag", OpKind::RemoveAt, Operand::Lit(Value::Int(9)));
    assert!(vm.apply_var_op(&bad).is_err());
    assert!(vm.eval_condition(&Condition::cmp("bag", Cmp::LenEq, 2i64)));
    assert!(vm.eval_condition(&Condition::cmp("m", Cmp::HasKey, "hp")));
    // UI changes are part of the current rollback point: going forward and
    // back keeps them.
    assert!(matches!(vm.step(), StepResult::Say { .. }));
    vm.rollback(1).unwrap();
    assert_eq!(vm.variable("bag"), Some(&strs(&["potion", "potion"])));
}

#[test]
fn persistent_collections_survive_and_are_type_checked() {
    let story = story_with(
        vec![VarDecl::persistent("endings", Value::empty_map(ScalarType::Bool))],
        |s| {
            vec![
                set("endings", "insert(endings, \"good\", true)"),
                say(s, "end"),
                node(NodeKind::End),
            ]
        },
    );
    let mut vm = Vm::from_start(story.clone()).unwrap();
    lines(&mut vm);
    let saved = vm.persistent_vars().clone();
    // Round trip through the RON the host writes.
    let text = ron::to_string(&saved).unwrap();
    let back: BTreeMap<String, Value> = ron::from_str(&text).unwrap();
    let mut next = Vm::from_start(story.clone()).unwrap();
    next.set_persistent_vars(&back);
    assert_eq!(
        next.variable("endings"),
        Some(&Value::map(ScalarType::Bool, [("good", Value::Bool(true))]))
    );
    // A stored value of another type (an old scalar, a list of ints) is
    // replaced by the initial value.
    for wrong in [Value::Int(3), Value::list(ScalarType::Int, [Value::Int(1)])] {
        let mut other = Vm::from_start(story.clone()).unwrap();
        other.set_persistent_vars(&BTreeMap::from([("endings".to_owned(), wrong)]));
        assert_eq!(other.variable("endings"), Some(&Value::empty_map(ScalarType::Bool)));
    }
}

/// ADR 0016 risk check: 1 000 rollback steps over a list of 1 000 elements.
/// Unchanged collections are shared between checkpoints (an `Arc` clone),
/// so the history does not hold 1 000 copies of the list.
#[test]
fn rollback_over_a_large_list_shares_unchanged_collections() {
    const STEPS: usize = 1000;
    let big = Value::list(ScalarType::Int, (0..1000).map(Value::Int));
    let story = story_with(
        vec![
            VarDecl::game("big", big.clone()),
            VarDecl::game("log", Value::list(ScalarType::Int, (0..1000).map(Value::Int))),
            VarDecl::game("n", Value::Int(0)),
        ],
        |s| {
            let top = NodeId::new();
            vec![
                Node { id: top, kind: NodeKind::Label { name: "top".into() } },
                set("n", "n + 1"),
                // One list changes every step (a copy of 1 000 elements)...
                set("log", "push(remove_at(log, 0), n)"),
                say(s, "tick"),
                node(NodeKind::Jump { target: top }),
            ]
        },
    );
    let mut vm = Vm::from_start(story).unwrap();
    vm.set_history_capacity(STEPS + 1);
    for _ in 0..=STEPS {
        assert!(matches!(vm.step(), StepResult::Say { .. }));
    }
    assert_eq!(vm.variable("n"), Some(&Value::Int(STEPS as i64 + 1)));
    // ... the other never does: every checkpoint points to the same list.
    let history = vm.history_snapshot(STEPS + 1, 0);
    assert_eq!(history.checkpoints.len(), STEPS + 1);
    let Some(Value::List(_, current)) = vm.variable("big") else {
        panic!("big is a list")
    };
    for c in &history.checkpoints {
        let Some(Value::List(_, l)) = c.state.vars.get("big") else {
            panic!("big is a list")
        };
        assert!(Arc::ptr_eq(l, current), "unchanged lists are shared");
    }
    // Roll all the way back, one step at a time: every state is exact.
    for back in 1..=STEPS {
        vm.rollback(1).unwrap();
        let n = (STEPS + 1 - back) as i64;
        assert_eq!(vm.variable("n"), Some(&Value::Int(n)));
        let Some(Value::List(_, log)) = vm.variable("log") else {
            panic!("log is a list")
        };
        assert_eq!(log.len(), 1000);
        assert_eq!(log.last(), Some(&Value::Int(n)));
    }
    assert_eq!(vm.variable("big"), Some(&big));
}
