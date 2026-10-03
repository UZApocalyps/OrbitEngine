//! Decoder for the body of format-version-1 saves.
//!
//! Version 2 added the sprite scale and mirroring to the VM's
//! `CharacterOnStage` (postcard is not self-describing, so the layout change
//! needs its own version). These mirror types are the v1 layouts, kept only to
//! read old saves; characters come back at scale 1.0 and not mirrored (which
//! is what they were drawn with).

use std::collections::BTreeMap;

use orbit_core::{AssetId, CharacterId, NodeId, Position, Value};
use orbit_vm::{
    BacklogEntry, CharacterOnStage, Checkpoint, Interaction, MusicState, Pending, Rng,
    RollbackHistory, SceneState, VmState,
};
use serde::Deserialize;

use crate::error::SaveError;

#[derive(Deserialize)]
struct CharacterOnStageV1 {
    character: CharacterId,
    expression: String,
    position: Position,
}

#[derive(Deserialize)]
struct SceneStateV1 {
    background: Option<AssetId>,
    characters: Vec<CharacterOnStageV1>,
    music: Option<MusicState>,
}

#[derive(Deserialize)]
struct VmStateV1 {
    pc: Option<NodeId>,
    pending: Pending,
    call_stack: Vec<Option<NodeId>>,
    vars: BTreeMap<String, Value>,
    scene: SceneStateV1,
    rng: Rng,
    shown: Option<Interaction>,
}

#[derive(Deserialize)]
struct CheckpointV1 {
    seq: u64,
    state: VmStateV1,
    barrier: bool,
    chosen: Option<usize>,
}

#[derive(Deserialize)]
struct RollbackHistoryV1 {
    checkpoints: Vec<CheckpointV1>,
    backlog: Vec<BacklogEntry>,
    next_seq: u64,
}

#[derive(Deserialize)]
struct SaveBodyV1 {
    vm_state: VmStateV1,
    history: Option<RollbackHistoryV1>,
}

impl From<SceneStateV1> for SceneState {
    fn from(old: SceneStateV1) -> Self {
        SceneState {
            background: old.background,
            characters: old
                .characters
                .into_iter()
                .map(|c| CharacterOnStage {
                    character: c.character,
                    expression: c.expression,
                    position: c.position,
                    scale: [1.0, 1.0],
                    flip_x: false,
                    flip_y: false,
                    source: None,
                })
                .collect(),
            music: old.music,
            scene3d: None,
            bg_layers: Vec::new(),
        }
    }
}

impl From<VmStateV1> for VmState {
    fn from(old: VmStateV1) -> Self {
        VmState {
            pc: old.pc,
            pending: old.pending,
            call_stack: old.call_stack,
            vars: old.vars,
            scene: old.scene.into(),
            rng: old.rng,
            shown: old.shown,
            active_anims: Vec::new(),
            scene_clock: 0.0,
            open_screens: Vec::new(),
        }
    }
}

/// Decodes a v1 body into the current types.
pub(crate) fn decode_body(bytes: &[u8]) -> Result<(VmState, Option<RollbackHistory>), SaveError> {
    let body: SaveBodyV1 = postcard::from_bytes(bytes)
        .map_err(|e| SaveError::Corrupt(format!("save body (v1): {e}")))?;
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

/// Test helper: encodes a v1 body from current types (dropping scale/mirror).
#[cfg(test)]
pub(crate) fn encode_body_for_tests(vm: &VmState, history: Option<&RollbackHistory>) -> Vec<u8> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct CharacterOnStageV1<'a> {
        character: CharacterId,
        expression: &'a str,
        position: &'a Position,
    }
    #[derive(Serialize)]
    struct SceneStateV1<'a> {
        background: Option<AssetId>,
        characters: Vec<CharacterOnStageV1<'a>>,
        music: &'a Option<MusicState>,
    }
    #[derive(Serialize)]
    struct VmStateV1<'a> {
        pc: Option<NodeId>,
        pending: &'a Pending,
        call_stack: &'a Vec<Option<NodeId>>,
        vars: &'a BTreeMap<String, Value>,
        scene: SceneStateV1<'a>,
        rng: Rng,
        shown: &'a Option<Interaction>,
    }
    #[derive(Serialize)]
    struct CheckpointV1<'a> {
        seq: u64,
        state: VmStateV1<'a>,
        barrier: bool,
        chosen: Option<usize>,
    }
    #[derive(Serialize)]
    struct RollbackHistoryV1<'a> {
        checkpoints: Vec<CheckpointV1<'a>>,
        backlog: &'a Vec<BacklogEntry>,
        next_seq: u64,
    }
    #[derive(Serialize)]
    struct SaveBodyV1<'a> {
        vm_state: VmStateV1<'a>,
        history: Option<RollbackHistoryV1<'a>>,
    }
    fn vm_v1(s: &VmState) -> VmStateV1<'_> {
        VmStateV1 {
            pc: s.pc,
            pending: &s.pending,
            call_stack: &s.call_stack,
            vars: &s.vars,
            scene: SceneStateV1 {
                background: s.scene.background,
                characters: s
                    .scene
                    .characters
                    .iter()
                    .map(|c| CharacterOnStageV1 {
                        character: c.character,
                        expression: &c.expression,
                        position: &c.position,
                    })
                    .collect(),
                music: &s.scene.music,
            },
            rng: s.rng,
            shown: &s.shown,
        }
    }
    let body = SaveBodyV1 {
        vm_state: vm_v1(vm),
        history: history.map(|h| RollbackHistoryV1 {
            checkpoints: h
                .checkpoints
                .iter()
                .map(|c| CheckpointV1 {
                    seq: c.seq,
                    state: vm_v1(&c.state),
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
