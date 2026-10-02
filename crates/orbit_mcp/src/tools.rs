//! The built-in MCP tools. Every edit goes through `orbit_project` commands.

use std::fs;

use orbit_core::{
    AssetEntry, AssetId, AssetKind, Character, CharacterId, ChoiceOption, Expr, Expression, Node,
    NodeId, NodeKind, Scene, SceneId, StringId, Transition, VarDecl, VarScope,
};
use orbit_plugin_api::ProjectRegistry;
use orbit_project::{Command, Project, ProjectManifest};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value as Json, json};

use crate::convert::{
    build_transition, parse_curve, check_identifier, check_scale, find_asset, find_character, find_label,
    find_scene, find_speaker, free_position, insertion_index, json_to_custom, json_to_value,
    parse_color, parse_id, parse_position,
};
use crate::error::{ErrorKind, ToolError};
use crate::sandbox::Sandbox;
use crate::views::{
    self, ChoicePick, SimulationLimits, assets_json, characters_json, render_scene, scenes_json,
    variables_json,
};

/// What a tool sees: the project (mutable, command API only), the folder it
/// is confined to and the installed plugins.
pub struct ToolCx<'a> {
    pub project: &'a mut Project,
    pub sandbox: &'a Sandbox,
    pub registry: &'a ProjectRegistry,
    pub manifest: &'a ProjectManifest,
    /// Set by `save`: write the story even if nothing changed.
    pub save_requested: bool,
}

type Run = Box<dyn Fn(&mut ToolCx<'_>, Json) -> Result<String, ToolError>>;

/// A registered tool: metadata for `tools/list` and the function to run.
pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Map<String, Json>,
    pub read_only: bool,
    /// Removes data (deleting scenes / nodes). Edits are undoable but not
    /// flagged: they only rewrite what the caller names.
    pub destructive: bool,
    pub idempotent: bool,
    /// Merge everything the call changes into one undo entry (false for
    /// undo / redo themselves).
    pub grouped: bool,
    pub run: Run,
}

#[derive(Clone, Copy)]
enum Kind {
    Read,
    Edit,
    Destructive,
    History,
}

/// JSON schema of an argument type, with sub-schemas inlined (no `$ref`) so
/// every client can read it.
pub fn input_schema_of<A: JsonSchema>() -> serde_json::Map<String, Json> {
    let settings = schemars::generate::SchemaSettings::draft2020_12().with(|s| {
        s.inline_subschemas = true;
    });
    let schema = settings.into_generator().into_root_schema_for::<A>();
    let mut map = match serde_json::to_value(&schema) {
        Ok(Json::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    map.remove("$schema");
    map.remove("title");
    map.entry("type").or_insert_with(|| Json::from("object"));
    map
}

fn def<A>(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    kind: Kind,
    f: fn(&mut ToolCx<'_>, A) -> Result<String, ToolError>,
) -> ToolDef
where
    A: DeserializeOwned + JsonSchema + 'static,
{
    ToolDef {
        name,
        title,
        description,
        input_schema: input_schema_of::<A>(),
        read_only: matches!(kind, Kind::Read),
        destructive: matches!(kind, Kind::Destructive),
        idempotent: matches!(kind, Kind::Read),
        grouped: !matches!(kind, Kind::History),
        run: Box::new(move |cx, args| {
            let parsed: A = serde_json::from_value(args)
                .map_err(|e| ToolError::invalid(format!("invalid arguments: {e}")))?;
            f(cx, parsed)
        }),
    }
}

fn exec(cx: &mut ToolCx<'_>, command: Command) -> Result<(), ToolError> {
    cx.project.execute(command).map_err(Into::into)
}

fn pretty(value: &Json) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn non_empty(text: &str, what: &str) -> Result<String, ToolError> {
    let t = text.trim();
    if t.is_empty() {
        Err(ToolError::invalid(format!("{what} must not be empty")))
    } else {
        Ok(t.to_owned())
    }
}

fn parse_expr(text: &str) -> Result<Expr, ToolError> {
    Expr::parse(text).map_err(|e| ToolError::invalid(format!("expression `{text}`: {e}")))
}

fn node_summary(project: &Project, id: NodeId) -> Json {
    let story = project.story();
    match story.locate(id) {
        Some((si, ni)) => views::node_json(project, &story.scenes[si].nodes[ni], ni),
        None => json!({"id": id.to_string()}),
    }
}

/// Removes strings that no node and no character uses any more.
fn release_strings(cx: &mut ToolCx<'_>, ids: Vec<StringId>) -> Result<(), ToolError> {
    let mut seen = Vec::new();
    for id in ids {
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let story = cx.project.story();
        let in_use = story
            .scenes
            .iter()
            .flat_map(|s| &s.nodes)
            .any(|n| n.kind.strings().contains(&id))
            || story.characters.iter().any(|c| c.name_id == id);
        if !in_use && story.strings.get(id).is_some() {
            exec(cx, Command::SetString { id, text: None })?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------- arguments

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// A stage position: a name or free normalized coordinates.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PositionArg {
    /// `left`, `center`, `right` or the name of a custom position.
    Named(String),
    /// Free position in normalized stage coordinates: `x` and `y` in
    /// `0.0..=1.0` (slightly outside is allowed to walk in from off screen),
    /// origin top-left; the sprite is anchored by its bottom-center point, so
    /// `y: 1.0` is standing on the bottom edge.
    Free { x: f32, y: f32 },
}

fn resolve_position_arg(
    story: &orbit_core::Story,
    position: Option<&PositionArg>,
) -> Result<orbit_core::Position, ToolError> {
    match position {
        None => parse_position(story, "center"),
        Some(PositionArg::Named(name)) => parse_position(story, name),
        Some(PositionArg::Free { x, y }) => free_position(*x, *y),
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SceneArg {
    /// Scene id (26-character ULID) or exact scene name.
    scene: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateSceneArgs {
    /// Name of the new scene (must be unique).
    name: String,
    /// Position in the scene list (default: last).
    index: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameSceneArgs {
    /// Scene id or current name.
    scene: String,
    /// New name (must be unique).
    new_name: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddDialogueArgs {
    /// Scene id or name.
    scene: String,
    /// Speaking character (id or name). Omit or use "narrator" for narration.
    speaker: Option<String>,
    /// The line of dialogue.
    text: String,
    /// If given, a `show` node displaying the speaker with this expression is
    /// inserted just before the line (requires a speaker).
    expression: Option<String>,
    /// Stage position for that `show` node: left, center (default), right,
    /// a custom position name, or a free `{"x": 0.4, "y": 1.0}`.
    position: Option<PositionArg>,
    /// Sprite scale override for that `show` node (default: the character's `default_scale`).
    scale: Option<f32>,
    /// Mirror the sprite horizontally in that `show` node.
    flip_x: Option<bool>,
    /// Mirror the sprite vertically in that `show` node.
    flip_y: Option<bool>,
    /// Insert after this node id (`start` inserts at the beginning). Default: at the end of the scene.
    after: Option<String>,
    /// Alternative to `after`: explicit index in the scene.
    index: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditDialogueArgs {
    /// Id of a dialogue (`say`) node.
    node: String,
    /// New text.
    text: Option<String>,
    /// New speaker (id or name); "narrator" removes the speaker.
    speaker: Option<String>,
    /// Version of the node you read (from read_scene); the call fails if the node changed since.
    expected_version: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChoiceOptionArg {
    /// Text shown to the player.
    text: String,
    /// Label the option jumps to: a label node id or a unique label name. Create labels first (add_command type=label).
    target: String,
    /// Optional condition in the expression language, e.g. `trust > 2 && !met_bob`.
    condition: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddChoiceArgs {
    /// Scene id or name.
    scene: String,
    /// The options (at least one), each jumping to a label.
    options: Vec<ChoiceOptionArg>,
    /// Insert after this node id (`start` inserts at the beginning). Default: at the end of the scene.
    after: Option<String>,
    /// Alternative to `after`: explicit index in the scene.
    index: Option<usize>,
}

/// A transition between two visual states.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionArg {
    /// `fade` (through a color) or `dissolve` (cross-fade).
    kind: TransitionKindArg,
    /// Duration in seconds (default 0.5).
    duration: Option<f32>,
    /// Fade color: `#rrggbb` or a name (default black). Ignored for dissolve.
    color: Option<String>,
    /// Easing: a preset name (`OutQuad`, `InOutCubic`, `OutBack`...) or the name of a story curve (see list_curves). Default: linear.
    ease: Option<String>,
    /// Easing as a CSS cubic-bezier `[x1, y1, x2, y2]` (x within 0..1). Alternative to `ease`.
    bezier: Option<[f32; 4]>,
}

#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKindArg {
    Fade,
    Dissolve,
}

impl TransitionArg {
    fn build(&self, story: &orbit_core::Story) -> Result<Transition, ToolError> {
        let kind = match self.kind {
            TransitionKindArg::Fade => "fade",
            TransitionKindArg::Dissolve => "dissolve",
        };
        let mut t = build_transition(kind, self.duration, self.color.as_deref())?;
        t.ease = parse_curve(self.ease.as_deref(), self.bezier, story)?;
        Ok(t)
    }
}

/// A story command (node) to add. `type` selects the kind.
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandSpec {
    /// Change the background image.
    Background {
        /// Image asset (id, manifest path or file name).
        asset: String,
        transition: Option<TransitionArg>,
    },
    /// Show a character on stage with an expression.
    Show {
        /// Character id or name.
        character: String,
        /// Expression name (must exist on the character).
        expression: String,
        /// left, center (default), right, a custom position name, or a free
        /// position `{"x": 0.4, "y": 1.0}` (normalized, origin top-left, the
        /// sprite is anchored by its bottom-center).
        position: Option<PositionArg>,
        transition: Option<TransitionArg>,
        /// Sprite scale override (1.0 = native size; 0.05 to 10). Default:
        /// the character's `default_scale`. Showing an already visible
        /// character again updates its position, scale and mirroring.
        scale: Option<f32>,
        /// Vertical scale for a non-uniform stretch (default: same as `scale`).
        scale_y: Option<f32>,
        /// Mirror the sprite on the X axis (default false).
        flip_x: Option<bool>,
        /// Mirror the sprite on the Y axis (default false).
        flip_y: Option<bool>,
    },
    /// Remove a character from the stage.
    Hide {
        /// Character id or name.
        character: String,
        transition: Option<TransitionArg>,
    },
    /// Start a music track (replaces the current one).
    PlayMusic {
        /// Music asset (id, manifest path or file name).
        asset: String,
        /// Fade-in in seconds (default 0).
        fade_in: Option<f32>,
        /// Loop the track (default true).
        looping: Option<bool>,
    },
    /// Stop the music.
    StopMusic {
        /// Fade-out in seconds (default 0).
        fade_out: Option<f32>,
    },
    /// Play a sound effect.
    PlaySound {
        /// Sound asset (id, manifest path or file name).
        asset: String,
    },
    /// Pause for a number of seconds.
    Wait { seconds: f32 },
    /// Assign a variable: `var = value`.
    SetVar {
        /// Variable name (declare it with declare_variable).
        var: String,
        /// Expression in the expression language, e.g. `trust + 1`, `true`, `"forest"`.
        value: String,
    },
    /// Conditional jump.
    If {
        /// Condition, e.g. `trust > 2 && route == "forest"`.
        condition: String,
        /// Label id or name to jump to when true.
        then_target: String,
        /// Label id or name when false (default: continue with the next node).
        else_target: Option<String>,
    },
    /// Unconditional jump to a label.
    Jump {
        /// Label id or name.
        target: String,
    },
    /// Jump to another scene: scenes never continue into the next one, this
    /// is the only way to go from one scene to another.
    JumpScene {
        /// Target scene id or name.
        scene: String,
        /// Label of that scene to land on (id or name; default: the start of
        /// the scene).
        label: Option<String>,
    },
    /// A jump destination.
    Label {
        /// Label name (unique in the project).
        name: String,
    },
    /// Call the routine starting at a label (`return` comes back).
    Call {
        /// Label id or name.
        label: String,
    },
    /// Return from the innermost `call`.
    Return,
    /// End the story.
    End,
    /// A node defined by a plugin.
    Custom {
        plugin_id: String,
        kind: String,
        /// Plugin data version (default 0).
        version: Option<u32>,
        /// Free-form JSON data for the plugin.
        data: Option<Json>,
    },
}

/// Name of the IR `NodeKind` variant a command spec produces.
pub fn spec_variant(spec: &CommandSpec) -> &'static str {
    match spec {
        CommandSpec::Background { .. } => "Background",
        CommandSpec::Show { .. } => "Show",
        CommandSpec::Hide { .. } => "Hide",
        CommandSpec::PlayMusic { .. } => "PlayMusic",
        CommandSpec::StopMusic { .. } => "StopMusic",
        CommandSpec::PlaySound { .. } => "PlaySound",
        CommandSpec::Wait { .. } => "Wait",
        CommandSpec::SetVar { .. } => "SetVar",
        CommandSpec::If { .. } => "If",
        CommandSpec::Jump { .. } => "Jump",
        CommandSpec::JumpScene { .. } => "JumpScene",
        CommandSpec::Label { .. } => "Label",
        CommandSpec::Call { .. } => "Call",
        CommandSpec::Return => "Return",
        CommandSpec::End => "End",
        CommandSpec::Custom { .. } => "Custom",
    }
}

/// Which tool (and `type`) creates each IR node kind. The coherence test
/// checks this table against the generated IR schema, so a new `NodeKind`
/// variant cannot be added without an MCP tool for it.
pub const NODE_KIND_TOOLS: &[(&str, &str)] = &[
    ("Label", "add_command type=label"),
    ("Say", "add_dialogue"),
    ("Choice", "add_choice"),
    ("Jump", "add_command type=jump"),
    ("JumpScene", "add_command type=jump_scene"),
    ("SetVar", "add_command type=set_var"),
    ("If", "add_command type=if"),
    ("Background", "add_command type=background"),
    ("Show", "add_command type=show"),
    ("Hide", "add_command type=hide"),
    ("PlayMusic", "add_command type=play_music"),
    ("StopMusic", "add_command type=stop_music"),
    ("PlaySound", "add_command type=play_sound"),
    ("Wait", "add_command type=wait"),
    ("Call", "add_command type=call"),
    ("Return", "add_command type=return"),
    ("End", "add_command type=end"),
    ("Custom", "add_command type=custom"),
];

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddCommandArgs {
    /// Scene id or name.
    scene: String,
    /// The command to add.
    command: CommandSpec,
    /// Insert after this node id (`start` inserts at the beginning). Default: at the end of the scene.
    after: Option<String>,
    /// Alternative to `after`: explicit index in the scene.
    index: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteNodeArgs {
    /// Node id. Refused while another node still jumps to it.
    node: String,
    /// Version of the node you read; the call fails if it changed since.
    expected_version: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveNodeArgs {
    /// Node id.
    node: String,
    /// Target scene id or name (default: the node's current scene).
    scene: Option<String>,
    /// Place after this node id (`start` for the beginning). Default: at the end.
    after: Option<String>,
    /// Alternative to `after`: index in the target scene once the node is taken out.
    index: Option<usize>,
    /// Version of the node you read; the call fails if it changed since.
    expected_version: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExpressionArg {
    /// Expression name, e.g. `happy`.
    name: String,
    /// Image asset (id, manifest path or file name).
    asset: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateCharacterArgs {
    /// Display name (must be unique).
    name: String,
    /// Name color: `#rrggbb` or a name. Default: a color picked for you.
    color: Option<String>,
    /// Sprites of the character, one per expression.
    expressions: Option<Vec<ExpressionArg>>,
    /// Default sprite scale (1.0 = native size; 0.05 to 10), used by `show`
    /// nodes that do not set their own `scale`.
    default_scale: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditCharacterArgs {
    /// Character id or name.
    character: String,
    /// New display name.
    name: Option<String>,
    /// New name color.
    color: Option<String>,
    /// Expressions to add or replace (matched by name).
    set_expressions: Option<Vec<ExpressionArg>>,
    /// Expression names to remove (refused while a `show` node uses them).
    remove_expressions: Option<Vec<String>>,
    /// New default sprite scale (1.0 = native size; 0.05 to 10).
    default_scale: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ScopeArg {
    Game,
    Persistent,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeclareVariableArgs {
    /// Variable name (letters, digits, `_`).
    name: String,
    /// Initial value: a boolean, a number or a string. It fixes the variable's type.
    initial: Json,
    /// `game` (saved with the playthrough, default) or `persistent` (survives across playthroughs).
    scope: Option<ScopeArg>,
}

/// A named easing curve of the story.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetCurveArgs {
    /// Curve name (reused by name in transitions).
    name: String,
    /// A preset: `Linear`, `InQuad`, `OutQuad`, `InOutQuad`, `InCubic`..., `InBack`, `OutBack`, `OutBounce`, `OutElastic`, `Step`...
    ease: Option<String>,
    /// A CSS cubic-bezier `[x1, y1, x2, y2]` (x within 0..1, y may overshoot). Alternative to `ease`.
    bezier: Option<[f32; 4]>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteCurveArgs {
    /// Name of an unused curve.
    name: String,
}

#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum AssetKindArg {
    Image,
    Music,
    Sound,
}

impl From<AssetKindArg> for AssetKind {
    fn from(kind: AssetKindArg) -> Self {
        match kind {
            AssetKindArg::Image => AssetKind::Image,
            AssetKindArg::Music => AssetKind::Music,
            AssetKindArg::Sound => AssetKind::Sound,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListAssetsArgs {
    /// Only this kind.
    kind: Option<AssetKindArg>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportAssetArgs {
    /// Path of a file **inside the project folder** (relative to it). It is copied under `assets/` and registered in the manifest.
    source: String,
    /// Asset kind. Needed for audio files (music or sound); inferred for images.
    kind: Option<AssetKindArg>,
    /// File name to use under `assets/` (default: the source file name).
    name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ValidateArgs {
    /// Also check that every asset file exists on disk (default true).
    check_files: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum ChoiceSel {
    /// Option index (0-based).
    Index(usize),
    /// Option text (exact, or the beginning of it).
    Text(String),
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SimulateArgs {
    /// Choices to make, in the order the story asks for them: each is an option index (0-based) or its text.
    /// When the story reaches a choice and the list is exhausted, the simulation stops and shows the options.
    choices: Option<Vec<ChoiceSel>>,
    /// Start from this node id instead of the beginning of the story.
    start_at: Option<String>,
    /// Safety limit on VM steps (default 2000, max 20000).
    max_steps: Option<usize>,
}

// -------------------------------------------------------------- handlers

fn list_scenes(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    Ok(pretty(&scenes_json(cx.project.story())))
}

fn read_scene(cx: &mut ToolCx<'_>, a: SceneArg) -> Result<String, ToolError> {
    let scene = find_scene(cx.project.story(), &a.scene)?;
    Ok(render_scene(cx.project, scene))
}

fn unique_scene_name(
    cx: &ToolCx<'_>,
    name: &str,
    except: Option<SceneId>,
) -> Result<(), ToolError> {
    if cx
        .project
        .story()
        .scenes
        .iter()
        .any(|s| Some(s.id) != except && s.name.eq_ignore_ascii_case(name))
    {
        return Err(ToolError::refused(format!(
            "a scene named `{name}` already exists"
        )));
    }
    Ok(())
}

fn create_scene(cx: &mut ToolCx<'_>, a: CreateSceneArgs) -> Result<String, ToolError> {
    let name = non_empty(&a.name, "scene name")?;
    unique_scene_name(cx, &name, None)?;
    let len = cx.project.story().scenes.len();
    let index = a.index.unwrap_or(len);
    if index > len {
        return Err(ToolError::invalid(format!(
            "index {index} is out of range (0..={len})"
        )));
    }
    let id = SceneId::new();
    exec(
        cx,
        Command::InsertScene {
            index,
            scene: Scene {
                id,
                name: name.clone(),
                nodes: Vec::new(),
            },
        },
    )?;
    Ok(pretty(
        &json!({"scene": id.to_string(), "name": name, "index": index}),
    ))
}

fn rename_scene(cx: &mut ToolCx<'_>, a: RenameSceneArgs) -> Result<String, ToolError> {
    let id = find_scene(cx.project.story(), &a.scene)?.id;
    let name = non_empty(&a.new_name, "scene name")?;
    unique_scene_name(cx, &name, Some(id))?;
    exec(
        cx,
        Command::RenameScene {
            id,
            name: name.clone(),
        },
    )?;
    Ok(pretty(&json!({"scene": id.to_string(), "name": name})))
}

fn delete_scene(cx: &mut ToolCx<'_>, a: SceneArg) -> Result<String, ToolError> {
    let scene = find_scene(cx.project.story(), &a.scene)?;
    let (id, name, count) = (scene.id, scene.name.clone(), scene.nodes.len());
    let strings: Vec<StringId> = scene.nodes.iter().flat_map(|n| n.kind.strings()).collect();
    exec(cx, Command::RemoveScene { id })?;
    release_strings(cx, strings)?;
    Ok(pretty(&json!({
        "deleted_scene": id.to_string(),
        "name": name,
        "nodes_removed": count,
        "note": "Undo with the `undo` tool if this was a mistake."
    })))
}

fn add_dialogue(cx: &mut ToolCx<'_>, a: AddDialogueArgs) -> Result<String, ToolError> {
    let text = non_empty(&a.text, "dialogue text")?;
    let story = cx.project.story();
    let scene = find_scene(story, &a.scene)?;
    let scene_id = scene.id;
    let speaker = find_speaker(story, a.speaker.as_deref())?;
    let nodes: Vec<&Node> = scene.nodes.iter().collect();
    let index = insertion_index(&nodes, a.after.as_deref(), a.index)?;

    let show = match (&a.expression, speaker) {
        (Some(expression), Some(character)) => {
            let ch = story
                .character(character)
                .ok_or_else(|| ToolError::not_found("unknown speaker"))?;
            if ch.expression(expression).is_none() {
                return Err(ToolError::not_found(format!(
                    "character has no expression `{expression}` (has: {})",
                    ch.expressions
                        .iter()
                        .map(|e| e.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            let position = resolve_position_arg(story, a.position.as_ref())?;
            Some(Node {
                id: NodeId::new(),
                kind: NodeKind::Show {
                    character,
                    expression: expression.clone(),
                    position,
                    transition: None,
                    scale: a.scale.map(|s| check_scale("scale", s)).transpose()?,
                    scale_y: None,
                    flip_x: a.flip_x.unwrap_or(false),
                    flip_y: a.flip_y.unwrap_or(false),
                },
            })
        }
        (Some(_), None) => {
            return Err(ToolError::invalid(
                "`expression` needs a speaker (a character) to show",
            ));
        }
        (None, _)
            if a.position.is_some()
                || a.scale.is_some()
                || a.flip_x.is_some()
                || a.flip_y.is_some() =>
        {
            return Err(ToolError::invalid(
                "`position`, `scale`, `flip_x` and `flip_y` only apply with `expression`",
            ));
        }
        (None, _) => None,
    };

    let text_id = StringId::new();
    let say = Node {
        id: NodeId::new(),
        kind: NodeKind::Say { speaker, text_id },
    };
    let say_id = say.id;
    let show_id = show.as_ref().map(|n| n.id);
    exec(
        cx,
        Command::SetString {
            id: text_id,
            text: Some(text),
        },
    )?;
    let mut at = index;
    if let Some(show) = show {
        exec(
            cx,
            Command::InsertNode {
                scene: scene_id,
                index: at,
                node: show,
            },
        )?;
        at += 1;
    }
    exec(
        cx,
        Command::InsertNode {
            scene: scene_id,
            index: at,
            node: say,
        },
    )?;
    Ok(pretty(&json!({
        "dialogue": node_summary(cx.project, say_id),
        "show": show_id.map(|id| node_summary(cx.project, id)),
    })))
}

fn edit_dialogue(cx: &mut ToolCx<'_>, a: EditDialogueArgs) -> Result<String, ToolError> {
    if a.text.is_none() && a.speaker.is_none() {
        return Err(ToolError::invalid("give `text` and/or `speaker`"));
    }
    let id: NodeId = parse_id(&a.node, "node")?;
    let story = cx.project.story();
    let node = story
        .node(id)
        .ok_or_else(|| ToolError::not_found(format!("no node {id}")))?;
    let NodeKind::Say { speaker, text_id } = &node.kind else {
        return Err(ToolError::invalid(format!(
            "node {id} is not a dialogue (say) node"
        )));
    };
    let speaker = match &a.speaker {
        Some(key) => find_speaker(story, Some(key))?,
        None => *speaker,
    };
    let text = match &a.text {
        Some(t) => non_empty(t, "dialogue text")?,
        None => story.text(*text_id).unwrap_or_default().to_owned(),
    };
    exec(
        cx,
        Command::SetSay {
            id,
            speaker,
            text,
            expected_version: a.expected_version,
        },
    )?;
    Ok(pretty(&node_summary(cx.project, id)))
}

fn add_choice(cx: &mut ToolCx<'_>, a: AddChoiceArgs) -> Result<String, ToolError> {
    if a.options.is_empty() {
        return Err(ToolError::invalid("a choice needs at least one option"));
    }
    let story = cx.project.story();
    let scene = find_scene(story, &a.scene)?;
    let scene_id = scene.id;
    let nodes: Vec<&Node> = scene.nodes.iter().collect();
    let index = insertion_index(&nodes, a.after.as_deref(), a.index)?;
    let mut options = Vec::new();
    let mut strings = Vec::new();
    for (i, option) in a.options.iter().enumerate() {
        let text = non_empty(&option.text, &format!("option {i} text"))?;
        let target = find_label(story, option.target.trim())?;
        let condition = option.condition.as_deref().map(parse_expr).transpose()?;
        let text_id = StringId::new();
        strings.push((text_id, text));
        options.push(ChoiceOption {
            text_id,
            target,
            condition,
        });
    }
    let node = Node {
        id: NodeId::new(),
        kind: NodeKind::Choice { options },
    };
    let id = node.id;
    for (text_id, text) in strings {
        exec(
            cx,
            Command::SetString {
                id: text_id,
                text: Some(text),
            },
        )?;
    }
    exec(
        cx,
        Command::InsertNode {
            scene: scene_id,
            index,
            node,
        },
    )?;
    Ok(pretty(&node_summary(cx.project, id)))
}

fn build_kind(cx: &ToolCx<'_>, spec: &CommandSpec) -> Result<NodeKind, ToolError> {
    let story = cx.project.story();
    let seconds = |label: &str, value: f32| {
        if value.is_finite() && value >= 0.0 {
            Ok(value)
        } else {
            Err(ToolError::invalid(format!(
                "{label} {value} must be a finite number >= 0"
            )))
        }
    };
    let transition = |t: &Option<TransitionArg>| t.as_ref().map(|t| t.build(story)).transpose();
    Ok(match spec {
        CommandSpec::Background {
            asset,
            transition: t,
        } => NodeKind::Background {
            asset: find_asset(story, asset, Some(AssetKind::Image))?,
            transition: transition(t)?,
        },
        CommandSpec::Show {
            character,
            expression,
            position,
            transition: t,
            scale,
            scale_y,
            flip_x,
            flip_y,
        } => {
            let id = find_character(story, character)?;
            let ch = story
                .character(id)
                .ok_or_else(|| ToolError::not_found("unknown character"))?;
            if ch.expression(expression).is_none() {
                return Err(ToolError::not_found(format!(
                    "character has no expression `{expression}` (has: {}); add one with edit_character",
                    ch.expressions
                        .iter()
                        .map(|e| e.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            NodeKind::Show {
                character: id,
                expression: expression.clone(),
                position: resolve_position_arg(story, position.as_ref())?,
                transition: transition(t)?,
                scale: scale.map(|s| check_scale("scale", s)).transpose()?,
                scale_y: scale_y.map(|s| check_scale("scale_y", s)).transpose()?,
                flip_x: flip_x.unwrap_or(false),
                flip_y: flip_y.unwrap_or(false),
            }
        }
        CommandSpec::Hide {
            character,
            transition: t,
        } => NodeKind::Hide {
            character: find_character(story, character)?,
            transition: transition(t)?,
        },
        CommandSpec::PlayMusic {
            asset,
            fade_in,
            looping,
        } => NodeKind::PlayMusic {
            asset: find_asset(story, asset, Some(AssetKind::Music))?,
            fade_in: seconds("fade_in", fade_in.unwrap_or(0.0))?,
            looping: looping.unwrap_or(true),
        },
        CommandSpec::StopMusic { fade_out } => NodeKind::StopMusic {
            fade_out: seconds("fade_out", fade_out.unwrap_or(0.0))?,
        },
        CommandSpec::PlaySound { asset } => NodeKind::PlaySound {
            asset: find_asset(story, asset, Some(AssetKind::Sound))?,
        },
        CommandSpec::Wait { seconds: s } => NodeKind::Wait {
            seconds: seconds("seconds", *s)?,
        },
        CommandSpec::SetVar { var, value } => {
            check_identifier(var)?;
            NodeKind::SetVar {
                var: var.clone(),
                value: parse_expr(value)?,
            }
        }
        CommandSpec::If {
            condition,
            then_target,
            else_target,
        } => NodeKind::If {
            condition: parse_expr(condition)?,
            then_target: find_label(story, then_target.trim())?,
            else_target: else_target
                .as_deref()
                .map(|t| find_label(story, t.trim()))
                .transpose()?,
        },
        CommandSpec::Jump { target } => NodeKind::Jump {
            target: find_label(story, target.trim())?,
        },
        CommandSpec::JumpScene { scene, label } => {
            let target = find_scene(story, scene.trim())?;
            let label = label
                .as_deref()
                .map(|l| {
                    let id = find_label(story, l.trim())?;
                    if target.nodes.iter().any(|n| n.id == id) {
                        Ok(id)
                    } else {
                        Err(ToolError::invalid(format!(
                            "label `{l}` is not in scene `{}`",
                            target.name
                        )))
                    }
                })
                .transpose()?;
            NodeKind::JumpScene {
                scene: target.id,
                label,
            }
        }
        CommandSpec::Label { name } => {
            let name = non_empty(name, "label name")?;
            let taken = story
                .scenes
                .iter()
                .flat_map(|s| &s.nodes)
                .any(|n| matches!(&n.kind, NodeKind::Label { name: other } if *other == name));
            if taken {
                return Err(ToolError::refused(format!(
                    "a label named `{name}` already exists (label names are unique)"
                )));
            }
            NodeKind::Label { name }
        }
        CommandSpec::Call { label } => NodeKind::Call {
            label: find_label(story, label.trim())?,
        },
        CommandSpec::Return => NodeKind::Return,
        CommandSpec::End => NodeKind::End,
        CommandSpec::Custom {
            plugin_id,
            kind,
            version,
            data,
        } => {
            let data = data.as_ref().map(json_to_custom).unwrap_or_default();
            if let Some(registered) = cx.registry.node_type(plugin_id, kind)
                && let Some(validate) = registered.def.validate
            {
                let problems = validate(&data);
                if !problems.is_empty() {
                    return Err(ToolError::invalid(format!(
                        "invalid data for {plugin_id}/{kind}: {}",
                        problems.join("; ")
                    )));
                }
            }
            NodeKind::Custom {
                plugin_id: plugin_id.clone(),
                kind: kind.clone(),
                version: version.unwrap_or(0),
                data,
            }
        }
    })
}

fn add_command(cx: &mut ToolCx<'_>, a: AddCommandArgs) -> Result<String, ToolError> {
    let story = cx.project.story();
    let scene = find_scene(story, &a.scene)?;
    let scene_id = scene.id;
    let nodes: Vec<&Node> = scene.nodes.iter().collect();
    let index = insertion_index(&nodes, a.after.as_deref(), a.index)?;
    let kind = build_kind(cx, &a.command)?;
    let mut note = None;
    if let NodeKind::SetVar { var, .. } = &kind
        && !cx.project.story().variables.iter().any(|v| &v.name == var)
    {
        note = Some(format!(
            "variable `{var}` is not declared yet: use declare_variable (validate warns about it)"
        ));
    }
    let node = Node {
        id: NodeId::new(),
        kind,
    };
    let id = node.id;
    exec(
        cx,
        Command::InsertNode {
            scene: scene_id,
            index,
            node,
        },
    )?;
    Ok(pretty(
        &json!({"node": node_summary(cx.project, id), "note": note}),
    ))
}

fn delete_node(cx: &mut ToolCx<'_>, a: DeleteNodeArgs) -> Result<String, ToolError> {
    let id: NodeId = parse_id(&a.node, "node")?;
    let node = cx
        .project
        .story()
        .node(id)
        .ok_or_else(|| ToolError::not_found(format!("no node {id}")))?;
    let strings = node.kind.strings();
    let summary = views::describe_kind(cx.project.story(), &node.kind);
    exec(
        cx,
        Command::RemoveNode {
            id,
            expected_version: a.expected_version,
        },
    )?;
    release_strings(cx, strings)?;
    Ok(pretty(&json!({
        "deleted_node": id.to_string(),
        "content": summary,
        "note": "Undo with the `undo` tool if this was a mistake."
    })))
}

fn move_node(cx: &mut ToolCx<'_>, a: MoveNodeArgs) -> Result<String, ToolError> {
    let id: NodeId = parse_id(&a.node, "node")?;
    let story = cx.project.story();
    let (si, _) = story
        .locate(id)
        .ok_or_else(|| ToolError::not_found(format!("no node {id}")))?;
    let target = match &a.scene {
        Some(key) => find_scene(story, key)?,
        None => &story.scenes[si],
    };
    if a.after.as_deref().is_some_and(|x| x == id.to_string()) {
        return Err(ToolError::invalid("a node cannot be moved after itself"));
    }
    let others: Vec<&Node> = target.nodes.iter().filter(|n| n.id != id).collect();
    let index = insertion_index(&others, a.after.as_deref(), a.index)?;
    let scene = target.id;
    exec(
        cx,
        Command::MoveNode {
            id,
            scene,
            index,
            expected_version: a.expected_version,
        },
    )?;
    Ok(pretty(&node_summary(cx.project, id)))
}

const NAME_COLORS: [(u8, u8, u8); 8] = [
    (230, 120, 120),
    (110, 170, 230),
    (240, 190, 90),
    (130, 200, 130),
    (190, 140, 220),
    (110, 200, 200),
    (230, 150, 190),
    (200, 200, 120),
];

fn build_expressions(
    cx: &ToolCx<'_>,
    items: &[ExpressionArg],
) -> Result<Vec<Expression>, ToolError> {
    let mut out: Vec<Expression> = Vec::new();
    for item in items {
        let name = non_empty(&item.name, "expression name")?;
        if out.iter().any(|e| e.name == name) {
            return Err(ToolError::invalid(format!(
                "expression `{name}` is given twice"
            )));
        }
        out.push(Expression {
            name,
            image: find_asset(cx.project.story(), &item.asset, Some(AssetKind::Image))?,
            motion: None,
        });
    }
    Ok(out)
}

fn create_character(cx: &mut ToolCx<'_>, a: CreateCharacterArgs) -> Result<String, ToolError> {
    let name = non_empty(&a.name, "character name")?;
    let story = cx.project.story();
    if story.characters.iter().any(|c| {
        story
            .text(c.name_id)
            .is_some_and(|n| n.eq_ignore_ascii_case(&name))
    }) {
        return Err(ToolError::refused(format!(
            "a character named `{name}` already exists"
        )));
    }
    let color = match &a.color {
        Some(c) => parse_color(c)?,
        None => {
            let (r, g, b) = NAME_COLORS[story.characters.len() % NAME_COLORS.len()];
            orbit_core::Color::rgb(r, g, b)
        }
    };
    let expressions = build_expressions(cx, a.expressions.as_deref().unwrap_or_default())?;
    let default_scale = a
        .default_scale
        .map(|s| check_scale("default_scale", s))
        .transpose()?
        .unwrap_or(1.0);
    let name_id = StringId::new();
    let id = CharacterId::new();
    exec(
        cx,
        Command::SetString {
            id: name_id,
            text: Some(name),
        },
    )?;
    exec(
        cx,
        Command::SetCharacter {
            id,
            character: Some(Character {
                id,
                name_id,
                color,
                expressions,
                default_position: None,
                default_scale,
            }),
            index: None,
        },
    )?;
    character_json(cx, id)
}

fn character_json(cx: &ToolCx<'_>, id: CharacterId) -> Result<String, ToolError> {
    let all = characters_json(cx.project.story());
    let found = all
        .as_array()
        .and_then(|items| items.iter().find(|c| c["id"] == id.to_string()))
        .cloned()
        .unwrap_or(Json::Null);
    Ok(pretty(&found))
}

fn edit_character(cx: &mut ToolCx<'_>, a: EditCharacterArgs) -> Result<String, ToolError> {
    let story = cx.project.story();
    let id = find_character(story, &a.character)?;
    let mut character = story
        .character(id)
        .cloned()
        .ok_or_else(|| ToolError::not_found("unknown character"))?;
    if a.name.is_none()
        && a.color.is_none()
        && a.set_expressions.is_none()
        && a.remove_expressions.is_none()
        && a.default_scale.is_none()
    {
        return Err(ToolError::invalid(
            "nothing to change: give name, color, default_scale, set_expressions and/or remove_expressions",
        ));
    }
    if let Some(scale) = a.default_scale {
        character.default_scale = check_scale("default_scale", scale)?;
    }
    let new_name = a
        .name
        .as_deref()
        .map(|n| non_empty(n, "character name"))
        .transpose()?;
    if let Some(name) = &new_name
        && story.characters.iter().any(|c| {
            c.id != id
                && story
                    .text(c.name_id)
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    {
        return Err(ToolError::refused(format!(
            "a character named `{name}` already exists"
        )));
    }
    if let Some(color) = &a.color {
        character.color = parse_color(color)?;
    }
    for removed in a.remove_expressions.as_deref().unwrap_or_default() {
        if !character.expressions.iter().any(|e| &e.name == removed) {
            return Err(ToolError::not_found(format!(
                "character has no expression `{removed}`"
            )));
        }
        let in_use = story.scenes.iter().flat_map(|s| &s.nodes).find(|n| {
            matches!(&n.kind, NodeKind::Show { character: c, expression, .. } if *c == id && expression == removed)
        });
        if let Some(user) = in_use {
            return Err(ToolError::refused(format!(
                "expression `{removed}` is still shown by node {}",
                user.id
            )));
        }
        character.expressions.retain(|e| &e.name != removed);
    }
    for new in build_expressions(cx, a.set_expressions.as_deref().unwrap_or_default())? {
        match character
            .expressions
            .iter_mut()
            .find(|e| e.name == new.name)
        {
            Some(existing) => existing.image = new.image,
            None => character.expressions.push(new),
        }
    }
    let name_id = character.name_id;
    if let Some(name) = new_name {
        exec(
            cx,
            Command::SetString {
                id: name_id,
                text: Some(name),
            },
        )?;
    }
    exec(
        cx,
        Command::SetCharacter {
            id,
            character: Some(character),
            index: None,
        },
    )?;
    character_json(cx, id)
}

fn declare_variable(cx: &mut ToolCx<'_>, a: DeclareVariableArgs) -> Result<String, ToolError> {
    check_identifier(&a.name)?;
    let initial = json_to_value(&a.initial)?;
    let scope = match a.scope {
        Some(ScopeArg::Persistent) => VarScope::Persistent,
        _ => VarScope::Game,
    };
    exec(
        cx,
        Command::SetVariable {
            name: a.name.clone(),
            decl: Some(VarDecl {
                name: a.name.clone(),
                initial,
                scope,
            }),
            index: None,
        },
    )?;
    let all = variables_json(cx.project.story());
    let found = all
        .as_array()
        .and_then(|items| items.iter().find(|v| v["name"] == a.name.as_str()))
        .cloned()
        .unwrap_or(Json::Null);
    Ok(pretty(&found))
}

fn list_assets(cx: &mut ToolCx<'_>, a: ListAssetsArgs) -> Result<String, ToolError> {
    Ok(pretty(&assets_json(
        cx.project.story(),
        a.kind.map(Into::into),
    )))
}

/// Per-language coverage and stale translations (`i18n/<story>.<lang>.ron`).
fn translation_status(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    const MAX_STALE: usize = 50;
    let story_path = cx.sandbox.root().join(&cx.manifest.story);
    let story = cx.project.story();
    let mut languages = Vec::new();
    for lang in orbit_i18n::files::list_languages(&story_path) {
        let path = orbit_i18n::files::translation_path(&story_path, &lang);
        let t = match orbit_i18n::files::load(&path) {
            Ok(Some(t)) => t,
            Ok(None) => continue,
            Err(e) => {
                languages.push(json!({"language": lang, "error": e}));
                continue;
            }
        };
        let report = orbit_i18n::coverage(story, &t);
        let stale = orbit_i18n::stale_entries(&story.strings, &t);
        let listed: Vec<Json> = stale
            .iter()
            .take(MAX_STALE)
            .map(|s| {
                json!({"id": s.id.to_string(), "source": s.source, "translation": s.translation})
            })
            .collect();
        languages.push(json!({
            "language": t.lang,
            "translated": report.translated,
            "total": report.total,
            "missing": report.missing.len(),
            "orphans": report.orphans.len(),
            "stale_count": stale.len(),
            "stale": listed,
        }));
    }
    Ok(pretty(&json!({
        "source_language": story.strings.source_lang,
        "languages": languages,
    })))
}

const MAX_IMPORT_BYTES: u64 = 256 * 1024 * 1024;

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    cleaned.trim_matches('.').to_owned()
}

fn import_asset(cx: &mut ToolCx<'_>, a: ImportAssetArgs) -> Result<String, ToolError> {
    let source = cx.sandbox.resolve(&a.source)?;
    let meta = fs::metadata(&source)
        .map_err(|_| ToolError::not_found(format!("no file `{}` in the project", a.source)))?;
    if !meta.is_file() {
        return Err(ToolError::invalid(format!("`{}` is not a file", a.source)));
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(ToolError::refused(format!(
            "file is too large ({} bytes, limit {MAX_IMPORT_BYTES})",
            meta.len()
        )));
    }
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let importer = cx.registry.importer_for(&ext);
    let mut kind: Option<AssetKind> = a.kind.map(Into::into);
    if kind.is_none()
        && matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
        )
    {
        kind = Some(AssetKind::Image);
    }
    let converted = match importer {
        Some(importer) => {
            let bytes = fs::read(&source)?;
            let imported = (importer.import)(&bytes)
                .map_err(|e| ToolError::invalid(format!("import failed: {e}")))?;
            kind = Some(imported.kind);
            Some(imported.bytes)
        }
        None => None,
    };
    let kind = kind.ok_or_else(|| {
        ToolError::invalid(
            "cannot tell the asset kind from the extension: pass `kind` (image, music or sound)",
        )
    })?;

    let source_rel = cx
        .sandbox
        .relative(&source)
        .ok_or_else(|| ToolError::new(ErrorKind::OutsideProject, "file is outside the project"))?;
    let (rel, copied) =
        if converted.is_none() && source_rel.starts_with("assets/") && a.name.is_none() {
            (source_rel, false)
        } else {
            let file_name = match &a.name {
                Some(n) => sanitize_file_name(n),
                None => sanitize_file_name(
                    source
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("asset"),
                ),
            };
            if file_name.is_empty() {
                return Err(ToolError::invalid("invalid asset file name"));
            }
            let dir = match kind {
                AssetKind::Image => "assets/images",
                AssetKind::Music => "assets/music",
                AssetKind::Sound => "assets/sounds",
            };
            let payload = match &converted {
                Some(bytes) => bytes.clone(),
                None => fs::read(&source)?,
            };
            let (stem, dot_ext) = match file_name.rsplit_once('.') {
                Some((s, e)) => (s.to_owned(), format!(".{e}")),
                None => (file_name.clone(), String::new()),
            };
            let mut candidate = format!("{dir}/{file_name}");
            let mut n = 1;
            loop {
                let dest = cx.sandbox.resolve(&candidate)?;
                if !dest.exists() {
                    break;
                }
                // Same content already there: reuse it instead of duplicating.
                if fs::read(&dest).is_ok_and(|existing| existing == payload) {
                    break;
                }
                n += 1;
                candidate = format!("{dir}/{stem}_{n}{dot_ext}");
            }
            let dest = cx.sandbox.resolve(&candidate)?;
            let created = !dest.exists();
            if created {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&dest, &payload)?;
            }
            (candidate, created)
        };

    if let Some(existing) = cx
        .project
        .story()
        .assets
        .assets
        .iter()
        .find(|e| e.path == rel)
    {
        return Ok(pretty(&json!({
            "id": existing.id.to_string(),
            "kind": existing.kind.to_string(),
            "path": existing.path,
            "already_registered": true,
        })));
    }
    let id = AssetId::new();
    let result = exec(
        cx,
        Command::SetAsset {
            id,
            entry: Some(AssetEntry {
                id,
                kind,
                path: rel.clone(),
            }),
            index: None,
        },
    );
    if let Err(err) = result {
        if copied && let Ok(dest) = cx.sandbox.resolve(&rel) {
            let _ = fs::remove_file(dest);
        }
        return Err(err);
    }
    Ok(pretty(&json!({
        "id": id.to_string(),
        "kind": kind.to_string(),
        "path": rel,
        "copied": copied,
    })))
}

fn validate(cx: &mut ToolCx<'_>, a: ValidateArgs) -> Result<String, ToolError> {
    // Same root as the editor: a story opened alone may keep its assets in a
    // parent folder. Paths are only tested for existence, never read.
    let root = orbit_project::find_assets_root(cx.sandbox.root(), cx.project.story());
    let exists = move |path: &str| {
        let rel = std::path::Path::new(path);
        let plain = rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        plain && root.join(rel).is_file()
    };
    let check = a.check_files.unwrap_or(true);
    let report = views::validate_project(
        cx.project.story(),
        cx.registry,
        check.then_some(&exists as &dyn Fn(&str) -> bool),
    );
    Ok(pretty(&report.to_json()))
}

fn simulate(cx: &mut ToolCx<'_>, a: SimulateArgs) -> Result<String, ToolError> {
    let picks: Vec<ChoicePick> = a
        .choices
        .unwrap_or_default()
        .into_iter()
        .map(|c| match c {
            ChoiceSel::Index(i) => ChoicePick::Index(i),
            ChoiceSel::Text(t) => ChoicePick::Text(t),
        })
        .collect();
    let start = a
        .start_at
        .as_deref()
        .map(|s| parse_id::<NodeId>(s, "node"))
        .transpose()?;
    if let Some(node) = start
        && cx.project.story().node(node).is_none()
    {
        return Err(ToolError::not_found(format!("no node {node}")));
    }
    let limits = SimulationLimits {
        max_steps: a.max_steps.unwrap_or(2000).clamp(1, 20_000),
    };
    views::simulate(cx.project.story(), &picks, start, &limits)
}

fn undo(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let label = cx.project.undo_label().map(str::to_owned);
    if cx.project.undo()? {
        Ok(pretty(&json!({"undone": label})))
    } else {
        Ok(pretty(&json!({"undone": null, "note": "nothing to undo"})))
    }
}

fn redo(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let label = cx.project.redo_label().map(str::to_owned);
    if cx.project.redo()? {
        Ok(pretty(&json!({"redone": label})))
    } else {
        Ok(pretty(&json!({"redone": null, "note": "nothing to redo"})))
    }
}

fn save(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    cx.save_requested = true;
    Ok(pretty(&json!({
        "saved": true,
        "note": "Changes are already saved after every successful tool call."
    })))
}

fn list_characters(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    Ok(pretty(&characters_json(cx.project.story())))
}

fn list_variables(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    Ok(pretty(&variables_json(cx.project.story())))
}

fn list_curves(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    Ok(pretty(&crate::views::curves_json(cx.project.story())))
}

fn set_curve(cx: &mut ToolCx<'_>, a: SetCurveArgs) -> Result<String, ToolError> {
    let name = non_empty(&a.name, "curve name")?;
    let curve = parse_curve(a.ease.as_deref(), a.bezier, cx.project.story())?
        .ok_or_else(|| ToolError::invalid("give `ease` (a preset name) or `bezier`"))?;
    if matches!(curve, orbit_core::Curve::Named(_)) {
        return Err(ToolError::invalid(
            "a named curve is a preset or a bezier, it cannot refer to another named curve",
        ));
    }
    exec(
        cx,
        Command::SetCurve {
            name: name.clone(),
            curve: Some(curve),
        },
    )?;
    Ok(pretty(&crate::views::curves_json(cx.project.story())))
}

fn delete_curve(cx: &mut ToolCx<'_>, a: DeleteCurveArgs) -> Result<String, ToolError> {
    orbit_project::ops::delete_curve(cx.project, &a.name)?;
    Ok(pretty(&crate::views::curves_json(cx.project.story())))
}

/// Every built-in tool.
pub fn builtin_tools() -> Vec<ToolDef> {
    vec![
        def::<NoArgs>(
            "list_scenes",
            "List scenes",
            "List the scenes of the story in order, with id, name and node count.",
            Kind::Read,
            list_scenes,
        ),
        def::<SceneArg>(
            "read_scene",
            "Read a scene",
            "Read a scene as numbered lines: index, node id, version and content (dialogue, choices with their target labels, commands). Use the ids with edit/delete/move tools, `after`, and the version as `expected_version`.",
            Kind::Read,
            read_scene,
        ),
        def::<CreateSceneArgs>(
            "create_scene",
            "Create a scene",
            "Create an empty scene. Execution starts at the first node of the first scene and continues into the next scene when one ends.",
            Kind::Edit,
            create_scene,
        ),
        def::<RenameSceneArgs>(
            "rename_scene",
            "Rename a scene",
            "Rename a scene (names are unique).",
            Kind::Edit,
            rename_scene,
        ),
        def::<SceneArg>(
            "delete_scene",
            "Delete a scene",
            "DESTRUCTIVE: delete a scene and all its nodes. Refused while nodes of other scenes still jump to its labels. Can be reverted with `undo`.",
            Kind::Destructive,
            delete_scene,
        ),
        def::<AddDialogueArgs>(
            "add_dialogue",
            "Add a line of dialogue",
            "Add a line spoken by a character (or the narrator) to a scene. With `expression`, a `show` node displaying the speaker is inserted before the line. Returns the new node ids and versions.",
            Kind::Edit,
            add_dialogue,
        ),
        def::<EditDialogueArgs>(
            "edit_dialogue",
            "Edit a line of dialogue",
            "Change the text and/or speaker of an existing dialogue node. Pass `expected_version` (from read_scene) to avoid overwriting a concurrent change.",
            Kind::Edit,
            edit_dialogue,
        ),
        def::<AddChoiceArgs>(
            "add_choice",
            "Add a choice",
            "Add a choice presenting options to the player; each option jumps to a label (create the labels first with add_command type=label). Options may have a condition.",
            Kind::Edit,
            add_choice,
        ),
        def::<AddCommandArgs>(
            "add_command",
            "Add a story command",
            "Add a command node: background, show/hide a character, play/stop music, play sound, wait, set a variable, conditional (if), jump, label, call, return, end, or a plugin-defined node. Targets are label names or ids.",
            Kind::Edit,
            add_command,
        ),
        def::<DeleteNodeArgs>(
            "delete_node",
            "Delete a node",
            "DESTRUCTIVE: delete a node. Refused while another node jumps to it (delete or retarget those first). Can be reverted with `undo`.",
            Kind::Destructive,
            delete_node,
        ),
        def::<MoveNodeArgs>(
            "move_node",
            "Move a node",
            "Move a node within its scene or to another scene. Jumps keep working (they target ids).",
            Kind::Edit,
            move_node,
        ),
        def::<NoArgs>(
            "list_characters",
            "List characters",
            "List characters with their ids, colors and expressions (name -> asset).",
            Kind::Read,
            list_characters,
        ),
        def::<CreateCharacterArgs>(
            "create_character",
            "Create a character",
            "Create a character with a display name, a color and optional expressions (name -> image asset).",
            Kind::Edit,
            create_character,
        ),
        def::<EditCharacterArgs>(
            "edit_character",
            "Edit a character",
            "Rename a character, change its color, add/replace or remove expressions.",
            Kind::Edit,
            edit_character,
        ),
        def::<NoArgs>(
            "list_variables",
            "List variables",
            "List declared variables with type, initial value and scope.",
            Kind::Read,
            list_variables,
        ),
        def::<DeclareVariableArgs>(
            "declare_variable",
            "Declare a variable",
            "Declare (or redeclare) a variable with its initial value, which fixes its type (boolean, number or string).",
            Kind::Edit,
            declare_variable,
        ),
        def::<NoArgs>(
            "list_curves",
            "List easing curves",
            "List the named easing curves of the story (usable as `ease` in transitions), with their definition and how many transitions use them.",
            Kind::Read,
            list_curves,
        ),
        def::<SetCurveArgs>(
            "set_curve",
            "Create or replace an easing curve",
            "Create or replace a named easing curve, from a preset name or a CSS cubic-bezier. Transitions refer to it by name with their `ease` field.",
            Kind::Edit,
            set_curve,
        ),
        def::<DeleteCurveArgs>(
            "delete_curve",
            "Delete an easing curve",
            "Delete a named curve. Refused while a transition still uses it.",
            Kind::Edit,
            delete_curve,
        ),
        def::<ListAssetsArgs>(
            "list_assets",
            "List assets",
            "List the assets of the manifest (id, kind, path relative to the project).",
            Kind::Read,
            list_assets,
        ),
        def::<NoArgs>(
            "translation_status",
            "Translation status",
            "Report, for each translation file (i18n/<story>.<lang>.ron), the coverage (translated, missing, orphaned strings) and the stale translations: those whose source text was edited since they were translated.",
            Kind::Read,
            translation_status,
        ),
        def::<ImportAssetArgs>(
            "import_asset",
            "Import an asset",
            "Register a file that is already inside the project folder as an asset: it is copied under assets/ (unless already there) and added to the manifest. Files outside the project folder are refused.",
            Kind::Edit,
            import_asset,
        ),
        def::<ValidateArgs>(
            "validate",
            "Validate the project",
            "Check the story (references, characters, assets, variables, reachability) and plugin validators. Returns errors and warnings. The same check as `orbit validate`.",
            Kind::Read,
            validate,
        ),
        def::<SimulateArgs>(
            "simulate",
            "Simulate the story",
            "Play the story headlessly and return the transcript (dialogue, choices, presentation events, final variables). Pass `choices` (indexes or texts) to follow a branch; use it to check every branch.",
            Kind::Read,
            simulate,
        ),
        def::<NoArgs>(
            "undo",
            "Undo",
            "Undo the last change (one tool call = one undo entry).",
            Kind::History,
            undo,
        ),
        def::<NoArgs>(
            "redo",
            "Redo",
            "Redo the last undone change.",
            Kind::History,
            redo,
        ),
        def::<NoArgs>(
            "save",
            "Save",
            "Write the story to disk now (atomic). Every successful edit already saves automatically.",
            Kind::Read,
            save,
        ),
    ]
}
