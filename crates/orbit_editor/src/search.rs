//! Global search: dialogue lines, choice options, characters, variables,
//! labels, scenes and assets of the open project.
//!
//! The index is a flat list of entries holding the text and its folded form
//! (lowercase, accents stripped), built from the story when a search needs
//! it and rebuilt only when the project revision changed. A query is a set
//! of words that must all appear in the folded text (so `cle fin` finds
//! « La clé du dernier fin… »); names (short texts) also match fuzzily.
//! 10 000 nodes are searched in a few milliseconds.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use orbit_core::{AssetKind, NodeKind, Story};
use orbit_editor_api::{Location, Translator};

use crate::fuzzy::{fold_str, fuzzy_match};

/// What a hit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HitKind {
    Line,
    Choice,
    Character,
    Variable,
    Label,
    Scene,
    Asset,
}

impl HitKind {
    /// Group title, in the language of `texts`.
    pub fn title(self, texts: &Translator) -> String {
        texts.t(match self {
            HitKind::Line => "search.kind.line",
            HitKind::Choice => "search.kind.choice",
            HitKind::Character => "search.kind.character",
            HitKind::Variable => "search.kind.variable",
            HitKind::Label => "search.kind.label",
            HitKind::Scene => "search.kind.scene",
            HitKind::Asset => "search.kind.asset",
        })
    }

    /// Long texts are searched by words only; names also fuzzily.
    fn is_name(self) -> bool {
        !matches!(self, HitKind::Line | HitKind::Choice)
    }
}

struct Entry {
    kind: HitKind,
    text: String,
    folded: String,
    /// Scene, speaker...: shown next to the text.
    context: String,
    location: Location,
}

/// One search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub kind: HitKind,
    pub text: String,
    pub context: String,
    pub location: Location,
    pub score: i32,
    /// Char ranges of `text` that matched, to highlight.
    pub marks: Vec<(usize, usize)>,
}

#[derive(Default)]
pub struct SearchIndex {
    entries: Vec<Entry>,
    key: Option<(u64, u64)>,
    /// Duration of the last (re)build.
    pub last_build: Option<Duration>,
}

/// Results per group, at most.
pub const PER_GROUP: usize = 60;

impl SearchIndex {
    /// Brings the index up to date for `key` (project generation,
    /// revision). Returns `true` when it was rebuilt. The context texts are
    /// written in the language of `texts`: clear the index when it changes.
    pub fn refresh(&mut self, story: &Story, key: (u64, u64), texts: &Translator) -> bool {
        if self.key == Some(key) {
            return false;
        }
        let started = Instant::now();
        self.rebuild(story, texts);
        self.key = Some(key);
        self.last_build = Some(started.elapsed());
        true
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.key = None;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn rebuild(&mut self, story: &Story, texts: &Translator) {
        let mut entries = Vec::with_capacity(
            story.strings.entries.len() + story.assets.assets.len() + story.characters.len() + 16,
        );
        let mut add = |kind: HitKind, text: &str, context: String, location: Location| {
            if !text.is_empty() {
                entries.push(Entry {
                    kind,
                    text: text.to_owned(),
                    folded: fold_str(text),
                    context,
                    location,
                });
            }
        };
        let speaker_name = |id| {
            story
                .character(id)
                .and_then(|c| story.text(c.name_id))
                .unwrap_or("?")
        };
        for scene in &story.scenes {
            add(
                HitKind::Scene,
                &scene.name,
                texts.tn("search.ctx.nodes", scene.nodes.len(), &[]),
                Location::Scene(scene.id),
            );
            for node in &scene.nodes {
                match &node.kind {
                    NodeKind::Say { speaker, text_id } => {
                        let text = story.text(*text_id).unwrap_or_default();
                        let who = speaker.map_or_else(
                            || texts.t("search.narrator"),
                            |id| speaker_name(id).to_owned(),
                        );
                        add(
                            HitKind::Line,
                            text,
                            format!("{} · {who}", scene.name),
                            Location::Node(node.id),
                        );
                    }
                    NodeKind::Choice { options } => {
                        for option in options {
                            add(
                                HitKind::Choice,
                                story.text(option.text_id).unwrap_or_default(),
                                format!("{} · {}", scene.name, texts.t("search.ctx.choice")),
                                Location::Node(node.id),
                            );
                        }
                    }
                    NodeKind::Label { name } => add(
                        HitKind::Label,
                        name,
                        scene.name.clone(),
                        Location::Node(node.id),
                    ),
                    _ => {}
                }
            }
        }
        for c in &story.characters {
            add(
                HitKind::Character,
                story.text(c.name_id).unwrap_or_default(),
                texts.tn("search.ctx.expressions", c.expressions.len(), &[]),
                Location::Character(c.id),
            );
        }
        for v in &story.variables {
            add(
                HitKind::Variable,
                &v.name,
                texts.t("search.ctx.variable"),
                Location::Variable(v.name.clone()),
            );
        }
        for a in &story.assets.assets {
            add(
                HitKind::Asset,
                &a.path,
                texts.t(match a.kind {
                    AssetKind::Image => "search.asset.image",
                    AssetKind::Music => "search.asset.music",
                    AssetKind::Sound => "search.asset.sound",
                    AssetKind::Scene3d => "search.asset.scene3d",
                }),
                Location::Asset(a.id),
            );
        }
        self.entries = entries;
    }

    /// Results for `query`, grouped by kind (in the order of [`HitKind`]),
    /// best first inside a group, at most [`PER_GROUP`] per group.
    pub fn query(&self, query: &str) -> Vec<Hit> {
        let words: Vec<String> = query.split_whitespace().map(fold_str).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(HitKind, i32, usize)> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(score) = score_entry(e, &words, query) {
                scored.push((e.kind, score, i));
            }
        }
        // Group, best score first, reading order for ties.
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        let mut out = Vec::new();
        let mut in_group = 0;
        let mut group = None;
        for (kind, score, i) in scored {
            if group != Some(kind) {
                group = Some(kind);
                in_group = 0;
            }
            if in_group >= PER_GROUP {
                continue;
            }
            in_group += 1;
            let e = &self.entries[i];
            out.push(Hit {
                kind,
                text: e.text.clone(),
                context: e.context.clone(),
                location: e.location.clone(),
                score,
                marks: marks(&e.folded, &words),
            });
        }
        out
    }
}

/// Score of an entry for the folded `words`; `None` when it does not match.
fn score_entry(e: &Entry, words: &[String], raw_query: &str) -> Option<i32> {
    let mut score = 0;
    let mut all = true;
    for w in words {
        match e.folded.find(w.as_str()) {
            Some(byte_pos) => {
                // Earlier, at a word start, and a bigger share of the text rank higher.
                let at_start = byte_pos == 0;
                let word_start = at_start
                    || e.folded[..byte_pos]
                        .chars()
                        .next_back()
                        .is_some_and(|c| !c.is_alphanumeric());
                score += 40
                    + if at_start { 30 } else { 0 }
                    + if word_start { 15 } else { 0 }
                    + (w.chars().count() as i32 * 100 / e.folded.chars().count().max(1) as i32)
                    - (byte_pos as i32).min(20);
            }
            None => {
                all = false;
                break;
            }
        }
    }
    if all {
        if e.folded == words.join(" ") {
            score += 100;
        }
        return Some(score);
    }
    if e.kind.is_name() {
        // Names: a fuzzy match still finds `affin` for « affinity » or `ca`
        // for « Clara »... only when the letters are in order.
        return fuzzy_match(raw_query, &e.text).map(|m| m.score.min(35));
    }
    None
}

/// Char ranges of `folded` where the words occur (merged), for highlighting.
fn marks(folded: &str, words: &[String]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for w in words {
        if let Some(byte_pos) = folded.find(w.as_str()) {
            let start = folded[..byte_pos].chars().count();
            ranges.push((start, start + w.chars().count()));
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in ranges {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// A search index shared between the search panel and the command palette
/// (both live in the one UI thread).
#[derive(Clone, Default)]
pub struct SearchHandle(pub Rc<RefCell<SearchIndex>>);

impl SearchHandle {
    pub fn refresh(&self, story: &Story, key: (u64, u64), texts: &Translator) {
        self.0.borrow_mut().refresh(story, key, texts);
    }

    pub fn query(&self, q: &str) -> Vec<Hit> {
        self.0.borrow().query(q)
    }
}

#[cfg(test)]
mod tests {
    use orbit_core::{Character, Color, Node, NodeId, Scene, SceneId, Value, VarDecl};

    use super::*;

    fn fr() -> Translator {
        crate::i18n::translator(orbit_editor_api::Lang::Fr)
    }

    fn story() -> Story {
        let mut story = Story::new("t");
        let name_id = story.strings.add("Clara");
        let clara = orbit_core::CharacterId::new();
        story.characters.push(Character {
            default_scale: 1.0,
            id: clara,
            name_id,
            color: Color::WHITE,
            expressions: vec![],
            default_position: None,
        });
        story
            .variables
            .push(VarDecl::game("affinity", Value::Int(0)));
        let mut nodes = Vec::new();
        for (who, text) in [
            (Some(clara), "J'ai trouvé la clé du grenier."),
            (None, "Il pleut sur la ville depuis trois jours."),
            (Some(clara), "Écoute… la clé tourne dans la serrure !"),
        ] {
            let text_id = story.strings.add(text);
            nodes.push(Node {
                id: NodeId::new(),
                kind: NodeKind::Say {
                    speaker: who,
                    text_id,
                },
            });
        }
        nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Label {
                name: "grenier".into(),
            },
        });
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "Prologue".into(),
            nodes,
        });
        story
    }

    fn index() -> SearchIndex {
        let mut idx = SearchIndex::default();
        idx.refresh(&story(), (1, 1), &fr());
        idx
    }

    #[test]
    fn finds_lines_ignoring_case_and_accents() {
        let idx = index();
        let hits = idx.query("ECOUTE");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, HitKind::Line);
        assert!(hits[0].context.contains("Clara"));
        assert!(matches!(hits[0].location, Location::Node(_)));
        // The marks cover the word in the original text (char indices).
        assert_eq!(hits[0].marks, vec![(0, 6)]);
    }

    #[test]
    fn every_word_must_match_and_results_are_grouped() {
        let idx = index();
        let hits = idx.query("cle grenier");
        // Only the first line holds both words; the label « grenier » has no `cle`.
        assert_eq!(hits.len(), 1);
        let hits = idx.query("grenier");
        let kinds: Vec<HitKind> = hits.iter().map(|h| h.kind).collect();
        assert_eq!(kinds, [HitKind::Line, HitKind::Label]);
    }

    #[test]
    fn names_of_all_kinds_are_found() {
        let idx = index();
        assert_eq!(idx.query("clara")[0].kind, HitKind::Character);
        assert_eq!(idx.query("affin")[0].kind, HitKind::Variable);
        assert_eq!(idx.query("prolog")[0].kind, HitKind::Scene);
        // Fuzzy match on a name only.
        assert_eq!(idx.query("afty")[0].kind, HitKind::Variable);
        assert!(idx.query("zzzz").is_empty());
        assert!(idx.query("   ").is_empty());
    }

    #[test]
    fn exact_and_prefix_matches_rank_first() {
        let mut story = story();
        let a = story.strings.add("clef");
        let b = story.strings.add("la clef de voûte");
        story.scenes[0].nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id: b,
            },
        });
        story.scenes[0].nodes.push(Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id: a,
            },
        });
        let mut idx = SearchIndex::default();
        idx.refresh(&story, (1, 1), &fr());
        let hits = idx.query("clef");
        assert_eq!(hits[0].text, "clef", "the exact text comes first");
        assert_eq!(hits[1].text, "la clef de voûte");
    }

    #[test]
    fn the_index_is_rebuilt_only_when_the_revision_changes() {
        let story = story();
        let mut idx = SearchIndex::default();
        assert!(idx.refresh(&story, (1, 1), &fr()));
        assert!(!idx.refresh(&story, (1, 1), &fr()));
        assert!(idx.refresh(&story, (1, 2), &fr()));
        idx.clear();
        assert!(idx.is_empty());
    }

    #[test]
    fn ten_thousand_nodes_are_searched_in_well_under_100_ms() {
        let story = crate::bench::generate_story(10_000);
        let mut idx = SearchIndex::default();
        let started = Instant::now();
        idx.refresh(&story, (1, 1), &fr());
        let build = started.elapsed();
        assert!(idx.len() > 5_000, "{}", idx.len());
        let mut worst = Duration::ZERO;
        for q in [
            "réplique 5",
            "bloc 400",
            "option a",
            "affinity",
            "texte ligne",
            "zzz",
        ] {
            let started = Instant::now();
            let hits = idx.query(q);
            worst = worst.max(started.elapsed());
            if q != "zzz" {
                assert!(!hits.is_empty(), "{q}");
            }
        }
        eprintln!("10k nodes: index {build:?}, slowest query {worst:?}");
        assert!(worst < Duration::from_millis(100), "{worst:?}");
    }
}
