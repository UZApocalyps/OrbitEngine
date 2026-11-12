//! The voice script, without any UI or disk access: the lines to record in
//! scene order, their expected voice file and status, the counters per
//! character and the two exports. Pure, so it is unit tested.
//!
//! **Convention** (see the manual, "Voice acting"): a voice file is named
//! after the line it voices, `<line-id>.ogg` with the id of the `Say` node;
//! the files of a translation are in a folder named after the language,
//! `assets/voice/fr/<line-id>.ogg`. The *expected file* of a line without a
//! voice is that path.

use std::collections::{HashMap, HashSet};

use orbit_editor_sdk::orbit_core::{CharacterId, NodeId, NodeKind, Story, StringId};
use orbit_editor_sdk::orbit_project::voices::plan_voice_import;
use orbit_i18n::Translation;

/// Where the voice files of a project go by default.
pub const VOICE_DIR: &str = "assets/voice";
/// Extension proposed in the expected file name.
pub const DEFAULT_EXTENSION: &str = "ogg";

/// Whether a line has its recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceState {
    /// A voice is assigned to the line and its file exists.
    Recorded,
    /// A file named after the line is in the project but not assigned to it
    /// yet (the voice import would assign it).
    PendingImport,
    /// No voice, or the assigned file is missing.
    Missing,
}

/// One line to record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub scene: String,
    pub node: NodeId,
    pub text_id: StringId,
    pub character: Option<CharacterId>,
    /// The character's name, or the narrator's label.
    pub speaker: String,
    pub text: String,
    /// The text is really in the chosen language (always true for the
    /// source language); otherwise `text` is the source text.
    pub translated: bool,
    pub words: usize,
    /// The voice file assigned to the line (in the chosen language).
    pub assigned: Option<String>,
    /// The file to record: the assigned one, the one found by name, or the
    /// path the naming convention expects.
    pub expected: String,
    pub state: VoiceState,
}

impl Line {
    /// The stable identifier of the line: the one voice files are named after.
    pub fn id(&self) -> String {
        self.node.to_string()
    }
}

/// What the disk holds, gathered in a background task.
#[derive(Clone, Debug, Default)]
pub struct VoiceDisk {
    /// Audio files of the asset folder (`/` separators), the input of the
    /// naming convention.
    pub files: Vec<String>,
    /// Lower-case paths of every file known to exist: `files` and the
    /// declared sounds that were found elsewhere.
    pub present: HashSet<String>,
}

impl VoiceDisk {
    pub fn exists(&self, path: &str) -> bool {
        self.present.contains(&norm_key(path))
    }
}

/// Lower case, `/` separators.
pub fn norm_key(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_lowercase()
}

/// Words of a text: runs of non-white-space characters.
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// The lines of the story, in scene order. `lang` is the translation to
/// script (`None`: the source language of the story). `translation` is that
/// language's file, if it exists.
pub fn build_lines(
    story: &Story,
    lang: Option<&str>,
    translation: Option<&Translation>,
    narrator: &str,
    disk: &VoiceDisk,
) -> Vec<Line> {
    // Files named after a line: node -> path, for the chosen language.
    let mut named: HashMap<NodeId, String> = HashMap::new();
    for m in plan_voice_import(story, &disk.files, None).matches {
        let same_language = match (&m.language, lang) {
            (None, None) => true,
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => false,
        };
        if same_language {
            named.insert(m.node, m.path);
        }
    }
    let name_of = |id: CharacterId| -> String {
        story
            .characters
            .iter()
            .find(|c| c.id == id)
            .map(|c| {
                translation
                    .and_then(|t| t.entries.get(&c.name_id))
                    .filter(|s| !s.trim().is_empty())
                    .map(String::as_str)
                    .or_else(|| story.strings.get(c.name_id))
                    .unwrap_or("?")
                    .to_owned()
            })
            .unwrap_or_else(|| "?".to_owned())
    };

    let mut out = Vec::new();
    for scene in &story.scenes {
        for node in &scene.nodes {
            let NodeKind::Say { speaker, text_id, voice } = &node.kind else { continue };
            let own = translation
                .and_then(|t| t.entries.get(text_id))
                .filter(|s| !s.trim().is_empty())
                .map(String::as_str);
            let (text, translated) = match (lang, own) {
                (None, _) => (story.strings.get(*text_id).unwrap_or("").to_owned(), true),
                (Some(_), Some(t)) => (t.to_owned(), true),
                (Some(_), None) => (story.strings.get(*text_id).unwrap_or("").to_owned(), false),
            };
            // The voice of the language: a translation replaces the node's.
            let voice_id = match lang {
                None => *voice,
                Some(_) => translation.and_then(|t| t.voices.get(text_id).copied()),
            };
            let assigned = voice_id.and_then(|v| story.asset_path(v)).map(str::to_owned);
            let found = named.get(&node.id).cloned();
            let (state, expected) = match (&assigned, &found) {
                (Some(path), _) if disk.exists(path) => (VoiceState::Recorded, path.clone()),
                (Some(path), _) => (VoiceState::Missing, path.clone()),
                (None, Some(path)) => (VoiceState::PendingImport, path.clone()),
                (None, None) => (VoiceState::Missing, default_expected(lang, node.id)),
            };
            out.push(Line {
                scene: scene.name.clone(),
                node: node.id,
                text_id: *text_id,
                character: *speaker,
                speaker: speaker.map_or_else(|| narrator.to_owned(), name_of),
                words: count_words(&text),
                text,
                translated,
                assigned,
                expected,
                state,
            });
        }
    }
    out
}

/// The path the naming convention gives to the voice of a line.
pub fn default_expected(lang: Option<&str>, node: NodeId) -> String {
    match lang {
        None => format!("{VOICE_DIR}/{node}.{DEFAULT_EXTENSION}"),
        Some(l) => format!("{VOICE_DIR}/{l}/{node}.{DEFAULT_EXTENSION}"),
    }
}

/// Counters of a group of lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub lines: usize,
    pub words: usize,
    pub recorded: usize,
    pub pending: usize,
    pub missing: usize,
}

impl Stats {
    pub fn of<'a>(lines: impl IntoIterator<Item = &'a Line>) -> Stats {
        let mut s = Stats::default();
        for l in lines {
            s.lines += 1;
            s.words += l.words;
            match l.state {
                VoiceState::Recorded => s.recorded += 1,
                VoiceState::PendingImport => s.pending += 1,
                VoiceState::Missing => s.missing += 1,
            }
        }
        s
    }
}

/// Who speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    Narrator,
    Character(CharacterId),
}

impl Line {
    pub fn speaker_key(&self) -> Speaker {
        self.character.map_or(Speaker::Narrator, Speaker::Character)
    }
}

/// A speaker with its counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeakerStats {
    pub who: Speaker,
    pub name: String,
    pub stats: Stats,
}

/// Counters per speaker, in order of first appearance in the story. Speakers
/// without a line are absent.
pub fn per_speaker(lines: &[Line]) -> Vec<SpeakerStats> {
    let mut order: Vec<Speaker> = Vec::new();
    let mut groups: HashMap<Speaker, Vec<&Line>> = HashMap::new();
    for l in lines {
        let key = l.speaker_key();
        if !groups.contains_key(&key) {
            order.push(key);
        }
        groups.entry(key).or_default().push(l);
    }
    order
        .into_iter()
        .map(|who| {
            let group = &groups[&who];
            SpeakerStats { who, name: group[0].speaker.clone(), stats: Stats::of(group.iter().copied()) }
        })
        .collect()
}

/// The lines to show or export: of one speaker (`None`: everybody) and, if
/// asked, only those still to record (missing or waiting for the import).
pub fn select(lines: &[Line], who: Option<Speaker>, only_todo: bool) -> Vec<&Line> {
    lines
        .iter()
        .filter(|l| who.is_none_or(|w| l.speaker_key() == w))
        .filter(|l| !only_todo || l.state != VoiceState::Recorded)
        .collect()
}

/// One CSV field: quoted when it holds the separator, a quote or a line break.
fn field(value: &str, sep: char) -> String {
    // A spreadsheet reads a cell starting with these as a formula: a leading
    // apostrophe makes it text (same rule as the story statistics export).
    let value = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{value}") } else { value.to_owned() };
    if value.contains([sep, '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

/// Number of columns of the CSV.
pub const CSV_COLUMNS: usize = 8;

/// Words for the states, in the export language.
#[derive(Clone, Debug)]
pub struct StateLabels {
    pub recorded: String,
    pub pending: String,
    pub missing: String,
}

impl StateLabels {
    fn of(&self, state: VoiceState) -> &str {
        match state {
            VoiceState::Recorded => &self.recorded,
            VoiceState::PendingImport => &self.pending,
            VoiceState::Missing => &self.missing,
        }
    }
}

/// The CSV of the script: UTF-8 with a byte order mark (Excel opens it as
/// UTF-8), `\r\n` line ends, `sep` between fields. Columns: number, scene,
/// line id, character, text, words, expected file, status. `header` has
/// [`CSV_COLUMNS`] titles.
pub fn to_csv(lines: &[&Line], header: &[String], states: &StateLabels, sep: char) -> String {
    let mut out = String::from('\u{feff}');
    let mut row = |cells: &[String]| {
        let cells: Vec<String> = cells.iter().map(|c| field(c, sep)).collect();
        out.push_str(&cells.join(&sep.to_string()));
        out.push_str("\r\n");
    };
    row(header);
    for (i, l) in lines.iter().enumerate() {
        row(&[
            (i + 1).to_string(),
            l.scene.clone(),
            l.id(),
            l.speaker.clone(),
            l.text.clone(),
            l.words.to_string(),
            l.expected.clone(),
            states.of(l.state).to_owned(),
        ]);
    }
    out
}

/// Words of the actors' text, in the export language. `{name}`, `{lines}`,
/// `{words}`, `{scene}` and `{id}` are replaced.
#[derive(Clone, Debug)]
pub struct TextLabels {
    /// Title of the document (already filled in).
    pub title: String,
    /// Heading of a character: `{name}`, `{lines}`, `{words}`.
    pub heading: String,
    /// Header of a line: `{n}`, `{scene}`, `{id}`.
    pub line: String,
    /// Said when a character has nothing to record.
    pub nothing: String,
}

fn fill(template: &str, pairs: &[(&str, String)]) -> String {
    pairs.iter().fold(template.to_owned(), |t, (k, v)| t.replace(&format!("{{{k}}}"), v))
}

/// The script for the actors, readable as plain text: grouped by character
/// (first appearance order), each line numbered with its scene and id, the
/// text indented. No file names or states: it is what is read in the booth.
pub fn to_actor_text(lines: &[&Line], labels: &TextLabels) -> String {
    let mut out = String::new();
    out.push_str(&labels.title);
    out.push('\n');
    let mut order: Vec<Speaker> = Vec::new();
    let mut groups: HashMap<Speaker, Vec<&Line>> = HashMap::new();
    for l in lines {
        let key = l.speaker_key();
        if !groups.contains_key(&key) {
            order.push(key);
        }
        groups.entry(key).or_default().push(l);
    }
    if order.is_empty() {
        out.push('\n');
        out.push_str(&labels.nothing);
        out.push('\n');
        return out;
    }
    for who in order {
        let group = &groups[&who];
        let stats = Stats::of(group.iter().copied());
        let heading = fill(
            &labels.heading,
            &[("name", group[0].speaker.clone()), ("lines", stats.lines.to_string()), ("words", stats.words.to_string())],
        );
        out.push_str("\n\n");
        out.push_str(&heading);
        out.push('\n');
        out.push_str(&"=".repeat(heading.chars().count().min(72)));
        out.push('\n');
        for (i, l) in group.iter().enumerate() {
            out.push('\n');
            out.push_str(&fill(
                &labels.line,
                &[("n", (i + 1).to_string()), ("scene", l.scene.clone()), ("id", l.id())],
            ));
            out.push('\n');
            for text_line in l.text.lines() {
                out.push_str("    ");
                out.push_str(text_line);
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cells_cannot_be_formulas() {
        assert_eq!(field("=1+1", ';'), "'=1+1");
        assert_eq!(field("@SUM(A1)", ';'), "'@SUM(A1)");
        assert_eq!(field("-2", ';'), "'-2");
        assert_eq!(field("+1", ';'), "'+1");
        assert_eq!(field("\tx", ';'), "'\tx");
        assert_eq!(field("\rx", ';'), "\"'\rx\"");
        assert_eq!(field("a=b", ';'), "a=b");
        assert_eq!(field("=a;b", ';'), "\"'=a;b\"");
    }
    use orbit_editor_sdk::orbit_core::{AssetEntry, AssetId, AssetKind, Character, Color, Node, Scene, SceneId};

    struct Fixture {
        story: Story,
        alice: CharacterId,
        nodes: Vec<NodeId>,
    }

    fn say(story: &mut Story, speaker: Option<CharacterId>, text: &str, voice: Option<AssetId>) -> Node {
        let text_id = story.strings.add(text);
        Node { id: NodeId::new(), kind: NodeKind::Say { speaker, text_id, voice } }
    }

    /// Two scenes: Alice speaks twice (the first line has a voice) and the
    /// narrator once; Bob speaks in the second scene.
    fn fixture() -> (Fixture, AssetId) {
        let mut story = Story::new("Test");
        story.strings.source_lang = "en".into();
        let alice = CharacterId::new();
        let bob = CharacterId::new();
        for (id, name) in [(alice, "Alice"), (bob, "Bob")] {
            let name_id = story.strings.add(name);
            story.characters.push(Character {
                id,
                name_id,
                color: Color::WHITE,
                expressions: Vec::new(),
                default_position: None,
                default_scale: 1.0,
            });
        }
        let voice = AssetId::new();
        story.assets.assets.push(AssetEntry { id: voice, kind: AssetKind::Sound, path: "assets/voice/alice1.ogg".into() });
        let n1 = say(&mut story, Some(alice), "Hello there, friend.", Some(voice));
        let n2 = say(&mut story, None, "The night falls.", None);
        let n3 = say(&mut story, Some(alice), "Where is Bob?\nHe was here.", None);
        let n4 = say(&mut story, Some(bob), "Right behind you!", None);
        let nodes = vec![n1.id, n2.id, n3.id, n4.id];
        story.scenes.push(Scene { id: SceneId::new(), name: "Intro".into(), nodes: vec![n1, n2] });
        story.scenes.push(Scene { id: SceneId::new(), name: "Park".into(), nodes: vec![n3, n4] });
        (Fixture { story, alice, nodes }, voice)
    }

    fn disk(paths: &[&str]) -> VoiceDisk {
        VoiceDisk {
            files: paths.iter().map(|p| p.to_string()).collect(),
            present: paths.iter().map(|p| norm_key(p)).collect(),
        }
    }

    #[test]
    fn lists_the_lines_in_scene_order_with_their_state() {
        let (f, _) = fixture();
        let third = f.nodes[2];
        let named = format!("assets/voice/{third}.wav");
        let lines = build_lines(&f.story, None, None, "Narrator", &disk(&["assets/voice/alice1.ogg", &named]));
        assert_eq!(lines.len(), 4);
        assert_eq!(lines.iter().map(|l| l.node).collect::<Vec<_>>(), f.nodes);
        assert_eq!(lines[0].scene, "Intro");
        assert_eq!(lines[2].scene, "Park");
        assert_eq!(lines[0].state, VoiceState::Recorded);
        assert_eq!(lines[0].expected, "assets/voice/alice1.ogg");
        assert_eq!(lines[1].speaker, "Narrator");
        assert_eq!(lines[1].state, VoiceState::Missing);
        assert_eq!(lines[1].expected, format!("assets/voice/{}.ogg", f.nodes[1]));
        assert_eq!(lines[2].state, VoiceState::PendingImport);
        assert_eq!(lines[2].expected, named);
        assert_eq!(lines[3].speaker, "Bob");
        assert_eq!(lines[2].words, 6);
        assert_eq!(lines[0].id(), f.nodes[0].to_string());
    }

    #[test]
    fn an_assigned_voice_whose_file_is_gone_is_missing() {
        let (f, _) = fixture();
        let lines = build_lines(&f.story, None, None, "N", &disk(&[]));
        assert_eq!(lines[0].state, VoiceState::Missing);
        assert_eq!(lines[0].assigned.as_deref(), Some("assets/voice/alice1.ogg"));
        // The case of the path does not matter on the file systems of the editor.
        let lines = build_lines(&f.story, None, None, "N", &disk(&["Assets/Voice/ALICE1.ogg"]));
        assert_eq!(lines[0].state, VoiceState::Recorded);
    }

    #[test]
    fn a_translation_has_its_own_text_and_voices() {
        let (mut f, _) = fixture();
        let fr_voice = AssetId::new();
        f.story.assets.assets.push(AssetEntry { id: fr_voice, kind: AssetKind::Sound, path: "assets/voice/fr/a1.ogg".into() });
        let mut fr = Translation::new("fr", "Francais");
        let first_text = match &f.story.scenes[0].nodes[0].kind {
            NodeKind::Say { text_id, .. } => *text_id,
            _ => unreachable!(),
        };
        fr.entries.insert(first_text, "Bonjour, mon ami.".into());
        fr.voices.insert(first_text, fr_voice);
        let on_disk = disk(&["assets/voice/alice1.ogg", "assets/voice/fr/a1.ogg"]);
        let lines = build_lines(&f.story, Some("fr"), Some(&fr), "Narrateur", &on_disk);
        assert_eq!(lines[0].text, "Bonjour, mon ami.");
        assert!(lines[0].translated);
        assert_eq!(lines[0].state, VoiceState::Recorded);
        assert_eq!(lines[0].expected, "assets/voice/fr/a1.ogg");
        // Untranslated: the source text is shown and flagged; the source
        // voice is not a recording in French.
        assert_eq!(lines[1].text, "The night falls.");
        assert!(!lines[1].translated);
        assert_eq!(lines[1].speaker, "Narrateur");
        assert_eq!(lines[1].expected, format!("assets/voice/fr/{}.ogg", f.nodes[1]));
        // Without the translation file at all, every line is still listed.
        let lines = build_lines(&f.story, Some("de"), None, "N", &on_disk);
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| !l.translated && l.state == VoiceState::Missing));
    }

    #[test]
    fn files_in_a_language_folder_belong_to_that_language() {
        let (f, _) = fixture();
        let n = f.nodes[1];
        let fr_file = format!("assets/voice/fr/{n}.ogg");
        let on_disk = disk(&[&fr_file]);
        let source = build_lines(&f.story, None, None, "N", &on_disk);
        assert_eq!(source[1].state, VoiceState::Missing, "a French file is not the source voice");
        let fr = build_lines(&f.story, Some("fr"), None, "N", &on_disk);
        assert_eq!(fr[1].state, VoiceState::PendingImport);
        assert_eq!(fr[1].expected, fr_file);
    }

    #[test]
    fn counters_per_speaker_and_selection() {
        let (f, _) = fixture();
        let lines = build_lines(&f.story, None, None, "Narrator", &disk(&["assets/voice/alice1.ogg"]));
        let stats = per_speaker(&lines);
        assert_eq!(stats.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Alice", "Narrator", "Bob"]);
        let alice = &stats[0].stats;
        assert_eq!((alice.lines, alice.words, alice.recorded, alice.missing), (2, 3 + 6, 1, 1));
        let all = Stats::of(&lines);
        assert_eq!((all.lines, all.recorded, all.pending, all.missing), (4, 1, 0, 3));
        assert_eq!(select(&lines, None, false).len(), 4);
        assert_eq!(select(&lines, Some(Speaker::Character(f.alice)), false).len(), 2);
        assert_eq!(select(&lines, Some(Speaker::Character(f.alice)), true).len(), 1);
        assert_eq!(select(&lines, Some(Speaker::Narrator), true).len(), 1);
        assert!(select(&lines, Some(Speaker::Character(CharacterId::new())), false).is_empty());
    }

    fn labels() -> StateLabels {
        StateLabels { recorded: "recorded".into(), pending: "to import".into(), missing: "missing".into() }
    }

    #[test]
    fn csv_has_a_bom_and_quotes_what_needs_it() {
        let (f, _) = fixture();
        let lines = build_lines(&f.story, None, None, "Narrator", &disk(&["assets/voice/alice1.ogg"]));
        let chosen = select(&lines, Some(Speaker::Character(f.alice)), false);
        let header: Vec<String> = (0..CSV_COLUMNS).map(|i| format!("h{i}")).collect();
        let csv = to_csv(&chosen, &header, &labels(), ';');
        assert!(csv.starts_with('\u{feff}'));
        let rows: Vec<&str> = csv.split("\r\n").filter(|r| !r.is_empty()).collect();
        // The header, then the first line, then the second whose text has a
        // line break and so spans two physical lines.
        assert_eq!(rows[0], "\u{feff}h0;h1;h2;h3;h4;h5;h6;h7");
        assert_eq!(
            rows[1],
            format!("1;Intro;{};Alice;Hello there, friend.;3;assets/voice/alice1.ogg;recorded", f.nodes[0])
        );
        assert!(rows[2].starts_with(&format!("2;Park;{};Alice;\"Where is Bob?", f.nodes[2])));
        assert!(csv.contains("He was here.\";6;"));
        // A comma separator forces quotes around "Hello there, friend.".
        let comma = to_csv(&chosen, &header, &labels(), ',');
        assert!(comma.contains("\"Hello there, friend.\""));
        // No line: just the header.
        assert_eq!(to_csv(&[], &header, &labels(), ';').trim_end(), "\u{feff}h0;h1;h2;h3;h4;h5;h6;h7");
    }

    #[test]
    fn actor_text_groups_by_character_without_technical_columns() {
        let (f, _) = fixture();
        let lines = build_lines(&f.story, None, None, "Narrator", &disk(&[]));
        let labels = TextLabels {
            title: "Script - Test".into(),
            heading: "{name} ({lines} lines, {words} words)".into(),
            line: "{n}. [{scene}] {id}".into(),
            nothing: "Nothing to record.".into(),
        };
        let text = to_actor_text(&select(&lines, None, false), &labels);
        assert!(text.starts_with("Script - Test\n"));
        let alice = text.find("Alice (2 lines, 9 words)").unwrap();
        let narrator = text.find("Narrator (1 lines, 3 words)").unwrap();
        let bob = text.find("Bob (1 lines, 3 words)").unwrap();
        assert!(alice < narrator && narrator < bob, "first appearance order");
        assert!(text.contains(&format!("2. [Park] {}\n    Where is Bob?\n    He was here.\n", f.nodes[2])));
        assert!(!text.contains(".ogg"), "no file names for the actors");
        // Empty selection.
        let empty = to_actor_text(&[], &labels);
        assert!(empty.contains("Nothing to record."));
    }

    #[test]
    fn a_story_without_lines_gives_nothing() {
        let story = Story::new("Empty");
        assert!(build_lines(&story, None, None, "N", &VoiceDisk::default()).is_empty());
        assert!(per_speaker(&[]).is_empty());
        assert_eq!(count_words("  two   words\n"), 2);
        assert_eq!(count_words(""), 0);
    }
}
