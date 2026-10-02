//! Small egui helpers shared by the built-in panels. They only read the
//! egui style (never the editor's `Theme`), like any third-party plugin
//! would.

use egui::{Color32, CornerRadius, Frame, Margin, Response, RichText, Sense, Stroke, Ui, vec2};

use crate::theme::bold;

/// The accent color (selection stroke of the style).
pub fn accent(ui: &Ui) -> Color32 {
    ui.visuals().selection.stroke.color
}

pub fn weak(ui: &Ui) -> Color32 {
    ui.visuals().weak_text_color()
}

pub fn faint(ui: &Ui) -> Color32 {
    ui.visuals().weak_text_color().gamma_multiply(0.7)
}

pub fn danger(ui: &Ui) -> Color32 {
    ui.visuals().error_fg_color
}

pub fn warning(ui: &Ui) -> Color32 {
    ui.visuals().warn_fg_color
}

pub fn success() -> Color32 {
    Color32::from_rgb(76, 195, 138)
}

/// Soft accent background for selected rows.
pub fn accent_soft(ui: &Ui, alpha: u8) -> Color32 {
    let c = accent(ui);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

/// Bold text.
pub fn strong(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(bold(size))
}

/// Section title: small, bold, weak.
pub fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .font(bold(10.5))
            .color(faint(ui)),
    );
    ui.add_space(2.0);
}

/// A raised card.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let fill = ui.visuals().widgets.inactive.bg_fill;
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(14))
        .show(ui, add)
        .inner
}

/// A flat icon button with a tooltip.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    let button = egui::Button::new(RichText::new(icon).size(15.0))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .min_size(vec2(26.0, 26.0));
    ui.add(button).on_hover_text(tooltip)
}

/// The primary (accent) call-to-action button.
pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let button = egui::Button::new(RichText::new(text).color(Color32::WHITE).font(bold(13.5)))
        .fill(accent(ui))
        .min_size(vec2(0.0, 30.0))
        .corner_radius(CornerRadius::same(8));
    ui.add(button)
}

pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    let button = egui::Button::new(text).min_size(vec2(0.0, 30.0));
    ui.add(button)
}

/// A full-width clickable row of a fixed height. Paints the hover / selected
/// background and returns the rect to draw the contents into.
pub fn list_row(ui: &mut Ui, height: f32, selected: bool) -> (egui::Rect, Response) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if selected || response.hovered() {
        let fill = if selected {
            accent_soft(ui, 70)
        } else {
            ui.visuals().widgets.hovered.bg_fill
        };
        ui.painter()
            .rect_filled(rect.shrink2(vec2(2.0, 0.0)), CornerRadius::same(5), fill);
    }
    (rect, response)
}
