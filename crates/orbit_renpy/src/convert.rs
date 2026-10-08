//! Ren'Py statements to the story IR.
//!
//! One scene per top-level label (`start` first); nested and local
//! (`.name`) labels become `Label` nodes of their scene. Ren'Py falls
//! through from a label into the next one of the same file: the scene then
//! ends with a `JumpScene` to it.

use std::collections::{BTreeMap, HashMap, HashSet};

use orbit_core::{
    AnimTarget, AssetEntry, AssetId, AssetKind, BinOp, Character, CharacterId, ChoiceOption, Color,
    Effect, Expr, Expression, Node, NodeId, NodeKind, Position, Scene, SceneId, Story, StringId,
    Transition, Value, VarDecl,
};

use crate::lexer::{Line, Tok, matching, parse_tree, split_args, tokenize};
use crate::pyexpr::{parse_expr_toks, parse_literal, to_rhai_toks, var_name, zero_like};
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
    nvl_warned: bool,
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
    pub fn new(title: &str, scripts: &'a [Script], media: &'a [(String, MediaKind)]) -> Self {
        let trees = scripts
            .iter()
            .map(|s| parse_tree(&s.source, false))
            .collect();
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
            nvl_warned: false,
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
        Converted {
            labels: self.labels.len(),
            story: self.story,
            copies: self.copies,
            generated: self.generated,
            text_index: self.text_index,
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
                    self.unsupported(line, "init python block (skipped)");
                } else {
                    for c in &line.children {
                        self.top_level(c);
                    }
                }
            }
            "python" => self.unsupported(line, "python block (skipped)"),
            "screen" => self.unsupported(
                line,
                "screen (OrbitEngine screens are made in the UI editor)",
            ),
            "style" => self.unsupported(line, "style (skipped)"),
            "transform" => self.unsupported(line, "transform / ATL (skipped)"),
            "layeredimage" => self.unsupported(line, "layered image (skipped)"),
            _ => self.unsupported(line, "statement outside a label (skipped)"),
        }
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
        if let Some(t) = self.transition(value) {
            self.transitions.insert(name, t);
            return;
        }
        let ignored = ["config.", "gui.", "build.", "style.", "preferences.", "_"];
        if ignored.iter().any(|p| name.starts_with(p)) {
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
                _ => {}
            }
        }
        if call == "NVLCharacter" {
            self.nvl(line.line);
        }
        if var == "narrator" || var == "centered" {
            return;
        }
        let shown = name.clone().unwrap_or_else(|| var.to_owned());
        let id = self.new_character(&shown, color);
        self.char_by_var.insert(var.to_owned(), id);
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
        if !self.nvl_warned {
            self.nvl_warned = true;
            self.warn(
                line,
                "NVL mode is not supported: NVL lines are shown in the normal text box",
            );
        }
    }

    fn image_statement(&mut self, line: &Line, toks: &[Tok]) {
        let Some(eq) = toks.iter().position(|t| t.is_op("=")) else {
            self.unsupported(line, "image with an ATL block");
            return;
        };
        let name: Vec<&str> = toks[1..eq].iter().filter_map(Tok::ident).collect();
        let name = image_name(&name.join(" "));
        match parse_literal(&toks[eq + 1..]) {
            Some(Value::Str(path)) => match self.find_media(&path, &["", "images/"]) {
                Some(found) => {
                    self.images.insert(name, found);
                }
                None => self.warn(
                    line.line,
                    format!("image `{name}`: file `{path}` not found or not PNG/JPEG"),
                ),
            },
            _ => self.unsupported(
                line,
                "image defined by an expression (Solid, im.*, Composite...)",
            ),
        }
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
        if visual {
            self.run.push(self.cur.len());
        } else {
            self.run.clear();
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
            "window" | "pass" => {}
            "nvl" => self.nvl(line.line),
            "python" | "init" => self.script_comment(line, "python block"),
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
            match who.as_str() {
                "narrator" | "centered" | "extend" => {}
                _ => match self.char_by_var.get(who).copied() {
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
                },
            }
            if who == "extend" {
                speaker = self.last_speaker();
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
        let text_id = self.add_text_raw(&raw, line.line);
        let voice = self.pending_voice.take();
        self.push(NodeKind::Say {
            speaker,
            text_id,
            voice,
        });
        // `"..." with vpunch`
        if let Some(w) = toks.iter().position(|t| t.is_word("with")) {
            self.with(line, &toks[w + 1..]);
        }
    }

    fn last_speaker(&self) -> Option<CharacterId> {
        self.cur.iter().rev().find_map(|n| match &n.kind {
            NodeKind::Say { speaker, .. } => Some(*speaker),
            _ => None,
        })?
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
        let mut notes = TextNotes::default();
        let converted = text::convert(raw, &mut notes);
        if line > 0 {
            for var in notes.interpolations {
                self.warn(
                    line,
                    format!("`[{var}]` interpolation is kept as plain text (not supported)"),
                );
            }
            for tag in notes.dropped_tags {
                self.warn(
                    line,
                    format!("text tag `{{{tag}}}` has no equivalent and was removed"),
                );
            }
        }
        let id = self.story.strings.add(converted);
        self.text_index.entry(raw.to_owned()).or_default().push(id);
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
            Some("screen") => self.unsupported(line, "call screen"),
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

    fn python(&mut self, line: &Line, code: &str) {
        let toks = tokenize(code);
        // renpy.pause(n)
        if toks.first().is_some_and(|t| t.is_word("renpy.pause")) {
            let seconds = toks.get(2).and_then(num).unwrap_or(1.0);
            self.push(NodeKind::Wait { seconds });
            return;
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
                if !self.vars.contains(&var) {
                    // `$ x = 0` without `default`: declare it.
                    match parse_literal(rest).filter(|_| op == "=") {
                        Some(v) => {
                            self.vars.insert(var.clone());
                            self.story
                                .variables
                                .push(VarDecl::game(var.clone(), zero_like(&v)));
                        }
                        None => self.warn(
                            line.line,
                            format!("variable `{var}` is never declared (`default`)"),
                        ),
                    }
                }
                self.push(NodeKind::SetVar { var, value });
                return;
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
            return;
        }
        self.script_comment(line, "Python statement");
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
        match self.transition(tail) {
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
    fn transition(&self, toks: &[Tok]) -> Option<Option<Transition>> {
        let name = toks.first()?.ident()?;
        let arg = |i: usize| -> Option<f32> {
            if !toks.get(1)?.is_op("(") {
                return None;
            }
            let close = matching(toks, 1)?;
            let args = split_args(&toks[2..close]);
            args.get(i).and_then(|a| a.first()).and_then(num)
        };
        let t = match name {
            "None" => None,
            "dissolve" => Some(Transition::dissolve(0.5)),
            "Dissolve" | "Pixellate" | "ImageDissolve" | "AlphaDissolve" => {
                Some(Transition::dissolve(arg(0).unwrap_or(0.5)))
            }
            "fade" => Some(Transition::fade(Color::BLACK, 1.0)),
            "Fade" => {
                let total = arg(0).unwrap_or(0.5) + arg(1).unwrap_or(0.0) + arg(2).unwrap_or(0.5);
                Some(Transition::fade(Color::BLACK, total))
            }
            "pixellate" => Some(Transition::dissolve(1.0)),
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
                    Some(Transition::dissolve(0.5))
                } else {
                    return None;
                }
            }
        };
        Some(t)
    }

    fn with(&mut self, line: &Line, tail: &[Tok]) {
        let name = tail.first().and_then(Tok::ident).unwrap_or("");
        if matches!(name, "vpunch" | "hpunch") && self.run.is_empty() {
            self.push(NodeKind::Effect(Effect::Shake {
                target: AnimTarget::Camera,
                amplitude: 0.02,
                duration: 0.5,
                ease: None,
                wait: true,
            }));
            return;
        }
        let transition = match self.transition(tail) {
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

    fn position(&mut self, line: &Line, toks: &[Tok]) -> Option<Position> {
        let at = toks.iter().position(|t| t.is_word("at"))?;
        let name = toks.get(at + 1).and_then(Tok::ident).unwrap_or("");
        Some(match name {
            "left" => Position::Left,
            "right" => Position::Right,
            "center" | "truecenter" => Position::Center,
            "offscreenleft" => Position::Free { x: -0.2, y: 1.0 },
            "offscreenright" => Position::Free { x: 1.2, y: 1.0 },
            other => {
                self.warn(
                    line.line,
                    format!("transform `{other}` replaced by the center position"),
                );
                Position::Center
            }
        })
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
        let Some((name, path)) = self.find_image(tag, attrs) else {
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
        let c = self.story.characters.iter_mut().find(|c| c.id == id)?;
        if c.expression(&expr).is_none() {
            c.expressions.push(Expression::still(expr.clone(), asset));
        }
        Some(expr)
    }

    fn show(&mut self, line: &Line, toks: &[Tok]) {
        let (words, _) = Self::image_words(toks);
        let Some(tag) = words.first().cloned() else {
            self.unsupported(line, "show");
            return;
        };
        if matches!(tag.as_str(), "screen" | "expression" | "text" | "layer") {
            self.unsupported(line, "show");
            return;
        }
        if !line.children.is_empty() || line.text.ends_with(':') {
            self.warn(line.line, "ATL block ignored");
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
        if self.is_background_tag(&tag) {
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
        let position = self
            .position(line, toks)
            .or_else(|| previous.map(|(_, p)| p))
            .unwrap_or(Position::Center);
        self.stage
            .insert(id, (expression.clone(), position.clone()));
        self.push(NodeKind::Show {
            character: id,
            expression,
            position,
            transition,
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
        });
    }

    fn hide(&mut self, line: &Line, toks: &[Tok]) {
        let (words, _) = Self::image_words(toks);
        let Some(tag) = words.first() else {
            self.unsupported(line, "hide");
            return;
        };
        if tag == "screen" {
            self.unsupported(line, "hide screen");
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
        let dest = project_path(game_path);
        if let Some(id) = self.assets.get(&format!("{kind}:{dest}")) {
            return *id;
        }
        let id = AssetId::new();
        if !self.copies.iter().any(|(_, d)| *d == dest) {
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
