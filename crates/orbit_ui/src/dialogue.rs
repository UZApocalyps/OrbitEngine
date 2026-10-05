//! The dialogue textbox.

use std::collections::HashMap;

use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId};
use orbit_text::{DrawParams, TextBlock, TextEngine, TextRenderer, TextStyle};

use crate::paint::Painter;
use crate::theme::{Fill, Style, TextBoxTheme, Theme, WidgetState, resolve_style, tint};
use crate::widgets::{escape_markup, estimate_text_width};

/// What an "advance" input did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// The typewriter was still running: the whole line is now visible.
    Revealed,
    /// The line was already complete: the caller should show the next one.
    Next,
}

/// The styles the textbox section of a theme names, resolved (`ui_version`
/// 7). All `None` for a theme written before: the box then draws exactly
/// as it always did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DialogueStyles {
    pub panel: Option<Style>,
    pub name: Option<Style>,
    pub plate: Option<Style>,
    /// Font families of the panel and name styles (fonts resolved).
    pub text_family: Option<String>,
    pub name_family: Option<String>,
}

impl DialogueStyles {
    pub fn from_theme(theme: &Theme) -> Self {
        let get = |name: &Option<String>| name.as_deref().and_then(|n| resolve_style(&theme.styles, n));
        let t = &theme.textbox;
        let panel = get(&t.style);
        let name = get(&t.name_style);
        let plate = t.name_plate.as_ref().and_then(|p| get(&p.style));
        let family = |s: &Option<Style>| {
            s.as_ref()
                .and_then(|s| s.font.as_deref())
                .map(|f| theme.font_family(f).to_owned())
        };
        Self {
            text_family: family(&panel),
            name_family: family(&name).or_else(|| family(&plate)).or_else(|| family(&panel)),
            panel,
            name,
            plate,
        }
    }
}

/// Panel + speaker name + typewriter text.
pub struct DialogueBox {
    theme: TextBoxTheme,
    styles: DialogueStyles,
    name: Option<TextBlock>,
    /// Estimated width of the name (a plate that fits the name).
    name_width: f32,
    text: Option<TextBlock>,
    time: f32,
}

impl DialogueBox {
    pub fn new(theme: TextBoxTheme) -> Self {
        Self {
            theme,
            styles: DialogueStyles::default(),
            name: None,
            name_width: 0.0,
            text: None,
            time: 0.0,
        }
    }

    pub fn set_theme(&mut self, theme: TextBoxTheme) {
        self.theme = theme;
    }

    /// The resolved styles of the box (see [`DialogueStyles::from_theme`]);
    /// they apply from the next line.
    pub fn set_styles(&mut self, styles: DialogueStyles) {
        self.styles = styles;
    }

    pub fn theme(&self) -> &TextBoxTheme {
        &self.theme
    }

    /// Shows a new line. `speaker` is the display name and its sRGB color
    /// (`None` for the narrator). `markup` supports the `orbit_text` tags.
    pub fn set_line(&mut self, speaker: Option<(&str, [u8; 3])>, markup: &str) {
        let t = &self.theme;
        let st = &self.styles;
        let layout = t.layout(0.0);
        let name_style = st.name.as_ref();
        let name_size = name_style.and_then(|s| s.text_size).unwrap_or(t.name_size);
        self.name_width = speaker.map_or(0.0, |(name, _)| estimate_text_width(name, name_size));
        self.name = speaker.map(|(name, [r, g, b])| {
            let color = if t.speaker_color {
                [r, g, b, 255]
            } else {
                name_style
                    .or(st.plate.as_ref())
                    .and_then(|s| s.text_color_for(WidgetState::Normal))
                    .unwrap_or(t.text_color)
            };
            let bold = name_style.and_then(|s| s.bold) == Some(true);
            let text = if bold {
                format!("{{b}}{}{{/b}}", escape_markup(name))
            } else {
                name.to_owned()
            };
            let width = match t.name_plate {
                // The plate grows with the name: never wrap it.
                Some(_) => 4096.0,
                None => layout.name_width,
            };
            let mut block = TextBlock::new(
                &text,
                TextStyle {
                    font_size: name_size,
                    color,
                    family: st.name_family.clone(),
                    ..Default::default()
                },
                Some(width),
                f32::INFINITY,
            );
            block.skip_to_end(); // names are shown at once
            block
        });
        let panel = st.panel.as_ref();
        let markup = if panel.and_then(|s| s.bold) == Some(true) {
            format!("{{b}}{markup}{{/b}}")
        } else {
            markup.to_owned()
        };
        self.text = Some(TextBlock::new(
            &markup,
            TextStyle {
                font_size: panel.and_then(|s| s.text_size).unwrap_or(t.text_size),
                color: panel
                    .and_then(|s| s.text_color_for(WidgetState::Normal))
                    .unwrap_or(t.text_color),
                align: panel
                    .and_then(|s| s.text_align)
                    .map_or(orbit_text::TextAlign::Left, crate::widgets::text_align),
                family: st.text_family.clone(),
                ..Default::default()
            },
            Some(layout.text_width),
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

    /// Appends the panel and its text to `scene` (UI layer). `images` are
    /// the loaded project images the styles may draw (nine-slices...).
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
        let Some(text) = &mut self.text else { return };
        let t = &self.theme;
        let st = &self.styles;
        let has_name = self.name.is_some();
        let lay = t.layout(if has_name { self.name_width } else { 0.0 });
        let painter = Painter {
            renderer,
            white,
            images,
            alpha: 1.0,
        };
        styled_box(&painter, scene, st.panel.as_ref(), lay.panel, t.background, 0.0);
        if let (Some(plate), true) = (lay.plate, has_name) {
            styled_box(&painter, scene, st.plate.as_ref(), plate, t.background, 0.5);
        }
        if let Some(name) = &mut self.name {
            let params = DrawParams {
                origin: lay.name_origin,
                ..Default::default()
            };
            name.draw(engine, text_renderer, renderer, scene, &params);
        }
        let params = DrawParams {
            origin: lay.text_origin,
            ..Default::default()
        };
        text.draw(engine, text_renderer, renderer, scene, &params);
        if text.is_complete() {
            let blink = if t.indicator_blink {
                0.55 + 0.45 * (self.time * 5.0).sin()
            } else {
                1.0
            };
            let [ix, iy, size, _] = lay.indicator;
            let image = t
                .indicator_image
                .as_deref()
                .and_then(|p| painter.texture(p));
            match image {
                Some((texture, _)) => scene.push(
                    Sprite2d::new(texture, [ix, iy], [size, size])
                        .layer(Layer::UI)
                        .z(2.0)
                        .alpha(blink),
                ),
                None => scene.push(
                    Sprite2d::new(white, [ix, iy], [size, size])
                        .rotation(std::f32::consts::FRAC_PI_4)
                        .layer(Layer::UI)
                        .z(2.0)
                        .tint(tint(t.indicator_color))
                        .alpha(blink),
                ),
            }
        }
    }
}

/// A box with an optional style: without one, the plain `color` rectangle
/// the textbox always drew; with one, its shadow, normal fill (the color
/// when its image is not loaded) and border.
fn styled_box(
    painter: &Painter<'_>,
    scene: &mut Scene,
    style: Option<&Style>,
    rect: [f32; 4],
    color: [u8; 4],
    z: f32,
) {
    let Some(style) = style else {
        scene.push(
            Sprite2d::new(painter.white, [rect[0], rect[1]], [rect[2], rect[3]])
                .anchor([0.0, 0.0])
                .layer(Layer::UI)
                .z(z)
                .tint(tint(color)),
        );
        return;
    };
    if let Some(shadow) = &style.shadow {
        painter.shadow(scene, shadow, rect, Layer::UI, z);
    }
    let fill = style.fill_for(WidgetState::Normal);
    let sliced = match &fill {
        Some(f) if painter.fill(scene, f, rect, Layer::UI, z) => matches!(f, Fill::NineSlice { .. }),
        _ => {
            let c = fill.as_ref().and_then(Fill::flat_color).unwrap_or(color);
            painter.quad(scene, rect, c, Layer::UI, z);
            false
        }
    };
    if !sliced && let Some(border) = &style.border {
        painter.border(scene, rect, border.width, border.color, Layer::UI, z + 0.2);
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

    #[test]
    fn a_legacy_theme_resolves_no_style_and_a_new_one_does() {
        let legacy = Theme::default();
        assert_eq!(DialogueStyles::from_theme(&legacy), DialogueStyles::default());
        let mut theme = Theme::default();
        theme.styles.insert(
            "box".into(),
            Style {
                font: Some("Serif".into()),
                bold: Some(true),
                ..Default::default()
            },
        );
        theme.textbox.style = Some("box".into());
        let s = DialogueStyles::from_theme(&theme);
        assert_eq!(s.text_family.as_deref(), Some("Serif"));
        assert_eq!(s.name_family.as_deref(), Some("Serif"), "the name falls back to the panel font");
        assert!(s.panel.is_some() && s.name.is_none());
    }
}
