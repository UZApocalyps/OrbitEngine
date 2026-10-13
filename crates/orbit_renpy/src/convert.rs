//! Ren'Py statements to the story IR.
//!
//! One scene per top-level label (`start` first); nested and local
//! (`.name`) labels become `Label` nodes of their scene. Ren'Py falls
//! through from a label into the next one of the same file: the scene then
//! ends with a `JumpScene` to it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use orbit_core::{
    Achievement, AchievementId, AnimFrames, AnimProp, AnimTarget, Animation, AssetEntry, AssetId,
    AssetKind, BinOp, Character, CharacterId, ChoiceOption, Color, Curve, Effect, Expr, Expression,
    Hotspot, HotspotMap, HotspotShape, Key, Loop, LoopMode, Motion, Node, NodeId, NodeKind,
    Position, PositionDef, Scene, SceneId, Story, StoryTrack, StringId, Track, TrackData,
    Transition, Value, VarDecl,
};

use crate::atl::{self, Atl, Prop};
use crate::lexer::{Line, Tok, matching, parse_tree, split_args, tokenize};
use crate::pyexpr::{parse_expr_toks, parse_literal, to_rhai_toks, var_name, zero_like};
use crate::screen::{self, Action};
use crate::text::{self, TextNotes};
use crate::{Issue, MediaKind};

/// A `.rpy` file of the game, path relative to the game folder.
pub struct Script {
    pub path: String,
    pub source: String,
}

/// Result of the conversion, before anything is written.
pub struct Converted {
    pub story: Story,
    /// (path in the game folder, path in the project) to copy.
    pub copies: Vec<(String, String)>,
    /// Generated files (project path, bytes).
    pub generated: Vec<(String, Vec<u8>)>,
    /// Raw Ren'Py text -> the strings showing it (translations).
    pub text_index: HashMap<String, Vec<StringId>>,
    /// Text put around a string by its speaker's style (`what_prefix`,
    /// `what_color`...), to put around its translations too.
    pub wrappers: HashMap<StringId, (String, String)>,
    pub unsupported: Vec<Issue>,
    pub warnings: Vec<Issue>,
    pub labels: usize,
}

struct LabelRef {
    scene: usize,
    node: NodeId,
    /// The label that starts its scene (a jump to it is a plain `JumpScene`).
    entry: bool,
}

struct SceneSrc {
    name: String,
    id: SceneId,
    file: usize,
    /// (file, top-level line index) of the label and of the top-level local
    /// labels that follow it.
    blocks: Vec<usize>,
}

const BLACK: &str = "black";

/// A repeated remark, reported once with its count.
struct Note {
    issue: Issue,
    count: usize,
    unsupported: bool,
}

/// The last line of dialogue, for `extend`.
struct LastSay {
    text_id: StringId,
    speaker: Option<CharacterId>,
    /// Converted text, before the speaker's style.
    core: String,
    raw: String,
    /// The line ended with `{nw}` (no click before the extension).
    no_wait: bool,
}

/// An image defined by an ATL block of frames (`image fire:` / `"a.png"` /
/// `pause 0.1` / `repeat`).
struct Frames {
    paths: Vec<String>,
    fps: f32,
    looping: bool,
}

/// Prefix of the generated images in the image table.
const GENERATED: &str = "gen:";

pub struct Conv<'a> {
    scripts: &'a [Script],
    trees: Vec<Vec<Line>>,
    /// Media files of the game folder (path relative to it, lowercase ext).
    media: &'a [(String, MediaKind)],
    story: Story,
    copies: Vec<(String, String)>,
    generated: Vec<(String, Vec<u8>)>,
    text_index: HashMap<String, Vec<StringId>>,
    unsupported: Vec<Issue>,
    warnings: Vec<Issue>,
    /// Image name ("eileen happy") -> file in the game folder.
    images: BTreeMap<String, String>,
    assets: HashMap<String, AssetId>,
    audio_alias: HashMap<String, String>,
    transitions: HashMap<String, Option<Transition>>,
    char_by_var: HashMap<String, CharacterId>,
    char_by_tag: HashMap<String, CharacterId>,
    tag_of: HashMap<CharacterId, String>,
    /// Characters possibly on stage: expression and position.
    stage: HashMap<CharacterId, (String, Position)>,
    scene_tags: HashSet<String>,
    vars: HashSet<String>,
    labels: HashMap<String, LabelRef>,
    called: HashSet<String>,
    scenes: Vec<SceneSrc>,
    // ---- state while converting one scene
    file: usize,
    global: String,
    cur: Vec<Node>,
    run: Vec<usize>,
    pending_voice: Option<AssetId>,
    counter: usize,
    scene_called: bool,
    // ---- palier 1
    /// The game folder, to read the size of button pictures.
    game_root: Option<PathBuf>,
    screen_size: atl::Screen,
    /// `transform name:` blocks, parsed.
    transforms: HashMap<String, Atl>,
    /// `transform name(params):` (not converted).
    transform_params: HashSet<String>,
    /// Navigation screens.
    screens: HashMap<String, screen::Screen>,
    /// Image name -> its frames.
    frames: HashMap<String, Frames>,
    /// Layered image tag -> its base picture.
    layered: HashMap<String, String>,
    achievements: HashMap<String, AchievementId>,
    /// Speaker -> (text before, text after) its lines.
    styles: HashMap<CharacterId, (String, String)>,
    wrappers: HashMap<StringId, (String, String)>,
    notes: Vec<Note>,
    note_index: HashMap<String, usize>,
    last_say: Option<LastSay>,
    /// Character variables that speak as the narrator.
    narrators: HashSet<String>,
}

/// Normalized image name: lowercase words separated by one space.
pub fn image_name(s: &str) -> String {
    s.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Project path of a game file: `assets/` + the path, spaces replaced.
pub fn project_path(game_rel: &str) -> String {
    format!("assets/{}", game_rel.replace(' ', "_"))
}

impl<'a> Conv<'a> {
    pub fn new(
        title: &str,
        scripts: &'a [Script],
        media: &'a [(String, MediaKind)],
        game_root: Option<PathBuf>,
    ) -> Self {
        let trees = scripts
            .iter()
            .map(|s| parse_tree(&s.source, false))
            .collect();
        let screen_size = screen_size(scripts);
        let mut images = BTreeMap::new();
        for (path, kind) in media {
            if *kind != MediaKind::Image {
                continue;
            }
            // Ren'Py defines an image for each file under images/ (folders
            // ignored): `images/eileen happy.png` is `eileen happy`.
            if let Some(rest) = path.strip_prefix("images/") {
                let file = rest.rsplit('/').next().unwrap_or(rest);
                let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
                images
                    .entry(image_name(stem))
                    .or_insert_with(|| path.clone());
            }
        }
        Self {
            scripts,
            trees,
            media,
            story: Story::new(title),
            copies: Vec::new(),
            generated: Vec::new(),
            text_index: HashMap::new(),
            unsupported: Vec::new(),
            warnings: Vec::new(),
            images,
            assets: HashMap::new(),
            audio_alias: HashMap::new(),
            transitions: HashMap::new(),
            char_by_var: HashMap::new(),
            char_by_tag: HashMap::new(),
            tag_of: HashMap::new(),
            stage: HashMap::new(),
            scene_tags: HashSet::new(),
            vars: HashSet::new(),
            labels: HashMap::new(),
            called: HashSet::new(),
            scenes: Vec::new(),
            file: 0,
            global: String::new(),
            cur: Vec::new(),
            run: Vec::new(),
            pending_voice: None,
            counter: 0,
            scene_called: false,
            game_root,
            screen_size,
            transforms: HashMap::new(),
            transform_params: HashSet::new(),
            screens: HashMap::new(),
            frames: HashMap::new(),
            layered: HashMap::new(),
            achievements: HashMap::new(),
            styles: HashMap::new(),
            wrappers: HashMap::new(),
            notes: Vec::new(),
            note_index: HashMap::new(),
            last_say: None,
            narrators: ["narrator", "centered", "extend"]
                .map(str::to_owned)
                .into_iter()
                .collect(),
        }
    }

    // ------------------------------------------------------------ issues

    fn issue(&self, line: usize, message: impl Into<String>) -> Issue {
        Issue {
            file: self.scripts[self.file].path.clone(),
            line,
            message: message.into(),
        }
    }

    fn warn(&mut self, line: usize, message: impl Into<String>) {
        let issue = self.issue(line, message);
        self.warnings.push(issue);
    }

    fn unsupported(&mut self, line: &Line, what: &str) {
        let text: String = line.text.chars().take(80).collect();
        let issue = self.issue(line.line, format!("{what}: `{text}`"));
        self.unsupported.push(issue);
    }

    /// A remark that may repeat: reported once (at its first place) with
    /// the number of times it was met. `key` groups the occurrences.
    fn note(&mut self, key: &str, line: usize, unsupported: bool, message: impl Into<String>) {
        match self.note_index.get(key) {
            Some(&i) => self.notes[i].count += 1,
            None => {
                let issue = self.issue(line, message);
                self.note_index.insert(key.to_owned(), self.notes.len());
                self.notes.push(Note {
                    issue,
                    count: 1,
                    unsupported,
                });
            }
        }
    }

    fn flush_notes(&mut self) {
        for n in std::mem::take(&mut self.notes) {
            let mut issue = n.issue;
            if n.count > 1 {
                issue.message = format!("{} ({} times, first here)", issue.message, n.count);
            }
            if n.unsupported {
                self.unsupported.push(issue);
            } else {
                self.warnings.push(issue);
            }
        }
    }

    // ------------------------------------------------------------ run

    pub fn run(mut self) -> Converted {
        // 1. definitions (image, define, default) everywhere at top level.
        for f in 0..self.trees.len() {
            self.file = f;
            let tree = std::mem::take(&mut self.trees[f]);
            for line in &tree {
                self.top_level(line);
            }
            self.trees[f] = tree;
        }
        // Persistent fields first assigned in a label, typed by the value.
        for f in 0..self.trees.len() {
            self.file = f;
            let tree = std::mem::take(&mut self.trees[f]);
            self.prescan_persistent(&tree);
            self.trees[f] = tree;
        }
        if let Some(side) = self.images.keys().find(|k| k.starts_with("side ")).cloned() {
            self.warnings.push(Issue {
                file: String::new(),
                line: 0,
                message: format!(
                    "side images (`{side}`...) are not shown: the engine has no side image next to the text box"
                ),
            });
        }
        // Labels called from navigation screens return.
        let screen_calls: Vec<String> = self
            .screens
            .values()
            .flat_map(|s| s.buttons.iter().flat_map(|b| b.actions.iter()))
            .filter_map(|a| match a {
                Action::Call(l) => Some(l.clone()),
                _ => None,
            })
            .collect();
        self.called.extend(screen_calls);
        // 2. labels, calls and scene tags.
        for f in 0..self.trees.len() {
            self.file = f;
            let tree = std::mem::take(&mut self.trees[f]);
            self.collect_labels(&tree);
            self.trees[f] = tree;
        }
        if let Some(i) = self.scenes.iter().position(|s| s.name == "start") {
            let start = self.scenes.remove(i);
            self.scenes.insert(0, start);
            for l in self.labels.values_mut() {
                l.scene = match l.scene {
                    s if s == i => 0,
                    s if s < i => s + 1,
                    s => s,
                };
            }
        } else if !self.scenes.is_empty() {
            self.warnings.push(Issue {
                file: String::new(),
                line: 0,
                message: "no `start` label: the game starts at the first label found".into(),
            });
        }
        // 3. scenes.
        for s in 0..self.scenes.len() {
            self.convert_scene(s);
        }
        if self.story.scenes.is_empty() {
            let text_id = self.story.strings.add("(empty Ren'Py import)");
            self.story.scenes.push(Scene {
                id: SceneId::new(),
                name: "start".into(),
                nodes: vec![
                    node(NodeKind::Say {
                        speaker: None,
                        text_id,
                        voice: None,
                    }),
                    node(NodeKind::End),
                ],
            });
        }
        self.declare_unused_media();
        self.flush_notes();
        Converted {
            labels: self.labels.len(),
            story: self.story,
            copies: self.copies,
            generated: self.generated,
            text_index: self.text_index,
            wrappers: self.wrappers,
            unsupported: self.unsupported,
            warnings: self.warnings,
        }
    }

    // ------------------------------------------------------------ top level

    fn top_level(&mut self, line: &Line) {
        let toks = tokenize(&line.text);
        let first = toks.first().and_then(Tok::ident).unwrap_or("");
        match first {
            "label" | "translate" => {}
            "define" | "default" => self.definition(line, &toks, first == "default"),
            "image" => self.image_statement(line, &toks),
            "init" => {
                if toks.iter().any(|t| t.is_word("python")) {
                    self.init_python(line);
                } else if toks.get(1).is_some_and(|t| t.is_word("offset")) {
                    // `init offset = -2`: only an ordering hint.
                } else {
                    for c in &line.children {
                        self.top_level(c);
                    }
                }
            }
            "python" => self.unsupported(line, "python block (skipped)"),
            "screen" => self.screen_definition(line, &toks),
            "style" => {
                let key = format!("style:{}", self.file);
                self.note(
                    &key,
                    line.line,
                    true,
                    "`style` statements skipped (the look of the interface is set in the UI editor)",
                );
            }
            "transform" => self.transform_definition(line, &toks),
            "layeredimage" => self.layered_image(line, &toks),
            _ => self.unsupported(line, "statement outside a label (skipped)"),
        }
    }

    /// `init python:` blocks: achievements, the screen size and simple
    /// assignments are read; the rest (functions, classes...) is reported.
    fn init_python(&mut self, line: &Line) {
        let mut skipped = 0;
        for c in &line.children {
            let toks = tokenize(&c.text);
            let first = toks.first().and_then(Tok::ident).unwrap_or("");
            if first == "achievement.register" {
                if let Some(name) = call_args(&toks).0.first().and_then(|a| str_arg(a)) {
                    self.achievement(&name);
                }
                continue;
            }
            let quiet = [
                "config.",
                "gui.",
                "build.",
                "style.",
                "preferences.",
                "renpy.music.register_channel",
                "renpy.add_layer",
            ];
            if quiet.iter().any(|p| first.starts_with(p)) {
                continue;
            }
            // NAME = literal: a variable.
            if let [Tok::Ident(name), Tok::Op(eq), rest @ ..] = toks.as_slice()
                && eq == "="
                && c.children.is_empty()
                && let Some(v) = parse_literal(rest)
            {
                let var = var_name(name);
                if self.vars.insert(var.clone()) {
                    self.story
                        .variables
                        .push(if name.starts_with("persistent.") {
                            VarDecl::persistent(var, v)
                        } else {
                            VarDecl::game(var, v)
                        });
                }
                continue;
            }
            skipped += 1;
        }
        if skipped > 0 {
            self.unsupported(
                line,
                &format!("init python block: {skipped} statement(s) not converted (functions, classes, imports...)"),
            );
        }
    }

    /// `$ persistent.x = literal` anywhere in the labels: declares `x`.
    fn prescan_persistent(&mut self, lines: &[Line]) {
        for l in lines {
            if let Some(code) = l.text.strip_prefix('$') {
                let toks = tokenize(code);
                if let [Tok::Ident(n), Tok::Op(eq), rest @ ..] = toks.as_slice()
                    && eq == "="
                    && n.starts_with("persistent.")
                    && let Some(v) = parse_literal(rest)
                {
                    self.declare_assigned(l.line, &var_name(n), n, Some(v));
                }
            }
            self.prescan_persistent(&l.children);
        }
    }

    /// An achievement by its Ren'Py name (registered when first met).
    fn achievement(&mut self, name: &str) -> AchievementId {
        if let Some(id) = self.achievements.get(name) {
            return *id;
        }
        let id = AchievementId::new();
        let mut title = name.replace(['_', '-'], " ");
        if let Some(f) = title.get_mut(0..1) {
            f.make_ascii_uppercase();
        }
        let title_id = self.story.strings.add(title);
        let description_id = self.story.strings.add("");
        self.story.achievements.push(Achievement {
            id,
            title_id,
            description_id,
            hidden: false,
            icon: None,
            locked_icon: None,
        });
        self.achievements.insert(name.to_owned(), id);
        id
    }

    fn screen_definition(&mut self, line: &Line, toks: &[Tok]) {
        let name = toks.get(1).and_then(Tok::ident).unwrap_or("").to_owned();
        if screen::STANDARD.contains(&name.as_str()) {
            self.note(
                "standard screens",
                line.line,
                true,
                "standard Ren'Py interface screens skipped (say, menus, save/load...: the engine has its own, made in the UI editor)",
            );
            return;
        }
        let s = screen::parse(line);
        if s.buttons.is_empty() {
            self.unsupported(
                line,
                "screen (not a navigation screen: OrbitEngine screens are made in the UI editor)",
            );
            return;
        }
        for (l, why) in &s.skipped {
            self.warn(
                *l,
                format!("screen `{name}`: {why}, the button is left out"),
            );
        }
        for (l, what) in &s.unsupported {
            self.warn(*l, format!("screen `{name}`: `{what}` is left out"));
        }
        self.screens.insert(name, s);
    }

    fn transform_definition(&mut self, line: &Line, toks: &[Tok]) {
        let Some(name) = toks.get(1).and_then(Tok::ident) else {
            self.unsupported(line, "transform");
            return;
        };
        if toks.get(2).is_some_and(|t| t.is_op("(")) {
            self.transform_params.insert(name.to_owned());
            self.unsupported(line, "transform with parameters (not converted)");
            return;
        }
        let parsed = atl::parse_block(&line.children, self.screen_size);
        if !parsed.ignored.is_empty() {
            self.warn(
                line.line,
                format!(
                    "transform `{name}`: {} not converted",
                    parsed.ignored.join(", ")
                ),
            );
        }
        self.transforms.insert(name.to_owned(), parsed);
    }

    /// `layeredimage name:` the engine does not composite layers: the
    /// always-shown layer (else the first default attribute) stands for
    /// the whole image.
    fn layered_image(&mut self, line: &Line, toks: &[Tok]) {
        let name: Vec<&str> = toks[1..].iter().filter_map(Tok::ident).collect();
        let tag = image_name(&name.join(" "));
        // Candidates for the base picture, best first.
        let mut always = Vec::new();
        let mut defaults = Vec::new();
        let mut attributes = 0;
        let first_string = |l: &Line| -> Option<String> {
            std::iter::once(l)
                .chain(l.children.iter())
                .flat_map(|x| tokenize(&x.text))
                .find_map(|t| match t {
                    Tok::Str(s) => Some(s),
                    _ => None,
                })
        };
        for c in &line.children {
            let t = tokenize(c.text.trim_end_matches(':'));
            let mut entries = vec![(c, None)];
            if t.first().is_some_and(|t| t.is_word("group")) {
                let g = t.get(1).and_then(Tok::ident).unwrap_or("").to_owned();
                entries = c.children.iter().map(|cc| (cc, Some(g.clone()))).collect();
            }
            for (l, group) in entries {
                let t = tokenize(l.text.trim_end_matches(':'));
                match t.first().and_then(Tok::ident) {
                    Some("always") => always.extend(first_string(l)),
                    Some("attribute") => {
                        attributes += 1;
                        if t.iter().any(|t| t.is_word("default")) {
                            let attr = t.get(1).and_then(Tok::ident).unwrap_or("");
                            defaults.push(first_string(l).unwrap_or_else(|| match &group {
                                Some(g) => format!("{tag}_{g}_{attr}"),
                                None => format!("{tag}_{attr}"),
                            }));
                        }
                    }
                    _ => {}
                }
            }
        }
        let base = always
            .iter()
            .chain(&defaults)
            .find_map(|s| self.resolve_picture(s));
        match base {
            Some(path) => {
                self.layered.insert(tag.clone(), path);
                self.warn(
                    line.line,
                    format!(
                        "layered image `{tag}`: layers are not composited, its base layer stands for it ({attributes} attribute(s) not shown)"
                    ),
                );
            }
            None => self.unsupported(line, "layered image without a base picture found"),
        }
    }

    /// A picture named in a script: an image name (`"eileen happy"`) or a
    /// file (`"images/a.png"`, `"a.png"`), as an image table path.
    fn resolve_picture(&self, s: &str) -> Option<String> {
        if let Some(p) = self.images.get(&image_name(s)) {
            return Some(p.clone());
        }
        self.find_media(s, &["", "images/"]).filter(|p| {
            self.media
                .iter()
                .any(|(m, k)| m == p && *k == MediaKind::Image)
        })
    }

    fn definition(&mut self, line: &Line, toks: &[Tok], default: bool) {
        // define [priority] NAME = EXPR
        let mut i = 1;
        if matches!(toks.get(i), Some(Tok::Num(_)) | Some(Tok::Op(_))) {
            while i < toks.len() && !matches!(toks[i], Tok::Ident(_)) {
                i += 1;
            }
        }
        let (Some(Tok::Ident(name)), Some(eq)) = (toks.get(i), toks.get(i + 1)) else {
            self.unsupported(line, "definition");
            return;
        };
        if !eq.is_op("=") {
            self.unsupported(line, "definition");
            return;
        }
        let name = name.clone();
        let value = &toks[i + 2..];
        if name == "config.name" {
            if let Some(Value::Str(title)) = parse_literal(value) {
                self.story.title = title;
            }
            return;
        }
        if let Some(alias) = name.strip_prefix("audio.") {
            if let Some(Value::Str(path)) = parse_literal(value) {
                self.audio_alias.insert(alias.to_owned(), path);
            }
            return;
        }
        let call = value.first().and_then(Tok::ident).unwrap_or("");
        if matches!(
            call,
            "Character" | "DynamicCharacter" | "ADVCharacter" | "NVLCharacter"
        ) {
            self.character(line, &name, call, value);
            return;
        }
        let ignored = ["config.", "gui.", "build.", "style.", "preferences.", "_"];
        if ignored.iter().any(|p| name.starts_with(p)) {
            return;
        }
        if let Some(t) = self.transition(line.line, value) {
            self.transitions.insert(name, t);
            return;
        }
        if matches!(call, "Transform" | "Position") && value.get(1).is_some_and(|t| t.is_op("(")) {
            let close = matching(value, 1).unwrap_or(value.len());
            let parsed = atl::parse_kwargs(&split_args(&value[2..close]), self.screen_size);
            if !parsed.ignored.is_empty() {
                self.warn(
                    line.line,
                    format!(
                        "transform `{name}`: {} not converted",
                        parsed.ignored.join(", ")
                    ),
                );
            }
            self.transforms.insert(name, parsed);
            return;
        }
        match parse_literal(value) {
            Some(v) => {
                let var = var_name(&name);
                if self.vars.insert(var.clone()) {
                    self.story
                        .variables
                        .push(if name.starts_with("persistent.") {
                            VarDecl::persistent(var, v)
                        } else {
                            VarDecl::game(var, v)
                        });
                }
            }
            None => {
                let what = if default {
                    "default with a non-literal value"
                } else {
                    "define with a non-literal value"
                };
                self.unsupported(line, what);
            }
        }
    }

    fn character(&mut self, line: &Line, var: &str, call: &str, value: &[Tok]) {
        let close = matching(value, 1).unwrap_or(value.len());
        let args = if value.len() > 2 {
            split_args(&value[2..close.min(value.len())])
        } else {
            Vec::new()
        };
        let mut name = None;
        let mut color = Color::WHITE;
        let mut tag = None;
        let (mut prefix, mut suffix) = (String::new(), String::new());
        let (mut open, mut close) = (String::new(), String::new());
        let mut ignored = Vec::new();
        for (n, arg) in args.iter().enumerate() {
            let kw = match arg {
                [Tok::Ident(k), Tok::Op(eq), rest @ ..] if eq == "=" => Some((k.as_str(), rest)),
                _ => None,
            };
            match kw {
                None if n == 0 => {
                    name = match parse_literal(arg) {
                        Some(Value::Str(s)) => Some(s),
                        _ => None,
                    };
                    if call == "DynamicCharacter" {
                        self.warn(
                            line.line,
                            format!(
                                "DynamicCharacter `{var}`: its name is fixed to the variable name"
                            ),
                        );
                    }
                }
                Some(("color" | "who_color", v)) => {
                    if let Some(Value::Str(c)) = parse_literal(v) {
                        color = parse_color(&c).unwrap_or(Color::WHITE);
                    }
                }
                Some(("image", v)) => {
                    if let Some(Value::Str(t)) = parse_literal(v) {
                        tag = Some(image_name(&t));
                    }
                }
                Some(("kind", v)) if v.first().is_some_and(|t| t.is_word("nvl")) => {
                    self.nvl(line.line)
                }
                Some(("kind", _)) => {}
                Some(("what_color", v)) => {
                    if let Some(Value::Str(c)) = parse_literal(v)
                        && parse_color(&c).is_some()
                    {
                        open.push_str(&format!("{{color={c}}}"));
                        close.insert_str(0, "{/color}");
                    }
                }
                Some(("what_prefix", v)) => {
                    if let Some(Value::Str(s)) = parse_literal(v) {
                        prefix = text::convert(&s, &mut TextNotes::default());
                    }
                }
                Some(("what_suffix", v)) => {
                    if let Some(Value::Str(s)) = parse_literal(v) {
                        suffix = text::convert(&s, &mut TextNotes::default());
                    }
                }
                Some((style @ ("what_italic" | "what_bold"), v))
                    if v.first().is_some_and(|t| t.is_word("True")) =>
                {
                    let t = if style == "what_italic" { "i" } else { "b" };
                    open.push_str(&format!("{{{t}}}"));
                    close.insert_str(0, &format!("{{/{t}}}"));
                }
                // Layout of the name box / text box: the engine's UI.
                Some((k, _))
                    if k.starts_with("who_") || k.starts_with("window_") || k == "dynamic" => {}
                Some((k, _)) => ignored.push(k.to_owned()),
                None => {}
            }
        }
        if call == "NVLCharacter" {
            self.nvl(line.line);
        }
        if !ignored.is_empty() {
            self.warn(
                line.line,
                format!("character `{var}`: {} ignored", ignored.join(", ")),
            );
        }
        let nameless = name.is_none()
            && args
                .first()
                .is_some_and(|a| matches!(a, [Tok::Ident(n)] if n == "None"));
        if var == "narrator" || var == "centered" || nameless {
            // `Character(None, kind=nvl)`: another narrator.
            self.narrators.insert(var.to_owned());
            return;
        }
        let shown = name.clone().unwrap_or_else(|| var.to_owned());
        let id = self.new_character(&shown, color);
        self.char_by_var.insert(var.to_owned(), id);
        let (before, after) = (format!("{prefix}{open}"), format!("{close}{suffix}"));
        if !before.is_empty() || !after.is_empty() {
            self.styles.insert(id, (before, after));
        }
        let tag = tag.unwrap_or_else(|| {
            // A sprite tag named like the variable or the character.
            let lower = image_name(&shown);
            if self.images.keys().any(|k| k.split(' ').next() == Some(var)) {
                var.to_owned()
            } else {
                lower.split(' ').next().unwrap_or("").to_owned()
            }
        });
        if !tag.is_empty() {
            self.char_by_tag.entry(tag.clone()).or_insert(id);
            self.tag_of.insert(id, tag);
        }
    }

    fn new_character(&mut self, name: &str, color: Color) -> CharacterId {
        let id = CharacterId::new();
        let name_id = self.add_text_raw(name, 0);
        self.story.characters.push(Character {
            id,
            name_id,
            color,
            expressions: Vec::new(),
            default_position: None,
            default_scale: 1.0,
        });
        id
    }

    fn nvl(&mut self, line: usize) {
        self.note(
            "nvl",
            line,
            false,
            "NVL mode (NVL characters, `nvl clear`): the engine has no NVL page, these lines are shown in the normal text box",
        );
    }

    fn image_statement(&mut self, line: &Line, toks: &[Tok]) {
        let Some(eq) = toks.iter().position(|t| t.is_op("=")) else {
            let end = toks.iter().position(|t| t.is_op(":")).unwrap_or(toks.len());
            let name: Vec<&str> = toks[1..end].iter().filter_map(Tok::ident).collect();
            let name = image_name(&name.join(" "));
            self.image_frames(line, name);
            return;
        };
        let name: Vec<&str> = toks[1..eq].iter().filter_map(Tok::ident).collect();
        let name = image_name(&name.join(" "));
        let value = &toks[eq + 1..];
        match self.displayable(line, &name, value) {
            Some(path) => {
                self.images.insert(name, path);
            }
            None => self.unsupported(line, "image defined by an expression not converted"),
        }
    }

    /// The picture of a displayable expression (image table path), with a
    /// warning for what is lost. `None`: nothing usable.
    fn displayable(&mut self, line: &Line, name: &str, value: &[Tok]) -> Option<String> {
        if let Some(Value::Str(s)) = parse_literal(value) {
            if s.starts_with('#') {
                return self.solid(line, name, &s);
            }
            let found = self.resolve_picture(&s);
            if found.is_none() {
                self.warn(
                    line.line,
                    format!("image `{name}`: `{s}` not found (or not PNG/JPEG)"),
                );
            }
            return found;
        }
        let call = value.first().and_then(Tok::ident)?;
        if !value.get(1).is_some_and(|t| t.is_op("(")) {
            // `image a = b` where b is another image name... not a literal.
            return None;
        }
        let close = matching(value, 1)?;
        let args = split_args(&value[2..close]);
        let strings: Vec<String> = args.iter().filter_map(|a| str_arg(a)).collect();
        match call {
            "Solid" => {
                let c = strings.first()?;
                self.solid(line, name, c)
            }
            "Image" | "im.Image" | "im.Scale" | "im.FactorScale" => {
                self.resolve_picture(strings.first()?)
            }
            "Composite" | "LiveComposite" | "im.Composite" => {
                let base = strings.iter().find_map(|s| self.resolve_picture(s))?;
                self.warn(
                    line.line,
                    format!(
                        "image `{name}`: `{call}` layers are not composited, its first layer stands for it ({} layer(s) dropped)",
                        strings.len().saturating_sub(1)
                    ),
                );
                Some(base)
            }
            "ConditionSwitch" => {
                // ("cond", displayable, ...): the "True" branch, else the last.
                let pairs: Vec<(&[Tok], &[Tok])> = args
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| (c[0], c[1]))
                    .collect();
                let pick = pairs
                    .iter()
                    .find(|(c, _)| str_arg(c).as_deref() == Some("True"))
                    .or(pairs.last())?;
                let path = self.displayable(line, name, pick.1)?;
                self.warn(
                    line.line,
                    format!(
                        "image `{name}`: `ConditionSwitch` keeps its default picture ({} condition(s) not evaluated)",
                        pairs.len().saturating_sub(1)
                    ),
                );
                Some(path)
            }
            other if other.starts_with("im.") || other == "Transform" => {
                let path = self.displayable(line, name, args.first()?)?;
                self.warn(
                    line.line,
                    format!(
                        "image `{name}`: `{other}` effect not applied, the picture is used as is"
                    ),
                );
                Some(path)
            }
            _ => None,
        }
    }

    /// `Solid("#rrggbb[aa]")`: a generated PNG.
    fn solid(&mut self, line: &Line, name: &str, color: &str) -> Option<String> {
        let Some(rgba) = parse_rgba(color) else {
            self.warn(
                line.line,
                format!("image `{name}`: color `{color}` not understood"),
            );
            return None;
        };
        let hex: String = rgba.iter().map(|b| format!("{b:02x}")).collect();
        let path = format!("assets/renpy_solid_{hex}.png");
        if !self.generated.iter().any(|(p, _)| *p == path) {
            let bytes = if rgba[3] == 255 {
                crate::png::solid_png(16, 9, [rgba[0], rgba[1], rgba[2]])
            } else {
                crate::png::solid_png_rgba(16, 9, rgba)
            };
            self.generated.push((path.clone(), bytes));
        }
        Some(format!("{GENERATED}{path}"))
    }

    /// `image name:` with an ATL block: a frame animation when it is only
    /// pictures, pauses and `repeat`; else its first picture.
    fn image_frames(&mut self, line: &Line, name: String) {
        let mut paths = Vec::new();
        let mut pauses = Vec::new();
        let mut looping = false;
        let mut other = Vec::new();
        for c in &line.children {
            let t = tokenize(&c.text);
            match t.as_slice() {
                [Tok::Str(s)] => match self.resolve_picture(s) {
                    Some(p) => paths.push(p),
                    None => other.push(format!("`{s}` not found")),
                },
                [Tok::Ident(p), Tok::Num(n)] if p == "pause" => {
                    pauses.push(n.parse::<f32>().unwrap_or(0.1));
                }
                [Tok::Ident(r), ..] if r == "repeat" => looping = true,
                [Tok::Ident(w), ..] => other.push(w.clone()),
                _ => other.push(c.text.chars().take(20).collect()),
            }
        }
        let Some(first) = paths.first().cloned() else {
            self.unsupported(line, "image with an ATL block (no picture found)");
            return;
        };
        if !other.is_empty() {
            self.warn(
                line.line,
                format!(
                    "image `{name}`: ATL not converted ({}), the first picture stands for it",
                    other.join(", ")
                ),
            );
        } else if paths.len() > 1 {
            let mean = pauses.iter().sum::<f32>() / pauses.len().max(1) as f32;
            let fps = if mean > 0.0 { 1.0 / mean } else { 10.0 };
            if pauses.iter().any(|p| (p - mean).abs() > 1e-3) {
                self.warn(
                    line.line,
                    format!("image `{name}`: uneven pauses played at a steady {fps:.1} frames per second"),
                );
            }
            self.frames.insert(
                name.clone(),
                Frames {
                    paths,
                    fps,
                    looping,
                },
            );
        }
        self.images.insert(name, first);
    }

    /// A media file of the game by a path written in a script, tried under
    /// each prefix.
    fn find_media(&self, path: &str, prefixes: &[&str]) -> Option<String> {
        let path = path.trim_start_matches("./").replace('\\', "/");
        prefixes.iter().find_map(|p| {
            let want = format!("{p}{path}").to_lowercase();
            self.media
                .iter()
                .find(|(m, _)| m.to_lowercase() == want)
                .map(|(m, _)| m.clone())
        })
    }

    // ------------------------------------------------------------ labels

    fn collect_labels(&mut self, tree: &[Line]) {
        for (index, line) in tree.iter().enumerate() {
            let toks = tokenize(&line.text);
            if !toks.first().is_some_and(|t| t.is_word("label")) {
                continue;
            }
            let Some(raw) = toks.get(1).and_then(Tok::ident) else {
                continue;
            };
            let local = raw.starts_with('.')
                || raw
                    .split_once('.')
                    .is_some_and(|(g, _)| !self.global.is_empty() && g == self.global);
            if local && !self.scenes.is_empty() {
                let full = self.full_name(raw);
                let scene = self.scenes.len() - 1;
                self.labels.insert(
                    full,
                    LabelRef {
                        scene,
                        node: NodeId::new(),
                        entry: false,
                    },
                );
                self.scenes[scene].blocks.push(index);
            } else {
                self.global = raw.to_owned();
                let scene = self.scenes.len();
                self.scenes.push(SceneSrc {
                    name: raw.to_owned(),
                    id: SceneId::new(),
                    file: self.file,
                    blocks: vec![index],
                });
                self.labels.insert(
                    raw.to_owned(),
                    LabelRef {
                        scene,
                        node: NodeId::new(),
                        entry: true,
                    },
                );
            }
            self.scan_body(&line.children, self.scenes.len() - 1);
        }
        self.global.clear();
    }

    /// Nested labels, `menu name:` labels, calls and scene tags of a body.
    fn scan_body(&mut self, lines: &[Line], scene: usize) {
        for line in lines {
            let toks = tokenize(&line.text);
            match toks.first().and_then(Tok::ident) {
                Some("label") | Some("menu") => {
                    if let Some(raw) = toks.get(1).and_then(Tok::ident) {
                        let full = self.full_name(raw);
                        self.labels.insert(
                            full,
                            LabelRef {
                                scene,
                                node: NodeId::new(),
                                entry: false,
                            },
                        );
                    }
                }
                Some("call") => {
                    if let Some(raw) = toks.get(1).and_then(Tok::ident)
                        && raw != "screen"
                        && raw != "expression"
                    {
                        let full = self.full_name(raw);
                        self.called.insert(full);
                    }
                }
                Some("scene") => {
                    if let Some(tag) = toks.get(1).and_then(Tok::ident) {
                        self.scene_tags.insert(tag.to_lowercase());
                    }
                }
                _ => {}
            }
            self.scan_body(&line.children, scene);
        }
    }

    /// `.local` -> `global.local`.
    fn full_name(&self, raw: &str) -> String {
        if raw.starts_with('.') {
            format!("{}{raw}", self.global)
        } else {
            raw.to_owned()
        }
    }

    // ------------------------------------------------------------ scenes

    fn convert_scene(&mut self, s: usize) {
        self.file = self.scenes[s].file;
        self.global = self.scenes[s].name.clone();
        self.cur.clear();
        self.run.clear();
        self.pending_voice = None;
        self.counter = 0;
        // A label reached by `call` returns; any other ends the game.
        self.scene_called = self.called.contains(&self.global)
            || self
                .labels
                .iter()
                .any(|(n, l)| l.scene == s && self.called.contains(n));
        let tree = std::mem::take(&mut self.trees[self.file]);
        let blocks = self.scenes[s].blocks.clone();
        for index in blocks {
            let line = &tree[index];
            let toks = tokenize(&line.text);
            let raw = toks.get(1).and_then(Tok::ident).unwrap_or("");
            self.label_node(&self.full_name(raw), line);
            self.block(&line.children);
        }
        self.trees[self.file] = tree;

        // Fall through into the next label of the same file.
        if !self.cur.last().is_some_and(|n| terminal(&n.kind)) {
            let next = self
                .scenes
                .iter()
                .enumerate()
                .find(|(i, n)| n.file == self.file && *i != s && self.follows(s, *i));
            let kind = match next {
                Some((_, n)) => NodeKind::JumpScene {
                    scene: n.id,
                    label: None,
                },
                None if self.scene_called => NodeKind::Return,
                None => NodeKind::End,
            };
            self.push(kind);
        }
        let scene = Scene {
            id: self.scenes[s].id,
            name: self.scenes[s].name.clone(),
            nodes: std::mem::take(&mut self.cur),
        };
        self.story.scenes.push(scene);
    }

    /// Whether scene `b` directly follows scene `a` in their file.
    fn follows(&self, a: usize, b: usize) -> bool {
        let last_a = *self.scenes[a].blocks.last().unwrap_or(&0);
        let first_b = self.scenes[b].blocks[0];
        first_b > last_a
            && !self.scenes.iter().any(|o| {
                o.file == self.scenes[a].file && o.blocks[0] > last_a && o.blocks[0] < first_b
            })
    }

    fn label_node(&mut self, full: &str, line: &Line) {
        let toks = tokenize(&line.text);
        if toks.get(2).is_some_and(|t| t.is_op("(")) {
            self.warn(line.line, format!("label `{full}`: parameters are ignored"));
        }
        let Some(id) = self.labels.get(full).map(|l| l.node) else {
            return;
        };
        self.cur.push(Node {
            id,
            kind: NodeKind::Label {
                name: full.to_owned(),
            },
        });
        self.run.clear();
    }

    fn fresh_label(&mut self, what: &str) -> NodeId {
        self.counter += 1;
        let _ = what;
        NodeId::new()
    }

    fn push(&mut self, kind: NodeKind) {
        let visual = matches!(
            kind,
            NodeKind::Background { .. } | NodeKind::Show { .. } | NodeKind::Hide { .. }
        );
        // The animation of a `show ... at`: part of the same visual run.
        let neutral = matches!(kind, NodeKind::Timeline { .. });
        if visual {
            self.run.push(self.cur.len());
        } else if !neutral {
            self.run.clear();
        }
        if !matches!(kind, NodeKind::Say { .. } | NodeKind::Timeline { .. }) {
            self.last_say = None;
        }
        self.cur.push(node(kind));
    }

    fn push_label(&mut self, id: NodeId, name: String) {
        self.cur.push(Node {
            id,
            kind: NodeKind::Label { name },
        });
        self.run.clear();
    }

    fn block(&mut self, lines: &[Line]) {
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];
            let toks = tokenize(&line.text);
            if toks.first().is_some_and(|t| t.is_word("if")) {
                let mut end = i + 1;
                while end < lines.len() {
                    let t = tokenize(&lines[end].text);
                    if t.first()
                        .is_some_and(|t| t.is_word("elif") || t.is_word("else"))
                    {
                        end += 1;
                    } else {
                        break;
                    }
                }
                self.if_chain(&lines[i..end]);
                i = end;
                continue;
            }
            self.statement(line, &toks);
            i += 1;
        }
    }

    fn statement(&mut self, line: &Line, toks: &[Tok]) {
        let first = match toks.first() {
            Some(Tok::Ident(w)) => w.as_str(),
            Some(Tok::Op(o)) if o == "$" => {
                self.python(line, &line.text[1..]);
                return;
            }
            Some(Tok::Str(_)) => {
                self.say(line, toks);
                return;
            }
            _ => {
                self.unsupported(line, "statement");
                return;
            }
        };
        match first {
            "label" => {
                let raw = toks.get(1).and_then(Tok::ident).unwrap_or("");
                let full = self.full_name(raw);
                self.label_node(&full, line);
                self.block(&line.children);
            }
            "jump" => self.jump(line, toks),
            "call" => self.call(line, toks),
            "return" => self.push(if self.scene_called {
                NodeKind::Return
            } else {
                NodeKind::End
            }),
            "menu" => self.menu(line, toks),
            "while" => self.while_loop(line, toks),
            "elif" | "else" => self.unsupported(line, "elif / else without if"),
            "show" => self.show(line, toks),
            "hide" => self.hide(line, toks),
            "scene" => self.scene(line, toks),
            "with" => self.with(line, &toks[1..]),
            "play" | "queue" => self.play(line, toks),
            "stop" => self.stop(line, toks),
            "voice" => {
                if let Some(t) = toks.get(1)
                    && !t.is_word("sustain")
                {
                    self.pending_voice = self.audio(line, t, AssetKind::Sound);
                }
            }
            "pause" => {
                let seconds = toks.get(1).and_then(num);
                if seconds.is_none() {
                    self.warn(
                        line.line,
                        "`pause` without a duration waits 1 second (no click-to-continue pause)",
                    );
                }
                self.push(NodeKind::Wait {
                    seconds: seconds.unwrap_or(1.0),
                });
            }
            "pass" => {}
            "window" => self.note(
                "window",
                line.line,
                false,
                "`window show / hide / auto` ignored (the engine shows the text box while a line is displayed)",
            ),
            "nvl" => self.nvl(line.line),
            "python" => self.python_block(line),
            "init" => self.script_comment(line, "python block"),
            "define" | "default" => self.definition(line, toks, first == "default"),
            "image" => self.image_statement(line, toks),
            "camera" | "transform" | "screen" | "style" | "layeredimage" | "translate" => {
                self.unsupported(line, "statement")
            }
            _ => self.say(line, toks),
        }
    }

    // ------------------------------------------------------------ dialogue

    fn say(&mut self, line: &Line, toks: &[Tok]) {
        let Some(si) = toks.iter().position(|t| matches!(t, Tok::Str(_))) else {
            self.unsupported(line, "statement");
            return;
        };
        let head = &toks[..si];
        if head.iter().any(|t| !matches!(t, Tok::Ident(_))) {
            self.unsupported(line, "statement");
            return;
        }
        let mut text_at = si;
        let mut speaker = None;
        // "Name" "text": a one-off speaker.
        if si == 0 && matches!(toks.get(1), Some(Tok::Str(_))) {
            if let Some(Tok::Str(name)) = toks.first() {
                let name = name.clone();
                let id = match self
                    .story
                    .characters
                    .iter()
                    .find(|c| self.story.strings.get(c.name_id) == Some(name.as_str()))
                {
                    Some(c) => c.id,
                    None => self.new_character(&name, Color::WHITE),
                };
                speaker = Some(id);
            }
            text_at = 1;
        } else if let Some(Tok::Ident(who)) = head.first() {
            if who == "extend"
                && let Some(Tok::Str(raw)) = toks.get(si)
            {
                let raw = raw.clone();
                self.extend(line, &raw);
                if let Some(w) = toks.iter().position(|t| t.is_word("with")) {
                    self.with(line, &toks[w + 1..]);
                }
                return;
            }
            if self.narrators.contains(who) {
                if !matches!(who.as_str(), "narrator" | "centered" | "extend") {
                    self.nvl(line.line);
                }
            } else {
                match self.char_by_var.get(who).copied() {
                    Some(id) => speaker = Some(id),
                    None => {
                        self.warn(
                            line.line,
                            format!("unknown speaker `{who}`: a character is created"),
                        );
                        let id = self.new_character(who, Color::WHITE);
                        self.char_by_var.insert(who.clone(), id);
                        speaker = Some(id);
                    }
                }
            }
            // `e happy "..."`: the sprite changes expression.
            let attrs: Vec<&str> = head[1..].iter().filter_map(Tok::ident).collect();
            if let (Some(id), false) = (speaker, attrs.is_empty()) {
                self.change_expression(line, id, &attrs);
            }
        }
        let Some(Tok::Str(raw)) = toks.get(text_at) else {
            return;
        };
        let raw = raw.clone();
        let core = self.convert_text(&raw, line.line);
        let no_wait = raw.trim_end().ends_with("{nw}");
        let text_id = self.add_styled(&raw, &core, speaker);
        let voice = self.pending_voice.take();
        self.push(NodeKind::Say {
            speaker,
            text_id,
            voice,
        });
        self.last_say = Some(LastSay {
            text_id,
            speaker,
            core,
            raw,
            no_wait,
        });
        // `"..." with vpunch`
        if let Some(w) = toks.iter().position(|t| t.is_word("with")) {
            self.with(line, &toks[w + 1..]);
        }
    }

    /// `extend "..."`: Ren'Py adds the text to the line on screen. After a
    /// `{nw}` line the two become one line; otherwise a new line repeats
    /// the previous text followed by the extension (same box content).
    fn extend(&mut self, line: &Line, raw: &str) {
        let ext = self.convert_text(raw, line.line);
        let Some(prev) = self.last_say.take() else {
            self.warn(
                line.line,
                "`extend` without a previous line: shown as narration",
            );
            let text_id = self.add_styled(raw, &ext, None);
            let voice = self.pending_voice.take();
            self.push(NodeKind::Say {
                speaker: None,
                text_id,
                voice,
            });
            return;
        };
        let core = format!("{}{ext}", prev.core);
        let joined_raw = format!("{}{raw}", prev.raw);
        let no_wait = raw.trim_end().ends_with("{nw}");
        if prev.no_wait {
            // Rewrite the previous line in place.
            let (before, after) = prev
                .speaker
                .and_then(|s| self.styles.get(&s).cloned())
                .unwrap_or_default();
            self.story
                .strings
                .set(prev.text_id, format!("{before}{core}{after}"));
            self.text_index
                .entry(joined_raw.clone())
                .or_default()
                .push(prev.text_id);
            self.last_say = Some(LastSay {
                core,
                raw: joined_raw,
                no_wait,
                ..prev
            });
            return;
        }
        let text_id = self.add_styled(&joined_raw, &core, prev.speaker);
        let voice = self.pending_voice.take();
        self.push(NodeKind::Say {
            speaker: prev.speaker,
            text_id,
            voice,
        });
        self.last_say = Some(LastSay {
            text_id,
            speaker: prev.speaker,
            core,
            raw: joined_raw,
            no_wait,
        });
    }

    fn change_expression(&mut self, line: &Line, id: CharacterId, attrs: &[&str]) {
        let Some(tag) = self.tag_of.get(&id).cloned() else {
            return;
        };
        let Some((_, position)) = self.stage.get(&id).cloned() else {
            return;
        };
        if let Some(expression) = self.expression(line, &tag, attrs, id) {
            self.stage
                .insert(id, (expression.clone(), position.clone()));
            self.push(NodeKind::Show {
                character: id,
                expression,
                position,
                transition: None,
                scale: None,
                scale_y: None,
                flip_x: false,
                flip_y: false,
            });
        }
    }

    /// Adds a source string (Ren'Py text converted to the engine markup).
    fn add_text_raw(&mut self, raw: &str, line: usize) -> StringId {
        let converted = if line > 0 {
            self.convert_text(raw, line)
        } else {
            text::convert(raw, &mut TextNotes::default())
        };
        let id = self.story.strings.add(converted);
        self.text_index.entry(raw.to_owned()).or_default().push(id);
        id
    }

    /// Ren'Py text to the engine markup; what is lost is reported (once per
    /// tag / variable, with a count).
    fn convert_text(&mut self, raw: &str, line: usize) -> String {
        let mut notes = TextNotes::default();
        let converted = text::convert(raw, &mut notes);
        for var in notes.interpolations {
            self.note(
                &format!("interp:{var}"),
                line,
                false,
                format!("`[{var}]` interpolation is kept as plain text (not supported)"),
            );
        }
        for tag in notes.dropped_tags {
            self.note(
                &format!("tag:{tag}"),
                line,
                false,
                format!("text tag `{{{tag}}}` has no equivalent and was removed"),
            );
        }
        converted
    }

    /// A line of `speaker`: its style (`what_prefix`, `what_color`...)
    /// around the converted text.
    fn add_styled(&mut self, raw: &str, core: &str, speaker: Option<CharacterId>) -> StringId {
        let style = speaker.and_then(|s| self.styles.get(&s).cloned());
        let text = match &style {
            Some((before, after)) => format!("{before}{core}{after}"),
            None => core.to_owned(),
        };
        let id = self.story.strings.add(text);
        self.text_index.entry(raw.to_owned()).or_default().push(id);
        if let Some(w) = style {
            self.wrappers.insert(id, w);
        }
        id
    }

    // ------------------------------------------------------------ flow

    fn resolve(&self, raw: &str) -> Option<&LabelRef> {
        self.labels.get(&self.full_name(raw))
    }

    fn jump_kind(&self, target: &LabelRef) -> NodeKind {
        let here = self.scene_index();
        if target.scene == here {
            NodeKind::Jump {
                target: target.node,
            }
        } else {
            NodeKind::JumpScene {
                scene: self.scenes[target.scene].id,
                label: (!target.entry).then_some(target.node),
            }
        }
    }

    fn scene_index(&self) -> usize {
        self.scenes
            .iter()
            .position(|s| s.name == self.global)
            .unwrap_or(0)
    }

    fn jump(&mut self, line: &Line, toks: &[Tok]) {
        match toks.get(1).and_then(Tok::ident) {
            Some("expression") | None => self.unsupported(line, "computed jump"),
            Some(raw) => match self.resolve(raw) {
                Some(t) => {
                    let kind = self.jump_kind(t);
                    self.push(kind);
                }
                None => self.unsupported(line, "jump to an unknown label"),
            },
        }
    }

    fn call(&mut self, line: &Line, toks: &[Tok]) {
        match toks.get(1).and_then(Tok::ident) {
            Some("screen") => {
                let name = toks.get(2).and_then(Tok::ident).unwrap_or("").to_owned();
                self.navigation(line, &name, false);
            }
            Some("expression") | None => self.unsupported(line, "computed call"),
            Some(raw) => {
                if toks.get(2).is_some_and(|t| t.is_op("(")) {
                    self.warn(line.line, "call arguments are ignored");
                }
                match self.resolve(raw).map(|t| t.node) {
                    Some(label) => self.push(NodeKind::Call { label }),
                    None => self.unsupported(line, "call of an unknown label"),
                }
            }
        }
    }

    /// A Ren'Py condition: an engine expression, else Rhai.
    fn condition(&mut self, line: &Line, toks: &[Tok]) -> Expr {
        // `persistent.x` never declared: Ren'Py reads None (false).
        for t in toks {
            if let Tok::Ident(n) = t
                && n.starts_with("persistent.")
            {
                let var = var_name(n);
                if self.vars.insert(var.clone()) {
                    self.warn(
                        line.line,
                        format!("`{n}` is never declared: declared as a persistent flag (false)"),
                    );
                    self.story
                        .variables
                        .push(VarDecl::persistent(var, Value::Bool(false)));
                }
            }
        }
        if let Some(e) = parse_expr_toks(toks) {
            return e;
        }
        let vars = self.vars.clone();
        let code = to_rhai_toks(toks, &|n| vars.contains(n));
        self.warn(
            line.line,
            format!("condition converted to Rhai, check it: `{code}`"),
        );
        Expr::Rhai(code)
    }

    fn if_chain(&mut self, lines: &[Line]) {
        let end = self.fresh_label("if_end");
        let mut branches = Vec::new();
        let mut else_body = None;
        for line in lines {
            let toks = tokenize(&line.text);
            let body = toks.len().saturating_sub(1);
            if toks.first().is_some_and(|t| t.is_word("else")) {
                else_body = Some(line);
                continue;
            }
            let condition = self.condition(line, &toks[1..body]);
            let target = self.fresh_label("if");
            self.push(NodeKind::If {
                condition,
                then_target: target,
                else_target: None,
            });
            branches.push((target, line));
        }
        if let Some(line) = else_body {
            self.block(&line.children);
        }
        self.jump_to(end);
        let n = self.counter;
        for (i, (target, line)) in branches.into_iter().enumerate() {
            self.push_label(target, format!("if_{n}_{}", i + 1));
            self.block(&line.children);
            self.jump_to(end);
        }
        self.push_label(end, format!("if_{n}_end"));
    }

    /// A jump to `target` unless the previous node already leaves.
    fn jump_to(&mut self, target: NodeId) {
        if !self.cur.last().is_some_and(|n| terminal(&n.kind)) {
            self.push(NodeKind::Jump { target });
        }
    }

    fn while_loop(&mut self, line: &Line, toks: &[Tok]) {
        let top = self.fresh_label("while");
        let body = NodeId::new();
        let end = NodeId::new();
        let n = self.counter;
        self.push_label(top, format!("while_{n}"));
        let condition = self.condition(line, &toks[1..toks.len().saturating_sub(1)]);
        self.push(NodeKind::If {
            condition,
            then_target: body,
            else_target: None,
        });
        self.push(NodeKind::Jump { target: end });
        self.push_label(body, format!("while_{n}_body"));
        self.block(&line.children);
        self.jump_to(top);
        self.push_label(end, format!("while_{n}_end"));
    }

    fn menu(&mut self, line: &Line, toks: &[Tok]) {
        if let Some(raw) = toks.get(1).and_then(Tok::ident) {
            let full = self.full_name(raw);
            if let Some(id) = self.labels.get(&full).map(|l| l.node) {
                self.push_label(id, full);
            }
        }
        let end = self.fresh_label("menu_end");
        let n = self.counter;
        let mut options = Vec::new();
        let mut bodies = Vec::new();
        for child in &line.children {
            let t = tokenize(&child.text);
            let is_option = child.text.ends_with(':') && matches!(t.first(), Some(Tok::Str(_)));
            if !is_option {
                match t.first() {
                    Some(Tok::Ident(w)) if w == "set" => {
                        self.warn(child.line, "menu `set` is ignored")
                    }
                    _ => self.say(child, &t),
                }
                continue;
            }
            let Some(Tok::Str(raw)) = t.first() else {
                continue;
            };
            let raw = raw.clone();
            let cond_at = t.iter().position(|t| t.is_word("if"));
            let condition = cond_at.map(|c| {
                let toks = &t[c + 1..t.len() - 1];
                self.condition(child, toks)
            });
            let text_id = self.add_text_raw(&raw, child.line);
            let target = NodeId::new();
            options.push(ChoiceOption {
                text_id,
                target,
                condition,
            });
            bodies.push((target, child));
        }
        if options.is_empty() {
            self.unsupported(line, "menu without choices");
            return;
        }
        self.push(NodeKind::Choice { options });
        for (i, (target, child)) in bodies.into_iter().enumerate() {
            self.push_label(target, format!("menu_{n}_{}", i + 1));
            self.block(&child.children);
            self.jump_to(end);
        }
        self.push_label(end, format!("menu_{n}_end"));
    }

    // ------------------------------------------------------------ python

    /// Declares `var` (engine name of the Ren'Py `raw` name) on its first
    /// assignment when no `default` did: typed by the assigned literal.
    fn declare_assigned(&mut self, line: usize, var: &str, raw: &str, literal: Option<Value>) {
        if self.vars.contains(var) {
            return;
        }
        let Some(v) = literal else {
            self.warn(
                line,
                format!("variable `{var}` is never declared (`default`)"),
            );
            return;
        };
        self.vars.insert(var.to_owned());
        self.story
            .variables
            .push(if raw.starts_with("persistent.") {
                // Ren'Py persistent fields start as None: the zero of the type.
                VarDecl::persistent(var.to_owned(), zero_like(&v))
            } else {
                VarDecl::game(var.to_owned(), zero_like(&v))
            });
    }

    /// A `python:` block in a label: each simple line is converted, the
    /// rest is kept as one commented script.
    fn python_block(&mut self, line: &Line) {
        let mut rest = Vec::new();
        for c in &line.children {
            if !c.children.is_empty() || !self.python_known(c, &c.text) {
                rest.push(c.clone());
            }
        }
        if !rest.is_empty() {
            let block = Line {
                children: rest,
                ..line.clone()
            };
            self.script_comment(&block, "python block");
        }
    }

    fn python(&mut self, line: &Line, code: &str) {
        if !self.python_known(line, code) {
            self.script_comment(line, "Python statement");
        }
    }

    /// Converts one line of Python; `false` when it does not map.
    fn python_known(&mut self, line: &Line, code: &str) -> bool {
        let toks = tokenize(code);
        let first = toks.first().and_then(Tok::ident).unwrap_or("");
        let is_call = toks.get(1).is_some_and(|t| t.is_op("("));
        if is_call && self.python_call(line, first, &toks) {
            return true;
        }
        if let [Tok::Ident(target), Tok::Op(op), rest @ ..] = toks.as_slice() {
            let var = var_name(target);
            let value = match op.as_str() {
                "=" => parse_expr_toks(rest),
                "+=" | "-=" | "*=" | "/=" => parse_expr_toks(rest).map(|r| {
                    let bin = match op.as_str() {
                        "+=" => BinOp::Add,
                        "-=" => BinOp::Sub,
                        "*=" => BinOp::Mul,
                        _ => BinOp::Div,
                    };
                    Expr::binary(bin, Expr::var(var.clone()), r)
                }),
                _ => None,
            };
            if let Some(value) = value {
                // `$ x = 0` without `default`: declare it.
                let literal = parse_literal(rest).filter(|_| op == "=");
                self.declare_assigned(line.line, &var, target, literal);
                self.push(NodeKind::SetVar { var, value });
                return true;
            }
        }
        // list.append(x) / list.remove(x)
        if let [Tok::Ident(call), Tok::Op(open), args @ .., Tok::Op(close)] = toks.as_slice()
            && open == "("
            && close == ")"
            && let Some((list, method)) = call.rsplit_once('.')
            && matches!(method, "append" | "remove")
            && let Some(arg) = parse_expr_toks(args)
        {
            let var = var_name(list);
            let func = if method == "append" {
                orbit_core::Func::Push
            } else {
                orbit_core::Func::Remove
            };
            let value = Expr::call(func, vec![Expr::var(var.clone()), arg]);
            self.push(NodeKind::SetVar { var, value });
            return true;
        }
        false
    }

    /// `renpy.*` / `achievement.*` calls that have an equivalent.
    fn python_call(&mut self, line: &Line, name: &str, toks: &[Tok]) -> bool {
        let (args, kwargs) = call_args(toks);
        let kw = |k: &str| kwargs.iter().find(|(n, _)| n == k).map(|(_, v)| *v);
        let kw_num = |k: &str| kw(k).and_then(|v| v.first()).and_then(num);
        let kw_str = |k: &str| kw(k).and_then(str_arg);
        match name {
            "renpy.pause" => {
                let seconds = args
                    .first()
                    .and_then(|a| a.first())
                    .and_then(num)
                    .or_else(|| kw_num("delay"));
                if seconds.is_none() {
                    self.warn(
                        line.line,
                        "`renpy.pause()` without a duration waits 1 second (no click-to-continue pause)",
                    );
                }
                self.push(NodeKind::Wait {
                    seconds: seconds.unwrap_or(1.0),
                });
            }
            "renpy.music.play" | "renpy.music.queue" | "renpy.sound.play" | "renpy.sound.queue"
            | "renpy.play" => {
                let default = if name.starts_with("renpy.music") {
                    "music"
                } else {
                    "sound"
                };
                let channel = kw_str("channel").unwrap_or_else(|| default.to_owned());
                let Some(file) = args.first() else {
                    return false;
                };
                // A list: its first file.
                let file_tok = match file {
                    [Tok::Op(o), first, ..] if o == "[" => first.clone(),
                    [t] => t.clone(),
                    _ => return false,
                };
                if channel == "music" {
                    let Some(asset) = self.audio(line, &file_tok, AssetKind::Music) else {
                        return true;
                    };
                    let looping =
                        !kw("loop").is_some_and(|v| v.first().is_some_and(|t| t.is_word("False")));
                    self.push(NodeKind::PlayMusic {
                        asset,
                        fade_in: kw_num("fadein").unwrap_or(0.0),
                        looping,
                    });
                } else if channel == "voice" {
                    self.pending_voice = self.audio(line, &file_tok, AssetKind::Sound);
                } else if let Some(asset) = self.audio(line, &file_tok, AssetKind::Sound) {
                    self.push(NodeKind::PlaySound { asset });
                }
            }
            "renpy.music.stop" => {
                let channel = kw_str("channel").unwrap_or_else(|| "music".to_owned());
                if channel != "music" {
                    return false;
                }
                self.push(NodeKind::StopMusic {
                    fade_out: kw_num("fadeout").unwrap_or(0.0),
                });
            }
            "renpy.sound.stop" => self.note(
                "sound.stop",
                line.line,
                false,
                "stopping sounds is ignored (sounds end by themselves)",
            ),
            "renpy.notify" => {
                let Some(raw) = args.first().and_then(|a| str_arg(a)) else {
                    return false;
                };
                self.push_notify(&raw, line.line);
            }
            "renpy.jump" | "renpy.call" => {
                let Some(label) = args.first().and_then(|a| str_arg(a)) else {
                    return false;
                };
                let t = [
                    Tok::Ident(name.trim_start_matches("renpy.").to_owned()),
                    Tok::Ident(label),
                ];
                if name == "renpy.jump" {
                    self.jump(line, &t);
                } else {
                    self.call(line, &t);
                }
            }
            "renpy.call_screen" | "renpy.show_screen" => {
                let Some(screen) = args.first().and_then(|a| str_arg(a)) else {
                    return false;
                };
                self.navigation(line, &screen, name == "renpy.show_screen");
            }
            "achievement.grant" => {
                let Some(id) = args.first().and_then(|a| str_arg(a)) else {
                    return false;
                };
                let achievement = self.achievement(&id);
                self.push(NodeKind::Unlock { achievement });
            }
            "achievement.register" => {
                let Some(id) = args.first().and_then(|a| str_arg(a)) else {
                    return false;
                };
                self.achievement(&id);
            }
            // Persistent data and achievements are saved by the engine.
            "renpy.save_persistent" | "achievement.sync" => {}
            "renpy.block_rollback" | "renpy.fix_rollback" | "renpy.checkpoint" => self.note(
                "rollback",
                line.line,
                false,
                "rollback control (`renpy.block_rollback`...) ignored",
            ),
            _ => return false,
        }
        true
    }

    /// A `Notify` node showing `raw` (Ren'Py text converted to the markup).
    fn push_notify(&mut self, raw: &str, line: usize) {
        let text_id = self.add_text_raw(raw, line);
        self.push(NodeKind::Notify(orbit_core::Notify {
            text_id,
            icon: None,
            sound: None,
            duration: None,
            style: None,
        }));
    }

    /// Python that does not map: kept as a commented Rhai script node so
    /// that nothing is lost, and reported.
    fn script_comment(&mut self, line: &Line, what: &str) {
        self.unsupported(line, what);
        let mut source = String::from("// Ren'Py code, not converted:\n");
        let mut push = |l: &Line, depth: usize, source: &mut String| {
            source.push_str(&format!("// {}{}\n", "    ".repeat(depth), l.text));
        };
        fn walk(
            l: &Line,
            depth: usize,
            source: &mut String,
            push: &mut dyn FnMut(&Line, usize, &mut String),
        ) {
            push(l, depth, source);
            for c in &l.children {
                walk(c, depth + 1, source, push);
            }
        }
        walk(line, 0, &mut source, &mut push);
        self.push(NodeKind::Script {
            source,
            branches: Vec::new(),
            on_error: None,
        });
    }

    // ------------------------------------------------------------ stage

    /// Words of an image statement up to the first keyword.
    fn image_words(toks: &[Tok]) -> (Vec<String>, usize) {
        let keywords = ["at", "with", "as", "behind", "zorder", "onlayer"];
        let mut words = Vec::new();
        let mut i = 1;
        while let Some(Tok::Ident(w)) = toks.get(i) {
            if keywords.contains(&w.as_str()) {
                break;
            }
            words.push(w.to_lowercase());
            i += 1;
        }
        (words, i)
    }

    fn inline_transition(&mut self, line: &Line, toks: &[Tok]) -> Option<Transition> {
        let at = toks.iter().position(|t| t.is_word("with"))?;
        let tail = &toks[at + 1..];
        match self.transition(line.line, tail) {
            Some(t) => t,
            None => {
                self.warn(line.line, "unknown transition replaced by a dissolve");
                Some(Transition::dissolve(0.5))
            }
        }
    }

    /// A transition expression: `dissolve`, `Dissolve(1.0)`, `fade`,
    /// `Fade(0.5, 0, 0.5)`, a `define`d one... `None` when it is not one;
    /// `Some(None)` for `with None`.
    fn transition(&mut self, line: usize, toks: &[Tok]) -> Option<Option<Transition>> {
        let name = toks.first()?.ident()?.to_owned();
        let (args, kwargs) = if toks.get(1).is_some_and(|t| t.is_op("(")) {
            call_args(toks)
        } else {
            (Vec::new(), Vec::new())
        };
        let arg = |i: usize| -> Option<f32> { args.get(i).and_then(|a| a.first()).and_then(num) };
        let kw = |k: &str| kwargs.iter().find(|(n, _)| n == k).map(|(_, v)| *v);
        let ease = kw("time_warp").and_then(|v| match v {
            [Tok::Ident(w)] => atl::warper(w.trim_start_matches("_warper.")),
            _ => None,
        });
        let approx = |this: &mut Self, what: &str, t: Transition| {
            this.note(
                &format!("transition:{what}"),
                line,
                false,
                format!("transition `{what}` approximated by a dissolve"),
            );
            Some(t)
        };
        let t = match name.as_str() {
            "None" => None,
            "dissolve" => Some(Transition::dissolve(0.5)),
            "Dissolve" => Some(Transition::dissolve(arg(0).unwrap_or(0.5))),
            "fade" => Some(Transition::fade(Color::BLACK, 1.0)),
            "Fade" => {
                let total = arg(0).unwrap_or(0.5) + arg(1).unwrap_or(0.0) + arg(2).unwrap_or(0.5);
                let color = kw("color")
                    .and_then(str_arg)
                    .and_then(|c| parse_color(&c))
                    .unwrap_or(Color::BLACK);
                Some(Transition::fade(color, total))
            }
            "Pixellate" | "ImageDissolve" | "AlphaDissolve" | "MoveTransition" | "CropMove"
            | "PushMove" | "ComposeTransition" | "MultipleTransition" | "SwingTransition"
            | "Swing" => {
                let d = arg(if name == "ImageDissolve" || name == "AlphaDissolve" {
                    1
                } else {
                    0
                });
                approx(self, &name, Transition::dissolve(d.unwrap_or(0.5)))
            }
            "pixellate" => approx(self, &name, Transition::dissolve(1.0)),
            other => {
                let lower = other.to_ascii_lowercase();
                if let Some(t) = self.transitions.get(other) {
                    t.clone()
                } else if [
                    "move", "ease", "wipe", "slide", "push", "iris", "zoom", "blinds", "squares",
                ]
                .iter()
                .any(|p| lower.starts_with(p))
                {
                    approx(self, other, Transition::dissolve(0.5))
                } else {
                    return None;
                }
            }
        };
        Some(t.map(|t| match ease {
            Some(e) => t.with_ease(e),
            None => t,
        }))
    }

    fn with(&mut self, line: &Line, tail: &[Tok]) {
        let name = tail.first().and_then(Tok::ident).unwrap_or("");
        if name == "Pause" {
            let seconds = call_args(tail)
                .0
                .first()
                .and_then(|a| a.first())
                .and_then(num);
            self.push(NodeKind::Wait {
                seconds: seconds.unwrap_or(1.0),
            });
            return;
        }
        if matches!(name, "vpunch" | "hpunch") {
            self.push(NodeKind::Effect(Effect::Shake {
                target: AnimTarget::Camera,
                amplitude: 0.02,
                duration: 0.5,
                ease: None,
                wait: true,
            }));
            return;
        }
        let transition = match self.transition(line.line, tail) {
            Some(t) => t,
            None => {
                self.warn(
                    line.line,
                    format!("transition `{name}` replaced by a dissolve"),
                );
                Some(Transition::dissolve(0.5))
            }
        };
        let run = std::mem::take(&mut self.run);
        for i in &run {
            match &mut self.cur[*i].kind {
                NodeKind::Background { transition: t, .. }
                | NodeKind::Show { transition: t, .. }
                | NodeKind::Hide { transition: t, .. }
                    if t.is_none() =>
                {
                    t.clone_from(&transition);
                }
                _ => {}
            }
        }
    }

    /// The transforms of `at a, b(...)` (and of an ATL block under the
    /// statement), combined. The name is set when the whole list is one
    /// user transform that only places the sprite (a stage position).
    fn at_list(&mut self, line: &Line, toks: &[Tok]) -> Option<(Atl, Option<String>)> {
        let mut out: Option<Atl> = None;
        let mut single_name = None;
        if let Some(at) = toks.iter().position(|t| t.is_word("at")) {
            let stop = ["with", "as", "behind", "zorder", "onlayer"];
            let end = toks[at + 1..]
                .iter()
                .position(|t| stop.iter().any(|w| t.is_word(w)))
                .map_or(toks.len(), |e| at + 1 + e);
            let items = split_args(&toks[at + 1..end]);
            let count = items.len();
            for item in items {
                let name = item.first().and_then(Tok::ident).unwrap_or("");
                let call = item.get(1).is_some_and(|t| t.is_op("("));
                let parsed = if call && matches!(name, "Transform" | "Position") {
                    let close = matching(item, 1).unwrap_or(item.len());
                    Some(atl::parse_kwargs(
                        &split_args(&item[2..close]),
                        self.screen_size,
                    ))
                } else if call || self.transform_params.contains(name) {
                    self.note(
                        &format!("transform-call:{name}"),
                        line.line,
                        true,
                        format!("transform `{name}(...)` with parameters not converted"),
                    );
                    None
                } else if let Some(t) = self.transforms.get(name) {
                    if count == 1 && t.is_position_only() {
                        single_name = Some(name.to_owned());
                    }
                    Some(t.clone())
                } else if let Some(t) = atl::builtin(name) {
                    Some(t)
                } else {
                    self.note(
                        &format!("transform:{name}"),
                        line.line,
                        true,
                        format!("unknown transform `{name}` (the position is kept)"),
                    );
                    None
                };
                if let Some(p) = parsed {
                    if let Some(o) = &mut out {
                        o.then(p);
                    } else {
                        out = Some(p);
                    }
                }
            }
        }
        if !line.children.is_empty() {
            let block = atl::parse_block(&line.children, self.screen_size);
            if !block.ignored.is_empty() {
                self.warn(
                    line.line,
                    format!("ATL: {} not converted", block.ignored.join(", ")),
                );
            }
            single_name = None;
            match &mut out {
                Some(o) => o.then(block),
                None => out = Some(block),
            }
        }
        out.map(|a| (a, single_name))
    }

    /// Normalized coordinates of a stage position.
    fn coords(&self, p: &Position) -> (f32, f32) {
        match p {
            Position::Free { x, y } => (*x, *y),
            Position::Custom(n) => self
                .story
                .positions
                .iter()
                .find(|d| d.name == *n)
                .map_or((0.5, 1.0), |d| (d.x, d.y)),
            other => orbit_core::defs::builtin_position(other).unwrap_or((0.5, 1.0)),
        }
    }

    /// The `Show` staging and the animation of a transform applied to the
    /// character `id` standing at `previous`.
    fn apply_atl(
        &mut self,
        id: CharacterId,
        atl: &Atl,
        custom: Option<String>,
        previous: &Position,
    ) -> (Position, Option<f32>, Option<f32>, bool, Option<NodeKind>) {
        let (bx, by) = self.coords(previous);
        let fin = |p: Prop, base: f32| atl.keys(p, base).map(|(_, v)| v);
        let (fx, fy) = (fin(Prop::X, bx), fin(Prop::Y, by));
        let (fxo, fyo) = (fin(Prop::XOff, 0.0), fin(Prop::YOff, 0.0));
        let moved = fx.is_some()
            || fy.is_some()
            || fxo.is_some_and(|v| v != 0.0)
            || fyo.is_some_and(|v| v != 0.0);
        let position = if let Some(name) = custom.filter(|_| fxo.is_none() && fyo.is_none()) {
            let (x, y) = (fx.unwrap_or(bx), fy.unwrap_or(by));
            if !self.story.positions.iter().any(|d| d.name == name) {
                self.story.positions.push(PositionDef {
                    name: name.clone(),
                    x: x.clamp(-2.0, 3.0),
                    y: y.clamp(-2.0, 3.0),
                });
            }
            Position::Custom(name)
        } else if moved {
            let x = (fx.unwrap_or(bx) + fxo.unwrap_or(0.0)).clamp(-2.0, 3.0);
            let y = (fy.unwrap_or(by) + fyo.unwrap_or(0.0)).clamp(-2.0, 3.0);
            let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
            match (x, y) {
                (x, y) if near(y, 1.0) && near(x, 0.25) => Position::Left,
                (x, y) if near(y, 1.0) && near(x, 0.5) => Position::Center,
                (x, y) if near(y, 1.0) && near(x, 0.75) => Position::Right,
                (x, y) => Position::Free { x, y },
            }
        } else {
            previous.clone()
        };
        let zoom = fin(Prop::Zoom, 1.0);
        let xzoom = fin(Prop::XZoom, 1.0);
        let yzoom = fin(Prop::YZoom, 1.0);
        let clamp = |s: f32| s.abs().clamp(orbit_core::SCALE_MIN, orbit_core::SCALE_MAX);
        let fz = clamp(zoom.unwrap_or(1.0));
        let (fxz, fyz) = (clamp(xzoom.unwrap_or(1.0)), clamp(yzoom.unwrap_or(1.0)));
        let (scale, scale_y) = if zoom.is_some() || xzoom.is_some() || yzoom.is_some() {
            let sx = clamp(fz * fxz);
            let sy = clamp(fz * fyz);
            (Some(sx), ((sy - sx).abs() > 1e-4).then_some(sy))
        } else {
            (None, None)
        };
        let flip = xzoom.is_some_and(|v| v < 0.0);

        // Tracks, relative to the final staging (a finished animation adds
        // nothing, except its final opacity and rotation).
        let target = AnimTarget::Character(id);
        let mut tracks = Vec::new();
        let vec_track = |keys: Vec<(f32, f32, Curve)>, f: &dyn Fn(f32) -> orbit_core::Vec2| {
            TrackData::Vec2(Track::new(
                keys.into_iter()
                    .map(|(t, v, c)| Key::with_ease(t, f(v), c))
                    .collect(),
            ))
        };
        let offsets: [(Prop, f32, f32, bool); 4] = [
            (Prop::X, bx, fx.unwrap_or(bx), true),
            (Prop::XOff, 0.0, fxo.unwrap_or(0.0), true),
            (Prop::Y, by, fy.unwrap_or(by), false),
            (Prop::YOff, 0.0, fyo.unwrap_or(0.0), false),
        ];
        for (prop, base, fin, horizontal) in offsets {
            if let Some((keys, _)) = atl.keys(prop, base).filter(|(k, _)| k.len() > 1) {
                let f = move |v: f32| {
                    if horizontal {
                        orbit_core::Vec2::new(v - fin, 0.0)
                    } else {
                        orbit_core::Vec2::new(0.0, v - fin)
                    }
                };
                tracks.push(StoryTrack {
                    target,
                    prop: AnimProp::Offset,
                    data: vec_track(keys, &f),
                });
            }
        }
        let scales: [(Prop, f32, bool, bool); 3] = [
            (Prop::Zoom, fz, true, true),
            (Prop::XZoom, fxz, true, false),
            (Prop::YZoom, fyz, false, true),
        ];
        for (prop, fin, sx, sy) in scales {
            if let Some((keys, _)) = atl.keys(prop, 1.0).filter(|(k, _)| k.len() > 1) {
                let f = move |v: f32| {
                    let r = v.abs() / fin;
                    orbit_core::Vec2::new(if sx { r } else { 1.0 }, if sy { r } else { 1.0 })
                };
                tracks.push(StoryTrack {
                    target,
                    prop: AnimProp::Scale,
                    data: vec_track(keys, &f),
                });
            }
        }
        if let Some((keys, last)) = atl.keys(Prop::Alpha, 1.0)
            && (keys.len() > 1 || last != 1.0)
        {
            tracks.push(StoryTrack {
                target,
                prop: AnimProp::Opacity,
                data: TrackData::Scalar(Track::new(
                    keys.into_iter()
                        .map(|(t, v, c)| Key::with_ease(t, v, c))
                        .collect(),
                )),
            });
        }
        if let Some((keys, last)) = atl.keys(Prop::Rotate, 0.0)
            && (keys.len() > 1 || last != 0.0)
        {
            tracks.push(StoryTrack {
                target,
                prop: AnimProp::Rotation,
                data: TrackData::Angle(Track::new(
                    keys.into_iter()
                        .map(|(t, v, c)| Key::with_ease(t, orbit_core::Angle(v), c))
                        .collect(),
                )),
            });
        }
        let timeline = (!tracks.is_empty()).then_some(NodeKind::Timeline {
            duration: atl.duration,
            looping: match atl.repeat {
                atl::Repeat::Once => Loop::Once,
                atl::Repeat::Times(n) => Loop::Loop(n),
                atl::Repeat::Forever if atl.duration > 0.0 => Loop::Forever,
                atl::Repeat::Forever => Loop::Once,
            },
            wait: false,
            tracks,
        });
        (position, scale, scale_y, flip, timeline)
    }

    fn is_background_tag(&self, tag: &str) -> bool {
        tag == "bg" || (self.scene_tags.contains(tag) && !self.char_by_tag.contains_key(tag))
    }

    fn character_for_tag(&mut self, tag: &str) -> CharacterId {
        if let Some(id) = self.char_by_tag.get(tag) {
            return *id;
        }
        let mut name = tag.to_owned();
        if let Some(f) = name.get_mut(0..1) {
            f.make_ascii_uppercase();
        }
        let id = self.new_character(&name, Color::WHITE);
        self.char_by_tag.insert(tag.to_owned(), id);
        self.tag_of.insert(id, tag.to_owned());
        id
    }

    /// The image `tag attrs...`: exact name, else the first image of the
    /// tag holding every attribute.
    fn find_image(&self, tag: &str, attrs: &[&str]) -> Option<(String, String)> {
        let mut want = vec![tag];
        want.extend_from_slice(attrs);
        let exact = want.join(" ");
        if let Some(path) = self.images.get(&exact) {
            return Some((exact, path.clone()));
        }
        self.images.iter().find_map(|(name, path)| {
            let mut words = name.split(' ');
            (words.next() == Some(tag) && {
                let rest: Vec<&str> = words.collect();
                attrs.iter().all(|a| rest.contains(a))
            })
            .then(|| (name.clone(), path.clone()))
        })
    }

    /// The expression of character `id` for the image `tag attrs`, added
    /// to the character when new.
    fn expression(
        &mut self,
        line: &Line,
        tag: &str,
        attrs: &[&str],
        id: CharacterId,
    ) -> Option<String> {
        let found = self.find_image(tag, attrs).or_else(|| {
            // A layered image: its base layer, whatever the attributes.
            self.layered
                .get(tag)
                .map(|p| (format!("{tag} default"), p.clone()))
        });
        let Some((name, path)) = found else {
            self.warn(
                line.line,
                format!("no image for `{tag} {}`", attrs.join(" ")),
            );
            return None;
        };
        let expr = name
            .split_once(' ')
            .map_or("default", |(_, a)| a)
            .to_owned();
        let asset = self.asset(&path, AssetKind::Image);
        let motion = match self.frames.get(&name) {
            Some(f) => {
                let (paths, fps, looping) = (f.paths.clone(), f.fps, f.looping);
                let frames = paths
                    .iter()
                    .map(|p| self.asset(p, AssetKind::Image))
                    .collect();
                Some(Motion::Animation(Animation {
                    frames: AnimFrames::Sequence(frames),
                    fps,
                    loop_mode: if looping {
                        LoopMode::Loop
                    } else {
                        LoopMode::Once
                    },
                }))
            }
            None => None,
        };
        let c = self.story.characters.iter_mut().find(|c| c.id == id)?;
        if c.expression(&expr).is_none() {
            let mut e = Expression::still(expr.clone(), asset);
            e.motion = motion;
            c.expressions.push(e);
        }
        Some(expr)
    }

    fn show(&mut self, line: &Line, toks: &[Tok]) {
        let (words, _) = Self::image_words(toks);
        let Some(tag) = words.first().cloned() else {
            self.unsupported(line, "show");
            return;
        };
        if tag == "screen" {
            let name = toks.get(2).and_then(Tok::ident).unwrap_or("").to_owned();
            self.navigation(line, &name, true);
            return;
        }
        if matches!(tag.as_str(), "expression" | "text" | "layer") {
            self.unsupported(line, "show");
            return;
        }
        if toks.iter().any(|t| {
            t.is_word("as") || t.is_word("behind") || t.is_word("zorder") || t.is_word("onlayer")
        }) {
            self.warn(
                line.line,
                "`as` / `behind` / `zorder` / `onlayer` are ignored",
            );
        }
        let transition = self.inline_transition(line, toks);
        let transform = self.at_list(line, toks);
        if self.is_background_tag(&tag) {
            if transform.is_some() {
                self.warn(line.line, "transform on a background ignored");
            }
            if let Some(asset) = self.background_asset(line, &words) {
                self.push(NodeKind::Background { asset, transition });
            }
            return;
        }
        let id = self.character_for_tag(&tag);
        let attrs: Vec<&str> = words[1..].iter().map(String::as_str).collect();
        let previous = self.stage.get(&id).cloned();
        let expression = if attrs.is_empty() {
            match &previous {
                Some((e, _)) => Some(e.clone()),
                None => self.expression(line, &tag, &[], id),
            }
        } else {
            self.expression(line, &tag, &attrs, id)
        };
        let Some(expression) = expression else {
            return;
        };
        let previous = previous.map_or(Position::Center, |(_, p)| p);
        let (position, scale, scale_y, flip_x, timeline) = match transform {
            Some((atl, custom)) => self.apply_atl(id, &atl, custom, &previous),
            None => (previous, None, None, false, None),
        };
        self.stage
            .insert(id, (expression.clone(), position.clone()));
        self.push(NodeKind::Show {
            character: id,
            expression,
            position,
            transition,
            scale,
            scale_y,
            flip_x,
            flip_y: false,
        });
        if let Some(t) = timeline {
            self.push(t);
        }
    }

    fn hide(&mut self, line: &Line, toks: &[Tok]) {
        let (words, _) = Self::image_words(toks);
        let Some(tag) = words.first() else {
            self.unsupported(line, "hide");
            return;
        };
        if tag == "screen" {
            let name = toks.get(2).and_then(Tok::ident).unwrap_or("");
            if !self.screens.contains_key(name) {
                self.unsupported(line, "hide screen");
            }
            return;
        }
        let Some(id) = self.char_by_tag.get(tag).copied() else {
            return;
        };
        let transition = self.inline_transition(line, toks);
        self.stage.remove(&id);
        self.push(NodeKind::Hide {
            character: id,
            transition,
        });
    }

    fn background_asset(&mut self, line: &Line, words: &[String]) -> Option<AssetId> {
        let name = words.join(" ");
        if let Some((_, path)) = self.find_image(
            &words[0],
            &words[1..].iter().map(String::as_str).collect::<Vec<_>>(),
        ) {
            return Some(self.asset(&path, AssetKind::Image));
        }
        if name == BLACK {
            return Some(self.black());
        }
        self.warn(line.line, format!("no image for background `{name}`"));
        None
    }

    fn black(&mut self) -> AssetId {
        let path = "assets/renpy_black.png".to_owned();
        if let Some(id) = self.assets.get(&format!("image:{path}")) {
            return *id;
        }
        let id = AssetId::new();
        self.generated
            .push((path.clone(), crate::png::solid_png(16, 9, [0, 0, 0])));
        self.story.assets.assets.push(AssetEntry {
            id,
            kind: AssetKind::Image,
            path: path.clone(),
        });
        self.assets.insert(format!("image:{path}"), id);
        id
    }

    fn scene(&mut self, line: &Line, toks: &[Tok]) {
        let (words, _) = Self::image_words(toks);
        // `scene` clears the sprites.
        let on_stage: Vec<CharacterId> = self.stage.drain().map(|(id, _)| id).collect();
        let mut ordered = on_stage;
        ordered.sort_by_key(|id| self.story.characters.iter().position(|c| c.id == *id));
        let run = std::mem::take(&mut self.run);
        for id in ordered {
            self.cur.push(node(NodeKind::Hide {
                character: id,
                transition: None,
            }));
        }
        self.run = run;
        let transition = self.inline_transition(line, toks);
        let asset = if words.is_empty() {
            Some(self.black())
        } else {
            self.background_asset(line, &words)
        };
        if let Some(asset) = asset {
            self.push(NodeKind::Background { asset, transition });
        }
    }

    // ------------------------------------------------------------ screens

    /// Pixel size of a picture of the image table (read from the file
    /// header).
    fn picture_size(&self, path: &str) -> Option<(f32, f32)> {
        let bytes = match path.strip_prefix(GENERATED) {
            Some(p) => self.generated.iter().find(|(g, _)| g == p)?.1.clone(),
            None => std::fs::read(self.game_root.as_ref()?.join(path)).ok()?,
        };
        crate::png::image_size(&bytes).map(|(w, h)| (w as f32, h as f32))
    }

    /// The zone of an image button, in normalized screen coordinates.
    fn button_rect(&mut self, b: &screen::Button, picture: Option<&str>) -> HotspotShape {
        let (sw, sh) = (self.screen_size.w, self.screen_size.h);
        let size = picture.and_then(|p| self.picture_size(p));
        let mut wh = size.unwrap_or_else(|| {
            self.note(
                "button-size",
                b.line,
                false,
                "image button whose picture size is unknown: its zone is 10% of the screen, resize it in the editor",
            );
            (sw * 0.1, sh * 0.1)
        });
        let px = |v: &[Tok], size: f32| -> Option<f32> {
            match v {
                [Tok::Num(n)] => {
                    let f: f32 = n.parse().ok()?;
                    Some(if n.contains('.') { f * size } else { f })
                }
                [Tok::Op(m), Tok::Num(n)] if m == "-" => {
                    let f: f32 = n.parse().ok()?;
                    Some(-(if n.contains('.') { f * size } else { f }))
                }
                _ => None,
            }
        };
        fn pair(v: &[Tok]) -> Option<(&[Tok], &[Tok])> {
            let close = matching(v, 0)?;
            let args = split_args(&v[1..close]);
            (args.len() == 2).then(|| (args[0], args[1]))
        }
        for (name, value) in &b.placement {
            if name == "xysize"
                && let Some((a, c)) = pair(value)
                && let (Some(w), Some(h)) = (px(a, sw), px(c, sh))
            {
                wh = (w, h);
            }
        }
        let (mut x, mut y) = (0.0, 0.0);
        let (mut ox, mut oy) = (0.0, 0.0);
        for (name, value) in &b.placement {
            let num = |v: &[Tok]| match v {
                [Tok::Num(n)] => n.parse::<f32>().ok(),
                _ => None,
            };
            match name.as_str() {
                "xpos" => x = px(value, sw).unwrap_or(x),
                "ypos" => y = px(value, sh).unwrap_or(y),
                "pos" => {
                    if let Some((a, c)) = pair(value) {
                        x = px(a, sw).unwrap_or(x);
                        y = px(c, sh).unwrap_or(y);
                    }
                }
                "xalign" => x = num(value).map_or(x, |a| a * (sw - wh.0)),
                "yalign" => y = num(value).map_or(y, |a| a * (sh - wh.1)),
                "align" => {
                    if let Some((a, c)) = pair(value) {
                        x = num(a).map_or(x, |a| a * (sw - wh.0));
                        y = num(c).map_or(y, |a| a * (sh - wh.1));
                    }
                }
                "xcenter" => x = px(value, sw).map_or(x, |c| c - wh.0 / 2.0),
                "ycenter" => y = px(value, sh).map_or(y, |c| c - wh.1 / 2.0),
                "xoffset" => ox = px(value, sw).unwrap_or(0.0),
                "yoffset" => oy = px(value, sh).unwrap_or(0.0),
                _ => {}
            }
        }
        HotspotShape::Rect {
            x: (x + ox) / sw,
            y: (y + oy) / sh,
            w: wh.0 / sw,
            h: wh.1 / sh,
        }
    }

    /// `call screen name` / `show screen name`: a navigation screen becomes
    /// a `Hotspots` node (imagemap, image buttons) or a `Choice` (text
    /// buttons); each button gets a small block (its variable changes,
    /// then its jump, call or return).
    fn navigation(&mut self, line: &Line, name: &str, shown: bool) {
        let Some(s) = self.screens.get(name).cloned() else {
            let what = if shown { "show screen" } else { "call screen" };
            self.unsupported(line, &format!("{what} (screen not converted)"));
            return;
        };
        if shown {
            self.warn(
                line.line,
                format!(
                    "`show screen {name}`: shown as a navigation step, the story waits for a click"
                ),
            );
        }
        let zones: Vec<&screen::Button> = s
            .buttons
            .iter()
            .filter(|b| b.rect.is_some() || b.idle.is_some())
            .collect();
        let use_zones = !zones.is_empty();
        let buttons: Vec<screen::Button> = if use_zones {
            zones.into_iter().cloned().collect()
        } else {
            s.buttons.clone()
        };
        if use_zones && buttons.len() < s.buttons.len() {
            self.warn(
                line.line,
                format!("screen `{name}`: text buttons mixed with image buttons are left out"),
            );
        }
        let end = self.fresh_label("screen_end");
        let n = self.counter;
        let ground = s.ground.as_deref().and_then(|g| self.resolve_picture(g));
        let hover = s.hover.as_deref().and_then(|h| self.resolve_picture(h));
        if s.ground.is_some() && ground.is_none() {
            self.warn(
                line.line,
                format!("screen `{name}`: ground picture not found"),
            );
        }
        let mut targets = Vec::new();
        let mut hotspots = Vec::new();
        let mut options = Vec::new();
        for (i, b) in buttons.iter().enumerate() {
            let target = NodeId::new();
            targets.push(target);
            let text = b.text.clone().unwrap_or_default();
            let text_id = self.add_text_raw(&text, 0);
            let condition = b.condition.as_ref().map(|c| self.condition(line, c));
            if use_zones {
                let shape = match b.rect {
                    Some((x, y, w, h)) => HotspotShape::Rect {
                        x: x / self.screen_size.w,
                        y: y / self.screen_size.h,
                        w: w / self.screen_size.w,
                        h: h / self.screen_size.h,
                    },
                    None => {
                        let idle = b.idle.as_deref().and_then(|p| self.resolve_picture(p));
                        if idle.is_some() {
                            self.note(
                                "button-pictures",
                                b.line,
                                false,
                                "image button pictures are not drawn (the engine draws a glowing zone over the background)",
                            );
                        }
                        self.button_rect(b, idle.as_deref())
                    }
                };
                let enabled = b.sensitive.as_ref().map(|c| self.condition(line, c));
                let hover_image = if b.rect.is_some() {
                    hover.as_ref().map(|h| self.asset(h, AssetKind::Image))
                } else {
                    None
                };
                hotspots.push(Hotspot {
                    name: b
                        .text
                        .clone()
                        .unwrap_or_else(|| format!("{name}_{}", i + 1)),
                    text_id,
                    shape,
                    target,
                    condition,
                    enabled,
                    style: None,
                    hover_image,
                    hover_sound: None,
                });
            } else {
                options.push(ChoiceOption {
                    text_id,
                    target,
                    condition,
                });
            }
        }
        if use_zones {
            let background = ground.map(|g| self.asset(&g, AssetKind::Image));
            self.push(NodeKind::Hotspots(HotspotMap {
                background,
                hotspots,
                style: Default::default(),
            }));
        } else {
            self.push(NodeKind::Choice { options });
        }
        for (i, (b, target)) in buttons.iter().zip(targets).enumerate() {
            self.push_label(target, format!("screen_{n}_{}", i + 1));
            self.button_actions(line, &b.actions, end);
            self.jump_to(end);
        }
        self.push_label(end, format!("screen_{n}_end"));
    }

    fn button_actions(&mut self, line: &Line, actions: &[Action], end: NodeId) {
        for a in actions {
            match a {
                Action::Set(var, value) => {
                    let name = var_name(var);
                    let value = match value {
                        Some(v) => parse_expr_toks(v),
                        None => Some(Expr::Unary(
                            orbit_core::UnOp::Not,
                            Box::new(Expr::var(name.clone())),
                        )),
                    };
                    match value {
                        Some(value) => {
                            let literal = match &value {
                                Expr::Lit(v) => Some(v.clone()),
                                _ => None,
                            };
                            self.declare_assigned(line.line, &name, var, literal);
                            self.push(NodeKind::SetVar { var: name, value });
                        }
                        None => self.unsupported(
                            line,
                            &format!(
                                "screen action setting `{var}` to an expression not converted"
                            ),
                        ),
                    }
                }
                Action::Jump(l) => match self.resolve(l) {
                    Some(t) => {
                        let kind = self.jump_kind(t);
                        self.push(kind);
                        return;
                    }
                    None => self.unsupported(
                        line,
                        &format!("screen action `Jump(\"{l}\")` to an unknown label"),
                    ),
                },
                Action::Call(l) => match self.resolve(l).map(|t| t.node) {
                    Some(label) => self.push(NodeKind::Call { label }),
                    None => self.unsupported(
                        line,
                        &format!("screen action `Call(\"{l}\")` of an unknown label"),
                    ),
                },
                Action::Notify(raw) => self.push_notify(raw, line.line),
                Action::Return => {
                    self.jump_to(end);
                    return;
                }
            }
        }
    }

    // ------------------------------------------------------------ audio

    fn audio(&mut self, line: &Line, tok: &Tok, kind: AssetKind) -> Option<AssetId> {
        let path = match tok {
            Tok::Str(p) => self.find_media(p, &["", "audio/", "music/", "sound/", "sounds/"]),
            Tok::Ident(name) => {
                let name = name.strip_prefix("audio.").unwrap_or(name);
                match self.audio_alias.get(name).cloned() {
                    Some(p) => self.find_media(&p, &["", "audio/"]),
                    None => self.media.iter().find_map(|(m, k)| {
                        let file = m.rsplit('/').next().unwrap_or(m);
                        let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
                        (*k == MediaKind::Audio && m.starts_with("audio/") && stem == name)
                            .then(|| m.clone())
                    }),
                }
            }
            _ => None,
        };
        match path {
            Some(p) => Some(self.asset(&p, kind)),
            None => {
                self.warn(
                    line.line,
                    format!("audio file not found (or not OGG/WAV): `{}`", line.text),
                );
                None
            }
        }
    }

    fn play(&mut self, line: &Line, toks: &[Tok]) {
        let channel = toks.get(1).and_then(Tok::ident).unwrap_or("");
        let Some(file_tok) = toks.get(2) else {
            self.unsupported(line, "play");
            return;
        };
        let file_tok = if file_tok.is_op("[") {
            self.warn(line.line, "playlist: only the first file is played");
            toks.get(3).cloned().unwrap_or(file_tok.clone())
        } else {
            file_tok.clone()
        };
        if toks[0].is_word("queue") {
            self.warn(line.line, "`queue` is played immediately");
        }
        let after = |w: &str| {
            toks.iter()
                .position(|t| t.is_word(w))
                .and_then(|i| toks.get(i + 1))
                .and_then(num)
        };
        match channel {
            "music" => {
                if let Some(asset) = self.audio(line, &file_tok, AssetKind::Music) {
                    let looping = !toks.iter().any(|t| t.is_word("noloop"));
                    let fade_in = after("fadein").unwrap_or(0.0);
                    self.push(NodeKind::PlayMusic {
                        asset,
                        fade_in,
                        looping,
                    });
                }
            }
            "voice" => self.pending_voice = self.audio(line, &file_tok, AssetKind::Sound),
            _ => {
                if channel != "sound" && channel != "audio" {
                    self.warn(line.line, format!("channel `{channel}` played as a sound"));
                }
                if toks.iter().any(|t| t.is_word("loop")) {
                    self.warn(line.line, "looping sounds are played once");
                }
                if let Some(asset) = self.audio(line, &file_tok, AssetKind::Sound) {
                    self.push(NodeKind::PlaySound { asset });
                }
            }
        }
    }

    fn stop(&mut self, line: &Line, toks: &[Tok]) {
        if toks.get(1).is_some_and(|t| t.is_word("music")) {
            let fade_out = toks
                .iter()
                .position(|t| t.is_word("fadeout"))
                .and_then(|i| toks.get(i + 1))
                .and_then(num)
                .unwrap_or(0.0);
            self.push(NodeKind::StopMusic { fade_out });
        } else {
            self.warn(
                line.line,
                "only `stop music` is supported (sounds end by themselves)",
            );
        }
    }

    // ------------------------------------------------------------ assets

    fn asset(&mut self, game_path: &str, kind: AssetKind) -> AssetId {
        let generated = game_path.strip_prefix(GENERATED);
        let dest = generated.map_or_else(|| project_path(game_path), str::to_owned);
        if let Some(id) = self.assets.get(&format!("{kind}:{dest}")) {
            return *id;
        }
        let id = AssetId::new();
        if generated.is_none() && !self.copies.iter().any(|(_, d)| *d == dest) {
            self.copies.push((game_path.to_owned(), dest.clone()));
        }
        self.story.assets.assets.push(AssetEntry {
            id,
            kind,
            path: dest.clone(),
        });
        self.assets.insert(format!("{kind}:{dest}"), id);
        id
    }

    /// Media no statement used are still copied and declared, so that the
    /// project holds every picture and sound of the game.
    fn declare_unused_media(&mut self) {
        let media: Vec<(String, MediaKind)> = self.media.to_vec();
        for (path, kind) in media {
            let dest = project_path(&path);
            if self.copies.iter().any(|(_, d)| *d == dest) {
                continue;
            }
            let kind = match kind {
                MediaKind::Image => AssetKind::Image,
                MediaKind::Audio => {
                    let lower = path.to_lowercase();
                    if lower.contains("music") || lower.contains("bgm") {
                        AssetKind::Music
                    } else {
                        AssetKind::Sound
                    }
                }
            };
            self.asset(&path, kind);
        }
    }
}

fn node(kind: NodeKind) -> Node {
    Node {
        id: NodeId::new(),
        kind,
    }
}

fn terminal(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Jump { .. } | NodeKind::JumpScene { .. } | NodeKind::Return | NodeKind::End
    )
}

fn num(t: &Tok) -> Option<f32> {
    match t {
        Tok::Num(n) => n.parse().ok(),
        _ => None,
    }
}

/// The screen size declared by the game (`gui.init(w, h)`,
/// `config.screen_width` / `config.screen_height`), else 1920 x 1080.
fn screen_size(scripts: &[Script]) -> atl::Screen {
    let mut size = atl::Screen::default();
    for s in scripts {
        for raw in s.source.lines() {
            let line = raw.trim();
            let toks = tokenize(line);
            let skip = usize::from(toks.first().is_some_and(|t| t.is_word("define")));
            match toks.get(skip..).unwrap_or_default() {
                [
                    Tok::Ident(f),
                    Tok::Op(o),
                    Tok::Num(w),
                    Tok::Op(c),
                    Tok::Num(h),
                    Tok::Op(e),
                    ..,
                ] if f == "gui.init" && o == "(" && c == "," && e == ")" => {
                    size.w = w.parse().unwrap_or(size.w);
                    size.h = h.parse().unwrap_or(size.h);
                }
                [Tok::Ident(n), Tok::Op(eq), Tok::Num(v)] if eq == "=" => match n.as_str() {
                    "config.screen_width" => size.w = v.parse().unwrap_or(size.w),
                    "config.screen_height" => size.h = v.parse().unwrap_or(size.h),
                    _ => {}
                },
                _ => {}
            }
        }
    }
    if size.w <= 0.0 || size.h <= 0.0 {
        size = atl::Screen::default();
    }
    size
}

/// Positional and keyword arguments of `name(...)` (tokens from `name`).
type CallArgs<'a> = (Vec<&'a [Tok]>, Vec<(String, &'a [Tok])>);

fn call_args(toks: &[Tok]) -> CallArgs<'_> {
    let mut positional = Vec::new();
    let mut keywords = Vec::new();
    if !toks.get(1).is_some_and(|t| t.is_op("(")) {
        return (positional, keywords);
    }
    let close = matching(toks, 1).unwrap_or(toks.len());
    for a in split_args(&toks[2..close.max(2)]) {
        match a {
            [Tok::Ident(k), Tok::Op(eq), v @ ..] if eq == "=" => keywords.push((k.clone(), v)),
            _ => positional.push(a),
        }
    }
    (positional, keywords)
}

/// A string argument (`"x"` or `_("x")`).
fn str_arg(toks: &[Tok]) -> Option<String> {
    match parse_literal(toks) {
        Some(Value::Str(s)) => Some(s),
        _ => None,
    }
}

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` with the alpha (255 when absent).
fn parse_rgba(s: &str) -> Option<[u8; 4]> {
    let hex = s.strip_prefix('#')?;
    let digit = |i: usize, n: usize| u8::from_str_radix(hex.get(i..i + n)?, 16).ok();
    match hex.len() {
        3 => Some([digit(0, 1)? * 17, digit(1, 1)? * 17, digit(2, 1)? * 17, 255]),
        4 => Some([
            digit(0, 1)? * 17,
            digit(1, 1)? * 17,
            digit(2, 1)? * 17,
            digit(3, 1)? * 17,
        ]),
        6 => Some([digit(0, 2)?, digit(2, 2)?, digit(4, 2)?, 255]),
        8 => Some([digit(0, 2)?, digit(2, 2)?, digit(4, 2)?, digit(6, 2)?]),
        _ => None,
    }
}

/// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    let digit = |i: usize, n: usize| u8::from_str_radix(hex.get(i..i + n)?, 16).ok();
    match hex.len() {
        3 | 4 => Some(Color::rgb(
            digit(0, 1)? * 17,
            digit(1, 1)? * 17,
            digit(2, 1)? * 17,
        )),
        6 | 8 => Some(Color::rgb(digit(0, 2)?, digit(2, 2)?, digit(4, 2)?)),
        _ => None,
    }
}
