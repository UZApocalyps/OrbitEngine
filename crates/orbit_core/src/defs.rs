//! Definitions referenced by story nodes: characters, positions, assets,
//! strings, variables and transitions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use orbit_anim::Curve;

use crate::ids::{AssetId, CharacterId, StringId};
use crate::value::Value;

/// 8-bit RGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// How a visual change is animated.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TransitionKind {
    /// Fades to `Color`, then to the new content.
    Fade(Color),
    /// Cross-fades from the old content to the new one.
    Dissolve,
}

/// A timed transition. A duration of `0` behaves like a cut.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Transition {
    pub kind: TransitionKind,
    /// Seconds.
    pub duration: f32,
    /// Easing of the progress (schema v9). `None` is linear, as before.
    #[serde(default)]
    pub ease: Option<Curve>,
}

impl Transition {
    pub fn fade(color: Color, duration: f32) -> Self {
        Self {
            kind: TransitionKind::Fade(color),
            duration,
            ease: None,
        }
    }

    pub fn dissolve(duration: f32) -> Self {
        Self {
            kind: TransitionKind::Dissolve,
            duration,
            ease: None,
        }
    }

    /// The same transition with an easing curve.
    pub fn with_ease(mut self, ease: Curve) -> Self {
        self.ease = Some(ease);
        self
    }
}

/// Smallest accepted sprite scale (1.0 = native size, see the runtime).
pub const SCALE_MIN: f32 = 0.05;
/// Largest accepted sprite scale.
pub const SCALE_MAX: f32 = 10.0;
/// Free positions may lie slightly off screen (a character walking in), but
/// stay within this normalized range on both axes.
pub const FREE_POSITION_RANGE: std::ops::RangeInclusive<f32> = -2.0..=3.0;

/// Whether `scale` is a valid sprite scale: finite and within
/// [`SCALE_MIN`]..=[`SCALE_MAX`].
pub fn scale_is_valid(scale: f32) -> bool {
    scale.is_finite() && (SCALE_MIN..=SCALE_MAX).contains(&scale)
}

/// Where a character stands on stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Position {
    Left,
    Center,
    Right,
    /// A position defined in [`crate::Story::positions`], by name.
    Custom(String),
    /// A free position given inline, in normalized stage coordinates (origin
    /// top-left, the sprite is anchored by its bottom-center point). Added in
    /// schema v6; this is what the scene view writes when a character is
    /// dragged.
    Free {
        x: f32,
        y: f32,
    },
}

/// A custom stage position in normalized screen coordinates: `x` and `y` are
/// in `0.0..=1.0` with the origin at the top-left corner. A sprite is anchored
/// by its bottom-center point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PositionDef {
    pub name: String,
    pub x: f32,
    pub y: f32,
}

/// Normalized coordinates of the built-in positions.
pub fn builtin_position(position: &Position) -> Option<(f32, f32)> {
    match position {
        Position::Left => Some((0.25, 1.0)),
        Position::Center => Some((0.5, 1.0)),
        Position::Right => Some((0.75, 1.0)),
        Position::Custom(_) | Position::Free { .. } => None,
    }
}

/// One expression (sprite) of a character: a still picture (`image`) that
/// can be turned into an animation by `motion` (schema v8). With a motion the
/// `image` stays the *poster*: the picture used by thumbnails and as a
/// fallback while the frames are not loaded (the editor keeps it on the
/// first frame / the sheet).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Expression {
    pub name: String,
    pub image: AssetId,
    /// `None` for a still image (not written to a RON file then).
    #[serde(default)]
    pub motion: Option<Motion>,
}

/// Same trick as [`Character`]: `motion` is only left out of human readable
/// formats, a binary one must always write it.
impl Serialize for Expression {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let write_motion = self.motion.is_some() || !serializer.is_human_readable();
        let mut st = serializer.serialize_struct("Expression", 2 + usize::from(write_motion))?;
        st.serialize_field("name", &self.name)?;
        st.serialize_field("image", &self.image)?;
        if write_motion {
            st.serialize_field("motion", &self.motion)?;
        } else {
            st.skip_field("motion")?;
        }
        st.end()
    }
}

impl Expression {
    /// A still expression.
    pub fn still(name: impl Into<String>, image: AssetId) -> Self {
        Self {
            name: name.into(),
            image,
            motion: None,
        }
    }

    /// The frame animation, when this expression has one.
    pub fn animation(&self) -> Option<&Animation> {
        match &self.motion {
            Some(Motion::Animation(a)) => Some(a),
            _ => None,
        }
    }

    /// Every asset this expression needs (poster first, then the frames),
    /// without duplicates.
    pub fn assets(&self) -> Vec<AssetId> {
        let mut out = vec![self.image];
        if let Some(a) = self.animation() {
            for id in a.frames.assets() {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        out
    }
}

/// How an expression moves.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Motion {
    /// Frame animation (sprite sheet or image sequence).
    Animation(Animation),
    /// Skeletal animation. Reserved: not supported yet (validation warns,
    /// the poster image is drawn).
    Rigged,
}

/// What to do when the last frame is reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum LoopMode {
    /// Starts over.
    #[default]
    Loop,
    /// Plays once and holds the last frame.
    Once,
    /// Forward then backward, endlessly.
    PingPong,
}

/// Where the frames of an animation come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AnimFrames {
    /// One image holding a `cols` x `rows` grid read left to right, top to
    /// bottom; only the first `frame_count` cells are frames. `frame_size`
    /// (pixels) overrides the cell size (default: image size / grid) for
    /// sheets with padding.
    SpriteSheet {
        asset: AssetId,
        cols: u32,
        rows: u32,
        frame_count: u32,
        #[serde(default)]
        frame_size: Option<[u32; 2]>,
    },
    /// One image per frame, in order.
    Sequence(Vec<AssetId>),
}

impl AnimFrames {
    /// Number of frames.
    pub fn len(&self) -> usize {
        match self {
            AnimFrames::SpriteSheet { frame_count, .. } => *frame_count as usize,
            AnimFrames::Sequence(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The assets the frames are read from.
    pub fn assets(&self) -> Vec<AssetId> {
        match self {
            AnimFrames::SpriteSheet { asset, .. } => vec![*asset],
            AnimFrames::Sequence(v) => v.clone(),
        }
    }
}

/// A frame animation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Animation {
    pub frames: AnimFrames,
    /// Frames per second (> 0).
    pub fps: f32,
    #[serde(default)]
    pub loop_mode: LoopMode,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Character {
    pub id: CharacterId,
    /// Display name, localizable.
    pub name_id: StringId,
    pub color: Color,
    #[serde(default)]
    pub expressions: Vec<Expression>,
    /// Where the character stands when shown without an explicit choice
    /// (editor convenience: pre-fills new `Show` nodes). Absent in the RON
    /// file when unset, so existing stories are unchanged.
    #[serde(default)]
    pub default_position: Option<Position>,
    /// Scale applied when a `Show` does not override it (schema v6). `1.0`
    /// is the native size; not written to the file when it is `1.0`.
    #[serde(default = "default_scale")]
    pub default_scale: f32,
}

fn default_scale() -> f32 {
    1.0
}

/// Written by hand (instead of `skip_serializing_if`) so that the field is
/// only left out of *human readable* formats: a binary, non self-describing
/// format such as the postcard game pack must always write it, or the reader
/// would misalign.
impl Serialize for Character {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let write_default = self.default_position.is_some() || !serializer.is_human_readable();
        let write_scale = self.default_scale != 1.0 || !serializer.is_human_readable();
        let mut st = serializer.serialize_struct(
            "Character",
            4 + usize::from(write_default) + usize::from(write_scale),
        )?;
        st.serialize_field("id", &self.id)?;
        st.serialize_field("name_id", &self.name_id)?;
        st.serialize_field("color", &self.color)?;
        st.serialize_field("expressions", &self.expressions)?;
        if write_default {
            st.serialize_field("default_position", &self.default_position)?;
        } else {
            st.skip_field("default_position")?;
        }
        if write_scale {
            st.serialize_field("default_scale", &self.default_scale)?;
        } else {
            st.skip_field("default_scale")?;
        }
        st.end()
    }
}

impl Character {
    pub fn expression(&self, name: &str) -> Option<&Expression> {
        self.expressions.iter().find(|e| e.name == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AssetKind {
    Image,
    Music,
    Sound,
    /// A TTF/OTF font. Added without a schema bump: stories that do not use
    /// it are unchanged, and older builds never wrote it.
    Font,
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AssetKind::Image => "image",
            AssetKind::Music => "music",
            AssetKind::Sound => "sound",
            AssetKind::Font => "font",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AssetEntry {
    pub id: AssetId,
    pub kind: AssetKind,
    /// Path relative to the project root, with `/` separators.
    pub path: String,
}

/// The asset manifest: stable id -> relative path. Nodes only ever refer to
/// assets by id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AssetManifest {
    #[serde(default)]
    pub assets: Vec<AssetEntry>,
}

impl AssetManifest {
    pub fn get(&self, id: AssetId) -> Option<&AssetEntry> {
        self.assets.iter().find(|a| a.id == id)
    }
}

/// Source-language string table: every displayed text has a stable
/// [`StringId`]. Other languages will be separate tables keyed by the same
/// ids (localization is not implemented yet).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct StringTable {
    /// BCP 47 tag of the source language, e.g. `"en"`.
    pub source_lang: String,
    #[serde(default)]
    pub entries: BTreeMap<StringId, String>,
}

impl Default for StringTable {
    fn default() -> Self {
        Self {
            source_lang: "en".into(),
            entries: BTreeMap::new(),
        }
    }
}

impl StringTable {
    pub fn get(&self, id: StringId) -> Option<&str> {
        self.entries.get(&id).map(String::as_str)
    }

    /// Inserts or replaces a string and returns the previous value.
    pub fn set(&mut self, id: StringId, text: impl Into<String>) -> Option<String> {
        self.entries.insert(id, text.into())
    }

    /// Adds a new string under a fresh id.
    pub fn add(&mut self, text: impl Into<String>) -> StringId {
        let id = StringId::new();
        self.entries.insert(id, text.into());
        id
    }
}

/// Where a variable lives.
///
/// * `Game`: part of the playthrough. Saved with the game, restored by load
///   and rollback.
/// * `Persistent`: survives across playthroughs (e.g. "endings seen", number
///   of completed games). Stored by the host in a separate file; it is **not**
///   part of the VM state, so loading a save or rolling back never changes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum VarScope {
    #[default]
    Game,
    Persistent,
}

/// Declares a variable and its initial value (and thus its type).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VarDecl {
    pub name: String,
    pub initial: Value,
    /// Defaults to [`VarScope::Game`] (schema v3).
    #[serde(default)]
    pub scope: VarScope,
}

impl VarDecl {
    /// A playthrough variable.
    pub fn game(name: impl Into<String>, initial: Value) -> Self {
        Self {
            name: name.into(),
            initial,
            scope: VarScope::Game,
        }
    }

    /// A variable that persists across playthroughs.
    pub fn persistent(name: impl Into<String>, initial: Value) -> Self {
        Self {
            name: name.into(),
            initial,
            scope: VarScope::Persistent,
        }
    }
}
