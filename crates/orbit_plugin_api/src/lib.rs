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
//! another version. Version 1 (M8-04) is the first one third-party plugins
//! may target; later changes are additive or bump it.
//!
//! The **runtime** half ([`runtime`]: [`RuntimePlugin`], [`NodeHandler`],
//! [`UiActionHandler`], [`RuntimeRegistry`]) runs custom nodes and custom UI
//! actions in the player (ADR 0015 §4). See that module for the rollback and
//! save rules a plugin must follow.

pub mod guard;
mod registry;
pub mod runtime;

pub use guard::PluginGuard;
pub use registry::{
    AssetImporter, FieldDef, FieldKind, ImportedAsset, McpToolDef, NodeTypeDef, PluginError,
    ProjectEvent, ProjectHook, ProjectMark, ProjectRegistry, RegisteredNodeType, check_fields, default_data_of,
};
pub use runtime::{
    CustomNode, Handled, NodeHandler, PluginContext, PluginEvent, PluginHost, PluginOutcome,
    RuntimePlugin, RuntimeRegistry, SharedPluginHost, UiActionHandler,
};

/// Version of this API, checked at install time (editor and player).
pub const API_VERSION: u32 = 1;

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
