//! The project's named easing curves (`Story::curves`, schema v9): a panel
//! listing them, each with the reusable `CurveEditor` widget. Transitions
//! pick them by name; renaming or deleting keeps the project consistent
//! (a rename follows its usages, a used curve cannot be deleted).

use egui::{RichText, TextEdit, Ui};
use orbit_core::Curve;
use orbit_editor_api::curve_editor::{CurveEditorOptions, curve_editor};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, PanelDef, Plugin, PluginManifest,
    UiText, icons,
};
use orbit_project::{Command, ops};

use super::ids;
use super::ui_util::{card, primary_button, strong, weak};

pub struct CurvesPlugin;

impl Plugin for CurvesPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::CURVES, "Courbes", "0.1.0")
    }
}

impl EditorPlugin for CurvesPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "curves",
            "curve.panel.title",
            icons::BEZIER_CURVE,
            DockSlot::Right,
            curves_panel,
        ));
    }
}

enum Do {
    Rename(String, String),
    Set(String, Curve),
    Delete(String),
}

fn curves_panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    if !ctx.has_project() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(ui.t("defs.no_project")).color(weak(ui)));
        });
        return;
    }
    let mut action: Option<Do> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(strong(ui.t("curve.panel.title"), 16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = format!("{}  {}", icons::PLUS, ui.t("curve.panel.new"));
                    if primary_button(ui, &label).clicked() {
                        let Some(story) = ctx.story() else { return };
                        let name = ops::unique_name(
                            &ui.t("curve.panel.default_name"),
                            story.curves.keys().map(String::as_str),
                        );
                        action = Some(Do::Set(name, Curve::css_ease()));
                    }
                });
            });
            ui.label(RichText::new(ui.t("curve.panel.hint")).small().color(weak(ui)));
            ui.add_space(8.0);
            let Some(story) = ctx.story() else { return };
            if story.curves.is_empty() {
                ui.add_space(16.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icons::BEZIER_CURVE).size(30.0).color(weak(ui)));
                    ui.add_space(6.0);
                    ui.label(RichText::new(ui.t("curve.panel.empty")).color(weak(ui)));
                });
                return;
            }
            let curves = story.curves.clone();
            for (name, curve) in &curves {
                let users = ops::curve_users(story, name).len();
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut text = name.clone();
                        let edit = ui.add(
                            TextEdit::singleline(&mut text)
                                .id_salt(("curve_name", name))
                                .desired_width(150.0)
                                .hint_text(ui.t("curve.panel.name")),
                        );
                        // A rename is applied when the field loses focus.
                        if edit.lost_focus() && text != *name {
                            action = Some(Do::Rename(name.clone(), text));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(icons::TRASH)
                                .on_hover_text(ui.t("curve.panel.delete"))
                                .clicked()
                            {
                                action = Some(Do::Delete(name.clone()));
                            }
                        });
                    });
                    let used = if users == 0 {
                        ui.t("curve.panel.unused")
                    } else {
                        ui.tf("curve.panel.used", &[("count", &users)])
                    };
                    ui.label(RichText::new(used).small().color(weak(ui)));
                    ui.add_space(4.0);
                    // A named curve cannot be a reference to another one.
                    let options = CurveEditorOptions {
                        allow_named: false,
                        ..Default::default()
                    };
                    if let Some(c) = curve_editor(ui, ("curve", name), curve, &curves, options) {
                        action = Some(Do::Set(name.clone(), c));
                    }
                });
                ui.add_space(6.0);
            }
        });
    let result = match action {
        None => return,
        Some(Do::Rename(old, new)) => ctx.ops(|p| ops::rename_curve(p, &old, &new)),
        Some(Do::Set(name, curve)) => {
            // Live edits of one curve (a handle drag) are one undo step.
            ctx.exec_merged(
                Command::SetCurve {
                    name: name.clone(),
                    curve: Some(curve),
                },
                &format!("curve:{name}"),
            )
        }
        Some(Do::Delete(name)) => ctx.ops(|p| ops::delete_curve(p, &name)),
    };
    if let Err(e) = result {
        ctx.error(e.to_string());
    }
}
