//! Schema v15 (ADR 0015, M8-01): `NodeKind::Script` and `Expr::Rhai`.

use orbit_core::{
    BinOp, ChoiceOption, Expr, Node, NodeId, NodeKind, Scene, SceneId, ScriptBranch, Story,
    StringId, ValidationError, Value, VarDecl, validate,
};

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

fn story(nodes: Vec<Node>) -> Story {
    let mut story = Story::new("t");
    story.variables.push(VarDecl::game("gold", Value::Int(1)));
    story.scenes.push(Scene {
        id: SceneId::new(),
        name: "s".into(),
        nodes,
    });
    story
}

fn script_errors(story: &Story) -> Vec<String> {
    validate(story)
        .errors
        .iter()
        .filter_map(|e| match e {
            ValidationError::InvalidScript { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn script_nodes_round_trip_and_their_targets_are_checked() {
    let win = node(NodeKind::Label { name: "win".into() });
    let script = node(NodeKind::Script {
        source: "branch(\"win\");\n// done".into(),
        branches: vec![ScriptBranch {
            name: "win".into(),
            target: win.id,
        }],
        on_error: Some(win.id),
    });
    assert_eq!(script.kind.targets(), vec![win.id, win.id]);
    let s = story(vec![node(NodeKind::Label { name: "a".into() }), script, win, node(NodeKind::End)]);
    let text = s.to_ron().unwrap();
    assert!(text.contains("Script("), "{text}");
    let back = Story::from_ron(&text).unwrap();
    assert_eq!(back, s);
    assert!(validate(&s).errors.is_empty(), "{:#?}", validate(&s).errors);
    assert!(validate(&s).warnings.is_empty(), "{:#?}", validate(&s).warnings);
    // A branch towards a node that is not a label is an error.
    let mut bad = s.clone();
    let end = bad.scenes[0].nodes[3].id;
    if let NodeKind::Script { branches, .. } = &mut bad.scenes[0].nodes[1].kind {
        branches[0].target = end;
    }
    assert!(
        validate(&bad)
            .errors
            .iter()
            .any(|e| matches!(e, ValidationError::TargetNotLabel { .. }))
    );
}

#[test]
fn empty_scripts_bad_branches_and_misplaced_rhai_are_errors() {
    let l = node(NodeKind::Label { name: "l".into() });
    let s = story(vec![
        node(NodeKind::Script {
            source: "  ".into(),
            branches: vec![
                ScriptBranch { name: "x".into(), target: l.id },
                ScriptBranch { name: "x".into(), target: l.id },
                ScriptBranch { name: "".into(), target: l.id },
            ],
            on_error: None,
        }),
        node(NodeKind::SetVar {
            var: "gold".into(),
            value: Expr::Rhai("1".into()),
        }),
        node(NodeKind::If {
            condition: Expr::binary(BinOp::And, Expr::Rhai("true".into()), Expr::lit(true)),
            then_target: l.id,
            else_target: None,
        }),
        node(NodeKind::If {
            condition: Expr::Rhai("get_var(\"gold\") > 0".into()),
            then_target: l.id,
            else_target: None,
        }),
        node(NodeKind::Choice {
            options: vec![ChoiceOption {
                text_id: StringId::new(),
                target: l.id,
                condition: Some(Expr::Rhai(" ".into())),
            }],
        }),
        l,
        node(NodeKind::End),
    ]);
    let errors = script_errors(&s);
    assert!(errors.iter().any(|m| m.contains("empty")), "{errors:?}");
    assert!(errors.iter().any(|m| m.contains("twice")), "{errors:?}");
    assert!(errors.iter().any(|m| m.contains("no name")), "{errors:?}");
    assert_eq!(
        errors.iter().filter(|m| m.contains("whole condition")).count(),
        2,
        "SetVar and nested: {errors:?}"
    );
    assert!(errors.iter().any(|m| m.contains("Rhai condition is empty")), "{errors:?}");
}

#[test]
fn rhai_expressions_have_a_text_form() {
    let e = Expr::parse("rhai: get_var(\"a\") > 1").unwrap();
    assert_eq!(e, Expr::Rhai("get_var(\"a\") > 1".into()));
    assert_eq!(e.to_string(), "rhai: get_var(\"a\") > 1");
    assert_eq!(Expr::parse(&e.to_string()).unwrap(), e);
    // Without a host, the core evaluator refuses it with a clear error.
    assert!(e.eval(&|_| None).unwrap_err().to_string().contains("script"));
    assert!(e.variables().is_empty());
}
