//! AI Studio as a native editor plugin: the AI game-creation wizard (pitch ->
//! bible -> script -> pictures -> project, `orbit_ai_studio`) and its MCP
//! tools. The editor ships without it; build it with `cargo xtask plugins`
//! and install `target/plugins/orbit.ai_studio-*.orbitplugin` from Tools >
//! Extensions > Plugins.
//!
//! The provider settings stay in the editor's configuration folder
//! (`ai_studio.ron` next to `config.ron`), as when the studio was built in, so
//! existing settings carry over.

mod studio;
mod ui_util;

use orbit_editor_api::native::{NativePlugin, PluginHost};

#[derive(Default)]
pub struct AiStudioNative;

impl NativePlugin for AiStudioNative {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        // Older layouts kept nothing in the plugin folder: the editor's
        // config folder is where the settings have always been.
        let config_dir = host
            .editor_config_dir()
            .map(std::path::Path::to_path_buf)
            .or_else(|| Some(host.data_dir().to_path_buf()));
        let plugin = studio::AiStudioPlugin::new(config_dir);
        host.install(&plugin)?;
        plugin.register_settings(host);
        Ok(())
    }
}

orbit_editor_api::export_plugin!(
    AiStudioNative,
    id = "orbit.ai_studio",
    name = "AI Studio",
    description = "AI game-creation wizard: pitch, story bible, script, generated pictures, implementation in the project; plus its MCP tools.",
    author = "OrbitEngine contributors",
);
