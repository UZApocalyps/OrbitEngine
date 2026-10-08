//! Console: the journal of the editor (project events, warnings, errors,
//! `log` output).

use egui::{Align2, FontId, Id, RichText, Ui};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, Level, PanelDef, Plugin,
    PluginManifest, icons,
};

use super::ids;
use super::ui_util::{danger, faint, icon_button, warning, weak};

pub struct ConsolePlugin;

const ROW_HEIGHT: f32 = 20.0;

impl Plugin for ConsolePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::CONSOLE, "Console", "0.1.0")
    }
}

impl EditorPlugin for ConsolePlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "console",
            "console.title",
            icons::TERMINAL_WINDOW,
            DockSlot::Bottom,
            panel,
        ));
        reg.action(
            ActionDef::new("clear", "console.clear", |c| {
                c.state_mut().console.clear();
            })
            .icon(icons::BROOM)
            .menu_group(super::GROUP_DIAG)
            .category("category.view")
            .menu("view", 200),
        );
    }
}

/// Which levels are shown: (info, warnings, errors).
type Filter = (bool, bool, bool);

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let id = Id::new("orbit_console_filter");
    let mut filter: Filter = ui.data(|d| d.get_temp(id)).unwrap_or((true, true, true));

    let (mut infos, mut warns, mut errors) = (0, 0, 0);
    for entry in ctx.state().console.entries() {
        match entry.level {
            Level::Info => infos += 1,
            Level::Warn => warns += 1,
            Level::Error => errors += 1,
        }
    }
    ui.horizontal(|ui| {
        ui.toggle_value(&mut filter.0, format!("{}  {infos}", icons::INFO));
        ui.toggle_value(&mut filter.1, format!("{}  {warns}", icons::WARNING));
        ui.toggle_value(
            &mut filter.2,
            format!("{}  {errors}", icons::WARNING_CIRCLE),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, icons::BROOM, &ctx.t("console.clear")).clicked() {
                ctx.state_mut().console.clear();
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(id, filter));
    ui.add_space(2.0);

    let entries: Vec<_> = ctx
        .state()
        .console
        .entries()
        .iter()
        .filter(|e| match e.level {
            Level::Info => filter.0,
            Level::Warn => filter.1,
            Level::Error => filter.2,
        })
        .collect();
    if entries.is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ctx.t("console.empty")).color(faint(ui)));
        });
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show_rows(ui, ROW_HEIGHT, entries.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for entry in &entries[range] {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                    egui::Sense::hover(),
                );
                let painter = ui.painter_at(rect);
                let mid = rect.center().y;
                let (icon, color) = match entry.level {
                    Level::Info => (icons::INFO, weak(ui)),
                    Level::Warn => (icons::WARNING, warning(ui)),
                    Level::Error => (icons::WARNING_CIRCLE, danger(ui)),
                };
                let minutes = (entry.time / 60.0) as u32;
                painter.text(
                    egui::pos2(rect.left() + 4.0, mid),
                    Align2::LEFT_CENTER,
                    format!("{minutes:02}:{:04.1}", entry.time % 60.0),
                    FontId::proportional(11.5),
                    faint(ui),
                );
                painter.text(
                    egui::pos2(rect.left() + 64.0, mid),
                    Align2::LEFT_CENTER,
                    icon,
                    FontId::proportional(13.0),
                    color,
                );
                painter.text(
                    egui::pos2(rect.left() + 86.0, mid),
                    Align2::LEFT_CENTER,
                    // One line per entry: the first line of a multi-line message.
                    entry.message.lines().next().unwrap_or_default(),
                    FontId::proportional(12.5),
                    if entry.level == Level::Info {
                        ui.visuals().text_color()
                    } else {
                        color
                    },
                );
            }
        });
}
