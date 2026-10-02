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
        PluginManifest::new(ids::WELCOME, "Welcome", "0.1.0")
    }
}

impl EditorPlugin for WelcomePlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "welcome",
            "welcome.title",
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

const LOGO_DARK: &[u8] = include_bytes!("../../../../assets/brand/logo-horizontal-dark.png");
const LOGO_LIGHT: &[u8] = include_bytes!("../../../../assets/brand/logo-horizontal-light.png");

/// The horizontal logo (variant matching the theme), loaded once per variant
/// as an egui texture and cached in the context.
fn logo(ui: &mut Ui) {
    let dark = ui.visuals().dark_mode;
    let key = egui::Id::new(("orbit-logo", dark));
    let cached = ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(key));
    let texture = cached.or_else(|| {
        let bytes = if dark { LOGO_DARK } else { LOGO_LIGHT };
        let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .ok()?
            .into_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let texture = ui.ctx().load_texture(
            "orbit-logo",
            egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        ui.ctx().data_mut(|d| d.insert_temp(key, texture.clone()));
        Some(texture)
    });
    match texture {
        // The image is 680x138: shown at half size (sharp on HiDPI screens).
        Some(texture) => {
            ui.add(
                egui::Image::new(&texture)
                    .fit_to_exact_size(vec2(340.0, 69.0))
                    .corner_radius(CornerRadius::same(10)),
            );
        }
        None => {
            ui.label(strong("OrbitEngine", 26.0));
        }
    }
}

fn content(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    ui.add_space(28.0);
    logo(ui);
    ui.add_space(6.0);
    ui.label(RichText::new(ctx.t("welcome.subtitle")).color(weak(ui)));
    ui.add_space(20.0);

    // Open project summary.
    if let (Some(info), Some(story)) = (ctx.project_info().cloned(), ctx.story()) {
        let scenes = story.scenes.len();
        let nodes: usize = story.scenes.iter().map(|s| s.nodes.len()).sum();
        let characters = story.characters.len();
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            section_title(ui, &ctx.t("welcome.open_project"));
            ui.horizontal(|ui| {
                ui.label(strong(&info.name, 18.0));
                if info.modified {
                    ui.label(
                        RichText::new(format!("● {}", ctx.t("status.unsaved")))
                            .color(warning(ui))
                            .small(),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("● {}", ctx.t("status.saved")))
                            .color(success())
                            .small(),
                    );
                }
            });
            ui.label(
                RichText::new(info.root.join(&info.story_file).display().to_string())
                    .small()
                    .color(faint(ui)),
            );
            ui.add_space(6.0);
            ui.label(format!(
                "{} · {} · {}",
                ctx.tn("project.scenes", scenes, &[]),
                ctx.tn("project.nodes", nodes, &[]),
                ctx.tn("project.characters", characters, &[])
            ));
            if !info.has_manifest {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(ctx.t("welcome.no_manifest"))
                        .small()
                        .color(faint(ui)),
                );
            }
        });
        ui.add_space(16.0);
    }

    // Start actions.
    ui.horizontal(|ui| {
        if primary_button(
            ui,
            &format!("{}  {}", icons::FILE_PLUS, ctx.t("welcome.new_project")),
        )
        .clicked()
        {
            ctx.request(EditorRequest::NewProjectDialog);
        }
        if secondary_button(
            ui,
            &format!("{}  {}", icons::FOLDER_OPEN, ctx.t("action.open_project")),
        )
        .clicked()
        {
            ctx.request(EditorRequest::PickOpenProject);
        }
        if secondary_button(
            ui,
            &format!("{}  {}", icons::FILM_SLATE, ctx.t("welcome.open_story")),
        )
        .clicked()
        {
            ctx.request(EditorRequest::PickOpenStory);
        }
    });
    ui.add_space(22.0);

    // Recent projects.
    section_title(ui, &ctx.t("welcome.recent"));
    let recent = ctx.state().recent.clone();
    if recent.is_empty() {
        ui.label(RichText::new(ctx.t("welcome.no_recent")).color(faint(ui)));
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
    section_title(ui, &ctx.t("welcome.tips"));
    ui.horizontal_wrapped(|ui| {
        for (keys, what) in [
            ("Ctrl+K", ctx.t("welcome.tip_palette")),
            ("Ctrl+S", ctx.t("welcome.tip_save")),
            ("Ctrl+Z", ctx.t("welcome.tip_undo")),
            ("Ctrl+Y", ctx.t("welcome.tip_redo")),
        ] {
            ui.label(RichText::new(keys).strong().color(accent(ui)));
            ui.label(RichText::new(what).color(weak(ui)));
            ui.add_space(14.0);
        }
    });
    ui.add_space(24.0);
}
