//! The dialogue textbox.

use std::collections::HashMap;

use orbit_render2d::{Layer, Renderer2d, Scene, Sprite2d, TextureId};
use orbit_text::{DrawParams, TextBlock, TextEngine, TextRenderer, TextStyle};

use crate::bubble::{self, BubbleLayout};
use crate::paint::Painter;
use crate::theme::{
    BubbleCharacter, BubbleKind, BubbleTheme, DialogueMode, Fill, NarratorStyle, OffscreenStyle, Style,
    TextBoxTheme, Theme, WidgetState, resolve_style, tint,
};
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
    /// Font family of the speech bubbles (`BubbleTheme::font` resolved).
    pub bubble_family: Option<String>,
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
            bubble_family: t.bubble.font.as_deref().map(|f| theme.font_family(f).to_owned()),
            panel,
            name,
            plate,
        }
    }
}

/// How the current line is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// The classic dialogue box.
    Box,
    /// A comic bubble near the speaker (`DialogueMode::Bubbles`).
    Bubble,
    /// The narrator's caption (`NarratorStyle::Caption`).
    Caption,
}

/// The texts of the current line laid out for a bubble (built next to the
/// box ones: both typewriters run in step, so the line can switch between
/// the box and a bubble as its speaker enters or leaves the screen).
struct BubbleLine {
    kind: BubbleKind,
    caption: bool,
    rgb: Option<[u8; 3]>,
    name: Option<TextBlock>,
    text: TextBlock,
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
    /// The line in bubble form (`Bubbles` mode only).
    bubble: Option<BubbleLine>,
    /// Character id of the speaker (per-character bubble overrides).
    speaker_key: Option<String>,
    /// `[left, top, right, bottom]` of the speaker's sprite on the canvas
    /// (`None`: not on screen).
    bounds: Option<[f32; 4]>,
    /// Seconds since the line appeared (the bubble pop-in).
    line_time: f32,
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
            bubble: None,
            speaker_key: None,
            bounds: None,
            line_time: 0.0,
        }
    }

    /// The character id of the next lines' speaker (picks its bubble
    /// overrides); call it before [`DialogueBox::set_line`].
    pub fn set_speaker_key(&mut self, key: Option<String>) {
        self.speaker_key = key;
    }

    /// Where the speaker is drawn this frame (`[left, top, right, bottom]`
    /// in canvas pixels; `None`: not on screen). Only bubbles use it.
    pub fn set_speaker_bounds(&mut self, bounds: Option<[f32; 4]>) {
        self.bounds = bounds;
    }

    /// How the current line is shown with the current speaker bounds.
    pub fn presentation(&self) -> Presentation {
        match &self.bubble {
            None => Presentation::Box,
            Some(b) if b.caption => Presentation::Caption,
            Some(_) => {
                let visible = self.bounds.is_some_and(crate::bubble::on_screen);
                if visible || self.theme.bubble.offscreen == OffscreenStyle::Bubble {
                    Presentation::Bubble
                } else {
                    Presentation::Box
                }
            }
        }
    }

    /// The overrides of the current speaker.
    fn overrides(&self) -> Option<&BubbleCharacter> {
        self.speaker_key
            .as_ref()
            .and_then(|k| self.theme.bubble.characters.get(k))
    }

    /// The bubble form of a line, when the theme shows it in one.
    fn bubble_line(&self, speaker: Option<(&str, [u8; 3])>, markup: &str) -> Option<BubbleLine> {
        let t = &self.theme;
        let bt = &t.bubble;
        if t.mode != DialogueMode::Bubbles {
            return None;
        }
        let caption = speaker.is_none();
        if caption && bt.narrator != NarratorStyle::Caption {
            return None;
        }
        let kind = if caption { BubbleKind::Speech } else { BubbleKind::of_line(markup) };
        let text_color = self.overrides().and_then(|o| o.text_color).unwrap_or(bt.text_color);
        // The widest text that keeps the bubble within `max_width`.
        let room = match kind {
            BubbleKind::Speech => bt.max_width - 2.0 * bt.padding[0],
            BubbleKind::Thought => bt.max_width * 0.85 - 2.0 * bt.padding[0] - 32.0,
            BubbleKind::Shout => bt.max_width / 1.5 - 2.0 * bt.padding[0],
        }
        .max(60.0);
        let family = self.styles.bubble_family.clone();
        let name = speaker.filter(|_| bt.show_name).map(|(name, [r, g, b])| {
            let color = if t.speaker_color { [r, g, b, 255] } else { text_color };
            let mut block = TextBlock::new(
                &format!("{{b}}{}{{/b}}", escape_markup(name)),
                TextStyle {
                    font_size: bt.name_size,
                    color,
                    family: family.clone(),
                    ..Default::default()
                },
                Some(room),
                f32::INFINITY,
            );
            block.skip_to_end();
            block
        });
        let text = TextBlock::new(
            markup,
            TextStyle {
                font_size: bt.text_size,
                color: text_color,
                family,
                ..Default::default()
            },
            Some(room),
            t.chars_per_second,
        );
        Some(BubbleLine {
            kind,
            caption,
            rgb: speaker.map(|(_, c)| c),
            name,
            text,
        })
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
        let bubble = self.bubble_line(speaker, markup);
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
        self.bubble = bubble;
        self.line_time = 0.0;
    }

    /// Removes the current line (the panel is not drawn without one).
    pub fn clear(&mut self) {
        self.name = None;
        self.text = None;
        self.bubble = None;
    }

    pub fn has_line(&self) -> bool {
        self.text.is_some()
    }

    pub fn update(&mut self, dt: f32) {
        self.time += dt;
        self.line_time += dt;
        if let Some(text) = &mut self.text {
            text.update(dt);
        }
        if let Some(b) = &mut self.bubble {
            b.text.update(dt);
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
                if let Some(b) = &mut self.bubble {
                    b.text.skip_to_end();
                }
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
        if self.presentation() != Presentation::Box {
            self.draw_bubble(engine, text_renderer, renderer, scene, white);
            return;
        }
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

impl DialogueBox {
    /// The bubble (or the narrator caption) of the current line.
    fn draw_bubble(
        &mut self,
        engine: &mut TextEngine,
        text_renderer: &mut TextRenderer,
        renderer: &Renderer2d,
        scene: &mut Scene,
        white: TextureId,
    ) {
        let over = self.overrides().cloned().unwrap_or_default();
        let t = &self.theme;
        let bt = &t.bubble;
        let bounds = self.bounds;
        let Some(b) = &mut self.bubble else { return };
        let text_size = b.text.measure(engine, renderer);
        let name_size = b.name.as_mut().map_or([0.0, 0.0], |n| n.measure(engine, renderer));
        let name_gap = if b.name.is_some() { 4.0 } else { 0.0 };
        let content = [text_size[0].max(name_size[0]), name_size[1] + name_gap + text_size[1]];
        let caption_theme;
        let (layout, kind, shape_theme, fill) = if b.caption {
            caption_theme = BubbleTheme {
                corner_radius: bt.corner_radius.min(6.0),
                border_width: bt.border_width,
                padding: bt.padding,
                ..BubbleTheme::default()
            };
            let size = bubble::bubble_size(content, BubbleKind::Speech, bt);
            let layout = BubbleLayout {
                rect: bubble::caption_rect(size, bt),
                tail: None,
            };
            (layout, BubbleKind::Speech, &caption_theme, bt.caption_fill)
        } else {
            let size = bubble::bubble_size(content, b.kind, bt);
            let layout = bubble::place(size, bounds, over.head_offset, bt);
            (layout, b.kind, bt, over.fill.unwrap_or(bt.fill))
        };
        let border = over
            .border
            .or_else(|| {
                b.rgb
                    .filter(|_| bt.speaker_border && !b.caption)
                    .map(|[r, g, b]| [r, g, b, 255])
            })
            .unwrap_or(bt.border);
        let (scale, alpha) = bubble::pop(self.line_time, bt.pop);
        let [x, y, w, h] = layout.rect;
        let pivot = layout.tail.map_or([x + w * 0.5, y + h * 0.5], |tail| tail.base);
        let shapes = bubble::shapes(&layout, kind, shape_theme);
        bubble::push_shapes(scene, white, &shapes, fill, border, 0.0, pivot, scale, alpha);
        let origin = [x + (w - content[0]) * 0.5, y + (h - content[1]) * 0.5];
        if let Some(name) = &mut b.name {
            let params = DrawParams {
                origin,
                alpha,
                ..Default::default()
            };
            name.draw(engine, text_renderer, renderer, scene, &params);
        }
        let params = DrawParams {
            origin: [origin[0], origin[1] + name_size[1] + name_gap],
            alpha,
            ..Default::default()
        };
        b.text.draw(engine, text_renderer, renderer, scene, &params);
        if b.text.is_complete() {
            let blink = if t.indicator_blink {
                0.55 + 0.45 * (self.time * 5.0).sin()
            } else {
                1.0
            };
            let size = t.indicator_size * 0.7;
            let color = over.text_color.unwrap_or(bt.text_color);
            scene.push(
                Sprite2d::new(
                    white,
                    [origin[0] + content[0] + size, origin[1] + content[1] - size * 0.6],
                    [size, size],
                )
                .rotation(std::f32::consts::FRAC_PI_4)
                .layer(Layer::UI)
                .z(2.0)
                .tint(tint(color))
                .alpha(blink * alpha),
            );
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

    fn bubbly(narrator: NarratorStyle, offscreen: OffscreenStyle) -> DialogueBox {
        let mut t = TextBoxTheme {
            chars_per_second: 10.0,
            mode: DialogueMode::Bubbles,
            ..Default::default()
        };
        t.bubble.narrator = narrator;
        t.bubble.offscreen = offscreen;
        DialogueBox::new(t)
    }

    #[test]
    fn the_box_mode_never_shows_bubbles() {
        let mut b = boxed(10.0);
        b.set_speaker_bounds(Some([700.0, 200.0, 1200.0, 1080.0]));
        assert_eq!(b.presentation(), Presentation::Box);
    }

    #[test]
    fn in_bubble_mode_the_narrator_keeps_the_box_and_characters_get_bubbles() {
        let mut b = bubbly(NarratorStyle::Box, OffscreenStyle::Bubble);
        b.set_speaker_bounds(Some([700.0, 200.0, 1200.0, 1080.0]));
        b.set_line(None, "Night falls.");
        assert_eq!(b.presentation(), Presentation::Box);
        b.set_line(Some(("Alice", [255, 0, 0])), "Hello!");
        assert_eq!(b.presentation(), Presentation::Bubble);
        // Caption narrator.
        let mut c = bubbly(NarratorStyle::Caption, OffscreenStyle::Bubble);
        c.set_line(None, "Night falls.");
        assert_eq!(c.presentation(), Presentation::Caption);
    }

    #[test]
    fn a_speaker_off_screen_falls_back_as_configured() {
        let mut b = bubbly(NarratorStyle::Box, OffscreenStyle::Bubble);
        b.set_line(Some(("Alice", [255, 0, 0])), "Hello!");
        b.set_speaker_bounds(None);
        assert_eq!(b.presentation(), Presentation::Bubble, "a tail-less bubble");
        let mut b = bubbly(NarratorStyle::Box, OffscreenStyle::Box);
        b.set_line(Some(("Alice", [255, 0, 0])), "Hello!");
        b.set_speaker_bounds(None);
        assert_eq!(b.presentation(), Presentation::Box);
        b.set_speaker_bounds(Some([-900.0, 0.0, -300.0, 1080.0]));
        assert_eq!(b.presentation(), Presentation::Box, "outside the canvas");
        b.set_speaker_bounds(Some([700.0, 200.0, 1200.0, 1080.0]));
        assert_eq!(b.presentation(), Presentation::Bubble);
    }

    #[test]
    fn the_bubble_typewriter_runs_with_the_box_one() {
        let mut b = bubbly(NarratorStyle::Box, OffscreenStyle::Bubble);
        b.set_line(Some(("Alice", [255, 0, 0])), "{bubble=shout}Hello there, traveler.");
        assert_eq!(b.bubble.as_ref().unwrap().kind, BubbleKind::Shout);
        assert!(!b.is_complete());
        assert_eq!(b.advance(), Advance::Revealed);
        assert!(b.bubble.as_ref().unwrap().text.is_complete());
        assert_eq!(b.advance(), Advance::Next);
        b.clear();
        assert!(b.bubble.is_none());
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
