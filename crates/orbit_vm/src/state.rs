//! Serializable VM state and the declarative scene state.

use std::collections::BTreeMap;

use orbit_core::{AssetId, CharacterId, NodeId, Position, StringId, Value};
use serde::{Deserialize, Serialize};

use crate::event::ChoiceView;

/// A character currently on stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CharacterOnStage {
    pub character: CharacterId,
    pub expression: String,
    pub position: Position,
}

/// The music currently playing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MusicState {
    pub asset: AssetId,
    pub looping: bool,
}

/// What the player sees and hears, as plain data. Restoring it (plus the
/// presentation events that lead to it) is how rollback brings back the
/// background, sprites and music. Characters are ordered back to front.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneState {
    pub background: Option<AssetId>,
    pub characters: Vec<CharacterOnStage>,
    pub music: Option<MusicState>,
}

impl SceneState {
    /// The stage entry of `character`, if shown.
    pub fn character(&self, character: CharacterId) -> Option<&CharacterOnStage> {
        self.characters.iter().find(|c| c.character == character)
    }
}

/// Seeded pseudo random generator (splitmix64). Its state is part of
/// [`VmState`], so random draws are reproducible and survive save/rollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// What the VM is waiting for besides a plain `step()`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pending {
    None,
    /// A choice was presented: `choose(i)` picks `visible[i]`, an index into
    /// the options of `node`.
    Choice {
        node: NodeId,
        visible: Vec<usize>,
    },
}

/// The blocking thing the player is looking at: what a host must re-display
/// after a rollback or a load, without running any node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Interaction {
    Say {
        node: NodeId,
        speaker: Option<CharacterId>,
        string_id: StringId,
    },
    Choice {
        node: NodeId,
        /// Visible options only, in the order `Vm::choose` indexes them.
        options: Vec<ChoiceView>,
    },
}

impl Interaction {
    pub fn node(&self) -> NodeId {
        match self {
            Interaction::Say { node, .. } | Interaction::Choice { node, .. } => *node,
        }
    }
}

/// Everything that defines the VM's progress. Cloning it is a snapshot;
/// `Vm::restore` brings it back.
///
/// Not included on purpose: the story itself (immutable, shared), pending
/// presentation events (a host concern), the "already read" set (persists
/// across rollback, see `Vm::read_nodes`) and **persistent variables**
/// (`VarScope::Persistent`, owned by the host, see `Vm::persistent_vars`).
/// `vars` only holds playthrough (`Game`) variables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VmState {
    /// Next node to execute; `None` once the story has ended.
    pub pc: Option<NodeId>,
    pub pending: Pending,
    /// Return addresses of active `Call`s (`None` = the call was the last node).
    pub call_stack: Vec<Option<NodeId>>,
    pub vars: BTreeMap<String, Value>,
    pub scene: SceneState,
    pub rng: Rng,
    /// The last line / choice presented to the player (what is on screen).
    #[serde(default)]
    pub shown: Option<Interaction>,
}

impl VmState {
    /// The choice node the VM is waiting on, if any.
    pub fn pending_choice_node(&self) -> Option<NodeId> {
        match &self.pending {
            Pending::Choice { node, .. } => Some(*node),
            Pending::None => None,
        }
    }

    /// Every node id this state points to (program counter, call stack,
    /// pending choice, shown interaction): they must exist in the story for
    /// the state to be restorable.
    pub fn referenced_nodes(&self) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = self.pc.into_iter().collect();
        out.extend(self.call_stack.iter().flatten().copied());
        if let Pending::Choice { node, .. } = &self.pending {
            out.push(*node);
        }
        if let Some(shown) = &self.shown {
            out.push(shown.node());
        }
        out
    }
}
