//! Definitions referenced by story nodes: characters, positions, assets,
//! strings, variables and transitions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::{AssetId, CharacterId, StringId};
use crate::value::Value;

/// 8-bit RGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
pub enum TransitionKind {
    /// Fades to `Color`, then to the new content.
    Fade(Color),
    /// Cross-fades from the old content to the new one.
    Dissolve,
}

/// A timed transition. A duration of `0` behaves like a cut.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Seconds.
    pub duration: f32,
}

impl Transition {
    pub fn fade(color: Color, duration: f32) -> Self {
        Self {
            kind: TransitionKind::Fade(color),
            duration,
        }
    }

    pub fn dissolve(duration: f32) -> Self {
        Self {
            kind: TransitionKind::Dissolve,
            duration,
        }
    }
}

/// Where a character stands on stage.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Position {
    Left,
    Center,
    Right,
    /// A position defined in [`crate::Story::positions`], by name.
    Custom(String),
}

/// A custom stage position in normalized screen coordinates: `x` and `y` are
/// in `0.0..=1.0` with the origin at the top-left corner. A sprite is anchored
/// by its bottom-center point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        Position::Custom(_) => None,
    }
}

/// One expression (sprite) of a character.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Expression {
    pub name: String,
    pub image: AssetId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Character {
    pub id: CharacterId,
    /// Display name, localizable.
    pub name_id: StringId,
    pub color: Color,
    #[serde(default)]
    pub expressions: Vec<Expression>,
}

impl Character {
    pub fn expression(&self, name: &str) -> Option<&Expression> {
        self.expressions.iter().find(|e| e.name == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetKind {
    Image,
    Music,
    Sound,
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AssetKind::Image => "image",
            AssetKind::Music => "music",
            AssetKind::Sound => "sound",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetEntry {
    pub id: AssetId,
    pub kind: AssetKind,
    /// Path relative to the project root, with `/` separators.
    pub path: String,
}

/// The asset manifest: stable id -> relative path. Nodes only ever refer to
/// assets by id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Declares a game variable and its initial value (and thus its type).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VarDecl {
    pub name: String,
    pub initial: Value,
}
