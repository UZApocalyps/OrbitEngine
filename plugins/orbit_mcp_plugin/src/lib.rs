//! The live MCP server of the editor as a native editor plugin (`orbit.mcp`):
//! the HTTP transport (tokio, hyper, rmcp, through `orbit_mcp`'s `server`
//! feature) and the "AI Assistant" panel. The editor itself links neither
//! an async runtime nor a network stack: it owns the request queue and runs
//! every tool on the open project (`orbit_editor::live`); this plugin only
//! speaks the protocol (see [`transport`]).
//!
//! Build it with `cargo xtask plugins` and install
//! `target/plugins/orbit.mcp-*.orbitplugin` from Tools > Extensions >
//! Plugins. The settings (port, token) stay where they always were, in the
//! editor's configuration under `orbit.ai`.

mod assistant;
pub mod transport;
mod ui_util;
#[cfg(test)]
mod live_mcp_tests;

use orbit_editor_sdk::native::{NativePlugin, PluginHost};

pub use assistant::PANEL_ID;
pub use transport::HttpTransport;

#[derive(Default)]
pub struct McpNative;

impl NativePlugin for McpNative {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        host.mcp_transport(Box::new(HttpTransport::new()));
        assistant::register(host.registry());
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(
    McpNative,
    id = "orbit.mcp",
    name = "MCP server",
    description = "Live MCP server of the editor (HTTP on 127.0.0.1, token) and the AI Assistant panel: lets an AI client such as Claude Code edit the open project.",
    author = "OrbitEngine contributors",
);
