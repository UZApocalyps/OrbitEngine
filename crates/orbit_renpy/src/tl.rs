//! Ren'Py translations (`game/tl/<language>/*.rpy`) to `orbit_i18n`
//! translations.
//!
//! * `translate <lang> strings:` blocks: `old "…"` / `new "…"` pairs, matched
//!   by source text (menu choices, character names, any text).
//! * dialogue blocks (`translate <lang> start_1a2b3c4d:`): Ren'Py writes the
//!   original line as a comment above the translated one; both are paired
//!   in order and matched by the original text.

use std::collections::HashMap;

use orbit_core::StringId;
use orbit_i18n::Translation;

use crate::lexer::{Line, Tok, parse_tree, tokenize};
use crate::text::{self, TextNotes};

/// BCP 47 tag and display name of a Ren'Py language name.
pub fn language(name: &str) -> (String, String) {
    let known: &[(&str, &str, &str)] = &[
        ("english", "en", "English"),
        ("french", "fr", "Français"),
        ("german", "de", "Deutsch"),
        ("spanish", "es", "Español"),
        ("italian", "it", "Italiano"),
        ("portuguese", "pt", "Português"),
        ("brazilian", "pt-BR", "Português (Brasil)"),
        ("russian", "ru", "Русский"),
        ("ukrainian", "uk", "Українська"),
        ("polish", "pl", "Polski"),
        ("dutch", "nl", "Nederlands"),
        ("turkish", "tr", "Türkçe"),
        ("japanese", "ja", "日本語"),
        ("korean", "ko", "한국어"),
        ("chinese", "zh-Hans", "简体中文"),
        ("schinese", "zh-Hans", "简体中文"),
        ("tchinese", "zh-Hant", "繁體中文"),
        ("indonesian", "id", "Bahasa Indonesia"),
        ("vietnamese", "vi", "Tiếng Việt"),
        ("czech", "cs", "Čeština"),
        ("finnish", "fi", "Suomi"),
        ("greek", "el", "Ελληνικά"),
        ("arabic", "ar", "العربية"),
    ];
    let lower = name.to_lowercase();
    match known.iter().find(|(n, _, _)| *n == lower) {
        Some((_, tag, display)) => ((*tag).to_owned(), (*display).to_owned()),
        None => (lower.clone(), name.to_owned()),
    }
}

/// The text of a say statement (`e happy "…"`, `"…"`, `"Name" "…"`).
fn say_text(toks: &[Tok]) -> Option<String> {
    let last_str = toks.iter().rposition(|t| matches!(t, Tok::Str(_)))?;
    let head_ok = toks[..last_str]
        .iter()
        .all(|t| matches!(t, Tok::Ident(_) | Tok::Str(_)));
    if !head_ok {
        return None;
    }
    let first = toks.first()?.ident();
    if matches!(
        first,
        Some("old" | "new" | "translate" | "voice" | "play" | "show" | "scene")
    ) {
        return None;
    }
    match &toks[last_str] {
        Tok::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// Statistics of one language.
pub struct LangResult {
    pub translation: Translation,
    pub matched: usize,
    pub unmatched: usize,
}

/// Builds the translation of one language from its `.rpy` sources.
pub fn translate(
    lang_name: &str,
    sources: &[String],
    index: &HashMap<String, Vec<StringId>>,
    wrappers: &HashMap<StringId, (String, String)>,
) -> LangResult {
    let (tag, display) = language(lang_name);
    let mut t = Translation::new(tag, display);
    let mut matched = 0;
    let mut unmatched = 0;
    let mut put = |original: &str, translated: &str, t: &mut Translation| match index.get(original)
    {
        Some(ids) => {
            let converted = text::convert(translated, &mut TextNotes::default());
            for id in ids {
                // The speaker's style (`what_prefix`, `what_color`...).
                let text = match wrappers.get(id) {
                    Some((before, after)) => format!("{before}{converted}{after}"),
                    None => converted.clone(),
                };
                t.entries.insert(*id, text);
            }
            matched += 1;
        }
        None => unmatched += 1,
    };
    for source in sources {
        for block in parse_tree(source, true) {
            let toks = tokenize(&block.text);
            if !toks.first().is_some_and(|t| t.is_word("translate")) {
                continue;
            }
            let id = toks.get(2).and_then(Tok::ident).unwrap_or("");
            if id == "strings" {
                strings(&block.children, &mut |o, n| put(o, n, &mut t));
            } else if id != "python" && id != "style" {
                dialogue(&block.children, &mut |o, n| put(o, n, &mut t));
            }
        }
    }
    LangResult {
        translation: t,
        matched,
        unmatched,
    }
}

fn strings(lines: &[Line], put: &mut dyn FnMut(&str, &str)) {
    let mut old: Option<String> = None;
    for line in lines {
        if line.text.starts_with('#') {
            continue;
        }
        let toks = tokenize(&line.text);
        match (toks.first().and_then(Tok::ident), toks.get(1)) {
            (Some("old"), Some(Tok::Str(s))) => old = Some(s.clone()),
            (Some("new"), Some(Tok::Str(s))) => {
                if let Some(o) = old.take()
                    && !s.is_empty()
                {
                    put(&o, s);
                }
            }
            _ => {}
        }
    }
}

fn dialogue(lines: &[Line], put: &mut dyn FnMut(&str, &str)) {
    let mut originals = Vec::new();
    let mut translated = Vec::new();
    for line in lines {
        if let Some(comment) = line.text.strip_prefix('#') {
            if let Some(s) = say_text(&tokenize(comment.trim())) {
                originals.push(s);
            }
        } else if let Some(s) = say_text(&tokenize(&line.text)) {
            translated.push(s);
        }
    }
    for (o, n) in originals.iter().zip(&translated) {
        if !n.is_empty() {
            put(o, n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs() {
        let a = StringId::new();
        let b = StringId::new();
        let mut index = HashMap::new();
        index.insert("Hello!".to_owned(), vec![a]);
        index.insert("Yes".to_owned(), vec![b]);
        let src = "# game/script.rpy:3\ntranslate french start_1234abcd:\n\n    # e \"Hello!\"\n    e \"Bonjour !\"\n\ntranslate french strings:\n\n    # game/script.rpy:9\n    old \"Yes\"\n    new \"Oui\"\n    old \"Gone\"\n    new \"Parti\"\n";
        let mut wrappers = HashMap::new();
        wrappers.insert(b, ("\"".to_owned(), "\"".to_owned()));
        let r = translate("french", &[src.to_owned()], &index, &wrappers);
        assert_eq!(r.translation.lang, "fr");
        assert_eq!(
            r.translation.entries.get(&a).map(String::as_str),
            Some("Bonjour !")
        );
        assert_eq!(
            r.translation.entries.get(&b).map(String::as_str),
            Some("\"Oui\"")
        );
        assert_eq!((r.matched, r.unmatched), (2, 1));
    }
}
