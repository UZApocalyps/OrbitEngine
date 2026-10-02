//! The compiled game: story, translations and identity, in postcard.
//!
//! ```text
//! "ORBITGAM"  magic (8 bytes)
//! version     u32 LE  (currently 2)
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
pub const PACK_VERSION: u32 = 2;

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
    /// The UI of the game (screens, theme, splashes) as the RON text of an
    /// `orbit_ui::UiBundle`, kept opaque here so that this crate needs no
    /// rendering code. `None`: the stock screens. Added in version 2.
    pub ui: Option<Vec<u8>>,
}

/// The layout of version 1 (no `ui`): postcard is not self-describing, so
/// the old format keeps its own struct.
#[derive(Deserialize)]
struct GamePackV1 {
    info: GameInfo,
    story: Story,
    translations: Vec<Translation>,
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
        let body = &bytes[12..];
        if version == 1 {
            let old: GamePackV1 =
                postcard::from_bytes(body).map_err(|e| PackError::Corrupt(e.to_string()))?;
            return Ok(GamePack {
                info: old.info,
                story: old.story,
                translations: old.translations,
                ui: None,
            });
        }
        postcard::from_bytes(body).map_err(|e| PackError::Corrupt(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixture: these tests index characters/expressions and compare sizes (test_features.ron).
    const FIXTURE: &str = include_str!("../../../examples/stories/test_features.ron");
    const FR: &str = include_str!("../../../examples/stories/i18n/test_minimal.fr.ron");

    fn fixture_pack() -> GamePack {
        GamePack {
            info: GameInfo {
                name: "Test Features".into(),
                id: "demo".into(),
                version: "1.2.3".into(),
                default_language: Some("fr".into()),
            },
            story: Story::from_ron_unchecked(FIXTURE).unwrap(),
            translations: vec![Translation::from_ron(FR).unwrap()],
            ui: None,
        }
    }

    #[test]
    fn the_fixture_story_survives_postcard() {
        let pack = fixture_pack();
        let bytes = pack.to_bytes().unwrap();
        assert!(GamePack::is_pack(&bytes));
        assert_eq!(GamePack::from_bytes(&bytes).unwrap(), pack);
        // Smaller than the RON it comes from.
        assert!(
            bytes.len() < FIXTURE.len(),
            "{} vs {}",
            bytes.len(),
            FIXTURE.len()
        );
    }

    #[test]
    fn a_character_default_position_survives_postcard() {
        let mut pack = fixture_pack();
        pack.story.characters[0].default_position = Some(orbit_core::Position::Right);
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(
            back.story.characters[0].default_position,
            pack.story.characters[0].default_position
        );
        assert_eq!(back, pack);
    }

    #[test]
    fn scale_mirror_and_free_positions_survive_postcard() {
        let mut pack = fixture_pack();
        pack.story.characters[0].default_scale = 0.75;
        let mut styled = 0;
        for node in pack.story.scenes.iter_mut().flat_map(|s| &mut s.nodes) {
            if let orbit_core::NodeKind::Show {
                position,
                scale,
                scale_y,
                flip_x,
                ..
            } = &mut node.kind
            {
                if styled % 2 == 0 {
                    *position = orbit_core::Position::Free { x: 0.4, y: 0.9 };
                    *scale = Some(0.5);
                    *scale_y = Some(1.5);
                    *flip_x = true;
                }
                styled += 1;
            }
        }
        assert!(styled > 1);
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
    }

    #[test]
    fn expression_animations_survive_postcard() {
        use orbit_core::{AnimFrames, Animation, LoopMode, Motion};
        let mut pack = fixture_pack();
        let image = pack.story.characters[0].expressions[0].image;
        pack.story.characters[0].expressions[0].motion = Some(Motion::Animation(Animation {
            frames: AnimFrames::SpriteSheet {
                asset: image,
                cols: 3,
                rows: 2,
                frame_count: 5,
                frame_size: Some([64, 96]),
            },
            fps: 10.0,
            loop_mode: LoopMode::PingPong,
        }));
        pack.story.characters[0].expressions[1].motion = Some(Motion::Rigged);
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
    }

    #[test]
    fn the_ui_bundle_survives_postcard() {
        let mut pack = fixture_pack();
        pack.ui = Some(b"(screens: [])".to_vec());
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back.ui, pack.ui);
    }

    #[test]
    fn a_version_1_pack_still_loads_without_ui() {
        let pack = fixture_pack();
        let v1 = GamePackV1Out {
            info: &pack.info,
            story: &pack.story,
            translations: &pack.translations,
        };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&postcard::to_stdvec(&v1).unwrap());
        let back = GamePack::from_bytes(&bytes).unwrap();
        assert_eq!(back, pack);
        assert!(back.ui.is_none());
    }

    #[derive(Serialize)]
    struct GamePackV1Out<'a> {
        info: &'a GameInfo,
        story: &'a Story,
        translations: &'a Vec<Translation>,
    }

    #[test]
    fn bad_packs_are_refused() {
        assert_eq!(
            GamePack::from_bytes(b"(schema_version: 5)").unwrap_err(),
            PackError::BadMagic
        );
        let mut bytes = fixture_pack().to_bytes().unwrap();
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
