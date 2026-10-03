//! The story IR: definitions and scenes made of nodes.

use std::collections::BTreeMap;

use orbit_anim::Curve;
use serde::{Deserialize, Serialize};

use crate::custom::CustomData;
use crate::defs::{
    AssetManifest, Character, Position, PositionDef, StringTable, Transition, VarDecl,
    builtin_position,
};
use crate::error::CoreError;
use crate::expr::Expr;
use crate::ids::{AssetId, CharacterId, NodeId, SceneId, StringId};
use crate::migrate;
use crate::validate;

/// Version of the on-disk schema produced by this build.
pub const CURRENT_SCHEMA_VERSION: u32 = 9;

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
const DEFAULT_LINES: [&str; 5] = [
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
