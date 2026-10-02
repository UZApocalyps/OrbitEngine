//! OrbitEngine project API (skeleton, M0).
//!
//! Invariant (ROADMAP 1.6): **every modification of a project goes through
//! [`Project::execute`] / [`Project::transaction`]**. The editor, the CLI and
//! the future MCP server all use this API, so every edit is validated,
//! undoable, observable and version-checked. `Project` deliberately exposes no
//! mutable access to its [`Story`].
//!
//! This crate has no graphics or platform dependency and compiles to wasm.

mod command;
mod ext;
pub mod graph;
pub mod graph_layout;
pub mod graph_ops;
pub mod ops;
mod project;
pub mod sandbox;
mod storage;

pub use command::{Command, CommandError};
pub use ext::{AssetUser, asset_users};
pub use project::{ChangeEvent, ChangeKind, Project, ProjectError, SubscriptionId, Transaction};
pub use storage::{
    DEFAULT_STORY_FILE, MANIFEST_FILE, MANIFEST_FORMAT, PluginRequirement, ProjectFileError,
    ProjectManifest, ProjectStorage, STATE_DIR, load_manifest, load_story, save_manifest,
    save_story,
};
