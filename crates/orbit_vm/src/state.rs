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
    /// Effective sprite scale `[x, y]`: the `Show` override or the
    /// character's default scale (1.0 = native size).
    #[serde(default = "unit_scale")]
    pub scale: [f32; 2],
    /// Mirrored on the X axis.
    #[serde(default)]
    pub flip_x: bool,
    /// Mirrored on the Y axis.
    #[serde(default)]
    pub flip_y: bool,
    /// The `Show` node that last placed the character. Editor metadata (the
    /// scene view selects it when the character is clicked); the VM never
    /// reads it, and it may be `None` (states restored from format-1 saves).
    #[serde(default)]
    pub source: Option<NodeId>,
}

fn unit_scale() -> [f32; 2] {
    [1.0, 1.0]
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
    /// The 3D scene shown over (instead of) the 2D background, if any
    /// (ADR 0014). Declarative only: scene and camera, never GPU state.
    /// Missing in format-2 saves (decoded by `orbit_save::legacy_v2`).
    #[serde(default)]
    pub scene3d: Option<Scene3dState>,
}

/// A 3D scene on screen: which `.scene3d.ron` asset, from which camera.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene3dState {
    pub scene: AssetId,
    /// Named camera of the scene; `None` is its first camera.
    pub camera: Option<String>,
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
    /// The `OpenScreen { wait: true }` node `node` opened a screen: the story
    /// resumes when the host calls `Vm::close_screen` for it (UI-14).
    /// Appended (save format 5): older variants keep their encoding.
    Screen {
        node: NodeId,
    },
}

/// A screen opened by the story (`NodeKind::OpenScreen`, UI-14, ADR 0016).
/// Declarative: the host builds the screen from it, so rollback and loading
/// bring it back in its initial state (scroll and focus are not kept).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenScreenState {
    /// Id of the screen (`ScreenAsset::id`).
    pub screen: String,
    /// The `OpenScreen` node that opened it (its `result` variable receives
    /// the value the screen closes with).
    pub node: NodeId,
    /// The parameters, evaluated when the node ran.
    pub params: Vec<(String, Value)>,
    /// The story waits for this screen to close.
    pub waiting: bool,
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
    /// The story waits for the screen `screen`, opened by the
    /// `OpenScreen { wait: true }` node `node`, to close (UI-14). Appended
    /// (save format 5).
    Screen { node: NodeId, screen: String },
}

impl Interaction {
    pub fn node(&self) -> NodeId {
        match self {
            Interaction::Say { node, .. }
            | Interaction::Choice { node, .. }
            | Interaction::Screen { node, .. } => *node,
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
    /// Running (or finished, until the scene changes) `Effect` / `Timeline`
    /// nodes, in start order (save format 3). The VM only knows *when* they
    /// started; the values come from sampling the node at
    /// `scene_clock - started_at`.
    #[serde(default)]
    pub active_anims: Vec<ActiveAnim>,
    /// Seconds of game time since the current scene was entered, advanced by
    /// the host (`Vm::advance_clock`), frozen in pauses.
    #[serde(default)]
    pub scene_clock: f64,
    /// Screens opened by the story, in opening order (save format 5, UI-14).
    /// Missing in format-4 saves (decoded by `orbit_save::legacy_v4`).
    #[serde(default)]
    pub open_screens: Vec<OpenScreenState>,
}

/// An animation started by an `Effect` / `Timeline` node.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveAnim {
    pub node: NodeId,
    /// `scene_clock` when the node ran.
    pub started_at: f64,
}

impl VmState {
    /// The choice node the VM is waiting on, if any.
    pub fn pending_choice_node(&self) -> Option<NodeId> {
        match &self.pending {
            Pending::Choice { node, .. } => Some(*node),
            Pending::None | Pending::Screen { .. } => None,
        }
    }

    /// Every node id this state points to (program counter, call stack,
    /// pending choice or screen, shown interaction, open screens): they must
    /// exist in the story for the state to be restorable.
    pub fn referenced_nodes(&self) -> Vec<NodeId> {
        let mut out: Vec<NodeId> = self.pc.into_iter().collect();
        out.extend(self.call_stack.iter().flatten().copied());
        if let Pending::Choice { node, .. } | Pending::Screen { node } = &self.pending {
            out.push(*node);
        }
        if let Some(shown) = &self.shown {
            out.push(shown.node());
        }
        out.extend(self.open_screens.iter().map(|s| s.node));
        out
    }
}
