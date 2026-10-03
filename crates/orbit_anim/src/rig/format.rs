//! `.rig.ron` text format: version detection and textual migration skeleton
//! (same model as `ui_version`: each step rewrites the text of version N into
//! version N + 1, then the current struct is decoded).

use serde::Deserialize;

use super::model::{RIG_VERSION, Rig};

#[derive(Debug, Clone, PartialEq)]
pub enum RigLoadError {
    /// Not RON, or not a rig.
    Parse(String),
    /// Written by a newer engine.
    TooNew { found: u32, supported: u32 },
    Migration { from: u32, message: String },
}

impl core::fmt::Display for RigLoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RigLoadError::Parse(e) => write!(f, "invalid rig: {e}"),
            RigLoadError::TooNew { found, supported } => {
                write!(f, "rig_version {found} is newer than this engine ({supported})")
            }
            RigLoadError::Migration { from, message } => write!(f, "rig migration from v{from} failed: {message}"),
        }
    }
}

impl core::error::Error for RigLoadError {}

/// A loaded rig and what was done to it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedRig {
    pub rig: Rig,
    /// Version found in the file when it was older than [`RIG_VERSION`].
    pub migrated_from: Option<u32>,
}

/// Migration step `i` turns version `i + 1` into `i + 2`. Never remove one
/// (backward compatibility, plan §1.3).
type Step = fn(&str) -> Result<String, String>;
const STEPS: &[Step] = &[];
const _: () = assert!(STEPS.len() as u32 + 1 == RIG_VERSION);

#[derive(Deserialize)]
struct VersionProbe {
    #[serde(default)]
    rig_version: u32,
}

/// Version declared by a `.rig.ron` text (a missing field reads as 1, the
/// first format).
pub fn rig_text_version(text: &str) -> Result<u32, RigLoadError> {
    let probe: VersionProbe = ron::from_str(text).map_err(|e| RigLoadError::Parse(e.to_string()))?;
    Ok(probe.rig_version.max(1))
}

/// Brings a `.rig.ron` text to [`RIG_VERSION`].
pub fn migrate_rig_text(text: &str) -> Result<(String, Option<u32>), RigLoadError> {
    let from = rig_text_version(text)?;
    if from > RIG_VERSION {
        return Err(RigLoadError::TooNew { found: from, supported: RIG_VERSION });
    }
    let mut cur = text.to_string();
    for (i, step) in STEPS.iter().enumerate().skip(from as usize - 1) {
        cur = step(&cur).map_err(|message| RigLoadError::Migration { from: i as u32 + 1, message })?;
    }
    Ok((cur, (from < RIG_VERSION).then_some(from)))
}

/// Parses (and migrates) a `.rig.ron` text. Validation is separate
/// ([`Rig::validate`]).
pub fn load_rig(text: &str) -> Result<LoadedRig, RigLoadError> {
    let (text, migrated_from) = migrate_rig_text(text)?;
    let mut rig: Rig = ron::from_str(&text).map_err(|e| RigLoadError::Parse(e.to_string()))?;
    rig.rig_version = RIG_VERSION;
    Ok(LoadedRig { rig, migrated_from })
}

/// Pretty RON, always at [`RIG_VERSION`], weights normalized.
pub fn save_rig(rig: &Rig) -> Result<String, String> {
    let mut rig = rig.clone();
    rig.rig_version = RIG_VERSION;
    rig.normalize_weights();
    ron::ser::to_string_pretty(&rig, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())
}
