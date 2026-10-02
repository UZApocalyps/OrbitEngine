//! Inspector: edits the selected element. Inspectors are registered per
//! node kind (`InspectorDef`); the one for `Say` proves the command chain
//! (live text edit, one undo step per editing session). Kinds without an
//! inspector fall back to a read-only summary until M3c.

use egui::{Align, FontId, Frame, Layout, Margin, RichText, TextEdit, Ui};
use orbit_core::{CharacterId, CustomData, NodeId, NodeKind};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, InspectorDef, PanelDef, Plugin,
    PluginManifest, Selection, icons,
};
use orbit_project::Command;

use super::ids;
use super::nodeinfo::{kind_icon, kind_key, kind_label, summary};
use super::ui_util::{accent, card, danger, faint, section_title, strong, warning, weak};

pub struct InspectorPlugin;

impl Plugin for InspectorPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::INSPECTOR, "Inspecteur", "0.1.0")
    }
}

impl EditorPlugin for InspectorPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "inspector",
            "Inspecteur",
            icons::SLIDERS_HORIZONTAL,
            DockSlot::Right,
            panel,
        ));
        reg.inspector(InspectorDef::new("Say", say_inspector));
    }
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match ctx.selection() {
            None => empty_state(ui, "Sélectionnez un élément dans l'explorateur."),
            Some(Selection::Scene(id)) => scene_header(ui, ctx, id),
            Some(Selection::Node(id)) => node_panel(ui, ctx, id),
        });
}

fn empty_state(ui: &mut Ui, text: &str) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(icons::CURSOR_CLICK)
                .size(28.0)
                .color(faint(ui)),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(text).color(weak(ui)));
    });
}

fn scene_header(ui: &mut Ui, ctx: &EditorContext<'_>, id: orbit_core::SceneId) {
    let Some(scene) = ctx.story().and_then(|s| s.scene(id)) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(icons::FOLDER_OPEN)
                .size(18.0)
                .color(accent(ui)),
        );
        ui.label(strong(&scene.name, 16.0));
    });
    ui.label(RichText::new("Scène").color(weak(ui)));
    ui.add_space(8.0);
    section_title(ui, "Contenu");
    ui.label(format!("{} nœud(s)", scene.nodes.len()));
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "Le renommage et les réglages de scène arrivent avec l'éditeur de scène (M3c).",
        )
        .small()
        .color(faint(ui)),
    );
}

fn node_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId) {
    let Some(node) = ctx.story().and_then(|s| s.node(id)).cloned() else {
        empty_state(ui, "L'élément sélectionné n'existe plus.");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(kind_icon(&node.kind))
                .size(18.0)
                .color(accent(ui)),
        );
        ui.label(strong(kind_label(&node.kind), 16.0));
    });
    ui.label(RichText::new(id.to_string()).small().color(faint(ui)));
    ui.add_space(8.0);

    // Plugin node whose plugin is absent: kept as is, read-only.
    if let NodeKind::Custom { plugin_id, .. } = &node.kind
        && !ctx.registry().project.has_plugin(plugin_id)
    {
        let color = warning(ui);
        Frame::new()
            .fill(color.gamma_multiply(0.12))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} Le plugin « {plugin_id} » n'est pas installé : ce nœud est conservé tel quel, en lecture seule.",
                        icons::LOCK_SIMPLE
                    ))
                    .color(color),
                );
            });
        ui.add_space(8.0);
    }

    let key = kind_key(&node.kind);
    match ctx.registry().inspector_for(&key) {
        Some(inspector) => (inspector.ui)(ui, ctx, id),
        None => generic_inspector(ui, ctx, id),
    }

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(4.0);
    let delete = egui::Button::new(
        RichText::new(format!("{}  Supprimer le nœud", icons::TRASH)).color(danger(ui)),
    );
    if ui.add(delete).clicked() {
        ctx.run_action("orbit.explorer.delete");
    }
}

/// Read-only view of any node kind.
fn generic_inspector(ui: &mut Ui, ctx: &EditorContext<'_>, id: NodeId) {
    let Some(story) = ctx.story() else { return };
    let Some(node) = story.node(id) else { return };
    section_title(ui, "Résumé");
    let line = summary(story, node);
    ui.label(if line.is_empty() {
        "—".to_owned()
    } else {
        line
    });
    if let NodeKind::Custom {
        kind,
        version,
        data,
        ..
    } = &node.kind
    {
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!("type « {kind} », données v{version}"))
                .small()
                .color(weak(ui)),
        );
        card(ui, |ui| show_data(ui, data));
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new("L'édition de ce type de nœud arrive avec l'éditeur de scène (M3c).")
            .small()
            .color(faint(ui)),
    );
}

fn show_data(ui: &mut Ui, data: &CustomData) {
    match data {
        CustomData::Null => {
            ui.label("null");
        }
        CustomData::Bool(b) => {
            ui.label(b.to_string());
        }
        CustomData::Int(i) => {
            ui.label(i.to_string());
        }
        CustomData::Float(f) => {
            ui.label(f.to_string());
        }
        CustomData::Str(s) => {
            ui.label(format!("« {s} »"));
        }
        CustomData::List(items) => {
            for (i, item) in items.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("[{i}]")).color(weak(ui)));
                    show_data(ui, item);
                });
            }
        }
        CustomData::Map(entries) => {
            for (key, value) in entries {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{key} :")).color(weak(ui)));
                    show_data(ui, value);
                });
            }
        }
    }
}

/// Inspector of a `Say` node: speaker and text. Text edits are live (one
/// command per change, merged into one undo entry per editing session).
fn say_inspector(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId) {
    let Some(story) = ctx.story() else { return };
    let Some(NodeKind::Say { speaker, text_id }) = story.node(id).map(|n| &n.kind) else {
        return;
    };
    let (speaker, text_id) = (*speaker, *text_id);
    let text = story.text(text_id).unwrap_or_default().to_owned();
    let characters: Vec<(CharacterId, String)> = story
        .characters
        .iter()
        .map(|c| (c.id, story.text(c.name_id).unwrap_or("?").to_owned()))
        .collect();
    let speaker_name = |s: Option<CharacterId>| match s {
        None => "Narrateur".to_owned(),
        Some(c) => characters
            .iter()
            .find(|(id, _)| *id == c)
            .map_or_else(|| "?".to_owned(), |(_, n)| n.clone()),
    };

    section_title(ui, "Personnage");
    let mut chosen = speaker;
    egui::ComboBox::from_id_salt(("say_speaker", id))
        .selected_text(speaker_name(speaker))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut chosen, None, "Narrateur");
            for (c, name) in &characters {
                ui.selectable_value(&mut chosen, Some(*c), name);
            }
        });
    if chosen != speaker {
        let command = Command::SetSay {
            id,
            speaker: chosen,
            text: text.clone(),
            expected_version: ctx.node_version(id),
        };
        ctx.exec_or_toast(command);
    }

    ui.add_space(8.0);
    section_title(ui, "Texte");
    let mut buffer = text.clone();
    let response = ui.add(
        TextEdit::multiline(&mut buffer)
            .desired_width(f32::INFINITY)
            .desired_rows(5)
            .font(FontId::proportional(14.0))
            .hint_text("Écrivez la réplique…")
            .margin(Margin::same(8)),
    );
    if buffer != text {
        let command = Command::SetSay {
            id,
            speaker,
            text: buffer,
            expected_version: ctx.node_version(id),
        };
        let key = format!("say-text:{id}");
        if let Err(e) = ctx.exec_merged(command, &key) {
            ctx.error(e.to_string());
        }
    }
    if response.lost_focus() {
        ctx.end_merge();
    }
    ui.add_space(4.0);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.label(
            RichText::new(format!("{} caractères", text.chars().count()))
                .small()
                .color(faint(ui)),
        );
    });
}
