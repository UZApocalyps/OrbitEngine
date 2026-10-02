//! "Assistant IA" panel: the live MCP server of the editor (M4).
//!
//! It shows whether the server runs and whether a client is connected, lets
//! the user start / stop it (off by default), choose the port, see / copy /
//! regenerate the access token and copy a ready-to-paste client config, and
//! lists live what the AI does: tool, arguments, result, duration, a link to
//! reveal the touched nodes, and a button that undoes the last AI action.
//!
//! The panel never touches the network: it reads `EditorState::ai` and posts
//! [`AiCommand`]s the host carries out (see `crate::live`).

use std::time::Duration;

use egui::{Align, Align2, Color32, FontId, Layout, RichText, Sense, Ui, vec2};
use orbit_editor_api::ai::{AiCommand, AiLogEntry, AiServerStatus};
use orbit_editor_api::{
    DockSlot, EditorContext, EditorPlugin, EditorRegistry, Location, PanelDef, Plugin,
    PluginManifest, icons,
};

use super::ids;
use super::reveal::reveal;
use super::ui_util::{
    accent, card, danger, faint, icon_button, list_row, primary_button, secondary_button,
    section_title, strong, success, warning, weak,
};

pub struct AssistantPlugin;

const ROW_HEIGHT: f32 = 44.0;
/// How long the "the AI is editing" indicator stays lit.
const ACTIVITY_SECONDS: f32 = 2.5;

impl Plugin for AssistantPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::ASSISTANT, "Assistant IA", "0.1.0")
    }
}

impl EditorPlugin for AssistantPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.panel(PanelDef::new(
            "assistant",
            "Assistant IA",
            icons::ROBOT,
            DockSlot::Bottom,
            panel,
        ));
    }
}

/// `orbit_a1b2…9f3c`: enough to recognise a token, not to copy it.
fn masked(token: &str) -> String {
    if token.len() <= 12 {
        return "•".repeat(token.len().max(8));
    }
    format!("{}{}", &token[..6], "•".repeat(14))
}

fn age(ms_ago: u64) -> String {
    let s = ms_ago / 1000;
    match s {
        0..=1 => "à l'instant".to_owned(),
        2..=59 => format!("il y a {s} s"),
        60..=3599 => format!("il y a {} min", s / 60),
        _ => format!("il y a {} h", s / 3600),
    }
}

fn status_line(status: &AiServerStatus, url: Option<&str>) -> (Color32, String) {
    match status {
        AiServerStatus::Stopped => (Color32::from_gray(140), "Arrêté".to_owned()),
        AiServerStatus::Listening => (
            success(),
            format!("À l'écoute · {}", url.unwrap_or("127.0.0.1")),
        ),
        AiServerStatus::Connected { clients } => (
            success(),
            format!(
                "{} client{} connecté{} · {}",
                clients,
                if *clients > 1 { "s" } else { "" },
                if *clients > 1 { "s" } else { "" },
                url.unwrap_or("127.0.0.1")
            ),
        ),
        AiServerStatus::Failed(message) => (Color32::from_rgb(220, 90, 90), message.clone()),
    }
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    egui::ScrollArea::vertical()
        .id_salt("orbit_assistant_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            server_card(ui, ctx);
            journal(ui, ctx);
        });
    // The "AI is editing" light goes out by itself.
    if ctx.state().ai.edited_within(ACTIVITY_SECONDS) {
        ui.ctx().request_repaint_after(Duration::from_millis(400));
    }
}

fn server_card(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let running = ctx.state().ai.status.is_running();
    let (dot, text) = status_line(&ctx.state().ai.status, ctx.state().ai.url.as_deref());
    let editing = ctx.state().ai.edited_within(ACTIVITY_SECONDS);
    let lock_warning = ctx.state().ai.lock_warning.clone();
    card(ui, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, dot);
            ui.label(strong(text, 13.5));
            if editing {
                ui.label(RichText::new("·").color(faint(ui)));
                ui.label(
                    RichText::new(format!("{} L'IA modifie le projet", icons::SPARKLE))
                        .color(accent(ui)),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if running {
                    if secondary_button(ui, &format!("{}  Arrêter", icons::STOP)).clicked() {
                        ctx.state_mut().ai.command(AiCommand::Stop);
                    }
                } else if primary_button(ui, &format!("{}  Activer le serveur MCP", icons::PLAY))
                    .clicked()
                {
                    ctx.state_mut().ai.command(AiCommand::Start);
                }
            });
        });
        if let Some(message) = &lock_warning {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("{}  {message}", icons::WARNING)).color(warning(ui)));
        }
        ui.add_space(8.0);
        settings_grid(ui, ctx, running);
        if !running {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Désactivé par défaut. Une fois activé, l'éditeur écoute uniquement sur \
                     127.0.0.1 et refuse toute requête sans le jeton ci-dessus.",
                )
                .size(12.0)
                .color(weak(ui)),
            );
        }
    });
}

fn settings_grid(ui: &mut Ui, ctx: &mut EditorContext<'_>, running: bool) {
    let token_id = egui::Id::new("orbit_assistant_token_visible");
    let mut show_token: bool = ui.data(|d| d.get_temp(token_id)).unwrap_or(false);
    let mut port = i32::from(ctx.state().ai.port);
    let token = ctx.state().ai.token.clone();
    let mut regenerate = false;
    egui::Grid::new("orbit_assistant_settings")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Port").color(weak(ui)));
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !running,
                    egui::DragValue::new(&mut port)
                        .range(1024..=65535)
                        .speed(1.0),
                )
                .on_hover_text("Port TCP sur 127.0.0.1 (modifiable quand le serveur est arrêté).");
            });
            ui.end_row();

            ui.label(RichText::new("Jeton").color(weak(ui)));
            ui.horizontal(|ui| {
                let shown = if show_token {
                    token.clone()
                } else {
                    masked(&token)
                };
                ui.label(RichText::new(shown).monospace().size(12.0));
                if icon_button(
                    ui,
                    if show_token {
                        icons::EYE_SLASH
                    } else {
                        icons::EYE
                    },
                    "Afficher / masquer le jeton",
                )
                .clicked()
                {
                    show_token = !show_token;
                }
                if icon_button(ui, icons::COPY, "Copier le jeton").clicked() {
                    ui.ctx().copy_text(token.clone());
                    ctx.state_mut()
                        .push_toast(orbit_editor_api::Level::Info, "Jeton copié.");
                }
                if icon_button(
                    ui,
                    icons::ARROWS_CLOCKWISE,
                    "Générer un nouveau jeton (l'ancien cesse de fonctionner)",
                )
                .clicked()
                {
                    regenerate = true;
                }
            });
            ui.end_row();

            ui.label(RichText::new("Configuration").color(weak(ui)));
            ui.horizontal_wrapped(|ui| {
                let port_now = ctx.state().ai.port;
                if secondary_button(ui, &format!("{}  Commande Claude Code", icons::COPY))
                    .on_hover_text(
                        "claude mcp add --transport http … (avec l'en-tête d'autorisation)",
                    )
                    .clicked()
                {
                    let text = ctx.state().ai.claude_command(port_now);
                    ui.ctx().copy_text(text);
                    ctx.state_mut()
                        .push_toast(orbit_editor_api::Level::Info, "Commande copiée.");
                }
                if secondary_button(ui, &format!("{}  .mcp.json", icons::COPY))
                    .on_hover_text("Bloc à coller dans le .mcp.json du dépôt de l'IA")
                    .clicked()
                {
                    let text = ctx.state().ai.mcp_json(port_now);
                    ui.ctx().copy_text(text);
                    ctx.state_mut()
                        .push_toast(orbit_editor_api::Level::Info, ".mcp.json copié.");
                }
            });
            ui.end_row();
        });
    ui.data_mut(|d| d.insert_temp(token_id, show_token));
    if !running {
        ctx.state_mut().ai.port = port.clamp(1024, 65535) as u16;
    }
    if regenerate {
        ctx.state_mut().ai.command(AiCommand::RegenerateToken);
    }
}

fn journal(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    ui.add_space(6.0);
    let can_undo = ctx.state().ai.can_undo_last;
    let count = ctx.state().ai.log.len();
    ui.horizontal(|ui| {
        section_title(ui, &format!("Journal de l'IA ({count})"));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, icons::TRASH, "Vider le journal").clicked() {
                ctx.state_mut().ai.command(AiCommand::ClearLog);
            }
            let undo = ui
                .add_enabled(
                    can_undo,
                    egui::Button::new(format!(
                        "{}  Annuler la dernière action de l'IA",
                        icons::ARROW_COUNTER_CLOCKWISE
                    ))
                    .min_size(vec2(0.0, 26.0)),
                )
                .on_hover_text(if can_undo {
                    "Annule l'entrée d'historique créée par le dernier appel de l'IA."
                } else {
                    "Disponible tant que la dernière modification de l'historique est celle de l'IA."
                });
            if undo.clicked() {
                ctx.state_mut().ai.command(AiCommand::UndoLast);
            }
        });
    });

    if count == 0 {
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(icons::ROBOT).size(26.0).color(faint(ui)));
            ui.label(RichText::new("Les actions de l'IA apparaîtront ici.").color(weak(ui)));
        });
        return;
    }

    let now = orbit_platform::unix_time_ms();
    // Newest first.
    let entries: Vec<AiLogEntry> = ctx.state().ai.log.iter().rev().cloned().collect();
    let mut to_reveal: Option<Location> = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    for entry in &entries {
        let (rect, response) = list_row(ui, ROW_HEIGHT, false);
        let painter = ui.painter_at(rect);
        let (icon, color) = if entry.ok {
            (icons::CHECK_CIRCLE, success())
        } else {
            (icons::WARNING_CIRCLE, danger(ui))
        };
        painter.text(
            egui::pos2(rect.left() + 14.0, rect.top() + 14.0),
            Align2::CENTER_CENTER,
            icon,
            FontId::proportional(14.0),
            color,
        );
        let reveal_width = if entry.touched.is_empty() { 0.0 } else { 92.0 };
        let text_clip = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 30.0, rect.top()),
            egui::pos2(rect.right() - 8.0 - reveal_width, rect.bottom()),
        );
        let text_painter = painter.with_clip_rect(text_clip);
        let galley = text_painter.layout_no_wrap(
            entry.tool.clone(),
            crate::theme::bold(13.0),
            ui.visuals().text_color(),
        );
        let tool_width = galley.size().x;
        text_painter.galley(
            egui::pos2(rect.left() + 30.0, rect.top() + 6.0),
            galley,
            ui.visuals().text_color(),
        );
        text_painter.text(
            egui::pos2(rect.left() + 38.0 + tool_width, rect.top() + 14.0),
            Align2::LEFT_CENTER,
            format!(
                "{} · {} ms{}",
                age(now.saturating_sub(entry.time_ms)),
                entry.duration_ms,
                if entry.undoable { " · annulable" } else { "" }
            ),
            FontId::proportional(11.5),
            faint(ui),
        );
        let detail = if entry.args.is_empty() || entry.args == "{}" {
            entry.result.clone()
        } else {
            format!("{}  →  {}", entry.args, entry.result)
        };
        text_painter.text(
            egui::pos2(rect.left() + 30.0, rect.top() + 31.0),
            Align2::LEFT_CENTER,
            detail,
            FontId::proportional(11.5),
            if entry.ok { weak(ui) } else { danger(ui) },
        );
        if let Some(first) = entry.touched.first() {
            let button = egui::Rect::from_min_size(
                egui::pos2(rect.right() - reveal_width - 4.0, rect.top() + 9.0),
                vec2(reveal_width - 2.0, 26.0),
            );
            let id = ui.id().with(("ai_reveal", entry.id));
            let click = ui.interact(button, id, Sense::click());
            if click.hovered() {
                painter.rect_filled(
                    button,
                    egui::CornerRadius::same(5),
                    ui.visuals().widgets.hovered.bg_fill,
                );
            }
            painter.text(
                button.center(),
                Align2::CENTER_CENTER,
                format!(
                    "{}  révéler{}",
                    icons::CURSOR_CLICK,
                    if entry.touched.len() > 1 {
                        format!(" ({})", entry.touched.len())
                    } else {
                        String::new()
                    }
                ),
                FontId::proportional(12.0),
                accent(ui),
            );
            if click
                .on_hover_text("Sélectionner le nœud touché dans l'éditeur")
                .clicked()
            {
                to_reveal = Some(Location::Node(*first));
            }
        }
        response.on_hover_text(format!("{}\n{}\n{}", entry.tool, entry.args, entry.result));
    }
    if let Some(location) = to_reveal {
        reveal(ctx, location);
    }
}
