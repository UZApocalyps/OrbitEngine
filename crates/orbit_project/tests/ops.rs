//! The M3c commands and compound operations: every edit is one undo entry
//! and comes back exactly on undo.

use orbit_core::{
    AssetKind, BinOp, ChoiceOption, Color, CustomData, Expr, Node, NodeId, NodeKind, Position,
    PositionDef, Story, Value, VarScope, validate,
};
use orbit_project::ops::{self, NodeClip, NodeDraft, ShowLook};
use orbit_project::{Command, Project, ProjectError};

fn project() -> (Project, orbit_core::SceneId) {
    let mut p = Project::new(Story::new("t"));
    let scene = ops::add_scene(&mut p, "intro").unwrap();
    (p, scene)
}

fn say(p: &mut Project, scene: orbit_core::SceneId, index: usize, text: &str) -> NodeId {
    let draft = NodeDraft::say(None, text);
    let id = draft.node.id;
    ops::insert_drafts(p, scene, index, vec![draft], "Ajouter une réplique").unwrap();
    id
}

fn order(p: &Project, scene: orbit_core::SceneId) -> Vec<NodeId> {
    p.story()
        .scene(scene)
        .unwrap()
        .nodes
        .iter()
        .map(|n| n.id)
        .collect()
}

/// Runs `edit`, checks the story changed, then that one undo restores it and
/// one redo reapplies it. The story is left as it was before the edit.
fn undoable(p: &mut Project, edit: impl FnOnce(&mut Project)) {
    let before = p.story().clone();
    edit(p);
    let after = p.story().clone();
    assert_ne!(before, after, "the edit changed nothing");
    assert!(p.undo().unwrap());
    assert_eq!(*p.story(), before, "undo must restore the story exactly");
    assert!(p.redo().unwrap());
    assert_eq!(*p.story(), after, "redo must reapply the edit");
    assert!(p.undo().unwrap());
    assert_eq!(*p.story(), before);
}

#[test]
fn replace_node_kind_round_trips_for_every_kind() {
    let (mut p, scene) = project();
    let label = NodeDraft::label("end");
    let label_id = label.node.id;
    let character = ops::add_character(&mut p, "Ana", Color::WHITE).unwrap();
    let img = ops::add_asset(&mut p, AssetKind::Image, "a/bg.png").unwrap();
    let music = ops::add_asset(&mut p, AssetKind::Music, "a/m.ogg").unwrap();
    let kinds = vec![
        NodeKind::Label { name: "x".into() },
        NodeKind::Choice {
            options: vec![ChoiceOption {
                text_id: orbit_core::StringId::new(),
                target: label_id,
                condition: Some(Expr::var("a")),
            }],
        },
        NodeKind::Jump { target: label_id },
        NodeKind::SetVar {
            var: "a".into(),
            value: Expr::lit(true),
        },
        NodeKind::If {
            condition: Expr::var("a"),
            then_target: label_id,
            else_target: None,
        },
        NodeKind::Background {
            asset: img,
            transition: Some(orbit_core::Transition::dissolve(1.0)),
        },
        NodeKind::Show {
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
            character,
            expression: "x".into(),
            position: Position::Left,
            transition: None,
        },
        NodeKind::Hide {
            character,
            transition: None,
        },
        NodeKind::PlayMusic {
            asset: music,
            fade_in: 1.0,
            looping: true,
        },
        NodeKind::StopMusic { fade_out: 2.0 },
        NodeKind::Wait { seconds: 1.5 },
        NodeKind::Call { label: label_id },
        NodeKind::Return,
        NodeKind::End,
        NodeKind::Custom {
            plugin_id: "acme.fx".into(),
            kind: "k".into(),
            version: 1,
            data: CustomData::Map(vec![("n".into(), CustomData::Int(3))]),
        },
        NodeKind::OpenScreen {
            screen: "phone".into(),
            wait: true,
            result: Some("picked".into()),
            params: vec![orbit_core::ScreenParam {
                name: "gold".into(),
                value: orbit_core::Operand::Var("a".into()),
            }],
        },
        NodeKind::CloseScreen {
            screen: "phone".into(),
        },
    ];
    ops::insert_drafts(&mut p, scene, 0, vec![label], "label").unwrap();
    let host = say(&mut p, scene, 1, "host");
    for kind in kinds {
        undoable(&mut p, |p| {
            p.execute(Command::SetNodeKind {
                id: host,
                kind: kind.clone(),
                expected_version: None,
            })
            .unwrap();
            assert_eq!(p.story().node(host).unwrap().kind, kind);
        });
    }
}

#[test]
fn a_targeted_label_cannot_change_kind() {
    let (mut p, scene) = project();
    let label = NodeDraft::label("l");
    let lid = label.node.id;
    let jump = NodeDraft::plain(NodeKind::Jump { target: lid });
    ops::insert_drafts(&mut p, scene, 0, vec![label, jump], "x").unwrap();
    let err = p.execute(Command::SetNodeKind {
        id: lid,
        kind: NodeKind::End,
        expected_version: None,
    });
    assert!(err.is_err());
    // Renaming a label stays possible.
    p.execute(Command::SetNodeKind {
        id: lid,
        kind: NodeKind::Label { name: "m".into() },
        expected_version: None,
    })
    .unwrap();
}

#[test]
fn moving_nodes_and_scenes_is_undoable() {
    let (mut p, scene) = project();
    let ids: Vec<NodeId> = (0..5)
        .map(|i| say(&mut p, scene, i, &format!("l{i}")))
        .collect();
    undoable(&mut p, |p| {
        p.execute(Command::MoveNode {
            id: ids[0],
            scene,
            index: 4,
            expected_version: None,
        })
        .unwrap();
        assert_eq!(order(p, scene), [ids[1], ids[2], ids[3], ids[4], ids[0]]);
    });
    // A block keeps its relative order and lands before the anchor.
    undoable(&mut p, |p| {
        ops::move_nodes(p, &[ids[3], ids[1]], scene, 0).unwrap();
        assert_eq!(order(p, scene), [ids[1], ids[3], ids[0], ids[2], ids[4]]);
    });
    // Moving into another scene.
    let other = ops::add_scene(&mut p, "other").unwrap();
    undoable(&mut p, |p| {
        ops::move_nodes(p, &[ids[2]], other, 0).unwrap();
        assert_eq!(order(p, other), [ids[2]]);
        assert_eq!(order(p, scene).len(), 4);
    });
    // Out of bounds is refused.
    assert!(
        p.execute(Command::MoveNode {
            id: ids[0],
            scene,
            index: 99,
            expected_version: None,
        })
        .is_err()
    );
    // Scenes.
    let first = p.story().scenes[0].id;
    undoable(&mut p, |p| {
        p.execute(Command::MoveScene {
            id: first,
            index: 1,
        })
        .unwrap();
        assert_eq!(p.story().scenes[1].id, first);
    });
    undoable(&mut p, |p| {
        p.execute(Command::RenameScene {
            id: first,
            name: "Prologue".into(),
        })
        .unwrap();
    });
}

#[test]
fn nudging_moves_the_selection_one_step() {
    let (mut p, scene) = project();
    let ids: Vec<NodeId> = (0..4)
        .map(|i| say(&mut p, scene, i, &format!("l{i}")))
        .collect();
    assert!(ops::nudge_nodes(&mut p, &[ids[1]], -1).unwrap());
    assert_eq!(order(&p, scene), [ids[1], ids[0], ids[2], ids[3]]);
    assert!(
        !ops::nudge_nodes(&mut p, &[ids[1]], -1).unwrap(),
        "top edge"
    );
    assert!(ops::nudge_nodes(&mut p, &[ids[1], ids[0]], 1).unwrap());
    assert_eq!(order(&p, scene), [ids[2], ids[1], ids[0], ids[3]]);
    assert!(
        !ops::nudge_nodes(&mut p, &[ids[3]], 1).unwrap(),
        "bottom edge"
    );
}

#[test]
fn removing_a_scene_is_undoable_and_checks_references() {
    let (mut p, scene) = project();
    let label = NodeDraft::label("l");
    let lid = label.node.id;
    ops::insert_drafts(&mut p, scene, 0, vec![label], "x").unwrap();
    let other = ops::add_scene(&mut p, "b").unwrap();
    ops::insert_drafts(
        &mut p,
        other,
        0,
        vec![NodeDraft::plain(NodeKind::Jump { target: lid })],
        "x",
    )
    .unwrap();
    // The label of scene 1 is targeted from scene 2.
    assert!(p.execute(Command::RemoveScene { id: scene }).is_err());
    undoable(&mut p, |p| {
        p.execute(Command::RemoveScene { id: other }).unwrap();
    });
}

#[test]
fn copy_paste_regenerates_ids_and_keeps_internal_references() {
    let (mut p, scene) = project();
    let label = NodeDraft::label("loop");
    let lid = label.node.id;
    let text = orbit_core::StringId::new();
    let choice = NodeDraft {
        node: Node {
            id: NodeId::new(),
            kind: NodeKind::Choice {
                options: vec![ChoiceOption {
                    text_id: text,
                    target: lid,
                    condition: None,
                }],
            },
        },
        strings: vec![(text, "again".into())],
    };
    let jump = NodeDraft::plain(NodeKind::Jump { target: lid });
    let say_draft = NodeDraft::say(None, "hello");
    let (label_id, choice_id, jump_id, say_id) = (
        label.node.id,
        choice.node.id,
        jump.node.id,
        say_draft.node.id,
    );
    ops::insert_drafts(&mut p, scene, 0, vec![label, say_draft, choice, jump], "x").unwrap();

    let clip = ops::copy_nodes(p.story(), &[jump_id, label_id, choice_id, say_id]);
    assert_eq!(clip.nodes.len(), 4);
    assert_eq!(clip.strings.len(), 2);

    undoable(&mut p, |p| {
        let new = ops::paste_nodes(p, &clip, scene, 4).unwrap();
        assert_eq!(new.len(), 4);
        assert!(
            new.iter()
                .all(|id| ![label_id, choice_id, jump_id, say_id].contains(id))
        );
        // The pasted jump and choice target the pasted label, not the original.
        let story = p.story();
        let NodeKind::Jump { target } = &story.node(new[3]).unwrap().kind else {
            panic!("jump expected")
        };
        assert_eq!(*target, new[0]);
        let NodeKind::Choice { options } = &story.node(new[2]).unwrap().kind else {
            panic!("choice expected")
        };
        assert_eq!(options[0].target, new[0]);
        // Fresh strings, same text.
        assert_ne!(options[0].text_id, text);
        assert_eq!(story.text(options[0].text_id), Some("again"));
    });
    assert!(validate(p.story()).errors.is_empty());

    // Pasting a lone jump keeps its target (a label that still exists).
    let lone = ops::copy_nodes(p.story(), &[jump_id]);
    let new = ops::paste_nodes(&mut p, &lone, scene, 0).unwrap();
    assert_eq!(
        p.story().node(new[0]).unwrap().kind,
        NodeKind::Jump { target: lid }
    );
    assert!(ops::paste_nodes(&mut p, &NodeClip::default(), scene, 0).is_err());
}

#[test]
fn duplicate_inserts_after_the_last_selected_node() {
    let (mut p, scene) = project();
    let a = say(&mut p, scene, 0, "a");
    let b = say(&mut p, scene, 1, "b");
    let c = say(&mut p, scene, 2, "c");
    undoable(&mut p, |p| {
        let new = ops::duplicate_nodes(p, &[b, a]).unwrap();
        let o = order(p, scene);
        assert_eq!(o[..2], [a, b]);
        assert_eq!(o[2..4], new[..]);
        assert_eq!(o[4], c);
    });
}

#[test]
fn deleting_nodes_removes_their_strings_and_refuses_dangling_references() {
    let (mut p, scene) = project();
    let label = NodeDraft::label("l");
    let lid = label.node.id;
    let jump = NodeDraft::plain(NodeKind::Jump { target: lid });
    let jid = jump.node.id;
    let line = NodeDraft::say(None, "bye");
    let line_id = line.node.id;
    ops::insert_drafts(&mut p, scene, 0, vec![jump, label, line], "x").unwrap();

    // The label alone is still targeted.
    assert!(ops::delete_nodes(&mut p, &[lid]).is_err());
    assert_eq!(order(&p, scene).len(), 3);

    // The jump and its label together can go, whatever the order.
    undoable(&mut p, |p| {
        ops::delete_nodes(p, &[lid, jid, line_id]).unwrap();
        assert!(order(p, scene).is_empty());
        assert!(p.story().strings.entries.is_empty());
    });
}

#[test]
fn characters_and_expressions() {
    let (mut p, scene) = project();
    let img = ops::add_asset(&mut p, AssetKind::Image, "a/ana.png").unwrap();
    let ana = ops::add_character(&mut p, "Ana", Color::rgb(1, 2, 3)).unwrap();
    let mut def = p.story().character(ana).unwrap().clone();
    def.expressions.push(orbit_core::Expression {
        name: "happy".into(),
        image: img,
        motion: None,
    });
    def.default_position = Some(Position::Right);
    p.execute(Command::SetCharacter {
        id: def.id,
        character: Some(def),
        index: None,
    })
    .unwrap();
    let show = NodeDraft::plain(NodeKind::Show {
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
        character: ana,
        expression: "happy".into(),
        position: Position::Right,
        transition: None,
    });
    let show_id = show.node.id;
    ops::insert_drafts(&mut p, scene, 0, vec![show], "x").unwrap();

    // Renaming the expression updates the Show nodes (one undo entry).
    undoable(&mut p, |p| {
        ops::rename_expression(p, ana, "happy", "joyeuse").unwrap();
        let NodeKind::Show { expression, .. } = &p.story().node(show_id).unwrap().kind else {
            panic!()
        };
        assert_eq!(expression, "joyeuse");
    });
    assert!(validate(p.story()).errors.is_empty());
    // Refusals.
    assert!(ops::rename_expression(&mut p, ana, "happy", "").is_err());
    assert!(ops::rename_expression(&mut p, ana, "nope", "x").is_err());

    // A used character cannot be removed.
    assert!(ops::delete_character(&mut p, ana).is_err());
    ops::delete_nodes(&mut p, &[show_id]).unwrap();
    undoable(&mut p, |p| ops::delete_character(p, ana).unwrap());
    // The default position survives the RON round trip (and is absent when unset).
    let ron = p.story().to_ron().unwrap();
    assert!(ron.contains("default_position"));
    let back = Story::from_ron_unchecked(&ron).unwrap();
    assert_eq!(back, *p.story());
}

#[test]
fn variables_rename_updates_every_usage_or_refuses() {
    let (mut p, scene) = project();
    ops::add_variable(&mut p, orbit_core::VarDecl::game("affinity", Value::Int(0))).unwrap();
    ops::add_variable(
        &mut p,
        orbit_core::VarDecl::persistent("seen", Value::Bool(false)),
    )
    .unwrap();
    let label = NodeDraft::label("l");
    let lid = label.node.id;
    let set = NodeDraft::plain(NodeKind::SetVar {
        var: "affinity".into(),
        value: Expr::binary(BinOp::Add, Expr::var("affinity"), Expr::lit(1i64)),
    });
    let cond = NodeDraft::plain(NodeKind::If {
        condition: Expr::binary(BinOp::Gt, Expr::var("affinity"), Expr::lit(2i64)),
        then_target: lid,
        else_target: None,
    });
    let text = orbit_core::StringId::new();
    let choice = NodeDraft {
        node: Node {
            id: NodeId::new(),
            kind: NodeKind::Choice {
                options: vec![ChoiceOption {
                    text_id: text,
                    target: lid,
                    condition: Some(Expr::var("affinity")),
                }],
            },
        },
        strings: vec![(text, "go".into())],
    };
    ops::insert_drafts(&mut p, scene, 0, vec![label, set, cond, choice], "x").unwrap();
    assert_eq!(ops::variable_users(p.story(), "affinity").len(), 3);

    undoable(&mut p, |p| {
        ops::rename_variable(p, "affinity", "love").unwrap();
        assert!(ops::variable_users(p.story(), "affinity").is_empty());
        assert_eq!(ops::variable_users(p.story(), "love").len(), 3);
        assert_eq!(p.story().variables[0].name, "love");
    });
    // Bad names and clashes are refused with a message and change nothing.
    let before = p.story().clone();
    for bad in ["", "1abc", "has space", "true", "seen"] {
        let err = ops::rename_variable(&mut p, "affinity", bad).unwrap_err();
        assert!(matches!(err, ProjectError::Rejected(_)), "{bad}: {err}");
    }
    assert_eq!(*p.story(), before);
    // A used variable cannot be deleted, an unused one can.
    assert!(ops::delete_variable(&mut p, "affinity").is_err());
    undoable(&mut p, |p| ops::delete_variable(p, "seen").unwrap());
    undoable(&mut p, |p| {
        ops::update_variable(p, "affinity", Value::Int(5), VarScope::Persistent).unwrap();
    });
}

#[test]
fn renaming_a_variable_follows_open_screen_result_and_params() {
    let (mut p, scene) = project();
    ops::add_variable(&mut p, orbit_core::VarDecl::game("gold", Value::Int(0))).unwrap();
    ops::add_variable(&mut p, orbit_core::VarDecl::game("picked", Value::Int(0))).unwrap();
    let open = NodeDraft::plain(NodeKind::OpenScreen {
        screen: "shop".into(),
        wait: true,
        result: Some("picked".into()),
        params: vec![orbit_core::ScreenParam {
            name: "g".into(),
            value: orbit_core::Operand::Var("gold".into()),
        }],
    });
    let id = open.node.id;
    ops::insert_drafts(&mut p, scene, 0, vec![open], "x").unwrap();
    assert_eq!(ops::variable_users(p.story(), "gold"), vec![id]);
    assert_eq!(ops::variable_users(p.story(), "picked"), vec![id]);
    assert!(ops::delete_variable(&mut p, "gold").is_err());
    ops::rename_variable(&mut p, "gold", "coins").unwrap();
    ops::rename_variable(&mut p, "picked", "choice").unwrap();
    match &p.story().node(id).unwrap().kind {
        NodeKind::OpenScreen { result, params, .. } => {
            assert_eq!(result.as_deref(), Some("choice"));
            assert_eq!(params[0].value, orbit_core::Operand::Var("coins".into()));
        }
        k => panic!("{k:?}"),
    }
}

#[test]
fn positions_and_assets_commands_are_undoable() {
    let (mut p, _) = project();
    undoable(&mut p, |p| {
        p.execute(Command::SetPositions {
            positions: vec![PositionDef {
                name: "far".into(),
                x: 0.1,
                y: 0.9,
            }],
        })
        .unwrap();
    });
    undoable(&mut p, |p| {
        ops::add_asset(p, AssetKind::Sound, "a/s.wav").unwrap();
    });
    // Registering the same path twice is idempotent.
    let a = ops::add_asset(&mut p, AssetKind::Sound, "a/s.wav").unwrap();
    let b = ops::add_asset(&mut p, AssetKind::Sound, "a/s.wav").unwrap();
    assert_eq!(a, b);
    assert!(ops::add_asset(&mut p, AssetKind::Image, "a/s.wav").is_err());
}

#[test]
fn unique_names() {
    assert_eq!(ops::unique_name("Scène", ["a"]), "Scène");
    assert_eq!(ops::unique_name("Scène", ["Scène"]), "Scène 2");
    assert_eq!(ops::unique_name("Scène", ["Scène", "Scène 2"]), "Scène 3");
}

fn show_node(p: &mut Project, scene: orbit_core::SceneId) -> (orbit_core::CharacterId, NodeId) {
    let img = ops::add_asset(p, AssetKind::Image, "a/ana.png").unwrap();
    let ana = ops::add_character(p, "Ana", Color::WHITE).unwrap();
    let mut def = p.story().character(ana).unwrap().clone();
    def.expressions.push(orbit_core::Expression {
        name: "happy".into(),
        image: img,
        motion: None,
    });
    p.execute(Command::SetCharacter {
        id: ana,
        character: Some(def),
        index: None,
    })
    .unwrap();
    let show = NodeDraft::plain(NodeKind::Show {
        character: ana,
        expression: "happy".into(),
        position: Position::Left,
        transition: None,
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
    });
    let id = show.node.id;
    ops::insert_drafts(p, scene, 0, vec![show], "x").unwrap();
    (ana, id)
}

#[test]
fn show_look_is_one_undo_entry_and_keeps_the_rest_of_the_node() {
    let (mut p, scene) = project();
    let (ana, show) = show_node(&mut p, scene);
    let mut look = ops::show_look(p.story(), show).unwrap();
    assert!(look.is_default_style());
    look.position = Position::Free { x: 0.3, y: 0.9 };
    look.set_effective_scale([0.8, 0.8]);
    look.flip_x = true;
    assert_eq!(look.scale_y, None, "uniform scale stores no scale_y");
    undoable(&mut p, |p| ops::set_show_look(p, show, &look).unwrap());
    ops::set_show_look(&mut p, show, &look).unwrap();
    assert_eq!(ops::show_look(p.story(), show), Some(look.clone()));
    let NodeKind::Show { expression, .. } = &p.story().node(show).unwrap().kind else {
        panic!()
    };
    assert_eq!(expression, "happy");
    assert!(validate(p.story()).errors.is_empty());

    // Renaming the expression keeps the look.
    ops::rename_expression(&mut p, ana, "happy", "joyeuse").unwrap();
    assert_eq!(ops::show_look(p.story(), show), Some(look.clone()));

    // Invalid looks are refused and change nothing.
    let before = p.story().clone();
    for bad in [
        ShowLook {
            scale: Some(0.0),
            ..look.clone()
        },
        ShowLook {
            scale: Some(f32::NAN),
            ..look.clone()
        },
        ShowLook {
            scale: Some(11.0),
            ..look.clone()
        },
        ShowLook {
            scale_y: Some(-1.0),
            ..look.clone()
        },
        ShowLook {
            position: Position::Free {
                x: f32::INFINITY,
                y: 0.5,
            },
            ..look.clone()
        },
        ShowLook {
            position: Position::Custom("nowhere".into()),
            ..look.clone()
        },
    ] {
        assert!(ops::set_show_look(&mut p, show, &bad).is_err(), "{bad:?}");
    }
    assert_eq!(*p.story(), before);
    // Not a Show node.
    let line = say(&mut p, scene, 1, "hi");
    assert!(ops::set_show_look(&mut p, line, &look).is_err());
}

#[test]
fn a_live_drag_of_a_show_is_one_undo_entry() {
    let (mut p, scene) = project();
    let (_ana, show) = show_node(&mut p, scene);
    let before = p.story().clone();
    let mut look = ops::show_look(p.story(), show).unwrap();
    // Many frames of one drag, then the gesture ends.
    for i in 1..=30 {
        look.position = Position::Free {
            x: 0.2 + i as f32 * 0.01,
            y: 0.95,
        };
        let c = ops::show_look_command(p.story(), show, &look).unwrap();
        p.execute_merged(c, "show-look").unwrap();
    }
    p.end_merge();
    let after = p.story().clone();
    assert_ne!(before, after);
    assert!(p.undo().unwrap());
    assert_eq!(*p.story(), before, "one undo reverts the whole drag");
    assert!(p.redo().unwrap());
    assert_eq!(*p.story(), after);
}

#[test]
fn default_scale_is_validated_undoable_and_round_trips() {
    let (mut p, _scene) = project();
    let ana = ops::add_character(&mut p, "Ana", Color::WHITE).unwrap();
    undoable(&mut p, |p| ops::set_default_scale(p, ana, 0.6).unwrap());
    ops::set_default_scale(&mut p, ana, 0.6).unwrap();
    for bad in [0.0, -1.0, f32::NAN, 100.0] {
        assert!(ops::set_default_scale(&mut p, ana, bad).is_err());
    }
    let ron = p.story().to_ron().unwrap();
    assert!(ron.contains("default_scale: 0.6"));
    assert_eq!(Story::from_ron(&ron).unwrap(), *p.story());
    // Back to 1.0: the field is not written any more.
    ops::set_default_scale(&mut p, ana, 1.0).unwrap();
    assert!(!p.story().to_ron().unwrap().contains("default_scale"));
}

#[test]
fn named_curves_are_created_renamed_with_their_usages_and_deleted() {
    use orbit_core::{Curve, Ease, Transition};
    let (mut p, scene) = project();
    let img = ops::add_asset(&mut p, AssetKind::Image, "a/bg.png").unwrap();
    undoable(&mut p, |p| {
        ops::set_curve(p, "snappy", Curve::css_ease_out()).unwrap();
    });
    ops::set_curve(&mut p, "snappy", Curve::css_ease_out()).unwrap();
    // A name chain and an empty name are refused.
    assert!(matches!(
        p.execute(Command::SetCurve {
            name: "alias".into(),
            curve: Some(Curve::Named("snappy".into())),
        }),
        Err(ProjectError::Command(orbit_project::CommandError::CurveChain(_)))
    ));
    assert!(ops::set_curve(&mut p, " ", Curve::LINEAR).is_err());

    let node = NodeDraft {
        node: Node {
            id: NodeId::new(),
            kind: NodeKind::Background {
                asset: img,
                transition: Some(Transition::dissolve(1.0).with_ease(Curve::Named("snappy".into()))),
            },
        },
        strings: vec![],
    };
    let node_id = node.node.id;
    ops::insert_drafts(&mut p, scene, 0, vec![node], "bg").unwrap();
    assert_eq!(ops::curve_users(p.story(), "snappy"), vec![node_id]);
    assert!(ops::delete_curve(&mut p, "snappy").is_err(), "in use");

    undoable(&mut p, |p| ops::rename_curve(p, "snappy", "quick").unwrap());
    ops::rename_curve(&mut p, "snappy", "quick").unwrap();
    assert!(!p.story().curves.contains_key("snappy"));
    assert_eq!(ops::curve_users(p.story(), "quick"), vec![node_id]);
    assert!(
        !validate(p.story())
            .warnings
            .iter()
            .any(|w| matches!(w, orbit_core::ValidationWarning::UnknownCurve { .. })),
        "no dangling reference"
    );
    assert!(ops::rename_curve(&mut p, "quick", "quick").is_ok());
    assert!(ops::rename_curve(&mut p, "nope", "x").is_err());

    undoable(&mut p, |p| ops::set_transition_ease(p, node_id, Some(Curve::Preset(Ease::OutBack))).unwrap());
    ops::set_transition_ease(&mut p, node_id, None).unwrap();
    assert!(ops::curve_users(p.story(), "quick").is_empty());
    undoable(&mut p, |p| ops::delete_curve(p, "quick").unwrap());
}
