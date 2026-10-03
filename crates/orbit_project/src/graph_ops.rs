//! Edits of the story driven by the graph: create blocks, link them, create
//! choices and conditions, redirect or cut links, rename, delete.
//!
//! Like [`crate::ops`], each function is one undo entry (a [`Project::group`])
//! and all-or-nothing, built from the commands of the project: the graph
//! panel, tests and future MCP tools share these rules.
//!
//! Linking rules, given that a link is stored in a node of the IR:
//! * dragging from the fall-through (`suite`) of a block appends a `Jump` to
//!   a block of the same scene, or a `JumpScene` to a block of another one
//!   (scenes never continue into each other);
//! * dragging from an existing jump / scene jump / option / branch / call
//!   redirects it;
//! * "cutting" a link removes what carries it (the jump, the option, the
//!   `else` branch...), since an IR target is never empty;
//! * targeting the head of a scene that does not start with a label inserts
//!   one (the block keeps its identity).

use std::collections::HashSet;

use orbit_core::{ChoiceOption, Expr, Node, NodeId, NodeKind, SceneId, Story, StringId, Value};

use crate::command::Command;
use crate::graph::{BlockId, GraphModel, SlotKind, SlotRef, Terminator};
use crate::ops::{self, NodeDraft};
use crate::project::{Project, ProjectError};

/// A refusal with a stable `code` and the French `message` (see
/// [`crate::Rejection`]).
fn reject(code: &'static str, message: impl Into<String>) -> ProjectError {
    reject_with(code, &[], message)
}

fn reject_with(
    code: &'static str,
    args: &[(&'static str, String)],
    message: impl Into<String>,
) -> ProjectError {
    ProjectError::Rejected(crate::Rejection {
        code,
        args: args.to_vec(),
        message: message.into(),
    })
}

/// Where a new link of a choice or a condition leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChoiceTarget {
    Block(BlockId),
    /// A new scene (starting with a label) with this name.
    NewScene(String),
}

fn block_missing() -> ProjectError {
    reject("block_missing", "Ce bloc n'existe plus.")
}

/// Index just after the last node of a block.
fn append_point(
    story: &Story,
    model: &GraphModel,
    block: BlockId,
) -> Result<(SceneId, usize), ProjectError> {
    let b = model.block(block).ok_or_else(block_missing)?;
    match b.nodes.last() {
        Some(&last) => {
            let (si, ni) = story.locate(last).ok_or_else(block_missing)?;
            Ok((story.scenes[si].id, ni + 1))
        }
        None => Ok((b.scene, 0)),
    }
}

/// Refuses to add something after a node that already ends the block.
fn require_open(model: &GraphModel, block: BlockId) -> Result<(), ProjectError> {
    let b = model.block(block).ok_or_else(block_missing)?;
    match b.terminator {
        Some((_, t)) => Err(reject_with(
            "block_already_ends",
            &[("terminator", t.code().to_owned())],
            format!(
                "Ce bloc se termine déjà par {} : supprimez-le d'abord.",
                t.label()
            ),
        )),
        None => Ok(()),
    }
}

fn label_names(story: &Story) -> Vec<String> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Label { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// The label node that a link to `block` must target, creating it at the
/// start of the scene when the head of the scene has none.
fn ensure_label(p: &mut Project, block: BlockId) -> Result<NodeId, ProjectError> {
    let story = p.story();
    match block {
        BlockId::Label(id) => match story.node(id).map(|n| &n.kind) {
            Some(NodeKind::Label { .. }) => Ok(id),
            _ => Err(block_missing()),
        },
        BlockId::Scene(sid) => {
            let scene = story.scene(sid).ok_or_else(block_missing)?;
            if let Some(Node {
                id,
                kind: NodeKind::Label { .. },
            }) = scene.nodes.first()
            {
                return Ok(*id);
            }
            let names = label_names(story);
            let name = ops::unique_name(&scene.name, names.iter().map(String::as_str));
            let id = NodeId::new();
            p.execute(Command::InsertNode {
                scene: sid,
                index: 0,
                node: Node {
                    id,
                    kind: NodeKind::Label { name },
                },
            })?;
            Ok(id)
        }
    }
}

/// Creates a scene named like `name` (made unique) that starts with a label
/// of the same name. It has no end of its own: reaching the end of a scene
/// ends the game (the graph shows it, and validation warns), which is what an
/// unfinished scene is.
fn new_scene(p: &mut Project, name: &str) -> Result<BlockId, ProjectError> {
    let story = p.story();
    let name = ops::unique_name(name, story.scenes.iter().map(|s| s.name.as_str()));
    let label_name = {
        let names = label_names(story);
        ops::unique_name(&name, names.iter().map(String::as_str))
    };
    let scene = ops::add_scene(p, &name)?;
    p.execute(Command::InsertNode {
        scene,
        index: 0,
        node: Node {
            id: NodeId::new(),
            kind: NodeKind::Label { name: label_name },
        },
    })?;
    Ok(BlockId::Scene(scene))
}

fn resolve(
    p: &mut Project,
    target: &ChoiceTarget,
    created: &mut Vec<BlockId>,
) -> Result<NodeId, ProjectError> {
    match target {
        ChoiceTarget::Block(b) => ensure_label(p, *b),
        ChoiceTarget::NewScene(name) => {
            let block = new_scene(p, name)?;
            created.push(block);
            ensure_label(p, block)
        }
    }
}

/// Replaces the content of a node through `f`.
fn update_kind(
    p: &mut Project,
    id: NodeId,
    f: impl FnOnce(&mut NodeKind) -> Result<(), ProjectError>,
) -> Result<(), ProjectError> {
    let mut kind = p
        .story()
        .node(id)
        .map(|n| n.kind.clone())
        .ok_or_else(block_missing)?;
    f(&mut kind)?;
    p.execute(Command::SetNodeKind {
        id,
        kind,
        expected_version: None,
    })
}

// ---------------------------------------------------------------- creation

/// Creates a scene made of a label, not linked to anything. Returns its
/// block.
pub fn create_scene(p: &mut Project, name: &str) -> Result<BlockId, ProjectError> {
    p.group("Créer une scène", |p| new_scene(p, name))
}

/// Creates a label (a new block) at the end of `scene`. Note that the block
/// before it, when it has no terminator, now falls through into it.
pub fn create_label(p: &mut Project, scene: SceneId, name: &str) -> Result<BlockId, ProjectError> {
    p.group("Créer une étiquette", |p| {
        let story = p.story();
        let sc = story.scene(scene).ok_or_else(block_missing)?;
        let names = label_names(story);
        let name = ops::unique_name(name.trim(), names.iter().map(String::as_str));
        let id = NodeId::new();
        let index = sc.nodes.len();
        p.execute(Command::InsertNode {
            scene,
            index,
            node: Node {
                id,
                kind: NodeKind::Label { name },
            },
        })?;
        Ok(BlockId::Label(id))
    })
}

/// Where a scene jump to `target` lands: its scene, and its label unless it
/// is the head of the scene (the jump then goes to the start).
fn scene_landing(
    model: &GraphModel,
    target: BlockId,
) -> Result<(SceneId, Option<NodeId>), ProjectError> {
    let b = model.block(target).ok_or_else(block_missing)?;
    Ok((
        b.scene,
        match target {
            BlockId::Scene(_) => None,
            BlockId::Label(id) => Some(id),
        },
    ))
}

/// Appends a link to `target` at the end of `block` (which must not already
/// end with a jump, a choice, an end or a return): a `Jump` to a label when
/// both are in the same scene, a `JumpScene` otherwise.
pub fn add_link(p: &mut Project, block: BlockId, target: BlockId) -> Result<(), ProjectError> {
    p.group("Relier les blocs", |p| {
        let model = GraphModel::build(p.story());
        require_open(&model, block)?;
        let same_scene = match (model.block(block), model.block(target)) {
            (Some(a), Some(b)) => a.scene == b.scene,
            _ => return Err(block_missing()),
        };
        let kind = if same_scene {
            NodeKind::Jump {
                target: ensure_label(p, target)?,
            }
        } else {
            let (scene, label) = scene_landing(&model, target)?;
            NodeKind::JumpScene { scene, label }
        };
        let model = GraphModel::build(p.story());
        let (scene, index) = append_point(p.story(), &model, block)?;
        ops::insert_drafts(
            p,
            scene,
            index,
            vec![NodeDraft::plain(kind)],
            "Relier les blocs",
        )?;
        Ok(())
    })
}

/// Appends a choice to `block`. Each option has a text and a target; the
/// blocks that had to be created (new scenes) are returned.
pub fn add_choice(
    p: &mut Project,
    block: BlockId,
    options: Vec<(String, ChoiceTarget)>,
) -> Result<Vec<BlockId>, ProjectError> {
    if options.is_empty() {
        return Err(reject(
            "choice_needs_option",
            "Un choix a besoin d'au moins une option.",
        ));
    }
    p.group("Ajouter un choix", |p| {
        require_open(&GraphModel::build(p.story()), block)?;
        let mut created = Vec::new();
        let mut kinds = Vec::new();
        let mut strings: Vec<(StringId, String)> = Vec::new();
        for (text, target) in &options {
            let label = resolve(p, target, &mut created)?;
            let text_id = StringId::new();
            strings.push((text_id, text.clone()));
            kinds.push(ChoiceOption {
                text_id,
                target: label,
                condition: None,
            });
        }
        let model = GraphModel::build(p.story());
        let (scene, index) = append_point(p.story(), &model, block)?;
        let draft = NodeDraft {
            node: Node {
                id: NodeId::new(),
                kind: NodeKind::Choice { options: kinds },
            },
            strings,
        };
        ops::insert_drafts(p, scene, index, vec![draft], "Ajouter un choix")?;
        Ok(created)
    })
}

/// Appends a condition (`true` until edited) to `block`: `then` and an
/// optional `else` branch (without one, execution continues).
pub fn add_condition(
    p: &mut Project,
    block: BlockId,
    then_target: ChoiceTarget,
    else_target: Option<ChoiceTarget>,
) -> Result<Vec<BlockId>, ProjectError> {
    p.group("Ajouter une condition", |p| {
        require_open(&GraphModel::build(p.story()), block)?;
        let mut created = Vec::new();
        let then_label = resolve(p, &then_target, &mut created)?;
        let else_label = match &else_target {
            Some(t) => Some(resolve(p, t, &mut created)?),
            None => None,
        };
        let model = GraphModel::build(p.story());
        let (scene, index) = append_point(p.story(), &model, block)?;
        ops::insert_drafts(
            p,
            scene,
            index,
            vec![NodeDraft::plain(NodeKind::If {
                condition: Expr::Lit(Value::Bool(true)),
                then_target: then_label,
                else_target: else_label,
            })],
            "Ajouter une condition",
        )?;
        Ok(created)
    })
}

/// Ends the block with an `End` node.
pub fn add_end(p: &mut Project, block: BlockId) -> Result<(), ProjectError> {
    p.group("Terminer le bloc", |p| {
        require_open(&GraphModel::build(p.story()), block)?;
        let model = GraphModel::build(p.story());
        let (scene, index) = append_point(p.story(), &model, block)?;
        ops::insert_drafts(
            p,
            scene,
            index,
            vec![NodeDraft::plain(NodeKind::End)],
            "Terminer le bloc",
        )?;
        Ok(())
    })
}

/// Removes the node that ends a block (jump, choice, end, return) so that
/// it falls through again. Returns what it was.
pub fn remove_terminator(p: &mut Project, block: BlockId) -> Result<Terminator, ProjectError> {
    let model = GraphModel::build(p.story());
    let (node, kind) = model
        .block(block)
        .ok_or_else(block_missing)?
        .terminator
        .ok_or_else(|| reject("block_nothing_to_remove", "Ce bloc n'a rien à retirer."))?;
    p.group("Retirer la fin du bloc", |p| {
        ops::delete_nodes(p, &[node])?;
        Ok(kind)
    })
}

// ------------------------------------------------------------------- links

fn slot_node(story: &Story, slot: SlotRef) -> Result<NodeId, ProjectError> {
    let id = slot.node.ok_or_else(|| {
        reject(
            "block_next_link",
            "La suite d'un bloc se relie en tirant un lien, ou se retire en le terminant.",
        )
    })?;
    story.node(id).map(|n| n.id).ok_or_else(block_missing)
}

/// Makes `slot` lead to `target`: appends a jump for the fall-through,
/// redirects a jump / call / option / branch otherwise.
pub fn set_slot_target(
    p: &mut Project,
    slot: SlotRef,
    target: BlockId,
) -> Result<(), ProjectError> {
    if slot.kind == SlotKind::Next {
        return add_link(p, slot.block, target);
    }
    p.group("Rediriger le lien", |p| {
        let node = slot_node(p.story(), slot)?;
        if slot.kind == SlotKind::JumpScene {
            let (scene, label) = scene_landing(&GraphModel::build(p.story()), target)?;
            return update_kind(p, node, |kind| match kind {
                NodeKind::JumpScene { scene: s, label: l } => {
                    *s = scene;
                    *l = label;
                    Ok(())
                }
                _ => Err(reject("link_stale", "Ce lien ne correspond plus au nœud.")),
            });
        }
        let label = ensure_label(p, target)?;
        update_kind(p, node, |kind| match (kind, slot.kind) {
            (NodeKind::Jump { target }, SlotKind::Jump) => {
                *target = label;
                Ok(())
            }
            (NodeKind::Call { label: l }, SlotKind::Call) => {
                *l = label;
                Ok(())
            }
            (NodeKind::Choice { options }, SlotKind::Option(i)) => {
                options
                    .get_mut(i)
                    .ok_or_else(|| reject("option_missing", "Cette option n'existe plus."))?
                    .target = label;
                Ok(())
            }
            (NodeKind::If { then_target, .. }, SlotKind::Then) => {
                *then_target = label;
                Ok(())
            }
            (NodeKind::If { else_target, .. }, SlotKind::Else) => {
                *else_target = Some(label);
                Ok(())
            }
            (NodeKind::Script { branches, .. }, SlotKind::Branch(i)) => {
                branches
                    .get_mut(i)
                    .ok_or_else(|| reject("branch_missing", "Cette branche n'existe plus."))?
                    .target = label;
                Ok(())
            }
            (NodeKind::Script { on_error, .. }, SlotKind::OnError) => {
                *on_error = Some(label);
                Ok(())
            }
            _ => Err(reject("link_stale", "Ce lien ne correspond plus au nœud.")),
        })
    })
}

/// Removes what carries a link (see the module docs).
fn sever(p: &mut Project, node: NodeId, kind: SlotKind) -> Result<(), ProjectError> {
    match kind {
        SlotKind::Next => Err(reject(
            "block_next_cannot_cut",
            "La suite d'un bloc ne se coupe pas : terminez-le.",
        )),
        SlotKind::Jump | SlotKind::JumpScene | SlotKind::Call | SlotKind::Then => {
            ops::delete_nodes(p, &[node])
        }
        SlotKind::OnError => update_kind(p, node, |k| match k {
            NodeKind::Script { on_error, .. } => {
                *on_error = None;
                Ok(())
            }
            _ => Err(reject("link_stale", "Ce lien ne correspond plus au nœud.")),
        }),
        SlotKind::Branch(i) => update_kind(p, node, |k| match k {
            NodeKind::Script { branches, .. } if i < branches.len() => {
                branches.remove(i);
                Ok(())
            }
            _ => Err(reject("link_stale", "Ce lien ne correspond plus au nœud.")),
        }),
        SlotKind::Else => update_kind(p, node, |k| match k {
            NodeKind::If { else_target, .. } => {
                *else_target = None;
                Ok(())
            }
            _ => Err(reject("link_stale", "Ce lien ne correspond plus au nœud.")),
        }),
        SlotKind::Option(i) => {
            let (len, text) = match p.story().node(node).map(|n| &n.kind) {
                Some(NodeKind::Choice { options }) => {
                    (options.len(), options.get(i).map(|o| o.text_id))
                }
                _ => return Err(reject("link_stale", "Ce lien ne correspond plus au nœud.")),
            };
            let text =
                text.ok_or_else(|| reject("option_missing", "Cette option n'existe plus."))?;
            if len == 1 {
                return ops::delete_nodes(p, &[node]);
            }
            update_kind(p, node, |k| {
                if let NodeKind::Choice { options } = k {
                    options.remove(i);
                }
                Ok(())
            })?;
            p.execute(Command::SetString {
                id: text,
                text: None,
            })
        }
    }
}

/// Cuts a link: removes the jump, the call, the condition, the option (the
/// whole choice when it is the last one) or the `else` branch.
pub fn remove_slot(p: &mut Project, slot: SlotRef) -> Result<(), ProjectError> {
    p.group("Supprimer le lien", |p| {
        let node = slot_node(p.story(), slot)?;
        sever(p, node, slot.kind)
    })
}

/// Text (and id) of the `i`-th option of a choice.
pub fn option_text(story: &Story, node: NodeId, i: usize) -> Option<(StringId, &str)> {
    match story.node(node).map(|n| &n.kind) {
        Some(NodeKind::Choice { options }) => {
            let id = options.get(i)?.text_id;
            Some((id, story.text(id).unwrap_or("")))
        }
        _ => None,
    }
}

/// Adds an option to an existing choice.
pub fn add_option(
    p: &mut Project,
    choice: NodeId,
    text: &str,
    target: ChoiceTarget,
) -> Result<Vec<BlockId>, ProjectError> {
    p.group("Ajouter une option", |p| {
        let mut created = Vec::new();
        let label = resolve(p, &target, &mut created)?;
        let text_id = StringId::new();
        p.execute(Command::SetString {
            id: text_id,
            text: Some(text.to_owned()),
        })?;
        update_kind(p, choice, |k| match k {
            NodeKind::Choice { options } => {
                options.push(ChoiceOption {
                    text_id,
                    target: label,
                    condition: None,
                });
                Ok(())
            }
            _ => Err(reject("not_a_choice", "Ce nœud n'est pas un choix.")),
        })?;
        Ok(created)
    })
}

// ----------------------------------------------------------------- renames

/// Renames a block: the scene for a head, the label otherwise.
pub fn rename_block(p: &mut Project, block: BlockId, name: &str) -> Result<(), ProjectError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(reject("name_empty", "Le nom ne peut pas être vide."));
    }
    match block {
        BlockId::Scene(id) => p.execute(Command::RenameScene {
            id,
            name: name.to_owned(),
        }),
        BlockId::Label(id) => p.group("Renommer l'étiquette", |p| {
            update_kind(p, id, |k| match k {
                NodeKind::Label { name: n } => {
                    *n = name.to_owned();
                    Ok(())
                }
                _ => Err(block_missing()),
            })
        }),
    }
}

// ----------------------------------------------------------------- deletion

/// What deleting a block does, to ask for confirmation first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletePlan {
    /// The whole scene goes (the block is the head of its scene).
    pub scene: bool,
    /// The scene the block belongs to.
    pub scene_id: SceneId,
    pub scene_name: String,
    /// Nodes removed.
    pub nodes: Vec<NodeId>,
    /// Links from elsewhere that are cut first: `(node, slot kind)`.
    pub incoming: Vec<(NodeId, SlotKind)>,
}

/// Links of nodes outside `inside` that target a node of `inside`, or jump
/// to the scene being deleted (`scene`).
fn references(
    story: &Story,
    inside: &HashSet<NodeId>,
    scene: Option<SceneId>,
) -> Vec<(NodeId, SlotKind)> {
    let mut out = Vec::new();
    for node in story.scenes.iter().flat_map(|s| &s.nodes) {
        if inside.contains(&node.id) {
            continue;
        }
        match &node.kind {
            NodeKind::Jump { target } if inside.contains(target) => {
                out.push((node.id, SlotKind::Jump));
            }
            NodeKind::JumpScene { scene: s, label }
                if scene == Some(*s) || label.is_some_and(|l| inside.contains(&l)) =>
            {
                out.push((node.id, SlotKind::JumpScene));
            }
            NodeKind::Call { label } if inside.contains(label) => {
                out.push((node.id, SlotKind::Call));
            }
            NodeKind::If {
                then_target,
                else_target,
                ..
            } => {
                if inside.contains(then_target) {
                    out.push((node.id, SlotKind::Then));
                }
                if else_target.is_some_and(|t| inside.contains(&t)) {
                    out.push((node.id, SlotKind::Else));
                }
            }
            NodeKind::Script {
                branches, on_error, ..
            } => {
                for (i, b) in branches.iter().enumerate().rev() {
                    if inside.contains(&b.target) {
                        out.push((node.id, SlotKind::Branch(i)));
                    }
                }
                if on_error.is_some_and(|t| inside.contains(&t)) {
                    out.push((node.id, SlotKind::OnError));
                }
            }
            NodeKind::Choice { options } => {
                for (i, o) in options.iter().enumerate().rev() {
                    if inside.contains(&o.target) {
                        out.push((node.id, SlotKind::Option(i)));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// What [`delete_block`] would remove and cut.
pub fn delete_plan(story: &Story, block: BlockId) -> Result<DeletePlan, ProjectError> {
    let model = GraphModel::build(story);
    let b = model.block(block).ok_or_else(block_missing)?;
    let (scene, nodes): (bool, Vec<NodeId>) = if b.is_head {
        let sc = story.scene(b.scene).ok_or_else(block_missing)?;
        (true, sc.nodes.iter().map(|n| n.id).collect())
    } else {
        (false, b.nodes.clone())
    };
    let inside: HashSet<NodeId> = nodes.iter().copied().collect();
    Ok(DeletePlan {
        scene,
        scene_id: b.scene,
        scene_name: b.scene_name.clone(),
        incoming: references(story, &inside, scene.then_some(b.scene)),
        nodes,
    })
}

/// Deletes a block: the whole scene for a head, the label and what follows
/// it otherwise; links from other blocks to it are cut first. One undo entry.
pub fn delete_block(p: &mut Project, block: BlockId) -> Result<(), ProjectError> {
    let plan = delete_plan(p.story(), block)?;
    let label = if plan.scene {
        "Supprimer la scène"
    } else {
        "Supprimer le bloc"
    };
    p.group(label, |p| {
        let inside: HashSet<NodeId> = plan.nodes.iter().copied().collect();
        // Cut the incoming links one at a time: cutting an option may
        // delete its choice, which invalidates the other addresses.
        for _ in 0..=plan.incoming.len() + inside.len() {
            let removed_scene = plan.scene.then_some(plan.scene_id);
            let Some(&(node, kind)) = references(p.story(), &inside, removed_scene).first() else {
                break;
            };
            sever(p, node, kind)?;
        }
        if plan.scene {
            let sid = match block {
                BlockId::Scene(id) => id,
                BlockId::Label(_) => return Err(block_missing()),
            };
            let strings: Vec<StringId> = p
                .story()
                .scene(sid)
                .map(|s| s.nodes.iter().flat_map(|n| n.kind.strings()).collect())
                .unwrap_or_default();
            p.execute(Command::RemoveScene { id: sid })?;
            for id in strings {
                p.execute(Command::SetString { id, text: None })?;
            }
            Ok(())
        } else {
            ops::delete_nodes(p, &plan.nodes)
        }
    })
}

#[cfg(test)]
mod tests {
    use orbit_core::{Scene, validate};

    use super::*;
    use crate::graph::SlotTarget;

    fn start() -> (Project, BlockId) {
        let mut story = Story::new("t");
        let text = story.strings.add("Salut");
        let scene = SceneId::new();
        story.scenes.push(Scene {
            id: scene,
            name: "début".into(),
            nodes: vec![Node {
                id: NodeId::new(),
                kind: NodeKind::Say {
                    speaker: None,
                    text_id: text,
                },
            }],
        });
        (Project::new(story), BlockId::Scene(scene))
    }

    /// Runs `op`, checks it is one undo entry that undoes and redoes
    /// exactly, and that the story stays valid.
    fn one_entry<R>(
        p: &mut Project,
        op: impl FnOnce(&mut Project) -> Result<R, ProjectError>,
    ) -> R {
        let before = p.story().clone();
        let out = op(p).expect("operation");
        let after = p.story().clone();
        assert_ne!(before, after);
        assert!(p.undo().unwrap());
        assert_eq!(p.story(), &before, "one undo restores the story");
        assert!(p.redo().unwrap());
        assert_eq!(p.story(), &after);
        let report = validate(p.story());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        out
    }

    fn targets(p: &Project, block: BlockId) -> Vec<SlotTarget> {
        GraphModel::build(p.story())
            .block(block)
            .unwrap()
            .slots
            .iter()
            .map(|s| s.target.clone())
            .collect()
    }

    #[test]
    fn choice_with_two_new_scenes_is_valid() {
        let (mut p, head) = start();
        let created = one_entry(&mut p, |p| {
            add_choice(
                p,
                head,
                vec![
                    (
                        "Aller à gauche".into(),
                        ChoiceTarget::NewScene("Gauche".into()),
                    ),
                    (
                        "Aller à droite".into(),
                        ChoiceTarget::NewScene("Droite".into()),
                    ),
                ],
            )
        });
        assert_eq!(created.len(), 2);
        let model = GraphModel::build(p.story());
        let head_block = model.block(head).unwrap();
        assert_eq!(head_block.slots.len(), 2);
        assert_eq!(head_block.slots[0].label, "Aller à gauche");
        assert_eq!(
            targets(&p, head),
            vec![SlotTarget::Block(created[0]), SlotTarget::Block(created[1])]
        );
        assert!(model.unreachable.is_empty());
        // The choice ends the block: a second terminator is refused.
        let err = add_end(&mut p, head).unwrap_err();
        assert!(err.to_string().contains("un choix"), "{err}");
        assert!(add_link(&mut p, head, created[0]).is_err());
    }

    #[test]
    fn links_between_scenes_are_scene_jumps_and_can_be_redirected_and_cut() {
        let (mut p, head) = start();
        let scene_b = one_entry(&mut p, |p| create_scene(p, "B"));
        let scene_c = one_entry(&mut p, |p| create_scene(p, "C"));
        // `suite` -> B, another scene: a scene jump.
        one_entry(&mut p, |p| {
            set_slot_target(
                p,
                SlotRef {
                    block: head,
                    node: None,
                    kind: SlotKind::Next,
                },
                scene_b,
            )
        });
        let model = GraphModel::build(p.story());
        let slot = model.block(head).unwrap().slots[0].clone();
        assert_eq!(slot.kind, SlotKind::JumpScene);
        assert_eq!(slot.target, SlotTarget::Block(scene_b));
        assert!(slot.label.contains('B'), "{}", slot.label);
        assert_eq!(
            model.block(head).unwrap().terminator.map(|t| t.1),
            Some(Terminator::JumpScene)
        );
        let jump = SlotRef {
            block: head,
            node: slot.node,
            kind: SlotKind::JumpScene,
        };
        // Redirect it to C.
        one_entry(&mut p, |p| set_slot_target(p, jump, scene_c));
        assert_eq!(targets(&p, head), vec![SlotTarget::Block(scene_c)]);
        // Cut it: the block ends the game again (no next block in the scene).
        one_entry(&mut p, |p| remove_slot(p, jump));
        let model = GraphModel::build(p.story());
        let block = model.block(head).unwrap();
        assert_eq!(block.slots[0].kind, SlotKind::Next);
        assert!(block.falls_off_scene());
    }

    #[test]
    fn links_inside_a_scene_are_jumps_to_labels() {
        let (mut p, head) = start();
        let BlockId::Scene(sid) = head else {
            unreachable!()
        };
        let label = one_entry(&mut p, |p| create_label(p, sid, "suite"));
        assert!(matches!(label, BlockId::Label(_)));
        // The head falls into the label; `suite` links back to the head: the
        // head has no label, so one is created at the start of the scene.
        one_entry(&mut p, |p| add_link(p, label, head));
        assert!(matches!(
            p.story().scene(sid).unwrap().nodes[0].kind,
            NodeKind::Label { .. }
        ));
        let model = GraphModel::build(p.story());
        assert_eq!(model.block(label).unwrap().slots[0].kind, SlotKind::Jump);
        // The head keeps its identity.
        assert!(model.block(head).is_some());
    }

    #[test]
    fn a_scene_jump_can_land_on_a_label_and_deleting_it_cuts_the_jump() {
        let (mut p, head) = start();
        let other = one_entry(&mut p, |p| create_scene(p, "Autre"));
        let BlockId::Scene(osid) = other else {
            unreachable!()
        };
        let inner = one_entry(&mut p, |p| create_label(p, osid, "milieu"));
        one_entry(&mut p, |p| add_link(p, head, inner));
        let model = GraphModel::build(p.story());
        let slot = &model.block(head).unwrap().slots[0];
        assert_eq!(slot.kind, SlotKind::JumpScene);
        assert_eq!(slot.target, SlotTarget::Block(inner));
        assert!(slot.label.contains("milieu"), "{}", slot.label);
        // The label cannot be deleted without cutting the jump: the plan says so.
        let plan = delete_plan(p.story(), inner).unwrap();
        assert_eq!(plan.incoming.len(), 1);
        one_entry(&mut p, |p| delete_block(p, inner));
        assert!(
            GraphModel::build(p.story())
                .block(head)
                .unwrap()
                .slots
                .iter()
                .all(|s| s.kind == SlotKind::Next)
        );
        // Deleting the target scene cuts a jump to its start too.
        one_entry(&mut p, |p| add_link(p, head, other));
        let plan = delete_plan(p.story(), other).unwrap();
        assert!(plan.scene && plan.incoming.len() == 1);
        one_entry(&mut p, |p| delete_block(p, other));
        assert!(p.story().scene(osid).is_none());
    }

    #[test]
    fn options_conditions_and_cuts() {
        let (mut p, head) = start();
        let a = one_entry(&mut p, |p| create_scene(p, "A"));
        let b = one_entry(&mut p, |p| create_scene(p, "B"));
        one_entry(&mut p, |p| {
            add_choice(
                p,
                head,
                vec![
                    ("un".into(), ChoiceTarget::Block(a)),
                    ("deux".into(), ChoiceTarget::Block(b)),
                ],
            )
        });
        let model = GraphModel::build(p.story());
        let choice = model.block(head).unwrap().slots[0].node.unwrap();
        // Add a third option, then remove the first.
        one_entry(&mut p, |p| {
            add_option(p, choice, "trois", ChoiceTarget::NewScene("C".into()))
        });
        assert_eq!(model_slots(&p, head), 3);
        one_entry(&mut p, |p| {
            remove_slot(
                p,
                SlotRef {
                    block: head,
                    node: Some(choice),
                    kind: SlotKind::Option(0),
                },
            )
        });
        assert_eq!(model_slots(&p, head), 2);
        assert_eq!(
            GraphModel::build(p.story()).block(head).unwrap().slots[0].label,
            "deux"
        );
        // Texts are editable through their string.
        let (id, text) = option_text(p.story(), choice, 0).unwrap();
        assert_eq!(text, "deux");
        p.execute(Command::SetString {
            id,
            text: Some("2".into()),
        })
        .unwrap();
        // A condition on another block, then its else branch is cut.
        let cond_block = a;
        one_entry(&mut p, |p| {
            add_condition(
                p,
                cond_block,
                ChoiceTarget::Block(b),
                Some(ChoiceTarget::NewScene("D".into())),
            )
        });
        let slots = GraphModel::build(p.story())
            .block(cond_block)
            .unwrap()
            .slots
            .clone();
        assert_eq!(slots[0].kind, SlotKind::Then);
        assert_eq!(slots[1].kind, SlotKind::Else);
        let node = slots[0].node.unwrap();
        one_entry(&mut p, |p| {
            remove_slot(
                p,
                SlotRef {
                    block: cond_block,
                    node: Some(node),
                    kind: SlotKind::Else,
                },
            )
        });
        assert_eq!(
            GraphModel::build(p.story())
                .block(cond_block)
                .unwrap()
                .slots[1]
                .target,
            SlotTarget::None
        );
        // The last option removes the whole choice.
        let head_slots = model_slots(&p, head);
        assert_eq!(head_slots, 2);
    }

    fn model_slots(p: &Project, block: BlockId) -> usize {
        GraphModel::build(p.story())
            .block(block)
            .unwrap()
            .slots
            .len()
    }

    #[test]
    fn script_branches_and_on_error_are_linkable_and_cuttable() {
        let (mut p, head) = start();
        let a = one_entry(&mut p, |p| create_scene(p, "A"));
        let b = one_entry(&mut p, |p| create_scene(p, "B"));
        let label_a = ensure_label(&mut p, a).unwrap();
        let script = NodeId::new();
        let BlockId::Scene(sid) = head else {
            unreachable!()
        };
        p.execute(Command::InsertNode {
            scene: sid,
            index: 1,
            node: Node {
                id: script,
                kind: NodeKind::Script {
                    source: "branch(\"x\")".into(),
                    branches: vec![orbit_core::ScriptBranch {
                        name: "x".into(),
                        target: label_a,
                    }],
                    on_error: None,
                },
            },
        })
        .unwrap();
        let model = GraphModel::build(p.story());
        let slots = &model.block(head).unwrap().slots;
        let kinds: Vec<_> = slots.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            [SlotKind::Branch(0), SlotKind::OnError, SlotKind::Next]
        );
        assert_eq!(slots[0].label, "x");
        assert_eq!(slots[1].target, SlotTarget::None);
        let slot = |kind| SlotRef {
            block: head,
            node: Some(script),
            kind,
        };
        one_entry(&mut p, |p| set_slot_target(p, slot(SlotKind::OnError), b));
        assert!(matches!(targets(&p, head)[1], SlotTarget::Block(_)));
        one_entry(&mut p, |p| set_slot_target(p, slot(SlotKind::Branch(0)), b));
        assert_eq!(targets(&p, head)[0], targets(&p, head)[1]);
        one_entry(&mut p, |p| remove_slot(p, slot(SlotKind::OnError)));
        assert_eq!(targets(&p, head)[1], SlotTarget::None);
        one_entry(&mut p, |p| remove_slot(p, slot(SlotKind::Branch(0))));
        assert_eq!(model_slots(&p, head), 2);
    }

    #[test]
    fn rename_and_terminators() {
        let (mut p, head) = start();
        one_entry(&mut p, |p| rename_block(p, head, "Prologue"));
        assert_eq!(
            GraphModel::build(p.story()).block(head).unwrap().title,
            "Prologue"
        );
        assert!(rename_block(&mut p, head, "  ").is_err());
        let label = one_entry(&mut p, |p| {
            let BlockId::Scene(sid) = head else {
                unreachable!()
            };
            create_label(p, sid, "x")
        });
        one_entry(&mut p, |p| rename_block(p, label, "renommé"));
        assert_eq!(
            GraphModel::build(p.story()).block(label).unwrap().title,
            "renommé"
        );
        one_entry(&mut p, |p| add_end(p, label));
        assert_eq!(
            one_entry(&mut p, |p| remove_terminator(p, label)),
            Terminator::End
        );
    }

    #[test]
    fn delete_block_cuts_incoming_links() {
        let (mut p, head) = start();
        let a = one_entry(&mut p, |p| create_scene(p, "A"));
        let BlockId::Scene(sa) = a else {
            unreachable!()
        };
        let label = one_entry(&mut p, |p| create_label(p, sa, "inside"));
        one_entry(&mut p, |p| {
            add_choice(
                p,
                head,
                vec![
                    ("vers a".into(), ChoiceTarget::Block(a)),
                    ("vers l".into(), ChoiceTarget::Block(label)),
                ],
            )
        });
        let plan = delete_plan(p.story(), label).unwrap();
        assert!(!plan.scene);
        assert_eq!(plan.incoming.len(), 1);
        // Deleting the label block cuts its option, keeps the other one.
        one_entry(&mut p, |p| delete_block(p, label));
        assert_eq!(model_slots(&p, head), 1);
        // Deleting a head deletes its scene and every link to it; the last
        // option takes the choice with it, so the block falls through.
        let plan = delete_plan(p.story(), a).unwrap();
        assert!(plan.scene && plan.incoming.len() == 1);
        one_entry(&mut p, |p| delete_block(p, a));
        assert!(p.story().scene(sa).is_none());
        assert_eq!(
            GraphModel::build(p.story()).block(head).unwrap().slots[0].kind,
            SlotKind::Next
        );
    }
}
