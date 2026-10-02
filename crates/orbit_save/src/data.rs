//! The save file format and [`SaveData`].
//!
//! # Format (version 1)
//!
//! A binary container, little endian:
//!
//! ```text
//! offset  size  content
//! 0       4     magic  "ORBS"
//! 4       4     format_version (u32)
//! 8       4     meta_len (u32)
//! 12      m     meta  = postcard(SaveMeta)       -- small, listing reads only this
//! 12+m    ...   body  = postcard(SaveBody)       -- vm state + rollback history
//! ```
//!
//! **Why postcard and not RON:** a save embeds up to ~50 checkpoints and an
//! optional PNG thumbnail; RON would spell the PNG as a list of decimal
//! numbers (4x larger than the raw bytes) and balloon the history. postcard is
//! compact, `serde`-based, MIT/Apache, and works on wasm. Its drawback (not
//! self-describing, no field defaults) is exactly what `format_version` is
//! for: any change to [`SaveMeta`], [`SaveBody`] or the serialized VM types
//! requires bumping [`FORMAT_VERSION`] and keeping a decoder for the old
//! layout. Human-editable data (persistent variables, preferences) use RON
//! instead, see [`crate::persistent`].
//!
//! The meta block is separate so the slot list can show the label, date,
//! preview text and thumbnail without decoding the VM state.

use orbit_vm::{Interaction, RollbackHistory, Vm, VmError, VmState};
use serde::{Deserialize, Serialize};

use crate::error::SaveError;
use crate::hash::story_hash;

/// Version of the container and of the postcard layouts below.
pub const FORMAT_VERSION: u32 = 1;
const MAGIC: &[u8; 4] = b"ORBS";
const HEADER_LEN: usize = 12;
/// Default number of checkpoints embedded in a save.
pub const DEFAULT_SAVED_HISTORY: usize = 50;

/// What a slot list needs; cheap to decode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveMeta {
    pub format_version: u32,
    /// [`story_hash`] of the story the save was made with.
    pub story_hash: u64,
    /// Creation time, milliseconds since the Unix epoch (given by the host:
    /// the save code never reads a clock).
    pub created_at_ms: u64,
    /// Total play time in seconds (given by the host).
    pub play_time_secs: u64,
    /// Name of the current scene, unless the host provided another label.
    pub label: String,
    /// Text of the last displayed line (source language unless the host
    /// provided a localized one).
    pub preview: String,
    /// The save embeds a rollback history.
    pub has_history: bool,
    /// PNG bytes, if the host took a screenshot.
    pub thumbnail: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SaveBody {
    vm_state: VmState,
    history: Option<RollbackHistory>,
}

/// A complete save.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveData {
    pub meta: SaveMeta,
    /// Playthrough progress. Persistent variables are *not* here.
    pub vm_state: VmState,
    /// Rollback history up to the current point (bounded), if embedded.
    pub history: Option<RollbackHistory>,
}

/// Host-provided information for [`SaveData::capture`].
#[derive(Clone, Debug)]
pub struct CaptureInfo {
    pub created_at_ms: u64,
    pub play_time_secs: u64,
    /// Overrides the default label (current scene name).
    pub label: Option<String>,
    /// Overrides the default preview (last line in the source language), e.g.
    /// with the localized text.
    pub preview: Option<String>,
    pub thumbnail: Option<Vec<u8>>,
    /// Number of checkpoints to embed; `None` embeds no history (saves are
    /// then smaller but cannot be rolled back after loading).
    pub max_history: Option<usize>,
}

impl CaptureInfo {
    pub fn new(created_at_ms: u64, play_time_secs: u64) -> Self {
        Self {
            created_at_ms,
            play_time_secs,
            label: None,
            preview: None,
            thumbnail: None,
            max_history: Some(DEFAULT_SAVED_HISTORY),
        }
    }
}

impl SaveData {
    /// Captures the current progress of `vm`.
    pub fn capture(vm: &Vm, info: CaptureInfo) -> SaveData {
        let story = vm.story();
        let vm_state = vm.snapshot();
        let label = info.label.unwrap_or_else(|| {
            let at = vm_state
                .shown
                .as_ref()
                .map(Interaction::node)
                .or(vm_state.pc);
            at.and_then(|n| story.locate(n))
                .map(|(scene, _)| story.scenes[scene].name.clone())
                .unwrap_or_default()
        });
        let preview = info.preview.unwrap_or_else(|| {
            vm.backlog()
                .last()
                .and_then(|b| vm.text(b.string_id))
                .unwrap_or_default()
                .to_owned()
        });
        let history = info
            .max_history
            .map(|n| vm.history_snapshot(n, DEFAULT_SAVED_HISTORY));
        SaveData {
            meta: SaveMeta {
                format_version: FORMAT_VERSION,
                story_hash: story_hash(story),
                created_at_ms: info.created_at_ms,
                play_time_secs: info.play_time_secs,
                label,
                preview,
                has_history: history.is_some(),
                thumbnail: info.thumbnail,
            },
            vm_state,
            history,
        }
    }

    /// Restores the progress into `vm` (which runs the same, or a compatible
    /// newer, story). The rollback history comes back too; if it does not fit
    /// the story any more, only the state is restored. Returns whether the
    /// history was restored. Persistent variables and read lines are the
    /// host's business (see [`crate::PersistentData`]).
    pub fn apply_to(&self, vm: &mut Vm) -> Result<bool, VmError> {
        if let Some(history) = &self.history
            && vm
                .restore_with_history(self.vm_state.clone(), history.clone())
                .is_ok()
        {
            return Ok(true);
        }
        vm.restore(self.vm_state.clone())?;
        Ok(false)
    }

    /// Serializes to the container format described in the module docs.
    pub fn encode(&self) -> Result<Vec<u8>, SaveError> {
        let meta = postcard::to_stdvec(&self.meta).map_err(|e| SaveError::Encode(e.to_string()))?;
        let body = postcard::to_stdvec(&SaveBody {
            vm_state: self.vm_state.clone(),
            history: self.history.clone(),
        })
        .map_err(|e| SaveError::Encode(e.to_string()))?;
        let meta_len =
            u32::try_from(meta.len()).map_err(|_| SaveError::Encode("meta too big".into()))?;
        let mut out = Vec::with_capacity(HEADER_LEN + meta.len() + body.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&meta_len.to_le_bytes());
        out.extend_from_slice(&meta);
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Decodes a whole save. Never panics on malformed input.
    pub fn decode(bytes: &[u8]) -> Result<SaveData, SaveError> {
        let (meta, body_bytes) = split(bytes)?;
        let body: SaveBody = postcard::from_bytes(body_bytes)
            .map_err(|e| SaveError::Corrupt(format!("save body: {e}")))?;
        Ok(SaveData {
            meta,
            vm_state: body.vm_state,
            history: body.history,
        })
    }

    /// Decodes only the metadata (cheap, for slot lists).
    pub fn decode_meta(bytes: &[u8]) -> Result<SaveMeta, SaveError> {
        split(bytes).map(|(meta, _)| meta)
    }
}

/// Parses the header and the meta block; returns the meta and the rest.
fn split(bytes: &[u8]) -> Result<(SaveMeta, &[u8]), SaveError> {
    if bytes.len() < HEADER_LEN || &bytes[..4] != MAGIC {
        return Err(SaveError::Corrupt("not an OrbitEngine save".into()));
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version == 0 {
        return Err(SaveError::Corrupt("invalid format version 0".into()));
    }
    if version > FORMAT_VERSION {
        return Err(SaveError::Incompatible(format!(
            "save format version {version} is newer than this game supports ({FORMAT_VERSION})"
        )));
    }
    // Older versions (< FORMAT_VERSION) would be decoded with their own
    // layout here. Only version 1 exists so far.
    let meta_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let rest = &bytes[HEADER_LEN..];
    if meta_len > rest.len() {
        return Err(SaveError::Corrupt("truncated save".into()));
    }
    let (meta_bytes, body) = rest.split_at(meta_len);
    let meta: SaveMeta = postcard::from_bytes(meta_bytes)
        .map_err(|e| SaveError::Corrupt(format!("save header: {e}")))?;
    Ok((meta, body))
}
