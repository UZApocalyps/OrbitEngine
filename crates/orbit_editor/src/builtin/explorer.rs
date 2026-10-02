//! Project explorer: scenes and their nodes, virtualized, with the minimal
//! edits that prove the command chain (add / delete a line). The full scene
//! editor is M3c.

use egui::{Align2, FontId, Key, Modifiers, Rect, Ui};
use orbit_core::{NodeId, NodeKind, Story};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, Plugin,
    PluginManifest, Selection, icons,
};

use super::ids;
use super::nodeinfo::{kind_icon, kind_label, summary};
use super::ui_util::{
    accent, faint, icon_button, list_row, primary_button, section_title, success, weak,
};

pub struct ExplorerPlugin;

const ROW_HEIGHT: f32 = 26.0;

impl Plugin for ExplorerPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::EXPLORER, "Explorateur de projet", "0.1.0")
    }
}

impl EditorPlugin for ExplorerPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(orbit_editor_api::PanelDef::new(
            "explorer",
            "Explorateur",
            icons::TREE_STRUCTURE,
            DockSlot::Left,
            panel,
        ));
        reg.action(
            ActionDef::new("add_say", "Ajouter une réplique", add_say)
                .icon(icons::CHAT_TEXT)
                .category("Édition")
                .shortcut(Modifiers::COMMAND, Key::Enter)
                .menu("edit", 200)
                .enabled(|c| c.story().is_some_and(|s| !s.scenes.is_empty())),
        );
        reg.action(
            ActionDef::new("delete", "Supprimer l'élément sélectionné", |c| {
                delete_selected(c);
            })
            .icon(icons::TRASH)
            .category("Édition")
            .shortcut(Modifiers::NONE, Key::Delete)
            .menu("edit", 210)
            .enabled(|c| matches!(c.selection(), Some(Selection::Node(_)))),
        );
    }
}

/// Inserts a new line after the selection (see `edit::add_say`).
pub fn add_say(ctx: &mut EditorContext<'_>) {
    super::edit::add_say(ctx);
}

/// Deletes the selected nodes (see `edit::delete_selection`).
pub fn delete_selected(ctx: &mut EditorContext<'_>) {
    super::edit::delete_selection(ctx);
}

#[derive(Clone, Copy)]
enum Row {
    Scene(usize),
    Node(usize, usize),
}

fn rows(story: &Story, collapsed: &std::collections::HashSet<orbit_core::SceneId>) -> Vec<Row> {
    let mut rows = Vec::with_capacity(story.scenes.iter().map(|s| s.nodes.len() + 1).sum());
    for (si, scene) in story.scenes.iter().enumerate() {
        rows.push(Row::Scene(si));
        if !collapsed.contains(&scene.id) {
            rows.extend((0..scene.nodes.len()).map(|ni| Row::Node(si, ni)));
        }
    }
    rows
}

/// Whether the pointer press that selected a row was on its play button.
fn play_rect_clicked(ui: &Ui, rect: Rect, selected: bool) -> bool {
    selected
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| rect.contains(p))
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !ctx.has_project() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(icons::FOLDER_SIMPLE)
                    .size(30.0)
                    .color(faint(ui)),
            );
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Aucun projet ouvert").color(weak(ui)));
            ui.add_space(10.0);
            if primary_button(ui, "Ouvrir un projet…").clicked() {
                ctx.request(EditorRequest::PickOpenProject);
            }
        });
        return;
    }

    // Toolbar.
    ui.horizontal(|ui| {
        section_title(ui, "Scènes");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_delete = matches!(ctx.selection(), Some(Selection::Node(_)));
            if ui
                .add_enabled_ui(can_delete, |ui| {
                    icon_button(ui, icons::TRASH, "Supprimer (Suppr)")
                })
                .inner
                .clicked()
            {
                ctx.run_action("orbit.explorer.delete");
            }
            if icon_button(ui, icons::PLUS, "Ajouter une réplique (Ctrl+Entrée)").clicked() {
                ctx.run_action("orbit.explorer.add_say");
            }
        });
    });

    let Some(story) = ctx.story() else { return };
    let rows = rows(story, &ctx.state().collapsed_scenes);
    let selection = ctx.selection();
    let playing_node = {
        let p = &ctx.state().preview;
        (p.status != orbit_editor_api::PreviewStatus::Stopped)
            .then_some(p.current_node)
            .flatten()
    };
    // What a click did this frame, applied after the list borrows the story.
    let mut clicked: Option<Selection> = None;
    let mut play_from: Option<NodeId> = None;
    let mut toggled: Option<orbit_core::SceneId> = None;

    // A reveal (problem, search hit, palette) selected something: scroll to it.
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    let serial = ctx.state().reveal_serial;
    let seen_id = egui::Id::new("orbit_explorer_reveal");
    if ui.data(|d| d.get_temp::<u64>(seen_id)).unwrap_or(0) != serial {
        ui.data_mut(|d| d.insert_temp(seen_id, serial));
        let index = rows.iter().position(|row| match *row {
            Row::Scene(si) => selection == Some(Selection::Scene(story.scenes[si].id)),
            Row::Node(si, ni) => selection == Some(Selection::Node(story.scenes[si].nodes[ni].id)),
        });
        if let Some(index) = index {
            area = area.vertical_scroll_offset((index as f32 * ROW_HEIGHT - 80.0).max(0.0));
        }
    }
    area.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let Some(story) = ctx.story() else { return };
        for row in &rows[range] {
            match *row {
                Row::Scene(si) => {
                    let scene = &story.scenes[si];
                    let collapsed = ctx.state().collapsed_scenes.contains(&scene.id);
                    let selected = selection == Some(Selection::Scene(scene.id));
                    let (rect, response) = list_row(ui, ROW_HEIGHT, selected);
                    let painter = ui.painter_at(rect);
                    let mid = rect.center().y;
                    let caret = if collapsed {
                        icons::CARET_RIGHT
                    } else {
                        icons::CARET_DOWN
                    };
                    painter.text(
                        egui::pos2(rect.left() + 12.0, mid),
                        Align2::CENTER_CENTER,
                        caret,
                        FontId::proportional(12.0),
                        weak(ui),
                    );
                    painter.text(
                        egui::pos2(rect.left() + 32.0, mid),
                        Align2::CENTER_CENTER,
                        if collapsed {
                            icons::FOLDER_SIMPLE
                        } else {
                            icons::FOLDER_OPEN
                        },
                        FontId::proportional(15.0),
                        accent(ui),
                    );
                    painter.text(
                        egui::pos2(rect.left() + 48.0, mid),
                        Align2::LEFT_CENTER,
                        &scene.name,
                        crate::theme::bold(13.5),
                        ui.visuals().text_color(),
                    );
                    painter.text(
                        egui::pos2(rect.right() - 10.0, mid),
                        Align2::RIGHT_CENTER,
                        scene.nodes.len().to_string(),
                        FontId::proportional(11.5),
                        faint(ui),
                    );
                    if response.clicked() {
                        clicked = Some(Selection::Scene(scene.id));
                        // A click on the caret area folds, elsewhere selects.
                        if response
                            .interact_pointer_pos()
                            .is_some_and(|p| p.x < rect.left() + 24.0)
                        {
                            toggled = Some(scene.id);
                        }
                    }
                    if response.double_clicked() {
                        toggled = Some(scene.id);
                    }
                }
                Row::Node(si, ni) => {
                    let node = &story.scenes[si].nodes[ni];
                    let selected = selection == Some(Selection::Node(node.id));
                    let (rect, response) = list_row(ui, ROW_HEIGHT, selected);
                    let painter = ui.painter_at(rect);
                    let mid = rect.center().y;
                    let x0 = rect.left() + 30.0;
                    // The line the preview is playing.
                    if playing_node == Some(node.id) {
                        painter.rect_filled(
                            rect.shrink2(egui::vec2(2.0, 0.0)),
                            egui::CornerRadius::same(5),
                            success().gamma_multiply(0.18),
                        );
                        painter.text(
                            egui::pos2(rect.left() + 12.0, mid),
                            Align2::CENTER_CENTER,
                            icons::PLAY,
                            FontId::proportional(11.0),
                            success(),
                        );
                    }
                    // "Play from here" on the selected row.
                    let play_rect = Rect::from_center_size(
                        egui::pos2(rect.right() - 16.0, mid),
                        egui::vec2(22.0, 20.0),
                    );
                    if selected {
                        let play = ui
                            .interact(
                                play_rect,
                                ui.id().with(("play_here", node.id)),
                                egui::Sense::click(),
                            )
                            .on_hover_text("Jouer depuis ici (F7)");
                        painter.text(
                            play_rect.center(),
                            Align2::CENTER_CENTER,
                            icons::CROSSHAIR,
                            FontId::proportional(14.0),
                            if play.hovered() { accent(ui) } else { weak(ui) },
                        );
                        if play.clicked() {
                            play_from = Some(node.id);
                        }
                    }
                    response.context_menu(|ui| {
                        if ui
                            .button(format!("{}  Jouer depuis ici", icons::CROSSHAIR))
                            .clicked()
                        {
                            play_from = Some(node.id);
                            ui.close();
                        }
                    });
                    painter.text(
                        egui::pos2(x0 + 8.0, mid),
                        Align2::CENTER_CENTER,
                        kind_icon(&node.kind),
                        FontId::proportional(14.0),
                        if selected { accent(ui) } else { weak(ui) },
                    );
                    let text = summary(story, node);
                    let is_say = matches!(node.kind, NodeKind::Say { .. });
                    let mut x = x0 + 24.0;
                    if !is_say || text.is_empty() {
                        let g = painter.layout_no_wrap(
                            kind_label(&node.kind).to_owned(),
                            FontId::proportional(13.0),
                            if text.is_empty() {
                                ui.visuals().text_color()
                            } else {
                                weak(ui)
                            },
                        );
                        let w = g.size().x;
                        painter.galley(
                            egui::pos2(x, mid - g.size().y / 2.0),
                            g,
                            egui::Color32::WHITE,
                        );
                        x += w + 8.0;
                    }
                    if !text.is_empty() {
                        let clip = egui::Rect::from_min_max(
                            egui::pos2(x, rect.top()),
                            egui::pos2(rect.right() - 6.0, rect.bottom()),
                        );
                        let g = painter.layout_no_wrap(
                            text,
                            FontId::proportional(13.0),
                            ui.visuals().text_color(),
                        );
                        painter.with_clip_rect(clip).galley(
                            egui::pos2(x, mid - g.size().y / 2.0),
                            g,
                            egui::Color32::WHITE,
                        );
                    }
                    if response.clicked() && !play_rect_clicked(ui, play_rect, selected) {
                        clicked = Some(Selection::Node(node.id));
                    }
                }
            }
        }
    });

    if let Some(selection) = clicked {
        ctx.select(selection);
    }
    if let Some(id) = play_from {
        ctx.select(Selection::Node(id));
        ctx.play_from(id);
        ctx.request(EditorRequest::ShowPanel(super::panels::PREVIEW.to_owned()));
    }
    if let Some(id) = toggled {
        let collapsed = &mut ctx.state_mut().collapsed_scenes;
        if !collapsed.remove(&id) {
            collapsed.insert(id);
        }
    }
}
