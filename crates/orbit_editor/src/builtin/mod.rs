//! Built-in tools of the editor. Every one is an ordinary `EditorPlugin`
//! registered through the same API as third-party plugins (dogfooding):
//! the editor itself has no privileged UI code.

mod actions;
mod assistant;
mod console;
mod definitions;
pub mod drafts;
pub mod edit;
mod explorer;
mod export;
pub mod graph;
mod importers;
mod inspector;
mod library;
mod nodeinfo;
mod preview;
mod problems;
pub mod reveal;
mod sample;
mod scene;
mod search;
pub mod thumbs;
mod ui_util;
mod welcome;

use orbit_editor_api::{
    ActionDef, EditorPlugin, EditorRegistry, EditorRequest, Plugin, PluginManifest,
};

pub use export::{ExportJob, JobStatus, LogLine as ExportLogLine};
pub use importers::{import_audio, import_image};
pub use library::import_paths;
pub use nodeinfo::{kind_icon, kind_key, kind_label, summary};
pub use preview::{CANVAS as PREVIEW_CANVAS, fit_16_9, to_virtual};
pub use sample::SamplePlugin;
pub use scene::click_selection;

/// Plugin ids of the built-in tools.
pub mod ids {
    pub const CORE: &str = "orbit.core";
    pub const EXPLORER: &str = "orbit.explorer";
    pub const INSPECTOR: &str = "orbit.inspector";
    pub const CONSOLE: &str = "orbit.console";
    pub const PREVIEW: &str = "orbit.preview";
    pub const WELCOME: &str = "orbit.welcome";
    pub const SAMPLE: &str = "orbit.sample";
    pub const SCENE: &str = "orbit.scene";
    pub const CHARACTERS: &str = "orbit.characters";
    pub const VARIABLES: &str = "orbit.variables";
    pub const GRAPH: &str = "orbit.graph";
    pub const IMPORTERS: &str = "orbit.importers";
    pub const LIBRARY: &str = "orbit.library";
    pub const PROBLEMS: &str = "orbit.problems";
    pub const SEARCH: &str = "orbit.search";
    pub const EXPORT: &str = "orbit.export";
    pub const ASSISTANT: &str = "orbit.assistant";
}

/// Full ids of the built-in panels.
pub mod panels {
    pub const EXPLORER: &str = "orbit.explorer.explorer";
    pub const INSPECTOR: &str = "orbit.inspector.inspector";
    pub const CONSOLE: &str = "orbit.console.console";
    pub const PREVIEW: &str = "orbit.preview.preview";
    pub const WELCOME: &str = "orbit.welcome.welcome";
    pub const SAMPLE: &str = "orbit.sample.panel";
    pub const SCENE: &str = "orbit.scene.scene";
    pub const CHARACTERS: &str = "orbit.characters.characters";
    pub const VARIABLES: &str = "orbit.variables.variables";
    pub const GRAPH: &str = "orbit.graph.graph";
    pub const LIBRARY: &str = "orbit.library.library";
    pub const PROBLEMS: &str = "orbit.problems.problems";
    pub const SEARCH: &str = "orbit.search.search";
    pub const EXPORT: &str = "orbit.export.export";
    pub const ASSISTANT: &str = "orbit.assistant.assistant";
}

/// State the built-in plugins share with the editor core.
#[derive(Clone, Default)]
pub struct Shared {
    /// The search index (search panel and command palette).
    pub search: crate::search::SearchHandle,
    /// File moves made by the asset library (undo / redo move them back).
    pub moves: std::rc::Rc<std::cell::RefCell<crate::assets::MoveJournal>>,
}

/// The built-in plugins, in registration order (menus first, then panels in
/// the order they appear in the default layout).
pub fn plugins() -> Vec<Box<dyn EditorPlugin>> {
    plugins_with(&Shared::default())
}

/// [`plugins`], sharing `shared` with the editor core.
pub fn plugins_with(shared: &Shared) -> Vec<Box<dyn EditorPlugin>> {
    vec![
        Box::new(actions::CorePlugin),
        Box::new(importers::ImportersPlugin),
        Box::new(welcome::WelcomePlugin),
        Box::new(scene::ScenePlugin),
        Box::new(graph::GraphPlugin),
        Box::new(explorer::ExplorerPlugin),
        Box::new(inspector::InspectorPlugin),
        Box::new(definitions::CharactersPlugin),
        Box::new(definitions::VariablesPlugin),
        Box::new(library::LibraryPlugin::new(shared.moves.clone())),
        Box::new(preview::PreviewPlugin),
        Box::new(console::ConsolePlugin),
        Box::new(problems::ProblemsPlugin),
        Box::new(search::SearchPlugin::new(shared.search.clone())),
        Box::new(export::ExportPlugin),
        Box::new(assistant::AssistantPlugin),
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
