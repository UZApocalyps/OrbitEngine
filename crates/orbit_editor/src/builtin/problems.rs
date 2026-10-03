//! Problems panel: the live validation findings of the project (errors and
//! warnings) with their location. A click selects and reveals the faulty
//! node (explorer, scene editor, inspector) or opens the definition.

use egui::{Align2, FontId, Ui};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, Level, Location, PanelDef,
    Plugin, PluginManifest, Problem, icons,
};

use super::ids;
use super::reveal::{describe, reveal};
use super::ui_util::{danger, faint, list_row, warning, weak};

pub struct ProblemsPlugin;

const ROW_HEIGHT: f32 = 42.0;

impl Plugin for ProblemsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::PROBLEMS, "Problèmes", "0.1.0")
    }
}

impl EditorPlugin for ProblemsPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "problems",
            "problems.title",
            icons::WARNING_CIRCLE,
            DockSlot::Bottom,
            panel,
        ));
        reg.action(
            ActionDef::new("next", "problems.next", |c| {
                let selected = c.selection();
                let problems = c.state().diagnostics.problems.clone();
                let current = problems.iter().position(|p| {
                    matches!(
                        (&p.location, selected),
                        (Some(Location::Node(n)), Some(orbit_editor_api::Selection::Node(s))) if *n == s
                    )
                });
                let next = problems
                    .iter()
                    .enumerate()
                    .cycle()
                    .skip(current.map_or(0, |i| i + 1))
                    .take(problems.len())
                    .find_map(|(_, p)| p.location.clone());
                if let Some(location) = next {
                    reveal(c, location);
                }
            })
            .icon(icons::ARROW_FAT_LINE_DOWN)
            .category("category.edit")
            .shortcut(egui::Modifiers::NONE, egui::Key::F8)
            .menu("edit", 400)
            .enabled(|c| !c.state().diagnostics.problems.is_empty()),
        );
    }
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !ctx.has_project() {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(ctx.t("problems.no_project")).color(weak(ui)));
        });
        return;
    }
    let id = egui::Id::new("orbit_problems_filter");
    let mut filter: (bool, bool) = ui.data(|d| d.get_temp(id)).unwrap_or((true, true));
    let (errors, warnings) = {
        let d = &ctx.state().diagnostics;
        let t = &ctx.state().text_problems;
        (
            d.error_count() + t.iter().filter(|p| p.level == Level::Error).count(),
            d.warning_count() + t.iter().filter(|p| p.level == Level::Warn).count(),
        )
    };
    ui.horizontal(|ui| {
        ui.toggle_value(
            &mut filter.0,
            format!(
                "{}  {}",
                icons::WARNING_CIRCLE,
                ctx.tn("problems.errors", errors, &[])
            ),
        );
        ui.toggle_value(
            &mut filter.1,
            format!(
                "{}  {}",
                icons::WARNING,
                ctx.tn("problems.warnings", warnings, &[])
            ),
        );
    });
    ui.data_mut(|d| d.insert_temp(id, filter));
    ui.add_space(2.0);

    let shown: Vec<Problem> = ctx
        .state()
        .diagnostics
        .problems
        .iter()
        .chain(ctx.state().text_problems.iter())
        .filter(|p| match p.level {
            Level::Error => filter.0,
            Level::Warn => filter.1,
            Level::Info => true,
        })
        .cloned()
        .collect();
    if shown.is_empty() {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            let (glyph, text) = if errors + warnings == 0 {
                (icons::CHECK_CIRCLE, ctx.t("problems.none"))
            } else {
                (icons::FUNNEL, ctx.t("problems.none_filtered"))
            };
            ui.label(
                egui::RichText::new(glyph)
                    .size(26.0)
                    .color(super::ui_util::success()),
            );
            ui.label(egui::RichText::new(text).color(weak(ui)));
        });
        return;
    }
    let locations: Vec<String> = shown
        .iter()
        .map(|p| {
            p.location
                .as_ref()
                .map(|l| describe(ctx, l))
                .unwrap_or_default()
        })
        .collect();
    let click_hint = ctx.t("problems.click_hint");
    let mut clicked: Option<Location> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, shown.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let problem = &shown[i];
                let (rect, response) = list_row(ui, ROW_HEIGHT, false);
                let painter = ui.painter_at(rect);
                let mid = rect.center().y;
                let (icon, color) = match problem.level {
                    Level::Error => (icons::WARNING_CIRCLE, danger(ui)),
                    Level::Warn => (icons::WARNING, warning(ui)),
                    Level::Info => (icons::INFO, weak(ui)),
                };
                painter.text(
                    egui::pos2(rect.left() + 14.0, mid),
                    Align2::CENTER_CENTER,
                    icon,
                    FontId::proportional(14.0),
                    color,
                );
                let location = &locations[i];
                let clip = egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 30.0, rect.top()),
                    egui::pos2(rect.right() - 8.0, rect.bottom()),
                );
                let text_painter = painter.with_clip_rect(clip);
                text_painter.text(
                    egui::pos2(rect.left() + 30.0, rect.top() + 13.0),
                    Align2::LEFT_CENTER,
                    &problem.message,
                    FontId::proportional(13.0),
                    ui.visuals().text_color(),
                );
                text_painter.text(
                    egui::pos2(rect.left() + 30.0, rect.top() + 30.0),
                    Align2::LEFT_CENTER,
                    location,
                    FontId::proportional(11.5),
                    faint(ui),
                );
                if problem.location.is_some() {
                    let response = response.on_hover_text(format!(
                        "{}\n{}\n{}",
                        problem.message, location, click_hint
                    ));
                    if response.clicked() {
                        clicked = problem.location.clone();
                    }
                }
            }
        });
    if let Some(location) = clicked {
        reveal(ctx, location);
    }
}
