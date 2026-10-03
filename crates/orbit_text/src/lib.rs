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
mod linebreak;
mod markup;
mod typewriter;

pub use atlas::{DrawParams, TextRenderer};
pub use block::TextBlock;
pub use linebreak::{
    cjk_break_between, cjk_break_opportunities, is_cjk, is_rtl_language, no_line_end,
    no_line_start,
};
pub use engine::{TextAlign, TextEngine, TextLayout, TextStyle, font_family};
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
    fn font_family_names_a_font_file_and_refuses_garbage() {
        assert_eq!(font_family(INTER).as_deref(), Some("Inter"));
        assert_eq!(font_family(&[1, 2, 3]), None);
        // A style naming a family picks that font, not the default one.
        let mut e = engine(&[INTER, JP]);
        let jp = font_family(JP).unwrap();
        let text = parse_markup("あ");
        let style = TextStyle {
            family: Some(jp),
            ..Default::default()
        };
        let layout = e.layout(&text, &style, Some(400.0), 1.0);
        assert!(layout.glyph_count() > 0 && layout.width > 0.0, "the named family shapes the text");
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

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn fallback_selection_picks_the_first_family_that_covers_the_text() {
        let mut e = engine(&[INTER, JP, ARABIC]);
        let jp = font_family(JP).unwrap();
        let ar = font_family(ARABIC).unwrap();
        let chain = vec!["Inter".to_owned(), jp.clone(), ar.clone()];
        assert_eq!(e.select_family(&chain, "Hello"), Some(0));
        assert_eq!(e.select_family(&chain, "こんにちは"), Some(1));
        assert_eq!(e.select_family(&chain, "مرحبا"), Some(2));
        // Nothing covers everything: the best partial cover wins.
        assert_eq!(e.select_family(&chain, "مرحبا שלום"), Some(2));
        // A family that is not loaded never wins over one that is.
        let chain = vec!["Nope".to_owned(), jp];
        assert_eq!(e.select_family(&chain, "日本"), Some(1));
        assert_eq!(e.select_family(&[], "x"), None);
    }

    #[test]
    fn a_language_chain_makes_the_engine_draw_with_the_covering_font() {
        let mut e = engine(&[INTER, JP]);
        let jp = font_family(JP).unwrap();
        let mut chains = std::collections::BTreeMap::new();
        chains.insert("ja".to_owned(), vec![jp]);
        e.set_language_fonts(chains);
        let g0 = e.generation();
        e.set_language("ja-JP"); // regional code uses the primary subtag
        assert_ne!(e.generation(), g0);
        assert_eq!(e.language_chain().len(), 1);
        let l = e.layout(&parse_markup("日本語"), &TextStyle::default(), Some(400.0), 1.0);
        let glyphs = l.glyphs.clone();
        assert_eq!(glyphs.len(), 3);
        for g in glyphs {
            assert!(e.rasterize(g.key).is_some(), "the JP font draws it");
        }
        e.set_language("fr");
        assert!(e.language_chain().is_empty());
    }

    #[test]
    fn glyph_coverage_reports_what_a_chain_cannot_draw() {
        let mut e = engine(&[INTER, JP, ARABIC]);
        let jp = font_family(JP).unwrap();
        assert_eq!(e.missing_chars(&names(&["Inter"]), "a あ b あ"), vec!['あ']);
        assert!(e.missing_chars(&names(&["Inter", &jp]), "a あ 日本。").is_empty());
        // Empty chain = default family; spaces and controls never count.
        assert!(e.missing_chars(&[], "Hello,\n world").is_empty());
        // Hebrew is covered by none of the loaded fonts.
        let m = e.missing_chars(&names(&["Inter", &jp, &font_family(ARABIC).unwrap()]), "שלום");
        assert_eq!(m.len(), 4);
    }

    /// First character of every visual line (by smallest x of each row).
    fn line_first_chars(text: &str, l: &TextLayout) -> Vec<char> {
        let graphemes: Vec<&str> = unicode_segmentation::UnicodeSegmentation::graphemes(text, true).collect();
        let mut rows: Vec<(i32, i32, usize)> = Vec::new(); // (y, x, grapheme)
        for g in &l.glyphs {
            match rows.iter_mut().find(|r| r.0 == g.y) {
                Some(r) => {
                    if g.x < r.1 {
                        r.1 = g.x;
                        r.2 = g.grapheme;
                    }
                }
                None => rows.push((g.y, g.x, g.grapheme)),
            }
        }
        rows.sort_by_key(|r| r.0);
        rows.iter()
            .filter_map(|r| graphemes[r.2].chars().next())
            .collect()
    }

    #[test]
    fn kinsoku_keeps_closing_punctuation_off_the_start_of_a_line() {
        let mut e = engine(&[INTER, JP]);
        let source = "吾輩は猫である。名前はまだ無い。どこで生れたか、とんと見当がつかぬ。「何でも」薄暗い所で、ニャーニャー泣いていた事だけは記憶している。";
        let text = parse_markup(source);
        let mut checked = 0;
        for w in (150..420).step_by(6) {
            let l = e.layout(&text, &TextStyle::default(), Some(w as f32), 1.0);
            assert_eq!(l.glyph_count(), text.grapheme_count(), "w={w}");
            assert!(l.width <= w as f32 + 0.5, "w={w} width {}", l.width);
            for c in line_first_chars(source, &l) {
                assert!(!crate::no_line_start(c), "line starts with {c} at w={w}");
                checked += 1;
            }
            // And no line ends with an opening bracket: the char before a
            // line start is never one.
        }
        assert!(checked > 40);
    }

    #[test]
    fn kinsoku_keeps_opening_brackets_off_the_end_of_a_line() {
        let mut e = engine(&[INTER, JP]);
        let source = "彼は言った「行こう」と言ってまた「帰ろう」と言ったのでみんな「笑った」のだ";
        let text = parse_markup(source);
        let chars: Vec<char> = source.chars().collect();
        for w in (120..400).step_by(5) {
            let l = e.layout(&text, &TextStyle::default(), Some(w as f32), 1.0);
            let firsts = line_first_chars(source, &l);
            assert!(firsts.iter().all(|c| !crate::no_line_start(*c)), "w={w}");
            // Rebuild line starts as char indices and check the char before.
            let mut starts: Vec<usize> = Vec::new();
            let mut rows: Vec<(i32, i32, usize)> = Vec::new();
            for g in &l.glyphs {
                match rows.iter_mut().find(|r| r.0 == g.y) {
                    Some(r) if g.x < r.1 => {
                        r.1 = g.x;
                        r.2 = g.grapheme;
                    }
                    Some(_) => {}
                    None => rows.push((g.y, g.x, g.grapheme)),
                }
            }
            starts.extend(rows.iter().map(|r| r.2));
            for s in starts.into_iter().filter(|s| *s > 0) {
                assert!(!crate::no_line_end(chars[s - 1]), "line ends with an opening bracket at w={w}");
            }
        }
    }

    #[test]
    fn latin_wrapping_is_unchanged_by_the_kinsoku_pass() {
        let mut e = engine(&[INTER]);
        let t = parse_markup("The quick brown fox jumps over the lazy dog and keeps running.");
        let a = e.layout(&t, &TextStyle::default(), Some(260.0), 1.0);
        assert!(a.line_count >= 3);
        // Every letter is drawn (spaces may or may not be glyphs).
        let letters = t.plain().chars().filter(|c| !c.is_whitespace()).count();
        assert!(a.glyph_count() >= letters && a.glyph_count() <= t.grapheme_count());
    }

    #[test]
    fn bidi_orders_arabic_glyphs_right_to_left() {
        let mut e = engine(&[INTER, ARABIC]);
        let t = parse_markup("مرحبا بالعالم");
        let l = e.layout(&t, &TextStyle::default(), Some(600.0), 1.0);
        let first = l.glyphs.iter().filter(|g| g.grapheme == 0).map(|g| g.x).max().unwrap();
        let last_g = l.glyphs.iter().map(|g| g.grapheme).max().unwrap();
        let last = l.glyphs.iter().filter(|g| g.grapheme == last_g).map(|g| g.x).min().unwrap();
        assert!(first > last, "first letter {first} must sit right of the last {last}");
        // Latin stays left to right.
        let l = e.layout(&parse_markup("abc"), &TextStyle::default(), Some(600.0), 1.0);
        assert!(l.glyphs[0].x < l.glyphs[2].x);
    }

    #[test]
    fn rtl_languages_right_align_left_aligned_text() {
        let mut e = engine(&[INTER, ARABIC]);
        let t = parse_markup("مرحبا");
        let left_edge = |e: &mut TextEngine| {
            let l = e.layout(&t, &TextStyle::default(), Some(600.0), 1.0);
            l.glyphs.iter().map(|g| g.x).min().unwrap()
        };
        let ltr = left_edge(&mut e);
        e.set_language("ar");
        let rtl = left_edge(&mut e);
        assert!(rtl > ltr + 300, "ltr {ltr} rtl {rtl}: text hugs the right edge");
        // Centered text stays centered.
        let centered = TextStyle { align: TextAlign::Center, ..Default::default() };
        let l = e.layout(&t, &centered, Some(600.0), 1.0);
        let min = l.glyphs.iter().map(|g| g.x).min().unwrap();
        assert!(min > 100 && min < 400);
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
