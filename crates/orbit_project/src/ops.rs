//! Compound editing operations built from [`Command`]s: each one is a single
//! undo entry (a labeled transaction) and all-or-nothing. They are what the
//! editor's gestures (insert, delete, drag and drop, copy / paste, renames)
//! and the future MCP tools call, so the rules live in one place.

use std::collections::{BTreeMap, HashMap, HashSet};

use orbit_core::{
    AssetEntry, AssetId, AssetKind, Character, CharacterId, Color, Expr, Node, NodeId, NodeKind,
    Position, Scene, SceneId, Story, StringId, Value, VarDecl,
};

use crate::command::Command;
use crate::project::{Project, ProjectError};

/// A new node together with the source-language strings it displays.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeDraft {
    pub node: Node,
    pub strings: Vec<(StringId, String)>,
}

impl NodeDraft {
    /// A draft without strings.
    pub fn plain(kind: NodeKind) -> Self {
        Self {
            node: Node {
                id: NodeId::new(),
                kind,
            },
            strings: Vec::new(),
        }
    }

    /// A line of dialogue with its text.
    pub fn say(speaker: Option<CharacterId>, text: impl Into<String>) -> Self {
        let text_id = StringId::new();
        Self {
            node: Node {
                id: NodeId::new(),
                kind: NodeKind::Say { speaker, text_id },
            },
            strings: vec![(text_id, text.into())],
        }
    }

    /// A label with a name.
    pub fn label(name: impl Into<String>) -> Self {
        Self::plain(NodeKind::Label { name: name.into() })
    }
}

/// Nodes copied out of a story, for pasting (internal clipboard). Carries the
/// strings the nodes display so that a paste works across projects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeClip {
    pub nodes: Vec<Node>,
    pub strings: BTreeMap<StringId, String>,
}

impl NodeClip {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

fn reject(message: impl Into<String>) -> ProjectError {
    ProjectError::Rejected(message.into())
}

/// Ids in reading order (scene order, then node order), without duplicates;
/// unknown ids are dropped.
pub fn in_story_order(story: &Story, ids: &[NodeId]) -> Vec<NodeId> {
    let wanted: HashSet<NodeId> = ids.iter().copied().collect();
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter(|n| wanted.contains(&n.id))
        .map(|n| n.id)
        .collect()
}

/// Copies nodes (in reading order) and the strings they display.
pub fn copy_nodes(story: &Story, ids: &[NodeId]) -> NodeClip {
    let mut clip = NodeClip::default();
    for id in in_story_order(story, ids) {
        if let Some(node) = story.node(id) {
            for s in node.kind.strings() {
                if let Some(text) = story.text(s) {
                    clip.strings.insert(s, text.to_owned());
                }
            }
            clip.nodes.push(node.clone());
        }
    }
    clip
}

/// Fresh ids for a clip: new node ids, new string ids; targets that point
/// inside the clip follow the new ids, targets outside are kept as they are.
fn instantiate(clip: &NodeClip) -> Vec<NodeDraft> {
    let node_map: HashMap<NodeId, NodeId> =
        clip.nodes.iter().map(|n| (n.id, NodeId::new())).collect();
    let remap = |id: NodeId| node_map.get(&id).copied().unwrap_or(id);
    clip.nodes
        .iter()
        .map(|n| {
            let mut kind = n.kind.clone();
            let mut strings = Vec::new();
            let mut fresh = |old: StringId| {
                let new = StringId::new();
                let text = clip.strings.get(&old).cloned().unwrap_or_default();
                strings.push((new, text));
                new
            };
            match &mut kind {
                NodeKind::Say { text_id, .. } => *text_id = fresh(*text_id),
                NodeKind::Choice { options } => {
                    for o in options {
                        o.text_id = fresh(o.text_id);
                        o.target = remap(o.target);
                    }
                }
                NodeKind::Jump { target } => *target = remap(*target),
                NodeKind::Call { label } => *label = remap(*label),
                NodeKind::If {
                    then_target,
                    else_target,
                    ..
                } => {
                    *then_target = remap(*then_target);
                    *else_target = else_target.map(remap);
                }
                _ => {}
            }
            NodeDraft {
                node: Node {
                    id: remap(n.id),
                    kind,
                },
                strings,
            }
        })
        .collect()
}

/// Inserts drafts (strings first, then nodes) at `index` of `scene` as one
/// undo entry labeled `label`. Returns the new ids in order.
pub fn insert_drafts(
    project: &mut Project,
    scene: SceneId,
    index: usize,
    drafts: Vec<NodeDraft>,
    label: &str,
) -> Result<Vec<NodeId>, ProjectError> {
    let ids = drafts.iter().map(|d| d.node.id).collect();
    project.transaction_labeled(label, |tx| {
        for (i, draft) in drafts.into_iter().enumerate() {
            for (id, text) in draft.strings {
                tx.execute(Command::SetString {
                    id,
                    text: Some(text),
                })?;
            }
            tx.execute(Command::InsertNode {
                scene,
                index: index + i,
                node: draft.node,
            })?;
        }
        Ok(())
    })?;
    Ok(ids)
}

/// Pastes a clip at `index` of `scene` with brand new ids. Returns them.
pub fn paste_nodes(
    project: &mut Project,
    clip: &NodeClip,
    scene: SceneId,
    index: usize,
) -> Result<Vec<NodeId>, ProjectError> {
    if clip.is_empty() {
        return Err(reject("Le presse-papiers est vide."));
    }
    insert_drafts(project, scene, index, instantiate(clip), "Coller")
}

/// Duplicates nodes right after the last one. Returns the new ids.
pub fn duplicate_nodes(project: &mut Project, ids: &[NodeId]) -> Result<Vec<NodeId>, ProjectError> {
    let story = project.story();
    let ordered = in_story_order(story, ids);
    let Some(&last) = ordered.last() else {
        return Err(reject("Rien à dupliquer."));
    };
    let (si, ni) = story
        .locate(last)
        .ok_or_else(|| reject("Nœud introuvable."))?;
    let scene = story.scenes[si].id;
    let clip = copy_nodes(story, &ordered);
    insert_drafts(project, scene, ni + 1, instantiate(&clip), "Dupliquer")
}

/// Deletes nodes and the strings only they displayed. Refused (nothing
/// changes) if a node outside the selection still targets one of them.
pub fn delete_nodes(project: &mut Project, ids: &[NodeId]) -> Result<(), ProjectError> {
    let story = project.story();
    let mut pending = in_story_order(story, ids);
    if pending.is_empty() {
        return Ok(());
    }
    let selected: HashSet<NodeId> = pending.iter().copied().collect();
    for n in story.scenes.iter().flat_map(|s| &s.nodes) {
        if selected.contains(&n.id) {
            continue;
        }
        if let Some(t) = n.kind.targets().into_iter().find(|t| selected.contains(t)) {
            let name = match story.node(t).map(|x| &x.kind) {
                Some(NodeKind::Label { name }) => format!("« {name} »"),
                _ => "ce nœud".to_owned(),
            };
            return Err(reject(format!(
                "Impossible de supprimer {name} : un autre nœud y fait encore référence."
            )));
        }
    }
    // Referencing nodes first, so that a Jump goes before its Label.
    pending.sort_by_key(|id| {
        story
            .node(*id)
            .map_or(1, |n| u8::from(matches!(n.kind, NodeKind::Label { .. })))
    });
    let strings: Vec<(NodeId, Vec<StringId>)> = pending
        .iter()
        .filter_map(|id| story.node(*id).map(|n| (*id, n.kind.strings())))
        .collect();
    let label = if pending.len() == 1 {
        match story.node(pending[0]).map(|n| &n.kind) {
            Some(NodeKind::Say { .. }) => "Supprimer la réplique",
            _ => "Supprimer le nœud",
        }
    } else {
        "Supprimer les nœuds"
    };
    project.transaction_labeled(label, |tx| {
        for (id, texts) in strings {
            tx.execute(Command::RemoveNode {
                id,
                expected_version: None,
            })?;
            for text in texts {
                tx.execute(Command::SetString {
                    id: text,
                    text: None,
                })?;
            }
        }
        Ok(())
    })
}

/// Moves nodes, keeping their relative order, so that the block starts at
/// `index` of `scene` (an index among the nodes of that scene that do not
/// move). One undo entry.
pub fn move_nodes(
    project: &mut Project,
    ids: &[NodeId],
    scene: SceneId,
    index: usize,
) -> Result<(), ProjectError> {
    let story = project.story();
    let ordered = in_story_order(story, ids);
    if ordered.is_empty() {
        return Ok(());
    }
    let target = story
        .scene(scene)
        .ok_or_else(|| reject("Scène introuvable."))?;
    // The node the block goes in front of (`None`: at the end).
    let anchor = target
        .nodes
        .iter()
        .filter(|n| !ordered.contains(&n.id))
        .nth(index)
        .map(|n| n.id);
    let label = if ordered.len() == 1 {
        "Déplacer le nœud"
    } else {
        "Déplacer les nœuds"
    };
    project.transaction_labeled(label, |tx| {
        for id in ordered {
            let story = tx.project().story();
            let sc = story
                .scene(scene)
                .ok_or_else(|| reject("Scène introuvable."))?;
            let own = sc.nodes.iter().position(|n| n.id == id);
            let at = match anchor.and_then(|a| sc.nodes.iter().position(|n| n.id == a)) {
                Some(a) => a - usize::from(own.is_some_and(|o| o < a)),
                None => sc.nodes.len() - usize::from(own.is_some()),
            };
            tx.execute(Command::MoveNode {
                id,
                scene,
                index: at,
            })?;
        }
        Ok(())
    })
}

/// Moves nodes one step up (`-1`) or down (`1`) within their scene. The
/// selection moves as a block; nothing happens at the edges.
pub fn nudge_nodes(
    project: &mut Project,
    ids: &[NodeId],
    direction: i32,
) -> Result<bool, ProjectError> {
    let story = project.story();
    let ordered = in_story_order(story, ids);
    let Some(&first) = ordered.first() else {
        return Ok(false);
    };
    let (si, first_index) = story
        .locate(first)
        .ok_or_else(|| reject("Nœud introuvable."))?;
    let scene = &story.scenes[si];
    // Only a selection inside a single scene can be nudged.
    let positions: Vec<usize> = ordered
        .iter()
        .filter_map(|id| scene.nodes.iter().position(|n| n.id == *id))
        .collect();
    if positions.len() != ordered.len() {
        return Ok(false);
    }
    let selected: HashSet<usize> = positions.iter().copied().collect();
    let remaining_before = scene.nodes.len() - selected.len();
    // Index among the unselected nodes of the block's first element.
    let rank = (0..first_index).filter(|i| !selected.contains(i)).count();
    let target = match direction.signum() {
        -1 if rank > 0 => rank - 1,
        1 if rank < remaining_before => rank + 1,
        _ => return Ok(false),
    };
    let scene_id = scene.id;
    move_nodes(project, &ordered, scene_id, target)?;
    Ok(true)
}

/// Creates an empty scene at the end. Returns its id.
pub fn add_scene(project: &mut Project, name: &str) -> Result<SceneId, ProjectError> {
    let id = SceneId::new();
    let index = project.story().scenes.len();
    project.execute(Command::InsertScene {
        index,
        scene: Scene {
            id,
            name: name.to_owned(),
            nodes: Vec::new(),
        },
    })?;
    Ok(id)
}

/// A free name like `base`, `base 2`, `base 3`...
pub fn unique_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: HashSet<&str> = taken.into_iter().collect();
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|c| !taken.contains(c.as_str()))
        .unwrap_or_else(|| base.to_owned())
}

/// Creates a character (its display name is a new string). Returns its id.
pub fn add_character(
    project: &mut Project,
    name: &str,
    color: Color,
) -> Result<CharacterId, ProjectError> {
    let id = CharacterId::new();
    let name_id = StringId::new();
    let index = project.story().characters.len();
    project.transaction_labeled("Ajouter un personnage", |tx| {
        tx.execute(Command::SetString {
            id: name_id,
            text: Some(name.to_owned()),
        })?;
        tx.execute(Command::InsertCharacter {
            index,
            character: Character {
                id,
                name_id,
                color,
                expressions: Vec::new(),
                default_position: None,
            },
        })
    })?;
    Ok(id)
}

/// Deletes a character and its display name. Refused while a node uses it.
pub fn delete_character(project: &mut Project, id: CharacterId) -> Result<(), ProjectError> {
    let name_id = project
        .story()
        .character(id)
        .map(|c| c.name_id)
        .ok_or_else(|| reject("Personnage introuvable."))?;
    project.transaction_labeled("Supprimer le personnage", |tx| {
        tx.execute(Command::RemoveCharacter { id })?;
        tx.execute(Command::SetString {
            id: name_id,
            text: None,
        })
    })
}

/// Registers an asset file (already inside the project) in the manifest.
pub fn add_asset(
    project: &mut Project,
    kind: AssetKind,
    path: &str,
) -> Result<AssetId, ProjectError> {
    if let Some(existing) = project
        .story()
        .assets
        .assets
        .iter()
        .find(|a| a.path == path)
    {
        return if existing.kind == kind {
            Ok(existing.id)
        } else {
            Err(reject(format!(
                "« {path} » est déjà déclaré d'un autre type."
            )))
        };
    }
    let id = AssetId::new();
    let index = project.story().assets.assets.len();
    project.execute(Command::InsertAsset {
        index,
        entry: AssetEntry {
            id,
            kind,
            path: path.to_owned(),
        },
    })?;
    Ok(id)
}

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !matches!(name, "true" | "false")
}

/// Whether `name` can name a variable (same lexical rules as expressions).
pub fn is_valid_variable_name(name: &str) -> bool {
    valid_identifier(name)
}

/// Renames an expression of the character's sprites; every `Show` of the
/// character that used the old name follows. Refused when the new name is
/// empty or already taken by another expression of the character.
pub fn rename_expression(
    project: &mut Project,
    character: CharacterId,
    old: &str,
    new: &str,
) -> Result<(), ProjectError> {
    let new = new.trim();
    let story = project.story();
    let def = story
        .character(character)
        .ok_or_else(|| reject("Personnage introuvable."))?;
    if new.is_empty() {
        return Err(reject("Le nom d'une expression ne peut pas être vide."));
    }
    if new == old {
        return Ok(());
    }
    if def.expression(new).is_some() {
        return Err(reject(format!(
            "Ce personnage a déjà une expression « {new} »."
        )));
    }
    if def.expression(old).is_none() {
        return Err(reject(format!("Expression « {old} » introuvable.")));
    }
    let mut def = def.clone();
    if let Some(e) = def.expressions.iter_mut().find(|e| e.name == old) {
        e.name = new.to_owned();
    }
    let updates: Vec<(NodeId, NodeKind)> = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Show {
                character: c,
                expression,
                position,
                transition,
            } if *c == character && expression == old => Some((
                n.id,
                NodeKind::Show {
                    character: *c,
                    expression: new.to_owned(),
                    position: position.clone(),
                    transition: *transition,
                },
            )),
            _ => None,
        })
        .collect();
    project.transaction_labeled("Renommer l'expression", |tx| {
        tx.execute(Command::ReplaceCharacter { character: def })?;
        for (id, kind) in updates {
            tx.execute(Command::ReplaceNodeKind {
                id,
                kind,
                expected_version: None,
            })?;
        }
        Ok(())
    })
}

/// Names of the expressions of `character` used by a `Show` node.
pub fn expression_in_use(story: &Story, character: CharacterId, name: &str) -> bool {
    story.scenes.iter().flat_map(|s| &s.nodes).any(|n| {
        matches!(&n.kind, NodeKind::Show { character: c, expression, .. }
            if *c == character && expression == name)
    })
}

fn rename_in_expr(expr: &Expr, old: &str, new: &str) -> Expr {
    match expr {
        Expr::Var(v) if v == old => Expr::Var(new.to_owned()),
        Expr::Unary(op, e) => Expr::Unary(*op, Box::new(rename_in_expr(e, old, new))),
        Expr::Binary(op, l, r) => Expr::Binary(
            *op,
            Box::new(rename_in_expr(l, old, new)),
            Box::new(rename_in_expr(r, old, new)),
        ),
        other => other.clone(),
    }
}

/// Nodes that read or assign variable `name`.
pub fn variable_users(story: &Story, name: &str) -> Vec<NodeId> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter(|n| match &n.kind {
            NodeKind::SetVar { var, value } => var == name || value.variables().contains(&name),
            NodeKind::If { condition, .. } => condition.variables().contains(&name),
            NodeKind::Choice { options } => options.iter().any(|o| {
                o.condition
                    .as_ref()
                    .is_some_and(|c| c.variables().contains(&name))
            }),
            _ => false,
        })
        .map(|n| n.id)
        .collect()
}

/// Declares a variable.
pub fn add_variable(project: &mut Project, decl: VarDecl) -> Result<(), ProjectError> {
    if !valid_identifier(&decl.name) {
        return Err(reject(format!(
            "« {} » n'est pas un nom de variable valide (lettres, chiffres et _).",
            decl.name
        )));
    }
    let mut variables = project.story().variables.clone();
    if variables.iter().any(|v| v.name == decl.name) {
        return Err(reject(format!(
            "La variable « {} » existe déjà.",
            decl.name
        )));
    }
    variables.push(decl);
    project.execute(Command::SetVariables { variables })
}

/// Replaces a variable declaration (type, initial value, scope).
pub fn update_variable(
    project: &mut Project,
    name: &str,
    initial: Value,
    scope: orbit_core::VarScope,
) -> Result<(), ProjectError> {
    let mut variables = project.story().variables.clone();
    let slot = variables
        .iter_mut()
        .find(|v| v.name == name)
        .ok_or_else(|| reject(format!("Variable « {name} » introuvable.")))?;
    slot.initial = initial;
    slot.scope = scope;
    project.execute(Command::SetVariables { variables })
}

/// Renames a variable and every usage (assignments, conditions, choice
/// conditions) as one undo entry. Refused if the new name is invalid or
/// already declared.
pub fn rename_variable(project: &mut Project, old: &str, new: &str) -> Result<(), ProjectError> {
    let new = new.trim();
    if new == old {
        return Ok(());
    }
    if !valid_identifier(new) {
        return Err(reject(format!(
            "« {new} » n'est pas un nom de variable valide (lettres, chiffres et _)."
        )));
    }
    let story = project.story();
    if story.variables.iter().any(|v| v.name == new) {
        return Err(reject(format!("La variable « {new} » existe déjà.")));
    }
    let mut variables = story.variables.clone();
    let slot = variables
        .iter_mut()
        .find(|v| v.name == old)
        .ok_or_else(|| reject(format!("Variable « {old} » introuvable.")))?;
    slot.name = new.to_owned();
    let updates: Vec<(NodeId, NodeKind)> = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| {
            let kind = match &n.kind {
                NodeKind::SetVar { var, value } => NodeKind::SetVar {
                    var: if var == old {
                        new.to_owned()
                    } else {
                        var.clone()
                    },
                    value: rename_in_expr(value, old, new),
                },
                NodeKind::If {
                    condition,
                    then_target,
                    else_target,
                } => NodeKind::If {
                    condition: rename_in_expr(condition, old, new),
                    then_target: *then_target,
                    else_target: *else_target,
                },
                NodeKind::Choice { options } => NodeKind::Choice {
                    options: options
                        .iter()
                        .map(|o| {
                            let mut o = o.clone();
                            o.condition = o.condition.as_ref().map(|c| rename_in_expr(c, old, new));
                            o
                        })
                        .collect(),
                },
                _ => return None,
            };
            (kind != n.kind).then_some((n.id, kind))
        })
        .collect();
    project.transaction_labeled("Renommer la variable", |tx| {
        tx.execute(Command::SetVariables { variables })?;
        for (id, kind) in updates {
            tx.execute(Command::ReplaceNodeKind {
                id,
                kind,
                expected_version: None,
            })?;
        }
        Ok(())
    })
}

/// Removes a variable declaration. Refused while a node uses it.
pub fn delete_variable(project: &mut Project, name: &str) -> Result<(), ProjectError> {
    if !variable_users(project.story(), name).is_empty() {
        return Err(reject(format!(
            "La variable « {name} » est encore utilisée par des nœuds."
        )));
    }
    let mut variables = project.story().variables.clone();
    variables.retain(|v| v.name != name);
    project.execute(Command::SetVariables { variables })
}

/// Names of the labels reachable as jump targets: `(id, name)` in reading order.
pub fn labels(story: &Story) -> Vec<(NodeId, String)> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Label { name } => Some((n.id, name.clone())),
            _ => None,
        })
        .collect()
}

/// Whether a position is usable in this story (built-in or declared).
pub fn position_exists(story: &Story, position: &Position) -> bool {
    story.resolve_position(position).is_some()
}
