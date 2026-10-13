//! The few egui helpers of the editor's built-in panels the studio uses
//! (copied: a plugin only sees the egui style, like the built-ins do).

use orbit_editor_api::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Response, RichText, Ui, vec2,
};

/// The editor's bold font (registered by the editor as the `bold` family).
fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

fn accent(ui: &Ui) -> Color32 {
    ui.visuals().selection.stroke.color
}

pub fn weak(ui: &Ui) -> Color32 {
    ui.visuals().weak_text_color()
}

fn faint(ui: &Ui) -> Color32 {
    ui.visuals().weak_text_color().gamma_multiply(0.7)
}

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
