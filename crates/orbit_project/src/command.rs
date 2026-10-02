//! Typed commands on the story IR.

use orbit_core::{
    AssetEntry, AssetId, Character, CharacterId, Node, NodeId, NodeKind, PositionDef, Scene,
    SceneId, Story, StringId, VarDecl,
};
use thiserror::Error;

/// A reversible edit of the story.
///
/// Applying a command yields its exact inverse, which is what the undo stack
/// stores. `expected_version` fields implement optimistic conflict detection:
/// when `Some(v)`, the command fails with [`CommandError::VersionConflict`] if
/// the target node was modified since the caller read version `v`.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Inserts `node` at `index` in `scene`.
    InsertNode {
        scene: SceneId,
        index: usize,
        node: Node,
    },
    /// Removes a node. Refused if another node still targets it.
    RemoveNode {
        id: NodeId,
        expected_version: Option<u64>,
    },
    /// Rewrites the speaker of a `Say` node and the source-language text it
    /// displays (its `text_id` is kept). The string must already exist.
    SetSay {
        id: NodeId,
        speaker: Option<CharacterId>,
        text: String,
        expected_version: Option<u64>,
    },
    /// Sets (`Some`) or removes (`None`) an entry of the source string table.
    /// Needed before inserting a node that displays a new string.
    SetString {
        id: StringId,
        text: Option<String>,
    },
    /// Replaces the whole content of a node (`kind`) keeping its id and place:
    /// the generic setter behind the inspector. A `Label` that is still
    /// targeted cannot be turned into another kind. Strings used by the new
    /// kind must be registered with [`Command::SetString`] first.
    ReplaceNodeKind {
        id: NodeId,
        kind: NodeKind,
        expected_version: Option<u64>,
    },
    /// Moves a node to position `index` of `scene` (an index among the nodes
    /// that remain once the node is taken out), possibly into another scene.
    MoveNode {
        id: NodeId,
        scene: SceneId,
        index: usize,
    },
    /// Inserts a scene (with its nodes, whose ids must be new) at `index`.
    InsertScene {
        index: usize,
        scene: Scene,
    },
    /// Removes a scene and its nodes. Refused if a node outside the scene
    /// still targets one of its nodes.
    RemoveScene {
        id: SceneId,
    },
    RenameScene {
        id: SceneId,
        name: String,
    },
    /// Moves a scene to position `index` (among the other scenes).
    MoveScene {
        id: SceneId,
        index: usize,
    },
    InsertCharacter {
        index: usize,
        character: Character,
    },
    /// Replaces the definition of an existing character (same id).
    ReplaceCharacter {
        character: Character,
    },
    /// Removes a character. Refused while a node still uses it.
    RemoveCharacter {
        id: CharacterId,
    },
    /// Replaces the list of declared variables.
    SetVariables {
        variables: Vec<VarDecl>,
    },
    /// Replaces the list of custom stage positions.
    SetPositions {
        positions: Vec<PositionDef>,
    },
    /// Registers an asset in the manifest at `index`.
    InsertAsset {
        index: usize,
        entry: AssetEntry,
    },
    /// Removes an asset from the manifest. Refused while it is used.
    RemoveAsset {
        id: AssetId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error("unknown scene {0}")]
    UnknownScene(SceneId),
    #[error("unknown node {0}")]
    UnknownNode(NodeId),
    #[error("node {0} already exists")]
    DuplicateNode(NodeId),
    #[error("index {index} out of bounds (scene has {len} nodes)")]
    IndexOutOfBounds { index: usize, len: usize },
    #[error("node {0} is not a Say node")]
    NotASay(NodeId),
    #[error("unknown string {0}")]
    UnknownString(StringId),
    #[error("string {0} is still displayed by node {1}")]
    StringInUse(StringId, NodeId),
    #[error("node {0} is still targeted by node {1}")]
    StillReferenced(NodeId, NodeId),
    #[error("scene {0} already exists")]
    DuplicateScene(SceneId),
    #[error("unknown character {0}")]
    UnknownCharacter(CharacterId),
    #[error("character {0} already exists")]
    DuplicateCharacter(CharacterId),
    #[error("character {0} is still used by node {1}")]
    CharacterInUse(CharacterId, NodeId),
    #[error("unknown asset {0}")]
    UnknownAsset(AssetId),
    #[error("asset {0} already exists")]
    DuplicateAsset(AssetId),
    #[error("asset {0} is still in use")]
    AssetInUse(AssetId),
    #[error("{0} index {1} out of bounds")]
    BadIndex(&'static str, usize),
    #[error("version conflict on node {node}: expected {expected}, found {actual}")]
    VersionConflict {
        node: NodeId,
        expected: u64,
        actual: u64,
    },
}

/// Result of a successful raw application: the inverse command and the ids of
/// the nodes whose content/existence changed.
pub(crate) struct Applied {
    pub inverse: Command,
    /// Nodes whose content or existence changed.
    pub touched: Vec<NodeId>,
}

/// Returns the id of a node that targets `id`, if any.
fn referencer(story: &Story, id: NodeId) -> Option<NodeId> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .find(|n| n.kind.targets().contains(&id))
        .map(|n| n.id)
}

/// Nodes displaying string `id`.
fn users_of_string(story: &Story, id: StringId) -> Vec<NodeId> {
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter(|n| n.kind.strings().contains(&id))
        .map(|n| n.id)
        .collect()
}

fn node_uses_character(kind: &NodeKind, id: CharacterId) -> bool {
    match kind {
        NodeKind::Say { speaker, .. } => *speaker == Some(id),
        NodeKind::Show { character, .. } | NodeKind::Hide { character, .. } => *character == id,
        _ => false,
    }
}

fn node_uses_asset(kind: &NodeKind, id: AssetId) -> bool {
    match kind {
        NodeKind::Background { asset, .. }
        | NodeKind::PlayMusic { asset, .. }
        | NodeKind::PlaySound { asset } => *asset == id,
        _ => false,
    }
}

impl Command {
    /// Short human label (French, like the editor UI) shown in the undo menu.
    pub fn label(&self) -> &'static str {
        match self {
            Command::InsertNode { .. } => "Insérer un nœud",
            Command::RemoveNode { .. } => "Supprimer un nœud",
            Command::SetSay { .. } => "Modifier la réplique",
            Command::SetString { .. } => "Modifier un texte",
            Command::ReplaceNodeKind { .. } => "Modifier le nœud",
            Command::MoveNode { .. } => "Déplacer un nœud",
            Command::InsertScene { .. } => "Ajouter une scène",
            Command::RemoveScene { .. } => "Supprimer la scène",
            Command::RenameScene { .. } => "Renommer la scène",
            Command::MoveScene { .. } => "Déplacer la scène",
            Command::InsertCharacter { .. } => "Ajouter un personnage",
            Command::ReplaceCharacter { .. } => "Modifier le personnage",
            Command::RemoveCharacter { .. } => "Supprimer le personnage",
            Command::SetVariables { .. } => "Modifier les variables",
            Command::SetPositions { .. } => "Modifier les positions",
            Command::InsertAsset { .. } => "Ajouter une ressource",
            Command::RemoveAsset { .. } => "Supprimer la ressource",
        }
    }

    /// Node targeted by the optimistic version check, if the command has one.
    pub(crate) fn expected(&self) -> Option<(NodeId, u64)> {
        match self {
            Command::RemoveNode {
                id,
                expected_version: Some(v),
            }
            | Command::SetSay {
                id,
                expected_version: Some(v),
                ..
            }
            | Command::ReplaceNodeKind {
                id,
                expected_version: Some(v),
                ..
            } => Some((*id, *v)),
            _ => None,
        }
    }

    /// Applies the command to `story` and returns its inverse. The story is
    /// left untouched on error. Version checks are done by the caller.
    pub(crate) fn apply(&self, story: &mut Story) -> Result<Applied, CommandError> {
        match self {
            Command::InsertNode { scene, index, node } => {
                if story.locate(node.id).is_some() {
                    return Err(CommandError::DuplicateNode(node.id));
                }
                let sc = story
                    .scenes
                    .iter_mut()
                    .find(|s| s.id == *scene)
                    .ok_or(CommandError::UnknownScene(*scene))?;
                if *index > sc.nodes.len() {
                    return Err(CommandError::IndexOutOfBounds {
                        index: *index,
                        len: sc.nodes.len(),
                    });
                }
                sc.nodes.insert(*index, node.clone());
                Ok(Applied {
                    inverse: Command::RemoveNode {
                        id: node.id,
                        expected_version: None,
                    },
                    touched: vec![node.id],
                })
            }
            Command::RemoveNode { id, .. } => {
                let (si, ni) = story.locate(*id).ok_or(CommandError::UnknownNode(*id))?;
                if let Some(by) = referencer(story, *id) {
                    return Err(CommandError::StillReferenced(*id, by));
                }
                let scene = story.scenes[si].id;
                let node = story.scenes[si].nodes.remove(ni);
                Ok(Applied {
                    inverse: Command::InsertNode {
                        scene,
                        index: ni,
                        node,
                    },
                    touched: vec![*id],
                })
            }
            Command::SetSay {
                id, speaker, text, ..
            } => {
                let text_id = match story.node(*id).map(|n| &n.kind) {
                    None => return Err(CommandError::UnknownNode(*id)),
                    Some(NodeKind::Say { text_id, .. }) => *text_id,
                    Some(_) => return Err(CommandError::NotASay(*id)),
                };
                if !story.strings.entries.contains_key(&text_id) {
                    return Err(CommandError::UnknownString(text_id));
                }
                let Some(NodeKind::Say {
                    speaker: old_speaker,
                    ..
                }) = story.node_mut(*id).map(|n| &mut n.kind)
                else {
                    return Err(CommandError::NotASay(*id));
                };
                let old_speaker = std::mem::replace(old_speaker, *speaker);
                let Some(old_text) = story.strings.entries.get_mut(&text_id) else {
                    return Err(CommandError::UnknownString(text_id));
                };
                let inverse = Command::SetSay {
                    id: *id,
                    speaker: old_speaker,
                    text: std::mem::replace(old_text, text.clone()),
                    expected_version: None,
                };
                Ok(Applied {
                    inverse,
                    touched: vec![*id],
                })
            }
            Command::SetString { id, text } => {
                let previous = match text {
                    Some(t) => story.strings.set(*id, t.clone()),
                    None => {
                        if let Some(user) = users_of_string(story, *id).first() {
                            return Err(CommandError::StringInUse(*id, *user));
                        }
                        story
                            .strings
                            .entries
                            .remove(id)
                            .ok_or(CommandError::UnknownString(*id))
                            .map(Some)?
                    }
                };
                Ok(Applied {
                    inverse: Command::SetString {
                        id: *id,
                        text: previous,
                    },
                    touched: users_of_string(story, *id),
                })
            }
            Command::ReplaceNodeKind { id, kind, .. } => {
                let (si, ni) = story.locate(*id).ok_or(CommandError::UnknownNode(*id))?;
                let was_label = matches!(story.scenes[si].nodes[ni].kind, NodeKind::Label { .. });
                if was_label
                    && !matches!(kind, NodeKind::Label { .. })
                    && let Some(by) = referencer(story, *id)
                {
                    return Err(CommandError::StillReferenced(*id, by));
                }
                let old = std::mem::replace(&mut story.scenes[si].nodes[ni].kind, kind.clone());
                Ok(Applied {
                    inverse: Command::ReplaceNodeKind {
                        id: *id,
                        kind: old,
                        expected_version: None,
                    },
                    touched: vec![*id],
                })
            }
            Command::MoveNode { id, scene, index } => {
                let (si, ni) = story.locate(*id).ok_or(CommandError::UnknownNode(*id))?;
                let target = story
                    .scenes
                    .iter()
                    .position(|s| s.id == *scene)
                    .ok_or(CommandError::UnknownScene(*scene))?;
                let remaining = story.scenes[target].nodes.len() - usize::from(target == si);
                if *index > remaining {
                    return Err(CommandError::IndexOutOfBounds {
                        index: *index,
                        len: remaining,
                    });
                }
                let old_scene = story.scenes[si].id;
                let node = story.scenes[si].nodes.remove(ni);
                story.scenes[target].nodes.insert(*index, node);
                Ok(Applied {
                    inverse: Command::MoveNode {
                        id: *id,
                        scene: old_scene,
                        index: ni,
                    },
                    touched: vec![*id],
                })
            }
            Command::InsertScene { index, scene } => {
                if story.scenes.iter().any(|s| s.id == scene.id) {
                    return Err(CommandError::DuplicateScene(scene.id));
                }
                if *index > story.scenes.len() {
                    return Err(CommandError::BadIndex("scene", *index));
                }
                for n in &scene.nodes {
                    if story.locate(n.id).is_some() {
                        return Err(CommandError::DuplicateNode(n.id));
                    }
                }
                story.scenes.insert(*index, scene.clone());
                Ok(Applied {
                    inverse: Command::RemoveScene { id: scene.id },
                    touched: scene.nodes.iter().map(|n| n.id).collect(),
                })
            }
            Command::RemoveScene { id } => {
                let index = story
                    .scenes
                    .iter()
                    .position(|s| s.id == *id)
                    .ok_or(CommandError::UnknownScene(*id))?;
                let own: Vec<NodeId> = story.scenes[index].nodes.iter().map(|n| n.id).collect();
                for (si, other) in story.scenes.iter().enumerate() {
                    if si == index {
                        continue;
                    }
                    for n in &other.nodes {
                        if let Some(t) = n.kind.targets().into_iter().find(|t| own.contains(t)) {
                            return Err(CommandError::StillReferenced(t, n.id));
                        }
                    }
                }
                let scene = story.scenes.remove(index);
                Ok(Applied {
                    inverse: Command::InsertScene { index, scene },
                    touched: own,
                })
            }
            Command::RenameScene { id, name } => {
                let scene = story
                    .scenes
                    .iter_mut()
                    .find(|s| s.id == *id)
                    .ok_or(CommandError::UnknownScene(*id))?;
                let old = std::mem::replace(&mut scene.name, name.clone());
                Ok(Applied {
                    inverse: Command::RenameScene { id: *id, name: old },
                    touched: Vec::new(),
                })
            }
            Command::MoveScene { id, index } => {
                let from = story
                    .scenes
                    .iter()
                    .position(|s| s.id == *id)
                    .ok_or(CommandError::UnknownScene(*id))?;
                if *index >= story.scenes.len() {
                    return Err(CommandError::BadIndex("scene", *index));
                }
                let scene = story.scenes.remove(from);
                story.scenes.insert(*index, scene);
                Ok(Applied {
                    inverse: Command::MoveScene {
                        id: *id,
                        index: from,
                    },
                    touched: Vec::new(),
                })
            }
            Command::InsertCharacter { index, character } => {
                if story.character(character.id).is_some() {
                    return Err(CommandError::DuplicateCharacter(character.id));
                }
                if *index > story.characters.len() {
                    return Err(CommandError::BadIndex("character", *index));
                }
                story.characters.insert(*index, character.clone());
                Ok(Applied {
                    inverse: Command::RemoveCharacter { id: character.id },
                    touched: Vec::new(),
                })
            }
            Command::ReplaceCharacter { character } => {
                let slot = story
                    .characters
                    .iter_mut()
                    .find(|c| c.id == character.id)
                    .ok_or(CommandError::UnknownCharacter(character.id))?;
                let old = std::mem::replace(slot, character.clone());
                Ok(Applied {
                    inverse: Command::ReplaceCharacter { character: old },
                    touched: Vec::new(),
                })
            }
            Command::RemoveCharacter { id } => {
                let index = story
                    .characters
                    .iter()
                    .position(|c| c.id == *id)
                    .ok_or(CommandError::UnknownCharacter(*id))?;
                if let Some(user) = story
                    .scenes
                    .iter()
                    .flat_map(|s| &s.nodes)
                    .find(|n| node_uses_character(&n.kind, *id))
                {
                    return Err(CommandError::CharacterInUse(*id, user.id));
                }
                let character = story.characters.remove(index);
                Ok(Applied {
                    inverse: Command::InsertCharacter { index, character },
                    touched: Vec::new(),
                })
            }
            Command::SetVariables { variables } => {
                let old = std::mem::replace(&mut story.variables, variables.clone());
                Ok(Applied {
                    inverse: Command::SetVariables { variables: old },
                    touched: Vec::new(),
                })
            }
            Command::SetPositions { positions } => {
                let old = std::mem::replace(&mut story.positions, positions.clone());
                Ok(Applied {
                    inverse: Command::SetPositions { positions: old },
                    touched: Vec::new(),
                })
            }
            Command::InsertAsset { index, entry } => {
                if story.assets.get(entry.id).is_some() {
                    return Err(CommandError::DuplicateAsset(entry.id));
                }
                if *index > story.assets.assets.len() {
                    return Err(CommandError::BadIndex("asset", *index));
                }
                story.assets.assets.insert(*index, entry.clone());
                Ok(Applied {
                    inverse: Command::RemoveAsset { id: entry.id },
                    touched: Vec::new(),
                })
            }
            Command::RemoveAsset { id } => {
                let index = story
                    .assets
                    .assets
                    .iter()
                    .position(|a| a.id == *id)
                    .ok_or(CommandError::UnknownAsset(*id))?;
                let used = story
                    .scenes
                    .iter()
                    .flat_map(|s| &s.nodes)
                    .any(|n| node_uses_asset(&n.kind, *id))
                    || story
                        .characters
                        .iter()
                        .flat_map(|c| &c.expressions)
                        .any(|e| e.image == *id);
                if used {
                    return Err(CommandError::AssetInUse(*id));
                }
                let entry = story.assets.assets.remove(index);
                Ok(Applied {
                    inverse: Command::InsertAsset { index, entry },
                    touched: Vec::new(),
                })
            }
        }
    }
}
