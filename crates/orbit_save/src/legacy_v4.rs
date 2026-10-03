//! Decoder for the body of format-version-4 saves.
//!
//! Version 5 added the screens opened by the story (`open_screens`, UI-14,
//! ADR 0016) at the end of the VM state. postcard is not self-describing, so
//! the layout change needs its own version. These mirror types are the v4
//! layouts, kept only to read old saves: they come back with no screen open,
//! which is exact since the story could not open any. (`Pending` and
//! `Interaction` gained variants at their end: the existing ones keep their
//! encoding, so the current types decode them.)

use std::collections::BTreeMap;

use orbit_core::{NodeId, Value};
use orbit_vm::{
    ActiveAnim, BacklogEntry, Checkpoint, Interaction, Pending, Rng, RollbackHistory, SceneState,
    VmState,
};
use serde::Deserialize;

use crate::error::SaveError;

#[derive(Deserialize)]
struct VmStateV4 {
    pc: Option<NodeId>,
    pending: Pending,
    call_stack: Vec<Option<NodeId>>,
    vars: BTreeMap<String, Value>,
    scene: SceneState,
    rng: Rng,
    shown: Option<Interaction>,
    active_anims: Vec<ActiveAnim>,
    scene_clock: f64,
}

#[derive(Deserialize)]
struct CheckpointV4 {
    seq: u64,
    state: VmStateV4,
    barrier: bool,
    chosen: Option<usize>,
}

#[derive(Deserialize)]
struct RollbackHistoryV4 {
    checkpoints: Vec<CheckpointV4>,
    backlog: Vec<BacklogEntry>,
    next_seq: u64,
}

#[derive(Deserialize)]
struct SaveBodyV4 {
    vm_state: VmStateV4,
    history: Option<RollbackHistoryV4>,
}

impl From<VmStateV4> for VmState {
    fn from(old: VmStateV4) -> Self {
        VmState {
            pc: old.pc,
            pending: old.pending,
            call_stack: old.call_stack,
            vars: old.vars,
            scene: old.scene,
            rng: old.rng,
            shown: old.shown,
            active_anims: old.active_anims,
            scene_clock: old.scene_clock,
            open_screens: Vec::new(),
        }
    }
}

/// Decodes a v4 body into the current types.
pub(crate) fn decode_body(bytes: &[u8]) -> Result<(VmState, Option<RollbackHistory>), SaveError> {
    let body: SaveBodyV4 = postcard::from_bytes(bytes)
        .map_err(|e| SaveError::Corrupt(format!("save body (v4): {e}")))?;
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
