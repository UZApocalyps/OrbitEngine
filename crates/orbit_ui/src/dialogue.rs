//! The dialogue textbox.

use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId};
use orbit_text::{DrawParams, TextBlock, TextEngine, TextRenderer, TextStyle};

use crate::theme::{TextBoxTheme, tint};

/// What an "advance" input did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// The typewriter was still running: the whole line is now visible.
    Revealed,
    /// The line was already complete: the caller should show the next one.
    Next,
}

/// Panel + speaker name + typewriter text.
pub struct DialogueBox {
    theme: TextBoxTheme,
    name: Option<TextBlock>,
    text: Option<TextBlock>,
    time: f32,
}

impl DialogueBox {
    pub fn new(theme: TextBoxTheme) -> Self {
        Self {
            theme,
            name: None,
            text: None,
            time: 0.0,
        }
    }

    pub fn set_theme(&mut self, theme: TextBoxTheme) {
        self.theme = theme;
    }

    /// Shows a new line. `speaker` is the display name and its sRGB color
    /// (`None` for the narrator). `markup` supports the `orbit_text` tags.
    pub fn set_line(&mut self, speaker: Option<(&str, [u8; 3])>, markup: &str) {
        let t = &self.theme;
        self.name = speaker.map(|(name, [r, g, b])| {
            let mut block = TextBlock::new(
                name,
                TextStyle {
                    font_size: t.name_size,
                    color: [r, g, b, 255],
                    ..Default::default()
                },
                Some(t.rect[2] - 2.0 * t.padding[0]),
                f32::INFINITY,
            );
            block.skip_to_end(); // names are shown at once
            block
        });
        self.text = Some(TextBlock::new(
            markup,
            TextStyle {
                font_size: t.text_size,
                color: t.text_color,
                ..Default::default()
            },
            Some(t.rect[2] - 2.0 * t.padding[0]),
            t.chars_per_second,
        ));
    }

    /// Removes the current line (the panel is not drawn without one).
    pub fn clear(&mut self) {
        self.name = None;
        self.text = None;
    }

    pub fn has_line(&self) -> bool {
        self.text.is_some()
    }

    pub fn update(&mut self, dt: f32) {
        self.time += dt;
        if let Some(text) = &mut self.text {
            text.update(dt);
        }
    }

    /// `true` when there is no line or the typewriter finished.
    pub fn is_complete(&self) -> bool {
        self.text.as_ref().is_none_or(TextBlock::is_complete)
    }

    /// Player pressed "advance": completes the line first, then asks for the
    /// next one.
    pub fn advance(&mut self) -> Advance {
        match &mut self.text {
            Some(text) if !text.is_complete() => {
                text.skip_to_end();
                Advance::Revealed
            }
            _ => Advance::Next,
        }
    }

    /// Appends the panel and its text to `scene` (UI layer).
    pub fn draw(
        &mut self,
        engine: &mut TextEngine,
        text_renderer: &mut TextRenderer,
        renderer: &Renderer2d,
        scene: &mut Scene,
        white: TextureId,
    ) {
        let Some(text) = &mut self.text else { return };
        let t = &self.theme;
        let [x, y, w, h] = t.rect;
        scene.push(
            Sprite2d::new(white, [x, y], [w, h])
                .anchor([0.0, 0.0])
                .layer(Layer::UI)
                .z(0.0)
                .tint(tint(t.background)),
        );
        if let Some(name) = &mut self.name {
            let params = DrawParams {
                origin: [x + t.padding[0], y + t.padding[1]],
                ..Default::default()
            };
            name.draw(engine, text_renderer, renderer, scene, &params);
        }
        let params = DrawParams {
            origin: [x + t.padding[0], y + t.padding[1] + t.text_offset],
            ..Default::default()
        };
        text.draw(engine, text_renderer, renderer, scene, &params);
        if text.is_complete() {
            let blink = if t.indicator_blink {
                0.55 + 0.45 * (self.time * 5.0).sin()
            } else {
                1.0
            };
            let size = t.indicator_size;
            scene.push(
                Sprite2d::new(
                    white,
                    [x + w - t.padding[0] - size, y + h - t.padding[1] - size],
                    [size, size],
                )
                .rotation(std::f32::consts::FRAC_PI_4)
                .layer(Layer::UI)
                .z(2.0)
                .tint(tint(t.indicator_color))
                .alpha(blink),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(chars_per_second: f32) -> DialogueBox {
        let mut b = DialogueBox::new(TextBoxTheme {
            chars_per_second,
            ..Default::default()
        });
        b.set_line(Some(("Alice", [255, 0, 0])), "Hello there, traveler.");
        b
    }

    #[test]
    fn no_line_is_complete_and_advances() {
        let mut b = DialogueBox::new(TextBoxTheme::default());
        assert!(!b.has_line());
        assert!(b.is_complete());
        assert_eq!(b.advance(), Advance::Next);
    }

    #[test]
    fn first_advance_reveals_second_goes_next() {
        let mut b = boxed(10.0);
        assert!(!b.is_complete());
        assert_eq!(b.advance(), Advance::Revealed);
        assert!(b.is_complete());
        assert_eq!(b.advance(), Advance::Next);
    }

    #[test]
    fn typewriter_completes_with_time() {
        let mut b = boxed(100.0);
        b.update(0.05);
        assert!(!b.is_complete());
        b.update(1.0);
        assert!(b.is_complete());
    }

    #[test]
    fn new_line_restarts_the_typewriter() {
        let mut b = boxed(10.0);
        b.advance();
        b.set_line(None, "Another line");
        assert!(!b.is_complete());
        b.clear();
        assert!(!b.has_line());
    }
}
