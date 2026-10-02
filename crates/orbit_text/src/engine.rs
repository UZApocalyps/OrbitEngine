//! Font management and text layout (no GPU involved).

use cosmic_text::{
    Align, Attrs, Buffer, CacheKey, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache,
    SwashContent, Weight, Wrap, fontdb,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::markup::ParsedText;

/// Horizontal alignment of lines inside the layout box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justified,
}

impl TextAlign {
    fn to_cosmic(self) -> Align {
        match self {
            Self::Left => Align::Left,
            Self::Center => Align::Center,
            Self::Right => Align::Right,
            Self::Justified => Align::Justified,
        }
    }
}

/// Base style of a text block (segments can override color/weight/slant).
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// Font size in virtual pixels.
    pub font_size: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Default sRGBA color.
    pub color: [u8; 4],
    pub align: TextAlign,
    /// Font family name; `None` uses [`TextEngine::set_default_family`].
    pub family: Option<String>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 36.0,
            line_height: 1.35,
            color: [255, 255, 255, 255],
            align: TextAlign::Left,
            family: None,
        }
    }
}

/// One positioned glyph of a [`TextLayout`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct LaidGlyph {
    pub key: CacheKey,
    /// Glyph origin in layout pixels (already multiplied by the raster scale).
    pub x: i32,
    pub y: i32,
    pub color: [u8; 4],
    /// Index of the grapheme cluster this glyph starts at.
    pub grapheme: usize,
}

/// The result of laying a text out in a box. Positions are in "layout pixels"
/// = virtual pixels x `scale`.
#[derive(Debug, Clone, Default)]
pub struct TextLayout {
    pub(crate) glyphs: Vec<LaidGlyph>,
    /// Width of the widest line, in virtual pixels.
    pub width: f32,
    /// Total height, in virtual pixels.
    pub height: f32,
    pub line_count: usize,
    pub grapheme_count: usize,
    pub(crate) scale: f32,
}

impl TextLayout {
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }
}

/// A rasterized glyph (RGBA8, straight alpha) with its placement.
pub(crate) struct GlyphBitmap {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub rgba: Vec<u8>,
}

/// Owns the fonts and shapes text. Create one per application.
pub struct TextEngine {
    fonts: FontSystem,
    swash: SwashCache,
    default_family: String,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    /// An engine with no fonts: system fonts are deliberately NOT loaded, so
    /// results are identical on every platform (and on the web). Add fonts
    /// with [`TextEngine::add_font_data`].
    pub fn new() -> Self {
        let db = fontdb::Database::new();
        Self {
            fonts: FontSystem::new_with_locale_and_db("en-US".to_owned(), db),
            swash: SwashCache::new(),
            default_family: "sans-serif".to_owned(),
        }
    }

    /// Registers a TTF/OTF/TTC font. Returns the number of faces it added
    /// (0 means the data was not a usable font).
    pub fn add_font_data(&mut self, data: Vec<u8>) -> usize {
        let db = self.fonts.db_mut();
        let before = db.len();
        db.load_font_data(data);
        db.len() - before
    }

    /// Family used when a [`TextStyle`] does not name one.
    pub fn set_default_family(&mut self, family: impl Into<String>) {
        self.default_family = family.into();
    }

    /// Distinct family names currently loaded (diagnostics).
    pub fn families(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .fonts
            .db()
            .faces()
            .filter_map(|f| f.families.first().map(|(n, _)| n.clone()))
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Lays `text` out in a box `box_width` virtual pixels wide (`None` = no
    /// wrapping). `scale` is the number of physical pixels per virtual pixel
    /// (use `Renderer2d::pixel_ratio()`): glyphs are shaped and rasterized at
    /// `font_size * scale` so they stay crisp on screen.
    pub fn layout(
        &mut self,
        text: &ParsedText,
        style: &TextStyle,
        box_width: Option<f32>,
        scale: f32,
    ) -> TextLayout {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        if self.fonts.db().is_empty() {
            // cosmic-text panics without any font: degrade to an empty layout.
            log::warn!("text layout requested but no font is loaded");
            return TextLayout {
                scale,
                grapheme_count: text.grapheme_count(),
                ..Default::default()
            };
        }
        let font_size = (style.font_size * scale).max(1.0);
        let metrics = Metrics::new(font_size, font_size * style.line_height.max(0.5));
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(box_width.map(|w| (w * scale).max(1.0)), None);

        // Normalize line endings so line indices map to '\n'-separated lines.
        let plain: String = text.plain().replace("\r\n", "\n").replace('\r', "\n");
        let family = style.family.as_deref().unwrap_or(&self.default_family);
        let base = |bold: bool, italic: bool, color: [u8; 4]| {
            Attrs::new()
                .family(Family::Name(family))
                .color(Color::rgba(color[0], color[1], color[2], color[3]))
                .weight(if bold { Weight::BOLD } else { Weight::NORMAL })
                .style(if italic { Style::Italic } else { Style::Normal })
        };
        let defaults = base(false, false, style.color);
        let spans: Vec<(String, Attrs)> = text
            .segments
            .iter()
            .map(|s| {
                (
                    s.text.replace("\r\n", "\n").replace('\r', "\n"),
                    base(
                        s.style.bold,
                        s.style.italic,
                        s.style.color.unwrap_or(style.color),
                    ),
                )
            })
            .collect();
        buffer.set_rich_text(
            spans.iter().map(|(t, a)| (t.as_str(), a.clone())),
            &defaults,
            Shaping::Advanced,
            Some(style.align.to_cosmic()),
        );
        buffer.shape_until_scroll(&mut self.fonts, false);

        let grapheme_starts: Vec<usize> = plain.grapheme_indices(true).map(|(i, _)| i).collect();
        let mut line_starts = vec![0usize];
        line_starts.extend(plain.match_indices('\n').map(|(i, _)| i + 1));

        let mut glyphs = Vec::new();
        let (mut width, mut height) = (0.0f32, 0.0f32);
        let mut line_count = 0;
        for run in buffer.layout_runs() {
            line_count += 1;
            width = width.max(run.line_w);
            height = height.max(run.line_top + run.line_height);
            let line_start = line_starts.get(run.line_i).copied().unwrap_or(0);
            for g in run.glyphs {
                let physical = g.physical((0.0, run.line_y), 1.0);
                let byte = line_start + g.start;
                let grapheme = grapheme_starts
                    .partition_point(|&s| s <= byte)
                    .saturating_sub(1);
                let c = g.color_opt.unwrap_or(Color::rgba(
                    style.color[0],
                    style.color[1],
                    style.color[2],
                    style.color[3],
                ));
                glyphs.push(LaidGlyph {
                    key: physical.cache_key,
                    x: physical.x,
                    y: physical.y,
                    color: c.as_rgba(),
                    grapheme,
                });
            }
        }
        TextLayout {
            glyphs,
            width: width / scale,
            height: height / scale,
            line_count,
            grapheme_count: grapheme_starts.len(),
            scale,
        }
    }

    /// Rasterizes one glyph to RGBA8. `None` for glyphs without an image
    /// (spaces, missing outlines).
    pub(crate) fn rasterize(&mut self, key: CacheKey) -> Option<GlyphBitmap> {
        let image = self.swash.get_image(&mut self.fonts, key).as_ref()?;
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            return None;
        }
        let n = w as usize * h as usize;
        let mut rgba = Vec::with_capacity(n * 4);
        match image.content {
            SwashContent::Mask => {
                for &a in image.data.iter().take(n) {
                    rgba.extend_from_slice(&[255, 255, 255, a]);
                }
            }
            SwashContent::Color => {
                rgba.extend_from_slice(&image.data[..(n * 4).min(image.data.len())])
            }
            SwashContent::SubpixelMask => {
                for px in image.data.chunks_exact(4).take(n) {
                    let a = ((px[0] as u16 + px[1] as u16 + px[2] as u16) / 3) as u8;
                    rgba.extend_from_slice(&[255, 255, 255, a]);
                }
            }
        }
        if rgba.len() != n * 4 {
            return None;
        }
        Some(GlyphBitmap {
            width: w,
            height: h,
            left: image.placement.left,
            top: image.placement.top,
            rgba,
        })
    }
}
