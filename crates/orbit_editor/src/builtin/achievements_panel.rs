//! The "Achievements" panel (M7-05): the achievements of the story (title,
//! description, hidden flag, icon), created, edited and deleted through
//! `orbit_project::achievements`, and the `Unlock` nodes that grant them.

use egui::{RichText, Ui};
use orbit_core::{AchievementId, AssetKind};
use orbit_editor_api::fields::{self, asset_picker};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, Location, PanelDef, Plugin,
    PluginManifest, Selection, UiText, icons,
};
use orbit_project::{Command, achievements as project_achievements};

use super::edit;
use super::ids;
use super::nodeinfo::{kind_label, summary};
use super::ui_util::{danger, faint, strong, weak};

pub struct AchievementsPlugin;

impl Plugin for AchievementsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::ACHIEVEMENTS, "Achievements", "0.1.0")
    }
}

impl EditorPlugin for AchievementsPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "achievements",
            "ach.panel.title",
            icons::TROPHY,
            DockSlot::Right,
            panel,
        ));
    }
}

#[derive(Clone, Default)]
struct State {
    selected: Option<AchievementId>,
    confirm_delete: bool,
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !ctx.has_project() {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ctx.t("ach.panel.no_project")).color(weak(ui)));
        });
        return;
    }
    let state_id = egui::Id::new("orbit_achievements_panel");
    let mut st: State = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let Some(story) = ctx.story().cloned() else { return };
    if st
        .selected
        .is_some_and(|s| !story.achievements.iter().any(|a| a.id == s))
    {
        st.selected = None;
    }
    if st.selected.is_none() {
        st.selected = story.achievements.first().map(|a| a.id);
    }
    let mut reveal_node = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(ui.t("ach.panel.hint")).small().color(weak(ui)));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for a in &story.achievements {
                    let name = story.text(a.title_id).unwrap_or("?");
                    let label = if a.hidden {
                        format!("{} {name}", icons::EYE_SLASH)
                    } else {
                        name.to_owned()
                    };
                    if ui.selectable_label(st.selected == Some(a.id), label).clicked() {
                        st.selected = Some(a.id);
                        st.confirm_delete = false;
                    }
                }
                if ui
                    .button(format!("{}  {}", icons::PLUS, ui.t("ach.panel.add")))
                    .on_hover_text(ui.t("ach.panel.add.tip"))
                    .clicked()
                {
                    let title = ui.t("ach.panel.new_title");
                    if let Some(id) = ctx.ops_or_toast(|p| {
                        project_achievements::add_achievement(p, &title, "", false, None)
                    }) {
                        st.selected = Some(id);
                    }
                }
            });
            if story.achievements.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(ui.t("ach.panel.empty")).color(faint(ui)));
                return;
            }
            let Some(id) = st.selected else { return };
            let Some(a) = story.achievements.iter().find(|a| a.id == id).cloned() else {
                return;
            };
            ui.separator();

            // ---- title and description (strings of the source table)
            let title = story.text(a.title_id).unwrap_or_default().to_owned();
            let description = story.text(a.description_id).unwrap_or_default().to_owned();
            let mut done = false;
            fields::field(ui, &ui.t("ach.field.title"), |ui| {
                if let Some(t) = fields::text_field(
                    ui,
                    ("ach_title", id),
                    &title,
                    &ui.t("ach.field.title.hint"),
                    1,
                    &mut done,
                ) && !t.trim().is_empty()
                {
                    let command = Command::SetString {
                        id: a.title_id,
                        text: Some(t),
                    };
                    if let Err(e) = ctx.exec_merged(command, &format!("ach-title:{id}")) {
                        ctx.error(e.to_string());
                    }
                }
            });
            fields::field(ui, &ui.t("ach.field.description"), |ui| {
                if let Some(t) = fields::text_field(
                    ui,
                    ("ach_description", id),
                    &description,
                    &ui.t("ach.field.description.hint"),
                    3,
                    &mut done,
                ) {
                    let command = Command::SetString {
                        id: a.description_id,
                        text: Some(t),
                    };
                    if let Err(e) = ctx.exec_merged(command, &format!("ach-description:{id}")) {
                        ctx.error(e.to_string());
                    }
                }
            });
            if done {
                ctx.end_merge();
            }

            // ---- hidden
            let mut hidden = a.hidden;
            if ui
                .checkbox(&mut hidden, ui.t("ach.field.hidden"))
                .on_hover_text(ui.t("ach.field.hidden.tip"))
                .changed()
            {
                ctx.ops_or_toast(|p| {
                    project_achievements::update_achievement(p, id, None, None, Some(hidden), None)
                });
            }

            // ---- icon, and the icon shown while locked
            for locked in [false, true] {
                let current = if locked { a.locked_icon } else { a.icon };
                let mut change: Option<Option<orbit_core::AssetId>> = None;
                let mut add = false;
                let label = if locked { ui.t("ach.field.locked_icon") } else { ui.t("ach.field.icon") };
                fields::field(ui, &label, |ui| {
                    match current {
                        Some(icon) => {
                            let key = if locked { "ach_locked_icon" } else { "ach_icon" };
                            if let Some(new) = asset_picker(ui, (key, id), &story, AssetKind::Image, icon) {
                                change = Some(Some(new));
                            }
                        }
                        None => {
                            let none = if locked { "ach.field.no_locked_icon" } else { "ach.field.no_icon" };
                            ui.label(RichText::new(ui.t(none)).color(weak(ui)));
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .small_button(format!("{}  {}", icons::FILE_PLUS, ui.t("inspector.asset.add_file")))
                            .clicked()
                        {
                            add = true;
                        }
                        if current.is_some()
                            && ui
                                .small_button(format!("{}  {}", icons::X, ui.t("ach.field.remove_icon")))
                                .clicked()
                        {
                            change = Some(None);
                        }
                    });
                });
                if add && let Some(new) = edit::import_asset_dialog(ctx, AssetKind::Image) {
                    change = Some(Some(new));
                }
                if let Some(icon) = change {
                    ctx.ops_or_toast(|p| {
                        if locked {
                            project_achievements::set_achievement_locked_icon(p, id, icon)
                        } else {
                            project_achievements::update_achievement(p, id, None, None, None, Some(icon))
                        }
                    });
                }
            }

            // ---- where it is unlocked
            ui.add_space(6.0);
            ui.label(strong(ui.t("ach.unlocked_by"), 14.0));
            let users = project_achievements::unlock_nodes(&story, id);
            if users.is_empty() {
                ui.label(
                    RichText::new(ui.t("ach.never_unlocked"))
                        .small()
                        .color(danger(ui)),
                );
            }
            let texts = ctx.translator();
            for n in &users {
                if let Some(node) = story.node(*n) {
                    let label = format!(
                        "{}  {}",
                        kind_label(&texts, &node.kind),
                        summary(&texts, &story, node)
                    );
                    if ui.small_button(label).clicked() {
                        reveal_node = Some(*n);
                    }
                }
            }
            // A new Unlock node after the selection.
            if ui
                .small_button(format!("{}  {}", icons::TROPHY, ui.t("ach.add_unlock")))
                .on_hover_text(ui.t("ach.add_unlock.tip"))
                .clicked()
            {
                if let Some((scene, index)) = super::edit::insertion_point(ctx) {
                    if let Some(node) =
                        ctx.ops_or_toast(|p| project_achievements::insert_unlock(p, scene, index, id))
                    {
                        reveal_node = Some(node);
                    }
                } else {
                    ctx.error(ctx.t("edit.no_scene"));
                }
            }

            // ---- delete
            ui.add_space(8.0);
            if st.confirm_delete {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ui.t("ach.delete.confirm")).color(danger(ui)));
                    if ui.button(ui.t("ach.delete")).clicked() {
                        if ctx
                            .ops_or_toast(|p| project_achievements::delete_achievement(p, id))
                            .is_some()
                        {
                            st.selected = None;
                        }
                        st.confirm_delete = false;
                    }
                    if ui.button(ui.t("common.cancel")).clicked() {
                        st.confirm_delete = false;
                    }
                });
            } else if ui
                .button(format!("{}  {}", icons::TRASH, ui.t("ach.delete")))
                .clicked()
            {
                st.confirm_delete = true;
            }
        });
    if let Some(node) = reveal_node {
        ctx.reveal(Location::Node(node));
        ctx.select(Selection::Node(node));
    }
    ui.data_mut(|d| d.insert_temp(state_id, st));

}
