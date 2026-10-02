//! OrbitEngine plugin API, **project side** (ADR 0009).
//!
//! Plugins are Rust crates linked statically. A [`Plugin`] contributes what
//! works without any UI: custom node types, validators, asset importers, MCP
//! tools and project event hooks. The editor-side half (panels, actions,
//! inspectors) is `orbit_editor_api::EditorPlugin`, which extends this trait;
//! a headless host (CLI, MCP server) only loads this part.
//!
//! This crate has no GPU, window or egui dependency and compiles to wasm.
//! The API is versioned by [`API_VERSION`]: a host refuses a plugin built for
//! another version. It stays at `0` (free to change) while only built-in
//! plugins exist.

mod registry;

pub use registry::{
    AssetImporter, ImportedAsset, McpToolDef, NodeTypeDef, PluginError, ProjectEvent, ProjectHook,
    ProjectRegistry, RegisteredNodeType,
};

/// Version of this API, checked at install time.
pub const API_VERSION: u32 = 0;

/// Identity of a plugin. Ids are lowercase dotted names (`orbit.core.explorer`,
/// `acme.anim`); every element a plugin registers is keyed `plugin_id.name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The [`API_VERSION`] the plugin was written against.
    pub api_version: u32,
}

impl PluginManifest {
    /// A manifest for the current API version.
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            api_version: API_VERSION,
        }
    }
}

/// The project-side part of a plugin.
pub trait Plugin: 'static {
    fn manifest(&self) -> PluginManifest;

    /// Registers node types, validators, importers, MCP tools and hooks.
    fn register(&self, _registry: &mut ProjectRegistry) {}
}
