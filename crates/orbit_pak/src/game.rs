//! The compiled game: story, translations and identity, in postcard.
//!
//! ```text
//! "ORBITGAM"  magic (8 bytes)
//! version     u32 LE  (currently 1)
//! postcard    GamePack
//! ```
//!
//! This is what `orbit build` produces from `story.ron` and the translation
//! files of a project: parsing RON is skipped at start-up, and a player needs
//! no access to the project files. The pack is the entry [`GAME_PACK_ENTRY`]
//! of the pak.

use orbit_core::Story;
use orbit_i18n::Translation;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Name of the pak entry that holds the game pack.
pub const GAME_PACK_ENTRY: &str = "game.pack";

/// Folder of the pak that holds the files of the engine itself (the fonts):
/// a requested `assets/fonts/Inter-Regular.ttf` is `engine/assets/fonts/...`.
pub const ENGINE_PREFIX: &str = "engine";

/// Pak entry of the game icon (a PNG).
pub const ICON_ENTRY: &str = "meta/icon.png";

const MAGIC: &[u8; 8] = b"ORBITGAM";
/// Current game pack version.
pub const PACK_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PackError {
    #[error("not a game pack (bad magic)")]
    BadMagic,
    #[error("game pack version {found} is not supported (this build reads up to {PACK_VERSION})")]
    UnsupportedVersion { found: u32 },
    #[error("the game pack is corrupt: {0}")]
    Corrupt(String),
}

/// Who the game is: shown in the window title, used to name the saves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameInfo {
    /// Display name (window title, shortcuts).
    pub name: String,
    /// Stable identifier of the game: where saves and preferences live. Must
    /// not change between versions or players lose their saves.
    pub id: String,
    /// Version of the game (free text, `1.0.0`).
    pub version: String,
    /// Language used when the player has no saved preference (BCP 47);
    /// `None` follows the system language.
    pub default_language: Option<String>,
}

/// A compiled game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GamePack {
    pub info: GameInfo,
    pub story: Story,
    pub translations: Vec<Translation>,
}

impl GamePack {
    pub fn to_bytes(&self) -> Result<Vec<u8>, PackError> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&PACK_VERSION.to_le_bytes());
        let body = postcard::to_stdvec(self).map_err(|e| PackError::Corrupt(e.to_string()))?;
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// `true` when `bytes` start like a game pack (as opposed to RON text).
    pub fn is_pack(bytes: &[u8]) -> bool {
        bytes.starts_with(MAGIC)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PackError> {
        if !Self::is_pack(bytes) {
            return Err(PackError::BadMagic);
        }
        let version = bytes
            .get(8..12)
            .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
            .ok_or_else(|| PackError::Corrupt("truncated".into()))?;
        if version == 0 || version > PACK_VERSION {
            return Err(PackError::UnsupportedVersion { found: version });
        }
        postcard::from_bytes(&bytes[12..]).map_err(|e| PackError::Corrupt(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = include_str!("../../../examples/stories/demo.ron");
    const FR: &str = include_str!("../../../examples/stories/i18n/demo.fr.ron");

    fn demo_pack() -> GamePack {
        GamePack {
            info: GameInfo {
                name: "Demo".into(),
                id: "demo".into(),
                version: "1.2.3".into(),
                default_language: Some("fr".into()),
            },
            story: Story::from_ron_unchecked(DEMO).unwrap(),
            translations: vec![Translation::from_ron(FR).unwrap()],
        }
    }

    #[test]
    fn the_demo_story_survives_postcard() {
        let pack = demo_pack();
        let bytes = pack.to_bytes().unwrap();
        assert!(GamePack::is_pack(&bytes));
        assert_eq!(GamePack::from_bytes(&bytes).unwrap(), pack);
        // Smaller than the RON it comes from.
        assert!(bytes.len() < DEMO.len(), "{} vs {}", bytes.len(), DEMO.len());
    }

    #[test]
    fn a_character_default_position_survives_postcard() {
        let mut pack = demo_pack();
        pack.story.characters[0].default_position = Some(orbit_core::Position::Right);
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back.story.characters[0].default_position, pack.story.characters[0].default_position);
        assert_eq!(back, pack);
    }

    #[test]
    fn bad_packs_are_refused() {
        assert_eq!(GamePack::from_bytes(b"(schema_version: 5)").unwrap_err(), PackError::BadMagic);
        let mut bytes = demo_pack().to_bytes().unwrap();
        assert!(matches!(
            GamePack::from_bytes(&bytes[..20]),
            Err(PackError::Corrupt(_))
        ));
        bytes[8] = 7;
        assert_eq!(
            GamePack::from_bytes(&bytes).unwrap_err(),
            PackError::UnsupportedVersion { found: 7 }
        );
    }
}
