//! OrbitEngine project API (skeleton, M0).
//!
//! Invariant (ROADMAP 1.6): **every modification of a project goes through
//! [`Project::execute`] / [`Project::transaction`]**. The editor, the CLI and
//! the future MCP server all use this API, so every edit is validated,
//! undoable, observable and version-checked. `Project` deliberately exposes no
//! mutable access to its [`Story`].
//!
//! This crate has no graphics or platform dependency and compiles to wasm.

mod assets_root;
mod command;
pub mod export;
mod ext;
pub mod graph;
pub mod graph_layout;
pub mod graph_ops;
pub mod lock;
pub mod ops;
pub mod rigmesh;
pub mod rigops;
pub mod rigs;
pub mod scripts;
pub mod achievements;
pub mod voices;
mod project;
pub mod sandbox;
mod storage;

pub use assets_root::find_assets_root;
pub use command::{Command, CommandError};
pub use export::{
    AndroidFormat, AndroidOrientation, DesktopQuality, ExportProfile, ExportTarget, PakCompression,
    is_valid_android_package, slugify,
};
pub use ext::{AssetUser, asset_users};
pub use project::{
    ChangeEvent, ChangeKind, Project, ProjectError, Rejection, SubscriptionId, Transaction,
};
pub use storage::{
    DEFAULT_STORY_FILE, LEGACY_STORY_EXT, LEGACY_STORY_FILE, MANIFEST_FILE, MANIFEST_FORMAT,
    PluginRequirement, ProjectFileError, ProjectManifest, ProjectStorage, STATE_DIR, STORY_EXT,
    default_story_in, is_story_file_name, load_manifest, load_story, orbit_story_name,
    save_manifest, save_story,
};
