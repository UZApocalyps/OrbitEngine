//! Script step: the LLM writes each scene in the OrbitEngine DSL; the text
//! is parsed and validated against a **scratch story** built from the bible
//! (characters with every allowed expression, the planned backgrounds, the
//! variables), and errors go back to the model for repair.

use std::collections::{BTreeMap, BTreeSet};

use orbit_core::{
    AssetEntry, AssetId, AssetKind, Character, CharacterId, Color, Expression, NodeKind, Story,
    StringId, Value, VarDecl, VarScope,
};
use orbit_dsl::Scope;
use serde_json::Value as Json;

use crate::bible::{
    Attempts, Bible, EXPRESSIONS, SceneOutline, StepError, parse_hex_color, repair_request,
};
use crate::text::{Message, ProviderError, TextModel};

/// Folder of everything the studio generates, relative to the project.
pub const AI_ASSET_DIR: &str = "assets/ai";

/// Manifest path of a location's background.
pub fn background_path(location: &str) -> String {
    format!("{AI_ASSET_DIR}/backgrounds/{location}.png")
}

/// Manifest path of a character sprite.
pub fn sprite_path(character: &str, expression: &str) -> String {
    format!("{AI_ASSET_DIR}/characters/{character}/{expression}.png")
}

/// A JSON scalar → story value.
pub fn json_value(v: &Json) -> Value {
    match v {
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::Str(s.clone()),
        _ => Value::Int(0),
    }
}

pub fn character_color(spec_color: &str, index: usize) -> Color {
    const PALETTE: [(u8, u8, u8); 6] = [
        (224, 96, 58),
        (70, 140, 220),
        (110, 180, 90),
        (200, 90, 170),
        (230, 180, 60),
        (90, 190, 190),
    ];
    let (r, g, b) = parse_hex_color(spec_color).unwrap_or(PALETTE[index % PALETTE.len()]);
    Color::rgb(r, g, b)
}

/// The story the script is checked against: what the project will hold
/// once the bible is implemented, with **every** allowed expression.
pub fn scratch_story(b: &Bible) -> Story {
    let mut story = Story::new(b.title.clone());
    let asset = |story: &mut Story, path: String| {
        let id = AssetId::new();
        story.assets.assets.push(AssetEntry { id, kind: AssetKind::Image, path });
        id
    };
    for l in &b.locations {
        asset(&mut story, background_path(&l.id));
    }
    for (i, c) in b.characters.iter().enumerate() {
        let name_id = StringId::new();
        story.strings.entries.insert(name_id, c.name.trim().to_owned());
        let expressions = EXPRESSIONS
            .iter()
            .map(|e| Expression {
                name: (*e).to_owned(),
                image: asset(&mut story, sprite_path(&c.id, e)),
                motion: None,
            })
            .collect();
        story.characters.push(Character {
            id: CharacterId::new(),
            name_id,
            color: character_color(&c.color, i),
            expressions,
            default_position: None,
            default_scale: 1.0,
        });
    }
    for v in &b.variables {
        story.variables.push(VarDecl {
            name: v.name.clone(),
            initial: json_value(&v.initial),
            scope: VarScope::Game,
        });
    }
    story
}

/// Grammar subset and rules given to the model. Kept small on purpose: the
/// full grammar is in `orbit_dsl::printer`.
pub const DSL_GUIDE: &str = r#"OrbitEngine script language (one statement per line, 4-space indentation for menu options, `#` starts a comment):

scene <scene_id>                          starts the scene (exactly once, first line)
label <name>                              a jump target inside the scene (lowercase_snake_case)
"text"                                    narration
say "<Character Name>" "text"             a character speaks (exact display name)
show "<Character Name>" <expression> at <left|center|right> [with dissolve(0.5)]
hide "<Character Name>" [with dissolve(0.5)]
bg "<background path>" [with dissolve(1.0)]
menu                                      a choice; each option on its own indented line:
    "Option text" -> <label> [if <condition>]
$ <variable> = <expression>               set a variable (true, false, numbers, "text")
$ <variable> += <number>                  also -=
if <condition> -> <label> [else <label>]  branch on a condition (&&, ||, !, ==, !=, <, <=, >, >=)
jump <label>                              go to a label of this scene
jump_scene <scene_id>                     continue with another scene
end                                       the game ends here (endings only)

Rules:
- Strings use double quotes; write \" for a quote inside a string. No line breaks inside a string.
- Every path through the scene must finish with `jump_scene <scene_id>` or `end`.
- After a menu, each option jumps to a label; each labelled branch then ends with `jump`, `jump_scene` or `end`.
- Only use the characters, expressions, backgrounds, variables and scene ids listed below.
- Show a character before it speaks; start the scene with `bg`.
- Output the script only: no markdown fences, no explanations."#;

/// A short complete example (checked by the tests to parse).
pub const DSL_EXAMPLE: &str = r#"scene arrival
bg "assets/ai/backgrounds/lighthouse.png" with dissolve(1.0)
"The storm hammers the windows."
show "Mara" neutral at center with dissolve(0.5)
say "Mara" "Hello? Is anyone here?"
menu
    "Climb the stairs" -> climb
    "Wait by the door" -> wait_here
label climb
$ trust += 1
show "Mara" happy at center
say "Mara" "Let's see what's up there."
jump_scene upstairs
label wait_here
say "Mara" "I'd better stay put."
if trust >= 1 -> climb else stay
label stay
jump_scene ending_wait"#;

/// What the model must know about the bible to write a scene.
pub fn bible_brief(b: &Bible) -> String {
    let mut s = format!("Title: {}\nSynopsis: {}\n\nCharacters (display name, expressions: {}):\n", b.title, b.synopsis, EXPRESSIONS.join(", "));
    for c in &b.characters {
        s.push_str(&format!("- \"{}\": {}. Personality: {}\n", c.name.trim(), c.description, c.personality));
    }
    s.push_str("\nBackgrounds:\n");
    for l in &b.locations {
        s.push_str(&format!("- \"{}\" ({})\n", background_path(&l.id), l.name));
    }
    if !b.variables.is_empty() {
        s.push_str("\nVariables (already declared):\n");
        for v in &b.variables {
            s.push_str(&format!("- {} = {} ({})\n", v.name, v.initial, v.description));
        }
    }
    s.push_str("\nScenes:\n");
    for sc in &b.scenes {
        s.push_str(&format!(
            "- {}{}: {}\n",
            sc.id,
            if sc.ending { " (ending)" } else { "" },
            sc.title
        ));
    }
    s
}

pub fn scene_messages(
    b: &Bible,
    scene: &SceneOutline,
    language: &str,
    lines: u32,
    previous_tail: Option<&str>,
) -> Vec<Message> {
    let system = format!(
        "You write visual novel scenes in the OrbitEngine script language.\n\n{DSL_GUIDE}\n\nExample:\n{DSL_EXAMPLE}"
    );
    let cast: Vec<String> = scene
        .characters
        .iter()
        .filter_map(|c| b.character(c))
        .map(|c| format!("\"{}\"", c.name.trim()))
        .collect();
    let mut user = format!("{}\n", bible_brief(b));
    if let Some(tail) = previous_tail {
        user.push_str(&format!("\nEnd of the previous scene, for continuity:\n{tail}\n"));
    }
    user.push_str(&format!(
        "\nWrite scene `{}` (\"{}\"). Location background: \"{}\". Characters: {}.\nWhat happens: {}\n",
        scene.id,
        scene.title,
        crate::script::background_path(&scene.location),
        if cast.is_empty() { "none".to_owned() } else { cast.join(", ") },
        scene.summary
    ));
    if scene.ending {
        user.push_str("This scene is an ending: it finishes with `end`.\n");
    } else {
        user.push_str(&format!(
            "It must continue with `jump_scene` to: {} (use a menu or a condition when there are several).\n",
            scene.next.join(", ")
        ));
    }
    user.push_str(&format!(
        "Write about {lines} lines of dialogue and narration, in {language}. First line: `scene {}`.",
        scene.id
    ));
    vec![Message::system(system), Message::user(user)]
}

/// Cleans a model answer: fences, `#@` id annotations, a missing `scene`
/// header.
pub fn normalize_scene(text: &str, scene_id: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            continue;
        }
        let line = match line.find("#@") {
            Some(i) => line[..i].trim_end(),
            None => line.trim_end(),
        };
        lines.push(line.to_owned());
    }
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let header = lines.iter().position(|l| l.trim_start().starts_with("scene "));
    match header {
        Some(i) => {
            // Chatter before the header goes.
            lines.drain(..i);
            lines[0] = format!("scene {scene_id}");
        }
        None => lines.insert(0, format!("scene {scene_id}")),
    }
    lines.join("\n")
}

/// Checks one scene against the scratch story. Other scenes of the bible are
/// stubbed so `jump_scene` resolves. Errors carry the scene's own line
/// numbers.
pub fn check_scene(base: &Story, b: &Bible, scene_id: &str, text: &str) -> Result<Story, Vec<String>> {
    let own_lines = text.lines().count();
    let mut full = text.to_owned();
    for s in &b.scenes {
        if s.id != scene_id {
            full.push_str(&format!("\nscene {}\nend", s.id));
        }
    }
    full.push('\n');
    let story = orbit_dsl::parse(&full, base, Scope::Story).map_err(|diags| {
        diags
            .iter()
            .map(|d| {
                if d.line <= own_lines {
                    let src = text.lines().nth(d.line - 1).unwrap_or("").trim();
                    format!("line {} `{src}`: {}", d.line, d.message)
                } else {
                    d.message.clone()
                }
            })
            .collect::<Vec<_>>()
    })?;
    let mut errors = Vec::new();
    if !story.scenes.iter().any(|s| s.name == scene_id) {
        errors.push(format!("the script must start with `scene {scene_id}`"));
    }
    let report = orbit_core::validate::validate(&story);
    errors.extend(report.errors.iter().map(|e| e.to_string()));
    if errors.is_empty() { Ok(story) } else { Err(errors) }
}

/// Writes one scene, repairing it until it checks or `max_repairs` failed.
#[allow(clippy::too_many_arguments)]
pub fn generate_scene(
    model: &dyn TextModel,
    base: &Story,
    b: &Bible,
    scene: &SceneOutline,
    language: &str,
    lines: u32,
    previous_tail: Option<&str>,
    max_repairs: u32,
    cancelled: &dyn Fn() -> bool,
) -> Result<Attempts<String>, StepError> {
    let mut messages = scene_messages(b, scene, language, lines, previous_tail);
    let mut repairs = 0;
    loop {
        if cancelled() {
            return Err(StepError::Provider(ProviderError::Cancelled));
        }
        let answer = model.complete(&messages).map_err(StepError::Provider)?;
        let text = normalize_scene(&answer, &scene.id);
        match check_scene(base, b, &scene.id, &text) {
            Ok(_) => return Ok(Attempts { value: text, repairs, warnings: Vec::new() }),
            Err(errors) => {
                if repairs >= max_repairs {
                    return Err(StepError::Invalid { errors, last: text });
                }
                repairs += 1;
                messages.push(Message::assistant(text));
                messages.push(Message::user(repair_request(&errors, "the corrected, complete scene script")));
            }
        }
    }
}

/// Last lines of a scene, for the next scene's prompt (context stays small).
pub fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Expressions each character (by bible id) uses in the given scenes,
/// `neutral` always included.
pub fn used_expressions<'a>(
    base: &Story,
    b: &Bible,
    scripts: impl Iterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = b
        .characters
        .iter()
        .map(|c| (c.id.clone(), BTreeSet::from(["neutral".to_owned()])))
        .collect();
    for (scene_id, text) in scripts {
        let Ok(story) = check_scene(base, b, scene_id, text) else {
            continue;
        };
        for scene in &story.scenes {
            for node in &scene.nodes {
                if let NodeKind::Show { character, expression, .. } = &node.kind {
                    let Some(idx) = story.characters.iter().position(|c| c.id == *character) else {
                        continue;
                    };
                    if let Some(spec) = b.characters.get(idx) {
                        out.entry(spec.id.clone()).or_default().insert(expression.clone());
                    }
                }
            }
        }
    }
    out
}
