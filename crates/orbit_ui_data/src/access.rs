//! Accessibility derivations of a [`Theme`] (M7-07, ADR 0017): the text
//! scale, the high-contrast override and the WCAG AA contrast check. The
//! project theme is never modified: the player derives the theme it draws
//! with from the preferences.

use crate::theme::{Fill, StateColors, Style, Theme};

/// Allowed text scale (1.0 = the theme sizes).
pub const TEXT_SCALE_RANGE: [f32; 2] = [1.0, 1.6];

/// Minimum WCAG AA contrast of normal text.
pub const AA_TEXT: f32 = 4.5;
/// Minimum WCAG AA contrast of a user-interface component (non-text).
pub const AA_COMPONENT: f32 = 3.0;

/// Clamps a text scale into [`TEXT_SCALE_RANGE`] (`1.0` when not finite).
pub fn clamp_text_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(TEXT_SCALE_RANGE[0], TEXT_SCALE_RANGE[1])
    } else {
        1.0
    }
}

fn channel(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of an sRGB color (alpha ignored).
pub fn luminance(c: [u8; 4]) -> f32 {
    0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2])
}

/// WCAG contrast ratio of two opaque colors, `1.0..=21.0`.
pub fn contrast_ratio(a: [u8; 4], b: [u8; 4]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `fg` drawn over the opaque `bg` (the result is opaque).
fn over(fg: [u8; 4], bg: [u8; 4]) -> [u8; 4] {
    let a = f32::from(fg[3]) / 255.0;
    let mix = |f: u8, b: u8| (f32::from(f) * a + f32::from(b) * (1.0 - a)).round() as u8;
    [mix(fg[0], bg[0]), mix(fg[1], bg[1]), mix(fg[2], bg[2]), 255]
}

/// Contrast of `fg` over `bg`. A mostly opaque `bg` (alpha 200+) is taken
/// as is; a clearly translucent one lets the scene through, which is
/// unknown, so both a black and a white backdrop are tried.
fn worst_ratio(fg: [u8; 4], bg: [u8; 4]) -> f32 {
    let backdrops: &[[u8; 4]] = if bg[3] >= 200 { &[BLACK] } else { &[BLACK, WHITE] };
    backdrops
        .iter()
        .copied()
        .map(|backdrop| {
            let bg = over(bg, backdrop);
            contrast_ratio(over(fg, bg), bg)
        })
        .fold(f32::INFINITY, f32::min)
}

/// A pair of colors of a theme below the WCAG AA contrast.
#[derive(Clone, Debug, PartialEq)]
pub struct ContrastIssue {
    /// What the pair is (`textbox text on background`).
    pub what: String,
    /// The style (and state) for a pair taken from a named style.
    pub subject: Option<String>,
    pub foreground: [u8; 4],
    pub background: [u8; 4],
    pub ratio: f32,
    pub required: f32,
}

impl ContrastIssue {
    /// A stable key of what the pair is (`dialogue_text_on_the_text_box`),
    /// for translating the message.
    pub fn id(&self) -> String {
        self.what
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
            .collect()
    }

    /// A one-line English message for the MCP tools and logs.
    pub fn message(&self) -> String {
        format!(
            "low contrast, {}{}: {:.1}:1 (WCAG AA needs {:.1}:1)",
            self.what,
            self.subject.as_ref().map(|s| format!(" `{s}`")).unwrap_or_default(),
            self.ratio,
            self.required
        )
    }
}

const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const YELLOW: [u8; 4] = [255, 221, 0, 255];

impl Theme {
    /// The color pairs of the theme that miss WCAG 2.1 AA (4.5:1 for text,
    /// 3:1 for the parts of the controls). Translucent backgrounds are
    /// judged over the worst of a black and a white scene.
    pub fn contrast_issues(&self) -> Vec<ContrastIssue> {
        let mut out = Vec::new();
        let mut check = |what: &str, subject: Option<String>, fg: [u8; 4], bg: [u8; 4], required: f32| {
            let ratio = worst_ratio(fg, bg);
            if ratio < required {
                out.push(ContrastIssue {
                    what: what.to_owned(),
                    subject,
                    foreground: fg,
                    background: bg,
                    ratio,
                    required,
                });
            }
        };
        let w = &self.widgets;
        let tb = &self.textbox;
        let ch = &self.choice;
        let sc = &self.screens;
        let mut text = |what: &str, fg: [u8; 4], bg: [u8; 4]| check(what, None, fg, bg, AA_TEXT);
        text("dialogue text on the text box", tb.text_color, tb.background);
        text("choice text on a choice button", ch.text_color, ch.button_color);
        text("selected choice text on the selected button", ch.selected_text_color, ch.selected_color);
        text("screen text on the screen background", sc.text_color, sc.background);
        text("error text on the screen background", sc.error_color, sc.background);
        text("text on a panel", w.text_color, w.panel.background);
        text("dimmed text on a panel", w.dim_text_color, w.panel.background);
        text("accent text on a panel", w.accent_color, w.panel.background);
        text("button text on a button", w.button.text_color, w.button.normal);
        text("button text on a hovered button", w.button.hover_text_color, w.button.hover);
        let c = &w.controls;
        let mut part = |what: &str, fg: [u8; 4], bg: [u8; 4]| check(what, None, fg, bg, AA_COMPONENT);
        part("slider fill on its track", c.fill, c.track);
        part("slider knob on its track", c.knob, c.track);
        part("toggle (on) against its off state", c.toggle_on, c.toggle_off);
        for (name, style) in &self.styles {
            // A style that sets both a text color and a background.
            for (state, fg, bg) in [
                ("normal", style.text_colors.normal, style.background(true, false, false)),
                ("hover", style.text_colors.hover, style.background(true, false, true)),
            ] {
                if let (Some(fg), Some(bg)) = (fg, bg) {
                    check("style text on its background", Some(format!("{name} ({state})")), fg, bg, AA_TEXT);
                }
            }
        }
        out
    }

    /// The theme with every text size multiplied by `scale` (clamped to
    /// [`TEXT_SCALE_RANGE`]) and the boxes that hold text grown to fit: the
    /// dialogue box rises from its bottom edge, the choice buttons get
    /// taller and wider. Control heights are left alone: they already leave
    /// room for text up to the largest scale, so stock screens keep fitting.
    pub fn scaled(&self, scale: f32) -> Theme {
        let s = clamp_text_scale(scale);
        let mut t = self.clone();
        if s == 1.0 {
            return t;
        }
        let tb = &mut t.textbox;
        tb.name_size *= s;
        tb.text_size *= s;
        tb.text_offset *= s;
        tb.indicator_size *= s;
        let [x, y, width, height] = tb.rect;
        let bottom = y + height;
        let new_height = (height * s).min(bottom - 40.0).max(height);
        tb.rect = [x, bottom - new_height, width, new_height];
        let ch = &mut t.choice;
        ch.text_size *= s;
        ch.button_size[1] *= s;
        ch.button_size[0] = (ch.button_size[0] * s).min(1800.0).max(ch.button_size[0]);
        t.screens.title_size *= s;
        t.screens.body_size *= s;
        let w = &mut t.widgets;
        w.title_size *= s;
        w.heading_size *= s;
        w.text_size *= s;
        w.small_text_size *= s;
        for style in t.styles.values_mut() {
            if let Some(size) = &mut style.text_size {
                *size *= s;
            }
        }
        t
    }

    /// The theme with every color replaced by a high-contrast one: white on
    /// black, yellow accents and focus rings, solid backgrounds, thick
    /// borders. Sizes, layout, fonts, sounds and style names are kept; a
    /// style that sets colors has them replaced (a nine-slice fill is
    /// dropped so that the solid color shows).
    pub fn high_contrast(&self) -> Theme {
        let mut t = self.clone();
        let tb = &mut t.textbox;
        tb.background = [0, 0, 0, 255];
        tb.text_color = WHITE;
        tb.indicator_color = YELLOW;
        let ch = &mut t.choice;
        ch.overlay = [0, 0, 0, 200];
        ch.button_color = BLACK;
        ch.selected_color = YELLOW;
        ch.text_color = WHITE;
        ch.selected_text_color = BLACK;
        let sc = &mut t.screens;
        sc.background = BLACK;
        sc.text_color = WHITE;
        sc.error_color = [255, 140, 140, 255];
        sc.bar_background = [60, 60, 60, 255];
        sc.bar_fill = YELLOW;
        let w = &mut t.widgets;
        w.text_color = WHITE;
        w.dim_text_color = [210, 210, 210, 255];
        w.accent_color = YELLOW;
        w.panel.background = BLACK;
        w.panel.border_color = WHITE;
        w.panel.border_width = w.panel.border_width.max(4.0);
        w.panel.dim = [0, 0, 0, 215];
        let b = &mut w.button;
        b.normal = BLACK;
        b.hover = YELLOW;
        b.pressed = [200, 170, 0, 255];
        b.disabled = [30, 30, 30, 255];
        b.selected = [70, 70, 70, 255];
        b.text_color = WHITE;
        b.hover_text_color = BLACK;
        b.disabled_text_color = [170, 170, 170, 255];
        b.focus_color = [0, 255, 255, 255];
        b.focus_width = b.focus_width.max(5.0);
        let c = &mut w.controls;
        c.track = [70, 70, 70, 255];
        c.fill = YELLOW;
        c.knob = WHITE;
        c.toggle_off = BLACK;
        c.toggle_on = YELLOW;
        let l = &mut w.list;
        l.bar_track = [40, 40, 40, 255];
        l.bar_thumb = WHITE;
        l.toast_background = BLACK;
        for style in t.styles.values_mut() {
            contrast_style(style);
        }
        t
    }

    /// The theme the player draws with: [`Theme::scaled`] then, when asked,
    /// [`Theme::high_contrast`].
    pub fn accessible(&self, text_scale: f32, high_contrast: bool) -> Theme {
        let t = self.scaled(text_scale);
        if high_contrast { t.high_contrast() } else { t }
    }
}

fn contrast_style(style: &mut Style) {
    let sets_text = style.text_colors != StateColors::default();
    let sets_back =
        style.colors != StateColors::default() || style.fill.is_some() || !style.fills.is_empty();
    if sets_text {
        style.text_colors = StateColors {
            normal: Some(WHITE),
            hover: Some(BLACK),
            pressed: Some(BLACK),
            disabled: Some([170, 170, 170, 255]),
            focused: Some(BLACK),
        };
    }
    if sets_back {
        style.fill = style.fill.take().and(Some(Fill::Color(BLACK)));
        // Images and gradients give way to the plain high-contrast colors.
        style.fills = Default::default();
        style.shadow = None;
        style.colors = StateColors {
            normal: Some(BLACK),
            hover: Some(YELLOW),
            pressed: Some([200, 170, 0, 255]),
            disabled: Some([30, 30, 30, 255]),
            focused: Some(YELLOW),
        };
    }
    if let Some(border) = &mut style.border {
        border.color = WHITE;
        border.width = border.width.max(3.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_ratio_matches_wcag_reference_values() {
        assert!((contrast_ratio(BLACK, WHITE) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(WHITE, WHITE) - 1.0).abs() < 1e-6);
        // #767676 on white is the classic 4.54:1.
        assert!((contrast_ratio([0x76, 0x76, 0x76, 255], WHITE) - 4.54).abs() < 0.02);
    }

    #[test]
    fn the_default_theme_passes_aa() {
        assert_eq!(Theme::default().contrast_issues(), Vec::new());
    }

    #[test]
    fn the_high_contrast_theme_passes_aa_whatever_the_project_theme() {
        let mut theme = Theme::default();
        theme.textbox.text_color = [120, 120, 120, 255];
        theme.textbox.background = [100, 100, 100, 120];
        theme.styles.insert(
            "pale".into(),
            Style {
                text_colors: StateColors { normal: Some([200, 200, 200, 255]), ..Default::default() },
                colors: StateColors { normal: Some([220, 220, 220, 255]), ..Default::default() },
                ..Default::default()
            },
        );
        assert!(!theme.contrast_issues().is_empty());
        let hc = theme.high_contrast();
        assert_eq!(hc.contrast_issues(), Vec::new());
        assert_eq!(hc.styles.len(), 1);
    }

    #[test]
    fn a_low_contrast_pair_is_reported_with_its_ratio() {
        let mut theme = Theme::default();
        theme.widgets.dim_text_color = [60, 70, 100, 255];
        let issues = theme.contrast_issues();
        let issue = issues.iter().find(|i| i.what.contains("dimmed")).expect("dimmed text flagged");
        assert!(issue.ratio < AA_TEXT && issue.required == AA_TEXT);
        assert!(issue.message().contains("WCAG AA"));
    }

    #[test]
    fn scale_one_changes_nothing_and_larger_scales_grow_text_and_boxes() {
        let base = Theme::default();
        assert_eq!(base.scaled(1.0), base);
        assert_eq!(base.accessible(1.0, false), base);
        assert_eq!(base.scaled(f32::NAN), base);
        let big = base.scaled(1.5);
        assert_eq!(big.textbox.text_size, 60.0);
        assert_eq!(big.widgets.text_size, 45.0);
        assert!(big.textbox.rect[3] > base.textbox.rect[3]);
        let [_, y0, _, h0] = base.textbox.rect;
        let [_, y1, _, h1] = big.textbox.rect;
        assert!((y0 + h0 - (y1 + h1)).abs() < 1e-3, "the box keeps its bottom edge");
        assert!(big.choice.button_size[1] > base.choice.button_size[1]);
        assert_eq!(base.scaled(9.0), base.scaled(TEXT_SCALE_RANGE[1]));
    }

    #[test]
    fn styles_keep_their_names_and_scale_their_text() {
        let mut theme = Theme::default();
        theme.styles.insert("big".into(), Style { text_size: Some(20.0), ..Default::default() });
        assert_eq!(theme.scaled(1.5).styles["big"].text_size, Some(30.0));
        assert_eq!(theme.high_contrast().styles["big"].text_size, Some(20.0));
    }
}
