//! Decoder for the body of format-version-2 saves.
//!
//! Version 3 added the running animations (`active_anims`) and the scene
//! clock to the VM state (effects and timelines, ADR 0012). postcard is not
//! self-describing, so the layout change needs its own version. These mirror
//! types are the v2 layouts, kept only to read old saves: they come back with
//! no running animation, which is exact since none existed.

use std::collections::BTreeMap;

use orbit_core::{NodeId, Value};
use orbit_vm::{
    BacklogEntry, Checkpoint, Interaction, Pending, Rng, RollbackHistory, SceneState, VmState,
};
use serde::Deserialize;

use crate::error::SaveError;

#[derive(Deserialize)]
struct VmStateV2 {
    pc: Option<NodeId>,
    pending: Pending,
    call_stack: Vec<Option<NodeId>>,
    vars: BTreeMap<String, Value>,
    scene: SceneState,
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
            scene: old.scene,
            rng: old.rng,
            shown: old.shown,
            active_anims: Vec::new(),
            scene_clock: 0.0,
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
