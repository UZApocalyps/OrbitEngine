//! OrbitEngine localization.
//!
//! The story holds the texts of its *source language* in
//! [`orbit_core::StringTable`], keyed by stable [`StringId`]s. Every other
//! language is a [`Translation`]: a RON file mapping the same ids to texts:
//!
//! ```ron
//! (
//!     lang: "fr",
//!     name: "Francais",
//!     entries: {
//!         "01JBX0T0000000000000000004": "Bonjour !",
//!     },
//! )
//! ```
//!
//! A [`Localizer`] resolves a string for the selected language and falls back
//! to the source language when a translation is missing or empty, so a
//! half-translated game still plays. [`coverage`] reports missing and
//! orphaned strings. The crate has no I/O: the host loads the RON files
//! (`Translation::from_ron`) and hands them over, so it works unchanged on
//! the web.
//!
//! The host drives it: `localizer.set_language("fr")`, then shows
//! `localizer.text(string_id)` instead of `Vm::text(string_id)`.

use std::collections::BTreeMap;
use std::fmt;

use orbit_core::{Story, StringId, StringTable};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum I18nError {
    #[error("cannot parse translation: {0}")]
    Parse(String),
    #[error("translation has an empty language tag")]
    EmptyLanguage,
    #[error("`{0}` is the source language of the story, it cannot be a translation")]
    SourceLanguage(String),
}

/// One language file: text per string id.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Translation {
    /// BCP 47 tag, e.g. `"fr"` or `"pt-BR"`.
    pub lang: String,
    /// Name shown in the language menu, in that language ("Francais").
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub entries: BTreeMap<StringId, String>,
}

impl Translation {
    pub fn new(lang: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            lang: lang.into(),
            name: name.into(),
            entries: BTreeMap::new(),
        }
    }

    pub fn from_ron(source: &str) -> Result<Self, I18nError> {
        ron::from_str(source).map_err(|e| I18nError::Parse(e.to_string()))
    }

    pub fn to_ron(&self) -> Result<String, I18nError> {
        let cfg = ron::ser::PrettyConfig::new().indentor("    ".to_owned());
        ron::ser::to_string_pretty(self, cfg).map_err(|e| I18nError::Parse(e.to_string()))
    }

    /// A template for translators: every source string, with an empty text.
    pub fn skeleton(lang: impl Into<String>, name: impl Into<String>, story: &Story) -> Self {
        let mut t = Self::new(lang, name);
        t.entries = story
            .strings
            .entries
            .keys()
            .map(|id| (*id, String::new()))
            .collect();
        t
    }
}

/// A language the player can pick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageInfo {
    pub code: String,
    pub name: String,
    pub is_source: bool,
}

/// Resolves strings for the selected language, with fallback to the source.
#[derive(Clone, Debug)]
pub struct Localizer {
    source_lang: String,
    source: BTreeMap<StringId, String>,
    /// Keyed by lowercase language tag.
    translations: BTreeMap<String, Translation>,
    /// Lowercase key into `translations`; `None` = the source language.
    current: Option<String>,
}

fn key(lang: &str) -> String {
    lang.trim().replace('_', "-").to_ascii_lowercase()
}

impl Localizer {
    /// A localizer for a story's source table, initially in the source
    /// language.
    pub fn new(source: &StringTable) -> Self {
        Self {
            source_lang: source.source_lang.clone(),
            source: source.entries.clone(),
            translations: BTreeMap::new(),
            current: None,
        }
    }

    pub fn from_story(story: &Story) -> Self {
        Self::new(&story.strings)
    }

    /// Adds (or replaces) a language. The selected language is unchanged.
    pub fn add_translation(&mut self, translation: Translation) -> Result<(), I18nError> {
        let k = key(&translation.lang);
        if k.is_empty() {
            return Err(I18nError::EmptyLanguage);
        }
        if k == key(&self.source_lang) {
            return Err(I18nError::SourceLanguage(translation.lang));
        }
        self.translations.insert(k, translation);
        Ok(())
    }

    /// Parses a RON language file and adds it.
    pub fn add_translation_ron(&mut self, source: &str) -> Result<(), I18nError> {
        self.add_translation(Translation::from_ron(source)?)
    }

    /// The source language first, then the translations sorted by tag.
    pub fn languages(&self) -> Vec<LanguageInfo> {
        let mut out = vec![LanguageInfo {
            code: self.source_lang.clone(),
            name: self.source_lang.clone(),
            is_source: true,
        }];
        out.extend(self.translations.values().map(|t| LanguageInfo {
            code: t.lang.clone(),
            name: if t.name.is_empty() {
                t.lang.clone()
            } else {
                t.name.clone()
            },
            is_source: false,
        }));
        out
    }

    /// Finds the available language best matching `requested` (case and
    /// `_`/`-` insensitive): an exact match, else the same primary language
    /// (`fr-CA` -> `fr`, or `pt` -> `pt-BR`). Returns its tag.
    pub fn resolve_language(&self, requested: &str) -> Option<String> {
        let wanted = key(requested);
        if wanted.is_empty() {
            return None;
        }
        let candidates = std::iter::once((key(&self.source_lang), self.source_lang.clone()))
            .chain(
                self.translations
                    .iter()
                    .map(|(k, t)| (k.clone(), t.lang.clone())),
            )
            .collect::<Vec<_>>();
        let primary = |k: &str| k.split('-').next().unwrap_or_default().to_owned();
        candidates
            .iter()
            .find(|(k, _)| *k == wanted)
            .or_else(|| candidates.iter().find(|(k, _)| *k == primary(&wanted)))
            .or_else(|| {
                candidates
                    .iter()
                    .find(|(k, _)| primary(k) == primary(&wanted))
            })
            .map(|(_, tag)| tag.clone())
    }

    /// Selects a language (see [`Localizer::resolve_language`] for matching).
    /// Returns `false`, keeping the current language, if none matches.
    pub fn set_language(&mut self, requested: &str) -> bool {
        let Some(tag) = self.resolve_language(requested) else {
            return false;
        };
        let k = key(&tag);
        self.current = (k != key(&self.source_lang)).then_some(k);
        true
    }

    /// Tag of the selected language.
    pub fn language(&self) -> &str {
        self.current
            .as_ref()
            .and_then(|k| self.translations.get(k))
            .map_or(self.source_lang.as_str(), |t| t.lang.as_str())
    }

    pub fn source_language(&self) -> &str {
        &self.source_lang
    }

    /// The text in the selected language, else in the source language, else
    /// `None` (unknown id).
    pub fn try_text(&self, id: StringId) -> Option<&str> {
        self.current
            .as_ref()
            .and_then(|k| self.translations.get(k))
            .and_then(|t| t.entries.get(&id))
            .map(String::as_str)
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.source.get(&id).map(String::as_str))
    }

    /// Like [`Localizer::try_text`], with `""` for an unknown id.
    pub fn text(&self, id: StringId) -> &str {
        self.try_text(id).unwrap_or("")
    }

    /// `true` if `id` has a translation in the selected language (always
    /// `true` for the source language).
    pub fn is_translated(&self, id: StringId) -> bool {
        match self.current.as_ref().and_then(|k| self.translations.get(k)) {
            None => self.source.contains_key(&id),
            Some(t) => t.entries.get(&id).is_some_and(|s| !s.trim().is_empty()),
        }
    }

    /// Coverage of the translation `lang` (as added, any case).
    pub fn coverage(&self, lang: &str) -> Option<CoverageReport> {
        let t = self.translations.get(&key(lang))?;
        Some(coverage_of(&self.source, t))
    }
}

/// Missing and orphaned strings of a translation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageReport {
    pub lang: String,
    /// Source strings without a (non-empty) translation: they fall back to
    /// the source language.
    pub missing: Vec<StringId>,
    /// Translated ids that no longer exist in the story (stale entries).
    pub orphans: Vec<StringId>,
    pub translated: usize,
    pub total: usize,
}

impl CoverageReport {
    /// No string is missing (orphans do not matter for playing).
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }

    /// Fraction of source strings translated, `0.0..=1.0` (`1.0` for an empty
    /// story).
    pub fn ratio(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.translated as f32 / self.total as f32
        }
    }
}

impl fmt::Display for CoverageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {}/{} translated ({:.0}%), {} missing, {} orphaned",
            self.lang,
            self.translated,
            self.total,
            self.ratio() * 100.0,
            self.missing.len(),
            self.orphans.len()
        )
    }
}

fn coverage_of(source: &BTreeMap<StringId, String>, t: &Translation) -> CoverageReport {
    let translated_ok = |id: &StringId| {
        t.entries
            .get(id)
            .is_some_and(|text| !text.trim().is_empty())
    };
    let missing: Vec<StringId> = source
        .keys()
        .filter(|id| !translated_ok(id))
        .copied()
        .collect();
    let orphans = t
        .entries
        .keys()
        .filter(|id| !source.contains_key(id))
        .copied()
        .collect();
    CoverageReport {
        lang: t.lang.clone(),
        translated: source.len() - missing.len(),
        total: source.len(),
        missing,
        orphans,
    }
}

/// Compares a translation with the strings the story actually uses.
pub fn coverage(story: &Story, translation: &Translation) -> CoverageReport {
    coverage_of(&story.strings.entries, translation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> (StringTable, [StringId; 3]) {
        let mut t = StringTable::default();
        let ids = [t.add("Hello"), t.add("Bye"), t.add("Thanks")];
        (t, ids)
    }

    fn french(ids: [StringId; 3]) -> Translation {
        let mut fr = Translation::new("fr", "Francais");
        fr.entries.insert(ids[0], "Bonjour".into());
        fr.entries.insert(ids[1], "  ".into()); // blank = untranslated
        fr.entries.insert(StringId::new(), "stale".into()); // orphan
        fr
    }

    #[test]
    fn source_language_by_default_and_fallback_per_string() {
        let (table, ids) = table();
        let mut l = Localizer::new(&table);
        l.add_translation(french(ids)).unwrap();
        assert_eq!(l.language(), "en");
        assert_eq!(l.text(ids[0]), "Hello");

        assert!(l.set_language("fr"));
        assert_eq!(l.language(), "fr");
        assert_eq!(l.text(ids[0]), "Bonjour");
        // Blank and absent entries fall back to the source language.
        assert_eq!(l.text(ids[1]), "Bye");
        assert_eq!(l.text(ids[2]), "Thanks");
        assert!(l.is_translated(ids[0]));
        assert!(!l.is_translated(ids[1]));
        // Unknown ids resolve to nothing, without panicking.
        assert_eq!(l.try_text(StringId::new()), None);
        assert_eq!(l.text(StringId::new()), "");

        assert!(l.set_language("en"));
        assert_eq!(l.text(ids[0]), "Hello");
    }

    #[test]
    fn language_matching_is_forgiving() {
        let (table, ids) = table();
        let mut l = Localizer::new(&table);
        l.add_translation(french(ids)).unwrap();
        let mut pt = Translation::new("pt-BR", "Portugues");
        pt.entries.insert(ids[0], "Ola".into());
        l.add_translation(pt).unwrap();

        assert_eq!(l.resolve_language("FR"), Some("fr".into()));
        assert_eq!(l.resolve_language("fr_CA"), Some("fr".into()));
        assert_eq!(l.resolve_language("pt"), Some("pt-BR".into()));
        assert_eq!(l.resolve_language("pt_br"), Some("pt-BR".into()));
        assert_eq!(l.resolve_language("en-GB"), Some("en".into()));
        assert_eq!(l.resolve_language("de"), None);
        assert_eq!(l.resolve_language(""), None);
        // A failed selection keeps the current language.
        assert!(l.set_language("fr"));
        assert!(!l.set_language("de"));
        assert_eq!(l.language(), "fr");

        let langs = l.languages();
        assert_eq!(langs.len(), 3);
        assert!(langs[0].is_source && langs[0].code == "en");
        assert_eq!(langs[1].code, "fr");
        assert_eq!(langs[2].name, "Portugues");
    }

    #[test]
    fn coverage_reports_missing_and_orphans() {
        let (table, ids) = table();
        let mut story = Story::new("t");
        story.strings = table;
        let fr = french(ids);
        let report = coverage(&story, &fr);
        let mut expected = vec![ids[1], ids[2]];
        expected.sort();
        assert_eq!(report.missing, expected);
        assert_eq!(report.orphans.len(), 1);
        assert_eq!((report.translated, report.total), (1, 3));
        assert!(!report.is_complete());
        assert!((report.ratio() - 1.0 / 3.0).abs() < 1e-6);
        assert!(report.to_string().contains("1/3"));

        let mut l = Localizer::from_story(&story);
        l.add_translation(fr).unwrap();
        assert_eq!(l.coverage("FR").unwrap(), report);
        assert!(l.coverage("de").is_none());

        let skeleton = Translation::skeleton("de", "Deutsch", &story);
        let r = coverage(&story, &skeleton);
        assert_eq!(r.missing.len(), 3, "empty skeleton translates nothing");
        assert!(r.orphans.is_empty());
    }

    #[test]
    fn ron_roundtrip_and_errors() {
        let (_, ids) = table();
        let fr = french(ids);
        let back = Translation::from_ron(&fr.to_ron().unwrap()).unwrap();
        assert_eq!(fr, back);
        assert!(matches!(
            Translation::from_ron("((("),
            Err(I18nError::Parse(_))
        ));
        let (table, _) = table();
        let mut l = Localizer::new(&table);
        assert_eq!(
            l.add_translation(Translation::new(" ", "x")),
            Err(I18nError::EmptyLanguage)
        );
        assert!(matches!(
            l.add_translation(Translation::new("EN", "English")),
            Err(I18nError::SourceLanguage(_))
        ));
    }
}
