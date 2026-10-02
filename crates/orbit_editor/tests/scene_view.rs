//! Headless tests of the scene view ("Éditeur de vue"): the stage at the
//! selected node, dragging a character to a free position (one undo entry
//! per gesture), scale handles, mirroring, snapping, picking another
//! character and the keyboard. No window, no GPU: the test plays the part of
//! the preview host (it publishes the stage at the selected node, with the
//! same `orbit_runtime::scene_view` functions the host uses) and feeds the
//! panel pointer events through egui.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Context, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
use orbit_core::{AssetKind, CharacterId, Color, Expression, NodeId, NodeKind, SceneId, Story};
use orbit_editor::core::{CoreOptions, EditorCore};
use orbit_editor_api::{EditorContext, EditorRequest, SceneViewActor, Selection};
use orbit_project::{Command, ops};
use orbit_runtime::scene_view;
use orbit_vm::SeekLimits;

const PANEL: &str = "orbit.sceneview.sceneview";
const WINDOW: (f32, f32) = (1360.0, 820.0);
/// Size of the character pictures the stand-in host reports (virtual px).
const BASE: [f32; 2] = [600.0, 920.0];

// ------------------------------------------------------------------ helpers

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "orbit-sceneview-it-{tag}-{}-{:?}",
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

fn with_ctx<R>(core: &mut EditorCore, f: impl FnOnce(&mut EditorContext<'_>) -> R) -> R {
    let session = core.session.as_mut().expect("a project is open");
    let mut ectx = EditorContext::new(Some(&mut session.project), &mut core.state, &core.registry);
    f(&mut ectx)
}

struct Fixture {
    core: EditorCore,
    ctx: Context,
    _dir: TempDir,
    time: f64,
    scene: SceneId,
    /// The characters and the `Show` node of each, in story order.
    ana: CharacterId,
    bob: CharacterId,
    show_ana: NodeId,
    show_bob: NodeId,
}

impl Fixture {
    /// A project with two characters, shown left and right, a line after
    /// each; the first `Show` is selected and the panel is open.
    fn new(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        let ctx = Context::default();
        let mut core = EditorCore::new(CoreOptions::default(), &ctx);
        let root = orbit_editor::storage::create_project(dir.path(), "Projet").unwrap();
        core.open_project(&root);
        let (scene, ana, bob, show_ana, show_bob) = with_ctx(&mut core, |c| {
            // The scene of the new project: the only one the story starts in.
            let scene = c.story().unwrap().scenes[0].id;
            let img = c
                .ops(|p| ops::add_asset(p, AssetKind::Image, "a/char.png"))
                .unwrap();
            let mut ids = Vec::new();
            for name in ["Ana", "Bob"] {
                let id = c
                    .ops(|p| ops::add_character(p, name, Color::WHITE))
                    .unwrap();
                let mut def = c.story().unwrap().character(id).unwrap().clone();
                def.expressions.push(Expression {
                    name: "neutral".into(),
                    image: img,
                });
                c.exec(Command::SetCharacter {
                    id,
                    character: Some(def),
                    index: None,
                })
                .unwrap();
                ids.push(id);
            }
            let mut shows = Vec::new();
            for (i, (who, pos)) in [
                (ids[0], orbit_core::Position::Left),
                (ids[1], orbit_core::Position::Right),
            ]
            .into_iter()
            .enumerate()
            {
                let show = ops::NodeDraft::plain(NodeKind::Show {
                    character: who,
                    expression: "neutral".into(),
                    position: pos,
                    transition: None,
                    scale: None,
                    scale_y: None,
                    flip_x: false,
                    flip_y: false,
                });
                shows.push(show.node.id);
                let line = ops::NodeDraft::say(Some(who), "Salut");
                c.ops(|p| ops::insert_drafts(p, scene, i * 2, vec![show, line], "x"))
                    .unwrap();
            }
            (scene, ids[0], ids[1], shows[0], shows[1])
        });
        let mut f = Self {
            core,
            ctx,
            _dir: dir,
            time: 0.0,
            scene,
            ana,
            bob,
            show_ana,
            show_bob,
        };
        f.select(show_ana);
        f.core
            .state
            .requests
            .push(EditorRequest::ShowPanel(PANEL.to_owned()));
        f.frames(3);
        f
    }

    fn story(&self) -> &Story {
        self.core.story().unwrap()
    }

    fn select(&mut self, node: NodeId) {
        with_ctx(&mut self.core, |c| c.select(Selection::Node(node)));
    }

    /// What the preview host does between the UI pass and the paint: finds
    /// the stage at the wanted node and publishes it. Here `BASE` stands for
    /// the size of the picture.
    fn host(&mut self) {
        let sv = &mut self.core.state.scene_view;
        let wanted = sv.wanted_node;
        let Some(session) = self.core.session.as_ref() else {
            return;
        };
        let story = Arc::new(session.project.story().clone());
        let Some(node) = wanted.filter(|n| story.locate(*n).is_some()) else {
            self.core.state.scene_view.node = None;
            self.core.state.scene_view.actors.clear();
            return;
        };
        let (source, _) = scene_view::seek_before(&story, node, SeekLimits::default()).unwrap();
        let at = scene_view::scene_at(&story, &source, node).unwrap();
        let actors = scene_view::actors(&story, &at.after)
            .into_iter()
            .map(|a| SceneViewActor {
                character: a.character,
                source: a.source,
                x: a.x,
                y: a.y,
                scale: a.scale,
                flip_x: a.flip_x,
                flip_y: a.flip_y,
                base_size: BASE,
            })
            .collect();
        let sv = &mut self.core.state.scene_view;
        sv.node = Some(node);
        sv.actors = actors;
    }

    fn frame(&mut self, mut events: Vec<Event>, modifiers: Modifiers) {
        self.time += 0.016;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(WINDOW.0, WINDOW.1))),
            time: Some(self.time),
            events,
            ..RawInput::default()
        };
        let core = &mut self.core;
        self.ctx
            .run_ui(input, |ui| core.ui(ui))
            .drop_without_applying_deltas();
        // The host runs between the UI pass and the paint; its picture is
        // seen by the next UI pass.
        self.host();
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(Vec::new(), Modifiers::NONE);
        }
    }

    fn picture(&self) -> Rect {
        self.core
            .state
            .scene_view
            .picture_rect
            .expect("the scene view panel is drawn")
    }

    /// Screen position of a virtual-canvas point.
    fn screen(&self, v: [f32; 2]) -> Pos2 {
        let p = self.picture();
        Pos2::new(
            p.left() + v[0] / 1920.0 * p.width(),
            p.top() + v[1] / 1080.0 * p.height(),
        )
    }

    fn press(&mut self, pos: Pos2, modifiers: Modifiers) {
        self.frame(
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
            modifiers,
        );
    }

    fn move_to(&mut self, pos: Pos2, modifiers: Modifiers) {
        self.frame(vec![Event::PointerMoved(pos)], modifiers);
    }

    fn release(&mut self, pos: Pos2, modifiers: Modifiers) {
        self.frame(
            vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
        self.frames(2);
    }

    /// A whole drag from `from` to `to` in several frames.
    fn drag(&mut self, from: Pos2, to: Pos2, modifiers: Modifiers) {
        self.press(from, modifiers);
        for i in 1..=8 {
            let t = i as f32 / 8.0;
            self.move_to(from + (to - from) * t, modifiers);
        }
        self.release(to, modifiers);
    }

    fn key(&mut self, key: egui::Key, modifiers: Modifiers) {
        self.frame(
            vec![Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }],
            modifiers,
        );
        self.frame(
            vec![Event::Key {
                key,
                physical_key: Some(key),
                pressed: false,
                repeat: false,
                modifiers,
            }],
            modifiers,
        );
    }

    fn actor(&self, node: NodeId) -> SceneViewActor {
        self.core
            .state
            .scene_view
            .actor_of(node)
            .cloned()
            .expect("the actor of that Show is on the published stage")
    }

    fn look(&self, node: NodeId) -> ops::ShowLook {
        ops::show_look(self.story(), node).unwrap()
    }

    /// Number of undos needed to get back to `before`; leaves the story as
    /// it was after the calls (redone).
    fn undo_steps_to(&mut self, before: &Story) -> usize {
        let mut steps = 0;
        while self.story() != before && steps < 10 {
            assert!(with_ctx(&mut self.core, |c| c.undo()).unwrap());
            steps += 1;
        }
        assert_eq!(self.story(), before, "undoing restores the story");
        for _ in 0..steps {
            assert!(with_ctx(&mut self.core, |c| c.redo()).unwrap());
        }
        self.frames(2);
        steps
    }
}

// -------------------------------------------------------------------- tests

#[test]
fn the_view_shows_the_stage_at_the_selected_node() {
    let mut f = Fixture::new("stage");
    let sv = &f.core.state.scene_view;
    assert!(sv.visible || sv.wanted_node.is_some());
    assert_eq!(sv.wanted_node, Some(f.show_ana));
    // At the first Show, only Ana is on stage, at the left.
    assert_eq!(sv.actors.len(), 1);
    assert_eq!(sv.actors[0].character, f.ana);
    assert_eq!(sv.actors[0].source, Some(f.show_ana));
    assert_eq!((sv.actors[0].x, sv.actors[0].y), (0.25, 1.0));

    // At the second Show, both are there; each knows its own Show.
    f.select(f.show_bob);
    f.frames(3);
    let sv = &f.core.state.scene_view;
    assert_eq!(sv.actors.len(), 2);
    assert_eq!(sv.actor_of(f.show_ana).unwrap().character, f.ana);
    assert_eq!(sv.actor_of(f.show_bob).unwrap().character, f.bob);
    assert_eq!(sv.actor_of(f.show_bob).unwrap().x, 0.75);
    // The panel reports where it drew the 16:9 picture.
    let p = f.picture();
    assert!((p.width() / p.height() - 16.0 / 9.0).abs() < 0.01);
}

#[test]
fn dragging_a_character_makes_a_free_position_with_one_undo_entry() {
    let mut f = Fixture::new("drag");
    let before = f.story().clone();
    // Grab Ana in the middle of her box and pull her to a new spot.
    let actor = f.actor(f.show_ana);
    let grab = f.screen([actor.x * 1920.0, actor.y * 1080.0 - BASE[1] * 0.5]);
    let to = grab + vec2(120.0, -30.0);
    f.drag(grab, to, Modifiers::NONE);

    let look = f.look(f.show_ana);
    let orbit_core::Position::Free { x, y } = look.position else {
        panic!("the Show is now a free position, got {:?}", look.position);
    };
    let p = f.picture();
    let expect_x = 0.25 + 120.0 / p.width();
    let expect_y = 1.0 - 30.0 / p.height();
    assert!((x - expect_x).abs() < 0.02, "x {x} vs {expect_x}");
    assert!((y - expect_y).abs() < 0.02, "y {y} vs {expect_y}");
    // The scale is untouched.
    assert_eq!(look.scale, None);

    // The whole gesture (many frames of live updates) is ONE undo entry.
    assert_eq!(f.undo_steps_to(&before), 1);
    // The published stage followed the edit.
    let moved = f.actor(f.show_ana);
    assert!((moved.x - x).abs() < 1e-3);
    // Still valid.
    assert!(orbit_core::validate(f.story()).errors.is_empty());
}

#[test]
fn a_press_without_a_drag_changes_nothing() {
    let mut f = Fixture::new("noop");
    let before = f.story().clone();
    let actor = f.actor(f.show_ana);
    let p = f.screen([actor.x * 1920.0, actor.y * 1080.0 - 400.0]);
    f.press(p, Modifiers::NONE);
    f.release(p, Modifiers::NONE);
    assert_eq!(*f.story(), before);
}

#[test]
fn dragging_near_a_named_position_snaps_to_it() {
    let mut f = Fixture::new("snap");
    let actor = f.actor(f.show_ana);
    let grab = f.screen([actor.x * 1920.0, actor.y * 1080.0 - 400.0]);
    let p = f.picture();
    // Ana at the left (0.25): release 3 points away from the center line.
    let center_x = f.screen([960.0, 0.0]).x;
    let to = Pos2::new(center_x - 3.0, grab.y);
    f.drag(grab, to, Modifiers::NONE);
    assert_eq!(f.look(f.show_ana).position, orbit_core::Position::Center);
    let _ = p;

    // Alt turns snapping off for the gesture: the position stays free.
    let actor = f.actor(f.show_ana);
    let grab = f.screen([actor.x * 1920.0, actor.y * 1080.0 - 400.0]);
    let to = Pos2::new(grab.x + f.picture().width() * 0.125 - 3.0, grab.y);
    f.drag(grab, to, Modifiers::ALT);
    match f.look(f.show_ana).position {
        orbit_core::Position::Free { x, .. } => assert!((x - 0.625).abs() < 0.02, "{x}"),
        other => panic!("expected a free position, got {other:?}"),
    }
}

#[test]
fn a_handle_scales_the_character_around_its_feet_in_one_entry() {
    let mut f = Fixture::new("scale");
    let before = f.story().clone();
    let a = f.actor(f.show_ana);
    let anchor = [a.x * 1920.0, a.y * 1080.0];
    // The top-right corner of the unscaled box.
    let corner = f.screen([anchor[0] + BASE[0] * 0.5, anchor[1] - BASE[1]]);
    // Pull it out to double the half-width / height (relative to the anchor).
    let target = f.screen([anchor[0] + BASE[0], anchor[1] - BASE[1] * 2.0]);
    f.drag(corner, target, Modifiers::NONE);
    let look = f.look(f.show_ana);
    let s = look.scale.expect("the Show now overrides the scale");
    assert!((s - 2.0).abs() < 0.15, "scale {s}");
    assert_eq!(
        look.scale_y, None,
        "a corner of a uniform sprite stays uniform"
    );
    // The position is unchanged and still the named one.
    assert_eq!(look.position, orbit_core::Position::Left);
    assert_eq!(f.undo_steps_to(&before), 1);

    // A side handle stretches one axis: the sprite is no longer uniform.
    let a = f.actor(f.show_ana);
    let anchor = [a.x * 1920.0, a.y * 1080.0];
    let right = f.screen([
        anchor[0] + BASE[0] * 0.5 * a.scale[0],
        anchor[1] - BASE[1] * 0.5 * a.scale[1],
    ]);
    // Narrower than now (x1.5 of the half width), same height.
    let target = f.screen([
        anchor[0] + BASE[0] * 0.5 * 1.5,
        anchor[1] - BASE[1] * 0.5 * a.scale[1],
    ]);
    f.drag(right, target, Modifiers::NONE);
    let look = f.look(f.show_ana);
    assert!(look.scale_y.is_some(), "{look:?}");
    let eff = look.effective_scale(1.0);
    assert!(
        (eff[0] - 1.5).abs() < 0.1 && (eff[1] - 2.0).abs() < 0.1,
        "{eff:?}"
    );
}

#[test]
fn escape_cancels_a_drag_and_leaves_no_undo_entry() {
    let mut f = Fixture::new("escape");
    let before = f.story().clone();
    let undo_before = with_ctx(&mut f.core, |c| c.undo_label().map(str::to_owned));
    let actor = f.actor(f.show_ana);
    let grab = f.screen([actor.x * 1920.0, actor.y * 1080.0 - 400.0]);
    f.press(grab, Modifiers::NONE);
    for i in 1..=6 {
        f.move_to(grab + vec2(i as f32 * 12.0, 0.0), Modifiers::NONE);
    }
    assert_ne!(*f.story(), before, "the drag is live");
    f.frame(
        vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    f.release(grab, Modifiers::NONE);
    assert_eq!(*f.story(), before);
    assert_eq!(
        with_ctx(&mut f.core, |c| c.undo_label().map(str::to_owned)),
        undo_before,
        "the cancelled gesture left nothing on the undo stack"
    );
}

#[test]
fn mirror_actions_toggle_the_axes_one_undo_entry_each() {
    let mut f = Fixture::new("flip");
    let before = f.story().clone();
    f.core.run_action_now("orbit.sceneview.flip_x", &f.ctx);
    assert!(f.look(f.show_ana).flip_x && !f.look(f.show_ana).flip_y);
    assert_eq!(f.undo_steps_to(&before), 1);

    f.core.run_action_now("orbit.sceneview.flip_y", &f.ctx);
    assert!(f.look(f.show_ana).flip_y);
    f.frames(2);
    assert!(f.actor(f.show_ana).flip_y, "the stage follows");
    // Toggling again turns it off.
    f.core.run_action_now("orbit.sceneview.flip_y", &f.ctx);
    assert!(!f.look(f.show_ana).flip_y);
    assert!(orbit_core::validate(f.story()).errors.is_empty());
}

#[test]
fn clicking_another_character_selects_the_show_that_placed_it() {
    let mut f = Fixture::new("pick");
    f.select(f.show_bob);
    f.frames(3);
    assert_eq!(f.core.state.scene_view.actors.len(), 2);
    // Bob is selected; click on Ana's box.
    let a = f.actor(f.show_ana);
    let p = f.screen([a.x * 1920.0, a.y * 1080.0 - 400.0]);
    f.press(p, Modifiers::NONE);
    f.release(p, Modifiers::NONE);
    assert_eq!(f.core.state.selection, Some(Selection::Node(f.show_ana)));
    f.frames(3);
    assert_eq!(f.core.state.scene_view.wanted_node, Some(f.show_ana));
}

#[test]
fn arrows_nudge_finely_and_shift_takes_a_big_step() {
    let mut f = Fixture::new("keys");
    let a = f.actor(f.show_ana);
    // Focus the picture with a click on the character first.
    let p = f.screen([a.x * 1920.0, a.y * 1080.0 - 400.0]);
    f.press(p, Modifiers::NONE);
    f.release(p, Modifiers::NONE);
    let before = f.story().clone();
    f.key(egui::Key::ArrowRight, Modifiers::NONE);
    let orbit_core::Position::Free { x, y } = f.look(f.show_ana).position else {
        panic!("a nudge makes the position free");
    };
    assert!(
        (x - 0.252).abs() < 1e-4 && (y - 1.0).abs() < 1e-4,
        "{x},{y}"
    );
    assert_eq!(f.undo_steps_to(&before), 1);
    f.key(egui::Key::ArrowUp, Modifiers::SHIFT);
    let orbit_core::Position::Free { y, .. } = f.look(f.show_ana).position else {
        panic!("free");
    };
    assert!((y - 0.98).abs() < 1e-4, "{y}");
}

#[test]
fn the_view_reads_a_scene_selection_and_survives_having_nothing_selected() {
    let mut f = Fixture::new("sel");
    with_ctx(&mut f.core, |c| c.select(Selection::Scene(f.scene)));
    f.frames(3);
    // A scene shows the stage at its first node (here Ana's `Show`).
    assert_eq!(f.core.state.scene_view.wanted_node, Some(f.show_ana));
    assert_eq!(f.core.state.scene_view.actors.len(), 1);
    with_ctx(&mut f.core, |c| c.clear_selection());
    f.frames(3);
    assert_eq!(f.core.state.scene_view.wanted_node, None);
    assert!(f.core.state.scene_view.actors.is_empty());
}

#[test]
fn only_a_selected_show_can_be_edited() {
    let mut f = Fixture::new("noshow");
    // Select the line after the first Show.
    let line = f.story().scene(f.scene).unwrap().nodes[1].id;
    f.select(line);
    f.frames(3);
    let before = f.story().clone();
    let a = f.actor(f.show_ana);
    let from = f.screen([a.x * 1920.0, a.y * 1080.0 - 400.0]);
    f.drag(from, from + vec2(80.0, 0.0), Modifiers::NONE);
    assert_eq!(*f.story(), before, "a Say node has nothing to drag");
    // The mirror actions are disabled without a selected Show.
    f.core.run_action_now("orbit.sceneview.flip_x", &f.ctx);
    assert_eq!(*f.story(), before);
}
