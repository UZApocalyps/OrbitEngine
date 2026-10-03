//! Decoder for the body of format-version-5 saves.
//!
//! Version 6 added the parallax layers of the background (`bg_layers`, M5A-07,
//! ADR 0012 addendum) at the end of the scene state. postcard is not
//! self-describing, so the layout change needs its own version. These mirror
//! types are the v5 layouts, kept only to read old saves: they come back with
//! a plain background, which is exact since the story could not set layers.
//! `SceneStateV5` is also what the older decoders (v3, v4) use, since they
//! read the scene state of their time. (Particles are not in the layout at
//! all: they are recomputed from the running animation list, which a v5 save
//! already has.)

use std::collections::BTreeMap;

use orbit_core::{AssetId, NodeId, Value};
use orbit_vm::{
    ActiveAnim, BacklogEntry, CharacterOnStage, Checkpoint, Interaction, MusicState,
    OpenScreenState, Pending, Rng, RollbackHistory, Scene3dState, SceneState, VmState,
};
use serde::{Deserialize, Serialize};

use crate::error::SaveError;

/// The scene state as it was in formats 3 to 5.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SceneStateV5 {
    pub background: Option<AssetId>,
    pub characters: Vec<CharacterOnStage>,
    pub music: Option<MusicState>,
    pub scene3d: Option<Scene3dState>,
}

impl From<SceneStateV5> for SceneState {
    fn from(old: SceneStateV5) -> Self {
        SceneState {
            background: old.background,
            characters: old.characters,
            music: old.music,
            scene3d: old.scene3d,
            bg_layers: Vec::new(),
        }
    }
}

impl From<&SceneState> for SceneStateV5 {
    fn from(s: &SceneState) -> Self {
        SceneStateV5 {
            background: s.background,
            characters: s.characters.clone(),
            music: s.music.clone(),
            scene3d: s.scene3d.clone(),
        }
    }
}

#[derive(Deserialize)]
struct VmStateV5 {
    pc: Option<NodeId>,
    pending: Pending,
    call_stack: Vec<Option<NodeId>>,
    vars: BTreeMap<String, Value>,
    scene: SceneStateV5,
    rng: Rng,
    shown: Option<Interaction>,
    active_anims: Vec<ActiveAnim>,
    scene_clock: f64,
    open_screens: Vec<OpenScreenState>,
}

#[derive(Deserialize)]
struct CheckpointV5 {
    seq: u64,
    state: VmStateV5,
    barrier: bool,
    chosen: Option<usize>,
}

#[derive(Deserialize)]
struct RollbackHistoryV5 {
    checkpoints: Vec<CheckpointV5>,
    backlog: Vec<BacklogEntry>,
    next_seq: u64,
}

#[derive(Deserialize)]
struct SaveBodyV5 {
    vm_state: VmStateV5,
    history: Option<RollbackHistoryV5>,
}

impl From<VmStateV5> for VmState {
    fn from(old: VmStateV5) -> Self {
        VmState {
            pc: old.pc,
            pending: old.pending,
            call_stack: old.call_stack,
            vars: old.vars,
            scene: old.scene.into(),
            rng: old.rng,
            shown: old.shown,
            active_anims: old.active_anims,
            scene_clock: old.scene_clock,
            open_screens: old.open_screens,
        }
    }
}

/// Decodes a v5 body into the current types.
pub(crate) fn decode_body(bytes: &[u8]) -> Result<(VmState, Option<RollbackHistory>), SaveError> {
    let body: SaveBodyV5 = postcard::from_bytes(bytes)
        .map_err(|e| SaveError::Corrupt(format!("save body (v5): {e}")))?;
    let history = body.history.map(|h| RollbackHistory {
        checkpoints: h
            .checkpoints
            .into_iter()
            .map(|c| Checkpoint {
                seq: c.seq,
                state: c.state.into(),
                barrier: c.barrier,
                chosen: c.chosen,
            })
            .collect(),
        backlog: h.backlog,
        next_seq: h.next_seq,
    });
    Ok((body.vm_state.into(), history))
}

/// Test helper: encodes a v5 body from current types (dropping the layers).
#[cfg(test)]
pub(crate) fn encode_body_for_tests(vm: &VmState, history: Option<&RollbackHistory>) -> Vec<u8> {
    #[derive(Serialize)]
    struct VmStateV5<'a> {
        pc: Option<NodeId>,
        pending: &'a Pending,
        call_stack: &'a Vec<Option<NodeId>>,
        vars: &'a BTreeMap<String, Value>,
        scene: SceneStateV5,
        rng: Rng,
        shown: &'a Option<Interaction>,
        active_anims: &'a Vec<ActiveAnim>,
        scene_clock: f64,
        open_screens: &'a Vec<OpenScreenState>,
    }
    #[derive(Serialize)]
    struct CheckpointV5<'a> {
        seq: u64,
        state: VmStateV5<'a>,
        barrier: bool,
        chosen: Option<usize>,
    }
    #[derive(Serialize)]
    struct RollbackHistoryV5<'a> {
        checkpoints: Vec<CheckpointV5<'a>>,
        backlog: &'a Vec<BacklogEntry>,
        next_seq: u64,
    }
    #[derive(Serialize)]
    struct SaveBodyV5<'a> {
        vm_state: VmStateV5<'a>,
        history: Option<RollbackHistoryV5<'a>>,
    }
    fn vm_v5(s: &VmState) -> VmStateV5<'_> {
        VmStateV5 {
            pc: s.pc,
            pending: &s.pending,
            call_stack: &s.call_stack,
            vars: &s.vars,
            scene: (&s.scene).into(),
            rng: s.rng,
            shown: &s.shown,
            active_anims: &s.active_anims,
            scene_clock: s.scene_clock,
            open_screens: &s.open_screens,
        }
    }
    let body = SaveBodyV5 {
        vm_state: vm_v5(vm),
        history: history.map(|h| RollbackHistoryV5 {
            checkpoints: h
                .checkpoints
                .iter()
                .map(|c| CheckpointV5 {
                    seq: c.seq,
                    state: vm_v5(&c.state),
                    barrier: c.barrier,
                    chosen: c.chosen,
                })
                .collect(),
            backlog: &h.backlog,
            next_seq: h.next_seq,
        }),
    };
    postcard::to_allocvec(&body).expect("v5 body encodes")
}
