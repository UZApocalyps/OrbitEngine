//! Problems panel: the live validation findings of the project (errors and
//! warnings) with their location. A click selects and reveals the faulty
//! node (explorer, scene editor, inspector) or opens the definition.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use egui::{Align2, FontId, Ui};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, Level, Location, PanelDef,
    Plugin, PluginManifest, Problem, UiText, icons,
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
        let cache = Rc::new(RefCell::new(ListCache::default()));
        reg.panel(PanelDef::new(
            "problems",
            "problems.title",
            icons::WARNING_CIRCLE,
            DockSlot::Bottom,
            move |ui, ctx| panel(ui, ctx, &mut cache.borrow_mut()),
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

/// The filtered list and the location texts of its rows, kept until the
/// findings, the story or the filter change: not rebuilt at every frame,
/// and a location is described only for the rows on screen (describing
/// one looks the node up in the story).
#[derive(Default)]
struct ListCache {
    key: Option<(u64, u64, u64, u64, bool, bool)>,
    shown: Rc<Vec<Problem>>,
    locations: HashMap<usize, String>,
}

/// Fingerprint of the text view's findings (few, published by the text
/// view without a serial).
fn text_problems_hash(problems: &[Problem]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in problems {
        p.message.hash(&mut h);
        (p.level as u8).hash(&mut h);
    }
    h.finish()
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>, cache: &mut ListCache) {
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
    // Results older than the project, with automatic validation off: dimmed.
    let stale = !ctx.state().validation.auto && ctx.state().validation.stale;
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let running = ctx.state().validation.running;
            if ui
                .button(format!("{}  {}", icons::CHECK_CIRCLE, ctx.t("problems.validate")))
                .on_hover_text(ctx.t("action.validate"))
                .clicked()
            {
                ctx.state_mut().validation.requested = true;
            }
            if running {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(egui::RichText::new(ctx.t("problems.running")).color(weak(ui)));
            } else if stale {
                let text = match &ctx.state().validation.at {
                    Some(at) => ctx.tf("problems.stale", &[("time", at)]),
                    None => ctx.t("problems.stale_no_time"),
                };
                ui.label(egui::RichText::new(text).color(weak(ui)));
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(id, filter));
    ui.add_space(2.0);

    let doc = ctx.doc_key();
    let key = (
        ctx.state().diagnostics.serial,
        doc.generation,
        doc.revision,
        text_problems_hash(&ctx.state().text_problems),
        filter.0,
        filter.1,
    );
    if cache.key != Some(key) {
        cache.key = Some(key);
        cache.locations.clear();
        cache.shown = Rc::new(
            ctx.state()
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
                .collect(),
        );
    }
    let shown = cache.shown.clone();
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
    let locations = &mut cache.locations;
    let click_hint = ctx.t("problems.click_hint");
    if stale {
        ui.set_opacity(0.5);
    }
    let mut clicked: Option<Location> = None;
    let mut edit: Option<orbit_core::NodeId> = None;
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
                let location: &String = locations.entry(i).or_insert_with(|| {
                    problem
                        .location
                        .as_ref()
                        .map(|l| describe(ctx, l))
                        .unwrap_or_default()
                });
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
                    if let Some(orbit_editor_api::Location::Node(id)) = &problem.location {
                        response.context_menu(|ui| {
                            if ui.button(ui.t("common.edit")).clicked() {
                                edit = Some(*id);
                                ui.close();
                            }
                        });
                    }
                }
            }
        });
    if let Some(id) = edit {
        super::reveal::edit_node(ctx, id);
    }
    if let Some(location) = clicked {
        reveal(ctx, location);
    }
}
