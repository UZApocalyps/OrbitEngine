//! Built-in tools of the editor. Every one is an ordinary `EditorPlugin`
//! registered through the same API as third-party plugins (dogfooding):
//! the editor itself has no privileged UI code.

mod actions;
mod anim_edit;
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
mod inspector_script;
mod inspector_voice;
pub(crate) mod inspector_hotspots;
mod inspector_hotspot_actions;
mod navmap;
pub(crate) mod inspector_notify;
mod hotspot_edit;
mod script_highlight;
mod scripts_panel;
mod achievements_panel;
mod inspector_stage;
mod library;
mod library_select;
pub mod manual;
mod nodeinfo;
mod game_plugins;
mod plugin_store;
mod plugins;
mod preview;
mod problems;
pub mod profiler;
pub mod reveal;
pub mod sample;
mod rig_expr;
mod rigedit;
mod scene;
pub(crate) mod scene3d;

/// Plugin id and preference key of the "experimental 3D features" switch.
pub const PREF_PLUGIN: &str = "orbit.editor";
pub const PREF_3D: &str = "experimental_3d";
/// Preference key of the automatic validation switch ("0" = off).
pub const PREF_AUTO_VALIDATION: &str = "auto_validation";

/// Is this plugin's UI (menu entries, palette actions) hidden because it
/// belongs to the experimental 3D features and they are off?
pub fn hidden_experimental(plugin_id: &str) -> bool {
    plugin_id == ids::SCENE3D && !orbit_core::experimental::enabled_3d()
}

/// Applies the saved preference to the process-wide switch (editor start).
pub fn apply_saved_prefs(
    prefs: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
) {
    let on = prefs
        .get(PREF_PLUGIN)
        .and_then(|p| p.get(PREF_3D))
        .is_some_and(|v| v == "1");
    orbit_core::experimental::set_preference_3d(on);
}
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
    pub const SCRIPTS: &str = "orbit.scripts";
    pub const ACHIEVEMENTS: &str = "orbit.achievements";
    pub const SEARCH: &str = "orbit.search";
    pub const SCENEVIEW: &str = "orbit.sceneview";
    pub const SCENE3D: &str = "orbit.scene3d";
    pub const EXPORT: &str = "orbit.export";
    pub const TRANSLATIONS: &str = "orbit.translations";
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
    pub const ACHIEVEMENTS: &str = "orbit.achievements.achievements";
    pub const SCENEVIEW: &str = "orbit.sceneview.sceneview";
    pub const SCENE3D_VIEWPORT: &str = "orbit.scene3d.viewport";
    pub const SCENE3D_HIERARCHY: &str = "orbit.scene3d.hierarchy";
    pub const SCENE3D_INSPECTOR: &str = "orbit.scene3d.inspector";
    pub const EXPORT: &str = "orbit.export.export";
    pub const TRANSLATIONS: &str = "orbit.translations.translations";
    /// The "AI Assistant" panel of the `orbit.mcp` plugin (not built in).
    pub const ASSISTANT: &str = "orbit.mcp.assistant";
    pub const UI_EDITOR: &str = "orbit.ui.editor";
    pub const TEXTVIEW: &str = "orbit.textview.textview";
    pub const MANUAL: &str = "orbit.manual.manual";
    pub const RIG_VIEWPORT: &str = "orbit.rig.viewport";
    pub const RIG_HIERARCHY: &str = "orbit.rig.hierarchy";
    pub const RIG_INSPECTOR: &str = "orbit.rig.inspector";
    pub const PROFILER: &str = "orbit.profiler.profiler";
    pub const HITCHES: &str = "orbit.profiler.hitches";
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
    /// Link with the GPU host of the animated thumbnails of rigged expressions.
    pub rig_thumbs: crate::rig_host::SharedThumbs,
    /// Per-user config folder.
    pub config_dir: Option<std::path::PathBuf>,
    /// Native plugins: what loaded, what failed (plugin manager).
    pub plugins: crate::plugins::SharedPlugins,
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
        Box::new(definitions::CharactersPlugin::new(shared.rig_thumbs.clone())),
        Box::new(definitions::VariablesPlugin),
        Box::new(curves::CurvesPlugin),
        Box::new(timeline::TimelinePlugin),
        Box::new(library::LibraryPlugin::new(shared.moves.clone())),
        Box::new(preview::PreviewPlugin),
        Box::new(sceneview::SceneViewPlugin),
        Box::new(console::ConsolePlugin),
        Box::new(problems::ProblemsPlugin),
        Box::new(scripts_panel::ScriptsPlugin),
        Box::new(achievements_panel::AchievementsPlugin),
        Box::new(profiler::ProfilerPlugin),
        Box::new(search::SearchPlugin::new(shared.search.clone())),
        Box::new(export::ExportPlugin),
        Box::new(translations::TranslationsPlugin),
        Box::new(uied::UiEditorPlugin::default()),
        Box::new(textview::TextViewPlugin),
        Box::new(manual::ManualPlugin),
        Box::new(scene3d::Scene3dPlugin::new(shared.scene3d.clone())),
        Box::new(rigedit::RigPlugin::new(shared.rig.clone())),
        Box::new(SamplePlugin::default()),
        Box::new(plugins::PluginsPlugin::new(shared.plugins.clone())),
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

/// Submenu (text keys joined by `/`) of the "Panels" menu for each area.
const GROUP_STORY: &str = "menu.panels/menu.panels.story";
const GROUP_EDIT: &str = "menu.panels/menu.panels.edit";
const GROUP_ASSETS: &str = "menu.panels/menu.panels.assets";
const GROUP_TOOLS: &str = "menu.panels/menu.panels.tools";
pub(crate) const GROUP_DIAG: &str = "menu.panels/menu.panels.diag";
const GROUP_OTHER: &str = "menu.panels/menu.panels.misc";

/// Menu group of a panel's "Show" action. Panels from plugins that are not
/// listed fall in "Other".
fn panel_group(panel_id: &str) -> &'static str {
    match panel_id {
        "orbit.welcome.welcome" | "orbit.scene.scene" | "orbit.graph.graph"
        | "orbit.sceneview.sceneview" | "orbit.textview.textview" | "orbit.preview.preview" => {
            GROUP_STORY
        }
        "orbit.inspector.inspector"
        | "orbit.characters.characters"
        | "orbit.variables.variables"
        | "orbit.curves.curves"
        | "orbit.scripts.scripts"
        | "orbit.translations.translations"
        | "orbit.achievements.achievements" => GROUP_EDIT,
        "orbit.library.library" | "orbit.explorer.explorer" | "orbit.search.search" => GROUP_ASSETS,
        "orbit.console.console"
        | "orbit.problems.problems"
        | "orbit.profiler.profiler"
        | "orbit.profiler.hitches" => GROUP_DIAG,
        "orbit.ui.editor"
        | "orbit.timeline.timeline"
        | "orbit.export.export"
        | "orbit.mcp.assistant"
        | "orbit.manual.manual"
        | "orbit.plugins.manager" => GROUP_TOOLS,
        id if id.starts_with("orbit.rig.") || id.starts_with("orbit.scene3d.") => GROUP_TOOLS,
        _ => GROUP_OTHER,
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
            let group = panel_group(panel_id);
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
                .menu_group(group)
                .category("category.view")
                .menu("view", 300 + n as i32),
            );
        }
    }
}
