//! The editor's own look: a sober palette with one accent, rounded corners,
//! a 4 px spacing grid, Inter for text and Phosphor for icons.
//!
//! The theme is plain data (RON-serializable) so that it can be tweaked and
//! compared in visual tests; [`Theme::apply`] turns it into egui `Visuals`
//! and `Style`.

use std::sync::Arc;

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow,
    Stroke, Style, TextStyle, Vec2, Visuals,
};
use orbit_editor_api::ThemeMode;
use serde::{Deserialize, Serialize};

/// Font family used for headings and emphasis (egui has no font weights).
pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// `FontId` for bold text of a given size.
pub fn bold(size: f32) -> FontId {
    FontId::new(size, bold_family())
}

type Rgb = [u8; 3];

fn c(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Colors and metrics of the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub mode: ThemeMode,
    /// Window background, behind panels and under inputs.
    pub bg: Rgb,
    /// Panels and tab bodies.
    pub panel: Rgb,
    /// Raised surfaces: buttons, popups, cards.
    pub surface: Rgb,
    pub surface_hover: Rgb,
    pub surface_active: Rgb,
    pub border: Rgb,
    pub text: Rgb,
    pub text_weak: Rgb,
    pub text_faint: Rgb,
    pub accent: Rgb,
    pub accent_hover: Rgb,
    /// Text drawn on the accent color.
    pub accent_text: Rgb,
    pub danger: Rgb,
    pub warning: Rgb,
    pub success: Rgb,
    /// Corner radius of controls, in points.
    pub rounding: u8,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            bg: [0x13, 0x15, 0x19],
            panel: [0x1a, 0x1d, 0x23],
            surface: [0x23, 0x27, 0x30],
            surface_hover: [0x2c, 0x31, 0x3d],
            surface_active: [0x36, 0x3c, 0x4a],
            border: [0x2b, 0x30, 0x3a],
            text: [0xe4, 0xe7, 0xec],
            text_weak: [0x9a, 0xa2, 0xaf],
            text_faint: [0x6b, 0x73, 0x80],
            accent: [0x5b, 0x8d, 0xf0],
            accent_hover: [0x7a, 0xa3, 0xf5],
            accent_text: [0xff, 0xff, 0xff],
            danger: [0xef, 0x5b, 0x5b],
            warning: [0xe5, 0xa9, 0x3d],
            success: [0x4c, 0xc3, 0x8a],
            rounding: 6,
        }
    }

    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            bg: [0xf1, 0xf3, 0xf6],
            panel: [0xff, 0xff, 0xff],
            surface: [0xf0, 0xf2, 0xf6],
            surface_hover: [0xe7, 0xea, 0xf0],
            surface_active: [0xdc, 0xe1, 0xea],
            border: [0xe0, 0xe4, 0xeb],
            text: [0x1c, 0x20, 0x28],
            text_weak: [0x59, 0x62, 0x70],
            text_faint: [0x8b, 0x93, 0xa1],
            accent: [0x35, 0x6a, 0xdf],
            accent_hover: [0x2a, 0x58, 0xc4],
            accent_text: [0xff, 0xff, 0xff],
            danger: [0xd6, 0x3b, 0x3b],
            warning: [0xb9, 0x7a, 0x0a],
            success: [0x1f, 0x9a, 0x62],
            rounding: 6,
        }
    }

    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    pub fn from_ron(source: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(source)
    }

    // Color accessors for UI code.
    pub fn bg(&self) -> Color32 {
        c(self.bg)
    }
    pub fn panel(&self) -> Color32 {
        c(self.panel)
    }
    pub fn surface(&self) -> Color32 {
        c(self.surface)
    }
    pub fn surface_hover(&self) -> Color32 {
        c(self.surface_hover)
    }
    pub fn border(&self) -> Color32 {
        c(self.border)
    }
    pub fn text(&self) -> Color32 {
        c(self.text)
    }
    pub fn text_weak(&self) -> Color32 {
        c(self.text_weak)
    }
    pub fn text_faint(&self) -> Color32 {
        c(self.text_faint)
    }
    pub fn accent(&self) -> Color32 {
        c(self.accent)
    }
    pub fn accent_text(&self) -> Color32 {
        c(self.accent_text)
    }
    pub fn danger(&self) -> Color32 {
        c(self.danger)
    }
    pub fn warning(&self) -> Color32 {
        c(self.warning)
    }
    pub fn success(&self) -> Color32 {
        c(self.success)
    }

    /// The accent with an alpha, for selection backgrounds.
    pub fn accent_soft(&self, alpha: u8) -> Color32 {
        let [r, g, b] = self.accent;
        Color32::from_rgba_unmultiplied(r, g, b, alpha)
    }

    fn visuals(&self) -> Visuals {
        let dark = self.mode == ThemeMode::Dark;
        let mut v = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let r = CornerRadius::same(self.rounding);
        let hairline = Stroke::new(1.0, c(self.border));

        v.panel_fill = c(self.panel);
        v.window_fill = c(self.surface);
        v.extreme_bg_color = c(self.bg);
        v.faint_bg_color = c(self.surface);
        v.code_bg_color = c(self.surface);
        v.window_stroke = hairline;
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.hyperlink_color = c(self.accent);
        v.warn_fg_color = c(self.warning);
        v.error_fg_color = c(self.danger);
        v.selection.bg_fill = self.accent_soft(if dark { 90 } else { 70 });
        v.selection.stroke = Stroke::new(1.0, c(self.accent));
        v.text_cursor.stroke = Stroke::new(2.0, c(self.accent));
        let shadow_alpha = if dark { 110 } else { 38 };
        v.window_shadow = Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(shadow_alpha),
        };
        v.popup_shadow = v.window_shadow;

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = c(self.panel);
        w.noninteractive.weak_bg_fill = c(self.panel);
        w.noninteractive.bg_stroke = hairline;
        w.noninteractive.fg_stroke = Stroke::new(1.0, c(self.text));
        w.noninteractive.corner_radius = r;

        w.inactive.bg_fill = c(self.surface);
        w.inactive.weak_bg_fill = c(self.surface);
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, c(self.text));
        w.inactive.corner_radius = r;

        w.hovered.bg_fill = c(self.surface_hover);
        w.hovered.weak_bg_fill = c(self.surface_hover);
        w.hovered.bg_stroke = Stroke::new(1.0, c(self.border));
        w.hovered.fg_stroke = Stroke::new(1.0, c(self.text));
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = c(self.surface_active);
        w.active.weak_bg_fill = c(self.surface_active);
        w.active.bg_stroke = Stroke::new(1.0, c(self.accent));
        w.active.fg_stroke = Stroke::new(1.0, c(self.text));
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open.bg_fill = c(self.surface_hover);
        w.open.weak_bg_fill = c(self.surface_hover);
        w.open.bg_stroke = hairline;
        w.open.fg_stroke = Stroke::new(1.0, c(self.text));
        w.open.corner_radius = r;
        v
    }

    fn style(&self, style: &mut Style) {
        style.visuals = self.visuals();
        let sp = &mut style.spacing;
        sp.item_spacing = Vec2::new(8.0, 6.0);
        sp.button_padding = Vec2::new(10.0, 4.0);
        sp.interact_size = Vec2::new(36.0, 26.0);
        sp.window_margin = Margin::same(12);
        sp.menu_margin = Margin::same(6);
        sp.indent = 16.0;
        sp.icon_width = 16.0;
        // Solid (invisible until hovered), not floating: floating scroll bars
        // keep animating forever in some layouts, which would defeat the
        // zero-frame idle.
        sp.scroll = egui::style::ScrollStyle::solid();
        style.text_styles = [
            (TextStyle::Heading, bold(17.0)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.5)),
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.interaction.tooltip_delay = 0.4;
        // No UI animations: every animation is a burst of frames, and the
        // editor draws nothing at all when idle.
        style.animation_time = 0.0;
    }

    /// Applies the theme (visuals and style) to a context.
    pub fn apply(&self, ctx: &Context) {
        ctx.all_styles_mut(|style| self.style(style));
        ctx.set_theme(match self.mode {
            ThemeMode::Dark => egui::Theme::Dark,
            ThemeMode::Light => egui::Theme::Light,
        });
    }

    /// Dock look derived from the current egui style, flattened to match.
    pub fn dock_style(&self, style: &Style) -> egui_dock::Style {
        let mut dock = egui_dock::Style::from_egui(style);
        dock.tab_bar.bg_fill = c(self.bg);
        dock.tab_bar.hline_color = c(self.border);
        dock.tab_bar.height = 30.0;
        dock.tab.tab_body.bg_fill = c(self.panel);
        dock.tab.tab_body.stroke = Stroke::NONE;
        dock.tab.tab_body.inner_margin = Margin::same(10);
        dock.tab.active.bg_fill = c(self.panel);
        dock.tab.active.text_color = c(self.text);
        dock.tab.focused.bg_fill = c(self.panel);
        dock.tab.focused.text_color = c(self.text);
        dock.tab.inactive.bg_fill = c(self.bg);
        dock.tab.inactive.text_color = c(self.text_weak);
        dock.tab.hovered.bg_fill = c(self.surface);
        dock.tab.hovered.text_color = c(self.text);
        dock.separator.width = 1.0;
        dock.separator.color_idle = c(self.border);
        dock.separator.color_hovered = c(self.accent);
        dock.separator.color_dragged = c(self.accent);
        dock
    }
}

/// Installs Inter (regular and bold, UI copies without private-use glyphs)
/// and the Phosphor icon font. egui is
/// built without its default fonts, so this is the complete font set.
pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::empty();
    let inter = |bytes: &'static [u8]| Arc::new(FontData::from_static(bytes));
    fonts.font_data.insert(
        "Inter".to_owned(),
        inter(include_bytes!(
            "../../../assets/fonts/ui/InterUI-Regular.ttf"
        )),
    );
    fonts.font_data.insert(
        "Inter-Bold".to_owned(),
        inter(include_bytes!("../../../assets/fonts/ui/InterUI-Bold.ttf")),
    );
    // Inter is not monospaced; there is no OFL monospace font in the repo yet.
    fonts
        .families
        .insert(FontFamily::Proportional, vec!["Inter".to_owned()]);
    fonts
        .families
        .insert(FontFamily::Monospace, vec!["Inter".to_owned()]);
    fonts
        .families
        .insert(bold_family(), vec!["Inter-Bold".to_owned()]);
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Icons are also available in the monospace and bold families. (The
    // proportional family already falls back to Phosphor, which is why the
    // UI fonts have their private-use glyphs removed, see tools/make_ui_fonts.py.)
    for family in [FontFamily::Monospace, bold_family()] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("phosphor".to_owned());
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_round_trip_through_ron() {
        for theme in [Theme::dark(), Theme::light()] {
            let text = ron::to_string(&theme).unwrap();
            assert_eq!(Theme::from_ron(&text).unwrap(), theme);
        }
    }

    #[test]
    fn apply_sets_visuals_for_both_modes() {
        let ctx = Context::default();
        Theme::dark().apply(&ctx);
        assert!(ctx.global_style().visuals.dark_mode);
        assert_eq!(ctx.global_style().visuals.panel_fill, Theme::dark().panel());
        Theme::light().apply(&ctx);
        assert!(!ctx.global_style().visuals.dark_mode);
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            Theme::light().panel()
        );
    }

    #[test]
    fn fonts_install_without_panicking_and_text_lays_out() {
        let ctx = Context::default();
        install_fonts(&ctx);
        Theme::dark().apply(&ctx);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label(format!("{} Hello", egui_phosphor::regular::FOLDER));
                ui.heading("Titre");
            });
        });
        output.drop_without_applying_deltas();
    }
}
