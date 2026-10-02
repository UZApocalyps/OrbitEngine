//! Built-in tools of the editor. Every one is an ordinary `EditorPlugin`
//! registered through the same API as third-party plugins (dogfooding):
//! the editor itself has no privileged UI code.

mod actions;
mod console;
mod explorer;
mod inspector;
mod nodeinfo;
mod sample;
mod ui_util;
mod welcome;

use orbit_editor_api::{
    ActionDef, EditorPlugin, EditorRegistry, EditorRequest, Plugin, PluginManifest,
};

pub use nodeinfo::{kind_icon, kind_key, kind_label, summary};
pub use sample::SamplePlugin;

/// Plugin ids of the built-in tools.
pub mod ids {
    pub const CORE: &str = "orbit.core";
    pub const EXPLORER: &str = "orbit.explorer";
    pub const INSPECTOR: &str = "orbit.inspector";
    pub const CONSOLE: &str = "orbit.console";
    pub const WELCOME: &str = "orbit.welcome";
    pub const SAMPLE: &str = "orbit.sample";
}

/// Full ids of the built-in panels.
pub mod panels {
    pub const EXPLORER: &str = "orbit.explorer.explorer";
    pub const INSPECTOR: &str = "orbit.inspector.inspector";
    pub const CONSOLE: &str = "orbit.console.console";
    pub const WELCOME: &str = "orbit.welcome.welcome";
    pub const SAMPLE: &str = "orbit.sample.panel";
}

/// The built-in plugins, in registration order (menus first, then panels in
/// the order they appear in the default layout).
pub fn plugins() -> Vec<Box<dyn EditorPlugin>> {
    vec![
        Box::new(actions::CorePlugin),
        Box::new(welcome::WelcomePlugin),
        Box::new(explorer::ExplorerPlugin),
        Box::new(inspector::InspectorPlugin),
        Box::new(console::ConsolePlugin),
        Box::new(SamplePlugin::default()),
    ]
}

/// Registers, once every panel is known, an "Afficher : <panel>" action for
/// each of them (View menu and palette), so that a closed panel can always
/// be reopened.
pub struct PanelActions {
    panels: Vec<(String, String, &'static str)>,
}

impl PanelActions {
    pub fn for_registry(registry: &EditorRegistry) -> Self {
        Self {
            panels: registry
                .panels()
                .iter()
                .map(|p| (p.def.id.clone(), p.def.title.clone(), p.def.icon))
                .collect(),
        }
    }
}

impl Plugin for PanelActions {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("orbit.core.views", "Affichage des panneaux", "0.1.0")
    }
}

impl EditorPlugin for PanelActions {
    fn register_ui(&self, registry: &mut EditorRegistry) {
        for (n, (panel_id, title, icon)) in self.panels.iter().enumerate() {
            let panel_id = panel_id.clone();
            registry.action(
                ActionDef::new(
                    format!("show.{panel_id}"),
                    format!("Afficher : {title}"),
                    move |ctx| {
                        ctx.request(EditorRequest::ShowPanel(panel_id.clone()));
                    },
                )
                .icon(icon)
                .category("Affichage")
                .menu("view", 300 + n as i32),
            );
        }
    }
}
