//! Glyph coverage of fonts (M7-02): which characters of a text no font of a
//! chain can draw. Pure data (no shaping, no GPU), so the editor and the
//! export can warn about a language whose font cannot render its translation.

use ttf_parser::Face;

use crate::theme::Theme;

/// `true` for characters that never need a glyph (spaces, controls,
/// direction marks, joiners, variation selectors).
fn is_invisible(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c as u32,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
            | 0xFE00..=0xFE0F | 0xFEFF)
}

/// The characters of `text` that none of the TTF/OTF/TTC files in `fonts`
/// has a glyph for, each reported once, in order of first appearance.
/// Unreadable font data covers nothing.
pub fn missing_chars(fonts: &[&[u8]], text: &str) -> Vec<char> {
    let mut faces = Vec::new();
    for data in fonts {
        let count = ttf_parser::fonts_in_collection(data).unwrap_or(1);
        for index in 0..count {
            if let Ok(face) = Face::parse(data, index) {
                faces.push(face);
            }
        }
    }
    let mut missing = Vec::new();
    for c in text.chars() {
        if is_invisible(c) || missing.contains(&c) {
            continue;
        }
        if !faces.iter().any(|f| f.glyph_index(c).is_some()) {
            missing.push(c);
        }
    }
    missing
}

/// Characters of `text` that the fonts of language `lang` cannot draw: the
/// default font plus the chain the theme configures for `lang` (exact code,
/// else its primary subtag). `read` loads a project font asset by path; a
/// chain name that is not a theme font (or whose file cannot be read) adds
/// nothing.
pub fn language_missing_chars(
    theme: Option<&Theme>,
    lang: &str,
    default_font: &[u8],
    read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    text: &str,
) -> Vec<char> {
    let mut loaded: Vec<Vec<u8>> = Vec::new();
    if let Some(theme) = theme {
        let primary = lang.split(['-', '_']).next().unwrap_or("");
        let chain = theme
            .language_fonts
            .get(lang)
            .or_else(|| theme.language_fonts.get(primary));
        for name in chain.into_iter().flatten() {
            if let Some(font) = theme.fonts.get(name)
                && let Some(bytes) = read(&font.asset)
            {
                loaded.push(bytes);
            }
        }
    }
    let mut fonts: Vec<&[u8]> = vec![default_font];
    fonts.extend(loaded.iter().map(Vec::as_slice));
    missing_chars(&fonts, text)
}

/// A short, readable sample of `chars` for a warning (`日, 本, 語 and 4
/// more`).
pub fn describe_chars(chars: &[char]) -> String {
    const SHOWN: usize = 6;
    let head: Vec<String> = chars.iter().take(SHOWN).map(char::to_string).collect();
    let mut out = head.join(", ");
    if chars.len() > SHOWN {
        out.push_str(&format!(" and {} more", chars.len() - SHOWN));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
    const JP: &[u8] = include_bytes!("../../../assets/fonts/cjk/NotoSansJP-Regular.otf");
    const ARABIC: &[u8] = include_bytes!("../../../assets/fonts/cjk/NotoSansArabic-VF.ttf");

    #[test]
    fn latin_is_covered_by_inter_and_cjk_is_not() {
        assert!(missing_chars(&[INTER], "Hello, déjà vu ! \n").is_empty());
        assert_eq!(missing_chars(&[INTER], "a日本日"), vec!['日', '本']);
    }

    #[test]
    fn a_chain_covers_what_any_of_its_fonts_covers() {
        assert!(missing_chars(&[INTER, JP], "Hello 日本語、こんにちは。").is_empty());
        assert!(missing_chars(&[INTER, ARABIC], "مرحبا").is_empty());
        // Hebrew is in none of the three.
        assert_eq!(missing_chars(&[INTER, JP, ARABIC], "שלום").len(), 4);
    }

    #[test]
    fn garbage_font_data_covers_nothing_and_empty_text_is_fine() {
        assert_eq!(missing_chars(&[&[1, 2, 3]], "ab"), vec!['a', 'b']);
        assert!(missing_chars(&[], "").is_empty());
        assert!(missing_chars(&[], " \n\t").is_empty());
    }

    #[test]
    fn a_language_chain_from_the_theme_fixes_the_coverage() {
        let theme = Theme::from_ron(
            "(fonts: {\"jp\": (asset: \"assets/fonts/jp.otf\", family: \"Noto Sans JP\")}, \
              language_fonts: {\"ja\": [\"jp\"], \"zh\": [\"missing\"]})",
        )
        .unwrap();
        let mut read = |path: &str| (path == "assets/fonts/jp.otf").then(|| JP.to_vec());
        let text = "日本語";
        // No chain: the default font alone cannot draw it.
        let m = language_missing_chars(Some(&theme), "fr", INTER, &mut read, text);
        assert_eq!(m.len(), 3);
        assert!(language_missing_chars(None, "ja", INTER, &mut read, text).len() == 3);
        // A regional code uses its primary subtag's chain.
        assert!(language_missing_chars(Some(&theme), "ja-JP", INTER, &mut read, text).is_empty());
        // A chain naming a font the theme does not declare adds nothing.
        assert_eq!(language_missing_chars(Some(&theme), "zh", INTER, &mut read, text).len(), 3);
    }

    #[test]
    fn descriptions_are_truncated() {
        assert_eq!(describe_chars(&['a', 'b']), "a, b");
        let many: Vec<char> = "abcdefghij".chars().collect();
        assert_eq!(describe_chars(&many), "a, b, c, d, e, f and 4 more");
    }
}
