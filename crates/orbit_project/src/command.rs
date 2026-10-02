//! Typed commands on the story IR.

use orbit_core::{
    AssetEntry, AssetId, Character, CharacterId, Node, NodeId, NodeKind, Scene, SceneId, Story,
    StringId, VarDecl,
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
    SetString { id: StringId, text: Option<String> },
    /// Inserts a whole scene at `index` in the scene list.
    InsertScene { index: usize, scene: Scene },
    /// Removes a scene and its nodes. Refused if a node of another scene
    /// still targets one of them.
    RemoveScene { id: SceneId },
    /// Renames a scene.
    RenameScene { id: SceneId, name: String },
    /// Moves a node to `index` of `scene` (an index counted once the node has
    /// been taken out of its current place).
    MoveNode {
        id: NodeId,
        scene: SceneId,
        index: usize,
        expected_version: Option<u64>,
    },
    /// Replaces the content of a node, keeping its id.
    SetNodeKind {
        id: NodeId,
        kind: NodeKind,
        expected_version: Option<u64>,
    },
    /// Creates or replaces (`Some`) or removes (`None`) a character. `index`
    /// only matters when creating (`None` appends).
    SetCharacter {
        id: CharacterId,
        character: Option<Character>,
        index: Option<usize>,
    },
    /// Declares or replaces (`Some`) or removes (`None`) a variable.
    SetVariable {
        name: String,
        decl: Option<VarDecl>,
        index: Option<usize>,
    },
    /// Registers or replaces (`Some`) or removes (`None`) an asset of the
    /// manifest (the file itself is the host's business).
    SetAsset {
        id: AssetId,
        entry: Option<AssetEntry>,
        index: Option<usize>,
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
    #[error("character {0} is still used by node {1}")]
    CharacterInUse(CharacterId, NodeId),
    #[error("unknown variable `{0}`")]
    UnknownVariable(String),
    #[error("unknown asset {0}")]
    UnknownAsset(AssetId),
    #[error("asset {0} is still used by node {1}")]
    AssetInUse(AssetId, NodeId),
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

impl Command {
    /// Short human label (French, like the editor UI) shown in the undo menu.
    pub fn label(&self) -> &'static str {
        match self {
            Command::InsertNode { .. } => "Insérer un nœud",
            Command::RemoveNode { .. } => "Supprimer un nœud",
            Command::SetSay { .. } => "Modifier la réplique",
            Command::SetString { .. } => "Modifier un texte",
            Command::InsertScene { .. } => "Créer une scène",
            Command::RemoveScene { .. } => "Supprimer une scène",
            Command::RenameScene { .. } => "Renommer une scène",
            Command::MoveNode { .. } => "Déplacer un nœud",
            Command::SetNodeKind { .. } => "Modifier un nœud",
            Command::SetCharacter { .. } => "Modifier un personnage",
            Command::SetVariable { .. } => "Modifier une variable",
            Command::SetAsset { .. } => "Modifier un asset",
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
            | Command::MoveNode {
                id,
                expected_version: Some(v),
                ..
            }
            | Command::SetNodeKind {
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
            Command::InsertScene { .. }
            | Command::RemoveScene { .. }
            | Command::RenameScene { .. }
            | Command::MoveNode { .. }
            | Command::SetNodeKind { .. }
            | Command::SetCharacter { .. }
            | Command::SetVariable { .. }
            | Command::SetAsset { .. } => crate::ext::apply(self, story),
        }
    }
}
