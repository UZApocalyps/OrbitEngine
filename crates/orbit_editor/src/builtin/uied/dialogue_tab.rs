//! The "Dialogue box" tab of the interface editor: the textbox, speaker
//! name and name plate, "continue" indicator, choice buttons and quick menu
//! of the theme (`ui_version` 8), each placeable by anchor and drawable with
//! a style of the theme (images and nine-slices included). The preview is
//! the real runtime drawing a sample line, or the choices.

use egui::{ComboBox, DragValue, RichText, ScrollArea, Ui};
use orbit_editor_api::{EditorContext, EditorRequest, Translator, icons};
use orbit_ui::{
    Anchor, BubbleTheme, ChoiceTheme, DialogueMode, Fill, NamePlate, NarratorStyle, OffscreenStyle, QuickMenuTheme,
    StateColors, Style, TextBoxTheme, UiBundle,
};

use super::props::{color, num, row};
use super::{Mode, State, Tab, model, panels};
use crate::builtin::ui_util::{card, section_title, weak};

/// The sample the preview shows: `(speaker, line, choices)` (`choices`
/// empty: the line).
pub(super) fn sample(ctx: &EditorContext<'_>, st: &State) -> (String, String, Vec<String>) {
    let or = |own: &str, key: &str| {
        if own.trim().is_empty() {
            ctx.t(key)
        } else {
            own.to_owned()
        }
    };
    let speaker = or(&st.dlg_speaker, "uied.dlg_sample_speaker");
    let line = or(&st.dlg_line, "uied.dlg_sample_line");
    let choices = if st.dlg_choices {
        (1..=3)
            .map(|i| ctx.tf("uied.dlg_sample_choice", &[("n", &i)]))
            .collect()
    } else {
        Vec::new()
    };
    (speaker, line, choices)
}

/// The anchor presets: `(canvas = pivot point, label key)`.
const PRESETS: [([f32; 2], &str); 9] = [
    ([0.0, 0.0], "uied.anchor.top_left"),
    ([0.5, 0.0], "uied.anchor.top"),
    ([1.0, 0.0], "uied.anchor.top_right"),
    ([0.0, 0.5], "uied.anchor.left"),
    ([0.5, 0.5], "uied.anchor.center"),
    ([1.0, 0.5], "uied.anchor.right"),
    ([0.0, 1.0], "uied.anchor.bottom_left"),
    ([0.5, 1.0], "uied.anchor.bottom"),
    ([1.0, 1.0], "uied.anchor.bottom_right"),
];

/// Placement: "fixed" (the caller's own position fields) or anchored to a
/// point of the screen, plus an offset.
fn anchor_editor(ui: &mut Ui, tr: &Translator, salt: &str, anchor: &mut Option<Anchor>, fixed_key: &str) {
    row(ui, &tr.t("uied.dlg_placement"), |ui| {
        let current = anchor.map(|a| a.canvas);
        let text = match current {
            None => tr.t(fixed_key),
            Some(c) => PRESETS
                .iter()
                .find(|(p, _)| *p == c)
                .map_or_else(|| tr.t("uied.anchor.custom"), |(_, k)| tr.t(k)),
        };
        ComboBox::from_id_salt((salt, "anchor"))
            .selected_text(text)
            .show_ui(ui, |ui| {
                if ui.selectable_label(anchor.is_none(), tr.t(fixed_key)).clicked() {
                    *anchor = None;
                }
                for (p, key) in PRESETS {
                    if ui.selectable_label(current == Some(p), tr.t(key)).clicked() {
                        let offset = anchor.map_or([0.0, 0.0], |a| a.offset);
                        *anchor = Some(Anchor::new(p, p, offset));
                    }
                }
            });
    });
    if let Some(a) = anchor {
        row(ui, &tr.t("uied.dlg_offset"), |ui| {
            num(ui, &mut a.offset[0], 1.0);
            num(ui, &mut a.offset[1], 1.0);
        });
    }
}

/// A style picker (`None` = plain colors) with a button that creates a
/// style for this slot (starting from `seed`) and one that opens it in the
/// Theme tab.
fn style_picker(
    ui: &mut Ui,
    tr: &Translator,
    salt: &str,
    label: &str,
    styles: &[String],
    slot: &mut Option<String>,
    create: &mut Option<(Slot, String, Style)>,
    open: &mut Option<String>,
    target: Slot,
    seed: impl FnOnce() -> (String, Style),
) {
    row(ui, label, |ui| {
        let mut name = slot.clone().unwrap_or_default();
        let text = if name.is_empty() { tr.t("uied.dlg_plain") } else { name.clone() };
        ComboBox::from_id_salt((salt, "style"))
            .selected_text(text)
            .width(130.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut name, String::new(), tr.t("uied.dlg_plain"));
                for s in styles {
                    ui.selectable_value(&mut name, s.clone(), s);
                }
            });
        *slot = (!name.is_empty()).then_some(name);
        match slot {
            Some(name) => {
                if ui.small_button(icons::PENCIL_SIMPLE).on_hover_text(tr.t("uied.dlg_edit_style")).clicked() {
                    *open = Some(name.clone());
                }
            }
            None => {
                if ui.small_button(icons::PLUS).on_hover_text(tr.t("uied.dlg_new_style")).clicked() {
                    let (base, style) = seed();
                    *create = Some((target, base, style));
                }
            }
        }
    });
}

pub(super) fn image_picker(ui: &mut Ui, tr: &Translator, salt: &str, images: &[String], slot: &mut Option<String>) {
    let mut path = slot.clone().unwrap_or_default();
    let text = if path.is_empty() { tr.t("uied.none") } else { path.clone() };
    ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(160.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut path, String::new(), tr.t("uied.none"));
            for img in images {
                ui.selectable_value(&mut path, img.clone(), img);
            }
        });
    *slot = (!path.is_empty()).then_some(path);
}

/// A plain style seeded from the old colors of a section.
fn seed_style(bg: [u8; 4], text: [u8; 4], hover: Option<([u8; 4], [u8; 4])>) -> Style {
    let mut s = Style::default();
    s.fills.normal = Some(Fill::Color(bg));
    s.text_colors = StateColors {
        normal: Some(text),
        ..Default::default()
    };
    if let Some((bg, text)) = hover {
        s.fills.hover = Some(Fill::Color(bg));
        s.text_colors.hover = Some(text);
    }
    s
}

pub(super) fn dialogue_tab(ui: &mut Ui, ctx: &mut EditorContext<'_>, st: &mut State) {
    let tr = ctx.translator();
    let Some(theme) = st.doc.as_ref().and_then(|d| d.bundle.theme.clone()) else {
        card(ui, |ui| {
            ui.label(RichText::new(tr.t("uied.theme_none_title")).strong());
            ui.label(RichText::new(tr.t("uied.dlg_none_text")).small().color(weak(ui)));
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
    let characters: Vec<(String, String, [u8; 3])> = ctx
        .story()
        .map(|s| {
            s.characters
                .iter()
                .map(|c| {
                    let name = s.strings.get(c.name_id).unwrap_or("?").to_owned();
                    (c.id.to_string(), name, [c.color.r, c.color.g, c.color.b])
                })
                .collect()
        })
        .unwrap_or_default();

    // ---- sample shown by the preview
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr.t("uied.dlg_preview_shows")).small().color(weak(ui)));
            ui.selectable_value(&mut st.dlg_choices, false, tr.t("uied.dlg_show_line"));
            ui.selectable_value(&mut st.dlg_choices, true, tr.t("uied.dlg_show_choices"));
            let on = st.mode == Mode::Dialogue;
            if ui
                .add(egui::Button::selectable(on, format!("{}  {}", icons::EYE, tr.t("uied.preview"))))
                .on_hover_text(tr.t("uied.dlg_preview_tip"))
                .clicked()
            {
                st.mode = if on { Mode::Off } else { Mode::Dialogue };
                if !on {
                    ctx.request(EditorRequest::ShowPanel(panels::PREVIEW.to_owned()));
                }
            }
        });
        if !st.dlg_choices {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut st.dlg_speaker)
                        .hint_text(tr.t("uied.dlg_sample_speaker"))
                        .desired_width(110.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut st.dlg_line)
                        .hint_text(tr.t("uied.dlg_sample_line"))
                        .desired_width(f32::INFINITY),
                );
            });
        }
    });

    let mut t: TextBoxTheme = theme.textbox.clone();
    let mut c: ChoiceTheme = theme.choice.clone();
    let mut q: QuickMenuTheme = theme.quick_menu.clone();
    let mut create: Option<(Slot, String, Style)> = None;
    let mut open: Option<String> = None;

    ScrollArea::vertical()
        .id_salt("uied_dialogue")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.columns(2, |cols| {
                let (left, right) = cols.split_at_mut(1);
                // ---------------- textbox
                let ui = &mut left[0];
                section_title(ui, &tr.t("uied.dlg_mode_title"));
                row(ui, &tr.t("uied.dlg_mode"), |ui| {
                    let label = |m: DialogueMode| match m {
                        DialogueMode::Box => tr.t("uied.dlg_mode_box"),
                        DialogueMode::Bubbles => tr.t("uied.dlg_mode_bubbles"),
                    };
                    ComboBox::from_id_salt("dlg_mode")
                        .selected_text(label(t.mode))
                        .show_ui(ui, |ui| {
                            for m in [DialogueMode::Box, DialogueMode::Bubbles] {
                                ui.selectable_value(&mut t.mode, m, label(m));
                            }
                        });
                });
                if t.mode == DialogueMode::Bubbles {
                    ui.label(RichText::new(tr.t("uied.dlg_mode_tip")).small().italics().color(weak(ui)));
                }
                section_title(ui, &tr.t("uied.dlg_textbox"));
                anchor_editor(ui, &tr, "tb", &mut t.anchor, "uied.dlg_fixed_pos");
                if t.anchor.is_none() {
                    row(ui, &tr.t("uied.dlg_position"), |ui| {
                        num(ui, &mut t.rect[0], 1.0);
                        num(ui, &mut t.rect[1], 1.0);
                    });
                }
                row(ui, &tr.t("uied.dlg_size"), |ui| {
                    num(ui, &mut t.rect[2], 1.0);
                    num(ui, &mut t.rect[3], 1.0);
                });
                row(ui, &tr.t("uied.padding"), |ui| {
                    num(ui, &mut t.padding[0], 0.5);
                    num(ui, &mut t.padding[1], 0.5);
                });
                row(ui, &tr.t("uied.dlg_text_offset"), |ui| num(ui, &mut t.text_offset, 0.5));
                let (bg, tc) = (t.background, t.text_color);
                style_picker(ui, &tr, "tb", &tr.t("uied.dlg_box_style"), &styles, &mut t.style, &mut create, &mut open, Slot::Box, || {
                    ("textbox".to_owned(), seed_style(bg, tc, None))
                });
                if t.style.is_none() {
                    row(ui, &tr.t("uied.dlg_background"), |ui| color(ui, &mut t.background));
                }
                row(ui, &tr.t("uied.font_size"), |ui| num(ui, &mut t.text_size, 0.5));
                row(ui, &tr.t("uied.dlg_text_color"), |ui| color(ui, &mut t.text_color));
                row(ui, &tr.t("uied.dlg_speed"), |ui| {
                    ui.add(DragValue::new(&mut t.chars_per_second).speed(0.5).range(1.0..=1000.0).suffix(" c/s"));
                })
                ;
                if t.style.is_some() {
                    ui.label(RichText::new(tr.t("uied.dlg_style_wins")).small().italics().color(weak(ui)));
                }

                section_title(ui, &tr.t("uied.dlg_name"));
                row(ui, &tr.t("uied.font_size"), |ui| num(ui, &mut t.name_size, 0.5));
                style_picker(ui, &tr, "name", &tr.t("uied.dlg_name_style"), &styles, &mut t.name_style, &mut create, &mut open, Slot::Name, || {
                    let mut s = Style::default();
                    s.bold = Some(true);
                    ("speaker_name".to_owned(), s)
                });
                row(ui, "", |ui| {
                    ui.checkbox(&mut t.speaker_color, tr.t("uied.dlg_speaker_color"))
                        .on_hover_text(tr.t("uied.dlg_speaker_color_tip"));
                });
                let mut plate_on = t.name_plate.is_some();
                row(ui, "", |ui| {
                    if ui.checkbox(&mut plate_on, tr.t("uied.dlg_name_plate")).changed() {
                        t.name_plate = plate_on.then(NamePlate::default);
                    }
                });
                if let Some(plate) = &mut t.name_plate {
                    row(ui, &tr.t("uied.dlg_offset"), |ui| {
                        num(ui, &mut plate.offset[0], 1.0);
                        num(ui, &mut plate.offset[1], 1.0);
                    });
                    row(ui, &tr.t("uied.dlg_size"), |ui| {
                        num(ui, &mut plate.size[0], 1.0);
                        num(ui, &mut plate.size[1], 1.0);
                    });
                    ui.label(RichText::new(tr.t("uied.dlg_plate_fit")).small().italics().color(weak(ui)));
                    row(ui, &tr.t("uied.padding"), |ui| {
                        num(ui, &mut plate.padding[0], 0.5);
                        num(ui, &mut plate.padding[1], 0.5);
                    });
                    let bg = t.background;
                    style_picker(ui, &tr, "plate", &tr.t("uied.dlg_plate_style"), &styles, &mut plate.style, &mut create, &mut open, Slot::Plate, || {
                        ("name_plate".to_owned(), seed_style(bg, [255, 255, 255, 255], None))
                    });
                }

                section_title(ui, &tr.t("uied.dlg_indicator"));
                row(ui, &tr.t("uied.dlg_size"), |ui| num(ui, &mut t.indicator_size, 0.5));
                row(ui, &tr.t("uied.image"), |ui| {
                    image_picker(ui, &tr, "ind_img", &images, &mut t.indicator_image);
                });
                if t.indicator_image.is_none() {
                    row(ui, &tr.t("uied.color"), |ui| color(ui, &mut t.indicator_color));
                }
                row(ui, "", |ui| {
                    ui.checkbox(&mut t.indicator_blink, tr.t("uied.dlg_blink"));
                });

                // ---------------- choices and quick menu
                let ui = &mut right[0];
                section_title(ui, &tr.t("uied.dlg_choices"));
                anchor_editor(ui, &tr, "ch", &mut c.anchor, "uied.dlg_centered");
                row(ui, &tr.t("uied.dlg_button_size"), |ui| {
                    num(ui, &mut c.button_size[0], 1.0);
                    num(ui, &mut c.button_size[1], 1.0);
                });
                row(ui, &tr.t("uied.dlg_spacing"), |ui| num(ui, &mut c.spacing, 0.5));
                let (bc, sc, tc, stc) = (c.button_color, c.selected_color, c.text_color, c.selected_text_color);
                style_picker(ui, &tr, "ch", &tr.t("uied.dlg_button_style"), &styles, &mut c.button_style, &mut create, &mut open, Slot::Choice, || {
                    ("choice_button".to_owned(), seed_style(bc, tc, Some((sc, stc))))
                });
                if c.button_style.is_none() {
                    row(ui, &tr.t("uied.state.normal"), |ui| {
                        color(ui, &mut c.button_color);
                        color(ui, &mut c.text_color);
                    });
                    row(ui, &tr.t("uied.dlg_selected"), |ui| {
                        color(ui, &mut c.selected_color);
                        color(ui, &mut c.selected_text_color);
                    });
                } else {
                    ui.label(RichText::new(tr.t("uied.dlg_choice_states")).small().italics().color(weak(ui)));
                }
                row(ui, &tr.t("uied.font_size"), |ui| num(ui, &mut c.text_size, 0.5));
                row(ui, &tr.t("uied.dlg_overlay"), |ui| color(ui, &mut c.overlay));
                row(ui, "", |ui| {
                    ui.checkbox(&mut c.show_numbers, tr.t("uied.dlg_numbers"));
                });

                section_title(ui, &tr.t("uied.dlg_quick"));
                row(ui, "", |ui| {
                    ui.checkbox(&mut q.hidden, tr.t("uied.dlg_quick_hidden"));
                });
                if !q.hidden {
                    anchor_editor(ui, &tr, "qm", &mut q.anchor, "uied.dlg_quick_default");
                    style_picker(ui, &tr, "qm", &tr.t("uied.dlg_button_style"), &styles, &mut q.button_style, &mut create, &mut open, Slot::Quick, || {
                        ("quick_button".to_owned(), Style::default())
                    });
                    let mut width = q.button_width.unwrap_or(150.0);
                    row(ui, &tr.t("uied.dlg_button_width"), |ui| num(ui, &mut width, 1.0));
                    q.button_width = (width != 150.0).then_some(width);
                    let mut gap = q.gap.unwrap_or(8.0);
                    row(ui, &tr.t("uied.dlg_spacing"), |ui| num(ui, &mut gap, 0.5));
                    q.gap = (gap != 8.0).then_some(gap);
                }

                if t.mode == DialogueMode::Bubbles {
                    bubble_editor(ui, &tr, &mut t.bubble, &fonts, &characters);
                }
            });
        });

    if let Some(name) = open {
        st.style = Some(name);
        st.tab = Tab::Theme;
        if st.mode == Mode::Dialogue {
            st.mode = Mode::Screen;
        }
        return;
    }
    let create_pending = create.is_some();
    let changed = t != theme.textbox || c != theme.choice || q != theme.quick_menu || create_pending;
    if changed && let Some(doc) = st.doc.as_mut() {
        let edit = |b: &mut UiBundle| {
            let Some(th) = b.theme.as_mut() else { return };
            if let Some((target, base, style)) = create {
                let name = model::add_style(th, &base);
                th.styles.insert(name.clone(), style);
                // The new style goes in the slot that asked for it.
                let slot = match target {
                    Slot::Box => &mut t.style,
                    Slot::Name => &mut t.name_style,
                    Slot::Plate => match &mut t.name_plate {
                        Some(plate) => &mut plate.style,
                        None => &mut t.style,
                    },
                    Slot::Choice => &mut c.button_style,
                    Slot::Quick => &mut q.button_style,
                };
                *slot = Some(name);
            }
            th.textbox = t;
            th.choice = c;
            th.quick_menu = q;
        };
        // A created style is its own undo step.
        if create_pending {
            doc.edit(edit);
        } else {
            doc.edit_merged("dialogue", edit);
        }
    }
}

/// The "Speech bubbles" section (`Bubbles` mode): look, placement,
/// narrator and off-screen fallbacks, per-character overrides.
fn bubble_editor(
    ui: &mut Ui,
    tr: &Translator,
    b: &mut BubbleTheme,
    fonts: &[String],
    characters: &[(String, String, [u8; 3])],
) {
    section_title(ui, &tr.t("uied.bub_title"));
    row(ui, &tr.t("uied.bub_colors"), |ui| {
        color(ui, &mut b.fill);
        color(ui, &mut b.border);
    });
    row(ui, &tr.t("uied.bub_border_width"), |ui| {
        ui.add(DragValue::new(&mut b.border_width).speed(0.2).range(0.0..=20.0));
    });
    row(ui, &tr.t("uied.bub_radius"), |ui| {
        ui.add(DragValue::new(&mut b.corner_radius).speed(0.5).range(0.0..=200.0));
    });
    row(ui, &tr.t("uied.padding"), |ui| {
        num(ui, &mut b.padding[0], 0.5);
        num(ui, &mut b.padding[1], 0.5);
    });
    row(ui, &tr.t("uied.bub_width"), |ui| {
        ui.add(DragValue::new(&mut b.min_width).speed(1.0).range(0.0..=1900.0));
        ui.add(DragValue::new(&mut b.max_width).speed(1.0).range(100.0..=1900.0));
    });
    row(ui, &tr.t("uied.font_size"), |ui| num(ui, &mut b.text_size, 0.5));
    row(ui, &tr.t("uied.dlg_text_color"), |ui| color(ui, &mut b.text_color));
    row(ui, &tr.t("uied.bub_font"), |ui| {
        let mut name = b.font.clone().unwrap_or_default();
        let text = if name.is_empty() { tr.t("uied.none") } else { name.clone() };
        ComboBox::from_id_salt("bub_font").selected_text(text).show_ui(ui, |ui| {
            ui.selectable_value(&mut name, String::new(), tr.t("uied.none"));
            for f in fonts {
                ui.selectable_value(&mut name, f.clone(), f);
            }
        });
        b.font = (!name.is_empty()).then_some(name);
    });
    row(ui, "", |ui| {
        ui.checkbox(&mut b.show_name, tr.t("uied.bub_show_name"));
    });
    if b.show_name {
        row(ui, &tr.t("uied.bub_name_size"), |ui| num(ui, &mut b.name_size, 0.5));
    }
    row(ui, &tr.t("uied.bub_tail"), |ui| {
        ui.add(DragValue::new(&mut b.tail_length).speed(0.5).range(0.0..=400.0));
        ui.add(DragValue::new(&mut b.tail_width).speed(0.5).range(2.0..=200.0));
    });
    row(ui, &tr.t("uied.bub_gap"), |ui| {
        ui.add(DragValue::new(&mut b.gap).speed(0.5).range(0.0..=200.0));
    });
    row(ui, &tr.t("uied.bub_margin"), |ui| {
        ui.add(DragValue::new(&mut b.margin).speed(0.5).range(0.0..=300.0));
    });
    row(ui, &tr.t("uied.bub_pop"), |ui| {
        ui.add(DragValue::new(&mut b.pop).speed(0.01).range(0.0..=1.0).suffix(" s"));
    });
    row(ui, "", |ui| {
        ui.checkbox(&mut b.speaker_border, tr.t("uied.bub_speaker_border"));
    });
    row(ui, &tr.t("uied.bub_narrator"), |ui| {
        let label = |n: NarratorStyle| match n {
            NarratorStyle::Box => tr.t("uied.bub_narrator_box"),
            NarratorStyle::Caption => tr.t("uied.bub_narrator_caption"),
        };
        ComboBox::from_id_salt("bub_narrator").selected_text(label(b.narrator)).show_ui(ui, |ui| {
            for n in [NarratorStyle::Box, NarratorStyle::Caption] {
                ui.selectable_value(&mut b.narrator, n, label(n));
            }
        });
    });
    if b.narrator == NarratorStyle::Caption {
        row(ui, &tr.t("uied.bub_caption_fill"), |ui| color(ui, &mut b.caption_fill));
    }
    row(ui, &tr.t("uied.bub_offscreen"), |ui| {
        let label = |o: OffscreenStyle| match o {
            OffscreenStyle::Bubble => tr.t("uied.bub_offscreen_bubble"),
            OffscreenStyle::Box => tr.t("uied.bub_offscreen_box"),
        };
        ComboBox::from_id_salt("bub_offscreen").selected_text(label(b.offscreen)).show_ui(ui, |ui| {
            for o in [OffscreenStyle::Bubble, OffscreenStyle::Box] {
                ui.selectable_value(&mut b.offscreen, o, label(o));
            }
        });
    });

    if characters.is_empty() {
        return;
    }
    section_title(ui, &tr.t("uied.bub_characters"));
    ui.label(RichText::new(tr.t("uied.bub_characters_tip")).small().italics().color(weak(ui)));
    for (id, name, [r, g, bl]) in characters {
        let mut entry = b.characters.get(id).cloned().unwrap_or_default();
        egui::CollapsingHeader::new(RichText::new(name).color(egui::Color32::from_rgb(*r, *g, *bl)))
            .id_salt(("bub_char", id))
            .show(ui, |ui| {
                let opt = |ui: &mut Ui, label: &str, slot: &mut Option<[u8; 4]>, seed: [u8; 4]| {
                    row(ui, label, |ui| {
                        let mut on = slot.is_some();
                        if ui.checkbox(&mut on, "").changed() {
                            *slot = on.then_some(seed);
                        }
                        if let Some(c) = slot {
                            color(ui, c);
                        }
                    });
                };
                // A pale version of the character's color reads well behind text.
                let pale = |c: u8| (c as u16 + 3 * 255).div_ceil(4) as u8;
                opt(ui, &tr.t("uied.bub_fill"), &mut entry.fill, [pale(*r), pale(*g), pale(*bl), 250]);
                opt(ui, &tr.t("uied.bub_border"), &mut entry.border, [*r, *g, *bl, 255]);
                opt(ui, &tr.t("uied.dlg_text_color"), &mut entry.text_color, b.text_color);
                row(ui, &tr.t("uied.bub_head_offset"), |ui| {
                    num(ui, &mut entry.head_offset[0], 1.0);
                    num(ui, &mut entry.head_offset[1], 1.0);
                });
            });
        if entry.is_default() {
            b.characters.remove(id);
        } else {
            b.characters.insert(id.clone(), entry);
        }
    }
}

/// The style slots of the dialogue box section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Box,
    Name,
    Plate,
    Choice,
    Quick,
}
