//! The compiled game: story, translations and identity, in postcard.
//!
//! ```text
//! "ORBITGAM"  magic (8 bytes)
//! version     u32 LE  (currently 3)
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
pub const PACK_VERSION: u32 = 3;

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
/// the old format keeps its own struct. Versions 1 and 2 were written with
/// the story of schema v8 and before (no `curves`, no transition easing),
/// read through the frozen mirrors of [`crate::legacy`].
#[derive(Deserialize)]
struct GamePackV1 {
    info: GameInfo,
    story: crate::legacy::StoryV8,
    translations: Vec<Translation>,
}

/// Version 2: version 1 plus the `ui` bundle.
#[derive(Deserialize)]
struct GamePackV2 {
    info: GameInfo,
    story: crate::legacy::StoryV8,
    translations: Vec<Translation>,
    ui: Option<Vec<u8>>,
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
                story: old.story.into(),
                translations: old.translations,
                ui: None,
            });
        }
        if version == 2 {
            let old: GamePackV2 =
                postcard::from_bytes(body).map_err(|e| PackError::Corrupt(e.to_string()))?;
            return Ok(GamePack {
                info: old.info,
                story: old.story.into(),
                translations: old.translations,
                ui: old.ui,
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
    fn scene3d_assets_and_commands_survive_postcard() {
        use orbit_core::{AssetEntry, AssetId, AssetKind, Node, NodeId, NodeKind, Transition};
        let mut pack = fixture_pack();
        // The 2D pack does not change: the new variants are appended.
        let before = pack.to_bytes().unwrap();
        let room = AssetId::new();
        pack.story.assets.assets.push(AssetEntry {
            id: room,
            kind: AssetKind::Scene3d,
            path: "assets/3d/room.scene3d.ron".into(),
        });
        let nodes = &mut pack.story.scenes[0].nodes;
        for kind in [
            NodeKind::ShowScene3d {
                scene: room,
                camera: Some("wide".into()),
                transition: Some(Transition::dissolve(0.5)),
            },
            NodeKind::Camera3d {
                camera: "close".into(),
                transition: None,
            },
            NodeKind::HideScene3d { transition: None },
        ] {
            nodes.insert(
                0,
                Node {
                    id: NodeId::new(),
                    kind,
                },
            );
        }
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
        assert_eq!(fixture_pack().to_bytes().unwrap(), before);
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

    /// Written by the build that still produced pack version 2 (story of
    /// schema v8): `examples/fixtures/compat/pak_v2/game.pack`, never regenerate.
    const PACK_V2: &[u8] = include_bytes!("../../../examples/fixtures/compat/pak_v2/game.pack");

    #[test]
    fn a_version_2_pack_still_loads_with_linear_transitions_and_no_curves() {
        assert_eq!(&PACK_V2[8..12], &2u32.to_le_bytes());
        let back = GamePack::from_bytes(PACK_V2).unwrap();
        assert_eq!(back.info.id, "compat");
        assert!(back.ui.is_none());
        assert!(back.story.curves.is_empty());
        // Same story as the file it was built from, migrated to the current schema.
        assert_eq!(back.story, Story::from_ron_unchecked(FIXTURE).unwrap());
        // And it re-encodes in the current format.
        let again = GamePack::from_bytes(&back.to_bytes().unwrap()).unwrap();
        assert_eq!(again, back);
    }

    #[test]
    fn a_version_1_pack_still_loads_without_ui() {
        // Version 1 is version 2 without the trailing `ui` (a single 0 byte
        // for `None`) and with the old version number.
        let mut bytes = PACK_V2[..PACK_V2.len() - 1].to_vec();
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        let back = GamePack::from_bytes(&bytes).unwrap();
        assert!(back.ui.is_none());
        assert_eq!(back.story, Story::from_ron_unchecked(FIXTURE).unwrap());
    }

    #[test]
    fn named_curves_and_transition_easing_survive_postcard() {
        use orbit_core::{Curve, Ease, NodeKind, Transition};
        let mut pack = fixture_pack();
        pack.story
            .curves
            .insert("snappy".into(), Curve::bezier(0.2, 0.0, 0.1, 1.4));
        pack.story
            .curves
            .insert("bouncy".into(), Curve::Preset(Ease::OutBounce));
        let mut eased = 0;
        for node in pack.story.scenes.iter_mut().flat_map(|s| &mut s.nodes) {
            if let NodeKind::Background { transition, .. } = &mut node.kind {
                *transition = Some(
                    Transition::dissolve(0.5).with_ease(Curve::Named("snappy".into())),
                );
                eased += 1;
            }
        }
        assert!(eased > 0);
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
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
