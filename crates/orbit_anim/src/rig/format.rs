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
///
/// The top level of the text is scanned for the `rig_version` field instead
/// of deserializing a probe struct: skipping the other fields goes through
/// `ron`'s self-describing path, which is super-linear on the long lines of
/// big meshes (4 s for a 500 KB rig, 35 s for 2 MB). Whether the text is
/// valid RON is left to the typed parse that follows.
pub fn rig_text_version(text: &str) -> Result<u32, RigLoadError> {
    match scan_top_level_u32(text, "rig_version") {
        Some(Ok(v)) => Ok(v.max(1)),
        Some(Err(())) => {
            // Unusual spelling of the value: let `ron` read it (and word the error).
            let probe: VersionProbe = ron::from_str(text).map_err(|e| RigLoadError::Parse(e.to_string()))?;
            Ok(probe.rig_version.max(1))
        }
        None => Ok(1),
    }
}

/// The value of the field `name` of the outermost struct of a RON text, in
/// one linear pass (strings and comments skipped). `None`: no such field;
/// `Some(Err)`: present but not a plain decimal `u32`.
fn scan_top_level_u32(text: &str, name: &str) -> Option<Result<u32, ()>> {
    let b = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            c if depth == 1 && (c.is_ascii_alphabetic() || c == b'_') => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let ident = &text[start..i];
                let rest = text[i..].trim_start();
                if ident == name && let Some(value) = rest.strip_prefix(':') {
                    let value = value.trim_start();
                    let digits: &str = &value[..value.bytes().take_while(u8::is_ascii_digit).count()];
                    let after = value[digits.len()..].trim_start();
                    let clean = after.is_empty() || after.starts_with(',') || after.starts_with(')');
                    return Some(digits.parse().ok().filter(|_| clean).ok_or(()));
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
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
