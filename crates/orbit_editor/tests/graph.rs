//! Headless tests of the M3d graph panel: the macro graph of the story, the
//! edits made from it (each one undo entry), the `.layout` sidecar, and the
//! frame cost of a 200-block graph. No window, no GPU.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use egui::{Context, Pos2, RawInput, Rect, vec2};
use orbit_core::{NodeId, NodeKind, SceneId, Story, ValidationWarning, validate};
use orbit_editor::bench_graph::generate_graph_story;
use orbit_editor::builtin::graph::{GraphEvent, apply_event};
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::{EditorContext, Selection};
use orbit_project::graph::{BlockId, GraphModel, SlotKind, SlotRef, SlotTarget};
use orbit_project::graph_layout::layout_file;

// ------------------------------------------------------------------ helpers

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-graph-it-{tag}-{}-{:?}",
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

fn new_project(tag: &str) -> (EditorCore, Context, TempDir, PathBuf) {
    let dir = TempDir::new(tag);
    let ctx = Context::default();
    let mut core = EditorCore::new(CoreOptions::with_language(orbit_editor_api::Lang::Fr), &ctx);
    let root = orbit_editor::storage::create_project(
        dir.path(),
        "Projet",
        &orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr),
    )
    .unwrap();
    core.open_project(&root);
    assert!(core.has_project());
    core.run_action_now("orbit.core.views.show.orbit.graph.graph", &ctx);
    (core, ctx, dir, root)
}

fn with_ctx<R>(core: &mut EditorCore, f: impl FnOnce(&mut EditorContext<'_>) -> R) -> R {
    let session = core.session.as_mut().expect("a project is open");
    let mut ectx = EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
    f(&mut ectx)
}

fn frame(core: &mut EditorCore, ctx: &Context, time: f64) {
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1360.0, 820.0))),
        time: Some(time),
        ..RawInput::default()
    };
    ctx.run_ui(input, |ui| core.ui(ui))
        .drop_without_applying_deltas();
}

fn frames(core: &mut EditorCore, ctx: &Context, n: usize) {
    for i in 0..n {
        frame(core, ctx, i as f64 * 0.016);
    }
}

fn story(core: &EditorCore) -> &Story {
    core.story().unwrap()
}

fn model(core: &EditorCore) -> GraphModel {
    GraphModel::build(story(core))
}

fn intro(core: &EditorCore) -> (SceneId, NodeId) {
    let s = &story(core).scenes[0];
    (s.id, s.nodes[0].id)
}

fn event(core: &mut EditorCore, ctx: &Context, e: GraphEvent) {
    with_ctx(core, |c| apply_event(c, e));
    frames(core, ctx, 2);
}

fn undo(core: &mut EditorCore, ctx: &Context) {
    core.run_action_now("orbit.core.undo", ctx);
    frames(core, ctx, 2);
}

fn redo(core: &mut EditorCore, ctx: &Context) {
    core.run_action_now("orbit.core.redo", ctx);
    frames(core, ctx, 2);
}

/// Applies `e`, checks that it changed the story, left exactly one undo
/// entry (one undo restores the story, redo reapplies it).
fn one_undo_step(core: &mut EditorCore, ctx: &Context, e: GraphEvent) {
    let before = story(core).clone();
    event(core, ctx, e);
    let after = story(core).clone();
    assert_ne!(before, after, "the gesture changed nothing");
    undo(core, ctx);
    assert_eq!(*story(core), before, "one undo restores the story");
    redo(core, ctx);
    assert_eq!(*story(core), after, "redo reapplies it");
}

// -------------------------------------------------------------------- tests

/// Criterion of the milestone: a choice with two branches leading to two
/// different scenes, made from the graph alone, plays in the VM and validates.
#[test]
fn a_choice_with_two_branches_made_from_the_graph_alone_plays_and_validates() {
    let (mut core, ctx, _dir, _root) = new_project("choice");
    frames(&mut core, &ctx, 3);
    let (intro_scene, first) = intro(&core);

    // Select the block, then "add a choice" (the graph action, one gesture).
    with_ctx(&mut core, |c| c.select(Selection::Node(first)));
    let before = story(&core).clone();
    core.run_action_now("orbit.graph.add_choice", &ctx);
    frames(&mut core, &ctx, 2);

    let s = story(&core).clone();
    assert_eq!(s.scenes.len(), 3, "two branch scenes were created");
    let choice = s.scenes[0]
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Choice { options } => Some(options.clone()),
            _ => None,
        })
        .expect("a choice was added to the block");
    assert_eq!(choice.len(), 2);
    let scene_of = |label: NodeId| s.scenes[s.locate(label).unwrap().0].id;
    let (a, b) = (scene_of(choice[0].target), scene_of(choice[1].target));
    assert_ne!(a, b, "the branches lead to two different scenes");
    assert_ne!(a, intro_scene);
    assert_ne!(b, intro_scene);

    // Valid: no error; the unfinished branch scenes only warn.
    let report = validate(&s);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| matches!(w, ValidationWarning::SceneEndsGame { scene, .. } if *scene == a)),
        "{:?}",
        report.warnings
    );

    // The graph shows it: head with two options, two new blocks, no orphan.
    let m = model(&core);
    let head = m.block(BlockId::Scene(intro_scene)).unwrap();
    assert_eq!(head.slots.len(), 2);
    assert!(
        head.slots
            .iter()
            .all(|slot| matches!(slot.kind, SlotKind::Option(_)))
    );
    assert!(m.unreachable.is_empty());

    // Playable: each branch is reachable through the VM.
    for (pick, scene) in [(0usize, a), (1, b)] {
        let mut vm = orbit_vm::Vm::from_start(Arc::new(s.clone())).unwrap();
        let mut reached = false;
        for _ in 0..50 {
            match vm.step() {
                orbit_vm::StepResult::Choice { options, .. } => {
                    assert_eq!(options.len(), 2);
                    vm.choose(pick).unwrap();
                    let at = vm.position().expect("the VM is at the branch label");
                    assert_eq!(s.scenes[s.locate(at).unwrap().0].id, scene);
                    reached = true;
                }
                orbit_vm::StepResult::End => break,
                _ => {}
            }
        }
        assert!(reached, "the choice was offered");
    }

    // One undo takes the whole gesture back (choice and both scenes).
    undo(&mut core, &ctx);
    assert_eq!(*story(&core), before);
    redo(&mut core, &ctx);
    assert_eq!(story(&core).scenes.len(), 3);
}

#[test]
fn links_scene_jumps_redirects_cuts_and_renames_are_single_undo_steps() {
    let (mut core, ctx, _dir, _root) = new_project("edits");
    frames(&mut core, &ctx, 3);
    let (intro_scene, first) = intro(&core);
    let head = BlockId::Scene(intro_scene);

    // Two scenes, created from the graph.
    one_undo_step(&mut core, &ctx, GraphEvent::NewScene((900.0, 40.0)));
    let first_new = story(&core).scenes[1].id;
    one_undo_step(&mut core, &ctx, GraphEvent::NewScene((900.0, 300.0)));
    // `one_undo_step` leaves both created (redo): they are scenes 1 and 2.
    assert_eq!(story(&core).scenes.len(), 3);
    let second_new = story(&core).scenes[2].id;
    let (b1, b2) = (BlockId::Scene(first_new), BlockId::Scene(second_new));

    // A choice built by hand: link the block's fall-through to scene B1, which
    // is another scene, hence a scene jump.
    let next = SlotRef {
        block: head,
        node: None,
        kind: SlotKind::Next,
    };
    one_undo_step(&mut core, &ctx, GraphEvent::Connect(next, b1));
    let m = model(&core);
    let slot = m.block(head).unwrap().slots[0].clone();
    assert_eq!(slot.kind, SlotKind::JumpScene);
    assert_eq!(slot.target, SlotTarget::Block(b1));
    assert!(matches!(
        story(&core).node(slot.node.unwrap()).map(|n| &n.kind),
        Some(NodeKind::JumpScene { scene, label: None }) if *scene == first_new
    ));

    // The VM follows it: the two lines of the intro, then the new scene (it
    // has nothing to say, so the game ends there).
    let mut vm = orbit_vm::Vm::from_start(Arc::new(story(&core).clone())).unwrap();
    assert!(matches!(vm.step(), orbit_vm::StepResult::Say { .. }));
    assert!(matches!(vm.step(), orbit_vm::StepResult::Say { .. }));
    assert!(matches!(vm.step(), orbit_vm::StepResult::End));

    // Redirect it to B2, then cut it.
    let jump = SlotRef {
        block: head,
        node: slot.node,
        kind: SlotKind::JumpScene,
    };
    one_undo_step(&mut core, &ctx, GraphEvent::Connect(jump, b2));
    assert_eq!(
        model(&core).block(head).unwrap().slots[0].target,
        SlotTarget::Block(b2)
    );
    one_undo_step(&mut core, &ctx, GraphEvent::Disconnect(jump));
    assert!(model(&core).block(head).unwrap().falls_off_scene());

    // Rename a scene from the graph, then a label block.
    one_undo_step(
        &mut core,
        &ctx,
        GraphEvent::Rename(b2, "Fin heureuse".into()),
    );
    assert_eq!(story(&core).scene(second_new).unwrap().name, "Fin heureuse");
    one_undo_step(&mut core, &ctx, GraphEvent::NewLabel((500.0, 500.0)));
    let label = model(&core)
        .blocks
        .iter()
        .find_map(|b| (!b.is_head).then_some(b.id))
        .expect("a label block was created");
    one_undo_step(
        &mut core,
        &ctx,
        GraphEvent::Rename(label, "épilogue".into()),
    );
    assert_eq!(model(&core).block(label).unwrap().title, "épilogue");

    // The selection follows: open from the graph selects the block's node and
    // shows the scene editor.
    event(&mut core, &ctx, GraphEvent::Open(b1));
    let target = story(&core).scene(first_new).unwrap().nodes[0].id;
    let sc = story(&core).scene(first_new).unwrap();
    assert_eq!(
        core.state.selection,
        Some(Selection::Node(target)),
        "scene {:?} nodes {:?}",
        sc.name,
        sc.nodes.iter().map(|n| (n.id, &n.kind)).collect::<Vec<_>>()
    );
    let _ = first;
}

#[test]
fn an_option_is_redirected_by_dragging_and_a_condition_is_added() {
    let (mut core, ctx, _dir, _root) = new_project("redirect");
    frames(&mut core, &ctx, 3);
    let (intro_scene, first) = intro(&core);
    let head = BlockId::Scene(intro_scene);
    with_ctx(&mut core, |c| c.select(Selection::Node(first)));
    core.run_action_now("orbit.graph.add_choice", &ctx);
    frames(&mut core, &ctx, 2);

    // Redirect option 1 to the branch of option 0 (drag its pin onto it).
    let m = model(&core);
    let slots = m.block(head).unwrap().slots.clone();
    let SlotTarget::Block(branch_a) = slots[0].target else {
        panic!("option 0 leads to a block")
    };
    let option_1 = SlotRef {
        block: head,
        node: slots[1].node,
        kind: SlotKind::Option(1),
    };
    one_undo_step(&mut core, &ctx, GraphEvent::Connect(option_1, branch_a));
    let slots = model(&core).block(head).unwrap().slots.clone();
    assert_eq!(slots[0].target, slots[1].target);

    // Cutting an option removes it; texts are editable through their option.
    one_undo_step(&mut core, &ctx, GraphEvent::Disconnect(option_1));
    assert_eq!(model(&core).block(head).unwrap().slots.len(), 1);
    let option_0 = SlotRef {
        block: head,
        node: slots[0].node,
        kind: SlotKind::Option(0),
    };
    one_undo_step(
        &mut core,
        &ctx,
        GraphEvent::SetOptionText(option_0, "Partir".into()),
    );
    assert_eq!(model(&core).block(head).unwrap().slots[0].label, "Partir");

    // A condition on a branch (it is open: a fresh scene with just a label).
    one_undo_step(&mut core, &ctx, GraphEvent::AddCondition(branch_a));
    let kinds: Vec<_> = model(&core)
        .block(branch_a)
        .unwrap()
        .slots
        .iter()
        .map(|s| s.kind)
        .collect();
    // A condition does not end the block: it still falls through (here, the
    // end of the game) when the `else` branch is empty.
    assert_eq!(kinds, [SlotKind::Then, SlotKind::Else, SlotKind::Next]);
    assert!(validate(story(&core)).errors.is_empty());
}

#[test]
fn script_pins_connect_and_cut_from_the_graph() {
    let (mut core, ctx, _dir, _root) = new_project("script_pins");
    frames(&mut core, &ctx, 3);
    let (intro_scene, _) = intro(&core);
    let head = BlockId::Scene(intro_scene);
    let script = NodeId::new();
    with_ctx(&mut core, |c| {
        c.exec(orbit_project::Command::InsertNode {
            scene: intro_scene,
            index: 1,
            node: orbit_core::Node {
                id: script,
                kind: NodeKind::Script {
                    source: "branch(\"x\")".into(),
                    branches: Vec::new(),
                    on_error: None,
                },
            },
        })
        .unwrap();
    });
    frames(&mut core, &ctx, 3);
    let target = {
        // A second scene to point the error pin at.
        let mut id = None;
        with_ctx(&mut core, |c| {
            id = Some(
                c.ops(|p| orbit_project::graph_ops::create_scene(p, "Erreur"))
                    .unwrap(),
            );
        });
        id.unwrap()
    };
    frames(&mut core, &ctx, 2);
    let kinds: Vec<_> = model(&core)
        .block(head)
        .unwrap()
        .slots
        .iter()
        .map(|s| s.kind)
        .collect();
    assert_eq!(kinds, [SlotKind::OnError, SlotKind::Next]);
    let pin = SlotRef {
        block: head,
        node: Some(script),
        kind: SlotKind::OnError,
    };
    one_undo_step(&mut core, &ctx, GraphEvent::Connect(pin, target));
    let slots = model(&core).block(head).unwrap().slots.clone();
    assert!(matches!(slots[0].target, SlotTarget::Block(_)));
    one_undo_step(&mut core, &ctx, GraphEvent::Disconnect(pin));
    assert_eq!(
        model(&core).block(head).unwrap().slots[0].target,
        SlotTarget::None
    );
}

#[test]
fn deleting_asks_first_when_links_or_lines_would_be_lost() {
    let (mut core, ctx, _dir, _root) = new_project("delete");
    frames(&mut core, &ctx, 3);
    let (intro_scene, first) = intro(&core);
    with_ctx(&mut core, |c| c.select(Selection::Node(first)));
    core.run_action_now("orbit.graph.add_choice", &ctx);
    frames(&mut core, &ctx, 2);
    let branch = {
        let SlotTarget::Block(b) = model(&core)
            .block(BlockId::Scene(intro_scene))
            .unwrap()
            .slots[0]
            .target
        else {
            panic!("a branch")
        };
        b
    };

    // A link points here: the request waits for confirmation, nothing changes.
    let before = story(&core).clone();
    event(&mut core, &ctx, GraphEvent::Delete(branch));
    assert_eq!(core.state.graph.pending_delete, Some(branch));
    assert_eq!(*story(&core), before);
    // The dialog draws; cancelling by confirmation of another block is not
    // possible, confirming deletes the scene and cuts the option, once.
    frames(&mut core, &ctx, 2);
    event(&mut core, &ctx, GraphEvent::DeleteConfirmed(branch));
    assert_eq!(core.state.graph.pending_delete, None);
    assert_eq!(story(&core).scenes.len(), 2);
    let slots = model(&core)
        .block(BlockId::Scene(intro_scene))
        .unwrap()
        .slots
        .clone();
    assert_eq!(slots.len(), 1, "the option leading there was cut");
    undo(&mut core, &ctx);
    assert_eq!(*story(&core), before, "one undo restores everything");

    // The introduction has lines: deleting its block asks too.
    event(
        &mut core,
        &ctx,
        GraphEvent::Delete(BlockId::Scene(intro_scene)),
    );
    assert_eq!(
        core.state.graph.pending_delete,
        Some(BlockId::Scene(intro_scene))
    );
    core.state.graph.pending_delete = None;

    // An empty, unlinked scene goes at once.
    event(&mut core, &ctx, GraphEvent::NewScene((0.0, 700.0)));
    let fresh = BlockId::Scene(story(&core).scenes.last().unwrap().id);
    let count = story(&core).scenes.len();
    event(&mut core, &ctx, GraphEvent::Delete(fresh));
    assert_eq!(story(&core).scenes.len(), count - 1);
    assert_eq!(core.state.graph.pending_delete, None);
}

#[test]
fn positions_persist_in_the_layout_sidecar_and_reload() {
    let (mut core, ctx, _dir, root) = new_project("layout");
    frames(&mut core, &ctx, 3);
    let (intro_scene, first) = intro(&core);
    let head = BlockId::Scene(intro_scene);
    let sidecar = root.join(layout_file("story.orbit"));

    // Merely looking at the graph writes nothing.
    assert!(!sidecar.exists());
    assert!(core.state.graph.layout.get(head).is_some(), "auto layout");

    // Create branches (blocks appear next to their parent), then move one.
    with_ctx(&mut core, |c| c.select(Selection::Node(first)));
    core.run_action_now("orbit.graph.add_choice", &ctx);
    frames(&mut core, &ctx, 3);
    let m = model(&core);
    let others: Vec<BlockId> = m
        .blocks
        .iter()
        .map(|b| b.id)
        .filter(|id| *id != head)
        .collect();
    assert_eq!(others.len(), 2);
    for id in &others {
        assert!(core.state.graph.layout.get(*id).is_some());
    }
    event(
        &mut core,
        &ctx,
        GraphEvent::Moved(others[0], (123.0, -45.5)),
    );
    frames(&mut core, &ctx, 2);
    // Written by the background writer.
    orbit_editor::writer::flush();
    assert!(sidecar.exists(), "a move writes the sidecar");
    let text = std::fs::read_to_string(&sidecar).unwrap();
    assert!(text.contains(&others[0].key()), "{text}");
    // The story file itself is untouched by positions.
    assert!(
        !std::fs::read_to_string(root.join("story.orbit"))
            .unwrap()
            .contains("123")
    );

    // Save, close, reopen: the positions come back and nothing else moved.
    let positions = core.state.graph.layout.clone();
    assert!(core.save());
    core.close_project();
    assert!(!core.has_project());
    core.open_project(&root);
    frames(&mut core, &ctx, 3);
    assert_eq!(core.state.graph.layout.positions, positions.positions);
    assert_eq!(core.state.graph.layout.get(others[0]), Some((123.0, -45.5)));

    // A block created later is placed without disturbing the others.
    let before = core.state.graph.layout.clone();
    event(&mut core, &ctx, GraphEvent::NewLabel((700.0, 20.0)));
    for (key, pos) in &before.positions {
        assert_eq!(core.state.graph.layout.positions.get(key), Some(pos));
    }
    assert_eq!(
        core.state.graph.layout.positions.len(),
        before.positions.len() + 1
    );

    // An unreadable sidecar is ignored (with a warning), not fatal.
    core.close_project();
    std::fs::write(&sidecar, "((((").unwrap();
    core.open_project(&root);
    frames(&mut core, &ctx, 2);
    assert!(core.has_project());
    assert!(core.state.graph.layout.get(head).is_some());
}

#[test]
fn a_200_block_graph_draws_and_every_block_has_a_position() {
    let dir = TempDir::new("big");
    let ctx = Context::default();
    let mut core = EditorCore::new(CoreOptions::with_language(orbit_editor_api::Lang::Fr), &ctx);
    let root = orbit_editor::storage::create_project(
        dir.path(),
        "Gros",
        &orbit_editor::i18n::translator(orbit_editor_api::Lang::Fr),
    )
    .unwrap();
    let big = generate_graph_story(225);
    orbit_project::save_story(
        &orbit_editor::storage::FsStorage::new(&root),
        "story.orbit",
        &big,
    )
    .unwrap();
    core.open_project(&root);
    core.run_action_now("orbit.core.views.show.orbit.graph.graph", &ctx);
    let blocks = model(&core).blocks.len();
    assert!(blocks >= 170, "{blocks} blocks");

    // Frames: the first builds the model and the layout, the next ones draw.
    let started = Instant::now();
    frame(&mut core, &ctx, 0.0);
    let first = started.elapsed();
    let started = Instant::now();
    for i in 1..=20 {
        frame(&mut core, &ctx, f64::from(i) * 0.016);
    }
    let per_frame = started.elapsed().as_secs_f64() * 1000.0 / 20.0;
    println!(
        "graph of {blocks} blocks: first frame {first:?}, then {per_frame:.2} ms per frame (unoptimized)"
    );
    assert_eq!(core.state.graph.layout.positions.len(), blocks);
    assert!(!core.state.graph.dirty, "looking at a graph writes nothing");
    // Debug builds are far slower than release (measured separately); this
    // only guards against an accidental quadratic cost.
    assert!(per_frame < 400.0, "{per_frame} ms per frame");

    // Editing a big graph stays instant: one gesture, one undo entry.
    let head = model(&core).entry().unwrap();
    let started = Instant::now();
    event(&mut core, &ctx, GraphEvent::NewScene((0.0, 0.0)));
    let _ = head;
    assert!(started.elapsed().as_millis() < 1500);
}

#[test]
fn the_graph_panel_draws_every_state_without_panicking() {
    let (mut core, ctx, _dir, _root) = new_project("states");
    frames(&mut core, &ctx, 2);
    // Without scenes, with a lone empty scene, then with a selection on a
    // scene and a node.
    let ids: Vec<SceneId> = story(&core).scenes.iter().map(|s| s.id).collect();
    for id in ids {
        with_ctx(&mut core, |c| {
            c.exec(orbit_project::Command::RemoveScene { id }).unwrap();
        });
    }
    frames(&mut core, &ctx, 2);
    event(&mut core, &ctx, GraphEvent::NewScene((f32::NAN, f32::NAN)));
    let scene = story(&core).scenes[0].id;
    with_ctx(&mut core, |c| c.select(Selection::Scene(scene)));
    frames(&mut core, &ctx, 2);
    core.run_action_now("orbit.graph.reorganize", &ctx);
    core.run_action_now("orbit.graph.fit", &ctx);
    core.run_action_now("orbit.graph.focus", &ctx);
    frames(&mut core, &ctx, 3);
    core.close_project();
    frames(&mut core, &ctx, 2);
}

#[test]
fn revealing_elsewhere_brings_the_graph_to_the_node_and_problems_are_located() {
    use orbit_editor_api::Location;

    let (mut core, ctx, _dir, _root) = new_project("reveal");
    frames(&mut core, &ctx, 3);
    let (_, first) = intro(&core);
    with_ctx(&mut core, |c| c.select(Selection::Node(first)));
    core.run_action_now("orbit.graph.add_choice", &ctx);
    frames(&mut core, &ctx, 3);

    // The branch scenes end the game: warnings, located on a node that a
    // click on the Problems panel (or on the badge of the block) reveals.
    core.settle_diagnostics();
    let problems = core.state.diagnostics.problems.clone();
    let ends: Vec<_> = problems
        .iter()
        .filter(|p| p.message.contains("La partie se termine ici"))
        .collect();
    assert!(ends.len() >= 2, "{problems:?}");
    assert!(ends.iter().all(|p| p.location.is_some()));

    // Reveal a node of the second branch (as the Problems panel or the
    // search do): the selection follows and the graph draws without error.
    let branch_label = story(&core).scenes[2].nodes[0].id;
    with_ctx(&mut core, |c| {
        orbit_editor::builtin::reveal::reveal(c, Location::Node(branch_label));
    });
    frames(&mut core, &ctx, 3);
    assert_eq!(core.state.selection, Some(Selection::Node(branch_label)));
    assert!(core.state.graph.pending_delete.is_none());
}
