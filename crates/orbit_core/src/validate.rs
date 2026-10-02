//! Semantic validation of a [`Story`].
//!
//! Errors make a story unplayable (dangling references, type errors...);
//! warnings flag suspicious constructs (undefined variables, unreachable
//! nodes, possible infinite loops, choices that may offer nothing).

use std::collections::{HashMap, HashSet};

use crate::defs::{AssetKind, Position};
use crate::error::{ValidationError, ValidationWarning};
use crate::expr::Expr;
use crate::ids::{AssetId, NodeId};
use crate::story::{Node, NodeKind, Story};
use crate::value::ValueType;

/// Result of [`validate`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationReport {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Checks the story against its own definitions (characters, manifest,
/// strings, positions, labels, variables). Does not touch the filesystem: see
/// [`check_asset_files`] for that.
pub fn validate(story: &Story) -> ValidationReport {
    let mut cx = Cx {
        story,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    cx.definitions();
    let flat: Vec<&Node> = story.scenes.iter().flat_map(|s| &s.nodes).collect();
    cx.node_ids(&flat);
    let mut index = HashMap::new();
    for (i, n) in flat.iter().enumerate() {
        index.entry(n.id).or_insert(i);
    }
    cx.nodes(&flat, &index);
    cx.variables(&flat);
    cx.flow(&flat, &index);
    ValidationReport {
        errors: cx.errors,
        warnings: cx.warnings,
    }
}

/// Reports every manifest asset whose file does not exist. `exists` receives
/// the manifest's relative path.
pub fn check_asset_files(story: &Story, exists: impl Fn(&str) -> bool) -> Vec<ValidationError> {
    story
        .assets
        .assets
        .iter()
        .filter(|a| !exists(&a.path))
        .map(|a| ValidationError::MissingAssetFile {
            asset: a.id,
            path: a.path.clone(),
        })
        .collect()
}

struct Cx<'a> {
    story: &'a Story,
    errors: Vec<ValidationError>,
    warnings: Vec<ValidationWarning>,
}

fn valid_duration(seconds: f32) -> bool {
    seconds.is_finite() && seconds >= 0.0
}

fn valid_asset_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path.split('/').any(|seg| seg == ".." || seg.is_empty())
}

impl Cx<'_> {
    fn definitions(&mut self) {
        let story = self.story;

        let mut asset_ids = HashSet::new();
        for a in &story.assets.assets {
            if !asset_ids.insert(a.id) {
                self.errors.push(ValidationError::DuplicateAssetId(a.id));
            }
            if !valid_asset_path(&a.path) {
                self.errors.push(ValidationError::InvalidAssetPath {
                    asset: a.id,
                    path: a.path.clone(),
                });
            }
        }

        let mut char_ids = HashSet::new();
        for c in &story.characters {
            if !char_ids.insert(c.id) {
                self.errors
                    .push(ValidationError::DuplicateCharacterId(c.id));
            }
            if story.strings.get(c.name_id).is_none() {
                self.errors.push(ValidationError::UnknownString {
                    node: None,
                    id: c.name_id,
                });
            }
            let mut names = HashSet::new();
            for e in &c.expressions {
                if !names.insert(e.name.as_str()) {
                    self.errors.push(ValidationError::DuplicateExpression {
                        character: c.id,
                        name: e.name.clone(),
                    });
                }
                self.check_asset(None, e.image, AssetKind::Image);
            }
        }

        let mut positions: HashSet<&str> = ["left", "center", "right"].into();
        for p in &story.positions {
            if !positions.insert(p.name.as_str()) {
                self.errors
                    .push(ValidationError::DuplicatePosition(p.name.clone()));
            }
            let in_range = |v: f32| (0.0..=1.0).contains(&v);
            if !in_range(p.x) || !in_range(p.y) {
                self.errors.push(ValidationError::InvalidPositionCoords {
                    name: p.name.clone(),
                });
            }
        }

        let mut vars = HashSet::new();
        for v in &story.variables {
            if !vars.insert(v.name.as_str()) {
                self.errors
                    .push(ValidationError::DuplicateVariable(v.name.clone()));
            }
        }
    }

    fn node_ids(&mut self, flat: &[&Node]) {
        let mut scenes = HashSet::new();
        for s in &self.story.scenes {
            if !scenes.insert(s.id) {
                self.errors.push(ValidationError::DuplicateSceneId(s.id));
            }
        }
        let mut seen = HashSet::new();
        for n in flat {
            if !seen.insert(n.id) {
                self.errors.push(ValidationError::DuplicateNodeId(n.id));
            }
        }
    }

    fn check_asset(&mut self, node: Option<NodeId>, asset: AssetId, expected: AssetKind) {
        match self.story.assets.get(asset) {
            None => self
                .errors
                .push(ValidationError::UnknownAsset { node, asset }),
            Some(entry) if entry.kind != expected => {
                self.errors.push(ValidationError::AssetKindMismatch {
                    node,
                    asset,
                    expected,
                    found: entry.kind,
                });
            }
            Some(_) => {}
        }
    }

    fn check_duration(&mut self, node: NodeId, seconds: f32) {
        if !valid_duration(seconds) {
            self.errors
                .push(ValidationError::InvalidDuration { node, seconds });
        }
    }

    fn check_string(&mut self, node: NodeId, id: crate::ids::StringId) {
        if self.story.strings.get(id).is_none() {
            self.errors.push(ValidationError::UnknownString {
                node: Some(node),
                id,
            });
        }
    }

    fn nodes(&mut self, flat: &[&Node], index: &HashMap<NodeId, usize>) {
        let story = self.story;
        for node in flat {
            let id = node.id;
            for target in node.kind.targets() {
                match index.get(&target) {
                    None => self
                        .errors
                        .push(ValidationError::UnknownTarget { from: id, target }),
                    Some(&i) if !matches!(flat[i].kind, NodeKind::Label { .. }) => self
                        .errors
                        .push(ValidationError::TargetNotLabel { from: id, target }),
                    Some(_) => {}
                }
            }
            for s in node.kind.strings() {
                self.check_string(id, s);
            }
            match &node.kind {
                NodeKind::Say {
                    speaker: Some(c), ..
                } if story.character(*c).is_none() => {
                    self.errors.push(ValidationError::UnknownCharacter {
                        node: id,
                        character: *c,
                    });
                }
                NodeKind::Choice { options } if options.is_empty() => {
                    self.errors.push(ValidationError::EmptyChoice(id));
                }
                NodeKind::Background { asset, transition } => {
                    self.check_asset(Some(id), *asset, AssetKind::Image);
                    if let Some(t) = transition {
                        self.check_duration(id, t.duration);
                    }
                }
                NodeKind::Show {
                    character,
                    expression,
                    position,
                    transition,
                } => {
                    match story.character(*character) {
                        None => self.errors.push(ValidationError::UnknownCharacter {
                            node: id,
                            character: *character,
                        }),
                        Some(c) if c.expression(expression).is_none() => {
                            self.errors.push(ValidationError::UnknownExpression {
                                node: id,
                                character: *character,
                                expression: expression.clone(),
                            });
                        }
                        Some(_) => {}
                    }
                    if story.resolve_position(position).is_none() {
                        let name = match position {
                            Position::Custom(n) => n.clone(),
                            other => format!("{other:?}"),
                        };
                        self.errors
                            .push(ValidationError::UnknownPosition { node: id, name });
                    }
                    if let Some(t) = transition {
                        self.check_duration(id, t.duration);
                    }
                }
                NodeKind::Hide {
                    character,
                    transition,
                } => {
                    if story.character(*character).is_none() {
                        self.errors.push(ValidationError::UnknownCharacter {
                            node: id,
                            character: *character,
                        });
                    }
                    if let Some(t) = transition {
                        self.check_duration(id, t.duration);
                    }
                }
                NodeKind::PlayMusic { asset, fade_in, .. } => {
                    self.check_asset(Some(id), *asset, AssetKind::Music);
                    self.check_duration(id, *fade_in);
                }
                NodeKind::StopMusic { fade_out } => self.check_duration(id, *fade_out),
                NodeKind::PlaySound { asset } => {
                    self.check_asset(Some(id), *asset, AssetKind::Sound);
                }
                NodeKind::Wait { seconds } => self.check_duration(id, *seconds),
                _ => {}
            }
        }
    }

    /// Variable definedness and expression typing.
    fn variables(&mut self, flat: &[&Node]) {
        let mut types: HashMap<String, Option<ValueType>> = self
            .story
            .variables
            .iter()
            .map(|v| (v.name.clone(), Some(v.initial.value_type())))
            .collect();
        let declared: HashSet<String> = types.keys().cloned().collect();
        let mut defined: HashSet<String> = declared.clone();

        // Infer the type of undeclared variables from their assignments. Two
        // passes so that the order of nodes matters little.
        for _ in 0..2 {
            for node in flat {
                if let NodeKind::SetVar { var, value } = &node.kind {
                    defined.insert(var.clone());
                    if !declared.contains(var) {
                        let known = types.get(var).copied().flatten();
                        if known.is_none() {
                            let t = value
                                .type_of(&|n| types.get(n).copied().flatten())
                                .ok()
                                .flatten();
                            types.insert(var.clone(), t);
                        }
                    }
                }
            }
        }

        let var_type = |n: &str| types.get(n).copied().flatten();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let check_expr = |errors: &mut Vec<ValidationError>,
                          warnings: &mut Vec<ValidationWarning>,
                          node: NodeId,
                          expr: &Expr,
                          want_bool: bool| {
            for v in expr.variables() {
                if !defined.contains(v) {
                    warnings.push(ValidationWarning::UndefinedVariable {
                        node,
                        var: v.to_owned(),
                    });
                }
            }
            match expr.type_of(&var_type) {
                Err(e) => errors.push(ValidationError::ExprType { node, message: e.0 }),
                Ok(Some(t)) if want_bool && t != ValueType::Bool => {
                    errors.push(ValidationError::ExprType {
                        node,
                        message: format!("condition must be bool, got {t}"),
                    });
                }
                Ok(_) => {}
            }
        };

        for node in flat {
            match &node.kind {
                NodeKind::If { condition, .. } => {
                    check_expr(&mut errors, &mut warnings, node.id, condition, true)
                }
                NodeKind::Choice { options } => {
                    for c in options.iter().filter_map(|o| o.condition.as_ref()) {
                        check_expr(&mut errors, &mut warnings, node.id, c, true);
                    }
                }
                NodeKind::SetVar { var, value } => {
                    check_expr(&mut errors, &mut warnings, node.id, value, false);
                    if declared.contains(var)
                        && let (Some(want), Ok(Some(got))) =
                            (var_type(var), value.type_of(&var_type))
                        && want != got
                        && !(want.is_numeric() && got.is_numeric())
                    {
                        errors.push(ValidationError::ExprType {
                            node: node.id,
                            message: format!("variable `{var}` is {want} but assigned a {got}"),
                        });
                    }
                }
                _ => {}
            }
        }
        self.errors.extend(errors);
        self.warnings.extend(warnings);
    }

    /// Reachability, blocking-free cycles and choices that can offer nothing.
    fn flow(&mut self, flat: &[&Node], index: &HashMap<NodeId, usize>) {
        if flat.is_empty() {
            return;
        }
        let blocking = |n: &Node| {
            matches!(
                n.kind,
                NodeKind::Say { .. } | NodeKind::Choice { .. } | NodeKind::Wait { .. }
            )
        };
        // `strict` edges only: no fall-through after a Call (it is taken when
        // the callee returns, which cycle detection cannot see).
        let succ = |i: usize, with_call_continuation: bool| -> Vec<usize> {
            let node = flat[i];
            let next = (i + 1 < flat.len()).then_some(i + 1);
            let target = |t: NodeId| index.get(&t).copied();
            match &node.kind {
                NodeKind::Jump { target: t } => target(*t).into_iter().collect(),
                NodeKind::If {
                    then_target,
                    else_target,
                    ..
                } => {
                    let mut v: Vec<usize> = target(*then_target).into_iter().collect();
                    match else_target {
                        Some(e) => v.extend(target(*e)),
                        None => v.extend(next),
                    }
                    v
                }
                NodeKind::Choice { options } => {
                    options.iter().filter_map(|o| target(o.target)).collect()
                }
                NodeKind::Call { label } => {
                    let mut v: Vec<usize> = target(*label).into_iter().collect();
                    if with_call_continuation {
                        v.extend(next);
                    }
                    v
                }
                NodeKind::Return | NodeKind::End => Vec::new(),
                _ => next.into_iter().collect(),
            }
        };

        // Reachability from the entry.
        let mut reached = vec![false; flat.len()];
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut reached[i], true) {
                continue;
            }
            stack.extend(succ(i, true));
        }
        for (i, node) in flat.iter().enumerate() {
            if !reached[i] && !matches!(node.kind, NodeKind::Label { .. }) {
                self.warnings
                    .push(ValidationWarning::UnreachableNode(node.id));
            }
        }

        // Cycles made only of non-blocking nodes (iterative DFS, 3 colors).
        let mut color = vec![0u8; flat.len()]; // 0 new, 1 on stack, 2 done
        let mut reported = false;
        for start in 0..flat.len() {
            if color[start] != 0 || blocking(flat[start]) || reported {
                continue;
            }
            let mut work: Vec<(usize, Vec<usize>)> = Vec::new();
            color[start] = 1;
            work.push((start, succ(start, false)));
            while let Some((node, pending)) = work.last_mut() {
                let node = *node;
                match pending.pop() {
                    Some(next) if blocking(flat[next]) => {}
                    Some(next) => match color[next] {
                        0 => {
                            color[next] = 1;
                            let s = succ(next, false);
                            work.push((next, s));
                        }
                        1 => {
                            self.warnings
                                .push(ValidationWarning::PossibleInfiniteLoop(flat[next].id));
                            reported = true;
                            work.clear();
                        }
                        _ => {}
                    },
                    None => {
                        color[node] = 2;
                        work.pop();
                    }
                }
            }
        }

        for node in flat {
            if let NodeKind::Choice { options } = &node.kind
                && !options.is_empty()
                && options.iter().all(|o| o.condition.is_some())
            {
                self.warnings
                    .push(ValidationWarning::AllOptionsConditional(node.id));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{AssetEntry, Character, Color, Expression, PositionDef, Transition, VarDecl};
    use crate::expr::BinOp;
    use crate::ids::{CharacterId, SceneId, StringId};
    use crate::story::{ChoiceOption, Scene};
    use crate::value::Value;

    fn node(kind: NodeKind) -> Node {
        Node {
            id: NodeId::new(),
            kind,
        }
    }

    fn label() -> Node {
        node(NodeKind::Label { name: "l".into() })
    }

    fn say(story: &mut Story) -> Node {
        node(NodeKind::Say {
            speaker: None,
            text_id: story.strings.add("x"),
        })
    }

    fn with_nodes(mut story: Story, nodes: Vec<Node>) -> Story {
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "s".into(),
            nodes,
        });
        story
    }

    fn story_with(nodes: Vec<Node>) -> Story {
        with_nodes(Story::new("t"), nodes)
    }

    fn asset(story: &mut Story, kind: AssetKind, path: &str) -> AssetId {
        let id = AssetId::new();
        story.assets.assets.push(AssetEntry {
            id,
            kind,
            path: path.into(),
        });
        id
    }

    fn character(story: &mut Story) -> (CharacterId, AssetId) {
        let img = asset(story, AssetKind::Image, "a/happy.png");
        let id = CharacterId::new();
        let name_id = story.strings.add("Ana");
        story.characters.push(Character {
            id,
            name_id,
            color: Color::WHITE,
            expressions: vec![Expression {
                name: "happy".into(),
                image: img,
            }],
        });
        (id, img)
    }

    #[test]
    fn valid_story_has_no_errors() {
        let mut s = Story::new("t");
        let l = label();
        let j = node(NodeKind::Jump { target: l.id });
        let say = say(&mut s);
        let s = with_nodes(s, vec![say, j, l]);
        let report = validate(&s);
        assert!(report.is_ok(), "{report:?}");
        assert!(report.warnings.is_empty(), "{report:?}");
    }

    #[test]
    fn missing_target_is_reported() {
        let ghost = NodeId::new();
        let j = node(NodeKind::Jump { target: ghost });
        let jid = j.id;
        assert_eq!(
            validate(&story_with(vec![j])).errors,
            vec![ValidationError::UnknownTarget {
                from: jid,
                target: ghost
            }]
        );
    }

    #[test]
    fn call_and_if_targets_must_be_labels() {
        let mut s = Story::new("t");
        let sy = say(&mut s);
        let call = node(NodeKind::Call { label: sy.id });
        let s = with_nodes(s, vec![sy, call]);
        let errs = validate(&s).errors;
        assert!(matches!(errs[..], [ValidationError::TargetNotLabel { .. }]));
    }

    #[test]
    fn duplicate_ids_and_empty_choice_are_reported() {
        let l = label();
        let dup = l.clone();
        let choice = node(NodeKind::Choice { options: vec![] });
        let errs = validate(&story_with(vec![l, dup, choice])).errors;
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::DuplicateNodeId(_)))
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::EmptyChoice(_)))
        );
    }

    #[test]
    fn choice_targets_and_strings_are_checked() {
        let c = node(NodeKind::Choice {
            options: vec![ChoiceOption {
                text_id: StringId::new(),
                target: NodeId::new(),
                condition: None,
            }],
        });
        let errs = validate(&story_with(vec![c])).errors;
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::UnknownTarget { .. }))
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, ValidationError::UnknownString { .. }))
        );
    }

    #[test]
    fn unknown_character_expression_asset_and_position() {
        let mut s = Story::new("t");
        let (ana, _) = character(&mut s);
        let music = asset(&mut s, AssetKind::Music, "m.ogg");
        s.positions.push(PositionDef {
            name: "far".into(),
            x: 0.2,
            y: 1.0,
        });
        let ok = node(NodeKind::Show {
            character: ana,
            expression: "happy".into(),
            position: Position::Custom("far".into()),
            transition: Some(Transition::dissolve(0.5)),
        });
        let bad_char = node(NodeKind::Hide {
            character: CharacterId::new(),
            transition: None,
        });
        let bad_expr = node(NodeKind::Show {
            character: ana,
            expression: "sad".into(),
            position: Position::Left,
            transition: None,
        });
        let bad_pos = node(NodeKind::Show {
            character: ana,
            expression: "happy".into(),
            position: Position::Custom("nowhere".into()),
            transition: None,
        });
        let bad_asset = node(NodeKind::Background {
            asset: AssetId::new(),
            transition: None,
        });
        let wrong_kind = node(NodeKind::Background {
            asset: music,
            transition: None,
        });
        let bad_time = node(NodeKind::Wait { seconds: -1.0 });
        let s = with_nodes(
            s,
            vec![
                ok, bad_char, bad_expr, bad_pos, bad_asset, wrong_kind, bad_time,
            ],
        );
        let errs = validate(&s).errors;
        assert_eq!(errs.len(), 6, "{errs:#?}");
        assert!(matches!(errs[0], ValidationError::UnknownCharacter { .. }));
        assert!(matches!(errs[1], ValidationError::UnknownExpression { .. }));
        assert!(matches!(errs[2], ValidationError::UnknownPosition { .. }));
        assert!(matches!(errs[3], ValidationError::UnknownAsset { .. }));
        assert!(matches!(errs[4], ValidationError::AssetKindMismatch { .. }));
        assert!(matches!(errs[5], ValidationError::InvalidDuration { .. }));
    }

    #[test]
    fn definitions_are_checked() {
        let mut s = Story::new("t");
        let (ana, img) = character(&mut s);
        // Duplicate ids / expressions / positions / variables, bad paths.
        let dup_char = s.characters[0].clone();
        s.characters.push(dup_char);
        s.characters[0].expressions.push(Expression {
            name: "happy".into(),
            image: img,
        });
        s.assets.assets.push(s.assets.assets[0].clone());
        asset(&mut s, AssetKind::Image, "../escape.png");
        asset(&mut s, AssetKind::Image, "C:/abs.png");
        asset(&mut s, AssetKind::Image, "back\\slash.png");
        s.positions.push(PositionDef {
            name: "left".into(),
            x: 0.0,
            y: 0.0,
        });
        s.positions.push(PositionDef {
            name: "off".into(),
            x: 2.0,
            y: 0.0,
        });
        for _ in 0..2 {
            s.variables.push(VarDecl {
                name: "v".into(),
                initial: Value::Int(0),
            });
        }
        let _ = ana;
        let errs = validate(&s).errors;
        let has = |f: fn(&ValidationError) -> bool| errs.iter().any(f);
        assert!(has(|e| matches!(
            e,
            ValidationError::DuplicateCharacterId(_)
        )));
        assert!(has(|e| matches!(
            e,
            ValidationError::DuplicateExpression { .. }
        )));
        assert!(has(|e| matches!(e, ValidationError::DuplicateAssetId(_))));
        assert_eq!(
            errs.iter()
                .filter(|e| matches!(e, ValidationError::InvalidAssetPath { .. }))
                .count(),
            3
        );
        assert!(has(|e| matches!(e, ValidationError::DuplicatePosition(_))));
        assert!(has(|e| matches!(
            e,
            ValidationError::InvalidPositionCoords { .. }
        )));
        assert!(has(|e| matches!(e, ValidationError::DuplicateVariable(_))));
    }

    #[test]
    fn expressions_are_typed_and_variables_checked() {
        let mut s = Story::new("t");
        s.variables.push(VarDecl {
            name: "n".into(),
            initial: Value::Int(0),
        });
        let l = label();
        let lid = l.id;
        let bad_cond = node(NodeKind::If {
            condition: Expr::var("n"), // int, not bool
            then_target: lid,
            else_target: None,
        });
        let bad_assign = node(NodeKind::SetVar {
            var: "n".into(),
            value: Expr::lit("text"),
        });
        let type_err = node(NodeKind::SetVar {
            var: "m".into(),
            value: Expr::binary(BinOp::Add, Expr::lit(1i64), Expr::lit(true)),
        });
        let undefined = node(NodeKind::If {
            condition: Expr::var("ghost"),
            then_target: lid,
            else_target: None,
        });
        let ok_numeric = node(NodeKind::SetVar {
            var: "n".into(),
            value: Expr::lit(1.5f64),
        });
        let s = with_nodes(
            s,
            vec![bad_cond, bad_assign, type_err, undefined, ok_numeric, l],
        );
        let report = validate(&s);
        assert_eq!(
            report
                .errors
                .iter()
                .filter(|e| matches!(e, ValidationError::ExprType { .. }))
                .count(),
            3,
            "{:#?}",
            report.errors
        );
        assert_eq!(
            report.warnings,
            vec![ValidationWarning::UndefinedVariable {
                node: s.scenes[0].nodes[3].id,
                var: "ghost".into()
            }]
        );
    }

    #[test]
    fn assigned_variables_count_as_defined() {
        let mut s = Story::new("t");
        let set = node(NodeKind::SetVar {
            var: "flag".into(),
            value: Expr::lit(true),
        });
        let l = label();
        let iff = node(NodeKind::If {
            condition: Expr::var("flag"),
            then_target: l.id,
            else_target: None,
        });
        let sy = say(&mut s);
        let s = with_nodes(s, vec![set, iff, sy, l]);
        let report = validate(&s);
        assert!(report.is_ok(), "{report:?}");
        assert!(report.warnings.is_empty(), "{report:?}");
    }

    #[test]
    fn flow_warnings() {
        let mut s = Story::new("t");
        // entry: Say, Jump -> end ; (dead) Say ; loop: L Jump L ; end: End
        let first = say(&mut s);
        let end = label();
        let jump_end = node(NodeKind::Jump { target: end.id });
        let dead = say(&mut s);
        let dead_id = dead.id;
        let lp = label();
        let lp_jump = node(NodeKind::Jump { target: lp.id });
        let lp_jump_id = lp_jump.id;
        let stop = node(NodeKind::End);
        let c = node(NodeKind::Choice {
            options: vec![ChoiceOption {
                text_id: s.strings.add("o"),
                target: end.id,
                condition: Some(Expr::lit(true)),
            }],
        });
        let cid = c.id;
        let s = with_nodes(s, vec![first, jump_end, dead, lp, lp_jump, end, stop, c]);
        let w = validate(&s).warnings;
        assert!(
            w.contains(&ValidationWarning::UnreachableNode(dead_id)),
            "{w:?}"
        );
        assert!(w.contains(&ValidationWarning::UnreachableNode(lp_jump_id)));
        assert!(w.contains(&ValidationWarning::PossibleInfiniteLoop(
            s.scenes[0].nodes[3].id
        )));
        assert!(w.contains(&ValidationWarning::AllOptionsConditional(cid)));
    }

    #[test]
    fn loop_with_blocking_node_is_fine() {
        let mut s = Story::new("t");
        let l = label();
        let sy = say(&mut s);
        let j = node(NodeKind::Jump { target: l.id });
        let s = with_nodes(s, vec![l, sy, j]);
        let report = validate(&s);
        assert!(report.warnings.is_empty(), "{report:?}");
    }

    #[test]
    fn asset_file_check_uses_the_callback() {
        let mut s = Story::new("t");
        asset(&mut s, AssetKind::Image, "a.png");
        asset(&mut s, AssetKind::Image, "b.png");
        let errs = check_asset_files(&s, |p| p == "a.png");
        assert!(
            matches!(&errs[..], [ValidationError::MissingAssetFile { path, .. }] if path == "b.png")
        );
    }
}
