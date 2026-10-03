//! Runtime half of the plugin API (ADR 0015 §4, M8-04): handlers for
//! `NodeKind::Custom` nodes and `UiAction::Custom` actions, run by the VM.
//!
//! A handler never touches the game state directly. It gets a
//! [`PluginContext`] (variables read-only, the VM's seeded RNG) and queues
//! [`orbit_core::VarOp`]s and events; the VM applies them all or nothing
//! once it returns. Handlers are `&self` + `Send + Sync`: **a plugin keeps
//! no game state of its own**. Everything that must survive a save, a load
//! or a rollback lives in story variables declared in the project (the
//! plugin's "serializable state"); RNG draws are part of the saved VM state.
//! Re-running a node from the same state thus gives the same result.
//!
//! Plugins are linked statically (ADR 0005): an exported game "with
//! plugins" is a player built with `cargo` that installs them in a
//! [`RuntimeRegistry`] (see `orbit_export::plugins`).

use std::collections::BTreeMap;
use std::sync::Arc;

use orbit_core::{CustomData, NodeKind, Story};
pub use orbit_vm::{Handled, PluginContext, PluginEvent, PluginHost, PluginOutcome, SharedPluginHost};

use crate::registry::{PluginError, check_manifest};
use crate::PluginManifest;

/// A custom node as a handler sees it.
#[derive(Clone, Copy, Debug)]
pub struct CustomNode<'a> {
    pub kind: &'a str,
    /// The plugin's own data version (for its migrations).
    pub version: u32,
    pub data: &'a CustomData,
}

/// Runs one kind of custom node.
pub trait NodeHandler: Send + Sync {
    fn run(&self, node: &CustomNode<'_>, ctx: &mut PluginContext<'_>) -> PluginOutcome;
}

impl<F> NodeHandler for F
where
    F: Fn(&CustomNode<'_>, &mut PluginContext<'_>) -> PluginOutcome + Send + Sync,
{
    fn run(&self, node: &CustomNode<'_>, ctx: &mut PluginContext<'_>) -> PluginOutcome {
        self(node, ctx)
    }
}

/// Runs one `UiAction::Custom` (UI-18).
pub trait UiActionHandler: Send + Sync {
    fn run(&self, args: &[String], ctx: &mut PluginContext<'_>);
}

impl<F> UiActionHandler for F
where
    F: Fn(&[String], &mut PluginContext<'_>) + Send + Sync,
{
    fn run(&self, args: &[String], ctx: &mut PluginContext<'_>) {
        self(args, ctx)
    }
}

/// The runtime part of a plugin. A plugin crate usually implements both
/// this and [`crate::Plugin`] (editor side: node types, field schema,
/// validation) on the same type.
pub trait RuntimePlugin: Send + Sync + 'static {
    fn manifest(&self) -> PluginManifest;

    /// Registers node and UI action handlers.
    fn register_runtime(&self, registry: &mut RuntimeRegistry);
}

/// The handlers of every installed runtime plugin. Implements
/// [`PluginHost`], which is what the VM / player take.
#[derive(Default)]
pub struct RuntimeRegistry {
    current: String,
    plugins: Vec<PluginManifest>,
    nodes: BTreeMap<(String, String), Arc<dyn NodeHandler>>,
    actions: BTreeMap<String, Arc<dyn UiActionHandler>>,
    errors: Vec<PluginError>,
}

impl std::fmt::Debug for RuntimeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeRegistry")
            .field("plugins", &self.plugins)
            .field("nodes", &self.nodes.keys().collect::<Vec<_>>())
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a plugin after checking its manifest (API version, id,
    /// duplicates). Registration errors (an element registered twice) are
    /// reported here too.
    pub fn install(&mut self, plugin: &dyn RuntimePlugin) -> Result<(), PluginError> {
        let manifest = plugin.manifest();
        check_manifest(&manifest, self.plugins.iter().any(|p| p.id == manifest.id))?;
        self.current = manifest.id.clone();
        self.plugins.push(manifest);
        plugin.register_runtime(self);
        match self.errors.drain(..).next() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Installed plugins, in order.
    pub fn plugins(&self) -> &[PluginManifest] {
        &self.plugins
    }

    pub fn has_plugin(&self, id: &str) -> bool {
        self.plugins.iter().any(|p| p.id == id)
    }

    /// Handles custom nodes `kind` of the plugin being installed.
    pub fn register_node(&mut self, kind: &str, handler: impl NodeHandler + 'static) {
        let key = (self.current.clone(), kind.to_owned());
        if self.nodes.contains_key(&key) {
            self.errors
                .push(PluginError::DuplicateElement(format!("{}.{kind}", self.current)));
            return;
        }
        self.nodes.insert(key, Arc::new(handler));
    }

    /// Handles `UiAction::Custom { name: "<plugin id>.<action>", .. }`.
    pub fn register_action(&mut self, action: &str, handler: impl UiActionHandler + 'static) {
        let key = format!("{}.{action}", self.current);
        if self.actions.contains_key(&key) {
            self.errors.push(PluginError::DuplicateElement(key));
            return;
        }
        self.actions.insert(key, Arc::new(handler));
    }

    /// Full names of the registered UI actions.
    pub fn actions(&self) -> impl Iterator<Item = &str> {
        self.actions.keys().map(String::as_str)
    }

    pub fn handles_node(&self, plugin_id: &str, kind: &str) -> bool {
        self.nodes
            .contains_key(&(plugin_id.to_owned(), kind.to_owned()))
    }

    /// Custom nodes of `story` no installed plugin runs (`(plugin, kind)`,
    /// sorted, deduplicated): an export refuses them, Play skips them.
    pub fn unhandled_nodes(&self, story: &Story) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = story
            .scenes
            .iter()
            .flat_map(|s| &s.nodes)
            .filter_map(|n| match &n.kind {
                NodeKind::Custom {
                    plugin_id, kind, ..
                } if !self.handles_node(plugin_id, kind) => {
                    Some((plugin_id.clone(), kind.clone()))
                }
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Shares the registry as the VM's plugin host.
    pub fn into_host(self) -> SharedPluginHost {
        Arc::new(self)
    }
}

impl PluginHost for RuntimeRegistry {
    fn run_node(
        &self,
        plugin_id: &str,
        kind: &str,
        version: u32,
        data: &CustomData,
        ctx: &mut PluginContext<'_>,
    ) -> Handled<PluginOutcome> {
        match self.nodes.get(&(plugin_id.to_owned(), kind.to_owned())) {
            Some(h) => Handled::Done(h.run(
                &CustomNode {
                    kind,
                    version,
                    data,
                },
                ctx,
            )),
            None => Handled::Unhandled,
        }
    }

    fn run_action(&self, name: &str, args: &[String], ctx: &mut PluginContext<'_>) -> Handled<()> {
        match self.actions.get(name) {
            Some(h) => {
                h.run(args, ctx);
                Handled::Done(())
            }
            None => Handled::Unhandled,
        }
    }

    fn plugin_ids(&self) -> Vec<String> {
        self.plugins.iter().map(|p| p.id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{Node, NodeId, Scene, SceneId, Value, VarDecl};
    use orbit_vm::{StepResult, Vm};

    use super::*;
    use crate::API_VERSION;

    struct Counter;

    impl RuntimePlugin for Counter {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("t.counter", "Counter", "1")
        }
        fn register_runtime(&self, reg: &mut RuntimeRegistry) {
            reg.register_node("add", |node: &CustomNode<'_>, ctx: &mut PluginContext<'_>| {
                let by = node.data.get("by").and_then(CustomData::as_int).unwrap_or(1);
                let n = match ctx.var("n") {
                    Some(Value::Int(i)) => i,
                    _ => 0,
                };
                if ctx.set("n", n + by).is_err() {
                    return PluginOutcome::Continue;
                }
                ctx.emit("added", CustomData::Int(by));
                PluginOutcome::Continue
            });
            reg.register_node("pause", |_: &CustomNode<'_>, _: &mut PluginContext<'_>| {
                PluginOutcome::Wait(0.5)
            });
            reg.register_node("bad", |_: &CustomNode<'_>, ctx: &mut PluginContext<'_>| {
                let _ = ctx.set("n", 99);
                let _ = ctx.set("missing", 1);
                PluginOutcome::Continue
            });
            reg.register_action("reset", |_: &[String], ctx: &mut PluginContext<'_>| {
                let _ = ctx.set("n", 0);
            });
        }
    }

    fn custom(kind: &str, data: CustomData) -> Node {
        Node {
            id: NodeId::new(),
            kind: NodeKind::Custom {
                plugin_id: "t.counter".into(),
                kind: kind.into(),
                version: 1,
                data,
            },
        }
    }

    fn story(nodes: Vec<Node>) -> Story {
        let mut story = Story::new("t");
        story.variables.push(VarDecl::game("n", Value::Int(0)));
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes,
        });
        story
    }

    fn vm(story: Story) -> Vm {
        let mut reg = RuntimeRegistry::new();
        reg.install(&Counter).unwrap();
        let mut vm = Vm::from_start(story).unwrap();
        vm.set_plugin_host(Some(reg.into_host()));
        vm
    }

    #[test]
    fn nodes_write_variables_and_emit_events() {
        let by3 = CustomData::Map(vec![("by".into(), CustomData::Int(3))]);
        let mut vm = vm(story(vec![custom("add", by3.clone()), custom("add", by3)]));
        assert_eq!(vm.step(), StepResult::End);
        assert_eq!(vm.variable("n"), Some(&Value::Int(6)));
        let events = vm.take_events();
        assert!(events.iter().any(|e| matches!(
            e,
            orbit_vm::PresentationEvent::Custom { kind, .. } if kind == "added"
        )));
        assert!(vm.take_warnings().is_empty());
    }

    #[test]
    fn a_node_can_block() {
        let mut vm = vm(story(vec![
            custom("pause", CustomData::Null),
            custom("add", CustomData::Null),
        ]));
        assert_eq!(vm.step(), StepResult::WaitTime(0.5));
        assert_eq!(vm.variable("n"), Some(&Value::Int(0)));
        assert_eq!(vm.step(), StepResult::End);
        assert_eq!(vm.variable("n"), Some(&Value::Int(1)));
    }

    #[test]
    fn a_failing_node_applies_nothing() {
        let mut vm = vm(story(vec![custom("bad", CustomData::Null)]));
        assert_eq!(vm.step(), StepResult::End);
        assert_eq!(vm.variable("n"), Some(&Value::Int(0)));
        assert!(!vm.take_warnings().is_empty());
    }

    #[test]
    fn unknown_nodes_are_skipped_with_a_warning() {
        let mut vm = vm(story(vec![custom("nope", CustomData::Null)]));
        assert_eq!(vm.step(), StepResult::End);
        assert!(vm.take_warnings()[0].contains("skipped"));
        let mut reg = RuntimeRegistry::new();
        reg.install(&Counter).unwrap();
        assert_eq!(
            reg.unhandled_nodes(&story(vec![custom("nope", CustomData::Null)])),
            vec![("t.counter".to_owned(), "nope".to_owned())]
        );
    }

    #[test]
    fn ui_actions_run_by_full_name() {
        let mut vm = vm(story(vec![custom("add", CustomData::Null)]));
        vm.step();
        assert!(vm.run_ui_action("t.counter.reset", &[]));
        assert_eq!(vm.variable("n"), Some(&Value::Int(0)));
        assert!(!vm.run_ui_action("t.counter.nope", &[]));
    }

    #[test]
    fn api_version_and_duplicates_are_checked() {
        struct Old;
        impl RuntimePlugin for Old {
            fn manifest(&self) -> PluginManifest {
                PluginManifest {
                    api_version: API_VERSION - 1,
                    ..PluginManifest::new("t.old", "Old", "1")
                }
            }
            fn register_runtime(&self, _: &mut RuntimeRegistry) {}
        }
        let mut reg = RuntimeRegistry::new();
        assert!(matches!(
            reg.install(&Old),
            Err(PluginError::IncompatibleApi { .. })
        ));
        reg.install(&Counter).unwrap();
        assert!(matches!(
            reg.install(&Counter),
            Err(PluginError::DuplicatePlugin(_))
        ));
    }
}
