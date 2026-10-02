//! The editor's own text tables (English and French), embedded in the binary.
//!
//! The mechanism (keys, fallback, parameters, plurals) lives in
//! `orbit_editor_api::i18n`; this module only lists the table files of
//! `locales/<lang>/<area>.ron` and builds the [`Translator`] the editor
//! starts with. A table is a RON map `{ "key": "text" }`.
//!
//! Each area owns the key prefixes listed in [`AREAS`]; the tests check that
//! both languages have exactly the same keys and `{parameters}`, that a key
//! lives in the file of its prefix and that no key is defined twice.

use orbit_editor_api::{Lang, Translator};

/// One table file per area: `(name, key prefixes it owns, English, French)`.
pub const AREAS: &[(&str, &[&str], &str, &str)] = &[
    (
        "common",
        &[
            "common", "menu", "category", "shortcut", "undo", "reject", "error",
        ],
        include_str!("../locales/en/common.ron"),
        include_str!("../locales/fr/common.ron"),
    ),
    (
        "host",
        &[
            "host", "status", "dialog", "palette", "dock", "welcome", "action", "lang", "time",
            "project", "problem",
        ],
        include_str!("../locales/en/host.ron"),
        include_str!("../locales/fr/host.ron"),
    ),
    (
        "scene",
        &["scene", "sceneview", "reveal"],
        include_str!("../locales/en/scene.ron"),
        include_str!("../locales/fr/scene.ron"),
    ),
    (
        "inspector",
        &["inspector", "node", "field"],
        include_str!("../locales/en/inspector.ron"),
        include_str!("../locales/fr/inspector.ron"),
    ),
    (
        "graph",
        &["graph"],
        include_str!("../locales/en/graph.ron"),
        include_str!("../locales/fr/graph.ron"),
    ),
    (
        "library",
        &["library", "import", "asset", "thumb"],
        include_str!("../locales/en/library.ron"),
        include_str!("../locales/fr/library.ron"),
    ),
    (
        "export",
        &["export"],
        include_str!("../locales/en/export.ron"),
        include_str!("../locales/fr/export.ron"),
    ),
    (
        "assistant",
        &["assistant", "live"],
        include_str!("../locales/en/assistant.ron"),
        include_str!("../locales/fr/assistant.ron"),
    ),
    (
        "definitions",
        &["defs"],
        include_str!("../locales/en/definitions.ron"),
        include_str!("../locales/fr/definitions.ron"),
    ),
    (
        "preview",
        &["preview"],
        include_str!("../locales/en/preview.ron"),
        include_str!("../locales/fr/preview.ron"),
    ),
    (
        "panels",
        &[
            "console", "problems", "explorer", "search", "draft", "edit", "util", "watch", "misc",
        ],
        include_str!("../locales/en/panels.ron"),
        include_str!("../locales/fr/panels.ron"),
    ),
];

/// Loads every built-in table into `translator`. A table that does not parse
/// is reported and skipped (the tests make sure none does).
pub fn load_builtin(translator: &Translator) -> Vec<String> {
    let mut errors = Vec::new();
    for (name, _, en, fr) in AREAS {
        for (lang, text) in [(Lang::En, en), (Lang::Fr, fr)] {
            if let Err(e) = translator.add_ron(lang, text) {
                errors.push(format!("text table {name} ({}): {e}", lang.code()));
            }
        }
    }
    errors
}

/// A translator with every built-in table, in `lang`.
pub fn translator(lang: Lang) -> Translator {
    let translator = Translator::new(lang);
    for error in load_builtin(&translator) {
        log::error!("{error}");
    }
    translator
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use orbit_editor_api::i18n::{parse_table, placeholders};

    use super::*;

    fn tables(text: &str) -> BTreeMap<String, String> {
        parse_table(text).unwrap().into_iter().collect()
    }

    #[test]
    fn both_languages_have_the_same_keys_and_parameters() {
        let mut problems = Vec::new();
        for (name, _, en, fr) in AREAS {
            let (en, fr) = (tables(en), tables(fr));
            for key in en.keys().filter(|k| !fr.contains_key(*k)) {
                problems.push(format!("{name}: `{key}` is missing in French"));
            }
            for key in fr.keys().filter(|k| !en.contains_key(*k)) {
                problems.push(format!("{name}: `{key}` is missing in English"));
            }
            for (key, en_text) in &en {
                if let Some(fr_text) = fr.get(key)
                    && placeholders(en_text) != placeholders(fr_text)
                {
                    problems.push(format!(
                        "{name}: `{key}` has different parameters ({:?} vs {:?})",
                        placeholders(en_text),
                        placeholders(fr_text)
                    ));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn keys_live_in_their_area_and_are_defined_once() {
        let mut seen: BTreeMap<String, &str> = BTreeMap::new();
        let mut problems = Vec::new();
        for (name, prefixes, en, _) in AREAS {
            for key in tables(en).keys() {
                let prefix = key.split('.').next().unwrap_or_default();
                if !prefixes.contains(&prefix) {
                    problems.push(format!(
                        "{name}: `{key}` has a prefix the area does not own"
                    ));
                }
                if let Some(other) = seen.insert(key.clone(), name) {
                    problems.push(format!("`{key}` is defined in {other} and {name}"));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn plural_keys_come_in_pairs() {
        let mut problems = Vec::new();
        for (name, _, en, fr) in AREAS {
            for table in [tables(en), tables(fr)] {
                for key in table.keys() {
                    let pair = if let Some(base) = key.strip_suffix(".one") {
                        format!("{base}.other")
                    } else if let Some(base) = key.strip_suffix(".other") {
                        format!("{base}.one")
                    } else {
                        continue;
                    };
                    if !table.contains_key(&pair) {
                        problems.push(format!("{name}: `{key}` has no `{pair}`"));
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn texts_are_not_empty_and_english_is_not_french() {
        // A cheap guard against a French text left in the English table.
        let accents: &[char] = &[
            'à', 'â', 'ç', 'é', 'è', 'ê', 'ë', 'î', 'ï', 'ô', 'ù', 'û', 'œ', 'É', 'À',
        ];
        let mut problems = Vec::new();
        for (name, _, en, fr) in AREAS {
            for (key, text) in tables(en) {
                if text.trim().is_empty() {
                    problems.push(format!("{name}: `{key}` is empty in English"));
                }
                if text.contains(accents) {
                    problems.push(format!("{name}: `{key}` looks French in English: {text}"));
                }
            }
            for (key, text) in tables(fr) {
                if text.trim().is_empty() {
                    problems.push(format!("{name}: `{key}` is empty in French"));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// Every key written as a literal in a call of the translation API
    /// (`.t("key")`, `.tf("key", ..)`, `.tn("key", ..)`) must exist.
    #[test]
    fn literal_keys_used_in_the_sources_exist() {
        let all: BTreeSet<String> = AREAS
            .iter()
            .map(|(_, _, en, _)| *en)
            .chain([crate::builtin::sample::TEXTS_EN])
            .flat_map(|en| tables(en).into_keys())
            .collect();
        let mut missing = Vec::new();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect(&root, &mut files);
        for file in files {
            if file.ends_with("i18n.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            for call in [".t(\"", ".tf(\"", ".tn(\"", "texts.get(\""] {
                for (at, _) in text.match_indices(call) {
                    let rest = &text[at + call.len()..];
                    let Some(end) = rest.find('"') else { continue };
                    let key = &rest[..end];
                    let exists = all.contains(key)
                        || all.contains(&format!("{key}.one"))
                        || key.contains('{');
                    if !exists {
                        missing.push(format!("{}: `{key}`", file.display()));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "unknown keys:\n{}", missing.join("\n"));
    }

    fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn the_sample_plugin_tables_match_too() {
        let (en, fr) = (
            tables(crate::builtin::sample::TEXTS_EN),
            tables(crate::builtin::sample::TEXTS_FR),
        );
        assert_eq!(
            en.keys().collect::<Vec<_>>(),
            fr.keys().collect::<Vec<_>>(),
            "the sample plugin has different keys in English and French"
        );
        for (key, text) in &en {
            assert_eq!(placeholders(text), placeholders(&fr[key]), "{key}");
            assert!(key.starts_with("orbit.sample."), "{key}");
        }
    }

    #[test]
    fn builtin_translator_resolves_in_both_languages() {
        let t = translator(Lang::En);
        assert!(load_builtin(&Translator::default()).is_empty());
        assert_eq!(t.lang(), Lang::En);
        t.set_lang(Lang::Fr);
        assert_eq!(t.lang(), Lang::Fr);
    }
}
