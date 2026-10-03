//! Decoder for the body of format-version-2 saves.
//!
//! Version 3 added the optional 3D scene (`SceneState::scene3d`, ADR 0014)
//! to the VM's scene state; postcard is not self-describing, so the layout
//! change needs its own version. These mirror types are the v2 layouts, kept
//! only to read old saves: they come back without a 3D scene, which is what
//! a game of that time showed.

use std::collections::BTreeMap;

use orbit_core::{AssetId, NodeId, Value};
use orbit_vm::{
    BacklogEntry, CharacterOnStage, Checkpoint, Interaction, MusicState, Pending, Rng,
    RollbackHistory, SceneState, VmState,
};
use serde::Deserialize;

use crate::error::SaveError;

#[derive(Deserialize)]
struct SceneStateV2 {
    background: Option<AssetId>,
    characters: Vec<CharacterOnStage>,
    music: Option<MusicState>,
}

#[derive(Deserialize)]
struct VmStateV2 {
    pc: Option<NodeId>,
    pending: Pending,
    call_stack: Vec<Option<NodeId>>,
    vars: BTreeMap<String, Value>,
    scene: SceneStateV2,
    rng: Rng,
    shown: Option<Interaction>,
}

#[derive(Deserialize)]
struct CheckpointV2 {
    seq: u64,
    state: VmStateV2,
    barrier: bool,
    chosen: Option<usize>,
}

#[derive(Deserialize)]
struct RollbackHistoryV2 {
    checkpoints: Vec<CheckpointV2>,
    backlog: Vec<BacklogEntry>,
    next_seq: u64,
}

#[derive(Deserialize)]
struct SaveBodyV2 {
    vm_state: VmStateV2,
    history: Option<RollbackHistoryV2>,
}

impl From<VmStateV2> for VmState {
    fn from(old: VmStateV2) -> Self {
        VmState {
            pc: old.pc,
            pending: old.pending,
            call_stack: old.call_stack,
            vars: old.vars,
            scene: SceneState {
                background: old.scene.background,
                characters: old.scene.characters,
                music: old.scene.music,
                scene3d: None,
                bg_layers: Vec::new(),
            },
            rng: old.rng,
            shown: old.shown,
            active_anims: Vec::new(),
            scene_clock: 0.0,
            open_screens: Vec::new(),
        }
    }
}

/// Decodes a v2 body into the current types.
pub(crate) fn decode_body(bytes: &[u8]) -> Result<(VmState, Option<RollbackHistory>), SaveError> {
    let body: SaveBodyV2 = postcard::from_bytes(bytes)
        .map_err(|e| SaveError::Corrupt(format!("save body (v2): {e}")))?;
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

/// Test helper: encodes a v2 body from current types (dropping the 3D scene).
#[cfg(test)]
pub(crate) fn encode_body_for_tests(vm: &VmState, history: Option<&RollbackHistory>) -> Vec<u8> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct SceneStateV2<'a> {
        background: Option<AssetId>,
        characters: &'a Vec<CharacterOnStage>,
        music: &'a Option<MusicState>,
    }
    #[derive(Serialize)]
    struct VmStateV2<'a> {
        pc: Option<NodeId>,
        pending: &'a Pending,
        call_stack: &'a Vec<Option<NodeId>>,
        vars: &'a BTreeMap<String, Value>,
        scene: SceneStateV2<'a>,
        rng: Rng,
        shown: &'a Option<Interaction>,
    }
    #[derive(Serialize)]
    struct CheckpointV2<'a> {
        seq: u64,
        state: VmStateV2<'a>,
        barrier: bool,
        chosen: Option<usize>,
    }
    #[derive(Serialize)]
    struct RollbackHistoryV2<'a> {
        checkpoints: Vec<CheckpointV2<'a>>,
        backlog: &'a Vec<BacklogEntry>,
        next_seq: u64,
    }
    #[derive(Serialize)]
    struct SaveBodyV2<'a> {
        vm_state: VmStateV2<'a>,
        history: Option<RollbackHistoryV2<'a>>,
    }
    fn vm_v2(s: &VmState) -> VmStateV2<'_> {
        VmStateV2 {
            pc: s.pc,
            pending: &s.pending,
            call_stack: &s.call_stack,
            vars: &s.vars,
            scene: SceneStateV2 {
                background: s.scene.background,
                characters: &s.scene.characters,
                music: &s.scene.music,
            },
            rng: s.rng,
            shown: &s.shown,
        }
    }
    let body = SaveBodyV2 {
        vm_state: vm_v2(vm),
        history: history.map(|h| RollbackHistoryV2 {
            checkpoints: h
                .checkpoints
                .iter()
                .map(|c| CheckpointV2 {
                    seq: c.seq,
                    state: vm_v2(&c.state),
                    barrier: c.barrier,
                    chosen: c.chosen,
                })
                .collect(),
            backlog: &h.backlog,
            next_seq: h.next_seq,
        }),
    };
    postcard::to_stdvec(&body).unwrap()
}
