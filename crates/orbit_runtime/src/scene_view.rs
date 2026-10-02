//! The stage at a node of the story, for the editor's scene view (no GPU).
//!
//! The scene view shows what the audience sees when the story reaches the
//! selected node. That is the VM state a player would have there, found by
//! the same headless search as "play from here" ([`crate::preview`]), and then
//! the node itself run once:
//!
//! 1. [`seek_before`] finds the state right before the node (the expensive
//!    part: a breadth-first search over the choices);
//! 2. [`scene_at`] restores it on the **current** story, reads the stage
//!    `before` the node and runs the node to read it `after`. It is cheap, so
//!    the editor can call it every frame while a character is dragged
//!    (the story changes at each frame, the path to the node does not).
//!
//! [`actors`] turns a scene into the placement data the editor draws gizmos
//! on, and [`stage_state`] into the [`StageState`] the real renderer draws, so
//! the picture and the gizmos come from the same numbers.

use std::sync::Arc;

use orbit_core::{AssetId, CharacterId, NodeId, Position, Story};
use orbit_vm::{SceneState, SeekLimits, Vm, scene_events};

use crate::preview::{ResumeSource, StartKind, plan_from_node};
use crate::stage::StageState;

/// A character on the stage at the selected node.
#[derive(Clone, Debug, PartialEq)]
pub struct StageActor {
    pub character: CharacterId,
    pub expression: String,
    /// Image of the expression.
    pub image: AssetId,
    pub position: Position,
    /// Normalized position of the bottom-center anchor.
    pub x: f32,
    pub y: f32,
    /// Effective scale `[x, y]`.
    pub scale: [f32; 2],
    pub flip_x: bool,
    pub flip_y: bool,
    /// The `Show` node that placed it.
    pub source: Option<NodeId>,
}

/// The stage around one node.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeScene {
    pub node: NodeId,
    /// Right before the node runs.
    pub before: SceneState,
    /// Right after it ran: what the audience sees once it is done.
    pub after: SceneState,
}

/// Finds the VM state right before `node` runs. The notice explains a
/// fallback (no path leads to the node: the state is the default one).
pub fn seek_before(
    story: &Arc<Story>,
    node: NodeId,
    limits: SeekLimits,
) -> Result<(ResumeSource, Option<String>), String> {
    let planned = plan_from_node(story, node, limits)?;
    let notice = match planned.kind {
        StartKind::Fallback => planned
            .notice
            .or_else(|| Some("Aucun chemin ne mène à ce nœud.".into())),
        _ => None,
    };
    Ok((ResumeSource::of(&planned.vm), notice))
}

/// The stage before and after `node`, with `story` (possibly edited since
/// `source` was found: only the nodes' contents must have changed, not the
/// path leading to the node).
pub fn scene_at(
    story: &Arc<Story>,
    source: &ResumeSource,
    node: NodeId,
) -> Result<NodeScene, String> {
    let entry = story.entry().ok_or("l'histoire est vide")?;
    let mut vm = Vm::new(story.clone(), entry).map_err(|e| e.to_string())?;
    vm.set_persistent_vars(&source.persistent);
    vm.set_read_nodes(source.read.clone());
    vm.restore(source.state.clone())
        .map_err(|e| e.to_string())?;
    let before = vm.scene().clone();
    // Run the node itself, and only it: stop before its successor.
    let next = story
        .locate(node)
        .and_then(|(s, n)| story.scenes[s].nodes.get(n + 1))
        .map(|n| n.id);
    match next {
        Some(next) => {
            let _ = vm.step_to(next);
        }
        None => {
            let _ = vm.step();
        }
    }
    let after = vm.scene().clone();
    Ok(NodeScene {
        node,
        before,
        after,
    })
}

/// The characters of `scene`, back to front, with their placement.
pub fn actors(story: &Story, scene: &SceneState) -> Vec<StageActor> {
    scene
        .characters
        .iter()
        .filter_map(|c| {
            let image = story
                .character(c.character)?
                .expression(&c.expression)?
                .image;
            let (x, y) = story.resolve_position(&c.position)?;
            Some(StageActor {
                character: c.character,
                expression: c.expression.clone(),
                image,
                position: c.position.clone(),
                x,
                y,
                scale: c.scale,
                flip_x: c.flip_x,
                flip_y: c.flip_y,
                source: c.source,
            })
        })
        .collect()
}

/// The renderer's stage for `scene` (background and characters).
pub fn stage_state(story: &Story, scene: &SceneState) -> StageState {
    let mut stage = StageState::default();
    for event in scene_events(story, scene) {
        stage.apply(&event);
    }
    stage
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::NodeKind;

    const DEMO: &str = include_str!("../../../examples/stories/test_minimal.ron");

    fn demo() -> Arc<Story> {
        Arc::new(Story::from_ron(DEMO).expect("demo loads"))
    }

    /// The first `Show` node of the story and the one after it.
    fn first_shows(story: &Story) -> Vec<NodeId> {
        story
            .scenes
            .iter()
            .flat_map(|s| &s.nodes)
            .filter(|n| matches!(n.kind, NodeKind::Show { .. }))
            .map(|n| n.id)
            .collect()
    }

    #[test]
    fn the_stage_at_a_show_has_the_character_after_but_not_before() {
        let story = demo();
        let show = first_shows(&story)[0];
        let (source, notice) = seek_before(&story, show, SeekLimits::default()).unwrap();
        assert_eq!(notice, None);
        let at = scene_at(&story, &source, show).unwrap();
        assert!(at.before.characters.is_empty());
        let after = actors(&story, &at.after);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].source, Some(show));
        assert_eq!(after[0].position, Position::Left);
        assert!((after[0].x, after[0].y) == (0.25, 1.0));
        // The renderer's stage agrees with the actors.
        let stage = stage_state(&story, &at.after);
        assert_eq!(stage.characters.len(), 1);
        assert_eq!(stage.characters[0].x, after[0].x);
        assert_eq!(stage.background, at.after.background);
    }

    #[test]
    fn an_edit_of_the_node_shows_without_a_new_search() {
        let story = demo();
        let show = first_shows(&story)[0];
        let (source, _) = seek_before(&story, show, SeekLimits::default()).unwrap();

        // The Show is dragged: new position, scale and mirror. The path to
        // the node is unchanged, so the cached source is reused.
        let mut edited = Story::clone(&story);
        let node = edited
            .scenes
            .iter_mut()
            .flat_map(|s| &mut s.nodes)
            .find(|n| n.id == show)
            .unwrap();
        let NodeKind::Show {
            position,
            scale,
            scale_y,
            flip_x,
            ..
        } = &mut node.kind
        else {
            panic!("a Show")
        };
        *position = Position::Free { x: 0.4, y: 0.9 };
        *scale = Some(0.5);
        *scale_y = Some(0.25);
        *flip_x = true;
        let edited = Arc::new(edited);

        let at = scene_at(&edited, &source, show).unwrap();
        let after = actors(&edited, &at.after);
        assert_eq!(after.len(), 1);
        assert_eq!((after[0].x, after[0].y), (0.4, 0.9));
        assert_eq!(after[0].scale, [0.5, 0.25]);
        assert!(after[0].flip_x && !after[0].flip_y);
        assert_eq!(after[0].source, Some(show));
    }

    #[test]
    fn a_later_node_sees_the_characters_placed_before_it() {
        let story = demo();
        let shows = first_shows(&story);
        // The second character's Show: the first one is already there.
        let second = *shows
            .iter()
            .find(|id| {
                let (source, _) = seek_before(&story, **id, SeekLimits::default()).unwrap();
                !scene_at(&story, &source, **id)
                    .unwrap()
                    .before
                    .characters
                    .is_empty()
            })
            .expect("a Show after another one");
        let (source, _) = seek_before(&story, second, SeekLimits::default()).unwrap();
        let at = scene_at(&story, &source, second).unwrap();
        let before = actors(&story, &at.before);
        let after = actors(&story, &at.after);
        assert!(!before.is_empty());
        assert!(after.len() >= before.len());
        // Every character on stage knows the Show that placed it.
        assert!(after.iter().all(|a| a.source.is_some()));
    }

    #[test]
    fn the_last_node_of_a_scene_and_non_show_nodes_are_fine() {
        let story = demo();
        for scene in &story.scenes {
            for node in &scene.nodes {
                let Ok((source, _)) = seek_before(&story, node.id, SeekLimits::default()) else {
                    continue;
                };
                scene_at(&story, &source, node.id)
                    .unwrap_or_else(|e| panic!("node {:?}: {e}", node.kind));
            }
        }
    }
}
