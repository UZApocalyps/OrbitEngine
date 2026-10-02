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
mod project;

pub use command::{Command, CommandError};
pub use project::{ChangeEvent, ChangeKind, Project, ProjectError, SubscriptionId, Transaction};
