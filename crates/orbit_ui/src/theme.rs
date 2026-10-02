//! The data-driven look of the in-game UI.

use serde::{Deserialize, Serialize};

/// Converts an sRGBA byte color to the linear RGBA expected by sprite tints.
pub fn tint(color: [u8; 4]) -> [f32; 4] {
    [
        orbit_text::srgb_to_linear(color[0]),
        orbit_text::srgb_to_linear(color[1]),
        orbit_text::srgb_to_linear(color[2]),
        color[3] as f32 / 255.0,
    ]
}

/// Look of the whole UI. Every field has a default, so a theme file only
/// needs to list what it overrides (`(textbox: (text_size: 36.0))`). All
/// lengths are in virtual pixels (a 1920x1080 canvas), colors are sRGBA bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Theme {
    pub textbox: TextBoxTheme,
    pub choice: ChoiceTheme,
    pub screens: ScreenTheme,
    /// Look of the widget toolkit and of the menu screens.
    pub widgets: WidgetTheme,
    /// Optional UI sounds, played by the host.
    pub sounds: UiSounds,
}

impl Theme {
    pub fn from_ron(source: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(source)
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new())
    }
}

/// The dialogue panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextBoxTheme {
    /// `[x, y, width, height]` of the panel (top-left origin).
    pub rect: [f32; 4],
    pub background: [u8; 4],
    /// Horizontal and vertical inner margin.
    pub padding: [f32; 2],
    pub name_size: f32,
    /// Distance between the top padding and the text (room for the name).
    pub text_offset: f32,
    pub text_size: f32,
    pub text_color: [u8; 4],
    /// Typewriter speed in characters per second.
    pub chars_per_second: f32,
    pub indicator_color: [u8; 4],
    /// Side of the diamond-shaped "continue" marker.
    pub indicator_size: f32,
}

impl Default for TextBoxTheme {
    fn default() -> Self {
        Self {
            rect: [90.0, 790.0, 1740.0, 260.0],
            background: [4, 6, 18, 190],
            padding: [50.0, 16.0],
            name_size: 44.0,
            text_offset: 64.0,
            text_size: 40.0,
            text_color: [255, 255, 255, 255],
            chars_per_second: 40.0,
            indicator_color: [255, 255, 255, 255],
            indicator_size: 22.0,
        }
    }
}

/// The choice screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChoiceTheme {
    /// Full-screen dimming drawn behind the buttons.
    pub overlay: [u8; 4],
    /// `[width, height]` of one button.
    pub button_size: [f32; 2],
    pub spacing: f32,
    pub text_size: f32,
    pub button_color: [u8; 4],
    /// Background of the hovered / keyboard-selected button.
    pub selected_color: [u8; 4],
    pub text_color: [u8; 4],
    pub selected_text_color: [u8; 4],
    /// Prefix the labels with their digit (`1.`, `2.`...), matching the
    /// number-key shortcuts.
    pub show_numbers: bool,
}

impl Default for ChoiceTheme {
    fn default() -> Self {
        Self {
            overlay: [0, 0, 0, 110],
            button_size: [1200.0, 90.0],
            spacing: 26.0,
            text_size: 38.0,
            button_color: [14, 18, 40, 215],
            selected_color: [70, 110, 210, 235],
            text_color: [225, 230, 245, 255],
            selected_text_color: [255, 255, 255, 255],
            show_numbers: true,
        }
    }
}

/// Loading, start, end and error screens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenTheme {
    pub background: [u8; 4],
    pub text_color: [u8; 4],
    pub error_color: [u8; 4],
    pub title_size: f32,
    pub body_size: f32,
    /// `[width, height]` of the loading bar.
    pub bar_size: [f32; 2],
    pub bar_background: [u8; 4],
    pub bar_fill: [u8; 4],
    pub texts: UiTexts,
}

impl Default for ScreenTheme {
    fn default() -> Self {
        Self {
            background: [5, 7, 14, 255],
            text_color: [235, 238, 248, 255],
            error_color: [255, 130, 130, 255],
            title_size: 64.0,
            body_size: 30.0,
            bar_size: [700.0, 14.0],
            bar_background: [30, 36, 56, 255],
            bar_fill: [110, 160, 255, 255],
            texts: UiTexts::default(),
        }
    }
}

/// Engine-provided strings (they will go through the localization table
/// later).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiTexts {
    pub loading: String,
    pub click_to_start: String,
    pub the_end: String,
    pub error_title: String,
    pub error_hint: String,
}

impl Default for UiTexts {
    fn default() -> Self {
        Self {
            loading: "Chargement\u{2026}".into(),
            click_to_start: "Cliquer pour d\u{e9}marrer".into(),
            the_end: "Fin \u{2014} cliquer pour recommencer".into(),
            error_title: "Impossible de lancer l'histoire".into(),
            error_hint: "Corrigez le fichier : il sera recharg\u{e9} automatiquement (natif)."
                .into(),
        }
    }
}

/// Paths (or asset ids, the host decides how to resolve them) of the optional
/// UI sounds. The UI never plays audio itself: it emits
/// [`crate::UiEvent::Sound`] and the host looks the sound up here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSounds {
    pub hover: Option<String>,
    pub confirm: Option<String>,
    pub cancel: Option<String>,
    pub slide: Option<String>,
}

impl UiSounds {
    /// The configured sound of `sound`, if any.
    pub fn get(&self, sound: crate::UiSound) -> Option<&str> {
        let slot = match sound {
            crate::UiSound::Hover => &self.hover,
            crate::UiSound::Confirm => &self.confirm,
            crate::UiSound::Cancel => &self.cancel,
            crate::UiSound::Slide => &self.slide,
        };
        slot.as_deref()
    }
}

/// Panels (menu backgrounds, cards).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelTheme {
    pub background: [u8; 4],
    pub border_color: [u8; 4],
    /// Border thickness (0 = none).
    pub border_width: f32,
    /// Full-screen dimming behind pause menus and modal dialogs.
    pub dim: [u8; 4],
}

impl Default for PanelTheme {
    fn default() -> Self {
        Self {
            background: [10, 14, 34, 240],
            border_color: [90, 110, 190, 255],
            border_width: 2.0,
            dim: [0, 0, 0, 150],
        }
    }
}

/// Buttons (also tabs and quick-menu entries).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ButtonTheme {
    pub height: f32,
    /// Height of the small variant (quick menu, delete buttons).
    pub small_height: f32,
    pub padding_x: f32,
    pub normal: [u8; 4],
    pub hover: [u8; 4],
    pub pressed: [u8; 4],
    pub disabled: [u8; 4],
    /// Background of an active tab / toggled quick-menu entry.
    pub selected: [u8; 4],
    pub text_color: [u8; 4],
    pub hover_text_color: [u8; 4],
    pub disabled_text_color: [u8; 4],
    /// Ring drawn around the keyboard-focused widget.
    pub focus_color: [u8; 4],
    pub focus_width: f32,
}

impl Default for ButtonTheme {
    fn default() -> Self {
        Self {
            height: 70.0,
            small_height: 44.0,
            padding_x: 30.0,
            normal: [14, 18, 40, 220],
            hover: [70, 110, 210, 235],
            pressed: [50, 85, 170, 245],
            disabled: [20, 22, 30, 160],
            selected: [40, 70, 150, 235],
            text_color: [225, 230, 245, 255],
            hover_text_color: [255, 255, 255, 255],
            disabled_text_color: [110, 115, 130, 255],
            focus_color: [255, 214, 100, 255],
            focus_width: 3.0,
        }
    }
}

/// Sliders, toggles and selectors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlTheme {
    pub height: f32,
    pub track_height: f32,
    pub track: [u8; 4],
    pub fill: [u8; 4],
    /// `[width, height]` of the slider knob.
    pub knob_size: [f32; 2],
    pub knob: [u8; 4],
    /// Side of the toggle box.
    pub toggle_size: f32,
    pub toggle_off: [u8; 4],
    pub toggle_on: [u8; 4],
    /// Keyboard step of a slider (fraction of its range).
    pub slider_step: f32,
}

impl Default for ControlTheme {
    fn default() -> Self {
        Self {
            height: 56.0,
            track_height: 10.0,
            track: [40, 46, 72, 255],
            fill: [110, 160, 255, 255],
            knob_size: [18.0, 38.0],
            knob: [240, 244, 255, 255],
            toggle_size: 38.0,
            toggle_off: [24, 28, 52, 255],
            toggle_on: [110, 160, 255, 255],
            slider_step: 0.05,
        }
    }
}

/// Scroll lists and toast notifications.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListTheme {
    pub bar_width: f32,
    pub bar_track: [u8; 4],
    pub bar_thumb: [u8; 4],
    /// Items scrolled by one mouse-wheel notch.
    pub wheel_items: usize,
    /// Seconds a toast stays visible.
    pub toast_seconds: f32,
    pub toast_background: [u8; 4],
}

impl Default for ListTheme {
    fn default() -> Self {
        Self {
            bar_width: 10.0,
            bar_track: [30, 34, 56, 200],
            bar_thumb: [120, 150, 230, 255],
            wheel_items: 1,
            toast_seconds: 2.5,
            toast_background: [12, 16, 40, 235],
        }
    }
}

/// Everything the widget toolkit and the menu screens need.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetTheme {
    pub title_size: f32,
    pub heading_size: f32,
    pub text_size: f32,
    pub small_text_size: f32,
    pub text_color: [u8; 4],
    pub dim_text_color: [u8; 4],
    pub accent_color: [u8; 4],
    /// Gap between siblings in the stock screens.
    pub spacing: f32,
    pub panel: PanelTheme,
    pub button: ButtonTheme,
    pub controls: ControlTheme,
    pub list: ListTheme,
}

impl Default for WidgetTheme {
    fn default() -> Self {
        Self {
            title_size: 72.0,
            heading_size: 48.0,
            text_size: 30.0,
            small_text_size: 22.0,
            text_color: [235, 238, 248, 255],
            dim_text_color: [150, 158, 185, 255],
            accent_color: [255, 214, 100, 255],
            spacing: 18.0,
            panel: PanelTheme::default(),
            button: ButtonTheme::default(),
            controls: ControlTheme::default(),
            list: ListTheme::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_widget_theme_keeps_the_other_defaults() {
        let theme = Theme::from_ron(
            "(widgets: (button: (height: 90.0), accent_color: (1, 2, 3, 4)), \
             sounds: (confirm: Some(\"ui/click.ogg\")))",
        )
        .unwrap();
        assert_eq!(theme.widgets.button.height, 90.0);
        assert_eq!(
            theme.widgets.button.hover,
            ButtonTheme::default().hover,
            "sibling fields keep their defaults"
        );
        assert_eq!(theme.widgets.accent_color, [1, 2, 3, 4]);
        assert_eq!(theme.widgets.panel, PanelTheme::default());
        assert_eq!(
            theme.sounds.get(crate::UiSound::Confirm),
            Some("ui/click.ogg")
        );
        assert_eq!(theme.sounds.get(crate::UiSound::Hover), None);
        assert_eq!(theme.textbox, TextBoxTheme::default());
    }

    #[test]
    fn default_theme_round_trips_through_ron() {
        let theme = Theme::default();
        let text = theme.to_ron().unwrap();
        assert_eq!(Theme::from_ron(&text).unwrap(), theme);
    }

    #[test]
    fn partial_theme_keeps_defaults() {
        let theme =
            Theme::from_ron("(textbox: (text_size: 30.0), choice: (show_numbers: false))").unwrap();
        assert_eq!(theme.textbox.text_size, 30.0);
        assert_eq!(theme.textbox.name_size, TextBoxTheme::default().name_size);
        assert!(!theme.choice.show_numbers);
        assert_eq!(theme.screens, ScreenTheme::default());
        assert_eq!(Theme::from_ron("()").unwrap(), Theme::default());
    }

    #[test]
    fn invalid_theme_is_an_error_not_a_panic() {
        assert!(Theme::from_ron("(textbox: 3)").is_err());
    }

    #[test]
    fn tint_converts_to_linear() {
        let t = tint([255, 0, 128, 51]);
        assert_eq!(t[0], 1.0);
        assert_eq!(t[1], 0.0);
        assert!(t[2] < 0.25 && t[2] > 0.18);
        assert!((t[3] - 0.2).abs() < 1e-6);
    }
}
