//! The choice screen: stacked buttons.

use std::collections::HashMap;

use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId};
use orbit_text::{DrawParams, TextBlock, TextEngine, TextRenderer, TextStyle};

use crate::paint::Painter;
use crate::theme::{ChoiceTheme, Fill, Style, Theme, WidgetState, resolve_style, tint};

/// Virtual canvas height buttons are laid out in.
const CANVAS: [f32; 2] = [1920.0, 1080.0];

/// Logical input of the menu (already decoded from mouse / keyboard).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuInput {
    /// Move the selection up (wraps around).
    Up,
    Down,
    /// Confirm the selected button (Enter / Space).
    Confirm,
    /// A digit key, 1-based.
    Number(usize),
    /// Pointer moved to this virtual position: hover selects.
    Hover([f32; 2]),
    /// Pointer clicked at this virtual position.
    Click([f32; 2]),
}

/// `[x, y, w, h]` of each of `count` buttons, centered on the canvas (or
/// placed by the theme's anchor). When the stack would not fit the screen,
/// heights and spacing shrink.
pub fn button_rects(theme: &ChoiceTheme, count: usize) -> Vec<[f32; 4]> {
    theme.button_rects(count)
}

/// Index of the rectangle containing `point`.
pub fn hit_test(rects: &[[f32; 4]], point: [f32; 2]) -> Option<usize> {
    rects.iter().position(|&[x, y, w, h]| {
        point[0] >= x && point[0] < x + w && point[1] >= y && point[1] < y + h
    })
}

/// A set of option buttons with a selection.
pub struct ChoiceMenu {
    theme: ChoiceTheme,
    /// The resolved `button_style` (`ui_version` 8), if any.
    style: Option<Style>,
    /// `[normal, selected]` label per button (colors are baked in a block).
    labels: Vec<[TextBlock; 2]>,
    rects: Vec<[f32; 4]>,
    selected: usize,
}

impl ChoiceMenu {
    pub fn new(theme: ChoiceTheme, options: &[String]) -> Self {
        Self::build(theme, None, None, options)
    }

    /// A menu drawn with the theme's choice section and its button style.
    pub fn styled(theme: &Theme, options: &[String]) -> Self {
        let style = theme
            .choice
            .button_style
            .as_deref()
            .and_then(|n| resolve_style(&theme.styles, n));
        let family = style
            .as_ref()
            .and_then(|s| s.font.as_deref())
            .map(|f| theme.font_family(f).to_owned());
        Self::build(theme.choice.clone(), style, family, options)
    }

    fn build(theme: ChoiceTheme, style: Option<Style>, family: Option<String>, options: &[String]) -> Self {
        let rects = button_rects(&theme, options.len());
        let s = style.as_ref();
        let size = s.and_then(|s| s.text_size).unwrap_or(theme.text_size);
        let align = s
            .and_then(|s| s.text_align)
            .map_or(orbit_text::TextAlign::Center, crate::widgets::text_align);
        let inset = s.and_then(|s| s.padding).map_or(30.0, |p| p[0]);
        let colors = [
            s.and_then(|s| s.text_color_for(WidgetState::Normal)).unwrap_or(theme.text_color),
            s.and_then(|s| s.text_color_for(WidgetState::Hover))
                .unwrap_or(theme.selected_text_color),
        ];
        let show = s.is_none_or(Style::text_visible);
        let labels = options
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let markup = if !show {
                    String::new()
                } else if theme.show_numbers {
                    format!("{}. {text}", i + 1)
                } else {
                    text.clone()
                };
                let markup = crate::widgets::styled_markup(s, markup);
                let make = |color: [u8; 4]| {
                    let mut block = TextBlock::new(
                        &markup,
                        TextStyle {
                            font_size: size,
                            color,
                            align,
                            family: family.clone(),
                            ..Default::default()
                        },
                        Some(theme.button_size[0] - 2.0 * inset),
                        f32::INFINITY,
                    );
                    block.skip_to_end();
                    block
                };
                [make(colors[0]), make(colors[1])]
            })
            .collect();
        Self {
            theme,
            style,
            labels,
            rects,
            selected: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn rects(&self) -> &[[f32; 4]] {
        &self.rects
    }

    /// Applies an input; returns the index of the confirmed option, if any.
    pub fn handle(&mut self, input: MenuInput) -> Option<usize> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        match input {
            MenuInput::Up => self.selected = (self.selected + n - 1) % n,
            MenuInput::Down => self.selected = (self.selected + 1) % n,
            MenuInput::Confirm => return Some(self.selected),
            MenuInput::Number(d) => return (1..=n).contains(&d).then(|| d - 1),
            MenuInput::Hover(p) => {
                if let Some(i) = hit_test(&self.rects, p) {
                    self.selected = i;
                }
            }
            MenuInput::Click(p) => {
                if let Some(i) = hit_test(&self.rects, p) {
                    self.selected = i;
                    return Some(i);
                }
            }
        }
        None
    }

    /// `images` are the loaded project images the button style may draw.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        engine: &mut TextEngine,
        text_renderer: &mut TextRenderer,
        renderer: &Renderer2d,
        scene: &mut Scene,
        white: TextureId,
        images: &HashMap<String, TextureId>,
    ) {
        let t = &self.theme;
        scene.push(
            Sprite2d::new(white, [0.0, 0.0], CANVAS)
                .anchor([0.0, 0.0])
                .layer(Layer::UI)
                .z(-1.0)
                .tint(tint(t.overlay)),
        );
        let painter = Painter {
            renderer,
            white,
            images,
            alpha: 1.0,
        };
        let size = self.style.as_ref().and_then(|s| s.text_size).unwrap_or(t.text_size);
        let inset = self.style.as_ref().and_then(|s| s.padding).map_or(30.0, |p| p[0]);
        for (i, (&[x, y, w, h], label)) in self.rects.iter().zip(&mut self.labels).enumerate() {
            let selected = i == self.selected;
            let bg = if selected {
                t.selected_color
            } else {
                t.button_color
            };
            let rect = [x, y, w, h];
            match &self.style {
                None => scene.push(
                    Sprite2d::new(white, [x, y], [w, h])
                        .anchor([0.0, 0.0])
                        .layer(Layer::UI)
                        .z(0.0)
                        .tint(tint(bg)),
                ),
                Some(style) => {
                    let state = if selected { WidgetState::Hover } else { WidgetState::Normal };
                    if let Some(shadow) = &style.shadow {
                        painter.shadow(scene, shadow, rect, Layer::UI, 0.0);
                    }
                    let fill = style.fill_for(state);
                    let sliced = match &fill {
                        Some(f) if painter.fill(scene, f, rect, Layer::UI, 0.0) => {
                            matches!(f, Fill::NineSlice { .. })
                        }
                        _ => {
                            let c = fill.as_ref().and_then(Fill::flat_color).unwrap_or(bg);
                            painter.quad(scene, rect, c, Layer::UI, 0.0);
                            false
                        }
                    };
                    if !sliced && let Some(border) = &style.border {
                        painter.border(scene, rect, border.width, border.color, Layer::UI, 0.2);
                    }
                }
            }
            // Vertically centered on a single line (labels are short).
            let line = size * 1.35;
            let params = DrawParams {
                origin: [x + inset, y + (h - line) * 0.5],
                z: 2.0,
                ..Default::default()
            };
            label[usize::from(selected)].draw(engine, text_renderer, renderer, scene, &params);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(n: usize) -> ChoiceMenu {
        let options: Vec<String> = (0..n).map(|i| format!("Option {i}")).collect();
        ChoiceMenu::new(ChoiceTheme::default(), &options)
    }

    #[test]
    fn rects_are_centered_and_stacked() {
        let rects = button_rects(&ChoiceTheme::default(), 2);
        assert_eq!(rects.len(), 2);
        assert!((rects[0][0] + rects[0][2] * 0.5 - 960.0).abs() < 1e-3);
        let top = rects[0][1];
        let bottom = rects[1][1] + rects[1][3];
        assert!(((top + bottom) * 0.5 - 540.0).abs() < 1e-3);
        assert!(rects[1][1] > rects[0][1] + rects[0][3]);
        assert!(button_rects(&ChoiceTheme::default(), 0).is_empty());
    }

    #[test]
    fn many_buttons_still_fit_the_screen() {
        let rects = button_rects(&ChoiceTheme::default(), 12);
        assert!(rects[0][1] >= 59.0);
        let last = rects[11];
        assert!(last[1] + last[3] <= 1080.0 - 59.0);
    }

    #[test]
    fn hit_testing() {
        let rects = button_rects(&ChoiceTheme::default(), 3);
        let [x, y, w, h] = rects[1];
        assert_eq!(hit_test(&rects, [x + w * 0.5, y + h * 0.5]), Some(1));
        assert_eq!(hit_test(&rects, [x - 1.0, y + 1.0]), None);
        assert_eq!(hit_test(&rects, [0.0, 0.0]), None);
        // The gap between two buttons belongs to none.
        assert_eq!(hit_test(&rects, [x + 1.0, y - 1.0]), None);
    }

    #[test]
    fn keyboard_navigation_wraps_and_confirms() {
        let mut m = menu(3);
        assert_eq!(m.selected(), 0);
        assert_eq!(m.handle(MenuInput::Up), None);
        assert_eq!(m.selected(), 2);
        assert_eq!(m.handle(MenuInput::Down), None);
        assert_eq!(m.selected(), 0);
        m.handle(MenuInput::Down);
        assert_eq!(m.handle(MenuInput::Confirm), Some(1));
    }

    #[test]
    fn digits_pick_directly_and_ignore_out_of_range() {
        let mut m = menu(2);
        assert_eq!(m.handle(MenuInput::Number(2)), Some(1));
        assert_eq!(m.handle(MenuInput::Number(1)), Some(0));
        assert_eq!(m.handle(MenuInput::Number(3)), None);
        assert_eq!(m.handle(MenuInput::Number(0)), None);
    }

    #[test]
    fn mouse_hover_selects_and_click_confirms() {
        let mut m = menu(2);
        let [x, y, w, h] = m.rects()[1];
        let inside = [x + w * 0.5, y + h * 0.5];
        assert_eq!(m.handle(MenuInput::Hover(inside)), None);
        assert_eq!(m.selected(), 1);
        // Hover on empty space keeps the selection.
        m.handle(MenuInput::Hover([5.0, 5.0]));
        assert_eq!(m.selected(), 1);
        assert_eq!(m.handle(MenuInput::Click([5.0, 5.0])), None);
        assert_eq!(m.handle(MenuInput::Click(inside)), Some(1));
    }

    #[test]
    fn empty_menu_is_inert() {
        let mut m = menu(0);
        assert!(m.is_empty());
        assert_eq!(m.handle(MenuInput::Confirm), None);
        assert_eq!(m.handle(MenuInput::Down), None);
    }
}
