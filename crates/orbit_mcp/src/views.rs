//! Read-only views of the project, written for an AI reader: scene listings
//! with ids and versions, JSON overviews, validation reports, simulation.

use std::fmt::Write as _;
use std::sync::Arc;

use orbit_core::{
    AssetKind, Node, NodeKind, Position, Scene, Story, Transition, TransitionKind, ValidationError,
    ValidationWarning,
};
use orbit_plugin_api::ProjectRegistry;
use orbit_project::Project;
use orbit_vm::{PresentationEvent, StepResult, Vm};
use serde_json::{Value as Json, json};

use crate::convert::{character_name, custom_to_json};
use crate::error::ToolError;

fn quote(text: &str) -> String {
    format!("{text:?}")
}

fn describe_transition(t: &Option<Transition>) -> String {
    match t {
        None => String::new(),
        Some(t) => {
            let ease = match &t.ease {
                None => String::new(),
                Some(orbit_core::Curve::Preset(e)) => format!(" eased {}", e.name()),
                Some(orbit_core::Curve::Bezier { p1, p2 }) => format!(
                    " eased cubic-bezier({}, {}, {}, {})",
                    p1[0], p1[1], p2[0], p2[1]
                ),
                Some(orbit_core::Curve::Named(n)) => format!(" eased `{n}`"),
            };
            match t.kind {
                TransitionKind::Dissolve => format!(" with dissolve({}s){ease}", t.duration),
                TransitionKind::Fade(c) => {
                    format!(
                        " with fade(#{:02x}{:02x}{:02x}, {}s){ease}",
                        c.r, c.g, c.b, t.duration
                    )
                }
            }
        }
    }
}

/// The story's named easing curves, with how many transitions use each.
pub fn curves_json(story: &Story) -> Json {
    Json::Array(
        story
            .curves
            .iter()
            .map(|(name, curve)| {
                let used_by = orbit_project::ops::curve_users(story, name).len();
                let definition = match curve {
                    orbit_core::Curve::Preset(e) => json!({"ease": e.name()}),
                    orbit_core::Curve::Bezier { p1, p2 } => {
                        json!({"bezier": [p1[0], p1[1], p2[0], p2[1]]})
                    }
                    orbit_core::Curve::Named(n) => json!({"named": n}),
                };
                json!({"name": name, "curve": definition, "used_by": used_by})
            })
            .collect(),
    )
}

fn describe_position(p: &Position) -> String {
    match p {
        Position::Left => "left".to_owned(),
        Position::Center => "center".to_owned(),
        Position::Right => "right".to_owned(),
        Position::Custom(name) => name.clone(),
        Position::Free { x, y } => format!("free({x:.3}, {y:.3})"),
    }
}

/// ` scale 0.8 mirrored-x` style suffix of a `Show` (empty for the standard
/// look), shared by the node and event descriptions.
fn describe_look(scale: [f32; 2], flip_x: bool, flip_y: bool) -> String {
    let mut out = String::new();
    if scale[0] != 1.0 || scale[1] != 1.0 {
        if scale[0] == scale[1] {
            out.push_str(&format!(" scale {}", scale[0]));
        } else {
            out.push_str(&format!(" scale {}x{}", scale[0], scale[1]));
        }
    }
    if flip_x {
        out.push_str(" mirrored-x");
    }
    if flip_y {
        out.push_str(" mirrored-y");
    }
    out
}

fn asset_label(story: &Story, id: orbit_core::AssetId) -> String {
    story
        .asset_path(id)
        .map_or_else(|| format!("{id} (unknown)"), |p| format!("{p} ({id})"))
}

fn label_ref(story: &Story, id: orbit_core::NodeId) -> String {
    match story.node(id).map(|n| &n.kind) {
        Some(NodeKind::Label { name }) => format!("label {} ({id})", quote(name)),
        Some(_) => format!("{id} (not a label)"),
        None => format!("{id} (missing)"),
    }
}

/// One-line, human-readable description of a node's content.
pub fn describe_kind(story: &Story, kind: &NodeKind) -> String {
    match kind {
        NodeKind::Label { name } => format!("label {}", quote(name)),
        NodeKind::Say { speaker, text_id } => {
            let who = speaker.map_or_else(|| "narrator".to_owned(), |c| character_name(story, c));
            format!(
                "say [{who}]: {}",
                quote(story.text(*text_id).unwrap_or("<missing text>"))
            )
        }
        NodeKind::Choice { options } => {
            let items: Vec<String> = options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let cond = o
                        .condition
                        .as_ref()
                        .map_or_else(String::new, |c| format!(" if {c}"));
                    format!(
                        "[{i}] {} -> {}{cond}",
                        quote(story.text(o.text_id).unwrap_or("<missing text>")),
                        label_ref(story, o.target)
                    )
                })
                .collect();
            format!("choice {}", items.join(" | "))
        }
        NodeKind::Jump { target } => format!("jump -> {}", label_ref(story, *target)),
        NodeKind::JumpScene { scene, label } => {
            let name = story.scene(*scene).map_or_else(
                || format!("{scene} (missing)"),
                |s| format!("{} ({scene})", quote(&s.name)),
            );
            match label {
                Some(l) => format!("jump_scene -> scene {name} at {}", label_ref(story, *l)),
                None => format!("jump_scene -> scene {name}"),
            }
        }
        NodeKind::SetVar { var, value } => format!("set {var} = {value}"),
        NodeKind::If {
            condition,
            then_target,
            else_target,
        } => {
            let else_part = else_target
                .map_or_else(String::new, |e| format!(" else -> {}", label_ref(story, e)));
            format!(
                "if {condition} then -> {}{else_part}",
                label_ref(story, *then_target)
            )
        }
        NodeKind::Background { asset, transition } => {
            format!(
                "background {}{}",
                asset_label(story, *asset),
                describe_transition(transition)
            )
        }
        NodeKind::Show {
            character,
            expression,
            position,
            transition,
            scale,
            scale_y,
            flip_x,
            flip_y,
        } => {
            // The override is shown as written; no override means the
            // character's default scale applies.
            let look = scale.map_or_else(String::new, |s| {
                describe_look([s, scale_y.unwrap_or(s)], false, false)
            }) + &describe_look([1.0, 1.0], *flip_x, *flip_y);
            format!(
                "show {} ({expression}) at {}{look}{}",
                character_name(story, *character),
                describe_position(position),
                describe_transition(transition)
            )
        }
        NodeKind::Hide {
            character,
            transition,
        } => format!(
            "hide {}{}",
            character_name(story, *character),
            describe_transition(transition)
        ),
        NodeKind::PlayMusic {
            asset,
            fade_in,
            looping,
        } => format!(
            "play_music {} (fade_in {fade_in}s, {})",
            asset_label(story, *asset),
            if *looping { "looping" } else { "once" }
        ),
        NodeKind::StopMusic { fade_out } => format!("stop_music (fade_out {fade_out}s)"),
        NodeKind::PlaySound { asset } => format!("play_sound {}", asset_label(story, *asset)),
        NodeKind::Wait { seconds } => format!("wait {seconds}s"),
        NodeKind::Call { label } => format!("call {}", label_ref(story, *label)),
        NodeKind::Return => "return".to_owned(),
        NodeKind::End => "end".to_owned(),
        NodeKind::Custom {
            plugin_id,
            kind,
            version,
            data,
        } => format!(
            "custom {plugin_id}/{kind} v{version} {}",
            custom_to_json(data)
        ),
        NodeKind::ShowScene3d {
            scene,
            camera,
            transition,
        } => format!(
            "3d scene {}{}{}",
            asset_label(story, *scene),
            camera
                .as_ref()
                .map_or_else(String::new, |c| format!(" camera {c}")),
            describe_transition(transition)
        ),
        NodeKind::Camera3d { camera, transition } => {
            format!("3d camera {camera}{}", describe_transition(transition))
        }
        NodeKind::HideScene3d { transition } => {
            format!("hide 3d scene{}", describe_transition(transition))
        }
    }
}

/// Lines of a scene: `index  node_id  vVERSION  description`. The version is
/// what `expected_version` expects.
pub fn render_scene(project: &Project, scene: &Scene) -> String {
    let story = project.story();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "scene {} (id {}), {} node(s). Columns: index, node id, version, content.",
        quote(&scene.name),
        scene.id,
        scene.nodes.len()
    );
    for (i, node) in scene.nodes.iter().enumerate() {
        let _ = writeln!(
            out,
            "{i:>3}  {}  v{}  {}",
            node.id,
            project.node_version(node.id).unwrap_or(0),
            describe_kind(story, &node.kind)
        );
    }
    out
}

pub fn scenes_json(story: &Story) -> Json {
    Json::Array(
        story
            .scenes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                json!({"index": i, "id": s.id.to_string(), "name": s.name, "nodes": s.nodes.len()})
            })
            .collect(),
    )
}

pub fn characters_json(story: &Story) -> Json {
    Json::Array(
        story
            .characters
            .iter()
            .map(|c| {
                json!({
                    "id": c.id.to_string(),
                    "name": story.text(c.name_id).unwrap_or(""),
                    "color": format!("#{:02x}{:02x}{:02x}", c.color.r, c.color.g, c.color.b),
                    // Through the shortest decimal form: 0.8f32 is not 0.800000011920929.
                    "default_scale": c.default_scale.to_string().parse::<f64>().unwrap_or(1.0),
                    "expressions": c.expressions.iter().map(|e| json!({
                        "name": e.name,
                        "asset": e.image.to_string(),
                        "path": story.asset_path(e.image),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

pub fn variables_json(story: &Story) -> Json {
    Json::Array(
        story
            .variables
            .iter()
            .map(|v| {
                json!({
                    "name": v.name,
                    "type": v.initial.value_type().to_string(),
                    "initial": match &v.initial {
                        orbit_core::Value::Bool(b) => json!(b),
                        orbit_core::Value::Int(i) => json!(i),
                        orbit_core::Value::Float(f) => json!(f),
                        orbit_core::Value::Str(s) => json!(s),
                    },
                    "scope": match v.scope {
                        orbit_core::VarScope::Game => "game",
                        orbit_core::VarScope::Persistent => "persistent",
                    },
                })
            })
            .collect(),
    )
}

pub fn assets_json(story: &Story, kind: Option<AssetKind>) -> Json {
    Json::Array(
        story
            .assets
            .assets
            .iter()
            .filter(|a| kind.is_none_or(|k| k == a.kind))
            .map(|a| json!({"id": a.id.to_string(), "kind": a.kind.to_string(), "path": a.path}))
            .collect(),
    )
}

/// Outcome of a validation run.
pub struct Validation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Validation {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn to_json(&self) -> Json {
        json!({
            "ok": self.is_ok(),
            "error_count": self.errors.len(),
            "warning_count": self.warnings.len(),
            "errors": self.errors,
            "warnings": self.warnings,
        })
    }
}

/// Core validation, plus plugin validators, missing plugins and (when
/// `file_exists` is given) asset file checks.
pub fn validate_project(
    story: &Story,
    registry: &ProjectRegistry,
    file_exists: Option<&dyn Fn(&str) -> bool>,
) -> Validation {
    let report = orbit_core::validate(story);
    let mut errors: Vec<String> = report
        .errors
        .iter()
        .map(ValidationError::to_string)
        .collect();
    let mut warnings: Vec<String> = report
        .warnings
        .iter()
        .map(ValidationWarning::to_string)
        .collect();
    if let Some(exists) = file_exists {
        errors.extend(
            orbit_core::check_asset_files(story, exists)
                .iter()
                .map(ValidationError::to_string),
        );
    }
    errors.extend(
        registry
            .validate_custom(story)
            .into_iter()
            .map(|(node, message)| format!("node {node}: {message}")),
    );
    for plugin in registry.missing_plugins(story) {
        warnings.push(format!(
            "plugin `{plugin}` is not installed: its custom nodes are kept untouched"
        ));
    }
    Validation { errors, warnings }
}

/// One choice made while simulating: an option index or (a prefix of) its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoicePick {
    Index(usize),
    Text(String),
}

pub struct SimulationLimits {
    pub max_steps: usize,
}

/// Plays the story headlessly. Returns the transcript.
pub fn simulate(
    story: &Story,
    picks: &[ChoicePick],
    start_at: Option<orbit_core::NodeId>,
    limits: &SimulationLimits,
) -> Result<String, ToolError> {
    let report = orbit_core::validate(story);
    if !report.errors.is_empty() {
        let shown: Vec<String> = report
            .errors
            .iter()
            .take(5)
            .map(ToString::to_string)
            .collect();
        return Err(ToolError::refused(format!(
            "the story has {} validation error(s), fix them first (call validate): {}",
            report.errors.len(),
            shown.join("; ")
        )));
    }
    let story = Arc::new(story.clone());
    let vm = match start_at {
        Some(node) => Vm::new(Arc::clone(&story), node),
        None => Vm::from_start(Arc::clone(&story)),
    };
    let mut vm = vm.map_err(|e| ToolError::refused(format!("cannot start the story: {e}")))?;

    let mut out = String::new();
    let mut picks = picks.iter();
    let mut ended = "step limit reached";
    let mut chosen = 0usize;
    for _ in 0..limits.max_steps {
        let result = vm.step();
        for event in vm.take_events() {
            let _ = writeln!(out, "  ({})", describe_event(&story, &event));
        }
        match result {
            StepResult::Say {
                speaker, string_id, ..
            } => {
                let who =
                    speaker.map_or_else(|| "narrator".to_owned(), |c| character_name(&story, c));
                let _ = writeln!(out, "{who}: {}", vm.text(string_id).unwrap_or(""));
            }
            StepResult::Choice { options, .. } => {
                let texts: Vec<String> = options
                    .iter()
                    .map(|o| vm.text(o.string_id).unwrap_or("").to_owned())
                    .collect();
                let listing: Vec<String> = texts
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("[{i}] {t}"))
                    .collect();
                let _ = writeln!(out, "? choice: {}", listing.join("  "));
                let Some(pick) = picks.next() else {
                    ended = "awaiting a choice (pass more entries in `choices`)";
                    break;
                };
                let index = match pick {
                    ChoicePick::Index(i) => *i,
                    ChoicePick::Text(t) => {
                        let wanted = t.to_lowercase();
                        texts
                            .iter()
                            .position(|x| x.to_lowercase() == wanted)
                            .or_else(|| {
                                texts
                                    .iter()
                                    .position(|x| x.to_lowercase().starts_with(&wanted))
                            })
                            .ok_or_else(|| {
                                ToolError::invalid(format!(
                                    "choice text `{t}` matches no option ({})",
                                    listing.join("  ")
                                ))
                            })?
                    }
                };
                if index >= texts.len() {
                    return Err(ToolError::invalid(format!(
                        "choice index {index} is out of range ({})",
                        listing.join("  ")
                    )));
                }
                vm.choose(index)
                    .map_err(|e| ToolError::refused(format!("cannot choose: {e}")))?;
                chosen += 1;
                let _ = writeln!(out, "> chose [{index}] {}", texts[index]);
            }
            StepResult::WaitTime(_) | StepResult::WaitTransition { .. } => {}
            StepResult::End => {
                ended = "story ended";
                break;
            }
            StepResult::Error(e) => {
                let _ = writeln!(out, "!! runtime error: {e}");
                ended = "runtime error";
                break;
            }
        }
    }
    let unused = picks.count();
    let _ = writeln!(out, "-- {ended} ({chosen} choice(s) made) --");
    if unused > 0 {
        let _ = writeln!(
            out,
            "note: {unused} entry(ies) of `choices` were not used (the story ended earlier)"
        );
    }
    let vars: Vec<String> = vm
        .variables()
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    if !vars.is_empty() {
        let _ = writeln!(out, "variables: {}", vars.join(", "));
    }
    Ok(out)
}

fn describe_event(story: &Story, event: &PresentationEvent) -> String {
    match event {
        PresentationEvent::Background { asset, transition } => format!(
            "background {}{}",
            asset_label(story, *asset),
            describe_transition(transition)
        ),
        PresentationEvent::Show {
            character,
            expression,
            position,
            transition,
            scale,
            flip_x,
            flip_y,
            ..
        } => format!(
            "show {} ({expression}) at {}{}{}",
            character_name(story, *character),
            describe_position(position),
            describe_look(*scale, *flip_x, *flip_y),
            describe_transition(transition)
        ),
        PresentationEvent::Hide {
            character,
            transition,
        } => format!(
            "hide {}{}",
            character_name(story, *character),
            describe_transition(transition)
        ),
        PresentationEvent::PlayMusic { asset, .. } => {
            format!("music {}", asset_label(story, *asset))
        }
        PresentationEvent::StopMusic { .. } => "music stops".to_owned(),
        PresentationEvent::PlaySound { asset } => format!("sound {}", asset_label(story, *asset)),
        PresentationEvent::Custom {
            plugin_id, kind, ..
        } => format!("custom {plugin_id}/{kind}"),
        PresentationEvent::Scene3d {
            scene,
            camera,
            transition,
        } => format!(
            "3d scene {}{}{}",
            asset_label(story, *scene),
            camera
                .as_ref()
                .map_or_else(String::new, |c| format!(" camera {c}")),
            describe_transition(transition)
        ),
        PresentationEvent::HideScene3d { transition } => {
            format!("hide 3d scene{}", describe_transition(transition))
        }
    }
}

/// Used by tools that echo the position of what they created.
pub fn node_json(project: &Project, node: &Node, index: usize) -> Json {
    json!({
        "id": node.id.to_string(),
        "version": project.node_version(node.id),
        "index": index,
        "content": describe_kind(project.story(), &node.kind),
    })
}
