//! Reaching a node without a player: "play from here".
//!
//! [`seek`] runs the VM headless (no GPU, no clock) from the start of the
//! story and explores the choices breadth-first until the next node to
//! execute is the target. Breadth-first means the path with the fewest
//! decisions wins; options are tried in order, so the result is
//! deterministic. The returned [`SeekPath`] rebuilds a VM whose state is
//! exactly the one a player would have by taking those choices.
//!
//! The search is bounded ([`SeekLimits`]) because the number of paths grows
//! exponentially with the number of choices. Choice points already met with
//! the same variables and call stack are not explored twice (loops and
//! diamonds of the graph), so a path that differs only by the background or
//! the sprites on stage is not distinguished from the first one found.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::Arc;

use orbit_core::{NodeId, Story, Value};

use crate::error::VmError;
use crate::event::StepResult;
use crate::state::VmState;
use crate::vm::Vm;

/// Bounds of the search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekLimits {
    /// Distinct choice points expanded at most.
    pub max_states: usize,
    /// `Vm::step_to` calls (lines, pauses, choices) at most, over the whole
    /// search.
    pub max_steps: usize,
}

impl Default for SeekLimits {
    fn default() -> Self {
        Self {
            max_states: 4_000,
            max_steps: 200_000,
        }
    }
}

/// One decision of a path: option `index` (among the visible ones, as
/// `Vm::choose` counts them) of the choice node `node`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChoiceStep {
    pub node: NodeId,
    pub index: usize,
}

/// A way to reach the target.
#[derive(Clone, Debug)]
pub struct SeekPath {
    /// The VM state right before the target node runs.
    pub state: VmState,
    /// Persistent variables as they stand then (they are not in `state`).
    pub persistent: BTreeMap<String, Value>,
    /// Lines already read along the way.
    pub read: BTreeSet<NodeId>,
    /// The decisions taken, in order.
    pub choices: Vec<ChoiceStep>,
    /// Steps (lines, pauses, choices) the whole search executed.
    pub steps: usize,
}

impl SeekPath {
    /// A VM standing right before the target, as if played up to there.
    pub fn into_vm(self, story: &Arc<Story>) -> Result<Vm, VmError> {
        let entry = story.entry().ok_or(VmError::EmptyStory)?;
        let mut vm = Vm::new(story.clone(), entry)?;
        vm.set_persistent_vars(&self.persistent);
        vm.set_read_nodes(self.read);
        vm.restore(self.state)?;
        Ok(vm)
    }
}

/// What the search found.
#[derive(Clone, Debug)]
pub enum SeekOutcome {
    Found(Box<SeekPath>),
    /// Every path was explored and none gets there (a condition that is
    /// never true, a scene nothing jumps to...).
    Unreachable {
        expanded: usize,
    },
    /// The bounds were hit before an answer: the target may or may not be
    /// reachable.
    LimitReached {
        expanded: usize,
    },
    /// The story cannot be played at all, or the target is not in it.
    Invalid(VmError),
}

/// How a linear stretch of the story ended.
enum Segment {
    Reached,
    /// Blocked on a choice with `options` visible options.
    Choice {
        node: NodeId,
        options: usize,
    },
    /// The story ended or failed before the target.
    Dead,
    /// The step budget ran out.
    Exhausted,
}

struct Frontier {
    state: VmState,
    persistent: BTreeMap<String, Value>,
    read: BTreeSet<NodeId>,
    choices: Vec<ChoiceStep>,
    /// Option to take first (`None` for the very first segment).
    pick: Option<usize>,
}

/// Runs `vm` until the target, a choice, the end or the budget.
fn run_segment(vm: &mut Vm, target: NodeId, steps: &mut usize, budget: usize) -> Segment {
    loop {
        if *steps >= budget {
            return Segment::Exhausted;
        }
        *steps += 1;
        match vm.step_to(target) {
            None => return Segment::Reached,
            Some(StepResult::Choice { node, options }) => {
                return Segment::Choice {
                    node,
                    options: options.len(),
                };
            }
            Some(StepResult::End | StepResult::Error(_)) => return Segment::Dead,
            Some(StepResult::Say { .. })
            | Some(StepResult::WaitTime(_))
            | Some(StepResult::WaitTransition { .. }) => {}
        }
        // Events are of no use here; do not let them pile up.
        vm.take_events();
    }
}

/// Key of a choice point for the "already explored" set.
fn visit_key(vm: &Vm, node: NodeId) -> String {
    let snapshot = vm.snapshot();
    format!(
        "{node:?}|{:?}|{:?}|{:?}",
        snapshot.vars, snapshot.call_stack, snapshot.pending
    ) + &format!("|{:?}", vm.persistent_vars())
}

/// Finds the choices that lead from the start of `story` to `target`.
pub fn seek(story: &Arc<Story>, target: NodeId, limits: SeekLimits) -> SeekOutcome {
    let mut vm = match Vm::from_start(story.clone()) {
        Ok(vm) => vm,
        Err(e) => return SeekOutcome::Invalid(e),
    };
    if story.locate(target).is_none() {
        return SeekOutcome::Invalid(VmError::UnknownNode(target));
    }
    let mut queue = VecDeque::new();
    queue.push_back(Frontier {
        state: vm.snapshot(),
        persistent: vm.persistent_vars().clone(),
        read: vm.read_nodes().clone(),
        choices: Vec::new(),
        pick: None,
    });
    let mut seen: HashSet<String> = HashSet::new();
    let mut expanded = 0usize;
    let mut steps = 0usize;
    let mut limit_hit = false;

    while let Some(frontier) = queue.pop_front() {
        // Resume exactly where the parent choice point was.
        if frontier.pick.is_some() {
            if vm.restore(frontier.state.clone()).is_err() {
                continue;
            }
            vm.set_persistent_vars(&frontier.persistent);
            vm.set_read_nodes(frontier.read.clone());
        }
        let mut choices = frontier.choices.clone();
        if let Some(index) = frontier.pick {
            let Some(last) = frontier.state.pending_choice_node() else {
                continue;
            };
            if vm.choose(index).is_err() {
                continue;
            }
            choices.push(ChoiceStep { node: last, index });
        }
        match run_segment(&mut vm, target, &mut steps, limits.max_steps) {
            Segment::Reached => {
                return SeekOutcome::Found(Box::new(SeekPath {
                    state: vm.snapshot(),
                    persistent: vm.persistent_vars().clone(),
                    read: vm.read_nodes().clone(),
                    choices,
                    steps,
                }));
            }
            Segment::Dead => {}
            Segment::Exhausted => {
                limit_hit = true;
                break;
            }
            Segment::Choice { node, options } => {
                if !seen.insert(visit_key(&vm, node)) {
                    continue;
                }
                expanded += 1;
                if expanded > limits.max_states {
                    limit_hit = true;
                    break;
                }
                let state = vm.snapshot();
                let persistent = vm.persistent_vars().clone();
                let read = vm.read_nodes().clone();
                for index in 0..options {
                    queue.push_back(Frontier {
                        state: state.clone(),
                        persistent: persistent.clone(),
                        read: read.clone(),
                        choices: choices.clone(),
                        pick: Some(index),
                    });
                }
            }
        }
    }
    if limit_hit {
        SeekOutcome::LimitReached { expanded }
    } else {
        SeekOutcome::Unreachable { expanded }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::{ChoiceOption, Expr, Node, NodeKind, Scene, SceneId};

    const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

    fn demo() -> Arc<Story> {
        Arc::new(Story::from_ron_unchecked(DEMO).expect("demo parses"))
    }

    fn node(id: &str) -> NodeId {
        id.parse().expect("valid node id")
    }

    /// Plays the demo like a person who answers every choice with `picks`
    /// (in order), stopping right before `target` runs. Uses only the
    /// stepping API, independently of the search.
    fn played_to(story: &Arc<Story>, target: NodeId, picks: &[usize]) -> Vm {
        let mut vm = Vm::from_start(story.clone()).unwrap();
        let mut picks = picks.iter();
        for _ in 0..500 {
            match vm.step_to(target) {
                None => return vm,
                Some(StepResult::Choice { .. }) => {
                    let pick = *picks.next().expect("a pick for every choice");
                    vm.choose(pick).unwrap();
                }
                Some(StepResult::End | StepResult::Error(_)) => panic!("target not reached"),
                Some(_) => {}
            }
        }
        panic!("too many steps");
    }

    #[test]
    fn a_node_in_a_branch_has_the_state_of_a_full_reading() {
        let story = demo();
        // "help" branch: SetVar(helped) right after the first choice.
        let target = node("01JBX0N0000000000000000020");
        let SeekOutcome::Found(path) = seek(&story, target, SeekLimits::default()) else {
            panic!("reachable");
        };
        assert_eq!(path.choices.len(), 1);
        assert_eq!(path.choices[0].index, 0);

        let reference = played_to(&story, target, &[0]);
        assert_eq!(path.state, reference.snapshot());
        assert_eq!(path.read, *reference.read_nodes());

        let vm = path.into_vm(&story).unwrap();
        assert_eq!(vm.position(), Some(target));
        assert_eq!(vm.snapshot(), reference.snapshot());
        // Real state, not defaults: the room, the music, both characters, and
        // `helped` still false (its SetVar is the target, not yet run).
        let scene = vm.scene();
        assert!(scene.background.is_some());
        assert!(scene.music.is_some());
        assert_eq!(scene.characters.len(), 2);
        assert_eq!(vm.variable("helped"), Some(&Value::Bool(false)));
    }

    #[test]
    fn choices_and_variables_decide_which_path_is_taken() {
        let story = demo();
        // "thanks" only runs when `helped` is true: option 0 at the choice.
        let thanks = node("01JBX0N0000000000000000039");
        let SeekOutcome::Found(path) = seek(&story, thanks, SeekLimits::default()) else {
            panic!("reachable");
        };
        assert_eq!(path.choices.len(), 1);
        assert_eq!(path.choices[0].index, 0);
        let vm = path.into_vm(&story).unwrap();
        assert_eq!(vm.variable("helped"), Some(&Value::Bool(true)));
        assert_eq!(vm.snapshot(), played_to(&story, thanks, &[0]).snapshot());

        // "grumpy" only runs when `helped` is false: option 1.
        let grumpy = node("01JBX0N0000000000000000040");
        let SeekOutcome::Found(path) = seek(&story, grumpy, SeekLimits::default()) else {
            panic!("reachable");
        };
        assert_eq!(path.choices[0].index, 1);
        let vm = path.into_vm(&story).unwrap();
        assert_eq!(vm.variable("helped"), Some(&Value::Bool(false)));
        assert_eq!(vm.snapshot(), played_to(&story, grumpy, &[1]).snapshot());
    }

    #[test]
    fn the_start_and_a_straight_line_need_no_choice() {
        let story = demo();
        let first = story.entry().unwrap();
        let SeekOutcome::Found(path) = seek(&story, first, SeekLimits::default()) else {
            panic!("reachable");
        };
        assert!(path.choices.is_empty());
        let target = node("01JBX0N0000000000000000010");
        let SeekOutcome::Found(path) = seek(&story, target, SeekLimits::default()) else {
            panic!("reachable");
        };
        assert!(path.choices.is_empty());
        assert_eq!(
            path.into_vm(&story).unwrap().snapshot(),
            played_to(&story, target, &[]).snapshot()
        );
    }

    fn say(story: &mut Story, text: &str) -> Node {
        Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id: story.strings.add(text),
            },
        }
    }

    #[test]
    fn a_node_behind_a_false_condition_is_unreachable() {
        let mut story = Story::new("t");
        let hidden = say(&mut story, "never shown");
        let after = say(&mut story, "after");
        let (hidden_id, after_id) = (hidden.id, after.id);
        let branch = Node {
            id: NodeId::new(),
            kind: NodeKind::If {
                condition: Expr::Lit(Value::Bool(false)),
                then_target: hidden_id,
                else_target: Some(after_id),
            },
        };
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "a".into(),
            nodes: vec![
                branch,
                after,
                Node {
                    id: NodeId::new(),
                    kind: NodeKind::End,
                },
                hidden,
            ],
        });
        let story = Arc::new(story);
        assert!(matches!(
            seek(&story, hidden_id, SeekLimits::default()),
            SeekOutcome::Unreachable { .. }
        ));
        assert!(matches!(
            seek(&story, after_id, SeekLimits::default()),
            SeekOutcome::Found(_)
        ));
        assert!(matches!(
            seek(&story, NodeId::new(), SeekLimits::default()),
            SeekOutcome::Invalid(VmError::UnknownNode(_))
        ));
    }

    /// `n` successive two-way choices whose options all lead to the next
    /// choice: 2^n paths but only `n` distinct choice points.
    fn ladder(n: usize) -> (Arc<Story>, NodeId) {
        let mut story = Story::new("ladder");
        let choice_ids: Vec<NodeId> = (0..n).map(|_| NodeId::new()).collect();
        let end = say(&mut story, "end");
        let end_id = end.id;
        let mut nodes = Vec::new();
        for i in 0..n {
            let next = choice_ids.get(i + 1).copied().unwrap_or(end_id);
            let mut options = Vec::new();
            for text in ["left", "right"] {
                let landing = NodeId::new();
                options.push(ChoiceOption {
                    text_id: story.strings.add(text),
                    target: landing,
                    condition: None,
                });
                nodes.push((
                    landing,
                    Node {
                        id: landing,
                        kind: NodeKind::Jump { target: next },
                    },
                ));
            }
            nodes.push((
                choice_ids[i],
                Node {
                    id: choice_ids[i],
                    kind: NodeKind::Choice { options },
                },
            ));
        }
        // Landing pads after their choice, in order; the first node of the
        // scene must be the first choice.
        let mut ordered = Vec::new();
        for (i, id) in choice_ids.iter().enumerate() {
            ordered.push(nodes.iter().find(|(k, _)| k == id).unwrap().1.clone());
            for (_, n) in &nodes[i * 3..i * 3 + 2] {
                ordered.push(n.clone());
            }
        }
        ordered.push(end);
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: ordered,
        });
        (Arc::new(story), end_id)
    }

    #[test]
    fn choice_points_with_the_same_state_are_not_explored_twice() {
        // 30 successive choices: 2^30 paths without the "seen" set.
        let (story, end) = ladder(30);
        let SeekOutcome::Found(path) = seek(&story, end, SeekLimits::default()) else {
            panic!("found quickly");
        };
        assert_eq!(path.choices.len(), 30);
        assert_eq!(path.state.pc, Some(end));
    }

    #[test]
    fn the_search_is_bounded() {
        let (story, end) = ladder(30);
        let few_states = SeekLimits {
            max_states: 5,
            max_steps: 200_000,
        };
        assert!(matches!(
            seek(&story, end, few_states),
            SeekOutcome::LimitReached { .. }
        ));
        let few_steps = SeekLimits {
            max_states: 4_000,
            max_steps: 10,
        };
        assert!(matches!(
            seek(&story, end, few_steps),
            SeekOutcome::LimitReached { .. }
        ));
    }

    #[test]
    fn the_result_is_deterministic() {
        let story = demo();
        let target = node("01JBX0N0000000000000000046");
        let run = || match seek(&story, target, SeekLimits::default()) {
            SeekOutcome::Found(p) => (p.choices, p.state),
            other => panic!("{other:?}"),
        };
        assert_eq!(run(), run());
    }
}
