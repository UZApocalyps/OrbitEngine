//! The story IR: definitions and scenes made of nodes.

use std::collections::BTreeMap;

use orbit_anim::{Curve, Loop};
use serde::{Deserialize, Serialize};

use crate::custom::CustomData;
use crate::achievements::Achievement;
use crate::defs::{
    AssetManifest, Character, Position, PositionDef, StringTable, Transition, VarDecl,
    builtin_position,
};
use crate::effects::{Effect, StoryTrack};
use crate::error::CoreError;
use crate::expr::Expr;
use crate::ids::{AchievementId, AssetId, CharacterId, NodeId, SceneId, StringId};
use crate::migrate;
use crate::validate;

/// Version of the on-disk schema produced by this build.
pub const CURRENT_SCHEMA_VERSION: u32 = 16;

/// A whole story: definitions (characters, assets, strings...) and an ordered
/// list of scenes. Execution starts at the first node of the first non-empty
/// scene. A scene never falls through into the next one: execution passes from
/// scene to scene only through an explicit [`NodeKind::JumpScene`], and a
/// scene whose last node is reached without a jump ends the game.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Story {
    pub schema_version: u32,
    pub title: String,
    /// Source-language texts, by stable id.
    #[serde(default)]
    pub strings: StringTable,
    /// Asset manifest (id -> relative path).
    #[serde(default)]
    pub assets: AssetManifest,
    #[serde(default)]
    pub characters: Vec<Character>,
    /// Custom stage positions (`left`, `center`, `right` are built in).
    #[serde(default)]
    pub positions: Vec<PositionDef>,
    /// Declared variables with their initial values.
    #[serde(default)]
    pub variables: Vec<VarDecl>,
    /// Named easing curves (schema v9), referenced by `Curve::Named`.
    #[serde(default)]
    pub curves: BTreeMap<String, Curve>,
    /// Achievements the player can unlock (schema v16, M7-05); unlocked by
    /// [`NodeKind::Unlock`]. Not written to the RON file when empty.
    #[serde(default, deserialize_with = "crate::achievements::v16_field", serialize_with = "crate::achievements::v16_skip")]
    pub achievements: Vec<Achievement>,
    pub scenes: Vec<Scene>,
}

/// A scene: a linear list of nodes. Control flow is expressed with
/// `Jump` / `Choice` / `If` / `Call` nodes targeting `Label` nodes by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Scene {
    pub id: SceneId,
    pub name: String,
    pub nodes: Vec<Node>,
}

/// A node with its stable id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum NodeKind {
    /// Jump destination. Names are for humans; targeting is done by id.
    Label {
        name: String,
    },
    /// A line of dialogue (`speaker: None` is the narrator). `text_id` is the
    /// stable localization key into [`Story::strings`].
    Say {
        #[serde(default)]
        speaker: Option<CharacterId>,
        text_id: StringId,
        /// Voice acting (schema v16, M7-06): a `Sound` asset played with the
        /// line. A translation can replace it (`orbit_i18n::Translation`).
        /// Not written to the RON file when `None`.
        #[serde(default, deserialize_with = "crate::achievements::v16_field", serialize_with = "crate::achievements::v16_skip")]
        voice: Option<AssetId>,
    },
    /// Presents options to the player; each option jumps to a Label. Options
    /// whose condition is false are hidden.
    Choice {
        options: Vec<ChoiceOption>,
    },
    /// Unconditional jump to a Label.
    Jump {
        target: NodeId,
    },
    /// Unconditional jump to another scene (schema v5): to its first node,
    /// or to `label` (a `Label` of that scene) when set. This is the only way
    /// to leave a scene: it does not fall through into the next one.
    JumpScene {
        scene: SceneId,
        #[serde(default)]
        label: Option<NodeId>,
    },
    /// Assigns the value of an expression to a variable.
    SetVar {
        var: String,
        value: Expr,
    },
    /// Jumps to `then_target` if the condition holds, otherwise to
    /// `else_target`, or falls through to the next node when it is `None`.
    If {
        condition: Expr,
        then_target: NodeId,
        #[serde(default)]
        else_target: Option<NodeId>,
    },
    /// Changes the background image.
    Background {
        asset: AssetId,
        #[serde(default)]
        transition: Option<Transition>,
    },
    /// Shows (or updates) a character on stage. Showing a character that is
    /// already there replaces its expression, position, scale and mirroring
    /// (a `Show` always describes the whole look), animated by `transition`.
    Show {
        character: CharacterId,
        expression: String,
        position: Position,
        #[serde(default)]
        transition: Option<Transition>,
        /// Sprite scale (uniform, or horizontal when `scale_y` is set).
        /// `None` uses the character's `default_scale` (schema v6).
        #[serde(default)]
        scale: Option<f32>,
        /// Vertical scale for a non-uniform stretch; `None` keeps the
        /// horizontal scale (schema v6).
        #[serde(default)]
        scale_y: Option<f32>,
        /// Mirrors the sprite on the X axis (schema v6).
        #[serde(default)]
        flip_x: bool,
        /// Mirrors the sprite on the Y axis (schema v6).
        #[serde(default)]
        flip_y: bool,
    },
    /// Removes a character from the stage.
    Hide {
        character: CharacterId,
        #[serde(default)]
        transition: Option<Transition>,
    },
    /// Starts a music track (replacing the current one).
    PlayMusic {
        asset: AssetId,
        #[serde(default)]
        fade_in: f32,
        #[serde(default)]
        looping: bool,
    },
    StopMusic {
        #[serde(default)]
        fade_out: f32,
    },
    PlaySound {
        asset: AssetId,
    },
    /// Pauses for a number of seconds.
    Wait {
        seconds: f32,
    },
    /// Calls the routine starting at a Label; `Return` comes back here.
    Call {
        label: NodeId,
    },
    /// Returns from the innermost `Call`.
    Return,
    /// Explicitly ends the story.
    End,
    /// A node defined by a plugin (schema v4). The VM does not interpret it:
    /// it emits a generic presentation event and moves on. A project opened
    /// without the plugin keeps the node untouched (lossless round trip).
    /// `version` is the plugin's own data version, for its own migrations.
    Custom {
        plugin_id: String,
        kind: String,
        #[serde(default)]
        version: u32,
        #[serde(default)]
        data: CustomData,
    },
    /// Shows a 3D scene (`.scene3d.ron` asset) as the background layer, seen
    /// from one of its named cameras (`None`: its first camera). The 2D
    /// background is hidden but kept (it comes back with `HideScene3d`); a
    /// later `Background` replaces the 3D scene (ADR 0014, M6-07 / M6-11).
    /// The three 3D variants are appended last: additive, older stories and
    /// paks keep their encoding.
    ShowScene3d {
        scene: AssetId,
        #[serde(default)]
        camera: Option<String>,
        #[serde(default)]
        transition: Option<Transition>,
    },
    /// Switches the camera of the 3D scene on screen (cut, or a fade /
    /// dissolve between the two views). Ignored when no 3D scene is shown.
    Camera3d {
        camera: String,
        #[serde(default)]
        transition: Option<Transition>,
    },
    /// Removes the 3D scene: the 2D background (if any) shows again.
    HideScene3d {
        #[serde(default)]
        transition: Option<Transition>,
    },
    /// A simple animated effect (move, shake, zoom, flash, tint), schema v9.
    /// Compiled to a small timeline by [`Effect::tracks`].
    Effect(Effect),
    /// Parallel keyframe tracks (schema v9). `wait` blocks the story until
    /// the end; otherwise the animation runs while the story goes on.
    /// `Loop::Forever` / `PingPong` require `wait: false`.
    Timeline {
        /// Seconds of one pass.
        duration: f32,
        #[serde(default)]
        looping: Loop,
        #[serde(default)]
        wait: bool,
        tracks: Vec<StoryTrack>,
    },
    /// Opens a data-driven screen of the project (`ui/*.screen.ron`, by its
    /// id) over the game (schema v12, UI-14, ADR 0016). With `wait` the story
    /// stops on this node until the screen closes; `result` receives the
    /// value of its `Close` action (`UiAction::CloseWith`). `params` are
    /// copied into the screen (`BindPath::Param`) when it opens. Opening a
    /// screen that is already open replaces it. Which screens are open is
    /// part of the VM state: rollback, saves and skip handle it.
    OpenScreen {
        screen: String,
        #[serde(default)]
        wait: bool,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        params: Vec<ScreenParam>,
    },
    /// Closes a screen opened by `OpenScreen` (typically a HUD). Closing a
    /// screen that is not open does nothing.
    CloseScreen {
        screen: String,
    },
    /// A background made of several layers with a depth factor (parallax,
    /// schema v13, M5A-07), back to front. Replaces the background (a later
    /// `Background` replaces the layers). The camera pan and zoom tracks move
    /// each layer by its depth. `transition` is the one of `Background`.
    BackgroundLayers {
        layers: Vec<crate::particles::BgLayer>,
        #[serde(default)]
        transition: Option<Transition>,
    },
    /// Starts a particle emitter (schema v13, M5A-08). It runs until
    /// `StopParticles`, a `Particles` node with the same name, or the next
    /// scene. Particles are recomputed from the start time, never stored.
    Particles(crate::particles::Emitter),
    /// Stops the births of the emitter called `name` (all emitters when the
    /// name is empty); particles already alive finish their life.
    StopParticles {
        #[serde(default)]
        name: String,
    },
    /// Runs a Rhai script (schema v15, ADR 0015 §1-§3), synchronously and
    /// within the sandbox limits. The script reads variables (`get_var`),
    /// queues writes (`set_var`), draws from the VM's seeded RNG and may
    /// choose a branch (`branch("name")` or by returning the name). The VM
    /// applies the writes after the script, all or nothing: a failing
    /// script changes nothing and jumps to `on_error` (or goes on with the
    /// next node, with a warning). Shared modules (`scripts/*.rhai`) are
    /// reached with `import "name" as m;` (a "module function call" is an
    /// inline source such as `import "shop" as shop; shop::buy("sword")`).
    /// Appended last: older encodings keep their variant indices.
    Script {
        source: String,
        /// Named outputs: a script choosing `name` jumps to `target` (a
        /// `Label`). Without a choice the story goes on with the next node.
        #[serde(default)]
        branches: Vec<ScriptBranch>,
        /// Where to go when the script fails (a `Label`); `None`: next node.
        #[serde(default)]
        on_error: Option<NodeId>,
    },
    /// Unlocks an achievement (schema v16, M7-05). The first time it runs
    /// in a (non-replay) playthrough it unlocks the achievement for good
    /// (persistent data) and announces it with a toast; afterwards, and
    /// after a rollback, it does nothing. Appended last.
    Unlock {
        achievement: AchievementId,
    },
}

/// A named output of a [`NodeKind::Script`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ScriptBranch {
    pub name: String,
    pub target: NodeId,
}

/// A value handed to a screen by [`NodeKind::OpenScreen`]: `name` is what
/// the screen binds (`BindPath::Param(name)`), `value` a literal or a story
/// variable read when the node runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ScreenParam {
    pub name: String,
    pub value: crate::varops::Operand,
}

impl NodeKind {
    /// Nodes this node can jump to (always `Label` nodes in a valid story).
    pub fn targets(&self) -> Vec<NodeId> {
        match self {
            NodeKind::Jump { target } => vec![*target],
            NodeKind::JumpScene { label, .. } => label.iter().copied().collect(),
            NodeKind::Call { label } => vec![*label],
            NodeKind::If {
                then_target,
                else_target,
                ..
            } => std::iter::once(*then_target).chain(*else_target).collect(),
            NodeKind::Choice { options } => options.iter().map(|o| o.target).collect(),
            NodeKind::Script {
                branches, on_error, ..
            } => branches
                .iter()
                .map(|b| b.target)
                .chain(*on_error)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The scene this node jumps to, for a `JumpScene`.
    pub fn target_scene(&self) -> Option<SceneId> {
        match self {
            NodeKind::JumpScene { scene, .. } => Some(*scene),
            _ => None,
        }
    }

    /// Strings displayed by this node.
    pub fn strings(&self) -> Vec<StringId> {
        match self {
            NodeKind::Say { text_id, .. } => vec![*text_id],
            NodeKind::Choice { options } => options.iter().map(|o| o.text_id).collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ChoiceOption {
    pub text_id: StringId,
    pub target: NodeId,
    /// The option is only offered when this evaluates to `true`.
    #[serde(default)]
    pub condition: Option<Expr>,
}

impl Story {
    /// An empty story at the current schema version.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            title: title.into(),
            strings: StringTable::default(),
            assets: AssetManifest::default(),
            characters: Vec::new(),
            positions: Vec::new(),
            variables: Vec::new(),
            curves: BTreeMap::new(),
            achievements: Vec::new(),
            scenes: Vec::new(),
        }
    }

    /// Parses RON, migrating older schema versions, **without** semantic
    /// validation (useful for editors that tolerate broken stories).
    pub fn from_ron_unchecked(source: &str) -> Result<Story, CoreError> {
        let version = migrate::read_version(source)?;
        let migrated = migrate::migrate(version, source)?;
        Ok(ron::from_str(&migrated)?)
    }

    /// Parses RON, migrates, and validates. Fails with
    /// [`CoreError::Invalid`] listing every error found (warnings are ignored).
    pub fn from_ron(source: &str) -> Result<Story, CoreError> {
        let story = Self::from_ron_unchecked(source)?;
        let report = validate::validate(&story);
        if report.errors.is_empty() {
            Ok(story)
        } else {
            Err(CoreError::Invalid(report.errors))
        }
    }

    /// Serializes to pretty RON.
    pub fn to_ron(&self) -> Result<String, CoreError> {
        let cfg = ron::ser::PrettyConfig::new().indentor("    ".to_owned());
        Ok(strip_default_fields(&ron::ser::to_string_pretty(
            self, cfg,
        )?))
    }

    /// Finds a node and returns `(scene index, node index)`.
    pub fn locate(&self, id: NodeId) -> Option<(usize, usize)> {
        self.scenes.iter().enumerate().find_map(|(si, scene)| {
            scene
                .nodes
                .iter()
                .position(|n| n.id == id)
                .map(|ni| (si, ni))
        })
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        let (s, n) = self.locate(id)?;
        Some(&self.scenes[s].nodes[n])
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let (s, n) = self.locate(id)?;
        Some(&mut self.scenes[s].nodes[n])
    }

    pub fn scene(&self, id: SceneId) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    /// Where a jump to `scene` lands: `label` when given, else the first
    /// node of the scene (`None` for an empty scene).
    pub fn scene_entry(&self, scene: SceneId, label: Option<NodeId>) -> Option<NodeId> {
        label.or_else(|| self.scene(scene)?.nodes.first().map(|n| n.id))
    }

    /// First node of the first non-empty scene.
    pub fn entry(&self) -> Option<NodeId> {
        self.scenes
            .iter()
            .find_map(|s| s.nodes.first())
            .map(|n| n.id)
    }

    /// Names of the variables declared with [`crate::VarScope::Persistent`].
    pub fn persistent_variables(&self) -> impl Iterator<Item = &VarDecl> {
        self.variables
            .iter()
            .filter(|v| v.scope == crate::VarScope::Persistent)
    }

    pub fn character(&self, id: CharacterId) -> Option<&Character> {
        self.characters.iter().find(|c| c.id == id)
    }

    /// Text of a string in the source language.
    pub fn text(&self, id: StringId) -> Option<&str> {
        self.strings.get(id)
    }

    /// Relative path of an asset, from the manifest.
    pub fn asset_path(&self, id: AssetId) -> Option<&str> {
        self.assets.get(id).map(|a| a.path.as_str())
    }

    /// Normalized stage coordinates of a position (built-in or custom).
    pub fn resolve_position(&self, position: &Position) -> Option<(f32, f32)> {
        match position {
            Position::Custom(name) => self
                .positions
                .iter()
                .find(|p| &p.name == name)
                .map(|p| (p.x, p.y)),
            Position::Free { x, y } => Some((*x, *y)),
            builtin => builtin_position(builtin),
        }
    }
}

/// Lines `to_ron` leaves out: the v6 look fields of `Show` and the character
/// scale at their default. They are always written by `Serialize` (a binary,
/// non self-describing format such as the postcard game pack would misalign
/// on a skipped field) and dropped from the human-readable RON, where each
/// field is on its own line and these names appear nowhere else as keys.
const DEFAULT_LINES: [&str; 7] = [
    "voice: None,",
    "achievements: [],",
    "scale: None,",
    "scale_y: None,",
    "flip_x: false,",
    "flip_y: false,",
    "default_scale: 1.0,",

];

fn strip_default_fields(ron: &str) -> String {
    let mut out = String::with_capacity(ron.len());
    for line in ron.split_inclusive('\n') {
        if !DEFAULT_LINES.contains(&line.trim()) {
            out.push_str(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Story {
        let label = Node {
            id: NodeId::new(),
            kind: NodeKind::Label { name: "end".into() },
        };
        let mut story = Story::new("t");
        let text_id = story.strings.add("Hello");
        let say = Node {
            id: NodeId::new(),
            kind: NodeKind::Say {
                speaker: None,
                text_id,
                voice: None,
            },
        };
        let jump = Node {
            id: NodeId::new(),
            kind: NodeKind::Jump { target: label.id },
        };
        story.scenes.push(Scene {
            id: SceneId::new(),
            name: "main".into(),
            nodes: vec![say, jump, label],
        });
        story
    }

    #[test]
    fn ron_roundtrip() {
        let story = sample();
        let text = story.to_ron().unwrap();
        let back = Story::from_ron(&text).unwrap();
        assert_eq!(story, back);
    }

    #[test]
    fn custom_node_survives_a_ron_round_trip() {
        let mut story = sample();
        let custom = Node {
            id: NodeId::new(),
            kind: NodeKind::Custom {
                plugin_id: "acme.anim".into(),
                kind: "play_clip".into(),
                version: 2,
                data: CustomData::Map(vec![
                    ("clip".into(), CustomData::Str("walk".into())),
                    ("loops".into(), CustomData::Int(3)),
                    ("speed".into(), CustomData::Float(1.5)),
                    (
                        "tags".into(),
                        CustomData::List(vec![CustomData::Bool(true), CustomData::Null]),
                    ),
                ]),
            },
        };
        story.scenes[0].nodes.insert(1, custom);
        let text = story.to_ron().unwrap();
        let back = Story::from_ron(&text).unwrap();
        assert_eq!(story, back);
    }

    #[test]
    fn newer_schema_is_rejected() {
        let mut story = sample();
        story.schema_version = CURRENT_SCHEMA_VERSION + 1;
        let text = story.to_ron().unwrap();
        assert!(matches!(
            Story::from_ron(&text),
            Err(CoreError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn garbage_is_a_parse_error() {
        assert!(matches!(Story::from_ron("((("), Err(CoreError::Parse(_))));
    }

    #[test]
    fn locate_finds_nodes() {
        let story = sample();
        let id = story.scenes[0].nodes[1].id;
        assert_eq!(story.locate(id), Some((0, 1)));
        assert!(story.node(NodeId::new()).is_none());
        assert_eq!(story.entry(), Some(story.scenes[0].nodes[0].id));
    }

    #[test]
    fn positions_resolve() {
        let mut story = sample();
        story.positions.push(PositionDef {
            name: "far".into(),
            x: 0.1,
            y: 0.9,
        });
        assert_eq!(story.resolve_position(&Position::Center), Some((0.5, 1.0)));
        assert_eq!(
            story.resolve_position(&Position::Custom("far".into())),
            Some((0.1, 0.9))
        );
        assert_eq!(story.resolve_position(&Position::Custom("x".into())), None);
    }
}
