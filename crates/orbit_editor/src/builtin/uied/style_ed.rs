//! The visual style editor of the UI editor: a live preview of sample
//! widgets in the state being edited, state tabs, per-state backgrounds of
//! any kind (color, gradient, image, nine-slice with draggable guides),
//! border, shadow, padding, text and sounds. Inherited fields are shown
//! with the parent's value; fields the style sets are highlighted and can
//! be reset to the parent.

use std::collections::HashMap;
use std::path::PathBuf;

use egui::epaint::Mesh;
use egui::{
    Align2, Color32, ComboBox, CornerRadius, CursorIcon, DragValue, FontId, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};
use orbit_core::AssetEntry;
use orbit_editor_api::{EditorContext, Waker, icons};
use orbit_ui::{
    Border, Fill, ImageFit, Shadow, Style, StyleSounds, TextAlignment, WidgetState, WidgetTheme,
};

use super::props::{Env, color, num, pick};
use crate::builtin::ui_util::{accent, weak};

/// Preview scale: game pixels to editor points.
const SCALE: f32 = 0.5;

// ------------------------------------------------------------------ images

/// Textures of the project images (thumbnails, decoded off the frame) by
/// asset path, with the size of the original picture.
pub struct ImageLookup {
    waker: Option<Waker>,
    root: Option<PathBuf>,
    entries: HashMap<String, AssetEntry>,
}

impl ImageLookup {
    pub fn new(ctx: &EditorContext<'_>) -> Self {
        let Some(story) = ctx.story() else {
            return Self::empty();
        };
        Self {
            waker: Some(ctx.state().waker.clone()),
            root: crate::builtin::thumbs::assets_root(ctx, story),
            entries: story
                .assets
                .assets
                .iter()
                .filter(|a| a.kind == orbit_core::AssetKind::Image)
                .map(|a| (a.path.clone(), a.clone()))
                .collect(),
        }
    }

    pub fn empty() -> Self {
        Self {
            waker: None,
            root: None,
            entries: HashMap::new(),
        }
    }

    /// The texture of `path` and the `[width, height]` of the picture, when
    /// it is ready (asks for it otherwise).
    pub fn get(&self, egui_ctx: &egui::Context, path: &str) -> Option<(egui::TextureHandle, [f32; 2])> {
        let entry = self.entries.get(path)?;
        let (waker, root) = (self.waker.as_ref()?, self.root.as_ref()?);
        match crate::builtin::thumbs::state(egui_ctx, waker, root, entry) {
            crate::builtin::thumbs::ThumbState::Ready {
                texture: Some(texture),
                info,
            } => {
                let size = parse_size(&info).unwrap_or_else(|| {
                    let s = texture.size();
                    [s[0] as f32, s[1] as f32]
                });
                Some((texture, size))
            }
            _ => None,
        }
    }
}

/// `"800×400"` -> `[800, 400]`.
fn parse_size(info: &str) -> Option<[f32; 2]> {
    let (w, h) = info.split_once('×')?;
    Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
}

fn c32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

// ----------------------------------------------------------------- preview

/// Paints `fill` in `rect` (editor points; `scale` maps game pixels to
/// points for nine-slice borders). Returns `false` when its image is not
/// ready.
fn paint_fill(ui: &Ui, images: &ImageLookup, fill: &Fill, rect: Rect, scale: f32) -> bool {
    let painter = ui.painter();
    match fill {
        Fill::Color(c) => {
            painter.rect_filled(rect, CornerRadius::ZERO, c32(*c));
            true
        }
        Fill::Gradient { top, bottom } => {
            let mut mesh = Mesh::default();
            mesh.colored_vertex(rect.left_top(), c32(*top));
            mesh.colored_vertex(rect.right_top(), c32(*top));
            mesh.colored_vertex(rect.right_bottom(), c32(*bottom));
            mesh.colored_vertex(rect.left_bottom(), c32(*bottom));
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
            true
        }
        Fill::Image { image, fit } => {
            let Some((tex, size)) = images.get(ui.ctx(), image) else {
                return false;
            };
            let (r, uv) = fit.place([rect.min.x, rect.min.y, rect.width(), rect.height()], size);
            painter.image(
                tex.id(),
                Rect::from_min_size(pos2(r[0], r[1]), vec2(r[2], r[3])),
                Rect::from_min_max(pos2(uv[0], uv[1]), pos2(uv[2], uv[3])),
                Color32::WHITE,
            );
            true
        }
        Fill::NineSlice { image, insets } => {
            let Some((tex, size)) = images.get(ui.ctx(), image) else {
                return false;
            };
            // Cut in game pixels, then scaled down to the preview.
            let game = [0.0, 0.0, rect.width() / scale, rect.height() / scale];
            for (q, uv) in orbit_render2d::nine_slice_quads(game, size, *insets) {
                let r = Rect::from_min_size(
                    rect.min + vec2(q[0] * scale, q[1] * scale),
                    vec2(q[2] * scale, q[3] * scale),
                );
                painter.image(
                    tex.id(),
                    r,
                    Rect::from_min_max(pos2(uv[0], uv[1]), pos2(uv[2], uv[3])),
                    Color32::WHITE,
                );
            }
            true
        }
    }
}

/// The background of a sample widget, as the game draws it.
fn paint_background(ui: &Ui, images: &ImageLookup, style: &Style, state: WidgetState, rect: Rect, default: [u8; 4]) {
    if let Some(shadow) = &style.shadow {
        let r = rect.translate(vec2(shadow.offset[0], shadow.offset[1]) * SCALE);
        ui.painter().rect_filled(r, CornerRadius::ZERO, c32(shadow.color));
    }
    let fill = style.fill_for(state);
    let sliced = match &fill {
        Some(f) if paint_fill(ui, images, f, rect, SCALE) => matches!(f, Fill::NineSlice { .. }),
        _ => {
            let c = style
                .colors
                .pick_state(state)
                .or(style.fill.as_ref().and_then(Fill::flat_color))
                .unwrap_or(default);
            ui.painter().rect_filled(rect, CornerRadius::ZERO, c32(c));
            false
        }
    };
    if !sliced && let Some(b) = &style.border {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new((b.width * SCALE).max(0.5), c32(b.color)),
            StrokeKind::Inside,
        );
    }
}

fn paint_text(ui: &Ui, style: &Style, state: WidgetState, rect: Rect, text: &str, default_color: [u8; 4], size: f32, align: TextAlignment) {
    if !style.text_visible() {
        return;
    }
    let color = style.text_color_for(state).unwrap_or(default_color);
    let size = style.text_size.unwrap_or(size) * SCALE;
    let pad = style.padding.map_or(15.0, |p| p[0]) * SCALE;
    let (anchor, x) = match style.text_align.unwrap_or(align) {
        TextAlignment::Left => (Align2::LEFT_CENTER, rect.left() + pad),
        TextAlignment::Center => (Align2::CENTER_CENTER, rect.center().x),
        TextAlignment::Right => (Align2::RIGHT_CENTER, rect.right() - pad),
    };
    let font = FontId::proportional(size.max(6.0));
    let pos = pos2(x, rect.center().y);
    if style.bold == Some(true) {
        // A poor man's bold: the text twice, a hair apart.
        ui.painter().text(pos + vec2(0.6, 0.0), anchor, &text, font.clone(), c32(color));
    }
    ui.painter().text(pos, anchor, text, font, c32(color));
}

/// Sample widgets drawn with `style` (resolved) in `state`: button, label,
/// panel, slider, toggle and dropdown, sized like the game's.
pub fn preview(ui: &mut Ui, env: &Env, images: &ImageLookup, style: &Style, state: WidgetState) {
    let w = env.theme.map_or_else(WidgetTheme::default, |t| t.widgets.clone());
    let bt = &w.button;
    let checker = ui.visuals().extreme_bg_color;
    let width = ui.available_width().max(200.0);
    let (area, _) = ui.allocate_exact_size(vec2(width, 210.0), Sense::hover());
    ui.painter().rect_filled(area, CornerRadius::same(6), checker);
    let default_bg = match state {
        WidgetState::Normal => bt.normal,
        WidgetState::Hover | WidgetState::Focused => bt.hover,
        WidgetState::Pressed => bt.pressed,
        WidgetState::Disabled => bt.disabled,
    };
    let default_text = match state {
        WidgetState::Disabled => bt.disabled_text_color,
        WidgetState::Hover | WidgetState::Focused | WidgetState::Pressed => bt.hover_text_color,
        WidgetState::Normal => bt.text_color,
    };
    let col = (width - 24.0) / 2.0;
    let cell = |i: usize, h: f32| {
        let (cx, cy) = (i % 2, i / 2);
        Rect::from_min_size(
            area.min + vec2(8.0 + cx as f32 * (col + 8.0), 8.0 + cy as f32 * 66.0),
            vec2(col, h),
        )
    };
    let label = |key: &str| env.tr.t(key);
    // Button.
    let r = cell(0, bt.height * SCALE);
    paint_background(ui, images, style, state, r, default_bg);
    if state == WidgetState::Focused {
        ui.painter().rect_stroke(r, CornerRadius::ZERO, Stroke::new(bt.focus_width * SCALE, c32(bt.focus_color)), StrokeKind::Outside);
    }
    paint_text(ui, style, state, r, &label("uied.sample.button"), default_text, w.text_size, TextAlignment::Center);
    // Label (text only).
    let r = cell(1, bt.height * SCALE);
    let still = if state == WidgetState::Disabled { WidgetState::Disabled } else { WidgetState::Normal };
    let mut label_style = style.clone();
    label_style.show_text = None;
    paint_text(ui, &label_style, still, r, &label("uied.sample.label"), w.text_color, w.text_size, TextAlignment::Left);
    // Panel.
    let r = cell(2, 56.0);
    paint_background(ui, images, style, still, r, w.panel.background);
    if style.fill_for(still).is_none() && style.border.is_none() {
        ui.painter().rect_stroke(
            r,
            CornerRadius::ZERO,
            Stroke::new(w.panel.border_width * SCALE, c32(w.panel.border_color)),
            StrokeKind::Inside,
        );
    }
    paint_text(ui, &label_style, still, r, &label("uied.sample.panel"), w.text_color, w.small_text_size, TextAlignment::Center);
    // Slider (the theme draws it; the style only sets the text around).
    let r = cell(3, 56.0);
    let c = &w.controls;
    let track = Rect::from_center_size(r.center(), vec2(r.width(), c.track_height * SCALE));
    ui.painter().rect_filled(track, CornerRadius::ZERO, c32(c.track));
    let fill = Rect::from_min_max(track.min, pos2(track.left() + track.width() * 0.6, track.max.y));
    ui.painter().rect_filled(fill, CornerRadius::ZERO, c32(c.fill));
    let knob = Rect::from_center_size(pos2(fill.right(), r.center().y), vec2(c.knob_size[0], c.knob_size[1]) * SCALE);
    ui.painter().rect_filled(knob, CornerRadius::ZERO, c32(c.knob));
    // Toggle.
    let r = cell(4, c.height * SCALE);
    let bx = Rect::from_min_size(pos2(r.left(), r.center().y - c.toggle_size * SCALE * 0.5), vec2(c.toggle_size, c.toggle_size) * SCALE);
    ui.painter().rect_filled(bx, CornerRadius::ZERO, c32(c.toggle_off));
    ui.painter().rect_filled(bx.shrink(c.toggle_size * SCALE * 0.22), CornerRadius::ZERO, c32(c.toggle_on));
    let tr = Rect::from_min_max(pos2(bx.right() + 10.0, r.top()), r.max);
    paint_text(ui, &label_style, still, tr, &label("uied.sample.toggle"), w.text_color, w.text_size, TextAlignment::Left);
    // Dropdown.
    let r = cell(5, bt.height * SCALE);
    let dd_bg = if matches!(state, WidgetState::Hover | WidgetState::Focused) { bt.hover } else { bt.normal };
    paint_background(ui, images, style, state, r, dd_bg);
    paint_text(ui, style, state, r, &format!("{}  v", label("uied.sample.dropdown")), default_text, w.text_size, TextAlignment::Center);
}

// ------------------------------------------------------------------ fields

/// A field row: the label is highlighted when the style sets the field;
/// a reset button clears it (back to the parent's value, or the default).
fn field_row(ui: &mut Ui, env: &Env, label: &str, set: bool, has_parent: bool, add: impl FnOnce(&mut Ui) -> bool) -> bool {
    let mut reset = false;
    ui.horizontal(|ui| {
        let text = RichText::new(label).small();
        let text = if set { text.color(accent(ui)).strong() } else { text.color(weak(ui)) };
        ui.add_sized([96.0, 20.0], egui::Label::new(text).truncate());
        add(ui);
        if set {
            let tip = if has_parent { env.tr.t("uied.style_reset_parent") } else { env.tr.t("uied.style_clear") };
            if ui.small_button(icons::ARROW_COUNTER_CLOCKWISE).on_hover_text(tip).clicked() {
                reset = true;
            }
        } else if has_parent {
            ui.label(RichText::new(env.tr.t("uied.style_inherited")).small().italics().color(weak(ui)));
        }
    });
    reset
}

/// An optional value of the style: edited in place; while unset the
/// parent's (or default) value shows, and editing it sets it.
fn opt<T: Clone + PartialEq>(
    ui: &mut Ui,
    env: &Env,
    label: &str,
    own: &mut Option<T>,
    inherited: Option<T>,
    has_parent: bool,
    default: T,
    edit: impl FnOnce(&mut Ui, &mut T),
) {
    let set = own.is_some();
    let mut value = own.clone().or(inherited).unwrap_or(default);
    let before = value.clone();
    let reset = field_row(ui, env, label, set, has_parent, |ui| {
        edit(ui, &mut value);
        true
    });
    if reset {
        *own = None;
    } else if value != before || set {
        *own = Some(value);
    }
}

/// Kind of a background, for the picker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FillKind {
    None,
    Color,
    Gradient,
    Image,
    NineSlice,
}

fn fill_kind(fill: Option<&Fill>) -> FillKind {
    match fill {
        None => FillKind::None,
        Some(Fill::Color(_)) => FillKind::Color,
        Some(Fill::Gradient { .. }) => FillKind::Gradient,
        Some(Fill::Image { .. }) => FillKind::Image,
        Some(Fill::NineSlice { .. }) => FillKind::NineSlice,
    }
}

fn image_combo(ui: &mut Ui, env: &Env, salt: &str, image: &mut String) {
    let text = if image.is_empty() { env.tr.t("uied.none") } else { image.clone() };
    ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(170.0)
        .show_ui(ui, |ui| {
            for img in env.images {
                ui.selectable_value(image, img.clone(), img);
            }
        });
}

/// Edits one background (any kind); `inherited` is what shows while it is
/// unset. Returns the new value.
fn fill_editor(ui: &mut Ui, env: &Env, images: &ImageLookup, salt: &str, label: &str, own: &mut Option<Fill>, inherited: Option<&Fill>, has_parent: bool) {
    let set = own.is_some();
    let mut kind = fill_kind(own.as_ref().or(inherited));
    let before = kind;
    let reset = field_row(ui, env, label, set, has_parent, |ui| {
        pick(
            ui,
            (salt, "kind"),
            &mut kind,
            &[
                (FillKind::None, env.tr.t("uied.fill_none")),
                (FillKind::Color, env.tr.t("uied.v.fill_color")),
                (FillKind::Gradient, env.tr.t("uied.fill_gradient")),
                (FillKind::Image, env.tr.t("uied.fill_image")),
                (FillKind::NineSlice, env.tr.t("uied.v.fill_nine_slice")),
            ],
        );
        true
    });
    if reset {
        *own = None;
        return;
    }
    if kind != before {
        let first = env.images.first().cloned().unwrap_or_default();
        *own = match kind {
            FillKind::None => None,
            FillKind::Color => Some(Fill::Color([40, 44, 66, 255])),
            FillKind::Gradient => Some(Fill::Gradient {
                top: [70, 110, 210, 255],
                bottom: [14, 18, 40, 255],
            }),
            FillKind::Image => Some(Fill::Image {
                image: first,
                fit: ImageFit::Stretch,
            }),
            FillKind::NineSlice => Some(Fill::NineSlice {
                image: first,
                insets: [16.0; 4],
            }),
        };
    }
    // The value edited: the own one, else a copy of the inherited one
    // (editing it makes it the style's own).
    let mut value = own.clone().or_else(|| inherited.cloned());
    let snapshot = value.clone();
    ui.indent((salt, "fill_body"), |ui| match &mut value {
        Some(Fill::Color(c)) => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(env.tr.t("uied.color")).small().color(weak(ui)));
                color(ui, c);
            });
        }
        Some(Fill::Gradient { top, bottom }) => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(env.tr.t("uied.gradient_top")).small().color(weak(ui)));
                color(ui, top);
                ui.label(RichText::new(env.tr.t("uied.gradient_bottom")).small().color(weak(ui)));
                color(ui, bottom);
            });
        }
        Some(Fill::Image { image, fit }) => {
            ui.horizontal(|ui| {
                image_combo(ui, env, &format!("{salt}_img"), image);
                pick(
                    ui,
                    (salt, "fit"),
                    fit,
                    &[
                        (ImageFit::Stretch, env.tr.t("uied.fit_stretch")),
                        (ImageFit::Fit, env.tr.t("uied.fit_fit")),
                        (ImageFit::Cover, env.tr.t("uied.fit_cover")),
                    ],
                );
            });
        }
        Some(Fill::NineSlice { image, insets }) => {
            image_combo(ui, env, &format!("{salt}_nine"), image);
            nine_slice_editor(ui, env, images, salt, image, insets);
        }
        None => {}
    });
    if value != snapshot {
        *own = value;
    }
}

/// The nine-slice insets over the picture: four guides to drag (left, top,
/// right, bottom borders, in pixels of the image), and their values.
pub fn nine_slice_editor(ui: &mut Ui, env: &Env, images: &ImageLookup, salt: &str, image: &str, insets: &mut [f32; 4]) {
    match images.get(ui.ctx(), image) {
        None => {
            ui.label(RichText::new(env.tr.t("uied.nine_loading")).small().color(weak(ui)));
        }
        Some((tex, size)) => {
            let max = vec2(ui.available_width().clamp(80.0, 320.0), 180.0);
            let scale = (max.x / size[0]).min(max.y / size[1]).min(8.0);
            let (rect, _) = ui.allocate_exact_size(vec2(size[0] * scale, size[1] * scale), Sense::hover());
            let painter = ui.painter_at(rect.expand(4.0));
            painter.rect_filled(rect, CornerRadius::ZERO, ui.visuals().extreme_bg_color);
            painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let accent = accent(ui);
            for i in 0..4 {
                let vertical = i % 2 == 0;
                let pos = match i {
                    0 => rect.left() + insets[0] * scale,
                    1 => rect.top() + insets[1] * scale,
                    2 => rect.right() - insets[2] * scale,
                    _ => rect.bottom() - insets[3] * scale,
                };
                let hit = if vertical {
                    Rect::from_min_max(pos2(pos - 4.0, rect.top()), pos2(pos + 4.0, rect.bottom()))
                } else {
                    Rect::from_min_max(pos2(rect.left(), pos - 4.0), pos2(rect.right(), pos + 4.0))
                };
                let resp = ui
                    .interact(hit, ui.id().with((salt, "guide", i)), Sense::drag())
                    .on_hover_cursor(if vertical { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
                if resp.dragged() {
                    let d = resp.drag_delta() / scale;
                    let (delta, limit) = match i {
                        0 => (d.x, size[0]),
                        1 => (d.y, size[1]),
                        2 => (-d.x, size[0]),
                        _ => (-d.y, size[1]),
                    };
                    insets[i] = (insets[i] + delta).clamp(0.0, limit);
                }
                let strong = resp.hovered() || resp.dragged();
                let stroke = Stroke::new(if strong { 2.0 } else { 1.0 }, accent);
                if vertical {
                    painter.line_segment([pos2(pos, rect.top()), pos2(pos, rect.bottom())], stroke);
                } else {
                    painter.line_segment([pos2(rect.left(), pos), pos2(rect.right(), pos)], stroke);
                }
            }
            if ui.input(|i| i.pointer.any_released()) {
                for v in insets.iter_mut() {
                    *v = v.round();
                }
            }
        }
    }
    ui.horizontal_wrapped(|ui| {
        for (key, i) in [("uied.inset_left", 0), ("uied.inset_top", 1), ("uied.inset_right", 2), ("uied.inset_bottom", 3)] {
            ui.label(RichText::new(env.tr.t(key)).small().color(weak(ui)));
            ui.add(DragValue::new(&mut insets[i]).speed(0.5).range(0.0..=4096.0));
        }
    });
}

fn state_label(env: &Env, s: WidgetState) -> String {
    env.tr.t(&format!("uied.state.{}", s.name()))
}

/// The style editor. `parent` is the resolved style `s` extends (or that a
/// widget's overrides sit on), `exclude` the style's own name (it cannot
/// extend itself), `salt` keeps the widgets of two editors apart.
#[allow(clippy::too_many_arguments)]
pub fn style_editor(
    ui: &mut Ui,
    env: &Env,
    images: &ImageLookup,
    s: &mut Style,
    parent: Option<&Style>,
    exclude: Option<&str>,
    salt: &str,
    with_extends: bool,
) {
    let has_parent = parent.is_some();
    let empty = Style::default();
    let p = parent.unwrap_or(&empty);
    let state_id = ui.id().with((salt, "style_state"));
    let mut state: WidgetState = ui.data(|d| d.get_temp(state_id)).unwrap_or(WidgetState::Normal);

    // ---- inheritance
    if with_extends {
        let set = s.extends.is_some();
        let mut name = s.extends.clone().unwrap_or_default();
        let reset = field_row(ui, env, &env.tr.t("uied.extends"), set, false, |ui| {
            let text = if name.is_empty() { env.tr.t("uied.none") } else { name.clone() };
            ComboBox::from_id_salt((salt, "extends"))
                .selected_text(text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut name, String::new(), env.tr.t("uied.none"));
                    for other in env.styles.iter().filter(|o| Some(o.as_str()) != exclude) {
                        ui.selectable_value(&mut name, other.clone(), other);
                    }
                });
            true
        });
        s.extends = if reset { None } else { (!name.is_empty()).then_some(name) };
    }
    if has_parent {
        let set: Vec<&str> = orbit_ui::STYLE_FIELDS
            .iter()
            .copied()
            .filter(|f| *f != "extends" && s.sets(f))
            .collect();
        ui.label(
            RichText::new(env.tr.tf("uied.style_overrides_n", &[("n", &set.len())]))
                .small()
                .color(weak(ui)),
        );
    }

    // ---- state tabs + live preview
    ui.horizontal_wrapped(|ui| {
        for st in WidgetState::ALL {
            ui.selectable_value(&mut state, st, state_label(env, st));
        }
    });
    ui.data_mut(|d| d.insert_temp(state_id, state));
    let resolved = s.over(p);
    preview(ui, env, images, &resolved, state);
    ui.add_space(6.0);

    // ---- background of the state
    ui.label(RichText::new(env.tr.tf("uied.bg_of_state", &[("state", &state_label(env, state))])).strong());
    let inherited_fill = p.fills.get(state).cloned();
    fill_editor(
        ui,
        env,
        images,
        &format!("{salt}_fill_{}", state.name()),
        &env.tr.t("uied.fill"),
        s.fills.get_mut(state),
        inherited_fill.as_ref(),
        has_parent,
    );
    if state != WidgetState::Normal && s.fills.get(state).is_none() && inherited_fill.is_none() {
        ui.label(RichText::new(env.tr.t("uied.fill_falls_back")).small().italics().color(weak(ui)));
    }
    // The pre-v7 fields, when the style still has them.
    if s.fill.is_some() || s.colors != Default::default() {
        crate::builtin::ui_util::card(ui, |ui| {
            ui.label(RichText::new(env.tr.t("uied.legacy_bg")).small());
            if ui.button(env.tr.t("uied.legacy_bg_convert")).clicked() {
                s.modernize_background();
            }
        });
    }

    // ---- border, shadow, padding
    ui.add_space(4.0);
    ui.label(RichText::new(env.tr.t("uied.box")).strong());
    opt(ui, env, &env.tr.t("uied.border"), &mut s.border, p.border.clone(), has_parent, Border { color: [255, 255, 255, 80], width: 2.0 }, |ui, b| {
        color(ui, &mut b.color);
        ui.add(DragValue::new(&mut b.width).speed(0.1).range(0.0..=64.0).suffix(" px"));
    });
    opt(ui, env, &env.tr.t("uied.shadow"), &mut s.shadow, p.shadow.clone(), has_parent, Shadow { color: [0, 0, 0, 120], offset: [6.0, 6.0] }, |ui, sh| {
        color(ui, &mut sh.color);
        num(ui, &mut sh.offset[0], 0.5);
        num(ui, &mut sh.offset[1], 0.5);
    });
    opt(ui, env, &env.tr.t("uied.padding"), &mut s.padding, p.padding, has_parent, [20.0, 12.0], |ui, v| {
        num(ui, &mut v[0], 1.0);
        num(ui, &mut v[1], 1.0);
    });
    ui.label(RichText::new(env.tr.t("uied.margins_hint")).small().italics().color(weak(ui)));

    // ---- text
    ui.add_space(4.0);
    ui.label(RichText::new(env.tr.t("uied.text_section")).strong());
    opt(ui, env, &env.tr.t("uied.font"), &mut s.font, p.font.clone(), has_parent, String::new(), |ui, font| {
        if !env.fonts.is_empty() {
            ComboBox::from_id_salt((salt, "font"))
                .selected_text(if font.is_empty() { env.tr.t("uied.font_default") } else { font.clone() })
                .show_ui(ui, |ui| {
                    ui.selectable_value(font, String::new(), env.tr.t("uied.font_default"));
                    for name in env.fonts {
                        ui.selectable_value(font, name.clone(), name);
                    }
                });
        }
        ui.add(egui::TextEdit::singleline(font).desired_width(90.0));
    });
    if s.font.as_deref() == Some("") {
        s.font = None;
    }
    opt(ui, env, &env.tr.t("uied.font_size"), &mut s.text_size, p.text_size, has_parent, 30.0, |ui, v| {
        ui.add(DragValue::new(v).speed(0.5).range(4.0..=400.0));
    });
    opt(ui, env, &env.tr.t("uied.bold"), &mut s.bold, p.bold, has_parent, false, |ui, b| {
        ui.checkbox(b, "");
    });
    opt(ui, env, &env.tr.t("uied.text_align"), &mut s.text_align, p.text_align, has_parent, TextAlignment::Center, |ui, a| {
        pick(
            ui,
            (salt, "align"),
            a,
            &[
                (TextAlignment::Left, env.tr.t("uied.align_left")),
                (TextAlignment::Center, env.tr.t("uied.align_center")),
                (TextAlignment::Right, env.tr.t("uied.align_right")),
            ],
        );
    });
    let label = env.tr.tf("uied.text_color_of_state", &[("state", &state_label(env, state))]);
    let inherited = p.text_colors.get(state);
    opt(ui, env, &label, s.text_colors.get_mut(state), inherited, has_parent, [235, 238, 248, 255], |ui, c| {
        color(ui, c);
    });
    opt(ui, env, &env.tr.t("uied.show_text"), &mut s.show_text, p.show_text, has_parent, true, |ui, b| {
        ui.checkbox(b, "").on_hover_text(env.tr.t("uied.show_text_tip"));
    });

    // ---- sounds
    ui.add_space(4.0);
    ui.label(RichText::new(env.tr.t("uied.sounds")).strong());
    let inherited_sounds = p.sounds.clone();
    let set = s.sounds.is_some();
    let mut sounds = s.sounds.clone().or(inherited_sounds).unwrap_or_default();
    let before = sounds.clone();
    let reset_hover = field_row(ui, env, &env.tr.t("uied.sound_hover"), set, has_parent, |ui| {
        sound_combo(ui, env, (salt, "snd_hover"), &mut sounds.hover);
        true
    });
    let reset = reset_hover || field_row(ui, env, &env.tr.t("uied.sound_confirm"), set, has_parent, |ui| {
        sound_combo(ui, env, (salt, "snd_confirm"), &mut sounds.confirm);
        true
    });
    if reset {
        s.sounds = None;
    } else if sounds != before {
        s.sounds = (sounds != StyleSounds::default()).then_some(sounds);
    }
}

fn sound_combo(ui: &mut Ui, env: &Env, salt: impl std::hash::Hash + std::fmt::Debug, slot: &mut Option<String>) {
    let mut path = slot.clone().unwrap_or_default();
    let text = if path.is_empty() { env.tr.t("uied.none") } else { path.clone() };
    ComboBox::from_id_salt(salt)
        .selected_text(text)
        .width(150.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut path, String::new(), env.tr.t("uied.none"));
            for s in env.sounds {
                ui.selectable_value(&mut path, s.clone(), s);
            }
        });
    *slot = (!path.is_empty()).then_some(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_infos_give_the_picture_size() {
        assert_eq!(parse_size("800×400"), Some([800.0, 400.0]));
        assert_eq!(parse_size("0:12"), None);
    }
}
