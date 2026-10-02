//! Where the preview starts and how it survives an edit: pure logic, no GPU,
//! unit tested.
//!
//! * [`plan_from_beginning`]: a fresh playthrough.
//! * [`plan_from_node`] ("play from here"): the VM state a player would have
//!   reached that node with, found by [`orbit_vm::seek`]; a node nothing
//!   leads to, or a search that grows too large, falls back to the default
//!   state with a notice.
//! * [`plan_resume`] (hot reload): the story changed under a running
//!   preview; keep going from the same point when the node still exists,
//!   else restart from the closest label before it, else from the beginning.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use orbit_core::{NodeId, NodeKind, Story, Value};
use orbit_vm::{SeekLimits, SeekOutcome, Vm, VmState, seek};

/// How the starting VM was obtained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartKind {
    /// From the first node.
    Beginning,
    /// The state of a path through the story's choices (`choices` taken).
    Path { choices: usize },
    /// The previous state, carried over to the edited story.
    Resumed,
    /// Starts at the node with the default state (no path found).
    Fallback,
}

/// A VM ready to be handed to the shell, and what to tell the author.
pub struct PlannedStart {
    pub vm: Vm,
    pub kind: StartKind,
    /// A warning for the console, when the result is not what was asked.
    pub notice: Option<String>,
}

/// What the preview was showing when the story changed.
#[derive(Clone, Debug)]
pub struct ResumeSource {
    /// The story the state belongs to (to find where a deleted node was).
    pub story: Arc<Story>,
    pub state: VmState,
    pub persistent: BTreeMap<String, Value>,
    pub read: BTreeSet<NodeId>,
}

impl ResumeSource {
    /// Captures the progress of `vm`.
    pub fn of(vm: &Vm) -> Self {
        Self {
            story: vm.story().clone(),
            state: vm.snapshot(),
            persistent: vm.persistent_vars().clone(),
            read: vm.read_nodes().clone(),
        }
    }

    /// The node that was on screen, else the one about to run.
    pub fn node(&self) -> Option<NodeId> {
        self.state
            .shown
            .as_ref()
            .map(orbit_vm::Interaction::node)
            .or(self.state.pc)
    }
}

/// A fresh playthrough.
pub fn plan_from_beginning(story: &Arc<Story>) -> Result<PlannedStart, String> {
    let vm = Vm::from_start(story.clone()).map_err(|e| e.to_string())?;
    Ok(PlannedStart {
        vm,
        kind: StartKind::Beginning,
        notice: None,
    })
}

fn node_label(story: &Story, id: NodeId) -> String {
    match story
        .locate(id)
        .map(|(s, n)| &story.scenes[s].nodes[n].kind)
    {
        Some(NodeKind::Label { name }) => format!("label « {name} »"),
        _ => id.to_string(),
    }
}

/// "Play from here": the state reached by playing up to `target`.
pub fn plan_from_node(
    story: &Arc<Story>,
    target: NodeId,
    limits: SeekLimits,
) -> Result<PlannedStart, String> {
    let fallback = |reason: String| -> Result<PlannedStart, String> {
        let vm = Vm::new(story.clone(), target).map_err(|e| e.to_string())?;
        Ok(PlannedStart {
            vm,
            kind: StartKind::Fallback,
            notice: Some(format!(
                "Jouer depuis ici : {reason} ; démarrage avec l'état par défaut."
            )),
        })
    };
    match seek(story, target, limits) {
        SeekOutcome::Found(path) => {
            let mut path = *path;
            let choices = path.choices.len();
            // Run the target itself: do not show the line before it again.
            path.state.shown = None;
            let vm = path.into_vm(story).map_err(|e| e.to_string())?;
            Ok(PlannedStart {
                vm,
                kind: StartKind::Path { choices },
                notice: None,
            })
        }
        SeekOutcome::Unreachable { expanded } => fallback(format!(
            "aucun chemin ne mène à {} depuis le début ({expanded} choix explorés)",
            node_label(story, target)
        )),
        SeekOutcome::LimitReached { expanded } => fallback(format!(
            "recherche de chemin trop longue ({expanded} choix explorés, borne atteinte)"
        )),
        SeekOutcome::Invalid(e) => Err(e.to_string()),
    }
}

/// The closest label at or before `node` in its scene of `old`, that still
/// exists in `new`; else the first node of that scene still in `new`.
fn nearest_anchor(old: &Story, new: &Story, node: NodeId) -> Option<NodeId> {
    let (scene, index) = old.locate(node)?;
    let nodes = &old.scenes[scene].nodes;
    let labels = nodes[..=index.min(nodes.len() - 1)].iter().rev();
    let found = labels
        .filter(|n| matches!(n.kind, NodeKind::Label { .. }))
        .map(|n| n.id)
        .find(|id| new.locate(*id).is_some());
    found.or_else(|| {
        nodes
            .iter()
            .map(|n| n.id)
            .find(|id| new.locate(*id).is_some())
    })
}

/// Continues `source` on the edited `story`.
pub fn plan_resume(
    story: &Arc<Story>,
    source: &ResumeSource,
    limits: SeekLimits,
) -> Result<PlannedStart, String> {
    let node = source.node();
    let present = |id: NodeId| story.locate(id).is_some();
    if node.is_none_or(present) {
        let entry = story.entry().ok_or("l'histoire est vide")?;
        let mut vm = Vm::new(story.clone(), entry).map_err(|e| e.to_string())?;
        vm.set_persistent_vars(&source.persistent);
        vm.set_read_nodes(source.read.clone());
        let mut state = source.state.clone();
        // Run the line on screen again rather than trusting what was shown:
        // its text, speaker or options may have been edited.
        match state.shown.take() {
            Some(shown) if state.pc.is_some() || state.pending != orbit_vm::Pending::None => {
                state.pc = Some(shown.node());
                state.pending = orbit_vm::Pending::None;
            }
            // The story had ended: it simply ends again.
            _ => {}
        }
        if vm.restore(state).is_ok() {
            return Ok(PlannedStart {
                vm,
                kind: StartKind::Resumed,
                notice: None,
            });
        }
        // The call stack points at a deleted node: restart close by below.
    }
    let anchor = node.and_then(|n| nearest_anchor(&source.story, story, n));
    match anchor {
        Some(anchor) => {
            let mut planned = plan_from_node(story, anchor, limits)?;
            let note = format!(
                "Le nœud en cours a changé : reprise à {}.",
                node_label(story, anchor)
            );
            planned.notice = Some(match planned.notice.take() {
                Some(other) => format!("{note} {other}"),
                None => note,
            });
            Ok(planned)
        }
        None => {
            let mut planned = plan_from_beginning(story)?;
            planned.notice =
                Some("Le nœud en cours a été supprimé : reprise au début de l'histoire.".into());
            Ok(planned)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{Node, Scene, SceneId};
    use orbit_vm::StepResult;

    const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

    fn demo() -> Arc<Story> {
        Arc::new(Story::from_ron_unchecked(DEMO).expect("demo parses"))
    }

    fn node(id: &str) -> NodeId {
        id.parse().unwrap()
    }

    /// Plays `vm` line by line, always taking option `pick`, up to `limit`
    /// steps or the end; returns the lines shown.
    fn read_lines(vm: &mut Vm, pick: usize, limit: usize) -> Vec<String> {
        let mut out = Vec::new();
        for _ in 0..limit {
            match vm.step() {
                StepResult::Say { string_id, .. } => {
                    out.push(vm.text(string_id).unwrap().to_owned());
                }
                StepResult::Choice { .. } => vm.choose(pick).unwrap(),
                StepResult::End | StepResult::Error(_) => break,
                _ => {}
            }
        }
        out
    }

    #[test]
    fn play_from_a_branch_node_gives_the_state_of_a_full_reading() {
        let story = demo();
        // The line Bob says in the "help" branch: after the choice.
        let target = node("01JBX0N0000000000000000021");
        let planned = plan_from_node(&story, target, SeekLimits::default()).unwrap();
        assert!(matches!(planned.kind, StartKind::Path { choices: 1 }));
        assert!(planned.notice.is_none());
        let mut vm = planned.vm;
        // Same state as playing the whole story up to this line and taking
        // the first option: variables, stage and music.
        let mut full = Vm::from_start(story.clone()).unwrap();
        loop {
            match full.step_to(target) {
                None => break,
                Some(StepResult::Choice { .. }) => full.choose(0).unwrap(),
                Some(StepResult::End | StepResult::Error(_)) => panic!("not reached"),
                Some(_) => {}
            }
        }
        let mut expected = full.snapshot();
        expected.shown = None; // the preview runs the target, it does not re-show the line before
        assert_eq!(vm.snapshot(), expected);
        assert_eq!(vm.position(), Some(target));
        assert_eq!(vm.variable("helped"), Some(&Value::Bool(true)));
        // And it plays on exactly like the full reading does.
        let from_here = read_lines(&mut vm, 0, 200);
        let from_full = read_lines(&mut full, 0, 200);
        assert_eq!(from_here, from_full);
    }

    #[test]
    fn an_unreachable_node_falls_back_to_the_default_state_with_a_notice() {
        let story = demo();
        // A scene nothing leads to.
        let mut tweaked = (*story).clone();
        let say = Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id: tweaked.strings.add("orphan"),
            },
        };
        let orphan = say.id;
        tweaked.scenes.push(Scene {
            id: SceneId::new(),
            name: "orphan".into(),
            nodes: vec![say],
        });
        // The demo's last node ends the story, so the orphan scene is only
        // reachable by falling through: cut that off with an End first.
        let last_scene = tweaked.scenes.len() - 2;
        tweaked.scenes[last_scene].nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::End,
        });
        let tweaked = Arc::new(tweaked);
        let planned = plan_from_node(&tweaked, orphan, SeekLimits::default()).unwrap();
        assert_eq!(planned.kind, StartKind::Fallback);
        assert!(planned.notice.as_deref().unwrap().contains("aucun chemin"));
        assert_eq!(planned.vm.position(), Some(orphan));
        assert_eq!(planned.vm.variable("helped"), Some(&Value::Bool(false)));
    }

    #[test]
    fn a_search_that_is_too_long_falls_back_too() {
        let story = demo();
        let target = node("01JBX0N0000000000000000046");
        let limits = SeekLimits {
            max_states: 0,
            max_steps: 5,
        };
        let planned = plan_from_node(&story, target, limits).unwrap();
        assert_eq!(planned.kind, StartKind::Fallback);
        assert!(planned.notice.unwrap().contains("trop longue"));
    }

    fn play_to_first_choice(story: &Arc<Story>) -> Vm {
        let mut vm = Vm::from_start(story.clone()).unwrap();
        loop {
            if let StepResult::Choice { .. } = vm.step() {
                return vm;
            }
        }
    }

    #[test]
    fn hot_reload_resumes_at_the_current_node() {
        let story = demo();
        let vm = play_to_first_choice(&story);
        let source = ResumeSource::of(&vm);
        let before = source.node().unwrap();

        // Edit a text (same ids), as a keystroke in the inspector would.
        let mut edited = (*story).clone();
        let some_text = edited
            .scenes
            .iter()
            .flat_map(|s| &s.nodes)
            .find_map(|n| match &n.kind {
                NodeKind::Say { text_id, .. } => Some(*text_id),
                _ => None,
            })
            .unwrap();
        edited.strings.set(some_text, "Edited line".to_owned());
        let edited = Arc::new(edited);

        let planned = plan_resume(&edited, &source, SeekLimits::default()).unwrap();
        assert_eq!(planned.kind, StartKind::Resumed);
        assert!(planned.notice.is_none());
        let mut resumed = planned.vm;
        // Back on the same choice, with the same stage.
        assert_eq!(resumed.scene(), vm.scene());
        match resumed.step() {
            StepResult::Choice { node, .. } => assert_eq!(node, before),
            other => panic!("expected the same choice, got {other:?}"),
        }
    }

    #[test]
    fn hot_reload_shows_an_edited_line_not_the_stale_one() {
        let story = demo();
        let mut vm = Vm::from_start(story.clone()).unwrap();
        let (node, text_id) = loop {
            if let StepResult::Say {
                node, string_id, ..
            } = vm.step()
            {
                break (node, string_id);
            }
        };
        let source = ResumeSource::of(&vm);
        let mut edited = (*story).clone();
        edited.strings.set(text_id, "Brand new text".to_owned());
        let edited = Arc::new(edited);
        let mut resumed = plan_resume(&edited, &source, SeekLimits::default())
            .unwrap()
            .vm;
        match resumed.step() {
            StepResult::Say {
                node: n, string_id, ..
            } => {
                assert_eq!(n, node);
                assert_eq!(resumed.text(string_id), Some("Brand new text"));
            }
            other => panic!("expected the line again, got {other:?}"),
        }
    }

    #[test]
    fn a_deleted_node_resumes_at_the_closest_label_before_it() {
        let story = demo();
        // Stand on the choice, inside the first scene.
        let vm = play_to_first_choice(&story);
        let source = ResumeSource::of(&vm);
        let choice = source.node().unwrap();

        // Delete the choice node itself.
        let mut edited = (*story).clone();
        for scene in &mut edited.scenes {
            scene.nodes.retain(|n| n.id != choice);
        }
        // The jump targets of its options are still there; the story stays
        // loadable for the VM.
        let edited = Arc::new(edited);
        assert!(edited.locate(choice).is_none());
        let planned = plan_resume(&edited, &source, SeekLimits::default()).unwrap();
        let notice = planned.notice.expect("the author is told");
        assert!(notice.contains("reprise"), "{notice}");
        // The closest label before the choice in its scene is `first_scene`
        // (or `welcome_back`'s sibling): the VM stands on a label of the
        // new story.
        let at = planned.vm.position().unwrap();
        assert!(matches!(
            edited.scenes[edited.locate(at).unwrap().0].nodes[edited.locate(at).unwrap().1].kind,
            NodeKind::Label { .. }
        ));
    }

    #[test]
    fn without_any_anchor_the_story_restarts_from_the_beginning() {
        let story = demo();
        let vm = play_to_first_choice(&story);
        let source = ResumeSource::of(&vm);
        // A brand new story: no node of the old one exists any more.
        let mut other = Story::new("other");
        let line = Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id: other.strings.add("hi"),
            },
        };
        let first = line.id;
        other.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: vec![line],
        });
        let planned = plan_resume(&Arc::new(other), &source, SeekLimits::default()).unwrap();
        assert_eq!(planned.kind, StartKind::Beginning);
        assert!(planned.notice.unwrap().contains("début"));
        assert_eq!(planned.vm.position(), Some(first));
    }

    #[test]
    fn a_story_that_was_ended_resumes_as_ended() {
        let story = demo();
        let mut vm = Vm::from_start(story.clone()).unwrap();
        for _ in 0..500 {
            match vm.step() {
                StepResult::Choice { .. } => vm.choose(0).unwrap(),
                StepResult::End => break,
                _ => {}
            }
        }
        assert!(vm.is_finished());
        let source = ResumeSource::of(&vm);
        let planned = plan_resume(&story, &source, SeekLimits::default()).unwrap();
        assert_eq!(planned.kind, StartKind::Resumed);
        assert!(planned.vm.is_finished());
    }
}
