//! Typed errors.

use thiserror::Error;

use crate::defs::AssetKind;
use crate::ids::{AssetId, CharacterId, NodeId, SceneId, StringId};

/// A semantic problem found by [`crate::validate::validate`]. A story with
/// errors must not be played.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    #[error("duplicate scene id {0}")]
    DuplicateSceneId(SceneId),
    #[error("duplicate node id {0}")]
    DuplicateNodeId(NodeId),
    #[error("duplicate character id {0}")]
    DuplicateCharacterId(CharacterId),
    #[error("duplicate asset id {0}")]
    DuplicateAssetId(AssetId),
    #[error("character {character} defines expression `{name}` twice")]
    DuplicateExpression {
        character: CharacterId,
        name: String,
    },
    #[error("duplicate or reserved position name `{0}`")]
    DuplicatePosition(String),
    #[error("duplicate variable declaration `{0}`")]
    DuplicateVariable(String),
    /// The initial value of a list or map variable mixes element types or
    /// exceeds the collection limits (schema v11).
    #[error("variable `{name}` has an invalid initial value: {reason}")]
    InvalidVariableValue { name: String, reason: String },
    #[error("position `{name}` has coordinates outside 0..1")]
    InvalidPositionCoords { name: String },
    #[error("asset {asset} has an invalid path `{path}` (must be relative, no `..`, no `\\`)")]
    InvalidAssetPath { asset: AssetId, path: String },
    #[error("node {from} targets unknown node {target}")]
    UnknownTarget { from: NodeId, target: NodeId },
    #[error("node {from} targets node {target} which is not a Label")]
    TargetNotLabel { from: NodeId, target: NodeId },
    #[error("node {node} jumps to unknown scene {scene}")]
    UnknownScene { node: NodeId, scene: SceneId },
    #[error("node {node} jumps to label {label} which is not in scene {scene}")]
    LabelNotInScene {
        node: NodeId,
        label: NodeId,
        scene: SceneId,
    },
    #[error("choice node {0} has no options")]
    EmptyChoice(NodeId),
    /// `node` is `None` when the string is a character name.
    #[error("{} uses unknown string {id}", owner(.node))]
    UnknownString { node: Option<NodeId>, id: StringId },
    #[error("node {node} uses unknown character {character}")]
    UnknownCharacter {
        node: NodeId,
        character: CharacterId,
    },
    #[error("node {node}: character {character} has no expression `{expression}`")]
    UnknownExpression {
        node: NodeId,
        character: CharacterId,
        expression: String,
    },
    /// `node` is `None` when the asset is used by a character expression.
    #[error("{} uses unknown asset {asset}", owner(.node))]
    UnknownAsset {
        node: Option<NodeId>,
        asset: AssetId,
    },
    #[error("{} uses asset {asset} as {expected} but it is a {found}", owner(.node))]
    AssetKindMismatch {
        node: Option<NodeId>,
        asset: AssetId,
        expected: AssetKind,
        found: AssetKind,
    },
    #[error("node {node} uses unknown position `{name}`")]
    UnknownPosition { node: NodeId, name: String },
    #[error("node {node} has a free position ({x}, {y}) that is not finite or lies outside -2..3")]
    InvalidFreePosition { node: NodeId, x: f32, y: f32 },
    /// `node` is `None` when the scale is a character's default scale.
    #[error("{} has an invalid scale {scale} (must be finite, in 0.05..=10)", owner(.node))]
    InvalidScale { node: Option<NodeId>, scale: f32 },
    #[error("node {node} has an invalid duration {seconds} (must be finite and >= 0)")]
    InvalidDuration { node: NodeId, seconds: f32 },
    #[error("node {node}: {message}")]
    ExprType { node: NodeId, message: String },
    #[error("character {character}, expression `{expression}`: invalid animation ({issue})")]
    InvalidAnimation {
        character: CharacterId,
        expression: String,
        issue: AnimationIssue,
    },
    #[error("asset {asset} file `{path}` does not exist")]
    MissingAssetFile { asset: AssetId, path: String },
    /// A curve has a non-finite control point.
    #[error("{} has an invalid curve (control points must be finite)", curve_owner(.owner))]
    InvalidCurve { owner: CurveOwner },
    /// A named curve is defined as a reference to another named curve.
    #[error("named curve `{0}` refers to another named curve (not allowed)")]
    NamedCurveChain(String),
    /// An `Effect` / `Timeline` node is malformed (bad target, property,
    /// value type, key order or amount); `message` says what.
    #[error("node {node} has an invalid animation: {message}")]
    InvalidTimeline { node: NodeId, message: String },
    /// A forever-looping timeline would block the story.
    #[error("node {0} waits for a timeline that never ends (forever / ping-pong loops cannot wait)")]
    BlockingInfiniteTimeline(NodeId),
    /// An `OpenScreen` / `CloseScreen` node is malformed (no screen, bad
    /// parameter, undeclared result variable); `message` says what.
    #[error("node {node} opens or closes a screen incorrectly: {message}")]
    InvalidScreenNode { node: NodeId, message: String },
    /// A `BackgroundLayers` / `Particles` node is malformed (no layer, bad
    /// asset, depth or emitter parameter, too many particles); `message`
    /// says what.
    #[error("node {node} has invalid background layers or particles: {message}")]
    InvalidStageNode { node: NodeId, message: String },
}

/// Where a curve lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurveOwner {
    /// The transition of a node.
    Node(NodeId),
    /// A named curve of `Story::curves`.
    Named(String),
}

fn curve_owner(o: &CurveOwner) -> String {
    match o {
        CurveOwner::Node(n) => format!("node {n}"),
        CurveOwner::Named(n) => format!("curve `{n}`"),
    }
}

fn owner(node: &Option<NodeId>) -> String {
    match *node {
        Some(n) => format!("node {n}"),
        None => "a character definition".to_owned(),
    }
}

/// A suspicious but playable construct.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationWarning {
    #[error("node {node} reads variable `{var}` which is never declared nor assigned")]
    UndefinedVariable { node: NodeId, var: String },
    #[error(
        "node {node} divides by zero (the divisor is 0 or a variable that is never different from 0)"
    )]
    DivisionByZero { node: NodeId },
    #[error("node {0} is unreachable from the story entry")]
    UnreachableNode(NodeId),
    #[error(
        "node {0} is part of a cycle without Say/Choice/Wait: it loops forever unless a condition breaks it"
    )]
    PossibleInfiniteLoop(NodeId),
    #[error("every option of choice {0} has a condition: the player may get stuck")]
    AllOptionsConditional(NodeId),
    /// `Motion::Rigged` is reserved: the poster image is drawn instead.
    #[error("character {character}, expression `{expression}`: rigged animation is not supported yet, the still image is used")]
    RiggedUnsupported {
        character: CharacterId,
        expression: String,
    },
    /// Execution reaches the end of `scene` without a jump, an `End` or a
    /// `Return`: the game ends there (scenes never fall into the next one).
    /// `node` is the last node reached, `None` for an empty scene.
    #[error("scene {scene} reaches its end without a jump, End or Return: the game ends here")]
    SceneEndsGame {
        scene: SceneId,
        node: Option<NodeId>,
    },
    /// A transition or named curve refers to a curve that is not defined in
    /// `Story::curves`: it is played as linear.
    #[error("{} refers to the unknown curve `{name}`: it is played as linear", curve_owner(.owner))]
    UnknownCurve { owner: CurveOwner, name: String },
    /// A key of a timeline lies after its duration: it is never reached.
    #[error("node {0} has a timeline key after the end of the timeline: it is never reached")]
    TimelineKeyAfterEnd(NodeId),
    /// A `StopParticles` node names an emitter no `Particles` node defines.
    #[error("node {node} stops the emitter `{name}`, which no Particles node defines")]
    UnknownEmitter { node: NodeId, name: String },
}

/// Errors from loading / saving / migrating a story.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("RON parse error: {0}")]
    Parse(#[from] ron::error::SpannedError),
    #[error("RON serialization error: {0}")]
    Serialize(#[from] ron::Error),
    #[error("unsupported schema version {found} (current is {current})")]
    UnsupportedVersion { found: u32, current: u32 },
    #[error("migration from schema version {from} failed: {reason}")]
    Migration { from: u32, reason: String },
    #[error("story failed validation with {} error(s)", .0.len())]
    Invalid(Vec<ValidationError>),
}

/// What is wrong with an expression's animation.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum AnimationIssue {
    #[error("fps must be finite and > 0, got {0}")]
    BadFps(f32),
    #[error("the sprite sheet grid needs at least 1 column and 1 row")]
    EmptyGrid,
    #[error("a sprite sheet needs at least 1 frame")]
    NoFrames,
    #[error("frame count {frame_count} exceeds the {capacity} cells of the grid")]
    FrameCountExceedsGrid { frame_count: u32, capacity: u64 },
    #[error("the frame size must be at least 1x1 pixel")]
    BadFrameSize,
    #[error("an image sequence needs at least 1 image")]
    EmptySequence,
}
