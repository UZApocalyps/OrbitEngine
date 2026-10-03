//! The macro flow graph of a story, computed from the IR.
//!
//! The IR is linear (a scene is a list of nodes, control flow is expressed by
//! `Jump` / `Choice` / `If` / `Call` nodes targeting `Label` nodes, and
//! `JumpScene` nodes leaving for another scene), so a
//! readable graph works on **blocks**: runs of nodes that execute in
//! sequence. A scene starts a block (its *head*, [`BlockId::Scene`], whether
//! or not it begins with a label) and every further `Label` of the scene
//! starts another one ([`BlockId::Label`]). Edges come from the **slots** of a
//! block: one per jump, per scene jump, per choice option, per `If` branch, per
//! call, plus the fall-through to the next block **of the same scene** when
//! nothing ends the sequence. A scene never continues into the next one: its
//! last block, when nothing ends it, ends the game.
//!
//! Nothing here touches the story: edits are in [`crate::graph_ops`], and the
//! graph is recomputed from the IR after each change (it is `O(nodes)`).

use std::collections::{HashMap, HashSet, VecDeque};

use orbit_core::{CharacterId, NodeId, NodeKind, SceneId, Story};

/// Identity of a block (the unit shown as a node of the graph).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlockId {
    /// The head of a scene: its nodes up to the first label that is not its
    /// first node. Exists for every scene, even an empty one.
    Scene(SceneId),
    /// A label that is not the first node of its scene, and what follows it.
    Label(NodeId),
}

impl BlockId {
    /// Stable text form, used as key in the layout sidecar.
    pub fn key(&self) -> String {
        match self {
            BlockId::Scene(id) => format!("s:{id}"),
            BlockId::Label(id) => format!("l:{id}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let (kind, id) = key.split_once(':')?;
        match kind {
            "s" => id.parse().ok().map(BlockId::Scene),
            "l" => id.parse().ok().map(BlockId::Label),
            _ => None,
        }
    }
}

/// What an output of a block does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotKind {
    /// Fall-through to the next block of the scene (the block has no
    /// terminator); with no next block, the game ends there.
    Next,
    Jump,
    /// A jump to another scene (to its start or to one of its labels).
    JumpScene,
    Call,
    /// The `n`-th option of a choice.
    Option(usize),
    Then,
    Else,
}

/// Where a slot leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotTarget {
    /// Nowhere: the game ends (`Next` of the last block of a scene) or, for
    /// `Else`, execution simply continues.
    None,
    Block(BlockId),
    /// The slot names a node that is not a label (or does not exist).
    Missing(NodeId),
}

/// An output of a block.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    /// The node that carries it (`None` for the fall-through).
    pub node: Option<NodeId>,
    pub kind: SlotKind,
    /// Text shown next to the pin (option text, condition...).
    pub label: String,
    pub target: SlotTarget,
    /// The option / branch only exists under a condition.
    pub conditional: bool,
}

/// A slot of a given block, as an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotRef {
    pub block: BlockId,
    pub node: Option<NodeId>,
    pub kind: SlotKind,
}

/// What ends the sequence of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminator {
    Jump,
    JumpScene,
    Choice,
    End,
    Return,
}

impl Terminator {
    /// Stable name of the terminator, for front ends that write the label
    /// in their own language.
    pub fn code(self) -> &'static str {
        match self {
            Terminator::Jump => "jump",
            Terminator::JumpScene => "jump_scene",
            Terminator::Choice => "choice",
            Terminator::End => "end",
            Terminator::Return => "return",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Terminator::Jump => "un saut",
            Terminator::JumpScene => "un saut de scène",
            Terminator::Choice => "un choix",
            Terminator::End => "une fin",
            Terminator::Return => "un retour",
        }
    }
}

/// A node of the graph: a run of story nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: BlockId,
    pub scene: SceneId,
    pub scene_name: String,
    /// Scene name for a head, label name otherwise.
    pub title: String,
    /// The head of its scene.
    pub is_head: bool,
    /// The label that opens the block, if any (a head may not have one).
    pub label: Option<NodeId>,
    /// All the nodes of the block, in order.
    pub nodes: Vec<NodeId>,
    /// Number of lines of dialogue.
    pub lines: usize,
    /// Characters that speak in the block, in order of appearance.
    pub speakers: Vec<CharacterId>,
    /// Whether the narrator speaks in the block.
    pub narrator: bool,
    /// Text of the first line of dialogue.
    pub first_line: Option<String>,
    pub slots: Vec<Slot>,
    /// The first node that ends the sequence, if any.
    pub terminator: Option<(NodeId, Terminator)>,
    /// Nodes after the terminator, which never run.
    pub dead_nodes: usize,
}

impl Block {
    /// The node to select for this block: its first one.
    pub fn head_node(&self) -> Option<NodeId> {
        self.nodes.first().copied()
    }

    pub fn has_choice(&self) -> bool {
        self.slots
            .iter()
            .any(|s| matches!(s.kind, SlotKind::Option(_)))
    }

    pub fn has_condition(&self) -> bool {
        self.slots.iter().any(|s| s.kind == SlotKind::Then)
    }

    pub fn has_call(&self) -> bool {
        self.slots.iter().any(|s| s.kind == SlotKind::Call)
    }

    pub fn ends_story(&self) -> bool {
        matches!(self.terminator, Some((_, Terminator::End)))
    }

    /// The game ends at the end of this block: nothing ends it and there is
    /// no next block in the scene.
    pub fn falls_off_scene(&self) -> bool {
        self.slots
            .last()
            .is_some_and(|s| s.kind == SlotKind::Next && s.target == SlotTarget::None)
    }
}

/// An edge between two blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: SlotRef,
    pub to: BlockId,
}

/// The graph of a story.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphModel {
    /// Blocks in story order (scene by scene).
    pub blocks: Vec<Block>,
    index: HashMap<BlockId, usize>,
    node_block: HashMap<NodeId, BlockId>,
    /// Blocks no path from the start of the story reaches.
    pub unreachable: HashSet<BlockId>,
}

impl GraphModel {
    /// Computes the graph of `story`.
    pub fn build(story: &Story) -> Self {
        // Pass 1: cut every scene into blocks.
        let mut blocks: Vec<Block> = Vec::new();
        let mut label_block: HashMap<NodeId, BlockId> = HashMap::new();
        let mut node_block: HashMap<NodeId, BlockId> = HashMap::new();
        for scene in &story.scenes {
            let mut current = new_block(BlockId::Scene(scene.id), scene.id, &scene.name, true);
            for node in &scene.nodes {
                if let NodeKind::Label { name } = &node.kind {
                    if current.nodes.is_empty() {
                        // First node of the scene: the head keeps the scene id.
                        current.label = Some(node.id);
                    } else {
                        blocks.push(std::mem::replace(
                            &mut current,
                            new_block(BlockId::Label(node.id), scene.id, name, false),
                        ));
                        current.scene_name = scene.name.clone();
                        current.label = Some(node.id);
                    }
                    label_block.insert(node.id, current.id);
                }
                node_block.insert(node.id, current.id);
                current.nodes.push(node.id);
            }
            blocks.push(current);
        }
        let index: HashMap<BlockId, usize> =
            blocks.iter().enumerate().map(|(i, b)| (b.id, i)).collect();

        // Pass 2: summaries and slots.
        let resolve = |id: NodeId| match label_block.get(&id) {
            Some(b) => SlotTarget::Block(*b),
            None => SlotTarget::Missing(id),
        };
        // The next block, only within the same scene.
        let next_ids: Vec<Option<BlockId>> = (0..blocks.len())
            .map(|i| {
                blocks
                    .get(i + 1)
                    .filter(|b| b.scene == blocks[i].scene)
                    .map(|b| b.id)
            })
            .collect();
        // Where a jump to a scene lands: its head block, or the block of a label.
        let scene_names: HashMap<SceneId, String> = story
            .scenes
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect();
        for (bi, block) in blocks.iter_mut().enumerate() {
            let mut terminator: Option<(NodeId, Terminator)> = None;
            let mut dead = 0;
            for &id in &block.nodes.clone() {
                let Some(node) = story.node(id) else { continue };
                if terminator.is_some() {
                    dead += 1;
                    continue;
                }
                match &node.kind {
                    NodeKind::Say { speaker, text_id, .. } => {
                        block.lines += 1;
                        match speaker {
                            Some(c) if !block.speakers.contains(c) => block.speakers.push(*c),
                            None => block.narrator = true,
                            _ => {}
                        }
                        if block.first_line.is_none() {
                            block.first_line = story.text(*text_id).map(str::to_owned);
                        }
                    }
                    NodeKind::Jump { target } => {
                        block.slots.push(Slot {
                            node: Some(id),
                            kind: SlotKind::Jump,
                            label: String::new(),
                            target: resolve(*target),
                            conditional: false,
                        });
                        terminator = Some((id, Terminator::Jump));
                    }
                    NodeKind::JumpScene { scene, label } => {
                        let target = match (label, scene_names.contains_key(scene)) {
                            (Some(l), _) => resolve(*l),
                            (None, true) => SlotTarget::Block(BlockId::Scene(*scene)),
                            (None, false) => SlotTarget::Missing(id),
                        };
                        let name = scene_names.get(scene).map_or("?", String::as_str);
                        let label_text = match label.and_then(|l| story.node(l)).map(|n| &n.kind) {
                            Some(NodeKind::Label { name: l }) => format!(" › {l}"),
                            _ => String::new(),
                        };
                        block.slots.push(Slot {
                            node: Some(id),
                            kind: SlotKind::JumpScene,
                            label: format!("scène « {name} »{label_text}"),
                            target,
                            conditional: false,
                        });
                        terminator = Some((id, Terminator::JumpScene));
                    }
                    NodeKind::Call { label } => block.slots.push(Slot {
                        node: Some(id),
                        kind: SlotKind::Call,
                        label: String::new(),
                        target: resolve(*label),
                        conditional: false,
                    }),
                    NodeKind::Choice { options } => {
                        for (i, option) in options.iter().enumerate() {
                            block.slots.push(Slot {
                                node: Some(id),
                                kind: SlotKind::Option(i),
                                label: story.text(option.text_id).unwrap_or("?").to_owned(),
                                target: resolve(option.target),
                                conditional: option.condition.is_some(),
                            });
                        }
                        terminator = Some((id, Terminator::Choice));
                    }
                    NodeKind::If {
                        condition,
                        then_target,
                        else_target,
                    } => {
                        block.slots.push(Slot {
                            node: Some(id),
                            kind: SlotKind::Then,
                            label: format!("si {condition}"),
                            target: resolve(*then_target),
                            conditional: true,
                        });
                        block.slots.push(Slot {
                            node: Some(id),
                            kind: SlotKind::Else,
                            label: "sinon".to_owned(),
                            target: else_target.map_or(SlotTarget::None, &resolve),
                            conditional: true,
                        });
                    }
                    NodeKind::End => terminator = Some((id, Terminator::End)),
                    NodeKind::Return => terminator = Some((id, Terminator::Return)),
                    _ => {}
                }
            }
            block.dead_nodes = dead;
            block.terminator = terminator;
            if terminator.is_none() {
                block.slots.push(Slot {
                    node: None,
                    kind: SlotKind::Next,
                    label: "suite".to_owned(),
                    target: next_ids[bi].map_or(SlotTarget::None, SlotTarget::Block),
                    conditional: false,
                });
            }
        }

        let mut model = Self {
            blocks,
            index,
            node_block,
            unreachable: HashSet::new(),
        };
        model.compute_reachability();
        model
    }

    fn compute_reachability(&mut self) {
        let Some(entry) = self.blocks.first().map(|b| b.id) else {
            return;
        };
        let mut seen: HashSet<BlockId> = HashSet::from([entry]);
        let mut queue = VecDeque::from([entry]);
        while let Some(id) = queue.pop_front() {
            let Some(block) = self.block(id) else {
                continue;
            };
            for slot in &block.slots {
                if let SlotTarget::Block(to) = slot.target
                    && seen.insert(to)
                {
                    queue.push_back(to);
                }
            }
        }
        self.unreachable = self
            .blocks
            .iter()
            .map(|b| b.id)
            .filter(|id| !seen.contains(id))
            .collect();
    }

    pub fn block(&self, id: BlockId) -> Option<&Block> {
        self.index.get(&id).map(|i| &self.blocks[*i])
    }

    /// The block holding a story node.
    pub fn block_of(&self, node: NodeId) -> Option<BlockId> {
        self.node_block.get(&node).copied()
    }

    /// The block the story starts in.
    pub fn entry(&self) -> Option<BlockId> {
        self.blocks.first().map(|b| b.id)
    }

    /// Every edge, in block order.
    pub fn edges(&self) -> Vec<Edge> {
        let mut out = Vec::new();
        for block in &self.blocks {
            for slot in &block.slots {
                if let SlotTarget::Block(to) = slot.target {
                    out.push(Edge {
                        from: SlotRef {
                            block: block.id,
                            node: slot.node,
                            kind: slot.kind,
                        },
                        to,
                    });
                }
            }
        }
        out
    }

    /// Slots (of any block) that lead to `block`.
    pub fn incoming(&self, block: BlockId) -> Vec<SlotRef> {
        self.edges()
            .into_iter()
            .filter(|e| e.to == block)
            .map(|e| e.from)
            .collect()
    }
}

fn new_block(id: BlockId, scene: SceneId, title: &str, is_head: bool) -> Block {
    Block {
        id,
        scene,
        scene_name: title.to_owned(),
        title: title.to_owned(),
        is_head,
        label: None,
        nodes: Vec::new(),
        lines: 0,
        speakers: Vec::new(),
        narrator: false,
        first_line: None,
        slots: Vec::new(),
        terminator: None,
        dead_nodes: 0,
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{ChoiceOption, Expr, Node, Scene, Value};

    use super::*;

    fn node(kind: NodeKind) -> Node {
        Node {
            id: NodeId::new(),
            kind,
        }
    }

    /// start: say, choice(A, B) ; A: say, jump end ; B: if → end2 else suite ;
    /// end: End ; second scene: label, call(sub), End ; sub: Return.
    fn sample() -> (Story, Vec<NodeId>) {
        let mut story = Story::new("t");
        let l_a = node(NodeKind::Label { name: "a".into() });
        let l_b = node(NodeKind::Label { name: "b".into() });
        let l_end = node(NodeKind::Label { name: "end".into() });
        let l_sub = node(NodeKind::Label { name: "sub".into() });
        let say_text = story.strings.add("Bonjour");
        let opt_a = story.strings.add("Option A");
        let opt_b = story.strings.add("Option B");
        let say = node(NodeKind::Say {
            speaker: None,
            text_id: say_text,
            voice: None,
        });
        let choice = node(NodeKind::Choice {
            options: vec![
                ChoiceOption {
                    text_id: opt_a,
                    target: l_a.id,
                    condition: None,
                },
                ChoiceOption {
                    text_id: opt_b,
                    target: l_b.id,
                    condition: None,
                },
            ],
        });
        let jump = node(NodeKind::Jump { target: l_end.id });
        let cond = node(NodeKind::If {
            condition: Expr::Lit(Value::Bool(true)),
            then_target: l_end.id,
            else_target: None,
        });
        let end = node(NodeKind::End);
        let call = node(NodeKind::Call { label: l_sub.id });
        let ret = node(NodeKind::Return);
        let ids = vec![
            say.id, choice.id, l_a.id, l_b.id, l_end.id, l_sub.id, call.id,
        ];
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "intro".into(),
            nodes: vec![
                say,
                choice,
                l_a,
                node(NodeKind::End),
                l_b,
                cond,
                l_end.clone(),
                end,
            ],
        });
        let _ = jump;
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "suite".into(),
            nodes: vec![call, node(NodeKind::End), l_sub, ret],
        });
        (story, ids)
    }

    #[test]
    fn blocks_follow_scenes_and_labels() {
        let (story, ids) = sample();
        let model = GraphModel::build(&story);
        // head of intro, a, b, end ; head of suite, sub
        assert_eq!(model.blocks.len(), 6);
        assert!(model.blocks[0].is_head && model.blocks[4].is_head);
        assert_eq!(model.blocks[0].title, "intro");
        assert_eq!(model.blocks[1].title, "a");
        assert_eq!(model.blocks[0].lines, 1);
        assert_eq!(model.blocks[0].first_line.as_deref(), Some("Bonjour"));
        assert!(model.blocks[0].narrator);
        assert_eq!(model.block_of(ids[0]), Some(model.blocks[0].id));
        assert_eq!(model.block_of(ids[2]), Some(model.blocks[1].id));
    }

    #[test]
    fn slots_cover_choices_conditions_calls_and_fall_through() {
        let (story, ids) = sample();
        let model = GraphModel::build(&story);
        let head = &model.blocks[0];
        assert_eq!(head.slots.len(), 2);
        assert_eq!(head.slots[0].kind, SlotKind::Option(0));
        assert_eq!(head.slots[0].label, "Option A");
        assert_eq!(
            head.slots[1].target,
            SlotTarget::Block(BlockId::Label(ids[3]))
        );
        assert_eq!(head.terminator.map(|t| t.1), Some(Terminator::Choice));
        // a: ends the story, no outputs.
        assert!(model.blocks[1].slots.is_empty());
        assert!(model.blocks[1].ends_story());
        // b: then → end, else (continue), and falls through into `end`.
        let b = &model.blocks[2];
        let kinds: Vec<_> = b.slots.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [SlotKind::Then, SlotKind::Else, SlotKind::Next]);
        assert_eq!(b.slots[2].target, SlotTarget::Block(model.blocks[3].id));
        // suite: call sub, then End.
        let suite = &model.blocks[4];
        assert_eq!(suite.slots[0].kind, SlotKind::Call);
        assert_eq!(
            suite.slots[0].target,
            SlotTarget::Block(BlockId::Label(ids[5]))
        );
        assert!(suite.has_call());
        assert_eq!(model.edges().len(), 2 + 2 + 1);
    }

    #[test]
    fn unreachable_blocks_and_missing_targets_are_found() {
        let (mut story, _) = sample();
        // A label nothing points to, and a jump to a node that is no label.
        let orphan = node(NodeKind::Label {
            name: "orphan".into(),
        });
        let orphan_id = orphan.id;
        let say_id = story.scenes[0].nodes[0].id;
        let bad_jump = node(NodeKind::Jump { target: say_id });
        story.scenes[1].nodes.push(orphan);
        story.scenes[1].nodes.push(bad_jump);
        let model = GraphModel::build(&story);
        assert!(model.unreachable.contains(&BlockId::Label(orphan_id)));
        let last = model.blocks.last().unwrap();
        assert_eq!(last.slots[0].target, SlotTarget::Missing(say_id));
        // The first scene is the entry: reachable.
        assert!(!model.unreachable.contains(&model.blocks[0].id));
    }

    #[test]
    fn scene_jumps_are_edges_and_scenes_do_not_fall_into_each_other() {
        let mut story = Story::new("t");
        let label = node(NodeKind::Label { name: "mid".into() });
        let label_id = label.id;
        let b = Scene {
            id: SceneId::new(),
            name: "b".into(),
            nodes: vec![node(NodeKind::End), label, node(NodeKind::End)],
        };
        let to_start = node(NodeKind::JumpScene {
            scene: b.id,
            label: None,
        });
        let to_label = node(NodeKind::If {
            condition: Expr::Lit(Value::Bool(true)),
            then_target: label_id,
            else_target: None,
        });
        let c = Scene {
            id: SceneId::new(),
            name: "c".into(),
            nodes: vec![node(NodeKind::JumpScene {
                scene: b.id,
                label: Some(label_id),
            })],
        };
        let a = Scene {
            id: SceneId::new(),
            name: "a".into(),
            nodes: vec![to_label, to_start],
        };
        let (a_id, b_id, c_id) = (a.id, b.id, c.id);
        story.scenes = vec![a, b, c];
        let model = GraphModel::build(&story);
        let head_a = model.block(BlockId::Scene(a_id)).unwrap();
        let kinds: Vec<_> = head_a.slots.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [SlotKind::Then, SlotKind::Else, SlotKind::JumpScene]);
        assert_eq!(
            head_a.slots[2].target,
            SlotTarget::Block(BlockId::Scene(b_id))
        );
        assert!(
            head_a.slots[2].label.contains("« b »"),
            "{}",
            head_a.slots[2].label
        );
        assert_eq!(head_a.terminator.map(|t| t.1), Some(Terminator::JumpScene));
        // The scene jump with a label lands on the label's block.
        let head_c = model.block(BlockId::Scene(c_id)).unwrap();
        assert_eq!(
            head_c.slots[0].target,
            SlotTarget::Block(BlockId::Label(label_id))
        );
        assert!(head_c.slots[0].label.contains("mid"));
        // Nothing falls from scene to scene: `a`'s end is a jump, `b`'s head
        // and label blocks end with `End`.
        assert!(model.blocks.iter().all(|b| !b.falls_off_scene()));
        assert!(model.unreachable.contains(&BlockId::Scene(c_id)));
        assert!(!model.unreachable.contains(&BlockId::Label(label_id)));
    }

    #[test]
    fn empty_scene_is_a_block_that_ends_the_game() {
        let mut story = Story::new("t");
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "vide".into(),
            nodes: Vec::new(),
        });
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "b".into(),
            nodes: vec![node(NodeKind::End)],
        });
        let model = GraphModel::build(&story);
        assert_eq!(model.blocks.len(), 2);
        // Scenes do not fall into each other: the empty one ends the game.
        assert_eq!(model.blocks[0].slots[0].kind, SlotKind::Next);
        assert_eq!(model.blocks[0].slots[0].target, SlotTarget::None);
        assert!(model.blocks[0].falls_off_scene());
        assert!(model.unreachable.contains(&model.blocks[1].id));
        assert!(GraphModel::build(&Story::new("e")).blocks.is_empty());
    }

    #[test]
    fn block_keys_round_trip() {
        let a = BlockId::Scene(SceneId::new());
        let b = BlockId::Label(NodeId::new());
        assert_eq!(BlockId::from_key(&a.key()), Some(a));
        assert_eq!(BlockId::from_key(&b.key()), Some(b));
        assert_eq!(BlockId::from_key("x:1"), None);
        assert_eq!(BlockId::from_key("garbage"), None);
    }
}
