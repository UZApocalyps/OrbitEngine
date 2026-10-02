//! Preview: the real runtime playing the open story, in a dock panel.
//!
//! This is only the panel: a toolbar, the picture, and the routing of the
//! pointer and keyboard to the game while the panel has the focus. The
//! runtime itself lives in the host (`crate::preview_host`); the two talk
//! through `EditorState::preview` (commands and input go down, the status and
//! the texture come up), so the panel is plain egui and tested headlessly.

use egui::{
    Align2, Color32, CornerRadius, Event, EventFilter, FontId, Id, Key, Modifiers, PointerButton,
    Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use orbit_editor_api::{
    ActionDef, DockSlot, EditorContext, EditorPlugin, EditorRegistry, EditorRequest, PanelDef,
    Plugin, PluginManifest, PreviewCommand, PreviewInput, PreviewSpeed, PreviewStatus, Selection,
    icons,
};

use super::ids;
use super::ui_util::{accent, danger, faint, icon_button, strong, success, warning, weak};

pub struct PreviewPlugin;

/// The virtual canvas the game draws on, in its own pixels.
pub const CANVAS: [f32; 2] = [1920.0, 1080.0];

const STATUS_HEIGHT: f32 = 22.0;

impl Plugin for PreviewPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::PREVIEW, "Preview", "0.1.0")
    }
}

impl EditorPlugin for PreviewPlugin {
    fn register_ui(&self, reg: &mut EditorRegistry) {
        reg.menu("preview", "Preview", 35);
        reg.panel(PanelDef::new(
            "preview",
            "Preview",
            icons::PLAY_CIRCLE,
            DockSlot::RightTop,
            panel,
        ));
        reg.action(
            ActionDef::new("play_pause", "Lecture / pause", |ctx| {
                let status = ctx.state().preview.status;
                ctx.preview_command(match status {
                    PreviewStatus::Playing => PreviewCommand::Pause,
                    PreviewStatus::Paused | PreviewStatus::Stopped => PreviewCommand::Play,
                });
                ctx.request(EditorRequest::ShowPanel(super::panels::PREVIEW.to_owned()));
            })
            .label_with(|ctx| {
                if ctx.state().preview.status == PreviewStatus::Playing {
                    "Preview : pause".to_owned()
                } else {
                    "Preview : lecture".to_owned()
                }
            })
            .icon(icons::PLAY)
            .category("Preview")
            .shortcut(Modifiers::NONE, Key::F6)
            .menu("preview", 100)
            .enabled(|c| c.has_project()),
        );
        reg.action(
            ActionDef::new("stop", "Preview : arrêter", |ctx| {
                ctx.preview_command(PreviewCommand::Stop);
            })
            .icon(icons::STOP)
            .category("Preview")
            .shortcut(Modifiers::SHIFT, Key::F6)
            .menu("preview", 110)
            .enabled(|c| c.state().preview.status != PreviewStatus::Stopped),
        );
        reg.action(
            ActionDef::new("restart", "Preview : recommencer", |ctx| {
                ctx.preview_command(PreviewCommand::Restart);
                ctx.request(EditorRequest::ShowPanel(super::panels::PREVIEW.to_owned()));
            })
            .icon(icons::ARROW_COUNTER_CLOCKWISE)
            .category("Preview")
            .shortcut(Modifiers::COMMAND, Key::F6)
            .menu("preview", 120)
            .enabled(|c| c.has_project()),
        );
        reg.action(
            ActionDef::new("play_from_here", "Preview : jouer depuis ici", |ctx| {
                if let Some(Selection::Node(id)) = ctx.selection() {
                    ctx.play_from(id);
                    ctx.request(EditorRequest::ShowPanel(super::panels::PREVIEW.to_owned()));
                }
            })
            .icon(icons::CROSSHAIR)
            .category("Preview")
            .shortcut(Modifiers::NONE, Key::F7)
            .menu("preview", 200)
            .enabled(|c| matches!(c.selection(), Some(Selection::Node(_)))),
        );
        reg.action(
            ActionDef::new("mute", "Preview : couper le son", |ctx| {
                let muted = ctx.state().preview.muted;
                ctx.preview_command(PreviewCommand::SetMuted(!muted));
            })
            .icon(icons::SPEAKER_X)
            .category("Preview")
            .menu("preview", 300),
        );
        reg.action(
            ActionDef::new("follow", "Preview : suivre la lecture", |ctx| {
                let preview = &mut ctx.state_mut().preview;
                preview.follow_playback = !preview.follow_playback;
            })
            .icon(icons::CROSSHAIR_SIMPLE)
            .category("Preview")
            .menu("preview", 310),
        );
        reg.action(
            ActionDef::new("fullscreen", "Preview : plein panneau", |ctx| {
                ctx.request(EditorRequest::ToggleMaximizePanel(
                    super::panels::PREVIEW.to_owned(),
                ));
            })
            .icon(icons::CORNERS_OUT)
            .category("Preview")
            .menu("preview", 320),
        );
    }
}

/// The largest 16:9 rectangle centered in `area`.
pub fn fit_16_9(area: Rect) -> Rect {
    let ratio = CANVAS[0] / CANVAS[1];
    let (mut w, mut h) = (area.width().max(1.0), area.height().max(1.0));
    if w / h > ratio {
        w = h * ratio;
    } else {
        h = w / ratio;
    }
    Rect::from_center_size(area.center(), vec2(w, h))
}

/// A point of the screen to virtual canvas pixels, given the picture's rect.
pub fn to_virtual(picture: Rect, pos: Pos2) -> [f32; 2] {
    let u = (pos.x - picture.left()) / picture.width().max(1.0);
    let v = (pos.y - picture.top()) / picture.height().max(1.0);
    [u * CANVAS[0], v * CANVAS[1]]
}

fn status_text(status: PreviewStatus) -> &'static str {
    match status {
        PreviewStatus::Stopped => "Arrêtée",
        PreviewStatus::Playing => "Lecture",
        PreviewStatus::Paused => "En pause",
    }
}

fn panel(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    ctx.state_mut().preview.visible = true;
    toolbar(ui, ctx);
    ui.add_space(2.0);

    let avail = ui.available_size();
    let size = vec2(avail.x.max(40.0), (avail.y - STATUS_HEIGHT).max(40.0));
    let (area, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let picture = fit_16_9(area);
    ctx.state_mut().preview.picture_rect = Some(picture);
    let ppp = ui.ctx().pixels_per_point();
    ctx.state_mut().preview.wanted_size = [
        (picture.width() * ppp).round() as u32,
        (picture.height() * ppp).round() as u32,
    ];

    let focused = response.has_focus();
    ctx.state_mut().preview.focused = focused;
    if focused {
        // Arrows, Tab and Escape belong to the game, not to focus navigation.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }

    draw_picture(ui, ctx, area, picture);
    route_input(ui, ctx, &response, picture);
    status_line(ui, ctx);
}

fn toolbar(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let (status, speed, skipping, muted, follow, available) = {
        let p = &ctx.state().preview;
        (
            p.status,
            p.speed,
            p.skipping,
            p.muted,
            p.follow_playback,
            p.available,
        )
    };
    let has_project = ctx.has_project();
    let selected_node = match ctx.selection() {
        Some(Selection::Node(id)) => Some(id),
        _ => None,
    };
    let maximized = ctx.state().maximized_panel.is_some();
    let can_play = has_project && available;

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let (icon, tip) = if status == PreviewStatus::Playing {
            (icons::PAUSE, "Pause (F6)")
        } else {
            (icons::PLAY, "Lecture (F6)")
        };
        if ui
            .add_enabled_ui(can_play, |ui| icon_button(ui, icon, tip))
            .inner
            .clicked()
        {
            ctx.run_action("orbit.preview.play_pause");
        }
        if ui
            .add_enabled_ui(status != PreviewStatus::Stopped, |ui| {
                icon_button(ui, icons::STOP, "Arrêter (Maj+F6)")
            })
            .inner
            .clicked()
        {
            ctx.preview_command(PreviewCommand::Stop);
        }
        if ui
            .add_enabled_ui(can_play, |ui| {
                icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, "Recommencer (Ctrl+F6)")
            })
            .inner
            .clicked()
        {
            ctx.preview_command(PreviewCommand::Restart);
        }
        ui.separator();
        if ui
            .add_enabled(
                can_play && selected_node.is_some(),
                egui::Button::new(RichText::new(format!("{}  Ici", icons::CROSSHAIR)).size(12.5))
                    .min_size(vec2(0.0, 26.0)),
            )
            .on_hover_text("Jouer depuis le nœud sélectionné (F7)")
            .clicked()
            && let Some(id) = selected_node
        {
            ctx.play_from(id);
        }
        ui.separator();
        for (label, value, tip) in [
            ("1×", PreviewSpeed::Normal, "Vitesse normale"),
            ("2×", PreviewSpeed::Double, "Vitesse double"),
        ] {
            let on = speed == value && !skipping;
            if ui
                .add_enabled(
                    can_play,
                    egui::Button::selectable(on, RichText::new(label).size(12.5))
                        .min_size(vec2(26.0, 26.0)),
                )
                .on_hover_text(tip)
                .clicked()
            {
                ctx.preview_command(PreviewCommand::SetSpeed(value));
                ctx.preview_command(PreviewCommand::SetSkip(false));
            }
        }
        if ui
            .add_enabled(
                can_play,
                egui::Button::selectable(skipping, RichText::new(icons::FAST_FORWARD).size(14.0))
                    .min_size(vec2(26.0, 26.0)),
            )
            .on_hover_text("Passer les répliques (s'arrête aux choix)")
            .clicked()
        {
            ctx.preview_command(PreviewCommand::SetSkip(!skipping));
        }
        ui.separator();
        let (mute_icon, mute_tip) = if muted {
            (icons::SPEAKER_X, "Réactiver le son")
        } else {
            (icons::SPEAKER_HIGH, "Couper le son")
        };
        if icon_button(ui, mute_icon, mute_tip).clicked() {
            ctx.preview_command(PreviewCommand::SetMuted(!muted));
        }
        if ui
            .add(
                egui::Button::selectable(follow, RichText::new(icons::CROSSHAIR_SIMPLE).size(14.0))
                    .min_size(vec2(26.0, 26.0)),
            )
            .on_hover_text("Suivre la lecture dans l'explorateur")
            .clicked()
        {
            ctx.run_action("orbit.preview.follow");
        }
        let (max_icon, max_tip) = if maximized {
            (icons::CORNERS_IN, "Revenir à la disposition")
        } else {
            (icons::CORNERS_OUT, "Plein panneau")
        };
        if icon_button(ui, max_icon, max_tip).clicked() {
            ctx.run_action("orbit.preview.fullscreen");
        }
    });
}

fn draw_picture(ui: &mut Ui, ctx: &mut EditorContext<'_>, area: Rect, picture: Rect) {
    let painter = ui.painter_at(area);
    painter.rect_filled(area, CornerRadius::same(4), ui.visuals().extreme_bg_color);
    let preview = &ctx.state().preview;
    let has_project = ctx.has_project();
    match preview.texture {
        Some(texture) if preview.available => {
            painter.image(
                texture,
                picture,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        _ => {}
    }
    painter.rect_stroke(
        picture,
        CornerRadius::same(2),
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        StrokeKind::Outside,
    );
    if preview.focused {
        painter.rect_stroke(
            picture,
            CornerRadius::same(2),
            Stroke::new(1.5, accent(ui)),
            StrokeKind::Outside,
        );
    }

    // What the picture does not say by itself.
    let centered = |text: &str, sub: &str| {
        painter.text(
            picture.center() - vec2(0.0, 8.0),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(15.0),
            weak(ui),
        );
        if !sub.is_empty() {
            painter.text(
                picture.center() + vec2(0.0, 12.0),
                Align2::CENTER_CENTER,
                sub,
                FontId::proportional(12.0),
                faint(ui),
            );
        }
    };
    if !preview.available {
        centered(
            "Preview indisponible",
            "Aucun GPU n'a pu être initialisé pour la preview.",
        );
    } else if !has_project {
        centered("Aucun projet ouvert", "Ouvrez un projet pour le jouer ici.");
    } else if preview.status == PreviewStatus::Stopped {
        centered(
            &format!("{}  Appuyez sur Lecture (F6)", icons::PLAY),
            "Sélectionnez un nœud puis « Ici » pour jouer depuis là.",
        );
    }
    if let Some(error) = preview.error.clone() {
        let header = "Erreur dans l'histoire : la preview garde la dernière version valide";
        let galley = painter.layout(
            format!("{header}\n{error}"),
            FontId::proportional(11.5),
            Color32::WHITE,
            (picture.width() - 16.0).max(40.0),
        );
        let box_rect = Rect::from_min_size(
            picture.min + vec2(6.0, 6.0),
            galley.size() + vec2(12.0, 8.0),
        )
        .intersect(picture.shrink(2.0));
        painter.rect_filled(
            box_rect,
            CornerRadius::same(5),
            Color32::from_rgba_unmultiplied(120, 28, 36, 225),
        );
        painter.with_clip_rect(box_rect).galley(
            box_rect.min + vec2(6.0, 4.0),
            galley,
            Color32::WHITE,
        );
    }
}

fn status_line(ui: &mut Ui, ctx: &mut EditorContext<'_>) {
    let preview = &ctx.state().preview;
    let (color, dot) = match preview.status {
        PreviewStatus::Playing => (success(), icons::CIRCLE),
        PreviewStatus::Paused => (warning(ui), icons::PAUSE),
        PreviewStatus::Stopped => (faint(ui), icons::STOP),
    };
    let text = if preview.flow.is_empty() {
        status_text(preview.status).to_owned()
    } else {
        format!("{} · {}", status_text(preview.status), preview.flow)
    };
    let focus_hint = preview.focused && preview.status == PreviewStatus::Playing;
    let failed = preview.error.is_some();
    ui.horizontal(|ui| {
        ui.label(RichText::new(dot).size(10.0).color(color));
        ui.label(strong(text, 12.0));
        if failed {
            ui.label(RichText::new(icons::WARNING).color(danger(ui)));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let hint = if focus_hint {
                "clavier et souris envoyés au jeu"
            } else {
                "cliquez dans l'image pour jouer"
            };
            ui.label(RichText::new(hint).size(11.0).color(faint(ui)));
        });
    });
}

/// Pointer, wheel and keys to the host, only while the panel has the focus
/// (a press on the picture is what gives it the focus, and goes through).
fn route_input(ui: &mut Ui, ctx: &mut EditorContext<'_>, response: &egui::Response, picture: Rect) {
    let playing = ctx.state().preview.status == PreviewStatus::Playing;
    // Take the focus on a press inside the picture.
    let (events, hover, modifiers) =
        ui.input(|i| (i.events.clone(), i.pointer.hover_pos(), i.modifiers));
    let pressed_inside = events.iter().any(|e| {
        matches!(
            e,
            Event::PointerButton { pos, pressed: true, .. } if picture.contains(*pos) && response.rect.contains(*pos)
        )
    });
    // A press anywhere else gives the keyboard back to the editor (egui only
    // moves the focus between text fields and focusable widgets).
    let pressed_outside = events.iter().any(|e| {
        matches!(
            e,
            Event::PointerButton { pos, pressed: true, .. } if !response.rect.contains(*pos)
        )
    });
    if pressed_inside {
        response.request_focus();
    } else if pressed_outside && response.has_focus() {
        response.surrender_focus();
    }
    let focused = (response.has_focus() && !pressed_outside) || pressed_inside;
    ctx.state_mut().preview.focused = focused;
    if !playing || !focused {
        return;
    }

    // Pointer position, when it moved over the picture.
    let last_id = Id::new("orbit_preview_last_pointer");
    let last: Option<[f32; 2]> = ui.data(|d| d.get_temp(last_id));
    let pos = hover
        .filter(|p| picture.contains(*p))
        .map(|p| to_virtual(picture, p));
    if pos != last {
        ui.data_mut(|d| {
            if let Some(p) = pos {
                d.insert_temp(last_id, p);
            } else {
                d.remove_temp::<[f32; 2]>(last_id);
            }
        });
        let input = match pos {
            Some(p) => PreviewInput::PointerMoved(p),
            None => PreviewInput::PointerLeft,
        };
        ctx.state_mut().preview.input.push(input);
    }

    let mut consumed: Vec<Key> = Vec::new();
    for event in &events {
        match event {
            Event::PointerButton {
                pos,
                button,
                pressed,
                ..
            } if picture.contains(*pos) || (!*pressed && *button == PointerButton::Primary) => {
                let input = match button {
                    PointerButton::Primary => {
                        // Position first, so the game clicks where the pointer is.
                        ctx.state_mut()
                            .preview
                            .input
                            .push(PreviewInput::PointerMoved(to_virtual(picture, *pos)));
                        Some(PreviewInput::Primary { pressed: *pressed })
                    }
                    PointerButton::Secondary if *pressed => Some(PreviewInput::Secondary),
                    _ => None,
                };
                if let Some(input) = input {
                    ctx.state_mut().preview.input.push(input);
                }
            }
            Event::MouseWheel { delta, .. }
                if delta.y != 0.0 && hover.is_some_and(|p| picture.contains(p)) =>
            {
                ctx.state_mut()
                    .preview
                    .input
                    .push(PreviewInput::Scroll(delta.y));
            }
            Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } if game_key(*key) => {
                ctx.state_mut().preview.input.push(PreviewInput::Key {
                    key: *key,
                    pressed: *pressed,
                    shift: modifiers.shift,
                    alt: modifiers.alt,
                });
                consumed.push(*key);
            }
            _ => {}
        }
    }
    // Ctrl is the game's temporary skip: report its changes.
    let ctrl_id = Id::new("orbit_preview_ctrl");
    let was: bool = ui.data(|d| d.get_temp(ctrl_id)).unwrap_or(false);
    if modifiers.ctrl != was {
        ui.data_mut(|d| d.insert_temp(ctrl_id, modifiers.ctrl));
        ctx.state_mut()
            .preview
            .input
            .push(PreviewInput::Ctrl(modifiers.ctrl));
    }
    // The keys the game took are not shortcuts of the editor.
    if !consumed.is_empty() {
        ui.input_mut(|i| {
            i.events
                .retain(|e| !matches!(e, Event::Key { key, .. } if consumed.contains(key)));
        });
    }
}

/// Keys the game reacts to (see `orbit_runtime::InputMapper`).
fn game_key(key: Key) -> bool {
    matches!(
        key,
        Key::Enter
            | Key::Space
            | Key::Escape
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::ArrowLeft
            | Key::ArrowRight
            | Key::Tab
            | Key::PageUp
            | Key::PageDown
            | Key::F5
            | Key::F9
            | Key::F11
            | Key::H
            | Key::S
            | Key::A
            | Key::Num1
            | Key::Num2
            | Key::Num3
            | Key::Num4
            | Key::Num5
            | Key::Num6
            | Key::Num7
            | Key::Num8
            | Key::Num9
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_keeps_the_16_9_ratio_with_bars() {
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 300.0));
        let r = fit_16_9(wide);
        assert!((r.width() / r.height() - 16.0 / 9.0).abs() < 1e-3);
        assert!((r.height() - 300.0).abs() < 1e-3);
        assert!((r.center().x - 500.0).abs() < 1e-3);
        let tall = Rect::from_min_size(pos2(10.0, 10.0), vec2(320.0, 600.0));
        let r = fit_16_9(tall);
        assert!((r.width() - 320.0).abs() < 1e-3);
        assert!((r.width() / r.height() - 16.0 / 9.0).abs() < 1e-3);
    }

    #[test]
    fn pointer_positions_become_virtual_coordinates() {
        let picture = Rect::from_min_size(pos2(100.0, 50.0), vec2(960.0, 540.0));
        assert_eq!(to_virtual(picture, pos2(100.0, 50.0)), [0.0, 0.0]);
        assert_eq!(to_virtual(picture, pos2(1060.0, 590.0)), [1920.0, 1080.0]);
        assert_eq!(to_virtual(picture, pos2(580.0, 320.0)), [960.0, 540.0]);
    }
}
