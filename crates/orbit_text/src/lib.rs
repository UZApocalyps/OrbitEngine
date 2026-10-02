//! OrbitEngine text: layout with `cosmic-text`, a dynamic glyph atlas drawn
//! through `orbit_render2d`, a typewriter effect and an inline tag syntax.
//!
//! * [`TextEngine`]: fonts + shaping + layout (CPU only, testable headless).
//! * [`TextRenderer`]: glyph atlas texture and drawing of a [`TextLayout`].
//! * [`TextBlock`]: markup + layout + [`Typewriter`] glued together.
//! * [`parse_markup`]: `{color=#ff0}..{/color}`, `{b}`, `{i}`, `{w=0.5}`.
//!
//! Known limits (M1):
//! * RTL (Arabic/Hebrew) is shaped and ordered by cosmic-text (bidi +
//!   HarfBuzz-compatible shaping) and does not panic, but it is not visually
//!   validated against a native speaker, and the typewriter reveals graphemes in
//!   *logical* order (so RTL text appears from the right side of the box only
//!   when the line is right-aligned).
//! * Ruby/furigana is not implemented.
//! * Only system-independent fonts you add yourself are used (no system font
//!   loading), so web and native render identically.

mod atlas;
mod block;
mod engine;
mod markup;
mod typewriter;

pub use atlas::{DrawParams, TextRenderer};
pub use block::TextBlock;
pub use engine::{TextAlign, TextEngine, TextLayout, TextStyle};
pub use markup::{
    ParsedText, Pause, Segment, SegmentStyle, parse_color, parse_markup, srgb_to_linear,
};
pub use typewriter::Typewriter;

#[cfg(test)]
mod tests {
    use super::*;

    const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
    const INTER_BOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-Bold.ttf");
    const JP: &[u8] = include_bytes!("../../../assets/fonts/cjk/NotoSansJP-Regular.otf");
    const ARABIC: &[u8] = include_bytes!("../../../assets/fonts/cjk/NotoSansArabic-VF.ttf");

    fn engine(with: &[&[u8]]) -> TextEngine {
        let mut e = TextEngine::new();
        for f in with {
            assert!(e.add_font_data(f.to_vec()) > 0, "font failed to load");
        }
        e.set_default_family("Inter");
        e
    }

    #[test]
    fn latin_wraps_inside_the_box() {
        let mut e = engine(&[INTER, INTER_BOLD]);
        let text =
            parse_markup("The quick brown fox jumps over the lazy dog. {b}Bold words{/b} follow.");
        let wide = e.layout(&text, &TextStyle::default(), Some(2000.0), 1.0);
        assert_eq!(wide.line_count, 1);
        let narrow = e.layout(&text, &TextStyle::default(), Some(300.0), 1.0);
        assert!(narrow.line_count > 2, "got {} lines", narrow.line_count);
        assert!(narrow.width <= 300.5, "width {}", narrow.width);
        assert!(narrow.height > wide.height);
        assert_eq!(narrow.grapheme_count, text.grapheme_count());
        assert!(narrow.glyph_count() >= 50);
    }

    #[test]
    fn multiline_and_empty_text() {
        let mut e = engine(&[INTER]);
        let l = e.layout(
            &parse_markup("a\nb\r\nc"),
            &TextStyle::default(),
            Some(500.0),
            1.0,
        );
        assert_eq!(l.line_count, 3);
        let empty = e.layout(&parse_markup(""), &TextStyle::default(), Some(500.0), 1.0);
        assert_eq!(empty.glyph_count(), 0);
    }

    #[test]
    fn grapheme_indices_follow_text_order() {
        let mut e = engine(&[INTER]);
        let l = e.layout(
            &parse_markup("ab\ncd"),
            &TextStyle::default(),
            Some(500.0),
            1.0,
        );
        let g: Vec<usize> = l.glyphs.iter().map(|g| g.grapheme).collect();
        assert_eq!(g, vec![0, 1, 3, 4]); // '\n' is grapheme 2
    }

    #[test]
    fn scale_changes_pixel_size_not_virtual_size() {
        let mut e = engine(&[INTER]);
        let t = parse_markup("Hello world");
        let a = e.layout(&t, &TextStyle::default(), Some(800.0), 1.0);
        let b = e.layout(&t, &TextStyle::default(), Some(800.0), 2.0);
        assert!((a.width - b.width).abs() / a.width < 0.05);
    }

    #[test]
    fn cjk_is_shaped_and_wrapped_without_spaces() {
        let mut e = engine(&[INTER, JP]);
        let text =
            parse_markup("吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。");
        let l = e.layout(&text, &TextStyle::default(), Some(400.0), 1.0);
        assert_eq!(l.glyph_count(), text.grapheme_count());
        assert!(l.line_count >= 3, "got {} lines", l.line_count);
        assert!(l.width <= 400.5);
        // Glyphs must really rasterize (the JP font has them).
        let mut with_image = 0;
        let glyphs: Vec<_> = l.glyphs.clone();
        for g in glyphs {
            if e.rasterize(g.key).is_some() {
                with_image += 1;
            }
        }
        assert_eq!(with_image, text.grapheme_count());
    }

    #[test]
    fn rtl_text_does_not_panic() {
        let mut e = engine(&[INTER, ARABIC]);
        let arabic = parse_markup("مرحبا بالعالم");
        let l = e.layout(&arabic, &TextStyle::default(), Some(500.0), 1.0);
        assert!(l.glyph_count() > 0);
        assert!(l.line_count >= 1);
        // Mixed LTR/RTL and a script no loaded font covers (Hebrew): no panic.
        let hebrew = parse_markup("שלום עולם and mixed English مرحبا");
        let l = e.layout(&hebrew, &TextStyle::default(), Some(500.0), 1.0);
        assert!(l.line_count >= 1);
        // Right alignment must work too.
        let style = TextStyle {
            align: TextAlign::Right,
            ..Default::default()
        };
        let _ = e.layout(&arabic, &style, Some(500.0), 1.0);
    }

    #[test]
    fn font_data_garbage_is_rejected() {
        let mut e = TextEngine::new();
        assert_eq!(e.add_font_data(vec![1, 2, 3]), 0);
        // Layout with no fonts at all must not panic either.
        let l = e.layout(
            &parse_markup("hello"),
            &TextStyle::default(),
            Some(100.0),
            1.0,
        );
        assert_eq!(l.glyph_count(), 0);
    }
}
