//! The serializable theme lives in `orbit_ui_data`; this module keeps the
//! only GPU-side helper and re-exports the data types under their old path.

pub use orbit_ui_data::{
    Border, ButtonTheme, ChoiceTheme, ControlTheme, DialogueLayout, Fill, FontRef, ImageFit, ListTheme, NamePlate,
    PanelTheme, QuickMenuTheme, ScreenTheme, Shadow, StateColors, StateFills, Style, StyleSounds, TextAlignment,
    TextBoxTheme, Theme, UiSounds, WidgetState, WidgetTheme, resolve_style,
};

/// Converts an sRGBA byte color to the linear RGBA expected by sprite tints.
pub fn tint(color: [u8; 4]) -> [f32; 4] {
    [
        orbit_text::srgb_to_linear(color[0]),
        orbit_text::srgb_to_linear(color[1]),
        orbit_text::srgb_to_linear(color[2]),
        color[3] as f32 / 255.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_converts_to_linear() {
        let t = tint([255, 0, 128, 51]);
        assert_eq!(t[0], 1.0);
        assert_eq!(t[1], 0.0);
        assert!(t[2] < 0.25 && t[2] > 0.18);
        assert!((t[3] - 0.2).abs() < 1e-6);
    }
}
