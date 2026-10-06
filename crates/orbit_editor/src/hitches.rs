//! Hitch reports: a UI frame longer than the threshold (100 ms) is logged
//! once with its most expensive spans and its GPU waits (console panel and
//! `hitches.log` in the editor's config folder), so that a user can send
//! the evidence. `orbit_profile` keeps the last 20 (View > Show hitches).
//!
//! Cost: one comparison per frame; a hitch builds one line of text.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Log file name, in the config folder.
pub const LOG_FILE: &str = "hitches.log";

static LOG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Sets where hitches are appended (first call wins). `None`: console only.
pub fn init(config_dir: Option<&Path>) {
    let path = config_dir.map(|d| d.join(LOG_FILE));
    let _ = LOG_PATH.set(path.clone());
    if let Some(path) = path {
        // Bounded: a log grown past 1 MB starts over (on the writer thread).
        crate::writer::submit(None, move || {
            if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
                let _ = std::fs::remove_file(&path);
            }
        });
    }
}

/// The hitch log file, when there is one.
pub fn log_path() -> Option<&'static Path> {
    LOG_PATH.get().and_then(|p| p.as_deref())
}

/// Logs the hitches recorded since the previous call. Call once per frame,
/// after `orbit_profile::finish_frame`.
pub fn report_new() {
    let hitches = orbit_profile::take_new_hitches();
    if hitches.is_empty() {
        return;
    }
    let mut text = String::new();
    for h in &hitches {
        let line = h.summary();
        log::warn!(target: "orbit_editor::hitch", "{line}");
        text.push_str(&format!("{} {line}\n", h.at_unix_ms));
    }
    if let Some(path) = log_path() {
        crate::writer::append(path.to_owned(), text);
    }
}
