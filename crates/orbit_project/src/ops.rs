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

/// A refusal with a stable `code` (the editor writes the message in its own
/// language from it) and the French `message` (see [`crate::Rejection`]).
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
                NodeKind::JumpScene { label, .. } => *label = label.map(remap),
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
        return Err(reject("clipboard_empty", "Le presse-papiers est vide."));
    }
    insert_drafts(project, scene, index, instantiate(clip), "Coller")
}

/// Duplicates nodes right after the last one. Returns the new ids.
pub fn duplicate_nodes(project: &mut Project, ids: &[NodeId]) -> Result<Vec<NodeId>, ProjectError> {
    let story = project.story();
    let ordered = in_story_order(story, ids);
    let Some(&last) = ordered.last() else {
        return Err(reject("nothing_to_duplicate", "Rien à dupliquer."));
    };
    let (si, ni) = story
        .locate(last)
        .ok_or_else(|| reject("node_not_found", "Nœud introuvable."))?;
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
            return Err(reject_with(
                "node_referenced",
                &[("name", name.to_string())],
                format!("Impossible de supprimer {name} : un autre nœud y fait encore référence."),
            ));
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
        .ok_or_else(|| reject("scene_not_found", "Scène introuvable."))?;
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
                .ok_or_else(|| reject("scene_not_found", "Scène introuvable."))?;
            let own = sc.nodes.iter().position(|n| n.id == id);
            let at = match anchor.and_then(|a| sc.nodes.iter().position(|n| n.id == a)) {
                Some(a) => a - usize::from(own.is_some_and(|o| o < a)),
                None => sc.nodes.len() - usize::from(own.is_some()),
            };
            tx.execute(Command::MoveNode {
                id,
                scene,
                index: at,
                expected_version: None,
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
        .ok_or_else(|| reject("node_not_found", "Nœud introuvable."))?;
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
        tx.execute(Command::SetCharacter {
            id,
            character: Some(Character {
                id,
                name_id,
                color,
                expressions: Vec::new(),
                default_position: None,
                default_scale: 1.0,
            }),
            index: Some(index),
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
        .ok_or_else(|| reject("character_not_found", "Personnage introuvable."))?;
    project.transaction_labeled("Supprimer le personnage", |tx| {
        tx.execute(Command::SetCharacter {
            id,
            character: None,
            index: None,
        })?;
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
            Err(reject_with(
                "asset_other_type",
                &[("path", path.to_string())],
                format!("« {path} » est déjà déclaré d'un autre type."),
            ))
        };
    }
    let id = AssetId::new();
    let index = project.story().assets.assets.len();
    project.execute(Command::SetAsset {
        id,
        entry: Some(AssetEntry {
            id,
            kind,
            path: path.to_owned(),
        }),
        index: Some(index),
    })?;
    Ok(id)
}

/// Whether `path` can be the manifest path of an asset: relative, `/`
/// separators, no `..`, no empty segment, no drive or `\`.
pub fn is_valid_asset_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.contains('\0')
        && !path.split('/').any(|seg| seg.is_empty() || seg == "..")
}

/// Changes the manifest path of an asset (its id, and so every reference to
/// it, stays the same). Refused when the path is invalid or already used by
/// another asset (compared without regard to case: Windows would merge two
/// files that Linux keeps apart). The file itself is moved by the caller.
pub fn set_asset_path(
    project: &mut Project,
    id: AssetId,
    new_path: &str,
) -> Result<(), ProjectError> {
    let entry = project
        .story()
        .assets
        .get(id)
        .cloned()
        .ok_or_else(|| reject("asset_not_found", "Ressource introuvable."))?;
    if !is_valid_asset_path(new_path) {
        return Err(reject_with(
            "asset_path_invalid",
            &[("path", new_path.to_string())],
            format!("« {new_path} » n'est pas un chemin valide."),
        ));
    }
    if entry.path == new_path {
        return Ok(());
    }
    let lower = new_path.to_lowercase();
    if project
        .story()
        .assets
        .assets
        .iter()
        .any(|a| a.id != id && a.path.to_lowercase() == lower)
    {
        return Err(reject_with(
            "asset_path_taken",
            &[("path", new_path.to_string())],
            format!(
                "Une autre ressource utilise déjà le chemin « {new_path} » (la casse ne compte pas)."
            ),
        ));
    }
    let index = project
        .story()
        .assets
        .assets
        .iter()
        .position(|a| a.id == id);
    project.transaction_labeled("Renommer la ressource", |tx| {
        tx.execute(Command::SetAsset {
            id,
            entry: Some(AssetEntry {
                path: new_path.to_owned(),
                ..entry
            }),
            index,
        })
    })
}

/// Removes an asset from the manifest (the file stays on disk). Refused,
/// with the reason in French, while a node or a character expression still
/// uses it.
pub fn remove_asset(project: &mut Project, id: AssetId) -> Result<(), ProjectError> {
    if project.story().assets.get(id).is_none() {
        return Err(reject("asset_not_found", "Ressource introuvable."));
    }
    let story = project.story();
    let users = crate::ext::asset_users(story, id);
    if !users.is_empty() {
        return Err(describe_asset_users(story, &users));
    }
    project.transaction_labeled("Supprimer la ressource", |tx| {
        tx.execute(Command::SetAsset {
            id,
            entry: None,
            index: None,
        })
    })
}

fn describe_asset_users(story: &Story, users: &[crate::ext::AssetUser]) -> ProjectError {
    use crate::ext::AssetUser;
    let nodes = users
        .iter()
        .filter(|u| matches!(u, AssetUser::Node(_)))
        .count();
    let names: Vec<String> = users
        .iter()
        .filter_map(|u| match u {
            AssetUser::Character(c) => story
                .character(*c)
                .and_then(|c| story.text(c.name_id))
                .map(|n| format!("« {n} »")),
            AssetUser::Node(_) => None,
        })
        .collect();
    let mut parts = Vec::new();
    if nodes > 0 {
        parts.push(format!("{nodes} nœud(s)"));
    }
    if !names.is_empty() {
        parts.push(format!("une expression de {}", names.join(", ")));
    }
    reject_with(
        "asset_in_use",
        &[
            ("nodes", nodes.to_string()),
            ("characters", names.join(", ")),
        ],
        format!(
            "Cette ressource est encore utilisée par {} : retirez-la d'abord.",
            parts.join(" et ")
        ),
    )
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
        .ok_or_else(|| reject("character_not_found", "Personnage introuvable."))?;
    if new.is_empty() {
        return Err(reject(
            "expression_name_empty",
            "Le nom d'une expression ne peut pas être vide.",
        ));
    }
    if new == old {
        return Ok(());
    }
    if def.expression(new).is_some() {
        return Err(reject_with(
            "expression_exists",
            &[("name", new.to_string())],
            format!("Ce personnage a déjà une expression « {new} »."),
        ));
    }
    if def.expression(old).is_none() {
        return Err(reject_with(
            "expression_not_found",
            &[("name", old.to_string())],
            format!("Expression « {old} » introuvable."),
        ));
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
                ..
            } if *c == character && expression == old => {
                // Only the expression changes: position, scale and mirroring
                // are kept.
                let mut kind = n.kind.clone();
                if let NodeKind::Show { expression, .. } = &mut kind {
                    *expression = new.to_owned();
                }
                Some((n.id, kind))
            }
            _ => None,
        })
        .collect();
    project.transaction_labeled("Renommer l'expression", |tx| {
        tx.execute(Command::SetCharacter {
            id: def.id,
            character: Some(def),
            index: None,
        })?;
        for (id, kind) in updates {
            tx.execute(Command::SetNodeKind {
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
        return Err(reject_with(
            "variable_name_invalid",
            &[("name", decl.name.clone())],
            format!(
                "« {} » n'est pas un nom de variable valide (lettres, chiffres et _).",
                decl.name
            ),
        ));
    }
    if project
        .story()
        .variables
        .iter()
        .any(|v| v.name == decl.name)
    {
        return Err(reject_with(
            "variable_exists",
            &[("name", decl.name.clone())],
            format!("La variable « {} » existe déjà.", decl.name),
        ));
    }
    project.execute(Command::SetVariable {
        name: decl.name.clone(),
        decl: Some(decl),
        index: None,
    })
}

/// Replaces a variable declaration (type, initial value, scope).
pub fn update_variable(
    project: &mut Project,
    name: &str,
    initial: Value,
    scope: orbit_core::VarScope,
) -> Result<(), ProjectError> {
    let mut decl = project
        .story()
        .variables
        .iter()
        .find(|v| v.name == name)
        .cloned()
        .ok_or_else(|| {
            reject_with(
                "variable_not_found",
                &[("name", name.to_string())],
                format!("Variable « {name} » introuvable."),
            )
        })?;
    decl.initial = initial;
    decl.scope = scope;
    project.execute(Command::SetVariable {
        name: name.to_owned(),
        decl: Some(decl),
        index: None,
    })
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
        return Err(reject_with(
            "variable_name_invalid",
            &[("name", new.to_string())],
            format!("« {new} » n'est pas un nom de variable valide (lettres, chiffres et _)."),
        ));
    }
    let story = project.story();
    if story.variables.iter().any(|v| v.name == new) {
        return Err(reject_with(
            "variable_exists",
            &[("name", new.to_string())],
            format!("La variable « {new} » existe déjà."),
        ));
    }
    let mut renamed = story
        .variables
        .iter()
        .find(|v| v.name == old)
        .cloned()
        .ok_or_else(|| {
            reject_with(
                "variable_not_found",
                &[("name", old.to_string())],
                format!("Variable « {old} » introuvable."),
            )
        })?;
    renamed.name = new.to_owned();
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
        tx.execute(Command::SetVariable {
            name: old.to_owned(),
            decl: Some(renamed),
            index: None,
        })?;
        for (id, kind) in updates {
            tx.execute(Command::SetNodeKind {
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
        return Err(reject_with(
            "variable_in_use",
            &[("name", name.to_string())],
            format!("La variable « {name} » est encore utilisée par des nœuds."),
        ));
    }
    project.execute(Command::SetVariable {
        name: name.to_owned(),
        decl: None,
        index: None,
    })
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

/// How a `Show` node places a character: where, how big, mirrored or not.
/// This is what the inspector and the scene view edit.
#[derive(Clone, Debug, PartialEq)]
pub struct ShowLook {
    pub position: Position,
    /// Scale override; `None` uses the character's default scale.
    pub scale: Option<f32>,
    /// Vertical scale for a non-uniform stretch; `None` follows `scale`.
    pub scale_y: Option<f32>,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl ShowLook {
    /// Effective `[x, y]` scale once the character's default is applied.
    pub fn effective_scale(&self, default_scale: f32) -> [f32; 2] {
        let x = self.scale.unwrap_or(default_scale);
        [x, self.scale_y.unwrap_or(x)]
    }

    /// Sets an effective `[x, y]` scale: uniform values are stored as a
    /// single `scale` (no `scale_y`), so the file stays minimal.
    pub fn set_effective_scale(&mut self, scale: [f32; 2]) {
        self.scale = Some(scale[0]);
        self.scale_y = (scale[1] != scale[0]).then_some(scale[1]);
    }

    /// Whether the look differs from "standard" (default scale, no mirror).
    pub fn is_default_style(&self) -> bool {
        self.scale.is_none() && self.scale_y.is_none() && !self.flip_x && !self.flip_y
    }
}

/// The look of a `Show` node, or `None` when `node` is not a `Show`.
pub fn show_look(story: &Story, node: NodeId) -> Option<ShowLook> {
    match &story.node(node)?.kind {
        NodeKind::Show {
            position,
            scale,
            scale_y,
            flip_x,
            flip_y,
            ..
        } => Some(ShowLook {
            position: position.clone(),
            scale: *scale,
            scale_y: *scale_y,
            flip_x: *flip_x,
            flip_y: *flip_y,
        }),
        _ => None,
    }
}

/// Checks a look: scales finite and bounded, free coordinates finite and
/// in range, named positions known.
pub fn check_show_look(story: &Story, look: &ShowLook) -> Result<(), ProjectError> {
    for s in [look.scale, look.scale_y].into_iter().flatten() {
        if !orbit_core::scale_is_valid(s) {
            return Err(reject_with(
                "scale_out_of_range_got",
                &[
                    ("min", orbit_core::SCALE_MIN.to_string()),
                    ("max", orbit_core::SCALE_MAX.to_string()),
                    ("value", s.to_string()),
                ],
                format!(
                    "L'échelle doit être comprise entre {} et {} (reçu {s}).",
                    orbit_core::SCALE_MIN,
                    orbit_core::SCALE_MAX
                ),
            ));
        }
    }
    if let Position::Free { x, y } = &look.position
        && !(orbit_core::FREE_POSITION_RANGE.contains(x)
            && orbit_core::FREE_POSITION_RANGE.contains(y))
    {
        return Err(reject_with(
            "free_position_out_of_range",
            &[
                ("x", x.to_string()),
                ("y", y.to_string()),
                ("min", orbit_core::FREE_POSITION_RANGE.start().to_string()),
                ("max", orbit_core::FREE_POSITION_RANGE.end().to_string()),
            ],
            format!(
                "La position libre ({x}, {y}) est hors limites ({}..{}).",
                orbit_core::FREE_POSITION_RANGE.start(),
                orbit_core::FREE_POSITION_RANGE.end()
            ),
        ));
    }
    if !position_exists(story, &look.position) {
        return Err(reject("position_unknown", "Position inconnue."));
    }
    Ok(())
}

/// The command that gives a `Show` node a new look (everything else of the
/// node is kept). Use it with [`Project::execute_merged`] for a live drag;
/// [`set_show_look`] is the one-shot form.
pub fn show_look_command(
    story: &Story,
    node: NodeId,
    look: &ShowLook,
) -> Result<Command, ProjectError> {
    check_show_look(story, look)?;
    let mut kind = story
        .node(node)
        .ok_or_else(|| reject("node_not_found", "Nœud introuvable."))?
        .kind
        .clone();
    let NodeKind::Show {
        position,
        scale,
        scale_y,
        flip_x,
        flip_y,
        ..
    } = &mut kind
    else {
        return Err(reject("not_a_show", "Ce nœud n'est pas un « Montrer »."));
    };
    *position = look.position.clone();
    *scale = look.scale;
    *scale_y = look.scale_y;
    *flip_x = look.flip_x;
    *flip_y = look.flip_y;
    Ok(Command::SetNodeKind {
        id: node,
        kind,
        expected_version: None,
    })
}

/// Gives a `Show` node a new position / scale / mirroring as one undo entry.
pub fn set_show_look(
    project: &mut Project,
    node: NodeId,
    look: &ShowLook,
) -> Result<(), ProjectError> {
    let command = show_look_command(project.story(), node, look)?;
    project.transaction_labeled("Placer le personnage", |tx| tx.execute(command))
}

/// Sets a character's default scale (one undo entry). Refused outside
/// `SCALE_MIN..=SCALE_MAX`.
pub fn set_default_scale(
    project: &mut Project,
    character: CharacterId,
    scale: f32,
) -> Result<(), ProjectError> {
    if !orbit_core::scale_is_valid(scale) {
        return Err(reject_with(
            "scale_out_of_range",
            &[
                ("min", orbit_core::SCALE_MIN.to_string()),
                ("max", orbit_core::SCALE_MAX.to_string()),
            ],
            format!(
                "L'échelle doit être comprise entre {} et {}.",
                orbit_core::SCALE_MIN,
                orbit_core::SCALE_MAX
            ),
        ));
    }
    let mut def = project
        .story()
        .character(character)
        .ok_or_else(|| reject("character_not_found", "Personnage introuvable."))?
        .clone();
    def.default_scale = scale;
    project.transaction_labeled("Échelle du personnage", |tx| {
        tx.execute(Command::SetCharacter {
            id: character,
            character: Some(def),
            index: None,
        })
    })
}
