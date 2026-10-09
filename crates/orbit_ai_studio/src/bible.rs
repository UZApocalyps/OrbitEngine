//! The story bible: the structured plan the LLM writes from the pitch, and
//! the JSON extraction / validation / repair loop around it.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::text::{Message, ProviderError, TextModel};

/// Expressions a character sprite may have. The script may only use these;
/// a picture is generated for each one the script actually uses.
pub const EXPRESSIONS: &[&str] = &[
    "neutral", "happy", "sad", "angry", "surprised", "embarrassed", "scared", "thinking",
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterSpec {
    /// Identifier (`lowercase_snake`), used for file names.
    pub id: String,
    /// Display name, as the script names the speaker.
    pub name: String,
    pub description: String,
    pub personality: String,
    /// Appearance, for the image prompts (hair, eyes, clothes...).
    pub visual: String,
    /// Name color, `#rrggbb`.
    pub color: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocationSpec {
    pub id: String,
    pub name: String,
    /// What the background shows, for the image prompt.
    pub visual: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneOutline {
    /// Identifier, also the scene name in the project.
    pub id: String,
    pub title: String,
    /// Location id of the opening background.
    pub location: String,
    /// Character ids appearing in the scene.
    pub characters: Vec<String>,
    /// What happens (a few sentences, choices included).
    pub summary: String,
    /// Scene ids this scene can continue to (empty for an ending).
    pub next: Vec<String>,
    /// The scene is an ending (it ends the game).
    pub ending: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VariableSpec {
    pub name: String,
    /// `true` / `false`, a number or a string.
    pub initial: Json,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bible {
    pub title: String,
    pub synopsis: String,
    pub characters: Vec<CharacterSpec>,
    pub locations: Vec<LocationSpec>,
    /// In play order; the first one starts the game.
    pub scenes: Vec<SceneOutline>,
    pub variables: Vec<VariableSpec>,
}

impl Bible {
    pub fn character(&self, id: &str) -> Option<&CharacterSpec> {
        self.characters.iter().find(|c| c.id == id)
    }
    pub fn location(&self, id: &str) -> Option<&LocationSpec> {
        self.locations.iter().find(|l| l.id == id)
    }
    pub fn scene(&self, id: &str) -> Option<&SceneOutline> {
        self.scenes.iter().find(|s| s.id == id)
    }
}

/// `lowercase_snake` identifier that is not a DSL keyword.
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !orbit_dsl::printer::KEYWORDS.contains(&s)
}

/// `#rrggbb` → (r, g, b).
pub fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

/// Every problem of a bible, as sentences the model can act on. Empty when
/// it is usable.
pub fn check(b: &Bible) -> Vec<String> {
    let mut errors = Vec::new();
    if b.title.trim().is_empty() {
        errors.push("`title` is empty".to_owned());
    }
    let ids = |what: &str, list: Vec<&str>, errors: &mut Vec<String>| {
        let mut seen = HashSet::new();
        for id in list {
            if !is_identifier(id) {
                errors.push(format!(
                    "{what} id `{id}` must be lowercase_snake_case (a-z, 0-9, _), start with a letter and not be a keyword"
                ));
            }
            if !seen.insert(id.to_owned()) {
                errors.push(format!("{what} id `{id}` is used twice"));
            }
        }
    };
    ids("character", b.characters.iter().map(|c| c.id.as_str()).collect(), &mut errors);
    ids("location", b.locations.iter().map(|c| c.id.as_str()).collect(), &mut errors);
    ids("scene", b.scenes.iter().map(|c| c.id.as_str()).collect(), &mut errors);
    ids("variable", b.variables.iter().map(|c| c.name.as_str()).collect(), &mut errors);

    let mut names = HashSet::new();
    for c in &b.characters {
        let name = c.name.trim();
        if name.is_empty() {
            errors.push(format!("character `{}` has no name", c.id));
        } else if name.contains('"') || name.contains('\\') {
            errors.push(format!("character name `{name}` must not contain quotes or backslashes"));
        } else if !names.insert(name.to_lowercase()) {
            errors.push(format!("two characters are named `{name}`"));
        }
        if !c.color.is_empty() && parse_hex_color(&c.color).is_none() {
            errors.push(format!("character `{}`: color `{}` is not #rrggbb", c.id, c.color));
        }
        if c.visual.trim().is_empty() {
            errors.push(format!("character `{}` has no `visual` description", c.id));
        }
    }
    if b.locations.is_empty() {
        errors.push("there is no location".to_owned());
    }
    if b.scenes.is_empty() {
        errors.push("there is no scene".to_owned());
    }
    if !b.scenes.is_empty() && !b.scenes.iter().any(|s| s.ending) {
        errors.push("no scene has `\"ending\": true`".to_owned());
    }
    for s in &b.scenes {
        if b.location(&s.location).is_none() {
            errors.push(format!("scene `{}`: unknown location `{}`", s.id, s.location));
        }
        for c in &s.characters {
            if b.character(c).is_none() {
                errors.push(format!("scene `{}`: unknown character `{c}`", s.id));
            }
        }
        for n in &s.next {
            if b.scene(n).is_none() {
                errors.push(format!("scene `{}`: `next` names unknown scene `{n}`", s.id));
            }
        }
        if !s.ending && s.next.is_empty() {
            errors.push(format!(
                "scene `{}` is not an ending but has no `next` scene",
                s.id
            ));
        }
    }
    for v in &b.variables {
        if !matches!(v.initial, Json::Bool(_) | Json::Number(_) | Json::String(_)) {
            errors.push(format!(
                "variable `{}`: `initial` must be a boolean, a number or a string",
                v.name
            ));
        }
    }
    errors
}

/// The JSON object inside a model answer: code fences and chatter around it
/// are dropped.
pub fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

/// Parses and checks an answer.
pub fn parse_bible(text: &str) -> Result<Bible, Vec<String>> {
    parse_bible_fixed(text).map(|(b, _)| b)
}

/// Like [`parse_bible`], also returning what the auto-fix repaired.
pub fn parse_bible_fixed(text: &str) -> Result<(Bible, Vec<String>), Vec<String>> {
    let json = extract_json(text).ok_or_else(|| vec!["the answer holds no JSON object".to_owned()])?;
    let mut bible: Bible =
        serde_json::from_str(json).map_err(|e| vec![format!("invalid JSON: {e}")])?;
    let warnings = crate::fix::autofix(&mut bible);
    let errors = check(&bible);
    if errors.is_empty() { Ok((bible, warnings)) } else { Err(errors) }
}

/// The user's starting point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pitch {
    /// The base text: premise, ideas, anything.
    pub text: String,
    pub genre: String,
    pub tone: String,
    /// Language of the dialogue (`French`, `English`...).
    pub language: String,
    pub scenes: u32,
    pub characters: u32,
    pub endings: u32,
    /// Art style, prepended to the image prompts when set.
    pub art_style: String,
    /// Lines of dialogue per scene, roughly.
    pub lines_per_scene: u32,
}

impl Default for Pitch {
    fn default() -> Self {
        Self {
            text: String::new(),
            genre: String::new(),
            tone: String::new(),
            language: "English".to_owned(),
            scenes: 6,
            characters: 3,
            endings: 2,
            art_style: String::new(),
            lines_per_scene: 25,
        }
    }
}

pub const BIBLE_EXAMPLE: &str = r##"{
  "title": "The Lighthouse Keeper",
  "synopsis": "Two strangers stranded by a storm uncover the keeper's secret.",
  "characters": [
    {"id": "mara", "name": "Mara", "description": "A marine biologist", "personality": "curious, stubborn",
     "visual": "young woman, short red hair, green eyes, yellow raincoat", "color": "#e0603a"}
  ],
  "locations": [
    {"id": "lighthouse", "name": "Lighthouse", "visual": "inside an old lighthouse at night, spiral stairs, storm outside the windows"}
  ],
  "scenes": [
    {"id": "arrival", "title": "Arrival", "location": "lighthouse", "characters": ["mara"],
     "summary": "Mara arrives soaked. Choice: explore upstairs or wait.", "next": ["upstairs", "ending_wait"], "ending": false},
    {"id": "upstairs", "title": "Upstairs", "location": "lighthouse", "characters": ["mara"], "summary": "...", "next": [], "ending": true},
    {"id": "ending_wait", "title": "Waiting", "location": "lighthouse", "characters": ["mara"], "summary": "...", "next": [], "ending": true}
  ],
  "variables": [
    {"name": "trust", "initial": 0, "description": "How much Mara trusts the keeper"}
  ]
}"##;

pub fn bible_messages(p: &Pitch) -> Vec<Message> {
    let system = format!(
        "You are a visual novel writer and designer. From the user's pitch you write a story bible \
as ONE JSON object and nothing else (no prose, no markdown). Schema, by example:\n{BIBLE_EXAMPLE}\n\
Rules:\n- every id (characters, locations, scenes, variable names) is lowercase_snake_case;\n\
- `characters[].visual` and `locations[].visual` are concrete visual descriptions for an image generator, in English;\n\
- scenes are listed in play order, the first one starts the game; `next` lists the scenes a scene can lead to (through choices or directly); endings have `\"ending\": true` and an empty `next`;\n\
- EVERY scene id listed in any `next` MUST be defined as its own object in `scenes`; never reference a scene you do not define (if unsure, reuse an existing id or an ending);
\n- every scene can be reached from the first one;\n\
- variables track the player's choices (booleans or numbers);\n\
- names, titles, synopsis and summaries are written in {lang}.",
        lang = p.language
    );
    let mut user = format!("Pitch:\n{}\n\n", p.text.trim());
    let mut opt = |k: &str, v: &str| {
        if !v.trim().is_empty() {
            user.push_str(&format!("{k}: {}\n", v.trim()));
        }
    };
    opt("Genre", &p.genre);
    opt("Tone", &p.tone);
    opt("Language", &p.language);
    opt("Art style", &p.art_style);
    user.push_str(&format!(
        "About {} scenes, {} main characters, {} endings.\nAnswer with the JSON object only.",
        p.scenes, p.characters, p.endings
    ));
    vec![Message::system(system), Message::user(user)]
}

/// Outcome of a generate-and-repair loop.
#[derive(Clone, Debug, PartialEq)]
pub struct Attempts<T> {
    pub value: T,
    /// Repairs that were needed (0 when the first answer was fine).
    pub repairs: u32,
    /// Things fixed automatically (shown as warnings).
    pub warnings: Vec<String>,
}

/// Asks for a bible, feeding validation errors back until it is valid or
/// `max_repairs` repairs failed.
pub fn generate_bible(
    model: &dyn TextModel,
    pitch: &Pitch,
    max_repairs: u32,
    cancelled: &dyn Fn() -> bool,
) -> Result<Attempts<Bible>, StepError> {
    let mut messages = bible_messages(pitch);
    let mut repairs = 0;
    loop {
        if cancelled() {
            return Err(StepError::Provider(ProviderError::Cancelled));
        }
        let answer = model.complete(&messages).map_err(StepError::Provider)?;
        match parse_bible_fixed(&answer) {
            Ok((value, warnings)) => return Ok(Attempts { value, repairs, warnings }),
            Err(errors) => {
                if repairs >= max_repairs {
                    return Err(StepError::Invalid { errors, last: answer });
                }
                repairs += 1;
                messages.push(Message::assistant(answer.clone()));
                let mut req = repair_request(&errors, "the corrected, complete JSON object");
                if let Some(ids) = defined_scene_ids(&answer) {
                    req.push_str(&format!(
                        "
The scene ids defined in `scenes` are: {ids}. Every `next` entry must be one of them, or add the missing scene object."
                    ));
                }
                messages.push(Message::user(req));
            }
        }
    }
}

fn defined_scene_ids(answer: &str) -> Option<String> {
    let v: Json = serde_json::from_str(extract_json(answer)?).ok()?;
    let ids: Vec<&str> = v.get("scenes")?.as_array()?.iter().filter_map(|s| s.get("id")?.as_str()).collect();
    (!ids.is_empty()).then(|| ids.join(", "))
}

pub(crate) fn repair_request(errors: &[String], what: &str) -> String {
    let mut s = String::from("Your answer has these problems:\n");
    for e in errors.iter().take(30) {
        s.push_str("- ");
        s.push_str(e);
        s.push('\n');
    }
    s.push_str(&format!("Answer again with {what} only."));
    s
}

/// Why a step gave up.
#[derive(Clone, Debug, PartialEq)]
pub enum StepError {
    Provider(ProviderError),
    /// Still invalid after the allowed repairs.
    Invalid { errors: Vec<String>, last: String },
    /// A precondition is missing (no bible yet...).
    Missing(String),
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepError::Provider(e) => write!(f, "{e}"),
            StepError::Invalid { errors, .. } => {
                write!(f, "still invalid after repairs: {}", errors.join("; "))
            }
            StepError::Missing(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for StepError {}
