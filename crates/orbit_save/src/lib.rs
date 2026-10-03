//! OrbitEngine saves.
//!
//! * [`SaveData`]: a versioned, compact save (VM state + bounded rollback
//!   history + metadata + optional PNG thumbnail), see [`data`] for the format.
//! * [`check_compat`] / [`Compat`]: loading a save after the story was edited.
//!   The story hash covers the *structure* only (ids and node kinds), never
//!   the texts.
//! * [`SaveManager`]: manual slots, a rotation of autosaves and a quicksave on
//!   top of any [`Storage`], plus the separate persistent data
//!   ([`PersistentData`]: persistent variables and read lines) and
//!   [`Preferences`].
//! * Storage backends: [`MemoryStorage`] (tests), `FileStorage` (native, one
//!   file per key, atomic writes) and `LocalStorage` (web, base64 in
//!   `localStorage`).
//!
//! The crate never reads a clock and never spawns a thread: the host passes
//! timestamps and play time in [`CaptureInfo`]. It is the only crate besides
//! `orbit_assets` that touches `std::fs` (behind `cfg(not(wasm32))`), and it
//! compiles to `wasm32-unknown-unknown`.

pub mod compat;
pub mod data;
pub mod error;
pub mod hash;
mod legacy_v1;
mod legacy_v2;
mod legacy_v3;
mod legacy_v4;

#[cfg(test)]
mod anim_save_tests;
#[cfg(test)]
mod collection_save_tests;
#[cfg(test)]
mod screen_save_tests;
#[cfg(test)]
mod unlock_tests;
pub mod manager;
pub mod persistent;
pub mod storage;

#[cfg(not(target_arch = "wasm32"))]
mod file_storage;
#[cfg(target_arch = "wasm32")]
mod web_storage;

pub use compat::{Compat, check_compat};
pub use data::{CaptureInfo, DEFAULT_SAVED_HISTORY, FORMAT_VERSION, SaveData, SaveMeta};
pub use error::SaveError;
#[cfg(not(target_arch = "wasm32"))]
pub use file_storage::{FileStorage, default_data_dir};
pub use hash::story_hash;
pub use manager::{LoadedSave, SaveConfig, SaveManager, SlotId, SlotInfo, SlotState};
pub use persistent::{
    BusVolumes, GAMEPAD_ACTIONS, PERSISTENT_KEY, PREFS_KEY, PersistentData, Preferences, SkipMode,
};
pub use storage::{MemoryStorage, Storage, StorageError, validate_key};
#[cfg(target_arch = "wasm32")]
pub use web_storage::LocalStorage;

#[cfg(test)]
mod tests;
