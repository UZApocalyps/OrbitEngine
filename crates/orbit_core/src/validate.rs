//! Semantic validation of a [`Story`].
//!
//! Errors make a story unplayable (dangling references, type errors...);
//! warnings flag suspicious constructs (undefined variables, unreachable
//! nodes, possible infinite loops, choices that may offer nothing).

use std::collections::{HashMap, HashSet};

use crate::defs::{AssetKind, FREE_POSITION_RANGE, Position, scale_is_valid};
use orbit_anim::Curve;

use crate::error::{CurveOwner, ValidationError, ValidationWarning};
use crate::expr::{BinOp, Expr};
use crate::ids::{AssetId, NodeId};
use crate::story::{Node, NodeKind, Story};
use crate::value::{Value, ValueType};

/// Most layers of a `BackgroundLayers` node.
pub const MAX_BG_LAYERS: usize = 8;

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

fn track_values_finite(data: &orbit_anim::TrackData) -> bool {
    use orbit_anim::TrackData as T;
    match data {
        T::Scalar(t) => t.keys.iter().all(|k| k.value.is_finite()),
        T::Vec2(t) => t.keys.iter().all(|k| k.value.x.is_finite() && k.value.y.is_finite()),
        T::Color(t) => t.keys.iter().all(|k| {
            [k.value.r, k.value.g, k.value.b, k.value.a].iter().all(|v| v.is_finite())
        }),
        T::Angle(t) => t.keys.iter().all(|k| k.value.0.is_finite()),
        T::Name(_) => true,
    }
}

fn for_each_ease(data: &orbit_anim::TrackData, f: &mut dyn FnMut(&Curve)) {
    use orbit_anim::TrackData as T;
    match data {
        T::Scalar(t) => t.keys.iter().for_each(|k| f(&k.ease)),
        T::Vec2(t) => t.keys.iter().for_each(|k| f(&k.ease)),
        T::Color(t) => t.keys.iter().for_each(|k| f(&k.ease)),
        T::Angle(t) => t.keys.iter().for_each(|k| f(&k.ease)),
        T::Name(t) => t.keys.iter().for_each(|k| f(&k.ease)),
    }
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
        for (name, curve) in &story.curves {
            if matches!(curve, Curve::Named(_)) {
                self.errors.push(ValidationError::NamedCurveChain(name.clone()));
            } else {
                self.check_curve(CurveOwner::Named(name.clone()), curve);
            }
        }

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
                self.check_motion(c.id, e);
            }
            if !scale_is_valid(c.default_scale) {
                self.errors.push(ValidationError::InvalidScale {
                    node: None,
                    scale: c.default_scale,
                });
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
            if !v.initial.is_well_formed() {
                self.errors.push(ValidationError::InvalidVariableValue {
                    name: v.name.clone(),
                    reason: format!(
                        "every element must be a {}",
                        v.initial
                            .value_type()
                            .element()
                            .map_or(String::new(), |t| t.to_string())
                    ),
                });
            } else if !v.initial.within_limits() {
                self.errors.push(ValidationError::InvalidVariableValue {
                    name: v.name.clone(),
                    reason: format!(
                        "at most {} elements and {} bytes of text",
                        crate::value::MAX_COLLECTION_LEN,
                        crate::value::MAX_COLLECTION_TEXT
                    ),
                });
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

    fn check_motion(&mut self, character: crate::ids::CharacterId, e: &crate::defs::Expression) {
        use crate::defs::{AnimFrames, Motion};
        use crate::error::AnimationIssue as I;
        let Some(motion) = &e.motion else { return };
        let anim = match motion {
            Motion::Rigged(r) => {
                // The clip, skin and pictures live in the rig file: they
                // are checked with the project's files (editor, MCP,
                // `orbit_project`), not by the story alone.
                if r.clip.trim().is_empty() {
                    self.errors.push(ValidationError::InvalidAnimation {
                        character,
                        expression: e.name.clone(),
                        issue: I::NoRigClip,
                    });
                }
                self.check_asset(None, r.rig, AssetKind::Rig);
                return;
            }
            Motion::Animation(a) => a,
        };
        let mut issue = |issue: I| {
            self.errors.push(ValidationError::InvalidAnimation {
                character,
                expression: e.name.clone(),
                issue,
            });
        };
        if !(anim.fps.is_finite() && anim.fps > 0.0) {
            issue(I::BadFps(anim.fps));
        }
        match &anim.frames {
            AnimFrames::SpriteSheet {
                cols,
                rows,
                frame_count,
                frame_size,
                ..
            } => {
                if *cols == 0 || *rows == 0 {
                    issue(I::EmptyGrid);
                } else if *frame_count == 0 {
                    issue(I::NoFrames);
                } else if u64::from(*frame_count) > u64::from(*cols) * u64::from(*rows) {
                    issue(I::FrameCountExceedsGrid {
                        frame_count: *frame_count,
                        capacity: u64::from(*cols) * u64::from(*rows),
                    });
                }
                if frame_size.is_some_and(|[w, h]| w == 0 || h == 0) {
                    issue(I::BadFrameSize);
                }
            }
            AnimFrames::Sequence(v) if v.is_empty() => issue(I::EmptySequence),
            AnimFrames::Sequence(_) => {}
        }
        for id in anim.frames.assets() {
            self.check_asset(None, id, AssetKind::Image);
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

    /// A curve must have finite control points; a `Named` one must exist.
    fn check_curve(&mut self, owner: CurveOwner, curve: &Curve) {
        match curve {
            Curve::Preset(_) => {}
            Curve::Bezier { p1, p2 } => {
                if !p1.iter().chain(p2).all(|v| v.is_finite()) {
                    self.errors.push(ValidationError::InvalidCurve { owner });
                }
            }
            Curve::Named(name) => {
                if !self.story.curves.contains_key(name) {
                    self.warnings.push(ValidationWarning::UnknownCurve {
                        owner,
                        name: name.clone(),
                    });
                }
            }
        }
    }

    fn stage_problem(&mut self, node: NodeId, message: impl Into<String>) {
        self.errors.push(ValidationError::InvalidStageNode {
            node,
            message: message.into(),
        });
    }

    /// Effects and timelines: finite numbers, known targets, properties that
    /// fit their target and value type, sorted keys, no blocking infinite
    /// loop.
    fn check_animation(&mut self, id: NodeId, kind: &NodeKind) {
        let Some(spec) = kind.animation() else { return };
        let bad = |this: &mut Self, message: String| {
            this.errors
                .push(ValidationError::InvalidTimeline { node: id, message });
        };
        self.check_duration(id, spec.duration);
        if spec.wait && spec.total_duration().is_none() {
            self.errors
                .push(ValidationError::BlockingInfiniteTimeline(id));
        }
        if let NodeKind::Effect(e) = kind {
            if let Some(c) = e.ease() {
                self.check_curve(CurveOwner::Node(id), c);
            }
            let finite = match e {
                crate::effects::Effect::Move { dx, dy, .. } => dx.is_finite() && dy.is_finite(),
                crate::effects::Effect::Shake { amplitude, .. } => amplitude.is_finite(),
                crate::effects::Effect::Zoom { factor, .. } => factor.is_finite() && *factor > 0.0,
                crate::effects::Effect::Flash { .. } => true,
                crate::effects::Effect::Tint { strength, .. } => {
                    strength.is_finite() && (0.0..=1.0).contains(strength)
                }
            };
            if !finite {
                bad(self, "effect parameter out of range".into());
            }
        }
        for (i, track) in spec.tracks.iter().enumerate() {
            if let crate::effects::AnimTarget::Character(c) = track.target
                && self.story.character(c).is_none()
            {
                self.errors.push(ValidationError::UnknownCharacter {
                    node: id,
                    character: c,
                });
            }
            if !track.prop.applies_to(&track.target) {
                bad(
                    self,
                    format!("track {i}: `{}` does not apply to this target", track.prop.name()),
                );
            }
            if !track.prop.accepts(&track.data) {
                bad(
                    self,
                    format!("track {i}: wrong value type for `{}`", track.prop.name()),
                );
            }
            if let Err(e) = track.data.validate() {
                bad(self, format!("track {i}: {e}"));
            }
            if !track_values_finite(&track.data) {
                bad(self, format!("track {i}: a key value is not finite"));
            }
            if track.data.end_time() > spec.duration + 1e-4 {
                self.warnings.push(ValidationWarning::TimelineKeyAfterEnd(id));
            }
            for_each_ease(&track.data, &mut |c| {
                self.check_curve(CurveOwner::Node(id), c);
            });
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
                NodeKind::JumpScene { scene, label } => match story.scene(*scene) {
                    None => self.errors.push(ValidationError::UnknownScene {
                        node: id,
                        scene: *scene,
                    }),
                    // A label that does not exist at all is reported with the
                    // other targets above.
                    Some(sc) => {
                        if let Some(l) = label
                            && index.contains_key(l)
                            && !sc.nodes.iter().any(|n| n.id == *l)
                        {
                            self.errors.push(ValidationError::LabelNotInScene {
                                node: id,
                                label: *l,
                                scene: *scene,
                            });
                        }
                    }
                },
                NodeKind::ShowScene3d {
                    scene, transition, ..
                } => {
                    self.check_asset(Some(id), *scene, AssetKind::Scene3d);
                    if let Some(t) = transition {
                        self.check_duration(id, t.duration);
                        if let Some(c) = &t.ease {
                            self.check_curve(CurveOwner::Node(id), c);
                        }
                    }
                }
                NodeKind::Camera3d { transition, .. } | NodeKind::HideScene3d { transition } => {
                    if let Some(t) = transition {
                        self.check_duration(id, t.duration);
                        if let Some(c) = &t.ease {
                            self.check_curve(CurveOwner::Node(id), c);
                        }
                    }
                }
                NodeKind::Background { asset, transition } => {
                    self.check_asset(Some(id), *asset, AssetKind::Image);
                    if let Some(t) = transition {
                        self.check_duration(id, t.duration);
                        if let Some(c) = &t.ease {
                            self.check_curve(CurveOwner::Node(id), c);
                        }
                    }
                }
                NodeKind::Show {
                    character,
                    expression,
                    position,
                    transition,
                    scale,
                    scale_y,
                    ..
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
                    if let Position::Free { x, y } = position
                        && !(FREE_POSITION_RANGE.contains(x) && FREE_POSITION_RANGE.contains(y))
                    {
                        self.errors.push(ValidationError::InvalidFreePosition {
                            node: id,
                            x: *x,
                            y: *y,
                        });
                    }
                    for s in [scale, scale_y].into_iter().flatten() {
                        if !scale_is_valid(*s) {
                            self.errors.push(ValidationError::InvalidScale {
                                node: Some(id),
                                scale: *s,
                            });
                        }
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
                        if let Some(c) = &t.ease {
                            self.check_curve(CurveOwner::Node(id), c);
                        }
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
                        if let Some(c) = &t.ease {
                            self.check_curve(CurveOwner::Node(id), c);
                        }
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
                NodeKind::Effect(_) | NodeKind::Timeline { .. } => self.check_animation(id, &node.kind),
                NodeKind::BackgroundLayers { layers, transition } => {
                    if layers.is_empty() {
                        self.stage_problem(id, "no layer");
                    }
                    if layers.len() > MAX_BG_LAYERS {
                        self.stage_problem(id, format!("more than {MAX_BG_LAYERS} layers"));
                    }
                    for layer in layers {
                        self.check_asset(Some(id), layer.asset, AssetKind::Image);
                        if !layer.depth.is_finite() || !(-4.0..=4.0).contains(&layer.depth) {
                            self.stage_problem(id, "a layer depth must be between -4 and 4");
                        }
                    }
                    if let Some(t) = transition {
                        self.check_duration(id, t.duration);
                        if let Some(c) = &t.ease {
                            self.check_curve(CurveOwner::Node(id), c);
                        }
                    }
                }
                NodeKind::Particles(emitter) => {
                    if let Some(message) = emitter.problem() {
                        self.stage_problem(id, message);
                    }
                    if let Some(sprite) = emitter.sprite {
                        self.check_asset(Some(id), sprite, AssetKind::Image);
                    }
                }
                NodeKind::StopParticles { name } if !name.is_empty() => {
                    let known = story.scenes.iter().flat_map(|s| &s.nodes).any(|n| {
                        matches!(&n.kind, NodeKind::Particles(e) if e.name == *name)
                    });
                    if !known {
                        self.warnings.push(ValidationWarning::UnknownEmitter {
                            node: id,
                            name: name.clone(),
                        });
                    }
                }
                NodeKind::OpenScreen { screen, params, .. } => {
                    let mut problems = Vec::new();
                    if screen.trim().is_empty() {
                        problems.push("no screen chosen".to_owned());
                    }
                    let mut names = HashSet::new();
                    for p in params {
                        if p.name.trim().is_empty() {
                            problems.push("a parameter has no name".to_owned());
                        } else if !names.insert(p.name.as_str()) {
                            problems.push(format!("parameter `{}` is given twice", p.name));
                        }
                        if p.value.is_contextual() {
                            problems.push(format!(
                                "parameter `{}` uses a value that only exists in a screen (item, parameter)",
                                p.name
                            ));
                        }
                    }
                    self.errors.extend(
                        problems
                            .into_iter()
                            .map(|message| ValidationError::InvalidScreenNode { node: id, message }),
                    );
                }
                NodeKind::Script {
                    source, branches, ..
                } => {
                    let mut problems = Vec::new();
                    if source.trim().is_empty() {
                        problems.push("the script is empty".to_owned());
                    }
                    let mut names = HashSet::new();
                    for b in branches {
                        if b.name.trim().is_empty() {
                            problems.push("a script branch has no name".to_owned());
                        } else if !names.insert(b.name.as_str()) {
                            problems.push(format!("script branch `{}` is defined twice", b.name));
                        }
                    }
                    self.errors.extend(
                        problems
                            .into_iter()
                            .map(|message| ValidationError::InvalidScript { node: id, message }),
                    );
                }
                NodeKind::CloseScreen { screen } if screen.trim().is_empty() => {
                    self.errors.push(ValidationError::InvalidScreenNode {
                        node: id,
                        message: "no screen chosen".to_owned(),
                    });
                }
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
        let story_vars: HashMap<&str, &Value> = self
            .story
            .variables
            .iter()
            .map(|v| (v.name.as_str(), &v.initial))
            .collect();
        let assigned: HashSet<&str> = flat
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::SetVar { var, .. } => Some(var.as_str()),
                _ => None,
            })
            .collect();
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

        // Rhai is only allowed as a whole condition (ADR 0015 §1).
        let rhai_misplaced = |errors: &mut Vec<ValidationError>, id: NodeId, e: &Expr, top: bool| {
            let nested = !matches!(e, Expr::Rhai(_)) && contains_rhai(e);
            if nested || (!top && matches!(e, Expr::Rhai(_))) {
                errors.push(ValidationError::InvalidScript {
                    node: id,
                    message: "a Rhai expression can only be a whole condition (If, choice option)"
                        .to_owned(),
                });
            }
        };
        for node in flat {
            match &node.kind {
                NodeKind::If { condition, .. } => {
                    rhai_misplaced(&mut errors, node.id, condition, true);
                    if let Expr::Rhai(code) = condition {
                        if code.trim().is_empty() {
                            errors.push(ValidationError::InvalidScript {
                                node: node.id,
                                message: "the Rhai condition is empty".to_owned(),
                            });
                        }
                    } else {
                        check_expr(&mut errors, &mut warnings, node.id, condition, true)
                    }
                }
                NodeKind::Choice { options } => {
                    for c in options.iter().filter_map(|o| o.condition.as_ref()) {
                        rhai_misplaced(&mut errors, node.id, c, true);
                        if let Expr::Rhai(code) = c {
                            if code.trim().is_empty() {
                                errors.push(ValidationError::InvalidScript {
                                    node: node.id,
                                    message: "the Rhai condition is empty".to_owned(),
                                });
                            }
                        } else {
                            check_expr(&mut errors, &mut warnings, node.id, c, true);
                        }
                    }
                }
                NodeKind::SetVar { var, value } => {
                    rhai_misplaced(&mut errors, node.id, value, false);
                    check_expr(&mut errors, &mut warnings, node.id, value, false);
                    if divides_by_zero(value, &story_vars, &assigned) {
                        warnings.push(ValidationWarning::DivisionByZero { node: node.id });
                    }
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
                NodeKind::OpenScreen { result, params, .. } => {
                    // The result keeps the declared type of its variable
                    // (coercion at run time): it must be declared.
                    if let Some(var) = result
                        && !declared.contains(var)
                    {
                        errors.push(ValidationError::InvalidScreenNode {
                            node: node.id,
                            message: format!("result variable `{var}` is not declared"),
                        });
                    }
                    for p in params {
                        if let crate::varops::Operand::Var(v) = &p.value
                            && !defined.contains(v)
                        {
                            warnings.push(ValidationWarning::UndefinedVariable {
                                node: node.id,
                                var: v.clone(),
                            });
                        }
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
                NodeKind::Say { .. }
                    | NodeKind::Choice { .. }
                    | NodeKind::Wait { .. }
                    | NodeKind::OpenScreen { wait: true, .. }
            )
        };
        // Scene of every node: a scene never falls into the next one.
        let mut scene_of: Vec<usize> = Vec::with_capacity(flat.len());
        for (si, scene) in self.story.scenes.iter().enumerate() {
            scene_of.extend(std::iter::repeat_n(si, scene.nodes.len()));
        }
        let story = self.story;
        // Where a jump to a scene lands (`None`: an empty scene, the end).
        let scene_entry = |scene: crate::ids::SceneId, label: Option<NodeId>| {
            story
                .scene_entry(scene, label)
                .and_then(|n| index.get(&n).copied())
        };
        let next_of =
            |i: usize| (i + 1 < flat.len() && scene_of[i + 1] == scene_of[i]).then_some(i + 1);
        // `strict` edges only: no fall-through after a Call (it is taken when
        // the callee returns, which cycle detection cannot see).
        let succ = |i: usize, with_call_continuation: bool| -> Vec<usize> {
            let node = flat[i];
            let next = next_of(i);
            let target = |t: NodeId| index.get(&t).copied();
            match &node.kind {
                NodeKind::Jump { target: t } => target(*t).into_iter().collect(),
                NodeKind::JumpScene { scene, label } => {
                    scene_entry(*scene, *label).into_iter().collect()
                }
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
                // A script goes to one of its branches, its error label or
                // the next node.
                NodeKind::Script { .. } => {
                    let mut v: Vec<usize> =
                        node.kind.targets().into_iter().filter_map(target).collect();
                    v.extend(next);
                    v
                }
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

        // Scenes whose end is reached: the game ends there.
        let mut ends: Vec<(crate::ids::SceneId, Option<NodeId>)> = Vec::new();
        for (i, node) in flat.iter().enumerate().filter(|(i, _)| reached[*i]) {
            let falls_off = next_of(i).is_none()
                && !matches!(
                    node.kind,
                    NodeKind::Jump { .. }
                        | NodeKind::JumpScene { .. }
                        | NodeKind::Choice { .. }
                        | NodeKind::End
                        | NodeKind::Return
                        | NodeKind::If {
                            else_target: Some(_),
                            ..
                        }
                );
            if falls_off {
                ends.push((self.story.scenes[scene_of[i]].id, Some(node.id)));
            }
            // A jump into an empty scene ends there too.
            if let NodeKind::JumpScene { scene, label: None } = &node.kind
                && self.story.scene(*scene).is_some_and(|s| s.nodes.is_empty())
            {
                ends.push((*scene, None));
            }
        }
        let mut seen_scenes = HashSet::new();
        for (scene, node) in ends {
            if seen_scenes.insert(scene) {
                self.warnings
                    .push(ValidationWarning::SceneEndsGame { scene, node });
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

/// Whether a Rhai expression appears anywhere in `expr`.
fn contains_rhai(expr: &Expr) -> bool {
    match expr {
        Expr::Rhai(_) => true,
        Expr::Lit(_) | Expr::Var(_) => false,
        Expr::Unary(_, e) => contains_rhai(e),
        Expr::Binary(_, l, r) | Expr::Index(l, r) => contains_rhai(l) || contains_rhai(r),
        Expr::List(items) | Expr::Call(_, items) => items.iter().any(contains_rhai),
        Expr::Map(entries) => entries.iter().any(|(_, e)| contains_rhai(e)),
    }
}

/// Whether `expr` divides by something statically zero: a literal `0` /
/// `0.0`, or a variable declared at 0 that no node ever assigns.
fn divides_by_zero(expr: &Expr, vars: &HashMap<&str, &Value>, assigned: &HashSet<&str>) -> bool {
    let is_zero = |e: &Expr| match e {
        Expr::Lit(Value::Int(0)) => true,
        Expr::Lit(Value::Float(f)) => *f == 0.0,
        Expr::Var(n) if !assigned.contains(n.as_str()) => match vars.get(n.as_str()) {
            Some(Value::Int(0)) => true,
            Some(Value::Float(f)) => *f == 0.0,
            _ => false,
        },
        _ => false,
    };
    match expr {
        Expr::Binary(op, l, r) => {
            (*op == BinOp::Div && is_zero(r))
                || divides_by_zero(l, vars, assigned)
                || divides_by_zero(r, vars, assigned)
        }
        Expr::Unary(_, e) => divides_by_zero(e, vars, assigned),
        Expr::Index(c, i) => {
            divides_by_zero(c, vars, assigned) || divides_by_zero(i, vars, assigned)
        }
        Expr::List(items) | Expr::Call(_, items) => {
            items.iter().any(|e| divides_by_zero(e, vars, assigned))
        }
        Expr::Map(entries) => entries.iter().any(|(_, e)| divides_by_zero(e, vars, assigned)),
        Expr::Lit(_) | Expr::Var(_) | Expr::Rhai(_) => false,
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
            default_scale: 1.0,
            id,
            name_id,
            color: Color::WHITE,
            expressions: vec![Expression {
                name: "happy".into(),
                image: img,
                motion: None,
            }],
            default_position: None,
        });
        (id, img)
    }

    #[test]
    fn valid_story_has_no_errors() {
        let mut s = Story::new("t");
        let l = label();
        let j = node(NodeKind::Jump { target: l.id });
        let say = say(&mut s);
        let s = with_nodes(s, vec![say, j, l, node(NodeKind::End)]);
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
    fn scale_and_free_position_are_validated() {
        let show = |ana, position: Position, scale, scale_y| {
            node(NodeKind::Show {
                character: ana,
                expression: "happy".into(),
                position,
                transition: None,
                scale,
                scale_y,
                flip_x: true,
                flip_y: true,
            })
        };
        let mut s = Story::new("t");
        let (ana, _) = character(&mut s);
        let ok = show(
            ana,
            Position::Free { x: 0.4, y: 0.95 },
            Some(0.5),
            Some(1.5),
        );
        // Off screen, but inside the allowed range: a character walking in.
        let walking_in = show(ana, Position::Free { x: -0.3, y: 1.0 }, None, None);
        let bad_pos = [
            Position::Free {
                x: f32::NAN,
                y: 0.5,
            },
            Position::Free {
                x: 0.5,
                y: f32::INFINITY,
            },
            Position::Free { x: 4.0, y: 0.5 },
        ]
        .map(|p| show(ana, p, None, None));
        let bad_scales =
            [0.0, -1.0, f32::NAN, 0.01, 11.0].map(|v| show(ana, Position::Left, Some(v), None));
        let bad_scale_y = show(ana, Position::Left, Some(1.0), Some(0.0));
        let mut nodes = vec![ok, walking_in, bad_scale_y];
        nodes.extend(bad_pos);
        nodes.extend(bad_scales);
        let errors = validate(&with_nodes(s.clone(), nodes)).errors;
        let count = |f: fn(&ValidationError) -> bool| errors.iter().filter(|e| f(e)).count();
        assert_eq!(
            count(|e| matches!(e, ValidationError::InvalidFreePosition { .. })),
            3,
            "{errors:?}"
        );
        assert_eq!(
            count(|e| matches!(e, ValidationError::InvalidScale { node: Some(_), .. })),
            6,
            "{errors:?}"
        );
        // A character's default scale is checked too.
        s.characters[0].default_scale = 0.0;
        let errors = validate(&s).errors;
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::InvalidScale { node: None, .. }))
        );
    }

    #[test]
    fn free_positions_resolve_without_a_declaration() {
        let s = Story::new("t");
        assert_eq!(
            s.resolve_position(&Position::Free { x: 0.1, y: 0.2 }),
            Some((0.1, 0.2))
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
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
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
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
            character: ana,
            expression: "sad".into(),
            position: Position::Left,
            transition: None,
        });
        let bad_pos = node(NodeKind::Show {
            scale: None,
            scale_y: None,
            flip_x: false,
            flip_y: false,
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
    fn scene3d_commands_are_checked() {
        let mut s = Story::new("t");
        let (_, img) = character(&mut s);
        let room = asset(&mut s, AssetKind::Scene3d, "assets/3d/room.scene3d.ron");
        let ok = node(NodeKind::ShowScene3d {
            scene: room,
            camera: Some("a".into()),
            transition: Some(Transition::dissolve(0.5)),
        });
        let image_as_scene = node(NodeKind::ShowScene3d {
            scene: img,
            camera: None,
            transition: None,
        });
        let unknown = node(NodeKind::ShowScene3d {
            scene: AssetId::new(),
            camera: None,
            transition: None,
        });
        let bad_time = node(NodeKind::Camera3d {
            camera: "a".into(),
            transition: Some(Transition::dissolve(-1.0)),
        });
        let hide = node(NodeKind::HideScene3d { transition: None });
        let s = with_nodes(s, vec![ok, image_as_scene, unknown, bad_time, hide]);
        let errs = validate(&s).errors;
        assert_eq!(errs.len(), 3, "{errs:#?}");
        assert!(matches!(errs[0], ValidationError::AssetKindMismatch { .. }));
        assert!(matches!(errs[1], ValidationError::UnknownAsset { .. }));
        assert!(matches!(errs[2], ValidationError::InvalidDuration { .. }));
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
            motion: None,
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
            s.variables.push(VarDecl::game("v", Value::Int(0)));
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
        s.variables.push(VarDecl::game("n", Value::Int(0)));
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
            vec![
                bad_cond,
                bad_assign,
                type_err,
                undefined,
                ok_numeric,
                l,
                node(NodeKind::End),
            ],
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
    fn static_division_by_zero_is_a_warning() {
        let div = |d: Expr| {
            node(NodeKind::SetVar {
                var: "x".into(),
                value: Expr::Binary(BinOp::Div, Box::new(Expr::lit(10i64)), Box::new(d)),
            })
        };
        let mut s = Story::new("t");
        s.variables.push(VarDecl::game("zero", Value::Int(0)));
        s.variables.push(VarDecl::game("two", Value::Int(2)));
        s.variables.push(VarDecl::game("moves", Value::Int(0)));
        s.variables.push(VarDecl::game("x", Value::Int(0)));
        let nodes = vec![
            div(Expr::lit(0i64)),
            div(Expr::var("zero")),
            div(Expr::var("two")),
            div(Expr::var("moves")),
            node(NodeKind::SetVar {
                var: "moves".into(),
                value: Expr::lit(3i64),
            }),
            node(NodeKind::End),
        ];
        let ids: Vec<_> = nodes.iter().map(|n| n.id).collect();
        let report = validate(&with_nodes(s, nodes));
        let flagged: Vec<_> = report
            .warnings
            .iter()
            .filter_map(|w| match w {
                ValidationWarning::DivisionByZero { node } => Some(*node),
                _ => None,
            })
            .collect();
        assert_eq!(flagged, vec![ids[0], ids[1]], "{report:?}");
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
        let s = with_nodes(s, vec![set, iff, sy, l, node(NodeKind::End)]);
        let report = validate(&s);
        assert!(report.is_ok(), "{report:?}");
        assert!(report.warnings.is_empty(), "{report:?}");
    }

    fn scene(name: &str, nodes: Vec<Node>) -> Scene {
        Scene {
            id: SceneId::new(),
            name: name.into(),
            nodes,
        }
    }

    #[test]
    fn jump_scene_targets_are_checked() {
        let mut s = Story::new("t");
        let inner = label();
        let b = scene("b", vec![inner.clone(), node(NodeKind::End)]);
        let other_label = label();
        let c = scene("c", vec![other_label.clone(), node(NodeKind::End)]);
        let ghost_scene = SceneId::new();
        let ghost_label = NodeId::new();
        let to_ghost = node(NodeKind::JumpScene {
            scene: ghost_scene,
            label: None,
        });
        let wrong_label = node(NodeKind::JumpScene {
            scene: b.id,
            label: Some(other_label.id),
        });
        let missing_label = node(NodeKind::JumpScene {
            scene: b.id,
            label: Some(ghost_label),
        });
        let fine = node(NodeKind::JumpScene {
            scene: b.id,
            label: Some(inner.id),
        });
        let (g, w, m) = (to_ghost.id, wrong_label.id, missing_label.id);
        s.scenes
            .push(scene("a", vec![to_ghost, wrong_label, missing_label, fine]));
        s.scenes.push(b.clone());
        s.scenes.push(c);
        let errors = validate(&s).errors;
        assert!(errors.contains(&ValidationError::UnknownScene {
            node: g,
            scene: ghost_scene
        }));
        assert!(errors.contains(&ValidationError::LabelNotInScene {
            node: w,
            label: other_label.id,
            scene: b.id
        }));
        assert!(errors.contains(&ValidationError::UnknownTarget {
            from: m,
            target: ghost_label
        }));
        assert_eq!(errors.len(), 3, "{errors:?}");
    }

    #[test]
    fn scenes_do_not_fall_into_each_other() {
        let mut s = Story::new("t");
        let first = say(&mut s);
        let first_id = first.id;
        let second = say(&mut s);
        let a = scene("a", vec![first]);
        let b = scene("b", vec![second]);
        s.scenes.push(a.clone());
        s.scenes.push(b.clone());
        let report = validate(&s);
        assert!(report.is_ok(), "{report:?}");
        // `a` ends the game, and `b` is never reached.
        assert!(report.warnings.contains(&ValidationWarning::SceneEndsGame {
            scene: a.id,
            node: Some(first_id)
        }));
        assert!(
            report
                .warnings
                .contains(&ValidationWarning::UnreachableNode(b.nodes[0].id))
        );
        assert!(
            !report.warnings.iter().any(
                |w| matches!(w, ValidationWarning::SceneEndsGame { scene, .. } if *scene == b.id)
            ),
            "an unreachable scene does not end the game"
        );
    }

    #[test]
    fn a_jump_scene_reaches_the_target_and_ends_the_flow() {
        let mut s = Story::new("t");
        let l = label();
        let say_b = say(&mut s);
        let say_b_id = say_b.id;
        let b = scene("b", vec![say_b, l.clone(), node(NodeKind::End)]);
        let a = scene(
            "a",
            vec![node(NodeKind::JumpScene {
                scene: b.id,
                label: None,
            })],
        );
        let c = scene(
            "c",
            vec![node(NodeKind::JumpScene {
                scene: b.id,
                label: Some(l.id),
            })],
        );
        s.scenes = vec![a, b, c];
        let report = validate(&s);
        assert!(report.is_ok(), "{report:?}");
        // Scene b is reached, a ends with the jump; only `c` is never reached.
        assert_eq!(
            report.warnings,
            vec![ValidationWarning::UnreachableNode(s.scenes[2].nodes[0].id)]
        );
        let _ = say_b_id;
    }

    #[test]
    fn a_jump_into_an_empty_scene_ends_the_game() {
        let mut s = Story::new("t");
        let empty = scene("vide", Vec::new());
        let a = scene(
            "a",
            vec![node(NodeKind::JumpScene {
                scene: empty.id,
                label: None,
            })],
        );
        s.scenes = vec![a, empty.clone()];
        let report = validate(&s);
        assert!(report.is_ok(), "{report:?}");
        assert_eq!(
            report.warnings,
            vec![ValidationWarning::SceneEndsGame {
                scene: empty.id,
                node: None
            }]
        );
    }

    #[test]
    fn an_if_without_else_at_the_end_of_a_scene_ends_the_game() {
        let mut s = Story::new("t");
        let l = label();
        let iff = node(NodeKind::If {
            condition: Expr::lit(true),
            then_target: l.id,
            else_target: None,
        });
        let iff_id = iff.id;
        let a = scene("a", vec![l, node(NodeKind::End), iff]);
        // The `If` is dead code after the End: unreachable, not an ending.
        s.scenes = vec![a.clone()];
        let report = validate(&s);
        assert_eq!(
            report.warnings,
            vec![ValidationWarning::UnreachableNode(iff_id)]
        );
        // Reached, it falls off the end of the scene.
        let l2 = label();
        let iff2 = node(NodeKind::If {
            condition: Expr::lit(true),
            then_target: l2.id,
            else_target: None,
        });
        let iff2_id = iff2.id;
        let mut s2 = Story::new("t");
        s2.scenes = vec![
            scene("b", vec![iff2]),
            scene("c", vec![l2, node(NodeKind::End)]),
        ];
        assert_eq!(
            validate(&s2).warnings,
            vec![ValidationWarning::SceneEndsGame {
                scene: s2.scenes[0].id,
                node: Some(iff2_id)
            }]
        );
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

    fn with_motion(motion: crate::defs::Motion) -> ValidationReport {
        let mut s = Story::new("t");
        character(&mut s);
        s.characters[0].expressions[0].motion = Some(motion);
        let l = label();
        let j = node(NodeKind::Jump { target: l.id });
        let say = say(&mut s);
        let s = with_nodes(s, vec![say, j, l, node(NodeKind::End)]);
        validate(&s)
    }

    fn sheet(
        cols: u32,
        rows: u32,
        frame_count: u32,
        fps: f32,
    ) -> crate::defs::Motion {
        use crate::defs::{AnimFrames, Animation, LoopMode, Motion};
        Motion::Animation(Animation {
            frames: AnimFrames::SpriteSheet {
                asset: AssetId::new(),
                cols,
                rows,
                frame_count,
                frame_size: None,
            },
            fps,
            loop_mode: LoopMode::Loop,
        })
    }

    #[test]
    fn animation_problems_are_reported() {
        use crate::error::AnimationIssue as I;
        let issues = |m| -> Vec<ValidationError> {
            with_motion(m)
                .errors
                .into_iter()
                .filter(|e| matches!(e, ValidationError::InvalidAnimation { .. }))
                .collect()
        };
        let issue_of = |e: &ValidationError| match e {
            ValidationError::InvalidAnimation { issue, .. } => issue.clone(),
            _ => unreachable!(),
        };
        let bad_fps = issues(sheet(2, 2, 4, 0.0));
        assert!(matches!(issue_of(&bad_fps[0]), I::BadFps(_)));
        let too_many = issues(sheet(2, 2, 5, 12.0));
        assert!(matches!(
            issue_of(&too_many[0]),
            I::FrameCountExceedsGrid { capacity: 4, .. }
        ));
        assert!(matches!(
            issue_of(&issues(sheet(0, 2, 1, 12.0))[0]),
            I::EmptyGrid
        ));
        assert!(matches!(
            issue_of(&issues(sheet(2, 2, 0, 12.0))[0]),
            I::NoFrames
        ));
        let empty = crate::defs::Motion::Animation(crate::defs::Animation {
            frames: crate::defs::AnimFrames::Sequence(vec![]),
            fps: 12.0,
            loop_mode: Default::default(),
        });
        assert!(matches!(issue_of(&issues(empty)[0]), I::EmptySequence));
        // A valid grid only complains about the unknown asset id we invented.
        let ok = with_motion(sheet(2, 2, 4, 12.0));
        assert_eq!(ok.errors.len(), 1, "{ok:?}");
        assert!(matches!(ok.errors[0], ValidationError::UnknownAsset { .. }));
    }

    #[test]
    fn animation_frames_must_be_known_images() {
        use crate::defs::{AnimFrames, Animation, Motion};
        let mut s = Story::new("t");
        let (_, img) = character(&mut s);
        let music = asset(&mut s, AssetKind::Music, "m.ogg");
        s.characters[0].expressions[0].motion = Some(Motion::Animation(Animation {
            frames: AnimFrames::Sequence(vec![img, music]),
            fps: 8.0,
            loop_mode: Default::default(),
        }));
        let l = label();
        let j = node(NodeKind::Jump { target: l.id });
        let say = say(&mut s);
        let s = with_nodes(s, vec![say, j, l, node(NodeKind::End)]);
        let report = validate(&s);
        assert_eq!(report.errors.len(), 1, "{report:?}");
        assert!(matches!(
            report.errors[0],
            ValidationError::AssetKindMismatch { .. }
        ));
    }

    #[test]
    fn rigged_needs_a_rig_asset_and_a_clip() {
        use crate::defs::{LoopMode, Motion, RiggedMotion};
        let rigged = |rig, clip: &str| {
            Motion::Rigged(RiggedMotion {
                rig,
                skin: None,
                clip: clip.into(),
                loop_mode: LoopMode::Loop,
            })
        };
        // Unknown asset.
        let report = with_motion(rigged(AssetId::new(), "idle"));
        assert!(matches!(report.errors[0], ValidationError::UnknownAsset { .. }));
        // Right kind of asset, empty clip name.
        let mut s = Story::new("t");
        character(&mut s);
        let rig = asset(&mut s, AssetKind::Rig, "assets/rigs/a.rig.ron");
        s.characters[0].expressions[0].motion = Some(rigged(rig, " "));
        let l = label();
        let j = node(NodeKind::Jump { target: l.id });
        let say = say(&mut s);
        let mut s = with_nodes(s, vec![say, j, l, node(NodeKind::End)]);
        let report = validate(&s);
        assert!(matches!(
            report.errors[0],
            ValidationError::InvalidAnimation { .. }
        ));
        // All good.
        s.characters[0].expressions[0].motion = Some(rigged(rig, "idle"));
        assert!(validate(&s).is_ok());
    }
}
