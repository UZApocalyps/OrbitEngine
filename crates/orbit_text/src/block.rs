//! A dialogue text block: markup + layout + typewriter, ready to draw.

use orbit_render2d::{Renderer2d, Scene};

use crate::atlas::{DrawParams, TextRenderer};
use crate::engine::{TextEngine, TextLayout, TextStyle};
use crate::markup::{ParsedText, parse_markup};
use crate::typewriter::Typewriter;

/// Markup text in a box with a typewriter reveal. The layout is recomputed
/// automatically when the raster scale (window size) changes.
pub struct TextBlock {
    parsed: ParsedText,
    style: TextStyle,
    box_width: Option<f32>,
    layout: Option<TextLayout>,
    laid_scale: f32,
    laid_generation: u32,
    typewriter: Typewriter,
}

impl TextBlock {
    /// `markup` uses the tag syntax of [`parse_markup`]. `chars_per_second`
    /// configures the typewriter (`f32::INFINITY` shows everything at once).
    pub fn new(
        markup: &str,
        style: TextStyle,
        box_width: Option<f32>,
        chars_per_second: f32,
    ) -> Self {
        let parsed = parse_markup(markup);
        let typewriter = Typewriter::new(
            parsed.grapheme_count(),
            parsed.pauses.clone(),
            chars_per_second,
        );
        Self {
            parsed,
            style,
            box_width,
            layout: None,
            laid_scale: 0.0,
            laid_generation: 0,
            typewriter,
        }
    }

    pub fn typewriter(&self) -> &Typewriter {
        &self.typewriter
    }

    pub fn typewriter_mut(&mut self) -> &mut Typewriter {
        &mut self.typewriter
    }

    pub fn update(&mut self, dt_seconds: f32) {
        self.typewriter.update(dt_seconds);
    }

    pub fn is_complete(&self) -> bool {
        self.typewriter.is_complete()
    }

    pub fn skip_to_end(&mut self) {
        self.typewriter.skip_to_end();
    }

    /// The current layout, if one has been computed by [`TextBlock::draw`].
    pub fn layout(&self) -> Option<&TextLayout> {
        self.layout.as_ref()
    }

    /// Lays the text out (if needed) and returns its `[width, height]` in
    /// virtual pixels, e.g. to size a box around it before drawing.
    pub fn measure(&mut self, engine: &mut TextEngine, renderer: &Renderer2d) -> [f32; 2] {
        self.ensure_layout(engine, renderer.pixel_ratio());
        self.layout.as_ref().map_or([0.0, 0.0], |l| [l.width, l.height])
    }

    /// Lays out (if needed) and appends the revealed glyphs to `scene`.
    /// `origin` is the top-left of the box in virtual pixels.
    pub fn draw(
        &mut self,
        engine: &mut TextEngine,
        text_renderer: &mut TextRenderer,
        renderer: &Renderer2d,
        scene: &mut Scene,
        params: &DrawParams,
    ) {
        self.ensure_layout(engine, renderer.pixel_ratio());
        if let Some(layout) = &self.layout {
            let params = DrawParams {
                revealed: self.typewriter.revealed().min(params.revealed),
                ..*params
            };
            text_renderer.draw(engine, renderer, scene, layout, &params);
        }
    }

    fn ensure_layout(&mut self, engine: &mut TextEngine, scale: f32) {
        if self.layout.is_none()
            || (self.laid_scale - scale).abs() > 1e-3
            || self.laid_generation != engine.generation()
        {
            self.layout = Some(engine.layout(&self.parsed, &self.style, self.box_width, scale));
            self.laid_scale = scale;
            self.laid_generation = engine.generation();
        }
    }
}
