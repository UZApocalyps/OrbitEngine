//! Decoder for the body of format-version-3 saves.
//!
//! Version 4 added the running animations (`active_anims`) and the scene
//! clock to the VM state (effects and timelines, ADR 0012). postcard is not
//! self-describing, so the layout change needs its own version. These mirror
//! types are the v3 layouts, kept only to read old saves: they come back with
//! no running animation, which is exact since none existed.

use std::collections::BTreeMap;

use orbit_core::{NodeId, Value};
use orbit_vm::{
    BacklogEntry, Checkpoint, Interaction, Pending, Rng, RollbackHistory, SceneState, VmState,
};
use serde::Deserialize;

use crate::error::SaveError;

#[derive(Deserialize)]
struct VmStateV3 {
    pc: Option<NodeId>,
    pending: Pending,
    call_stack: Vec<Option<NodeId>>,
    vars: BTreeMap<String, Value>,
    scene: SceneState,
    rng: Rng,
    shown: Option<Interaction>,
}

#[derive(Deserialize)]
struct CheckpointV3 {
    seq: u64,
    state: VmStateV3,
    barrier: bool,
    chosen: Option<usize>,
}

#[derive(Deserialize)]
struct RollbackHistoryV3 {
    checkpoints: Vec<CheckpointV3>,
    backlog: Vec<BacklogEntry>,
    next_seq: u64,
}

#[derive(Deserialize)]
struct SaveBodyV3 {
    vm_state: VmStateV3,
    history: Option<RollbackHistoryV3>,
}

impl From<VmStateV3> for VmState {
    fn from(old: VmStateV3) -> Self {
        VmState {
            pc: old.pc,
            pending: old.pending,
            call_stack: old.call_stack,
            vars: old.vars,
            scene: old.scene,
            rng: old.rng,
            shown: old.shown,
            active_anims: Vec::new(),
            scene_clock: 0.0,
            open_screens: Vec::new(),
        }
    }
}

/// Decodes a v3 body into the current types.
pub(crate) fn decode_body(bytes: &[u8]) -> Result<(VmState, Option<RollbackHistory>), SaveError> {
    let body: SaveBodyV3 = postcard::from_bytes(bytes)
        .map_err(|e| SaveError::Corrupt(format!("save body (v3): {e}")))?;
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

/// Test helper: encodes a v3 body from current types (dropping animations).
#[cfg(test)]
pub(crate) fn encode_body_for_tests(vm: &VmState, history: Option<&RollbackHistory>) -> Vec<u8> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct VmStateV3<'a> {
        pc: Option<NodeId>,
        pending: &'a Pending,
        call_stack: &'a Vec<Option<NodeId>>,
        vars: &'a BTreeMap<String, Value>,
        scene: &'a SceneState,
        rng: Rng,
        shown: &'a Option<Interaction>,
    }
    #[derive(Serialize)]
    struct CheckpointV3<'a> {
        seq: u64,
        state: VmStateV3<'a>,
        barrier: bool,
        chosen: Option<usize>,
    }
    #[derive(Serialize)]
    struct RollbackHistoryV3<'a> {
        checkpoints: Vec<CheckpointV3<'a>>,
        backlog: &'a Vec<BacklogEntry>,
        next_seq: u64,
    }
    #[derive(Serialize)]
    struct SaveBodyV3<'a> {
        vm_state: VmStateV3<'a>,
        history: Option<RollbackHistoryV3<'a>>,
    }
    fn vm_v3(s: &VmState) -> VmStateV3<'_> {
        VmStateV3 {
            pc: s.pc,
            pending: &s.pending,
            call_stack: &s.call_stack,
            vars: &s.vars,
            scene: &s.scene,
            rng: s.rng,
            shown: &s.shown,
        }
    }
    let body = SaveBodyV3 {
        vm_state: vm_v3(vm),
        history: history.map(|h| RollbackHistoryV3 {
            checkpoints: h
                .checkpoints
                .iter()
                .map(|c| CheckpointV3 {
                    seq: c.seq,
                    state: vm_v3(&c.state),
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
