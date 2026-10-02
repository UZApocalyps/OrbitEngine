//! Editor internationalization: stable text keys, one table per language.
//!
//! Every text the editor shows is a **key** (`menu.file`, `scene.add_line`)
//! resolved at display time against the current language, so switching the
//! language is instant (egui redraws everything every frame).
//!
//! * Tables are RON maps (`{ "key": "text" }`) embedded with `include_str!`.
//!   RON is already a dependency, a table can be edited by a translator
//!   without touching Rust, and a typo is a parse error caught by the tests.
//! * Lookup falls back from the current language to English, then to the key
//!   itself (so a plugin may pass a plain literal where a key is expected:
//!   it is displayed as is).
//! * Parameters are `{name}` placeholders ([`Translator::format`]).
//! * Plurals use two keys, `key.one` and `key.other`, picked by
//!   [`Translator::plural`] from a count (also available as `{count}`).
//! * A plugin registers its own tables with `EditorRegistry::texts`, keyed
//!   with its own prefix (`my_plugin.hello`).

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

/// A language of the editor interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    En,
    Fr,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Fr];

    /// Two-letter code (`en`, `fr`).
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }

    /// Name of the language in itself, for the language menu.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Fr => "Français",
        }
    }

    /// Parses a code or a BCP 47 tag (`fr`, `fr-FR`, `fr_CA.UTF-8`, `EN-us`).
    /// `None` for a language the editor does not have.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let primary = tag
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        match primary.as_str() {
            "en" => Some(Lang::En),
            "fr" => Some(Lang::Fr),
            _ => None,
        }
    }

    /// The language to start with from the system's tag: French for `fr*`,
    /// English for everything else (or when the system says nothing).
    pub fn from_system(tag: Option<&str>) -> Lang {
        match tag.and_then(Lang::from_tag) {
            Some(Lang::Fr) => Lang::Fr,
            _ => Lang::En,
        }
    }

    /// Whether a count takes the singular form (`key.one`): `1` in English,
    /// `0` and `1` in French.
    pub fn is_singular(self, n: usize) -> bool {
        match self {
            Lang::En => n == 1,
            Lang::Fr => n <= 1,
        }
    }
}

/// `key -> text`.
pub type Table = HashMap<String, String>;

/// Parses a RON table (`{ "key": "text", ... }`).
pub fn parse_table(ron_text: &str) -> Result<Table, String> {
    ron::from_str::<Table>(ron_text).map_err(|e| e.to_string())
}

/// The `{name}` placeholders of a text, sorted and deduplicated (the test
/// that compares languages uses it).
pub fn placeholders(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            found.push(name.to_owned());
        }
        rest = &after[end + 1..];
    }
    found.sort();
    found.dedup();
    found
}

#[derive(Default)]
struct Inner {
    lang: Lang,
    tables: HashMap<Lang, Table>,
    /// Bumped by every language change.
    generation: u64,
}

/// Resolves text keys. Clones share the same tables and language, so one
/// can be handed to a worker thread.
#[derive(Clone, Default)]
pub struct Translator(Arc<RwLock<Inner>>);

impl std::fmt::Debug for Translator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Translator({:?})", self.lang())
    }
}

impl Translator {
    pub fn new(lang: Lang) -> Self {
        let translator = Self::default();
        translator.set_lang(lang);
        translator
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.0.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.0.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn lang(&self) -> Lang {
        self.read().lang
    }

    /// Changes the language for every clone. Texts are looked up at display
    /// time, so the next frame is in the new language.
    pub fn set_lang(&self, lang: Lang) {
        let mut inner = self.write();
        if inner.lang != lang {
            inner.lang = lang;
            inner.generation += 1;
        }
    }

    /// Number of language changes so far (caches of translated text compare it).
    pub fn generation(&self) -> u64 {
        self.read().generation
    }

    /// Merges a table into a language (an existing key is replaced).
    pub fn add_table(&self, lang: Lang, table: Table) {
        self.write().tables.entry(lang).or_default().extend(table);
    }

    /// Parses and merges a RON table; returns the number of keys.
    pub fn add_ron(&self, lang: Lang, ron_text: &str) -> Result<usize, String> {
        let table = parse_table(ron_text)?;
        let n = table.len();
        self.add_table(lang, table);
        Ok(n)
    }

    /// Whether the key exists in the current language or in English.
    pub fn has(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    fn lookup(&self, key: &str) -> Option<String> {
        let inner = self.read();
        inner
            .tables
            .get(&inner.lang)
            .and_then(|t| t.get(key))
            .or_else(|| inner.tables.get(&Lang::En).and_then(|t| t.get(key)))
            .cloned()
    }

    /// The text of `key` (current language, then English, then the key).
    pub fn get(&self, key: &str) -> String {
        self.lookup(key).unwrap_or_else(|| key.to_owned())
    }

    /// Short names for [`Translator::get`], [`Translator::format`] and
    /// [`Translator::plural`], like `EditorContext::t`, `tf` and `tn`.
    pub fn t(&self, key: &str) -> String {
        self.get(key)
    }

    pub fn tf(&self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        self.format(key, args)
    }

    pub fn tn(&self, key: &str, n: usize, args: &[(&str, &dyn Display)]) -> String {
        self.plural(key, n, args)
    }

    /// The text of `key` with `{name}` placeholders replaced.
    pub fn format(&self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        interpolate(&self.get(key), args)
    }

    /// Singular or plural form: `key.one` or `key.other`, with `{count}` set
    /// to `n` on top of `args`.
    pub fn plural(&self, key: &str, n: usize, args: &[(&str, &dyn Display)]) -> String {
        let lang = self.lang();
        let full = format!(
            "{key}.{}",
            if lang.is_singular(n) { "one" } else { "other" }
        );
        let text = self.get(&full);
        let mut all: Vec<(&str, &dyn Display)> = vec![("count", &n)];
        all.extend_from_slice(args);
        interpolate(&text, &all)
    }
}

/// Replaces `{name}` with the matching argument; an unknown placeholder is
/// left as is.
pub fn interpolate(text: &str, args: &[(&str, &dyn Display)]) -> String {
    if args.is_empty() || !text.contains('{') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, value)) => out.push_str(&value.to_string()),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------ egui access

fn data_id() -> egui::Id {
    egui::Id::new("orbit_editor_translator")
}

impl Translator {
    /// Stores this translator in the egui context so that code that only has
    /// a `Ui` (field widgets, helpers) can translate with [`UiText`]. The
    /// host calls it once; clones share the language, so no refresh is needed.
    pub fn install(&self, ctx: &egui::Context) {
        let this = self.clone();
        ctx.data_mut(|d| d.insert_temp(data_id(), this));
    }

    /// The translator installed in `ctx`, if any.
    pub fn of(ctx: &egui::Context) -> Option<Translator> {
        ctx.data(|d| d.get_temp::<Translator>(data_id()))
    }
}

/// `ui.t("key")`, `ctx.tf("key", &[("name", &value)])`: translation for code
/// that has a `Ui` or a `Context` but no `EditorContext`. Without an
/// installed translator the key is returned.
pub trait UiText {
    fn translator(&self) -> Option<Translator>;

    /// The installed translator, or an empty one (keys come back as text).
    fn texts(&self) -> Translator {
        self.translator().unwrap_or_default()
    }

    fn t(&self, key: &str) -> String {
        match self.translator() {
            Some(t) => t.get(key),
            None => key.to_owned(),
        }
    }

    fn tf(&self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        match self.translator() {
            Some(t) => t.format(key, args),
            None => interpolate(key, args),
        }
    }

    fn tn(&self, key: &str, n: usize, args: &[(&str, &dyn Display)]) -> String {
        match self.translator() {
            Some(t) => t.plural(key, n, args),
            None => key.to_owned(),
        }
    }
}

impl UiText for egui::Context {
    fn translator(&self) -> Option<Translator> {
        Translator::of(self)
    }
}

impl UiText for egui::Ui {
    fn translator(&self) -> Option<Translator> {
        Translator::of(self.ctx())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translator() -> Translator {
        let t = Translator::new(Lang::Fr);
        t.add_ron(
            Lang::En,
            r#"{ "hello": "Hello {name}", "only_en": "English only",
                 "n.one": "{count} node", "n.other": "{count} nodes" }"#,
        )
        .unwrap();
        t.add_ron(
            Lang::Fr,
            r#"{ "hello": "Bonjour {name}",
                 "n.one": "{count} noeud", "n.other": "{count} noeuds" }"#,
        )
        .unwrap();
        t
    }

    #[test]
    fn falls_back_to_english_then_to_the_key() {
        let t = translator();
        assert_eq!(t.get("hello"), "Bonjour {name}");
        assert_eq!(t.get("only_en"), "English only");
        assert_eq!(t.get("Plain literal"), "Plain literal");
        assert!(!t.has("missing"));
    }

    #[test]
    fn interpolates_and_keeps_unknown_placeholders() {
        let t = translator();
        assert_eq!(t.format("hello", &[("name", &"Léa")]), "Bonjour Léa");
        assert_eq!(t.format("hello", &[("other", &1)]), "Bonjour {name}");
        assert_eq!(interpolate("a {x", &[("x", &1)]), "a {x");
        assert_eq!(interpolate("{x}{x}", &[("x", &7)]), "77");
    }

    #[test]
    fn plurals_follow_the_language() {
        let t = translator();
        assert_eq!(t.plural("n", 0, &[]), "0 noeud");
        assert_eq!(t.plural("n", 2, &[]), "2 noeuds");
        t.set_lang(Lang::En);
        assert_eq!(t.plural("n", 0, &[]), "0 nodes");
        assert_eq!(t.plural("n", 1, &[]), "1 node");
    }

    #[test]
    fn switching_is_shared_by_clones() {
        let t = translator();
        let clone = t.clone();
        let g = t.generation();
        t.set_lang(Lang::En);
        assert_eq!(clone.get("hello"), "Hello {name}");
        assert_eq!(clone.generation(), g + 1);
    }

    #[test]
    fn language_tags() {
        assert_eq!(Lang::from_tag("fr-FR"), Some(Lang::Fr));
        assert_eq!(Lang::from_tag("fr_CA.UTF-8"), Some(Lang::Fr));
        assert_eq!(Lang::from_tag("EN-us"), Some(Lang::En));
        assert_eq!(Lang::from_tag("de"), None);
        assert_eq!(Lang::from_system(Some("fr-BE")), Lang::Fr);
        assert_eq!(Lang::from_system(Some("ja-JP")), Lang::En);
        assert_eq!(Lang::from_system(None), Lang::En);
    }

    #[test]
    fn placeholder_extraction() {
        assert_eq!(placeholders("{b} and {a} and {b} {not valid}"), ["a", "b"]);
    }
}
