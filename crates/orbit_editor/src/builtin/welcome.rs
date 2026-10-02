//! Welcome page: start actions, recent projects, and a summary of the open
//! project.

use egui::{Align2, CornerRadius, FontId, RichText, Sense, Stroke, Ui, vec2};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, PanelDef, Plugin,
    PluginManifest, icons,
};

use super::ids;
use super::ui_util::{
    accent, card, faint, primary_button, secondary_button, section_title, strong, success, warning,
    weak,
};

pub struct WelcomePlugin;

impl Plugin for WelcomePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::WELCOME, "Bienvenue", "0.1.0")
    }
}

impl EditorPlugin for WelcomePlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "welcome",
            "Bienvenue",
            icons::HOUSE,
            DockSlot::Center,
            panel,
        ));
    }
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width().min(680.0);
            ui.vertical_centered(|ui| {
                ui.allocate_ui(vec2(width, 0.0), |ui| {
                    ui.set_width(width);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        content(ui, ctx);
                    });
                });
            });
        });
}

fn content(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    ui.add_space(28.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(icons::ROCKET_LAUNCH)
                .size(34.0)
                .color(accent(ui)),
        );
        ui.vertical(|ui| {
            ui.label(strong("OrbitEngine", 26.0));
            ui.label(RichText::new("Éditeur de visual novels").color(weak(ui)));
        });
    });
    ui.add_space(20.0);

    // Open project summary.
    if let (Some(info), Some(story)) = (ctx.project_info().cloned(), ctx.story()) {
        let scenes = story.scenes.len();
        let nodes: usize = story.scenes.iter().map(|s| s.nodes.len()).sum();
        let characters = story.characters.len();
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, "Projet ouvert");
            ui.horizontal(|ui| {
                ui.label(strong(&info.name, 18.0));
                if info.modified {
                    ui.label(
                        RichText::new("● modifications non enregistrées")
                            .color(warning(ui))
                            .small(),
                    );
                } else {
                    ui.label(RichText::new("● enregistré").color(success()).small());
                }
            });
            ui.label(
                RichText::new(info.root.join(&info.story_file).display().to_string())
                    .small()
                    .color(faint(ui)),
            );
            ui.add_space(6.0);
            ui.label(format!(
                "{scenes} scène(s) · {nodes} nœud(s) · {characters} personnage(s)"
            ));
            if !info.has_manifest {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Histoire ouverte sans dossier de projet (pas de orbit.project.ron).",
                    )
                    .small()
                    .color(faint(ui)),
                );
            }
        });
        ui.add_space(16.0);
    }

    // Start actions.
    ui.horizontal(|ui| {
        if primary_button(ui, &format!("{}  Nouveau projet", icons::FILE_PLUS)).clicked() {
            ctx.request(EditorRequest::NewProjectDialog);
        }
        if secondary_button(ui, &format!("{}  Ouvrir un projet…", icons::FOLDER_OPEN)).clicked() {
            ctx.request(EditorRequest::PickOpenProject);
        }
        if secondary_button(ui, &format!("{}  Ouvrir une histoire…", icons::FILM_SLATE)).clicked()
        {
            ctx.request(EditorRequest::PickOpenStory);
        }
    });
    ui.add_space(22.0);

    // Recent projects.
    section_title(ui, "Projets récents");
    let recent = ctx.state().recent.clone();
    if recent.is_empty() {
        ui.label(
            RichText::new("Aucun projet récent. Créez-en un ou ouvrez-en un existant.")
                .color(faint(ui)),
        );
    }
    for project in recent {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
        let hovered = response.hovered();
        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            CornerRadius::same(8),
            if hovered {
                ui.visuals().widgets.hovered.bg_fill
            } else {
                ui.visuals().widgets.inactive.bg_fill
            },
            Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
            egui::StrokeKind::Inside,
        );
        painter.text(
            egui::pos2(rect.left() + 18.0, rect.center().y),
            Align2::CENTER_CENTER,
            icons::FOLDER_SIMPLE,
            FontId::proportional(18.0),
            accent(ui),
        );
        painter.text(
            egui::pos2(rect.left() + 38.0, rect.center().y - 8.0),
            Align2::LEFT_CENTER,
            &project.name,
            crate::theme::bold(13.5),
            ui.visuals().text_color(),
        );
        painter.text(
            egui::pos2(rect.left() + 38.0, rect.center().y + 9.0),
            Align2::LEFT_CENTER,
            project.path.display().to_string(),
            FontId::proportional(11.5),
            faint(ui),
        );
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            ctx.request(EditorRequest::OpenProject(project.path.clone()));
        }
        ui.add_space(4.0);
    }

    // Tips.
    ui.add_space(22.0);
    section_title(ui, "Raccourcis utiles");
    ui.horizontal_wrapped(|ui| {
        for (keys, what) in [
            ("Ctrl+K", "palette de commandes"),
            ("Ctrl+S", "enregistrer"),
            ("Ctrl+Z", "annuler"),
            ("Ctrl+Y", "rétablir"),
        ] {
            ui.label(RichText::new(keys).strong().color(accent(ui)));
            ui.label(RichText::new(what).color(weak(ui)));
            ui.add_space(14.0);
        }
    });
    ui.add_space(24.0);
}
