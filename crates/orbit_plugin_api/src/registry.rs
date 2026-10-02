//! The project-side registry filled by [`crate::Plugin::register`].

use std::collections::BTreeSet;

use orbit_core::{AssetKind, CustomData, NodeId, NodeKind, Story};
use orbit_project::{ChangeEvent, Project};
use thiserror::Error;

use crate::{API_VERSION, Plugin, PluginManifest};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PluginError {
    #[error("plugin `{id}` targets plugin API {found}, this host speaks {API_VERSION}")]
    IncompatibleApi { id: String, found: u32 },
    #[error("plugin `{0}` is already installed")]
    DuplicatePlugin(String),
    #[error("invalid plugin id `{0}` (expected lowercase dotted name)")]
    InvalidId(String),
    #[error("`{0}` is registered twice")]
    DuplicateElement(String),
}

/// What happened to the open project, for [`ProjectHook`]s.
#[derive(Debug)]
pub enum ProjectEvent<'a> {
    Opened { title: &'a str },
    Closed,
    Changed(&'a ChangeEvent),
    BeforeSave,
}

pub type ProjectHook = Box<dyn Fn(&ProjectEvent<'_>)>;

/// A node type defined by a plugin (stored as `NodeKind::Custom`).
#[derive(Clone)]
pub struct NodeTypeDef {
    /// Local name; the node's `kind` field.
    pub kind: String,
    pub label: String,
    /// Icon key understood by the editor (a glyph).
    pub icon: String,
    pub default_data: fn() -> CustomData,
    /// Problems of one node's data, empty when valid.
    pub validate: Option<fn(&CustomData) -> Vec<String>>,
}

/// A node type together with the plugin that owns it.
#[derive(Clone)]
pub struct RegisteredNodeType {
    pub plugin_id: String,
    pub def: NodeTypeDef,
}

/// Result of an [`AssetImporter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedAsset {
    pub kind: AssetKind,
    pub bytes: Vec<u8>,
}

/// Converts a dropped file into a project asset.
pub struct AssetImporter {
    pub label: String,
    /// Lowercase extensions without the dot.
    pub extensions: Vec<String>,
    pub import: fn(&[u8]) -> Result<ImportedAsset, String>,
}

type McpCall = Box<dyn Fn(&mut Project, &str) -> Result<String, String>>;

/// A tool offered to an AI client over MCP. Calls go through the project's
/// command API like every other edit.
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    /// JSON schema of the arguments.
    pub input_schema: String,
    pub destructive: bool,
    pub call: McpCall,
}

/// Everything plugins registered on the project side. Keys are
/// `plugin_id.element`.
#[derive(Default)]
pub struct ProjectRegistry {
    current: String,
    plugins: Vec<PluginManifest>,
    node_types: Vec<RegisteredNodeType>,
    importers: Vec<(String, AssetImporter)>,
    mcp_tools: Vec<(String, McpToolDef)>,
    hooks: Vec<(String, ProjectHook)>,
}

fn valid_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

impl ProjectRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a plugin: checks its manifest, then lets it register.
    pub fn install(&mut self, plugin: &dyn Plugin) -> Result<(), PluginError> {
        let manifest = plugin.manifest();
        self.begin(&manifest)?;
        plugin.register(self);
        Ok(())
    }

    /// Checks `manifest` and makes it the owner of what is registered next.
    /// Used by hosts that drive several registries (editor).
    pub fn begin(&mut self, manifest: &PluginManifest) -> Result<(), PluginError> {
        if manifest.api_version != API_VERSION {
            return Err(PluginError::IncompatibleApi {
                id: manifest.id.clone(),
                found: manifest.api_version,
            });
        }
        if !valid_plugin_id(&manifest.id) {
            return Err(PluginError::InvalidId(manifest.id.clone()));
        }
        if self.plugins.iter().any(|p| p.id == manifest.id) {
            return Err(PluginError::DuplicatePlugin(manifest.id.clone()));
        }
        self.current = manifest.id.clone();
        self.plugins.push(manifest.clone());
        Ok(())
    }

    /// Plugins installed so far, in order.
    pub fn plugins(&self) -> &[PluginManifest] {
        &self.plugins
    }

    /// Id of the plugin being installed.
    pub fn current_plugin(&self) -> &str {
        &self.current
    }

    pub fn has_plugin(&self, id: &str) -> bool {
        self.plugins.iter().any(|p| p.id == id)
    }

    /// Registers a node type owned by the plugin being installed.
    pub fn register_node_type(&mut self, def: NodeTypeDef) -> Result<(), PluginError> {
        if self.node_type(&self.current, &def.kind).is_some() {
            return Err(PluginError::DuplicateElement(format!(
                "{}.{}",
                self.current, def.kind
            )));
        }
        self.node_types.push(RegisteredNodeType {
            plugin_id: self.current.clone(),
            def,
        });
        Ok(())
    }

    pub fn register_importer(&mut self, importer: AssetImporter) {
        self.importers.push((self.current.clone(), importer));
    }

    pub fn register_mcp_tool(&mut self, tool: McpToolDef) -> Result<(), PluginError> {
        let key = format!("{}.{}", self.current, tool.name);
        if self
            .mcp_tools
            .iter()
            .any(|(p, t)| format!("{p}.{}", t.name) == key)
        {
            return Err(PluginError::DuplicateElement(key));
        }
        self.mcp_tools.push((self.current.clone(), tool));
        Ok(())
    }

    /// Subscribes to project events (opened, closed, changed, before save).
    pub fn on_project_event(&mut self, hook: impl Fn(&ProjectEvent<'_>) + 'static) {
        self.hooks.push((self.current.clone(), Box::new(hook)));
    }

    pub fn node_type(&self, plugin_id: &str, kind: &str) -> Option<&RegisteredNodeType> {
        self.node_types
            .iter()
            .find(|n| n.plugin_id == plugin_id && n.def.kind == kind)
    }

    pub fn node_types(&self) -> &[RegisteredNodeType] {
        &self.node_types
    }

    pub fn importer_for(&self, extension: &str) -> Option<&AssetImporter> {
        let ext = extension.trim_start_matches('.').to_ascii_lowercase();
        self.importers
            .iter()
            .map(|(_, i)| i)
            .find(|i| i.extensions.contains(&ext))
    }

    pub fn mcp_tools(&self) -> impl Iterator<Item = (String, &McpToolDef)> {
        self.mcp_tools
            .iter()
            .map(|(p, t)| (format!("{p}.{}", t.name), t))
    }

    /// Delivers an event to every hook.
    pub fn dispatch(&self, event: &ProjectEvent<'_>) {
        for (_, hook) in &self.hooks {
            hook(event);
        }
    }

    /// Custom nodes whose plugin is not installed: kept untouched on save,
    /// shown read-only, reported as a warning.
    pub fn missing_plugin_nodes(&self, story: &Story) -> Vec<(NodeId, String)> {
        story
            .scenes
            .iter()
            .flat_map(|s| &s.nodes)
            .filter_map(|n| match &n.kind {
                NodeKind::Custom { plugin_id, .. } if !self.has_plugin(plugin_id) => {
                    Some((n.id, plugin_id.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Plugins (ids) that the story needs and that are not installed.
    pub fn missing_plugins(&self, story: &Story) -> BTreeSet<String> {
        self.missing_plugin_nodes(story)
            .into_iter()
            .map(|(_, p)| p)
            .collect()
    }

    /// Runs the validators of installed node types over `story`.
    pub fn validate_custom(&self, story: &Story) -> Vec<(NodeId, String)> {
        let mut problems = Vec::new();
        for node in story.scenes.iter().flat_map(|s| &s.nodes) {
            if let NodeKind::Custom {
                plugin_id,
                kind,
                data,
                ..
            } = &node.kind
                && let Some(validate) = self.node_type(plugin_id, kind).and_then(|t| t.def.validate)
            {
                problems.extend(validate(data).into_iter().map(|m| (node.id, m)));
            }
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use orbit_core::{Node, Scene, SceneId};

    use super::*;

    struct Anim;

    fn clip_default() -> CustomData {
        CustomData::Map(vec![("clip".into(), CustomData::Str(String::new()))])
    }

    fn clip_validate(data: &CustomData) -> Vec<String> {
        match data.get("clip").and_then(CustomData::as_str) {
            Some("") | None => vec!["clip name is empty".into()],
            Some(_) => Vec::new(),
        }
    }

    impl Plugin for Anim {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("acme.anim", "Animation", "0.1.0")
        }

        fn register(&self, reg: &mut ProjectRegistry) {
            reg.register_node_type(NodeTypeDef {
                kind: "play_clip".into(),
                label: "Play clip".into(),
                icon: "play".into(),
                default_data: clip_default,
                validate: Some(clip_validate),
            })
            .unwrap();
            reg.register_importer(AssetImporter {
                label: "Clip".into(),
                extensions: vec!["clip".into()],
                import: |bytes| {
                    Ok(ImportedAsset {
                        kind: AssetKind::Image,
                        bytes: bytes.to_vec(),
                    })
                },
            });
        }
    }

    fn story_with_custom(plugin: &str, data: CustomData) -> Story {
        let mut story = Story::new("t");
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes: vec![Node {
                id: NodeId::new(),
                kind: NodeKind::Custom {
                    plugin_id: plugin.into(),
                    kind: "play_clip".into(),
                    version: 1,
                    data,
                },
            }],
        });
        story
    }

    #[test]
    fn install_registers_and_lookups_work() {
        let mut reg = ProjectRegistry::new();
        reg.install(&Anim).unwrap();
        assert!(reg.has_plugin("acme.anim"));
        assert!(reg.node_type("acme.anim", "play_clip").is_some());
        assert!(reg.node_type("acme.anim", "nope").is_none());
        assert!(reg.importer_for(".CLIP").is_some());
        assert!(reg.importer_for("png").is_none());
    }

    #[test]
    fn duplicates_and_bad_versions_are_refused() {
        let mut reg = ProjectRegistry::new();
        reg.install(&Anim).unwrap();
        assert_eq!(
            reg.install(&Anim),
            Err(PluginError::DuplicatePlugin("acme.anim".into()))
        );
        struct Future;
        impl Plugin for Future {
            fn manifest(&self) -> PluginManifest {
                PluginManifest {
                    api_version: API_VERSION + 1,
                    ..PluginManifest::new("x.y", "Y", "1")
                }
            }
        }
        assert!(matches!(
            reg.install(&Future),
            Err(PluginError::IncompatibleApi { .. })
        ));
        struct BadId;
        impl Plugin for BadId {
            fn manifest(&self) -> PluginManifest {
                PluginManifest::new("Bad Id", "B", "1")
            }
        }
        assert!(matches!(
            reg.install(&BadId),
            Err(PluginError::InvalidId(_))
        ));
    }

    #[test]
    fn a_node_type_cannot_be_registered_twice_by_one_plugin() {
        struct Twice;
        impl Plugin for Twice {
            fn manifest(&self) -> PluginManifest {
                PluginManifest::new("t.twice", "T", "1")
            }
            fn register(&self, reg: &mut ProjectRegistry) {
                let def = NodeTypeDef {
                    kind: "k".into(),
                    label: "K".into(),
                    icon: String::new(),
                    default_data: CustomData::default,
                    validate: None,
                };
                assert!(reg.register_node_type(def.clone()).is_ok());
                assert!(matches!(
                    reg.register_node_type(def),
                    Err(PluginError::DuplicateElement(_))
                ));
            }
        }
        let mut reg = ProjectRegistry::new();
        reg.install(&Twice).unwrap();
    }

    #[test]
    fn missing_plugin_nodes_are_reported_and_validators_run() {
        let mut reg = ProjectRegistry::new();
        let story = story_with_custom("acme.anim", clip_default());
        assert_eq!(reg.missing_plugins(&story).len(), 1);
        reg.install(&Anim).unwrap();
        assert!(reg.missing_plugins(&story).is_empty());
        assert_eq!(reg.validate_custom(&story).len(), 1); // empty clip name
        let good = story_with_custom(
            "acme.anim",
            CustomData::Map(vec![("clip".into(), CustomData::Str("walk".into()))]),
        );
        assert!(reg.validate_custom(&good).is_empty());
    }

    #[test]
    fn hooks_receive_events() {
        let seen = Rc::new(Cell::new(0));
        let mut reg = ProjectRegistry::new();
        struct H(Rc<Cell<u32>>);
        impl Plugin for H {
            fn manifest(&self) -> PluginManifest {
                PluginManifest::new("h.hook", "H", "1")
            }
            fn register(&self, reg: &mut ProjectRegistry) {
                let seen = self.0.clone();
                reg.on_project_event(move |e| {
                    if matches!(e, ProjectEvent::BeforeSave) {
                        seen.set(seen.get() + 1);
                    }
                });
            }
        }
        reg.install(&H(seen.clone())).unwrap();
        reg.dispatch(&ProjectEvent::BeforeSave);
        reg.dispatch(&ProjectEvent::Closed);
        assert_eq!(seen.get(), 1);
    }
}
