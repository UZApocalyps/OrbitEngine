//! Line breaking rules for CJK text (M7-02): break opportunities between
//! ideographs plus the basic kinsoku (prohibition) rules, and the helpers the
//! engine needs to pick fonts per script and direction.

/// `true` for a character a line may break around without a space: han
/// ideographs, kana, hangul syllables, bopomofo and the CJK symbols block.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF       // hangul jamo
        | 0x2E80..=0x303F     // radicals, CJK symbols and punctuation
        | 0x3040..=0x30FF     // hiragana, katakana
        | 0x3100..=0x312F     // bopomofo
        | 0x3130..=0x318F     // hangul compatibility jamo
        | 0x31A0..=0x31FF     // bopomofo ext, katakana ext
        | 0x3200..=0x33FF     // enclosed CJK, CJK compatibility
        | 0x3400..=0x4DBF     // extension A
        | 0x4E00..=0x9FFF     // unified ideographs
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7FF     // hangul syllables
        | 0xF900..=0xFAFF     // compatibility ideographs
        | 0xFE30..=0xFE4F     // CJK compatibility forms
        | 0xFF00..=0xFFEF     // full-width forms
        | 0x20000..=0x2FFFF
        | 0x30000..=0x3FFFF)
}

/// Characters that must not start a line (closing brackets, sentence
/// punctuation, small kana, the prolonged sound mark, iteration marks).
pub fn no_line_start(c: char) -> bool {
    matches!(
        c,
        '、' | '。' | '，' | '．' | '・' | '：' | '；' | '！' | '？' | '）' | '］' | '｝' | '〕'
            | '〉' | '》' | '」' | '』' | '】' | '〙' | '〗' | '｠' | '»' | '”' | '’' | '〜'
            | '～' | '…' | '‥' | 'ー' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '々' | '〻' | 'ぁ' | 'ぃ' | 'ぅ'
            | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ'
            | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ' | '゛' | '゜'
    )
}

/// Characters that must not end a line (opening brackets and quotes).
pub fn no_line_end(c: char) -> bool {
    matches!(
        c,
        '（' | '［' | '｛' | '〔' | '〈' | '《' | '「' | '『' | '【' | '〘' | '〖' | '｟' | '«' | '“'
            | '‘'
    )
}

/// `true` when the text contains a character the kinsoku rules look at.
pub(crate) fn has_kinsoku_chars(text: &str) -> bool {
    text.chars().any(|c| no_line_start(c) || no_line_end(c))
}

/// Whether a line may break between `a` and `b` for CJK purposes: `Some(bool)`
/// when a CJK rule decides, `None` when neither side is CJK (the regular
/// word breaking applies).
pub fn cjk_break_between(a: char, b: char) -> Option<bool> {
    if !is_cjk(a) && !is_cjk(b) && !no_line_start(b) && !no_line_end(a) {
        return None;
    }
    if no_line_start(b) || no_line_end(a) {
        return Some(false);
    }
    // Whitespace is handled by the regular rules.
    if a.is_whitespace() || b.is_whitespace() {
        return None;
    }
    Some(is_cjk(a) || is_cjk(b))
}

/// Byte offsets in `text` where a line may break next to a CJK character
/// (kinsoku applied). Offsets are on `char` boundaries, never 0.
pub fn cjk_break_opportunities(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    for (i, c) in text.char_indices() {
        if let Some(p) = prev
            && cjk_break_between(p, c) == Some(true)
        {
            out.push(i);
        }
        prev = Some(c);
    }
    out
}

/// Languages written right to left (primary subtag of a BCP 47 code).
pub fn is_rtl_language(code: &str) -> bool {
    let primary = code
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        primary.as_str(),
        "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "sd" | "yi" | "dv" | "ug" | "ckb" | "ku"
    )
}

/// `true` for characters that have no glyph of their own (spaces, controls,
/// direction marks, joiners, variation selectors).
pub(crate) fn is_invisible(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c as u32,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
            | 0xFE00..=0xFE0F | 0xFEFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_fall_between_ideographs_but_not_before_closing_punctuation() {
        let t = "吾輩は猫である。名前";
        let offs = cjk_break_opportunities(t);
        let at = |needle: &str| t.find(needle).unwrap();
        assert!(offs.contains(&at("輩")));
        assert!(offs.contains(&at("名")));
        // Not between "る" and "。" (kinsoku: 。 never starts a line).
        assert!(!offs.contains(&at("。")));
    }

    #[test]
    fn opening_brackets_stay_with_what_follows() {
        let t = "彼は「行く」と言った";
        let offs = cjk_break_opportunities(t);
        assert!(!offs.contains(&t.find('行').unwrap()), "no break after 「");
        assert!(!offs.contains(&t.find('」').unwrap()));
        assert!(offs.contains(&t.find('と').unwrap()));
    }

    #[test]
    fn latin_pairs_are_left_to_word_breaking() {
        assert_eq!(cjk_break_between('a', 'b'), None);
        assert_eq!(cjk_break_between('a', ' '), None);
        assert_eq!(cjk_break_between('a', '漢'), Some(true));
        assert_eq!(cjk_break_between('漢', '、'), Some(false));
        assert!(cjk_break_opportunities("hello world").is_empty());
    }

    #[test]
    fn rtl_languages_are_recognised() {
        for code in ["ar", "ar-EG", "he", "fa_IR", "ur"] {
            assert!(is_rtl_language(code), "{code}");
        }
        for code in ["en", "fr", "ja", "zh-Hans", ""] {
            assert!(!is_rtl_language(code), "{code}");
        }
    }
}
