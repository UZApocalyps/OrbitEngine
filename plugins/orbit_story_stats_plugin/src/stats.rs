//! Pure statistics of a story (no egui): counting, reading time, CSV.

use std::collections::HashMap;

use orbit_editor_sdk::orbit_core::{CharacterId, NodeKind, Story};

/// Default reading speed, in words per minute.
pub const DEFAULT_WPM: u32 = 200;
/// Bounds of the reading speed setting.
pub const MIN_WPM: u32 = 50;
pub const MAX_WPM: u32 = 600;

/// Who says a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    Narrator,
    Character(CharacterId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneStats {
    pub name: String,
    /// Nodes of the scene.
    pub steps: usize,
    pub lines: usize,
    pub words: usize,
    pub voiced: usize,
    pub choices: usize,
    pub options: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterStats {
    pub speaker: Speaker,
    /// Display name; empty for the narrator and for an id the story does not
    /// define.
    pub name: String,
    pub lines: usize,
    pub words: usize,
    pub voiced: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoryStats {
    pub scenes: Vec<SceneStats>,
    pub characters: Vec<CharacterStats>,
    pub steps: usize,
    pub lines: usize,
    pub words: usize,
    pub voiced: usize,
    pub choices: usize,
    pub options: usize,
}

impl StoryStats {
    pub fn unvoiced(&self) -> usize {
        self.lines - self.voiced
    }

    /// Share of lines that have a voice, 0..=1 (0 without lines).
    pub fn voice_coverage(&self) -> f32 {
        if self.lines == 0 { 0.0 } else { self.voiced as f32 / self.lines as f32 }
    }
}

/// Words of a text: whitespace-separated pieces with at least one letter or
/// digit (a lone dash or ellipsis is not a word).
pub fn count_words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
}

pub fn clamp_wpm(wpm: u32) -> u32 {
    wpm.clamp(MIN_WPM, MAX_WPM)
}

/// Estimated reading time in seconds.
pub fn reading_seconds(words: usize, wpm: u32) -> u64 {
    let wpm = clamp_wpm(wpm) as u64;
    (words as u64 * 60).div_ceil(wpm)
}

/// `1 h 05 min`, `12 min 30 s` or `45 s`: numbers only, units are the same
/// in every language.
pub fn format_duration(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if h > 0 {
        format!("{h} h {m:02} min")
    } else if m > 0 {
        format!("{m} min {s:02} s")
    } else {
        format!("{s} s")
    }
}

pub fn compute(story: &Story) -> StoryStats {
    let mut out = StoryStats::default();
    let mut by_speaker: HashMap<Speaker, CharacterStats> = HashMap::new();
    for scene in &story.scenes {
        let mut s = SceneStats {
            name: scene.name.clone(),
            steps: scene.nodes.len(),
            ..SceneStats::default()
        };
        for node in &scene.nodes {
            match &node.kind {
                NodeKind::Say { speaker, text_id, voice } => {
                    let words = story.text(*text_id).map_or(0, count_words);
                    s.lines += 1;
                    s.words += words;
                    let voiced = usize::from(voice.is_some());
                    s.voiced += voiced;
                    let who = speaker.map_or(Speaker::Narrator, Speaker::Character);
                    let c = by_speaker.entry(who).or_insert_with(|| CharacterStats {
                        speaker: who,
                        name: String::new(),
                        lines: 0,
                        words: 0,
                        voiced: 0,
                    });
                    c.lines += 1;
                    c.words += words;
                    c.voiced += voiced;
                }
                NodeKind::Choice { options } => {
                    s.choices += 1;
                    s.options += options.len();
                }
                _ => {}
            }
        }
        out.steps += s.steps;
        out.lines += s.lines;
        out.words += s.words;
        out.voiced += s.voiced;
        out.choices += s.choices;
        out.options += s.options;
        out.scenes.push(s);
    }

    // Every defined character is listed (even one who never speaks); the
    // narrator only when it has lines.
    for c in &story.characters {
        let who = Speaker::Character(c.id);
        by_speaker.entry(who).or_insert_with(|| CharacterStats {
            speaker: who,
            name: String::new(),
            lines: 0,
            words: 0,
            voiced: 0,
        });
    }
    let mut characters: Vec<CharacterStats> = by_speaker.into_values().collect();
    for c in &mut characters {
        if let Speaker::Character(id) = c.speaker {
            c.name = story
                .character(id)
                .and_then(|ch| story.text(ch.name_id))
                .unwrap_or_default()
                .to_owned();
        }
    }
    characters.sort_by(|a, b| {
        b.words
            .cmp(&a.words)
            .then(b.lines.cmp(&a.lines))
            .then_with(|| a.name.cmp(&b.name))
    });
    out.characters = characters;
    out
}

/// One CSV cell: quoted when needed, and neutralized against spreadsheet
/// formula injection (a name starting with `=`, `+`, `-` or `@`).
fn cell(text: &str) -> String {
    let mut t = text.to_owned();
    if t.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        t.insert(0, '\'');
    }
    if t.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", t.replace('"', "\"\""))
    } else {
        t
    }
}

/// The statistics as one CSV table (header in English, `\n` line ends):
/// `kind` is `scene`, `character` (the narrator has an empty name) or
/// `total`; columns that do not apply to a row are empty.
pub fn to_csv(stats: &StoryStats, wpm: u32) -> String {
    let mut out = String::from("kind,name,steps,lines,words,voiced_lines,choices,options,reading_seconds\n");
    for s in &stats.scenes {
        out.push_str(&format!(
            "scene,{},{},{},{},{},{},{},{}\n",
            cell(&s.name),
            s.steps,
            s.lines,
            s.words,
            s.voiced,
            s.choices,
            s.options,
            reading_seconds(s.words, wpm)
        ));
    }
    for c in &stats.characters {
        out.push_str(&format!(
            "character,{},,{},{},{},,,{}\n",
            cell(&c.name),
            c.lines,
            c.words,
            c.voiced,
            reading_seconds(c.words, wpm)
        ));
    }
    out.push_str(&format!(
        "total,,{},{},{},{},{},{},{}\n",
        stats.steps,
        stats.lines,
        stats.words,
        stats.voiced,
        stats.choices,
        stats.options,
        reading_seconds(stats.words, wpm)
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_editor_sdk::orbit_core::{
        AssetId, Character, ChoiceOption, Color, Node, NodeId, Scene, SceneId, StringId,
    };

    fn node(kind: NodeKind) -> Node {
        Node { id: NodeId::new(), kind }
    }

    fn say(story: &mut Story, speaker: Option<CharacterId>, text: &str, voiced: bool) -> Node {
        let text_id = story.strings.add(text);
        node(NodeKind::Say { speaker, text_id, voice: voiced.then(AssetId::new) })
    }

    fn character(story: &mut Story, name: &str) -> CharacterId {
        let id = CharacterId::new();
        let name_id = story.strings.add(name);
        story.characters.push(Character {
            id,
            name_id,
            color: Color::WHITE,
            expressions: Vec::new(),
            default_position: None,
            default_scale: 1.0,
        });
        id
    }

    fn scene(name: &str, nodes: Vec<Node>) -> Scene {
        Scene { id: SceneId::new(), name: name.into(), nodes }
    }

    #[test]
    fn words_ignore_punctuation_only_pieces() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("  Hello,   world!  "), 2);
        assert_eq!(count_words("Wait ... what — really?"), 3);
        assert_eq!(count_words("l'été 42"), 2);
    }

    #[test]
    fn reading_time_rounds_up_and_clamps_the_speed() {
        assert_eq!(reading_seconds(0, 200), 0);
        assert_eq!(reading_seconds(200, 200), 60);
        assert_eq!(reading_seconds(1, 200), 1);
        // 0 words per minute does not divide by zero: clamped to the minimum.
        assert_eq!(reading_seconds(50, 0), 60);
        assert_eq!(reading_seconds(600, 100_000), 60);
        assert_eq!(format_duration(45), "45 s");
        assert_eq!(format_duration(750), "12 min 30 s");
        assert_eq!(format_duration(3900), "1 h 05 min");
    }

    #[test]
    fn an_empty_story_has_empty_stats() {
        let stats = compute(&Story::new("t"));
        assert_eq!(stats, StoryStats::default());
        assert_eq!(stats.voice_coverage(), 0.0);
        assert_eq!(stats.unvoiced(), 0);
        let csv = to_csv(&stats, 200);
        assert_eq!(csv.lines().count(), 2, "{csv}");
    }

    #[test]
    fn counts_per_scene_and_per_character() {
        let mut story = Story::new("t");
        let ann = character(&mut story, "Ann");
        let bob = character(&mut story, "Bob");
        let _silent = character(&mut story, "Silent");
        let a = vec![
            say(&mut story, Some(ann), "Hello there, Bob!", true),
            say(&mut story, Some(bob), "Hi.", false),
            say(&mut story, None, "The door opens slowly.", false),
        ];
        let opt = |story: &mut Story| ChoiceOption {
            text_id: story.strings.add("Go"),
            target: NodeId::new(),
            condition: None,
        };
        let choice = node(NodeKind::Choice { options: vec![opt(&mut story), opt(&mut story)] });
        let mut b = vec![say(&mut story, Some(ann), "Again", true), choice];
        b.push(node(NodeKind::End));
        story.scenes.push(scene("a", a));
        story.scenes.push(scene("b", b));
        // A line whose text is missing counts as a line with no word.
        story.scenes[1].nodes.push(node(NodeKind::Say {
            speaker: Some(ann),
            text_id: StringId::new(),
            voice: None,
        }));

        let stats = compute(&story);
        assert_eq!(stats.scenes.len(), 2);
        assert_eq!((stats.steps, stats.lines, stats.words), (3 + 4, 3 + 2, 3 + 1 + 4 + 1));
        assert_eq!((stats.choices, stats.options), (1, 2));
        assert_eq!((stats.voiced, stats.unvoiced()), (2, 3));
        assert!((stats.voice_coverage() - 0.4).abs() < 1e-6);
        assert_eq!(stats.scenes[1].choices, 1);

        let by_name = |n: &str| stats.characters.iter().find(|c| c.name == n).unwrap();
        assert_eq!((by_name("Ann").lines, by_name("Ann").words, by_name("Ann").voiced), (3, 4, 2));
        assert_eq!(by_name("Bob").lines, 1);
        // Listed although it never speaks.
        assert_eq!(by_name("Silent").lines, 0);
        let narrator = stats.characters.iter().find(|c| c.speaker == Speaker::Narrator).unwrap();
        assert_eq!((narrator.lines, narrator.words), (1, 4));
        // Sorted by words, most talkative first.
        assert_eq!(stats.characters[0].name, "Ann");
    }

    #[test]
    fn a_dangling_speaker_is_kept_without_a_name() {
        let mut story = Story::new("t");
        let ghost = CharacterId::new();
        let n = say(&mut story, Some(ghost), "Boo", false);
        story.scenes.push(scene("a", vec![n]));
        let stats = compute(&story);
        assert_eq!(stats.characters.len(), 1);
        assert_eq!(stats.characters[0].speaker, Speaker::Character(ghost));
        assert_eq!(stats.characters[0].name, "");
    }

    #[test]
    fn csv_quotes_and_neutralizes_cells() {
        assert_eq!(cell("plain"), "plain");
        assert_eq!(cell("a,b"), "\"a,b\"");
        assert_eq!(cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(cell("=1+1"), "'=1+1");
        assert_eq!(cell("-x"), "'-x");

        let mut story = Story::new("t");
        let n = say(&mut story, None, "one two three", false);
        story.scenes.push(scene("Act 1, intro", vec![n]));
        let csv = to_csv(&compute(&story), 180);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "kind,name,steps,lines,words,voiced_lines,choices,options,reading_seconds");
        assert_eq!(lines[1], "scene,\"Act 1, intro\",1,1,3,0,0,0,1");
        assert_eq!(lines[2], "character,,,1,3,0,,,1");
        assert_eq!(lines[3], "total,,1,1,3,0,0,0,1");
        assert!(csv.ends_with('\n'));
    }
}
