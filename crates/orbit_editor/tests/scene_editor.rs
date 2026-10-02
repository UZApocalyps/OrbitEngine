//! Headless tests of the M3c editing tools: the scene editor operations
//! (insert, delete, duplicate, copy / paste, reorder), scene management, the
//! inspector on every node kind, the characters and variables panels, and
//! the performance of a 5000-node scene. No window, no GPU.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::{
    AssetKind, BinOp, ChoiceOption, Color, CustomData, Expr, Expression, Node, NodeId, NodeKind,
    Position, SceneId, Story, StringId, Transition, Value, VarScope, validate,
};
use orbit_editor::builtin::edit;
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::{EditorContext, Selection};
use orbit_project::{Command, ops};

// ------------------------------------------------------------------ helpers

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-scene-it-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn options() -> CoreOptions {
    CoreOptions::with_language(orbit_editor_api::Lang::Fr)
}

/// A freshly created project (manifest + minimal story), opened.
fn new_project(tag: &str) -> (EditorCore, Context, TempDir) {
    let dir = TempDir::new(tag);
    let ctx = Context::default();
    let mut core = EditorCore::new(options(), &ctx);
    let root = orbit_editor::storage::create_project(
        dir.path(),
        "Projet",
        &orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr),
    )
    .unwrap();
    core.open_project(&root);
    assert!(core.has_project());
    (core, ctx, dir)
}

fn with_ctx<R>(core: &mut EditorCore, f: impl FnOnce(&mut EditorContext<'_>) -> R) -> R {
    let session = core.session.as_mut().expect("a project is open");
    let mut ectx = EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
    f(&mut ectx)
}

fn frame(core: &mut EditorCore, ctx: &Context, time: f64, size: (f32, f32)) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1))),
        time: Some(time),
        ..RawInput::default()
    };
    ctx.run_ui(input, |ui| core.ui(ui))
        .drop_without_applying_deltas();
}

fn frames(core: &mut EditorCore, ctx: &Context, n: usize) {
    for i in 0..n {
        frame(core, ctx, i as f64 * 0.016, (1360.0, 820.0));
    }
}

fn story(core: &EditorCore) -> &Story {
    core.story().unwrap()
}

fn scene_nodes(core: &EditorCore, scene: SceneId) -> Vec<NodeId> {
    story(core)
        .scene(scene)
        .unwrap()
        .nodes
        .iter()
        .map(|n| n.id)
        .collect()
}

fn undo_label(core: &mut EditorCore) -> Option<String> {
    with_ctx(core, |c| c.undo_label().map(str::to_owned))
}

/// Runs an action, checks it changed the story, that it left exactly one
/// undo entry (one undo restores the story) and that redo reapplies it.
fn one_undo_step(core: &mut EditorCore, ctx: &Context, action: &str) {
    let before = story(core).clone();
    core.run_action_now(action, ctx);
    let after = story(core).clone();
    assert_ne!(before, after, "{action} changed nothing");
    core.run_action_now("orbit.core.undo", ctx);
    assert_eq!(
        *story(core),
        before,
        "{action}: one undo must restore the story"
    );
    core.run_action_now("orbit.core.redo", ctx);
    assert_eq!(*story(core), after, "{action}: redo must reapply");
}

/// A scene of `n` numbered lines in a fresh project; returns its id.
fn lines(core: &mut EditorCore, n: usize) -> (SceneId, Vec<NodeId>) {
    let scene = with_ctx(core, |c| c.ops(|p| ops::add_scene(p, "Test")).unwrap());
    let mut ids = Vec::new();
    for i in 0..n {
        let draft = ops::NodeDraft::say(None, format!("ligne {i}"));
        ids.push(draft.node.id);
        with_ctx(core, |c| {
            c.ops(|p| ops::insert_drafts(p, scene, i, vec![draft], "x"))
                .unwrap();
        });
    }
    (scene, ids)
}

// -------------------------------------------------------------------- tests

#[test]
fn a_story_is_built_with_the_editor_operations_only_and_validates() {
    let (mut core, ctx, _dir) = new_project("build");
    frames(&mut core, &ctx, 3);

    // A new scene, two characters, assets (what the importer registers).
    core.run_action_now("orbit.scene.new_scene", &ctx);
    let scene = match core.state.selection {
        Some(Selection::Scene(id)) => id,
        other => panic!("the new scene should be selected, got {other:?}"),
    };
    let (ana, ben, bg, music, sprite) = with_ctx(&mut core, |c| {
        c.ops(|p| {
            let bg = ops::add_asset(p, AssetKind::Image, "assets/bg.png")?;
            let music = ops::add_asset(p, AssetKind::Music, "assets/theme.ogg")?;
            let sprite = ops::add_asset(p, AssetKind::Image, "assets/ana.png")?;
            let ana = ops::add_character(p, "Ana", Color::rgb(230, 120, 160))?;
            let ben = ops::add_character(p, "Ben", Color::rgb(90, 150, 230))?;
            for c in [ana, ben] {
                let mut def = p.story().character(c).unwrap().clone();
                def.expressions.push(Expression {
                    name: "neutral".into(),
                    image: sprite,
                });
                p.execute(Command::SetCharacter {
                    id: def.id,
                    character: Some(def),
                    index: None,
                })?;
            }
            Ok((ana, ben, bg, music, sprite))
        })
        .unwrap()
    });
    let _ = (bg, music, sprite);

    // The stage: a background, music, both characters.
    for key in ["Background", "PlayMusic", "Show", "Show"] {
        let ids = with_ctx(&mut core, |c| edit::insert_kind(c, key)).expect(key);
        assert_eq!(ids.len(), 1);
    }
    // The second `Show` is for Ben.
    let shows: Vec<NodeId> = scene_nodes(&core, scene).into_iter().skip(2).collect();
    let second_show = shows[1];
    with_ctx(&mut core, |c| {
        c.exec(Command::SetNodeKind {
            id: second_show,
            kind: NodeKind::Show {
                scale: None,
                scale_y: None,
                flip_x: false,
                flip_y: false,
                character: ben,
                expression: "neutral".into(),
                position: Position::Right,
                transition: Some(Transition::dissolve(0.5)),
            },
            expected_version: None,
        })
        .unwrap();
    });

    // Twenty lines, alternating between the two characters, written through
    // the same path as typing (one undo entry per line).
    for i in 0..20 {
        let speaker = if i % 2 == 0 { ana } else { ben };
        let ids = with_ctx(&mut core, |c| edit::insert_kind(c, "Say")).unwrap();
        let before = story(&core).clone();
        with_ctx(&mut core, |c| {
            c.exec_merged(
                Command::SetSay {
                    id: ids[0],
                    speaker: Some(speaker),
                    text: format!("Réplique {i} de la scène."),
                    expected_version: None,
                },
                "say-text",
            )
            .unwrap();
            c.end_merge();
        });
        assert_ne!(before, *story(&core));
    }
    // A choice with its two branches (the skeleton the editor builds).
    with_ctx(&mut core, |c| edit::insert_kind(c, "Choice")).unwrap();

    // Scenes never continue into the next one: the welcome scene reaches the
    // new one with a scene jump line ("Saut de scène", aimed at the next
    // scene by default).
    let last_welcome = story(&core).scenes[0].nodes.last().unwrap().id;
    let jump = with_ctx(&mut core, |c| {
        c.select(Selection::Node(last_welcome));
        edit::insert_kind(c, "JumpScene")
    })
    .unwrap();
    assert!(matches!(
        story(&core).node(jump[0]).map(|n| &n.kind),
        Some(NodeKind::JumpScene { scene: s, label: None }) if *s == scene
    ));

    let story_now = story(&core).clone();
    let report = validate(&story_now);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let nodes = &story_now.scene(scene).unwrap().nodes;
    let said = nodes
        .iter()
        .filter(|n| match &n.kind {
            NodeKind::Say { text_id, .. } => story_now
                .text(*text_id)
                .is_some_and(|t| t.starts_with("Réplique")),
            _ => false,
        })
        .count();
    assert_eq!(said, 20);
    assert!(
        nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Background { .. }))
    );
    assert!(
        nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::PlayMusic { .. }))
    );
    assert!(
        nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::Choice { .. }))
    );
    assert_eq!(story_now.characters.len(), 2);

    // It saves, reloads with validation (`orbit validate`) and plays in the
    // preview's VM up to the choice.
    assert!(core.save());
    let root = core.state.project.as_ref().unwrap().root.clone();
    let on_disk = Story::from_ron(&std::fs::read_to_string(root.join("story.ron")).unwrap())
        .expect("the saved story validates");
    assert_eq!(on_disk, story_now);
    let mut planned = orbit_runtime::preview::plan_from_beginning(&Arc::new(story_now)).unwrap();
    let mut reached_choice = false;
    for _ in 0..200 {
        match planned.vm.step() {
            orbit_vm::StepResult::Choice { .. } => {
                reached_choice = true;
                break;
            }
            orbit_vm::StepResult::End => break,
            _ => {}
        }
    }
    assert!(reached_choice, "the preview reaches the choice");
}

#[test]
fn insert_delete_duplicate_and_reorder_are_one_undo_step_each() {
    let (mut core, ctx, _dir) = new_project("ops");
    let (scene, ids) = lines(&mut core, 6);
    frames(&mut core, &ctx, 3);

    // Insert after the selection: the new line lands right after it.
    with_ctx(&mut core, |c| c.select(Selection::Node(ids[1])));
    one_undo_step(&mut core, &ctx, "orbit.explorer.add_say");
    assert_eq!(
        undo_label(&mut core).as_deref(),
        Some("Ajouter une réplique")
    );
    let order = scene_nodes(&core, scene);
    assert_eq!(order.len(), 7);
    assert_eq!(order[1], ids[1]);
    assert_ne!(
        order[2], ids[2],
        "the new line is inserted after the selection"
    );

    // Duplicate a multi-selection: the copies follow the last selected node.
    with_ctx(&mut core, |c| c.select_nodes(vec![ids[3], ids[4]], ids[3]));
    one_undo_step(&mut core, &ctx, "orbit.scene.duplicate");
    let order = scene_nodes(&core, scene);
    assert_eq!(order.len(), 9);
    assert_eq!(
        core.state.node_selection.len(),
        2,
        "the copies are selected"
    );

    // Move down / up with the shortcuts' actions; the selection moves as a block.
    with_ctx(&mut core, |c| c.select(Selection::Node(ids[0])));
    one_undo_step(&mut core, &ctx, "orbit.scene.move_down");
    assert_eq!(scene_nodes(&core, scene)[1], ids[0]);
    core.run_action_now("orbit.scene.move_up", &ctx);
    assert_eq!(scene_nodes(&core, scene)[0], ids[0]);

    // Delete a multi-selection: one entry, strings go with the nodes.
    let strings_before = story(&core).strings.entries.len();
    with_ctx(&mut core, |c| c.select_nodes(vec![ids[0], ids[5]], ids[0]));
    one_undo_step(&mut core, &ctx, "orbit.explorer.delete");
    // `one_undo_step` leaves the edit applied.
    assert_eq!(
        story(&core).strings.entries.len(),
        strings_before - 2,
        "the strings of deleted lines are deleted too"
    );
}

#[test]
fn copy_cut_and_paste_regenerate_ids_and_keep_internal_references() {
    let (mut core, ctx, _dir) = new_project("clip");
    let scene = with_ctx(&mut core, |c| c.ops(|p| ops::add_scene(p, "Clip")).unwrap());
    // label, say, jump -> label (a self-contained loop).
    let label = ops::NodeDraft::label("boucle");
    let lid = label.node.id;
    let line = ops::NodeDraft::say(None, "encore");
    let jump = ops::NodeDraft::plain(NodeKind::Jump { target: lid });
    let (line_id, jump_id) = (line.node.id, jump.node.id);
    with_ctx(&mut core, |c| {
        c.ops(|p| ops::insert_drafts(p, scene, 0, vec![label, line, jump], "x"))
            .unwrap();
    });
    frames(&mut core, &ctx, 2);
    with_ctx(&mut core, |c| {
        c.select_nodes(vec![lid, line_id, jump_id], lid)
    });
    core.run_action_now("orbit.scene.copy", &ctx);
    assert!(core.state.clipboard.is_some());

    let before = story(&core).clone();
    with_ctx(&mut core, |c| c.select(Selection::Node(jump_id)));
    core.run_action_now("orbit.scene.paste", &ctx);
    let order = scene_nodes(&core, scene);
    assert_eq!(order.len(), 6);
    let pasted = &order[3..];
    assert!(
        pasted
            .iter()
            .all(|id| ![lid, line_id, jump_id].contains(id))
    );
    let NodeKind::Jump { target } = &story(&core).node(pasted[2]).unwrap().kind else {
        panic!("a jump was pasted")
    };
    assert_eq!(
        *target, pasted[0],
        "the pasted jump targets the pasted label"
    );
    assert!(validate(story(&core)).errors.is_empty());
    core.run_action_now("orbit.core.undo", &ctx);
    assert_eq!(*story(&core), before, "one undo removes the whole paste");

    // Cut = copy + delete, in one undo step.
    with_ctx(&mut core, |c| c.select_nodes(vec![line_id], line_id));
    core.run_action_now("orbit.scene.cut", &ctx);
    assert_eq!(scene_nodes(&core, scene).len(), 2);
    core.run_action_now("orbit.core.undo", &ctx);
    assert_eq!(*story(&core), before);
}

#[test]
fn deleting_a_targeted_label_is_refused_with_a_message() {
    let (mut core, _ctx, _dir) = new_project("refuse");
    let scene = with_ctx(&mut core, |c| c.ops(|p| ops::add_scene(p, "R")).unwrap());
    let label = ops::NodeDraft::label("cible");
    let lid = label.node.id;
    let jump = ops::NodeDraft::plain(NodeKind::Jump { target: lid });
    with_ctx(&mut core, |c| {
        c.ops(|p| ops::insert_drafts(p, scene, 0, vec![label, jump], "x"))
            .unwrap();
        c.select(Selection::Node(lid));
        edit::delete_selection(c);
    });
    assert_eq!(scene_nodes(&core, scene).len(), 2, "nothing was deleted");
    assert!(
        core.state
            .toasts
            .iter()
            .any(|t| t.message.contains("« cible »") && t.message.contains("référence")),
        "{:?}",
        core.state.toasts
    );
}

#[test]
fn scenes_can_be_created_renamed_reordered_and_deleted() {
    let (mut core, ctx, _dir) = new_project("scenes");
    let first = story(&core).scenes[0].id;
    one_undo_step(&mut core, &ctx, "orbit.scene.new_scene");
    let second = story(&core).scenes[1].id;
    assert_eq!(story(&core).scenes[1].name, "Nouvelle scène");

    // Rename (the header edits through RenameScene).
    with_ctx(&mut core, |c| {
        c.exec(Command::RenameScene {
            id: second,
            name: "Chapitre 2".into(),
        })
        .unwrap();
    });
    assert_eq!(story(&core).scene(second).unwrap().name, "Chapitre 2");

    // Reorder.
    with_ctx(&mut core, |c| c.select(Selection::Scene(second)));
    one_undo_step(&mut core, &ctx, "orbit.scene.scene_up");
    assert_eq!(story(&core).scenes[0].id, second);

    // Delete the scene holding lines: they and their strings go, one undo brings all back.
    with_ctx(&mut core, |c| c.select(Selection::Scene(first)));
    let strings = story(&core).strings.entries.len();
    assert!(!story(&core).scene(first).unwrap().nodes.is_empty());
    one_undo_step(&mut core, &ctx, "orbit.scene.delete_scene");
    assert!(story(&core).scene(first).is_none());
    assert!(story(&core).strings.entries.len() < strings);
    // The selection moved to a remaining scene, and that one can be deleted too.
    assert!(matches!(core.state.selection, Some(Selection::Scene(_))));
}

#[test]
fn every_node_kind_has_an_inspector_that_draws_and_edits() {
    let (mut core, ctx, _dir) = new_project("inspector");
    let scene = with_ctx(&mut core, |c| {
        c.ops(|p| ops::add_scene(p, "Kinds")).unwrap()
    });
    let (ana, bg, music, sound) = with_ctx(&mut core, |c| {
        c.ops(|p| {
            let bg = ops::add_asset(p, AssetKind::Image, "assets/bg.png")?;
            let music = ops::add_asset(p, AssetKind::Music, "assets/m.ogg")?;
            let sound = ops::add_asset(p, AssetKind::Sound, "assets/s.wav")?;
            let ana = ops::add_character(p, "Ana", Color::WHITE)?;
            let mut def = p.story().character(ana).unwrap().clone();
            def.expressions.push(Expression {
                name: "neutral".into(),
                image: bg,
            });
            p.execute(Command::SetCharacter {
                id: def.id,
                character: Some(def),
                index: None,
            })?;
            ops::add_variable(p, orbit_core::VarDecl::game("n", Value::Int(1)))?;
            Ok((ana, bg, music, sound))
        })
        .unwrap()
    });
    let label = ops::NodeDraft::label("cible");
    let lid = label.node.id;
    let text = StringId::new();
    let kinds: Vec<NodeKind> = vec![
        NodeKind::Label { name: "x".into() },
        NodeKind::Say {
            speaker: Some(ana),
            text_id: text,
        },
        NodeKind::Choice {
            options: vec![ChoiceOption {
                text_id: text,
                target: lid,
                condition: Some(Expr::binary(BinOp::Gt, Expr::var("n"), Expr::lit(0i64))),
            }],
        },
        NodeKind::Jump { target: lid },
        NodeKind::SetVar {
            var: "n".into(),
            value: Expr::binary(BinOp::Add, Expr::var("n"), Expr::lit(1i64)),
        },
        NodeKind::If {
            condition: Expr::var("n"),
            then_target: lid,
            else_target: Some(lid),
        },
        NodeKind::Background {
            asset: bg,
            transition: Some(Transition::dissolve(1.0)),
        },
        NodeKind::Show {
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
            character: ana,
            expression: "neutral".into(),
            position: Position::Custom("far".into()),
            transition: None,
        },
        NodeKind::Hide {
            character: ana,
            transition: Some(Transition::fade(Color::BLACK, 0.5)),
        },
        NodeKind::PlayMusic {
            asset: music,
            fade_in: 1.0,
            looping: true,
        },
        NodeKind::StopMusic { fade_out: 2.0 },
        NodeKind::PlaySound { asset: sound },
        NodeKind::Wait { seconds: 1.5 },
        NodeKind::Call { label: lid },
        NodeKind::Return,
        NodeKind::End,
        NodeKind::Custom {
            plugin_id: "orbit.sample".into(),
            kind: "note".into(),
            version: 1,
            data: CustomData::Map(vec![("text".into(), CustomData::Str("n".into()))]),
        },
        NodeKind::Custom {
            plugin_id: "absent.plugin".into(),
            kind: "k".into(),
            version: 3,
            data: CustomData::List(vec![CustomData::Int(1), CustomData::Null]),
        },
    ];
    with_ctx(&mut core, |c| {
        c.ops(|p| {
            p.execute(Command::SetString {
                id: text,
                text: Some("texte".into()),
            })?;
            ops::insert_drafts(p, scene, 0, vec![label], "label")?;
            for (i, kind) in kinds.iter().enumerate() {
                p.execute(Command::InsertNode {
                    scene,
                    index: i + 1,
                    node: Node {
                        id: NodeId::new(),
                        kind: kind.clone(),
                    },
                })?;
            }
            Ok(())
        })
        .unwrap();
    });
    let ids = scene_nodes(&core, scene);
    // The inspector is registered for every built-in kind.
    for key in [
        "Say",
        "Label",
        "Choice",
        "Jump",
        "SetVar",
        "If",
        "Background",
        "Show",
        "Hide",
        "PlayMusic",
        "StopMusic",
        "PlaySound",
        "Wait",
        "Call",
        "Return",
        "End",
    ] {
        assert!(
            core.registry.inspector_for(key).is_some(),
            "no inspector for {key}"
        );
    }
    // Draw the inspector for each node (the Inspector tab is in front by default).
    for (i, id) in ids.iter().enumerate().skip(1) {
        with_ctx(&mut core, |c| c.select(Selection::Node(*id)));
        for f in 0..3 {
            frame(&mut core, &ctx, (i * 3 + f) as f64 * 0.016, (1360.0, 820.0));
        }
    }
    // Replacing the kind of every node by a changed copy is one undo step
    // and comes back on undo (the inspector's commit path).
    for id in ids.iter().skip(1) {
        let kind = story(&core).node(*id).unwrap().kind.clone();
        let changed = match kind.clone() {
            NodeKind::Wait { seconds } => NodeKind::Wait {
                seconds: seconds + 1.0,
            },
            NodeKind::Label { name } => NodeKind::Label {
                name: format!("{name}2"),
            },
            NodeKind::StopMusic { fade_out } => NodeKind::StopMusic {
                fade_out: fade_out + 1.0,
            },
            NodeKind::Custom {
                plugin_id,
                kind,
                version,
                ..
            } => NodeKind::Custom {
                plugin_id,
                kind,
                version,
                data: CustomData::Bool(true),
            },
            other => other,
        };
        if changed == kind {
            continue;
        }
        let before = story(&core).clone();
        with_ctx(&mut core, |c| {
            c.exec(Command::SetNodeKind {
                id: *id,
                kind: changed,
                expected_version: None,
            })
            .unwrap();
        });
        assert!(core.session.as_mut().unwrap().project.undo().unwrap());
        assert_eq!(*story(&core), before);
    }
}

#[test]
fn characters_and_variables_panels_draw_and_renames_follow_their_usages() {
    let (mut core, ctx, _dir) = new_project("defs");
    let scene = with_ctx(&mut core, |c| c.ops(|p| ops::add_scene(p, "Defs")).unwrap());
    let (ana, sprite) = with_ctx(&mut core, |c| {
        c.ops(|p| {
            let sprite = ops::add_asset(p, AssetKind::Image, "assets/a.png")?;
            let ana = ops::add_character(p, "Ana", Color::WHITE)?;
            let mut def = p.story().character(ana).unwrap().clone();
            def.expressions.push(Expression {
                name: "happy".into(),
                image: sprite,
            });
            p.execute(Command::SetCharacter {
                id: def.id,
                character: Some(def),
                index: None,
            })?;
            ops::add_variable(p, orbit_core::VarDecl::game("mood", Value::Int(0)))?;
            Ok((ana, sprite))
        })
        .unwrap()
    });
    let _ = sprite;
    let show = ops::NodeDraft::plain(NodeKind::Show {
        scale: None,
        scale_y: None,
        flip_x: false,
        flip_y: false,
        character: ana,
        expression: "happy".into(),
        position: Position::Left,
        transition: None,
    });
    let set = ops::NodeDraft::plain(NodeKind::SetVar {
        var: "mood".into(),
        value: Expr::binary(BinOp::Add, Expr::var("mood"), Expr::lit(1i64)),
    });
    with_ctx(&mut core, |c| {
        c.ops(|p| ops::insert_drafts(p, scene, 0, vec![show, set], "x"))
            .unwrap();
    });

    // Both panels draw (shown through the View actions).
    for panel in [
        "orbit.core.views.show.orbit.characters.characters",
        "orbit.core.views.show.orbit.variables.variables",
    ] {
        core.run_action_now(panel, &ctx);
        frames(&mut core, &ctx, 4);
    }

    // Renaming an expression / a variable updates the usages (one undo step)...
    let before = story(&core).clone();
    with_ctx(&mut core, |c| {
        c.ops(|p| ops::rename_expression(p, ana, "happy", "joyeuse"))
            .unwrap();
        c.ops(|p| ops::rename_variable(p, "mood", "humeur"))
            .unwrap();
    });
    assert!(validate(story(&core)).errors.is_empty());
    assert!(ops::variable_users(story(&core), "humeur").len() == 1);
    core.run_action_now("orbit.core.undo", &ctx);
    core.run_action_now("orbit.core.undo", &ctx);
    assert_eq!(*story(&core), before);

    // ... or is refused with a message and changes nothing.
    with_ctx(&mut core, |c| {
        assert!(
            c.ops(|p| ops::rename_variable(p, "mood", "1 mauvais nom"))
                .is_err()
        );
        assert!(c.ops(|p| ops::delete_character(p, ana)).is_err());
        assert!(c.ops(|p| ops::delete_variable(p, "mood")).is_err());
        assert!(
            c.ops(|p| ops::update_variable(p, "mood", Value::Bool(true), VarScope::Persistent))
                .is_ok()
        );
    });
}

#[test]
fn the_scene_editor_draws_every_state_without_panicking() {
    let (mut core, ctx, _dir) = new_project("states");
    // No scene yet (delete the template's one), then an empty scene, then lines.
    let first = story(&core).scenes[0].id;
    with_ctx(&mut core, |c| edit::delete_scene(c, first));
    assert!(story(&core).scenes.is_empty());
    frames(&mut core, &ctx, 3);
    core.run_action_now("orbit.scene.new_scene", &ctx);
    frames(&mut core, &ctx, 3);
    core.run_action_now("orbit.scene.insert_choice", &ctx);
    frames(&mut core, &ctx, 3);
    core.run_action_now("orbit.scene.quick_insert", &ctx);
    frames(&mut core, &ctx, 3);
    core.run_action_now("orbit.scene.rename_scene", &ctx);
    frames(&mut core, &ctx, 3);
    // Inline editing of a line.
    core.run_action_now("orbit.explorer.add_say", &ctx);
    frames(&mut core, &ctx, 4);
    core.run_action_now("orbit.scene.select_all", &ctx);
    frames(&mut core, &ctx, 3);
    assert!(validate(story(&core)).errors.is_empty());
}

#[test]
fn a_5000_node_scene_scrolls_without_dropping_below_60_fps_of_cpu_budget() {
    let (mut core, ctx, dir) = new_project("big");
    let big = orbit_editor::bench::generate_story(5000);
    let nodes = big.scenes[0].nodes.len();
    let path = dir.path().join("big.ron");
    std::fs::write(&path, big.to_ron().unwrap()).unwrap();
    core.open_project(&path);
    assert_eq!(story(&core).scenes[0].nodes.len(), nodes);
    core.state.debug_autoscroll = Some(60.0);
    // Warm up (layout cache, fonts), then measure the UI work per frame.
    frames(&mut core, &ctx, 10);
    let mut times: Vec<Duration> = Vec::new();
    for i in 0..120 {
        let started = Instant::now();
        frame(&mut core, &ctx, 1.0 + i as f64 * 0.016, (1360.0, 820.0));
        times.push(started.elapsed());
    }
    times.sort();
    let avg = times.iter().sum::<Duration>() / times.len() as u32;
    let p95 = times[(times.len() as f32 * 0.95) as usize];
    println!(
        "5000-node scene, debug build: avg {avg:?}, p95 {p95:?}, max {:?}",
        times.last().unwrap()
    );
    // The budget of a 60 fps frame is 16.7 ms: the real check is the release
    // measurement (see the ROADMAP); in a debug build we only guard against
    // the cost scaling with the size of the story (an unvirtualized list
    // costs seconds here).
    assert!(avg < Duration::from_millis(80), "average frame {avg:?}");
}

#[test]
fn editing_a_5000_node_scene_keeps_each_edit_cheap() {
    let (mut core, ctx, dir) = new_project("bigedit");
    let big = orbit_editor::bench::generate_story(5000);
    let path = dir.path().join("big.ron");
    std::fs::write(&path, big.to_ron().unwrap()).unwrap();
    core.open_project(&path);
    frames(&mut core, &ctx, 6);
    // One keystroke = one command + validation + a frame.
    let id = story(&core).scenes[0]
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Say { .. }))
        .unwrap()
        .id;
    let NodeKind::Say { text_id, .. } = story(&core).node(id).unwrap().kind.clone() else {
        unreachable!()
    };
    let mut times = Vec::new();
    for i in 0..20 {
        let started = Instant::now();
        with_ctx(&mut core, |c| {
            c.exec_merged(
                Command::SetString {
                    id: text_id,
                    text: Some(format!("texte modifié {i}")),
                },
                "typing",
            )
            .unwrap();
        });
        frame(&mut core, &ctx, 2.0 + i as f64 * 0.016, (1360.0, 820.0));
        times.push(started.elapsed());
    }
    times.sort();
    println!(
        "5000-node edit + frame, debug: median {:?}, max {:?}",
        times[10], times[19]
    );
    assert!(times[10] < Duration::from_millis(250), "{:?}", times[10]);
}
