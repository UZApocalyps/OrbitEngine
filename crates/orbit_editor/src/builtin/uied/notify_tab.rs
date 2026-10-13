//! The "Notifications" tab of the interface editor: where the in-game toasts
//! go (anchor, offset, width, how many, gap, stacking), how long they stay
//! and how they move, their sound, what happens while skipping, and the
//! named looks (`default`, `achievement`, `system`, custom ones) drawn with
//! a style of the theme or plain colors. "Show test notification" pops one
//! over the live preview.

use egui::{ComboBox, RichText, ScrollArea, Ui};
use orbit_editor_api::{EditorContext, EditorRequest, Translator, icons};
use orbit_ui::{
    Border, NotificationStyle, NotificationTheme, NotifyAnchor, NotifyAnimation, NotifySkip, StackDirection,
    UiBundle,
};

use super::dialogue_tab::image_picker;
use super::props::{color, num, optional, pick, row};
use super::{Mode, State, panels};
use crate::builtin::ui_util::{card, section_title, weak};

const BUILTIN: [&str; 3] = ["default", "achievement", "system"];

fn anchor_key(a: NotifyAnchor) -> &'static str {
    match a {
        NotifyAnchor::TopLeft => "uied.anchor.top_left",
        NotifyAnchor::TopCenter => "uied.anchor.top",
        NotifyAnchor::TopRight => "uied.anchor.top_right",
        NotifyAnchor::BottomLeft => "uied.anchor.bottom_left",
        NotifyAnchor::BottomCenter => "uied.anchor.bottom",
        NotifyAnchor::BottomRight => "uied.anchor.bottom_right",
    }
}

fn sound_picker(ui: &mut Ui, tr: &Translator, salt: &str, sounds: &[String], slot: &mut Option<String>) {
    let mut path = slot.clone().unwrap_or_default();
    let text = if path.is_empty() { tr.t("uied.none") } else { path.clone() };
    ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(160.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut path, String::new(), tr.t("uied.none"));
            for s in sounds {
                ui.selectable_value(&mut path, s.clone(), s);
            }
        });
    *slot = (!path.is_empty()).then_some(path);
}

pub(super) fn notify_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let tr = ctx.translator();
    let Some(theme) = st.doc.as_ref().and_then(|d| d.bundle.theme.clone()) else {
        card(ui, |ui| {
            ui.label(RichText::new(tr.t("uied.theme_none_title")).strong());
            ui.label(RichText::new(tr.t("uied.ntf_none_text")).small().color(weak(ui)));
            if ui
                .button(format!("{}  {}", icons::PALETTE, tr.t("uied.theme_create")))
                .clicked()
                && let Some(doc) = st.doc.as_mut()
            {
                doc.edit(|b| b.theme = UiBundle::builtin().theme);
            }
        });
        return;
    };
    let styles: Vec<String> = theme.styles.keys().cloned().collect();
    let fonts: Vec<String> = theme.fonts.keys().cloned().collect();
    let images = super::images_of(ctx);
    let sounds = super::sounds_of(ctx);
    let mut n: NotificationTheme = theme.notifications.clone();
    if st.ntf_style.is_empty() {
        st.ntf_style = "default".to_owned();
    }

    // ---- test notification over the preview
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.ntf_text)
                    .hint_text(tr.t("uied.ntf_sample"))
                    .desired_width(220.0),
            );
            if ui
                .button(format!("{}  {}", icons::BELL_RINGING, tr.t("uied.ntf_test")))
                .on_hover_text(tr.t("uied.ntf_test_tip"))
                .clicked()
            {
                if st.mode != Mode::Dialogue {
                    st.mode = Mode::Dialogue;
                    ctx.request(EditorRequest::ShowPanel(panels::PREVIEW.to_owned()));
                }
                let text = if st.ntf_text.trim().is_empty() { tr.t("uied.ntf_sample") } else { st.ntf_text.clone() };
                let style = (st.ntf_style != "default").then(|| st.ntf_style.clone());
                st.ntf_pending = Some((text, style));
            }
        });
    });
    // Sent once the preview shows the current theme (next frames).
    if st.mode == Mode::Dialogue
        && st.sent.as_ref().is_some_and(|s| s.0 == Mode::Dialogue)
        && let Some(test) = st.ntf_pending.take()
    {
        ctx.state_mut().preview.ui_test_notification = Some(test);
    }

    let mut delete: Option<String> = None;
    ScrollArea::vertical()
        .id_salt("uied_notifications")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.columns(2, |cols| {
                let (left, right) = cols.split_at_mut(1);
                let ui = &mut left[0];
                // ---------------- placement
                section_title(ui, &tr.t("uied.ntf_placement"));
                row(ui, &tr.t("uied.dlg_placement"), |ui| {
                    let options: Vec<(NotifyAnchor, String)> =
                        NotifyAnchor::ALL.iter().map(|a| (*a, tr.t(anchor_key(*a)))).collect();
                    pick(ui, "ntf_anchor", &mut n.anchor, &options);
                });
                row(ui, &tr.t("uied.dlg_offset"), |ui| {
                    num(ui, &mut n.offset[0], 1.0);
                    num(ui, &mut n.offset[1], 1.0);
                });
                row(ui, &tr.t("uied.ntf_width"), |ui| {
                    num(ui, &mut n.width, 1.0);
                    ui.label(RichText::new(tr.t("uied.ntf_width_fit")).small().color(weak(ui)));
                });
                row(ui, &tr.t("uied.ntf_max"), |ui| {
                    ui.add(egui::DragValue::new(&mut n.max_visible).range(1..=12));
                });
                row(ui, &tr.t("uied.ntf_gap"), |ui| num(ui, &mut n.gap, 0.5));
                row(ui, &tr.t("uied.ntf_direction"), |ui| {
                    let options = [
                        (StackDirection::Auto, tr.t("uied.ntf_dir_auto")),
                        (StackDirection::Down, tr.t("uied.ntf_dir_down")),
                        (StackDirection::Up, tr.t("uied.ntf_dir_up")),
                    ];
                    pick(ui, "ntf_dir", &mut n.direction, &options);
                });

                // ---------------- timing
                section_title(ui, &tr.t("uied.ntf_timing"));
                row(ui, &tr.t("uied.ntf_duration"), |ui| {
                    optional(ui, &mut n.duration, theme.widgets.list.toast_seconds, |ui, d| {
                        ui.add(egui::DragValue::new(d).speed(0.1).range(0.1..=60.0).suffix(" s"));
                    });
                });
                row(ui, &tr.t("uied.ntf_animation"), |ui| {
                    let options = [
                        (NotifyAnimation::Slide, tr.t("uied.ntf_anim_slide")),
                        (NotifyAnimation::Fade, tr.t("uied.ntf_anim_fade")),
                        (NotifyAnimation::None, tr.t("uied.ntf_anim_none")),
                    ];
                    pick(ui, "ntf_anim", &mut n.animation, &options);
                    ui.add(
                        egui::DragValue::new(&mut n.animation_seconds)
                            .speed(0.01)
                            .range(0.0..=3.0)
                            .suffix(" s"),
                    );
                });
                row(ui, &tr.t("uied.ntf_sound"), |ui| sound_picker(ui, &tr, "ntf_sound", &sounds, &mut n.sound));
                row(ui, &tr.t("uied.ntf_click"), |ui| {
                    ui.checkbox(&mut n.click_to_dismiss, "");
                });
                row(ui, &tr.t("uied.ntf_skip"), |ui| {
                    let options = [
                        (NotifySkip::Show, tr.t("uied.ntf_skip_show")),
                        (NotifySkip::Brief, tr.t("uied.ntf_skip_brief")),
                        (NotifySkip::Suppress, tr.t("uied.ntf_skip_hide")),
                    ];
                    pick(ui, "ntf_skip", &mut n.skip, &options);
                });
                ui.label(RichText::new(tr.t("uied.ntf_info")).small().color(weak(ui)));

                // ---------------- styles
                let ui = &mut right[0];
                section_title(ui, &tr.t("uied.ntf_styles"));
                let names = n.style_names();
                ui.horizontal_wrapped(|ui| {
                    for name in &names {
                        let own = n.styles.contains_key(name);
                        let label = if own { format!("{name} •") } else { name.clone() };
                        if ui.selectable_label(st.ntf_style == *name, label).clicked() {
                            st.ntf_style = name.clone();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut st.ntf_new)
                            .hint_text(tr.t("uied.ntf_new_hint"))
                            .desired_width(140.0),
                    );
                    let name = st.ntf_new.trim().to_owned();
                    if ui
                        .add_enabled(!name.is_empty() && !names.contains(&name), egui::Button::new(icons::PLUS))
                        .on_hover_text(tr.t("uied.ntf_add"))
                        .clicked()
                    {
                        n.styles.insert(name.clone(), NotificationStyle::default());
                        st.ntf_style = name;
                        st.ntf_new.clear();
                    }
                });
                let selected = st.ntf_style.clone();
                let mut s = n.styles.get(&selected).cloned().unwrap_or_default();
                ui.label(
                    RichText::new(tr.tf("uied.ntf_editing", &[("name", &selected)]))
                        .small()
                        .color(weak(ui)),
                );
                row(ui, &tr.t("uied.ntf_theme_style"), |ui| {
                    let mut name = s.style.clone().unwrap_or_default();
                    let text = if name.is_empty() { tr.t("uied.dlg_plain") } else { name.clone() };
                    ComboBox::from_id_salt("ntf_theme_style")
                        .selected_text(text)
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut name, String::new(), tr.t("uied.dlg_plain"));
                            for st in &styles {
                                ui.selectable_value(&mut name, st.clone(), st);
                            }
                        });
                    s.style = (!name.is_empty()).then_some(name);
                });
                row(ui, &tr.t("uied.ntf_background"), |ui| {
                    optional(ui, &mut s.background, [12, 16, 40, 235], |ui, c| color(ui, c));
                });
                row(ui, &tr.t("uied.ntf_border"), |ui| {
                    optional(ui, &mut s.border, Border { color: [255, 214, 100, 255], width: 2.0 }, |ui, b| {
                        color(ui, &mut b.color);
                        num(ui, &mut b.width, 0.1);
                    });
                });
                row(ui, &tr.t("uied.ntf_radius"), |ui| {
                    optional(ui, &mut s.corner_radius, 12.0, |ui, r| num(ui, r, 0.5));
                });
                row(ui, &tr.t("uied.ntf_padding"), |ui| {
                    optional(ui, &mut s.padding, [24.0, 14.0], |ui, p| {
                        num(ui, &mut p[0], 0.5);
                        num(ui, &mut p[1], 0.5);
                    });
                });
                row(ui, &tr.t("uied.ntf_icon_size"), |ui| {
                    optional(ui, &mut s.icon_size, 64.0, |ui, v| num(ui, v, 0.5));
                });
                row(ui, &tr.t("uied.ntf_icon"), |ui| image_picker(ui, &tr, "ntf_icon", &images, &mut s.icon));
                row(ui, &tr.t("uied.ntf_text_size"), |ui| {
                    optional(ui, &mut s.text_size, 30.0, |ui, v| num(ui, v, 0.5));
                });
                row(ui, &tr.t("uied.ntf_text_color"), |ui| {
                    optional(ui, &mut s.text_color, [235, 238, 248, 255], |ui, c| color(ui, c));
                });
                row(ui, &tr.t("uied.ntf_font"), |ui| {
                    let mut name = s.font.clone().unwrap_or_default();
                    let text = if name.is_empty() { tr.t("uied.none") } else { name.clone() };
                    ComboBox::from_id_salt("ntf_font").selected_text(text).show_ui(ui, |ui| {
                        ui.selectable_value(&mut name, String::new(), tr.t("uied.none"));
                        for f in &fonts {
                            ui.selectable_value(&mut name, f.clone(), f);
                        }
                    });
                    s.font = (!name.is_empty()).then_some(name);
                });
                row(ui, &tr.t("uied.ntf_width"), |ui| {
                    optional(ui, &mut s.width, n.width, |ui, v| num(ui, v, 1.0));
                });
                row(ui, &tr.t("uied.ntf_duration"), |ui| {
                    optional(ui, &mut s.duration, 3.0, |ui, v| {
                        ui.add(egui::DragValue::new(v).speed(0.1).range(0.1..=60.0).suffix(" s"));
                    });
                });
                row(ui, &tr.t("uied.ntf_sound"), |ui| {
                    sound_picker(ui, &tr, "ntf_style_sound", &sounds, &mut s.sound);
                });
                if s == NotificationStyle::default() && BUILTIN.contains(&selected.as_str()) {
                    n.styles.remove(&selected);
                } else {
                    n.styles.insert(selected.clone(), s);
                }
                if !BUILTIN.contains(&selected.as_str())
                    && ui
                        .small_button(format!("{}  {}", icons::TRASH, tr.t("uied.ntf_delete")))
                        .clicked()
                {
                    delete = Some(selected);
                }
            });
        });
    if let Some(name) = delete {
        n.styles.remove(&name);
        st.ntf_style = "default".to_owned();
    }
    if n != theme.notifications
        && let Some(doc) = st.doc.as_mut()
    {
        doc.edit_merged("notifications", |b| {
            if let Some(th) = b.theme.as_mut() {
                th.notifications = n;
            }
        });
    }
}
