//! Pure analysis of a story (no egui): variable and character usage that the
//! editor's own validation does not report, and the scene graph.
//!
//! Already covered by `orbit_core::validate` (so not repeated here): nodes
//! unreachable from the entry (`UnreachableNode`), scenes whose end is reached
//! without a jump, `End` or `Return` (`SceneEndsGame`), and variables read but
//! never declared nor assigned (`UndefinedVariable`).

use std::collections::{HashMap, HashSet};

use orbit_editor_sdk::orbit_core::{
    CharacterId, Expr, HotspotAction, NodeId, NodeKind, Operand, SceneId, Story, interpolate_vars,
};

/// A variable the analysis has something to say about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarIssue {
    pub name: String,
    /// Declared in the story's variable list (otherwise it only exists
    /// because some node assigns it).
    pub declared: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterIssue {
    pub id: CharacterId,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Jump,
    Choice,
    If,
    Call,
    Script,
}

impl EdgeKind {
    fn label(self) -> &'static str {
        match self {
            EdgeKind::Jump => "jump",
            EdgeKind::Choice => "choice",
            EdgeKind::If => "if",
            EdgeKind::Call => "call",
            EdgeKind::Script => "script",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphScene {
    pub id: SceneId,
    pub name: String,
    /// Reached from the entry scene by following scene-to-scene links
    /// (always true when the story has no entry).
    pub reachable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub kind: EdgeKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneGraph {
    pub scenes: Vec<GraphScene>,
    pub edges: Vec<Edge>,
    /// Index of the scene the story starts in.
    pub start: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Analysis {
    /// Variables nothing ever reads (declared or assigned only).
    pub never_read: Vec<VarIssue>,
    /// Declared variables that are read but never assigned: constants. (An
    /// undeclared variable in that case is a validation warning already.)
    pub never_written: Vec<VarIssue>,
    pub unused_characters: Vec<CharacterIssue>,
    pub graph: SceneGraph,
}

impl Analysis {
    pub fn issue_count(&self) -> usize {
        self.never_read.len() + self.never_written.len() + self.unused_characters.len()
    }
}

// ---------------------------------------------------------------- variables

#[derive(Default)]
struct Usage {
    reads: HashSet<String>,
    writes: HashSet<String>,
    /// Sources of scripts and Rhai conditions: the variables they touch are
    /// only known when they run, so any variable whose name appears in the
    /// text counts as read (and, for a script, written).
    script_sources: Vec<String>,
    rhai_conditions: Vec<String>,
}

impl Usage {
    fn expr(&mut self, e: &Expr, own: Option<&str>) {
        if let Expr::Rhai(code) = e {
            self.rhai_conditions.push(code.clone());
            return;
        }
        for v in e.variables() {
            if Some(v) != own {
                self.reads.insert(v.to_owned());
            }
        }
    }

    fn operand(&mut self, o: &Operand, own: Option<&str>) {
        if let Operand::Var(v) = o
            && Some(v.as_str()) != own
        {
            self.reads.insert(v.clone());
        }
    }
}

fn collect_usage(story: &Story) -> Usage {
    let mut u = Usage::default();
    for scene in &story.scenes {
        for node in &scene.nodes {
            match &node.kind {
                NodeKind::SetVar { var, value } => {
                    u.writes.insert(var.clone());
                    // `x = x + 1` does not make `x` read by anything else.
                    u.expr(value, Some(var));
                }
                NodeKind::If { condition, .. } => u.expr(condition, None),
                NodeKind::Choice { options } => {
                    for c in options.iter().filter_map(|o| o.condition.as_ref()) {
                        u.expr(c, None);
                    }
                }
                NodeKind::Hotspots(map) => {
                    for h in &map.hotspots {
                        for c in h.condition.iter().chain(&h.enabled) {
                            u.expr(c, None);
                        }
                        for a in h.effective_actions() {
                            if let HotspotAction::SetVar { var, value, .. } = a {
                                if !var.trim().is_empty() {
                                    u.writes.insert(var.clone());
                                }
                                u.operand(value, Some(var));
                            }
                        }
                    }
                }
                NodeKind::OpenScreen { result, params, .. } => {
                    if let Some(var) = result {
                        u.writes.insert(var.clone());
                    }
                    for p in params {
                        u.operand(&p.value, None);
                    }
                }
                NodeKind::Script { source, .. } => u.script_sources.push(source.clone()),
                _ => {}
            }
        }
    }
    // `[name]` in any text is replaced by the variable's value.
    for text in story.strings.entries.values() {
        if text.contains('[') {
            interpolate_vars(text, |name| {
                u.reads.insert(name.to_owned());
                // `[map.key]` reads the variable `map`.
                if let Some((base, _)) = name.split_once('.') {
                    u.reads.insert(base.to_owned());
                }
                None
            });
        }
    }
    u
}

fn analyze_variables(story: &Story) -> (Vec<VarIssue>, Vec<VarIssue>) {
    let usage = collect_usage(story);
    let declared: HashSet<&str> = story.variables.iter().map(|v| v.name.as_str()).collect();
    // Declaration order first, then assigned-only variables by name.
    let mut names: Vec<&str> = story.variables.iter().map(|v| v.name.as_str()).collect();
    let mut extra: Vec<&str> = usage
        .writes
        .iter()
        .map(String::as_str)
        .filter(|n| !declared.contains(n))
        .collect();
    extra.sort_unstable();
    names.extend(extra);
    let mut seen = HashSet::new();
    names.retain(|n| !n.trim().is_empty() && seen.insert(*n));

    let in_script = |name: &str| usage.script_sources.iter().any(|s| s.contains(name));
    let in_rhai = |name: &str| usage.rhai_conditions.iter().any(|s| s.contains(name));

    let mut never_read = Vec::new();
    let mut never_written = Vec::new();
    for name in names {
        let read = usage.reads.contains(name) || in_script(name) || in_rhai(name);
        let written = usage.writes.contains(name) || in_script(name);
        let issue = || VarIssue { name: name.to_owned(), declared: declared.contains(name) };
        if !read {
            never_read.push(issue());
        } else if !written && declared.contains(name) {
            never_written.push(issue());
        }
    }
    (never_read, never_written)
}

// --------------------------------------------------------------- characters

fn unused_characters(story: &Story) -> Vec<CharacterIssue> {
    let mut used: HashSet<CharacterId> = HashSet::new();
    for node in story.scenes.iter().flat_map(|s| &s.nodes) {
        match &node.kind {
            NodeKind::Say { speaker: Some(c), .. }
            | NodeKind::Show { character: c, .. }
            | NodeKind::Hide { character: c, .. } => {
                used.insert(*c);
            }
            _ => {}
        }
    }
    story
        .characters
        .iter()
        .filter(|c| !used.contains(&c.id))
        // A track animating a character that is never shown or speaking is
        // still no use, but it does reference it: keep the rule simple and
        // count it as used.
        .filter(|c| !story.scenes.iter().flat_map(|s| &s.nodes).any(|n| n.kind.animates_character(c.id)))
        .map(|c| CharacterIssue {
            id: c.id,
            name: story.text(c.name_id).unwrap_or_default().to_owned(),
        })
        .collect()
}

// -------------------------------------------------------------- scene graph

fn scene_graph(story: &Story) -> SceneGraph {
    let index_of: HashMap<SceneId, usize> =
        story.scenes.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    let mut scene_of_node: HashMap<NodeId, usize> = HashMap::new();
    for (i, s) in story.scenes.iter().enumerate() {
        for n in &s.nodes {
            scene_of_node.insert(n.id, i);
        }
    }

    let mut edges: Vec<Edge> = Vec::new();
    let mut seen: HashSet<(usize, usize, EdgeKind)> = HashSet::new();
    let mut add = |from: usize, to: Option<usize>, kind: EdgeKind| {
        if let Some(to) = to
            && to != from
            && seen.insert((from, to, kind))
        {
            edges.push(Edge { from, to, kind });
        }
    };
    for (from, scene) in story.scenes.iter().enumerate() {
        for node in &scene.nodes {
            let by_label = |t: &NodeId| scene_of_node.get(t).copied();
            match &node.kind {
                NodeKind::JumpScene { scene, label } => {
                    add(from, index_of.get(scene).copied(), EdgeKind::Jump);
                    add(from, label.as_ref().and_then(by_label), EdgeKind::Jump);
                }
                NodeKind::Jump { target } => add(from, by_label(target), EdgeKind::Jump),
                NodeKind::Call { label } => add(from, by_label(label), EdgeKind::Call),
                NodeKind::If { then_target, else_target, .. } => {
                    add(from, by_label(then_target), EdgeKind::If);
                    add(from, else_target.as_ref().and_then(by_label), EdgeKind::If);
                }
                NodeKind::Choice { options } => {
                    for o in options {
                        add(from, by_label(&o.target), EdgeKind::Choice);
                    }
                }
                NodeKind::Hotspots(map) => {
                    for h in &map.hotspots {
                        for t in h.targets() {
                            add(from, by_label(&t), EdgeKind::Choice);
                        }
                        for a in h.effective_actions() {
                            if let HotspotAction::JumpScene { scene, .. } = a {
                                add(from, index_of.get(scene).copied(), EdgeKind::Choice);
                            }
                        }
                    }
                }
                NodeKind::Script { branches, on_error, .. } => {
                    for t in branches.iter().map(|b| &b.target).chain(on_error) {
                        add(from, by_label(t), EdgeKind::Script);
                    }
                }
                _ => {}
            }
        }
    }

    let start = story.entry().and_then(|n| scene_of_node.get(&n).copied());
    let mut reachable = vec![start.is_none(); story.scenes.len()];
    if let Some(start) = start {
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut reachable[i], true) {
                continue;
            }
            stack.extend(edges.iter().filter(|e| e.from == i).map(|e| e.to));
        }
    }
    SceneGraph {
        scenes: story
            .scenes
            .iter()
            .zip(reachable)
            .map(|(s, reachable)| GraphScene { id: s.id, name: s.name.clone(), reachable })
            .collect(),
        edges,
        start,
    }
}

pub fn analyze(story: &Story) -> Analysis {
    let (never_read, never_written) = analyze_variables(story);
    Analysis {
        never_read,
        never_written,
        unused_characters: unused_characters(story),
        graph: scene_graph(story),
    }
}

// ------------------------------------------------------------------ Mermaid

/// Text of a Mermaid quoted label: quotes and angle brackets are written as
/// entities, line breaks become spaces.
fn mermaid_label(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("#quot;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '&' => out.push_str("#amp;"),
            '`' => out.push_str("#96;"),
            '\n' | '\r' | '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The scene graph as a Mermaid `flowchart`. Scenes are `S0`, `S1`... in story
/// order; the entry scene is fed by a `start` node, unreachable scenes are
/// dashed, and links are labelled with what makes them (jump, choice...).
pub fn to_mermaid(graph: &SceneGraph) -> String {
    let mut out = String::from("flowchart TD\n");
    for (i, s) in graph.scenes.iter().enumerate() {
        let name = if s.name.trim().is_empty() { format!("scene {}", i + 1) } else { s.name.clone() };
        out.push_str(&format!("    S{i}[\"{}\"]\n", mermaid_label(&name)));
    }
    if let Some(start) = graph.start {
        out.push_str(&format!("    start((start)) --> S{start}\n"));
    }
    for e in &graph.edges {
        out.push_str(&format!("    S{} -->|{}| S{}\n", e.from, e.kind.label(), e.to));
    }
    let dead: Vec<String> = graph
        .scenes
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.reachable)
        .map(|(i, _)| format!("S{i}"))
        .collect();
    if !dead.is_empty() {
        out.push_str("    classDef unreachable stroke:#c0392b,stroke-dasharray:5 5\n");
        out.push_str(&format!("    class {} unreachable\n", dead.join(",")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_editor_sdk::orbit_core::{
        BinOp, Character, ChoiceOption, Color, Node, Scene, ScriptBranch, Value, VarDecl,
    };

    fn node(kind: NodeKind) -> Node {
        Node { id: NodeId::new(), kind }
    }

    fn label() -> Node {
        node(NodeKind::Label { name: "l".into() })
    }

    fn scene(name: &str, nodes: Vec<Node>) -> Scene {
        Scene { id: SceneId::new(), name: name.into(), nodes }
    }

    fn say(story: &mut Story, speaker: Option<CharacterId>, text: &str) -> Node {
        let text_id = story.strings.add(text);
        node(NodeKind::Say { speaker, text_id, voice: None })
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

    fn set(var: &str, value: Expr) -> Node {
        node(NodeKind::SetVar { var: var.into(), value })
    }

    fn var(name: &str) -> Expr {
        Expr::Var(name.into())
    }

    fn lit(i: i64) -> Expr {
        Expr::Lit(Value::Int(i))
    }

    fn names(v: &[VarIssue]) -> Vec<&str> {
        v.iter().map(|i| i.name.as_str()).collect()
    }

    #[test]
    fn an_empty_story_has_nothing_to_report() {
        let a = analyze(&Story::new("t"));
        assert_eq!(a.issue_count(), 0);
        assert_eq!(a.graph, SceneGraph::default());
        assert_eq!(to_mermaid(&a.graph), "flowchart TD\n");
    }

    #[test]
    fn unused_and_constant_variables_are_found() {
        let mut story = Story::new("t");
        story.variables.push(VarDecl::game("unused", Value::Int(0)));
        story.variables.push(VarDecl::game("written_only", Value::Int(0)));
        story.variables.push(VarDecl::game("constant", Value::Int(3)));
        story.variables.push(VarDecl::game("used", Value::Int(0)));
        let cond = Expr::Binary(BinOp::Add, Box::new(var("constant")), Box::new(var("used")));
        let iff = node(NodeKind::If { condition: cond, then_target: NodeId::new(), else_target: None });
        story.scenes.push(scene(
            "a",
            vec![set("written_only", lit(1)), set("used", lit(2)), set("temp", lit(3)), iff],
        ));
        let a = analyze(&story);
        // Declared ones in declaration order, then assigned-only by name.
        assert_eq!(names(&a.never_read), ["unused", "written_only", "temp"]);
        assert_eq!(
            a.never_read.iter().map(|v| v.declared).collect::<Vec<_>>(),
            [true, true, false]
        );
        assert_eq!(names(&a.never_written), ["constant"]);
    }

    #[test]
    fn reading_only_itself_does_not_count_as_read() {
        let mut story = Story::new("t");
        story.variables.push(VarDecl::game("counter", Value::Int(0)));
        let inc = Expr::Binary(BinOp::Add, Box::new(var("counter")), Box::new(lit(1)));
        story.scenes.push(scene("a", vec![set("counter", inc)]));
        assert_eq!(names(&analyze(&story).never_read), ["counter"]);
    }

    #[test]
    fn text_interpolation_scripts_and_rhai_count_as_use() {
        let mut story = Story::new("t");
        for n in ["gold", "inventory", "hp", "mana", "scripted"] {
            story.variables.push(VarDecl::game(n, Value::Int(0)));
        }
        let shown = say(&mut story, None, "You have [gold] coins and [inventory.sword].");
        let rhai = node(NodeKind::If {
            condition: Expr::Rhai("hp > 3".into()),
            then_target: NodeId::new(),
            else_target: None,
        });
        let script = node(NodeKind::Script {
            source: "set_var(\"scripted\", get_var(\"scripted\") + 1);".into(),
            branches: Vec::new(),
            on_error: None,
        });
        story.scenes.push(scene("a", vec![shown, rhai, script]));
        let a = analyze(&story);
        // `gold`, `inventory` (text), `hp` (Rhai), `scripted` (script) are
        // read; only `mana` is not. `hp` is read but never written.
        assert_eq!(names(&a.never_read), ["mana"]);
        assert_eq!(names(&a.never_written), ["gold", "inventory", "hp"]);
    }

    #[test]
    fn screen_results_and_choice_conditions_are_handled() {
        let mut story = Story::new("t");
        story.variables.push(VarDecl::game("answer", Value::Int(0)));
        story.variables.push(VarDecl::game("key", Value::Bool(false)));
        let open = node(NodeKind::OpenScreen {
            screen: "ask".into(),
            wait: true,
            result: Some("answer".into()),
            params: Vec::new(),
        });
        let l = label();
        let choice = node(NodeKind::Choice {
            options: vec![ChoiceOption {
                text_id: story.strings.add("Open"),
                target: l.id,
                condition: Some(var("key")),
            }],
        });
        story.scenes.push(scene("a", vec![open, choice, l]));
        let a = analyze(&story);
        assert_eq!(names(&a.never_read), ["answer"], "written by the screen, read by nobody");
        assert_eq!(names(&a.never_written), ["key"]);
    }

    #[test]
    fn unused_characters_ignore_speakers_shows_and_animations() {
        let mut story = Story::new("t");
        let speaker = character(&mut story, "Speaker");
        let shown = character(&mut story, "Shown");
        let hidden = character(&mut story, "Hidden");
        let _ghost = character(&mut story, "Ghost");
        let s = say(&mut story, Some(speaker), "hi");
        let show = node(NodeKind::Show {
            character: shown,
            expression: "neutral".into(),
            position: orbit_editor_sdk::orbit_core::Position::Center,
            transition: None,
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
        });
        let hide = node(NodeKind::Hide { character: hidden, transition: None });
        story.scenes.push(scene("a", vec![s, show, hide]));
        let a = analyze(&story);
        assert_eq!(a.unused_characters.len(), 1);
        assert_eq!(a.unused_characters[0].name, "Ghost");
    }

    /// a --jump--> b --choice--> c ; d is never linked ; e only from d.
    fn graph_story() -> Story {
        let mut story = Story::new("t");
        let (lb, lc) = (label(), label());
        let (b_id, c_id) = (SceneId::new(), SceneId::new());
        let opt = ChoiceOption { text_id: story.strings.add("Go"), target: lc.id, condition: None };
        let a = scene("a", vec![node(NodeKind::JumpScene { scene: b_id, label: None })]);
        let b = Scene {
            id: b_id,
            name: "b \"quoted\" <x>".into(),
            nodes: vec![lb, node(NodeKind::Choice { options: vec![opt] })],
        };
        let c = Scene { id: c_id, name: "c".into(), nodes: vec![lc, node(NodeKind::End)] };
        let e = scene("e", vec![node(NodeKind::End)]);
        let d = scene("d", vec![node(NodeKind::JumpScene { scene: e.id, label: None })]);
        story.scenes = vec![a, b, c, d, e];
        story
    }

    #[test]
    fn the_scene_graph_has_edges_and_unreachable_scenes() {
        let g = analyze(&graph_story()).graph;
        assert_eq!(g.start, Some(0));
        let kinds: Vec<(usize, usize, EdgeKind)> = g.edges.iter().map(|e| (e.from, e.to, e.kind)).collect();
        assert_eq!(kinds, [(0, 1, EdgeKind::Jump), (1, 2, EdgeKind::Choice), (3, 4, EdgeKind::Jump)]);
        let reach: Vec<bool> = g.scenes.iter().map(|s| s.reachable).collect();
        assert_eq!(reach, [true, true, true, false, false]);
    }

    #[test]
    fn a_choice_inside_the_same_scene_is_not_an_edge_and_duplicates_collapse() {
        let mut story = Story::new("t");
        let l = label();
        let target = SceneId::new();
        let two = |s: &mut Story| ChoiceOption { text_id: s.strings.add("x"), target: l.id, condition: None };
        let choice = node(NodeKind::Choice { options: vec![two(&mut story), two(&mut story)] });
        let jumps = vec![
            node(NodeKind::JumpScene { scene: target, label: None }),
            node(NodeKind::JumpScene { scene: target, label: None }),
        ];
        let mut nodes = vec![l, choice];
        nodes.extend(jumps);
        story.scenes.push(scene("a", nodes));
        story.scenes.push(Scene { id: target, name: "b".into(), nodes: vec![node(NodeKind::End)] });
        let g = analyze(&story).graph;
        assert_eq!(g.edges.len(), 1);
    }

    #[test]
    fn script_branches_and_calls_make_edges() {
        let mut story = Story::new("t");
        let l = label();
        let callee = Scene { id: SceneId::new(), name: "routine".into(), nodes: vec![l.clone(), node(NodeKind::Return)] };
        let branch_target = label();
        let other = Scene { id: SceneId::new(), name: "other".into(), nodes: vec![branch_target.clone()] };
        let script = node(NodeKind::Script {
            source: String::new(),
            branches: vec![ScriptBranch { name: "go".into(), target: branch_target.id }],
            on_error: None,
        });
        let call = node(NodeKind::Call { label: l.id });
        story.scenes = vec![scene("main", vec![call, script]), callee, other];
        let g = analyze(&story).graph;
        let kinds: Vec<(usize, usize, EdgeKind)> = g.edges.iter().map(|e| (e.from, e.to, e.kind)).collect();
        assert_eq!(kinds, [(0, 1, EdgeKind::Call), (0, 2, EdgeKind::Script)]);
        assert!(g.scenes.iter().all(|s| s.reachable));
    }

    #[test]
    fn mermaid_output_is_escaped_and_marks_unreachable_scenes() {
        let text = to_mermaid(&analyze(&graph_story()).graph);
        let expected = "\
flowchart TD
    S0[\"a\"]
    S1[\"b #quot;quoted#quot; #lt;x#gt;\"]
    S2[\"c\"]
    S3[\"d\"]
    S4[\"e\"]
    start((start)) --> S0
    S0 -->|jump| S1
    S1 -->|choice| S2
    S3 -->|jump| S4
    classDef unreachable stroke:#c0392b,stroke-dasharray:5 5
    class S3,S4 unreachable
";
        assert_eq!(text, expected);
    }

    #[test]
    fn unnamed_scenes_get_a_fallback_label_and_newlines_are_flattened() {
        let mut story = Story::new("t");
        story.scenes.push(scene("", vec![node(NodeKind::End)]));
        story.scenes.push(scene("two\nlines", vec![node(NodeKind::End)]));
        let text = to_mermaid(&analyze(&story).graph);
        assert!(text.contains("S0[\"scene 1\"]"), "{text}");
        assert!(text.contains("S1[\"two lines\"]"), "{text}");
        // Not linked: S1 is not reachable from the entry.
        assert!(text.contains("class S1 unreachable"), "{text}");
    }

    #[test]
    fn a_story_without_nodes_marks_nothing_unreachable() {
        let mut story = Story::new("t");
        story.scenes.push(scene("empty", Vec::new()));
        story.scenes.push(scene("also empty", Vec::new()));
        let g = analyze(&story).graph;
        assert_eq!(g.start, None);
        assert!(g.scenes.iter().all(|s| s.reachable));
    }
}
