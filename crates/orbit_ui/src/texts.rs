//! Interface strings: a table of keys (`ui.menu.new_game`) behind an
//! injectable provider, with built-in English and French tables.
//!
//! Screens never hold literal text: they ask a [`Texts`] for a key. The
//! integration can plug a localization crate in by implementing
//! [`TextProvider`]; keys it does not know fall back to the built-in English
//! table, then to the key itself (so a missing translation is visible rather
//! than blank).

use std::collections::HashMap;
use std::sync::Arc;

/// A source of interface strings.
pub trait TextProvider: Send + Sync {
    /// The text for `key`, or `None` when this provider does not know it.
    fn get(&self, key: &str) -> Option<String>;
}

/// A plain key -> text map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextTable {
    map: HashMap<String, String>,
}

impl TextTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self {
            map: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    pub fn insert(&mut self, key: impl Into<String>, text: impl Into<String>) {
        self.map.insert(key.into(), text.into());
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// The built-in table of a language code (`"fr"`, `"fr-CA"`...). Unknown
    /// languages give the English table.
    pub fn builtin(language: &str) -> Self {
        let lang = language.split(['-', '_']).next().unwrap_or("en");
        match lang.to_ascii_lowercase().as_str() {
            "fr" => Self::from_pairs(FR),
            _ => Self::from_pairs(EN),
        }
    }
}

impl TextProvider for TextTable {
    fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }
}

/// Cheap-to-clone handle used by the screens.
#[derive(Clone)]
pub struct Texts {
    provider: Arc<dyn TextProvider>,
    fallback: Arc<TextTable>,
}

impl std::fmt::Debug for Texts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Texts")
    }
}

impl Texts {
    /// Wraps a provider (built-in English is the fallback).
    pub fn new(provider: Arc<dyn TextProvider>) -> Self {
        Self {
            provider,
            fallback: Arc::new(TextTable::builtin("en")),
        }
    }

    /// The built-in table of `language` (`"en"` or `"fr"`).
    pub fn builtin(language: &str) -> Self {
        Self::new(Arc::new(TextTable::builtin(language)))
    }

    /// The text of `key`: provider, then built-in English, then the key.
    pub fn get(&self, key: &str) -> String {
        self.provider
            .get(key)
            .or_else(|| self.fallback.get(key))
            .unwrap_or_else(|| key.to_owned())
    }

    /// Like [`Texts::get`], replacing each `{name}` with its value.
    pub fn format(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.get(key);
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }
}

impl Default for Texts {
    fn default() -> Self {
        Self::builtin("en")
    }
}

/// Built-in English strings.
pub const EN: &[(&str, &str)] = &[
    ("ui.menu.new_game", "New Game"),
    ("ui.menu.continue", "Continue"),
    ("ui.menu.load", "Load"),
    ("ui.menu.save", "Save"),
    ("ui.menu.preferences", "Preferences"),
    ("ui.menu.history", "History"),
    ("ui.menu.quit", "Quit"),
    ("ui.menu.resume", "Resume"),
    ("ui.menu.main_menu", "Main Menu"),
    ("ui.menu.back", "Back"),
    ("ui.pause.title", "Paused"),
    ("ui.save.title", "Save"),
    ("ui.load.title", "Load"),
    ("ui.menu.gallery", "Gallery"),
    ("ui.menu.replay", "Replay"),
    ("ui.gallery.title", "Gallery"),
    ("ui.gallery.locked", "Locked"),
    ("ui.gallery.empty", "Nothing to see yet."),
    ("ui.replay.title", "Scene Replay"),
    ("ui.replay.empty", "No scene seen yet."),
    ("ui.menu.achievements", "Achievements"),
    ("ui.achievements.title", "Achievements"),
    ("ui.achievements.empty", "No achievements."),
    ("ui.achievements.locked", "Locked"),
    ("ui.achievements.hidden_title", "Hidden achievement"),
    ("ui.achievements.hidden_description", "Keep playing to discover it."),
    ("ui.achievements.toast", "Achievement unlocked"),
    ("ui.save.tab.manual", "Slots"),
    ("ui.save.tab.auto", "Autosaves"),
    ("ui.save.tab.quick", "Quicksave"),
    ("ui.save.slot", "Slot {n}"),
    ("ui.save.auto_slot", "Autosave {n}"),
    ("ui.save.quick_slot", "Quicksave"),
    ("ui.save.empty", "Empty"),
    ("ui.save.incompatible", "Incompatible save"),
    ("ui.save.delete", "Delete"),
    ("ui.save.page", "Page {page} / {pages}"),
    ("ui.save.prev_page", "Previous"),
    ("ui.save.next_page", "Next"),
    ("ui.save.confirm_overwrite", "Overwrite this save?"),
    ("ui.save.confirm_delete", "Delete this save?"),
    ("ui.confirm.yes", "Yes"),
    ("ui.confirm.no", "No"),
    ("ui.prefs.title", "Preferences"),
    ("ui.prefs.volume_master", "Master volume"),
    ("ui.prefs.volume_music", "Music"),
    ("ui.prefs.volume_sfx", "Sound effects"),
    ("ui.prefs.volume_voice", "Voices"),
    ("ui.prefs.text_speed", "Text speed"),
    ("ui.prefs.auto_delay", "Auto-mode delay"),
    ("ui.prefs.skip_mode", "Skip"),
    ("ui.prefs.skip_read", "Read text only"),
    ("ui.prefs.skip_all", "All text"),
    ("ui.prefs.fullscreen", "Fullscreen"),
    ("ui.prefs.screen_reader", "Screen reader"),
    ("ui.prefs.text_scale", "Text size"),
    ("ui.prefs.high_contrast", "High contrast"),
    ("ui.prefs.reduced_motion", "Reduce motion"),
    ("ui.a11y.choices", "Choices"),
    ("ui.prefs.language", "Language"),
    ("ui.prefs.defaults", "Reset to defaults"),
    ("ui.prefs.chars_per_second", "{v} chars/s"),
    ("ui.prefs.seconds", "{v} s"),
    ("ui.history.title", "History"),
    ("ui.history.rewind", "Rewind here"),
    ("ui.history.voice", "Voice"),
    ("ui.history.empty", "Nothing to show yet."),
    ("ui.quick.back", "Back"),
    ("ui.quick.history", "History"),
    ("ui.quick.skip", "Skip"),
    ("ui.quick.auto", "Auto"),
    ("ui.quick.qsave", "Q.Save"),
    ("ui.quick.qload", "Q.Load"),
    ("ui.quick.prefs", "Prefs"),
    ("ui.quick.menu", "Menu"),
    ("ui.quick.voice", "Voice"),
    ("ui.toast.saved", "Saved"),
    ("ui.toast.loaded", "Loaded"),
    ("ui.toast.deleted", "Save deleted"),
    ("ui.toast.incompatible", "Incompatible save"),
    ("ui.toast.quick_saved", "Quicksaved"),
    ("ui.toast.save_failed", "Could not save"),
    ("ui.toast.load_failed", "Could not load this save"),
    ("ui.toast.corrupt", "This save is damaged"),
    ("ui.toast.nothing_to_load", "No quicksave yet"),
    (
        "ui.toast.storage_unavailable",
        "Saving is unavailable: progress will not be kept",
    ),
    ("ui.loading", "Loading\u{2026}"),
    ("ui.click_to_start", "Click to start"),
    ("ui.the_end", "The end \u{2014} click to return to the menu"),
    ("ui.error.title", "The story cannot be started"),
    (
        "ui.error.hint",
        "Fix the file: it reloads automatically (native).",
    ),
];

/// Built-in French strings.
pub const FR: &[(&str, &str)] = &[
    ("ui.menu.new_game", "Nouvelle partie"),
    ("ui.menu.continue", "Continuer"),
    ("ui.menu.load", "Charger"),
    ("ui.menu.save", "Sauvegarder"),
    ("ui.menu.preferences", "Pr\u{e9}f\u{e9}rences"),
    ("ui.menu.history", "Historique"),
    ("ui.menu.quit", "Quitter"),
    ("ui.menu.resume", "Reprendre"),
    ("ui.menu.main_menu", "Menu principal"),
    ("ui.menu.back", "Retour"),
    ("ui.pause.title", "Pause"),
    ("ui.save.title", "Sauvegarder"),
    ("ui.load.title", "Charger"),
    ("ui.menu.gallery", "Galerie"),
    ("ui.menu.replay", "Rejouer"),
    ("ui.gallery.title", "Galerie"),
    ("ui.gallery.locked", "Verrouill\u{e9}e"),
    ("ui.gallery.empty", "Rien \u{e0} voir pour l'instant."),
    ("ui.replay.title", "Rejouer une sc\u{e8}ne"),
    ("ui.replay.empty", "Aucune sc\u{e8}ne vue pour l'instant."),
    ("ui.menu.achievements", "Succ\u{e8}s"),
    ("ui.achievements.title", "Succ\u{e8}s"),
    ("ui.achievements.empty", "Aucun succ\u{e8}s."),
    ("ui.achievements.locked", "Verrouill\u{e9}"),
    ("ui.achievements.hidden_title", "Succ\u{e8}s cach\u{e9}"),
    ("ui.achievements.hidden_description", "Continuez \u{e0} jouer pour le d\u{e9}couvrir."),
    ("ui.achievements.toast", "Succ\u{e8}s d\u{e9}bloqu\u{e9}"),
    ("ui.save.tab.manual", "Emplacements"),
    ("ui.save.tab.auto", "Auto-saves"),
    ("ui.save.tab.quick", "Sauvegarde rapide"),
    ("ui.save.slot", "Emplacement {n}"),
    ("ui.save.auto_slot", "Auto-save {n}"),
    ("ui.save.quick_slot", "Sauvegarde rapide"),
    ("ui.save.empty", "Vide"),
    ("ui.save.incompatible", "Sauvegarde incompatible"),
    ("ui.save.delete", "Supprimer"),
    ("ui.save.page", "Page {page} / {pages}"),
    ("ui.save.prev_page", "Pr\u{e9}c\u{e9}dent"),
    ("ui.save.next_page", "Suivant"),
    (
        "ui.save.confirm_overwrite",
        "\u{c9}craser cette sauvegarde ?",
    ),
    ("ui.save.confirm_delete", "Supprimer cette sauvegarde ?"),
    ("ui.confirm.yes", "Oui"),
    ("ui.confirm.no", "Non"),
    ("ui.prefs.title", "Pr\u{e9}f\u{e9}rences"),
    ("ui.prefs.volume_master", "Volume g\u{e9}n\u{e9}ral"),
    ("ui.prefs.volume_music", "Musique"),
    ("ui.prefs.volume_sfx", "Effets sonores"),
    ("ui.prefs.volume_voice", "Voix"),
    ("ui.prefs.text_speed", "Vitesse du texte"),
    ("ui.prefs.auto_delay", "D\u{e9}lai du mode auto"),
    ("ui.prefs.skip_mode", "Saut"),
    ("ui.prefs.skip_read", "Texte lu uniquement"),
    ("ui.prefs.skip_all", "Tout le texte"),
    ("ui.prefs.fullscreen", "Plein \u{e9}cran"),
    ("ui.prefs.screen_reader", "Lecteur d'\u{e9}cran"),
    ("ui.prefs.text_scale", "Taille du texte"),
    ("ui.prefs.high_contrast", "Contraste \u{e9}lev\u{e9}"),
    ("ui.prefs.reduced_motion", "R\u{e9}duire les animations"),
    ("ui.a11y.choices", "Choix"),
    ("ui.prefs.language", "Langue"),
    ("ui.prefs.defaults", "Valeurs par d\u{e9}faut"),
    ("ui.prefs.chars_per_second", "{v} car./s"),
    ("ui.prefs.seconds", "{v} s"),
    ("ui.history.title", "Historique"),
    ("ui.history.rewind", "Revenir ici"),
    ("ui.history.voice", "Voix"),
    ("ui.history.empty", "Rien \u{e0} afficher pour l'instant."),
    ("ui.quick.back", "Retour"),
    ("ui.quick.history", "Historique"),
    ("ui.quick.skip", "Saut"),
    ("ui.quick.auto", "Auto"),
    ("ui.quick.qsave", "Q.Save"),
    ("ui.quick.qload", "Q.Load"),
    ("ui.quick.prefs", "Pr\u{e9}f."),
    ("ui.quick.menu", "Menu"),
    ("ui.quick.voice", "Voix"),
    ("ui.toast.saved", "Sauvegard\u{e9}"),
    ("ui.toast.loaded", "Charg\u{e9}"),
    ("ui.toast.deleted", "Sauvegarde supprim\u{e9}e"),
    ("ui.toast.incompatible", "Sauvegarde incompatible"),
    ("ui.toast.quick_saved", "Sauvegarde rapide"),
    ("ui.toast.save_failed", "Sauvegarde impossible"),
    ("ui.toast.load_failed", "Chargement impossible"),
    ("ui.toast.corrupt", "Cette sauvegarde est endommag\u{e9}e"),
    ("ui.toast.nothing_to_load", "Aucune sauvegarde rapide"),
    (
        "ui.toast.storage_unavailable",
        "Sauvegarde indisponible : la progression sera perdue",
    ),
    ("ui.loading", "Chargement\u{2026}"),
    ("ui.click_to_start", "Cliquer pour d\u{e9}marrer"),
    ("ui.the_end", "Fin \u{2014} cliquer pour revenir au menu"),
    ("ui.error.title", "Impossible de lancer l'histoire"),
    (
        "ui.error.hint",
        "Corrigez le fichier : il sera recharg\u{e9} automatiquement (natif).",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_and_french_define_the_same_keys() {
        let en = TextTable::from_pairs(EN);
        let fr = TextTable::from_pairs(FR);
        assert_eq!(en.len(), EN.len(), "duplicate English key");
        assert_eq!(fr.len(), FR.len(), "duplicate French key");
        for (key, _) in EN {
            assert!(fr.contains(key), "missing French key {key}");
        }
        for (key, _) in FR {
            assert!(en.contains(key), "missing English key {key}");
        }
    }

    #[test]
    fn lookup_falls_back_to_english_then_to_the_key() {
        let mut partial = TextTable::new();
        partial.insert("ui.menu.quit", "Salir");
        let texts = Texts::new(Arc::new(partial));
        assert_eq!(texts.get("ui.menu.quit"), "Salir");
        assert_eq!(texts.get("ui.menu.load"), "Load");
        assert_eq!(texts.get("nope.nope"), "nope.nope");
    }

    #[test]
    fn builtin_picks_language_and_regional_variants() {
        assert_eq!(Texts::builtin("fr").get("ui.menu.quit"), "Quitter");
        assert_eq!(Texts::builtin("fr-CA").get("ui.menu.quit"), "Quitter");
        assert_eq!(Texts::builtin("xx").get("ui.menu.quit"), "Quit");
    }

    #[test]
    fn format_replaces_named_arguments() {
        let t = Texts::builtin("en");
        assert_eq!(
            t.format("ui.save.page", &[("page", "2"), ("pages", "5")]),
            "Page 2 / 5"
        );
    }
}
