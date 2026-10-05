//! OrbitEngine MCP server, headless mode (M3f).
//!
//! `orbit mcp --project <dir>` serves the Model Context Protocol over stdio so
//! an AI client (Claude Code...) can read and edit a project. Everything goes
//! through the `orbit_project` command API: one tool call is one undo entry,
//! it is rolled back as a whole if it fails, and it is saved atomically on
//! success. File access is confined to the project folder.
//!
//! Layers:
//! * [`tools`]: the tools and their input schemas (generated from Rust types);
//! * [`host`]: the project, tool dispatch, saving, the action log
//!   (`.orbit/mcp.log`) and the resources;
//! * [`server`]: the `rmcp` handler and the stdio entry point;
//! * [`sandbox`]: path confinement.
//!
//! This crate is never linked into the player (checked by `xtask check-arch`).

pub mod convert;
pub mod error;
pub mod host;
pub mod live;
pub mod live_tools;
pub use orbit_project::sandbox;
pub mod rig_tools;
pub mod scene3d_tools;
pub mod script_tools;
pub mod voice_tools;
pub mod schema;
pub mod server;
pub mod tools;
pub mod ui_tools;
pub mod views;

pub use error::{ErrorKind, ToolError};
pub use host::{
    ExportRequest, HeadlessHost, LOG_FILE, ProjectHost, ResourceInfo, ToolInfo, ToolOutput,
    ToolRunner, UNDO_PREFIX,
};
pub use server::{LIVE_INSTRUCTIONS, OrbitMcpServer, serve_stdio, serve_stdio_with};
