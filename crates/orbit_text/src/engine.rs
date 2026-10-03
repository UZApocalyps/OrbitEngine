//! Font management and text layout (no GPU involved).

use cosmic_text::{
    Align, Attrs, Buffer, CacheKey, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache,
    SwashContent, Weight, Wrap, fontdb,
};
use std::collections::BTreeMap;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use crate::linebreak::{has_kinsoku_chars, is_invisible, is_rtl_language, no_line_end, no_line_start};
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
    /// Current language code and the font chain of each language (M7-02).
    language: String,
    language_fonts: BTreeMap<String, Vec<String>>,
    /// Bumped whenever fonts or the language profile change, so cached
    /// layouts know they are stale.
    generation: u32,
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
            language: String::new(),
            language_fonts: BTreeMap::new(),
            generation: 0,
        }
    }

    /// Registers a TTF/OTF/TTC font. Returns the number of faces it added
    /// (0 means the data was not a usable font).
    pub fn add_font_data(&mut self, data: Vec<u8>) -> usize {
        let db = self.fonts.db_mut();
        let before = db.len();
        db.load_font_data(data);
        let added = db.len() - before;
        if added > 0 {
            self.generation = self.generation.wrapping_add(1);
        }
        added
    }

    /// Changes whenever fonts or the language profile change: a cached
    /// [`TextLayout`] made at an older generation must be recomputed.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Language of the text drawn from now on: selects its font chain and,
    /// for right-to-left languages, right-aligns left-aligned text.
    pub fn set_language(&mut self, code: &str) {
        if self.language != code {
            self.language = code.to_owned();
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// Per-language font fallback chains (family names, first = preferred),
    /// keyed by language code (`"ja"`; a regional code falls back to its
    /// primary subtag).
    pub fn set_language_fonts(&mut self, chains: BTreeMap<String, Vec<String>>) {
        if self.language_fonts != chains {
            self.language_fonts = chains;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// The font chain configured for the current language.
    pub fn language_chain(&self) -> &[String] {
        self.chain_for(&self.language)
    }

    fn chain_for(&self, code: &str) -> &[String] {
        let primary = code.split(['-', '_']).next().unwrap_or("");
        self.language_fonts
            .get(code)
            .or_else(|| self.language_fonts.get(primary))
            .map_or(&[][..], Vec::as_slice)
    }

    /// Faces of `family` (case-insensitive), ready to be asked for glyphs.
    fn family_fonts(&mut self, family: &str) -> Vec<Arc<cosmic_text::Font>> {
        let ids: Vec<fontdb::ID> = self
            .fonts
            .db()
            .faces()
            .filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(family)))
            .map(|f| f.id)
            .collect();
        ids.into_iter()
            .filter_map(|id| self.fonts.get_font(id, fontdb::Weight::NORMAL))
            .collect()
    }

    /// The characters of `text` that no family of `chain` can draw (spaces
    /// and control characters excluded), each reported once, in order. An
    /// empty `chain` means the default family. Basis of the "this language
    /// needs a font that covers its text" warning.
    pub fn missing_chars(&mut self, chain: &[String], text: &str) -> Vec<char> {
        let default = [self.default_family.clone()];
        let chain = if chain.is_empty() { &default[..] } else { chain };
        let faces: Vec<_> = chain.iter().flat_map(|f| self.family_fonts(f)).collect();
        let mut missing = Vec::new();
        for c in text.chars() {
            if is_invisible(c) || missing.contains(&c) {
                continue;
            }
            if !faces.iter().any(|f| f.as_swash().charmap().map(c) != 0) {
                missing.push(c);
            }
        }
        missing
    }

    /// Index in `candidates` of the first family that draws every visible
    /// character of `text`; else the one that draws the most (the first on a
    /// tie). `None` for an empty candidate list.
    pub fn select_family(&mut self, candidates: &[String], text: &str) -> Option<usize> {
        let mut chars: Vec<char> = text.chars().filter(|c| !is_invisible(*c)).collect();
        chars.sort_unstable();
        chars.dedup();
        let mut best: Option<(usize, usize)> = None;
        for (i, family) in candidates.iter().enumerate() {
            let faces = self.family_fonts(family);
            let covered = chars
                .iter()
                .filter(|&&c| faces.iter().any(|f| f.as_swash().charmap().map(c) != 0))
                .count();
            if covered == chars.len() && !faces.is_empty() {
                return Some(i);
            }
            if best.is_none_or(|(_, n)| covered > n) {
                best = Some((i, covered));
            }
        }
        best.map(|(i, _)| i)
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
        // Font: the style's family (else the default) first, then the chain
        // of the current language; the first that draws the whole text wins.
        let plain_text = text.plain();
        let mut candidates = vec![
            style
                .family
                .clone()
                .unwrap_or_else(|| self.default_family.clone()),
        ];
        candidates.extend(self.language_chain().iter().cloned());
        let family = match self.select_family(&candidates, &plain_text) {
            Some(i) => candidates.swap_remove(i),
            None => candidates.swap_remove(0),
        };
        let mut style = style.clone();
        if is_rtl_language(&self.language) && style.align == TextAlign::Left {
            style.align = TextAlign::Right;
        }
        let style = &style;
        // CJK kinsoku: the shaper may break anywhere between ideographs, so
        // lines that would start with closing punctuation (or end with an
        // opening bracket) get an explicit break before the offending
        // character, until no line breaks the rules.
        let mut breaks: Vec<usize> = Vec::new();
        let kinsoku = has_kinsoku_chars(&plain_text);
        for _ in 0..24 {
            let (layout, forced) = self.layout_pass(text, style, &family, box_width, scale, &breaks, kinsoku);
            if forced.is_empty() {
                return layout;
            }
            breaks.extend(forced);
            breaks.sort_unstable();
            breaks.dedup();
        }
        self.layout_pass(text, style, &family, box_width, scale, &breaks, false).0
    }

    /// One shaping pass. `breaks` are original byte offsets that get a hard
    /// line break. With `kinsoku` the second value lists the extra break
    /// offsets needed to satisfy the line-start/line-end rules.
    #[allow(clippy::too_many_arguments)]
    fn layout_pass(
        &mut self,
        text: &ParsedText,
        style: &TextStyle,
        family: &str,
        box_width: Option<f32>,
        scale: f32,
        breaks: &[usize],
        kinsoku: bool,
    ) -> (TextLayout, Vec<usize>) {
        let font_size = (style.font_size * scale).max(1.0);
        let metrics = Metrics::new(font_size, font_size * style.line_height.max(0.5));
        let mut buffer = Buffer::new(&mut self.fonts, metrics);
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(box_width.map(|w| (w * scale).max(1.0)), None);

        // Normalize line endings so line indices map to '\n'-separated lines.
        let plain: String = text.plain().replace("\r\n", "\n").replace('\r', "\n");
        let base = |bold: bool, italic: bool, color: [u8; 4]| {
            Attrs::new()
                .family(Family::Name(family))
                .color(Color::rgba(color[0], color[1], color[2], color[3]))
                .weight(if bold { Weight::BOLD } else { Weight::NORMAL })
                .style(if italic { Style::Italic } else { Style::Normal })
        };
        let defaults = base(false, false, style.color);
        let mut seg_start = 0usize;
        let spans: Vec<(String, Attrs)> = text
            .segments
            .iter()
            .map(|s| {
                let normalized = s.text.replace("\r\n", "\n").replace('\r', "\n");
                let shaped = with_breaks(&normalized, seg_start, breaks);
                seg_start += normalized.len();
                (
                    shaped,
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
        let shaped_plain = with_breaks(&plain, 0, breaks);
        let mut line_starts = vec![0usize];
        line_starts.extend(shaped_plain.match_indices('\n').map(|(i, _)| i + 1));
        // Shaped byte -> original byte (inserted breaks take one byte each).
        let to_orig = |shaped: usize| {
            shaped
                - breaks
                    .iter()
                    .enumerate()
                    .filter(|(k, o)| **o + *k < shaped)
                    .count()
        };

        let mut glyphs = Vec::new();
        let mut forced: Vec<usize> = Vec::new();
        let (mut width, mut height) = (0.0f32, 0.0f32);
        let mut line_count = 0;
        for run in buffer.layout_runs() {
            line_count += 1;
            width = width.max(run.line_w);
            height = height.max(run.line_top + run.line_height);
            let line_start = line_starts.get(run.line_i).copied().unwrap_or(0);
            if kinsoku && !run.rtl && run.glyphs.len() > 1 {
                let first = run.glyphs.iter().map(|g| g.start).min().unwrap_or(0);
                let last = run.glyphs.iter().map(|g| g.start).max().unwrap_or(0);
                let first_byte = to_orig(line_start + first);
                let last_byte = to_orig(line_start + last);
                if first > 0
                    && let Some(c) = plain[first_byte..].chars().next()
                    && no_line_start(c)
                    && let Some((p, pc)) = plain[..first_byte].char_indices().next_back()
                    && pc != '\n'
                    && !breaks.contains(&p)
                    && plain[..p].chars().next_back().is_some_and(|b| b != '\n')
                {
                    forced.push(p);
                }
                if let Some(c) = plain[last_byte..].chars().next()
                    && no_line_end(c)
                    && last_byte > first_byte
                    && !breaks.contains(&last_byte)
                    && plain[last_byte + c.len_utf8()..]
                        .chars()
                        .next()
                        .is_some_and(|n| n != '\n')
                {
                    forced.push(last_byte);
                }
            }
            for g in run.glyphs {
                let physical = g.physical((0.0, run.line_y), 1.0);
                let byte = to_orig(line_start + g.start);
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
        let layout = TextLayout {
            glyphs,
            width: width / scale,
            height: height / scale,
            line_count,
            grapheme_count: grapheme_starts.len(),
            scale,
        };
        (layout, forced)
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

/// First family name of a TTF/OTF/TTC file (`None` when the data is not a
/// usable font). Lets a host name the font a project asset provides.
pub fn font_family(data: &[u8]) -> Option<String> {
    let mut db = fontdb::Database::new();
    db.load_font_data(data.to_vec());
    db.faces()
        .next()
        .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
}

/// `text` (which starts at byte `start` of the whole text) with a newline
/// inserted at each original offset of `breaks` that falls inside it.
fn with_breaks(text: &str, start: usize, breaks: &[usize]) -> String {
    let mut out = String::with_capacity(text.len() + breaks.len());
    let mut last = 0;
    for &b in breaks {
        if b > start && b < start + text.len() && text.is_char_boundary(b - start) {
            out.push_str(&text[last..b - start]);
            out.push('\n');
            last = b - start;
        }
    }
    out.push_str(&text[last..]);
    out
}
