//! The trial plugin: a panel, an action, a custom node type and a project
//! hook, registered through the public plugin API only. It exists to prove
//! that the extension points work (and is exercised by the tests); its panel
//! is hidden by default (View menu or palette).
//!
//! It also shows how a plugin brings its own translations: it registers an
//! English and a French table (`reg.texts`) under its own key prefix
//! (`orbit.sample.`), and every label it gives the registry is such a key.

use std::cell::Cell;
use std::rc::Rc;

use orbit_core::{CustomData, Node, NodeId, NodeKind};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, Lang, PanelDef, Plugin,
    PluginManifest, ProjectRegistry, Selection, icons,
};
use orbit_plugin_api::{NodeTypeDef, ProjectEvent};
use orbit_project::Command;

use super::ids;

/// The plugin's English and French texts (RON maps, see `orbit_editor_api::i18n`).
pub const TEXTS_EN: &str = include_str!("../../locales/sample.en.ron");
pub const TEXTS_FR: &str = include_str!("../../locales/sample.fr.ron");

/// Kind of the custom node this plugin defines.
pub const NOTE_KIND: &str = "note";

#[derive(Default)]
pub struct SamplePlugin {
    /// Project changes seen by the hook (shown in the panel).
    changes: Rc<Cell<u32>>,
}

fn note_default() -> CustomData {
    CustomData::Map(vec![("text".into(), CustomData::Str("Note".into()))])
}

fn note_validate(data: &CustomData) -> Vec<String> {
    match data.get("text").and_then(CustomData::as_str) {
        Some(_) => Vec::new(),
        // A key: the editor translates what a validator returns when it can.
        None => vec!["orbit.sample.note_no_text".to_owned()],
    }
}

impl Plugin for SamplePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::SAMPLE, "Sample plugin", "0.1.0")
    }

    fn register(&self, reg: &mut ProjectRegistry) {
        let _ = reg.register_node_type(NodeTypeDef {
            kind: NOTE_KIND.to_owned(),
            label: "orbit.sample.note".to_owned(),
            icon: icons::NOTE.to_owned(),
            default_data: note_default,
            validate: Some(note_validate),
        });
        let changes = self.changes.clone();
        reg.on_project_event(move |event| {
            if matches!(event, ProjectEvent::Changed(_)) {
                changes.set(changes.get() + 1);
            }
        });
    }
}

/// Inserts a custom note node after the selection.
fn insert_note(ctx: &mut EditorContext<'_>) {
    let Some(story) = ctx.story() else { return };
    let (scene, index) = match ctx.selection() {
        Some(Selection::Node(id)) => match story.locate(id) {
            Some((s, n)) => (story.scenes[s].id, n + 1),
            None => return,
        },
        _ => match story.scenes.last() {
            Some(sc) => (sc.id, sc.nodes.len()),
            None => return,
        },
    };
    let data = CustomData::Map(vec![(
        "text".into(),
        CustomData::Str(ctx.t("orbit.sample.note_default")),
    )]);
    let node = Node {
        id: NodeId::new(),
        kind: NodeKind::Custom {
            plugin_id: ids::SAMPLE.to_owned(),
            kind: NOTE_KIND.to_owned(),
            version: 1,
            data,
        },
    };
    let id = node.id;
    if ctx.exec_or_toast(Command::InsertNode { scene, index, node }) {
        ctx.select(Selection::Node(id));
    }
}

impl EditorPlugin for SamplePlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.texts(Lang::En, TEXTS_EN);
        reg.texts(Lang::Fr, TEXTS_FR);
        let changes = self.changes.clone();
        reg.panel(PanelDef::new(
            "panel",
            "orbit.sample.title",
            icons::FLASK,
            DockSlot::Hidden,
            move |ui, ctx| {
                ui.heading(ctx.t("orbit.sample.title"));
                ui.label(ctx.t("orbit.sample.intro"));
                ui.add_space(6.0);
                ui.label(ctx.tf("orbit.sample.changes", &[("count", &changes.get())]));
                ui.add_space(6.0);
                if ui
                    .add_enabled(
                        ctx.has_project(),
                        egui::Button::new(ctx.t("orbit.sample.insert_button")),
                    )
                    .clicked()
                {
                    insert_note(ctx);
                }
            },
        ));
        reg.action(
            ActionDef::new("hello", "orbit.sample.hello", |ctx| {
                let message = ctx.t("orbit.sample.hello_toast");
                ctx.info(message);
            })
            .icon(icons::FLASK)
            .category("category.tools")
            .menu("tools", 100),
        );
        reg.action(
            ActionDef::new("insert_note", "orbit.sample.insert_note", insert_note)
                .icon(icons::NOTE)
                .category("category.tools")
                .menu("tools", 110)
                .enabled(|c| c.story().is_some_and(|s| !s.scenes.is_empty())),
        );
    }
}
