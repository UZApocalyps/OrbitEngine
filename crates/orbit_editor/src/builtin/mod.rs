//! Built-in tools of the editor. Every one is an ordinary `EditorPlugin`
//! registered through the same API as third-party plugins (dogfooding):
//! the editor itself has no privileged UI code.

mod actions;
mod anim_edit;
mod assistant;
mod console;
mod curves;
mod definitions;
pub mod drafts;
pub mod edit;
mod explorer;
mod fx_text;
mod export;
pub mod graph;
mod importers;
mod inspector;
mod inspector_fx;
mod inspector_screen;
mod inspector_stage;
mod library;
pub mod manual;
mod nodeinfo;
mod preview;
mod problems;
pub mod profiler;
pub mod reveal;
pub mod sample;
mod rigedit;
mod scene;
pub(crate) mod scene3d;
mod sceneview;
mod timeline;
mod timeline_model;
mod timeline_ui;
mod translations;
mod search;
pub mod thumbs;
mod ui_util;
mod uied;
pub mod textview;
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
    pub const CURVES: &str = "orbit.curves";
    pub const GRAPH: &str = "orbit.graph";
    pub const IMPORTERS: &str = "orbit.importers";
    pub const LIBRARY: &str = "orbit.library";
    pub const PROBLEMS: &str = "orbit.problems";
    pub const SEARCH: &str = "orbit.search";
    pub const SCENEVIEW: &str = "orbit.sceneview";
    pub const SCENE3D: &str = "orbit.scene3d";
    pub const EXPORT: &str = "orbit.export";
    pub const TRANSLATIONS: &str = "orbit.translations";
    pub const ASSISTANT: &str = "orbit.assistant";
    pub const UI_EDITOR: &str = "orbit.ui";
    pub const TEXTVIEW: &str = "orbit.textview";
    pub const MANUAL: &str = "orbit.manual";
    pub const TIMELINE: &str = "orbit.timeline";
    pub const RIG: &str = "orbit.rig";
    pub const PROFILER: &str = "orbit.profiler";
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
    pub const SCENEVIEW: &str = "orbit.sceneview.sceneview";
    pub const SCENE3D_VIEWPORT: &str = "orbit.scene3d.viewport";
    pub const SCENE3D_HIERARCHY: &str = "orbit.scene3d.hierarchy";
    pub const SCENE3D_INSPECTOR: &str = "orbit.scene3d.inspector";
    pub const EXPORT: &str = "orbit.export.export";
    pub const TRANSLATIONS: &str = "orbit.translations.translations";
    pub const ASSISTANT: &str = "orbit.assistant.assistant";
    pub const UI_EDITOR: &str = "orbit.ui.editor";
    pub const TEXTVIEW: &str = "orbit.textview.textview";
    pub const MANUAL: &str = "orbit.manual.manual";
    pub const RIG_VIEWPORT: &str = "orbit.rig.viewport";
    pub const RIG_HIERARCHY: &str = "orbit.rig.hierarchy";
    pub const RIG_INSPECTOR: &str = "orbit.rig.inspector";
    pub const PROFILER: &str = "orbit.profiler.profiler";
    pub const RIG_CLIPS: &str = "orbit.rig.clips";
    pub const RIG_TIMELINE: &str = "orbit.rig.timeline";
}

/// State the built-in plugins share with the editor core.
#[derive(Clone, Default)]
pub struct Shared {
    /// The search index (search panel and command palette).
    pub search: crate::search::SearchHandle,
    /// File moves made by the asset library (undo / redo move them back).
    pub moves: std::rc::Rc<std::cell::RefCell<crate::assets::MoveJournal>>,
    /// Link with the GPU host of the 3D editor (viewport textures).
    pub scene3d: crate::scene3d_host::SharedLink,
    /// Link with the GPU host of the rig editor (viewport texture).
    pub rig: crate::rig_host::SharedLink,
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
        Box::new(curves::CurvesPlugin),
        Box::new(timeline::TimelinePlugin),
        Box::new(library::LibraryPlugin::new(shared.moves.clone())),
        Box::new(preview::PreviewPlugin),
        Box::new(sceneview::SceneViewPlugin),
        Box::new(console::ConsolePlugin),
        Box::new(problems::ProblemsPlugin),
        Box::new(profiler::ProfilerPlugin),
        Box::new(search::SearchPlugin::new(shared.search.clone())),
        Box::new(export::ExportPlugin),
        Box::new(translations::TranslationsPlugin),
        Box::new(assistant::AssistantPlugin),
        Box::new(uied::UiEditorPlugin::default()),
        Box::new(textview::TextViewPlugin),
        Box::new(manual::ManualPlugin),
        Box::new(scene3d::Scene3dPlugin::new(shared.scene3d.clone())),
        Box::new(rigedit::RigPlugin::new(shared.rig.clone())),
        Box::new(SamplePlugin::default()),
    ]
}

/// Registers, once every panel is known, a "Show: <panel>" action for
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
        PluginManifest::new("orbit.core.views", "Panel views", "0.1.0")
    }
}

impl EditorPlugin for PanelActions {
    fn register_ui(&self, registry: &mut EditorRegistry) {
        for (n, (panel_id, title, icon)) in self.panels.iter().enumerate() {
            let panel_id = panel_id.clone();
            let title = title.clone();
            registry.action(
                ActionDef::new(
                    format!("show.{panel_id}"),
                    "action.show_panel",
                    move |ctx| {
                        ctx.request(EditorRequest::ShowPanel(panel_id.clone()));
                    },
                )
                .label_with(move |ctx| ctx.tf("action.show_panel", &[("panel", &ctx.t(&title))]))
                .icon(icon)
                .category("category.view")
                .menu("view", 300 + n as i32),
            );
        }
    }
}
