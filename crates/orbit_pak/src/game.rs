//! The compiled game: story, translations and identity, in postcard.
//!
//! ```text
//! "ORBITGAM"  magic (8 bytes)
//! version     u32 LE  (3; 4 for a game with 3D content, 5 for a rigged one, see below)
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
/// Newest game pack version this build reads and writes.
///
/// Version 4 has the layout of version 3; it only tells players that the
/// pak holds the 3D content of ADR 0014 (addendum "Blobs 3D dans le pak":
/// `3d/` entries, procedural shapes), so that a player predating it
/// refuses the game cleanly instead of showing empty 3D scenes. The writer
/// picks the lowest version that can hold the game ([`GamePack::to_bytes`]
/// writes 3, [`GamePack::to_bytes_with_3d`] writes 4): a 2D game stays
/// readable by older players.
///
/// Version 5 (schema v14) changes the layout of `Motion::Rigged`, which now
/// has a payload (rig, skin, clip, loop mode), and adds the `Rig` asset
/// kind. It is written only for a game with a rigged expression
/// ([`GamePack::to_bytes`] and [`GamePack::to_bytes_with_3d`] pick it by
/// themselves); versions 1 to 4 are read with the layout of their time
/// (`orbit_core::with_legacy_motion_layout`: the old unit `Rigged`, which
/// never had an effect, reads as a still image).
///
/// Version 6 (schema v16) adds the voice of a `Say`, the achievements of the
/// story, the `Unlock` node and the voices of a translation. It is written
/// only for a game that uses one of them ([`GamePack::uses_v16`]): any other
/// game is encoded in the layout of versions 3 to 5, byte for byte as before
/// (`orbit_core::with_legacy_v15_layout`), so older players keep reading it.
/// Versions 1 to 5 are read with that layout.
pub const PACK_VERSION: u32 = 6;
/// Version written for a game with a rigged expression (and nothing newer).
pub const PACK_VERSION_RIGGED: u32 = 5;
/// Version written for a game without pak-level 3D content.
pub const BASE_PACK_VERSION: u32 = 3;
/// Version written for a game with pak-level 3D content.
pub const PACK_VERSION_3D: u32 = 4;

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
    /// Encodes a game without pak-level 3D content (version 3, readable by
    /// every player since version 3).
    pub fn to_bytes(&self) -> Result<Vec<u8>, PackError> {
        self.encode(BASE_PACK_VERSION)
    }

    /// Encodes a game whose pak holds 3D content (`3d/` entries): version 4.
    pub fn to_bytes_with_3d(&self) -> Result<Vec<u8>, PackError> {
        self.encode(PACK_VERSION_3D)
    }

    /// The version number of an encoded pack (`None`: not a pack).
    pub fn version_of(bytes: &[u8]) -> Option<u32> {
        if !Self::is_pack(bytes) {
            return None;
        }
        bytes
            .get(8..12)
            .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
    }

    /// Whether an expression of the game plays a rig clip (such a game needs
    /// pack version 5).
    pub fn uses_rigs(&self) -> bool {
        self.story
            .characters
            .iter()
            .flat_map(|c| &c.expressions)
            .any(|e| e.rigged().is_some())
    }

    /// Whether the game needs version 6 (schema v16): a voiced line, an
    /// achievement, an `Unlock` node or a translated voice.
    pub fn uses_v16(&self) -> bool {
        !self.story.achievements.is_empty()
            || self.translations.iter().any(|t| !t.voices.is_empty())
            || self.story.scenes.iter().flat_map(|s| &s.nodes).any(|n| {
                matches!(
                    n.kind,
                    orbit_core::NodeKind::Unlock { .. }
                        | orbit_core::NodeKind::Say { voice: Some(_), .. }
                )
            })
    }

    fn encode(&self, version: u32) -> Result<Vec<u8>, PackError> {
        let v16 = self.uses_v16();
        let version = if v16 {
            PACK_VERSION
        } else if self.uses_rigs() {
            PACK_VERSION_RIGGED
        } else {
            version
        };
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&version.to_le_bytes());
        let encode = || postcard::to_stdvec(self).map_err(|e| PackError::Corrupt(e.to_string()));
        let body = if v16 {
            encode()?
        } else {
            orbit_core::with_legacy_v15_layout(encode)?
        };
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
        let decode = || {
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
            // Versions 3 and 4 share the layout of version 5 except for the
            // rigged motion (handled by the flag above).
            postcard::from_bytes(body).map_err(|e| PackError::Corrupt(e.to_string()))
        };
        // `Motion::Rigged` had no payload before version 5; `Say`, `Story` and
        // translations had no v16 fields before version 6.
        let decode = || {
            if version <= 4 {
                orbit_core::with_legacy_motion_layout(decode)
            } else {
                decode()
            }
        };
        if version <= 5 {
            orbit_core::with_legacy_v15_layout(decode)
        } else {
            decode()
        }
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
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
        assert_eq!(GamePack::version_of(&pack.to_bytes().unwrap()), Some(3));
    }

    fn rigged_pack() -> GamePack {
        use orbit_core::{AssetEntry, AssetId, AssetKind, LoopMode, Motion, RiggedMotion};
        let mut pack = fixture_pack();
        let rig = AssetId::new();
        pack.story.assets.assets.push(AssetEntry {
            id: rig,
            kind: AssetKind::Rig,
            path: "assets/rigs/hero.rig.ron".into(),
        });
        pack.story.characters[0].expressions[1].motion = Some(Motion::Rigged(RiggedMotion {
            rig,
            skin: Some("outfit".into()),
            clip: "wave".into(),
            loop_mode: LoopMode::Once,
        }));
        pack
    }

    #[test]
    fn a_rigged_game_is_a_version_5_pack_and_survives_postcard() {
        let pack = rigged_pack();
        assert!(pack.uses_rigs());
        for bytes in [pack.to_bytes().unwrap(), pack.to_bytes_with_3d().unwrap()] {
            assert_eq!(GamePack::version_of(&bytes), Some(5));
            assert_eq!(GamePack::from_bytes(&bytes).unwrap(), pack);
        }
        // Without a rigged expression the lowest version is still written.
        assert!(!fixture_pack().uses_rigs());
        assert_eq!(
            GamePack::version_of(&fixture_pack().to_bytes_with_3d().unwrap()),
            Some(4)
        );
    }

    /// Written before schema v14 (pack version 3), when `Motion::Rigged`
    /// was a unit variant: `examples/fixtures/compat/pak_v3_rigged/game.pack`,
    /// never regenerate (see `generate_pak_v3_rigged`, kept for the record).
    const PACK_V3_RIGGED: &[u8] =
        include_bytes!("../../../examples/fixtures/compat/pak_v3_rigged/game.pack");

    #[test]
    fn a_frozen_version_3_pack_with_the_unit_rigged_motion_loads_as_a_still() {
        assert_eq!(&PACK_V3_RIGGED[8..12], &3u32.to_le_bytes());
        let back = GamePack::from_bytes(PACK_V3_RIGGED).unwrap();
        assert_eq!(back.story.characters[0].expressions[1].motion, None);
        assert_eq!(back.story.characters[0].expressions[0].motion, None);
        assert!(!back.uses_rigs());
        // It re-encodes to a pack of the same version, with the same story.
        let again = GamePack::from_bytes(&back.to_bytes().unwrap()).unwrap();
        assert_eq!(again, back);
    }

    /// Builds the frozen fixture above from the current encoder: the byte of
    /// `motion: None` of expression 1 is replaced by `Some(Rigged)` as the
    /// old layout wrote it (`Some`, variant 1, no payload).
    /// Fixture generators run only with `ORBIT_REGENERATE_FIXTURES=1` and refuse
    /// to overwrite an existing file unless it is `force`.
    fn regen_allowed(out: &std::path::Path) -> bool {
        let mode = std::env::var("ORBIT_REGENERATE_FIXTURES").unwrap_or_default();
        if mode != "1" && mode != "force" {
            eprintln!("skipped: set ORBIT_REGENERATE_FIXTURES=1 to regenerate the frozen fixtures");
            return false;
        }
        if out.exists() && mode != "force" {
            eprintln!("skipped: {} exists; ORBIT_REGENERATE_FIXTURES=force overwrites it", out.display());
            return false;
        }
        true
    }

    #[test]
    #[ignore = "writes a fixture; run once with --ignored"]
    fn generate_pak_v3_rigged() {
        if !regen_allowed(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/fixtures/compat/pak_v3_rigged/game.pack")) {
            return;
        }
        use orbit_core::{AnimFrames, Animation, LoopMode, Motion};
        let plain = fixture_pack();
        let mut marked = plain.clone();
        marked.story.characters[0].expressions[1].motion = Some(Motion::Animation(Animation {
            frames: AnimFrames::Sequence(vec![]),
            fps: 1.0,
            loop_mode: LoopMode::Loop,
        }));
        let a = plain.to_bytes().unwrap();
        let b = marked.to_bytes().unwrap();
        let at = a.iter().zip(&b).position(|(x, y)| x != y).unwrap();
        assert_eq!((a[at], b[at]), (0, 1), "Option tag None / Some");
        let tail = &b[at..];
        // `Some(Animation(Sequence([]), 1.0, Loop))`: tag, variant 0, kind 1,
        // len 0, f32 1.0, LoopMode 0.
        assert_eq!(tail[..4], [1, 0, 1, 0]);
        let mut out = a[..at].to_vec();
        out.extend_from_slice(&[1, 1]);
        out.extend_from_slice(&a[at + 1..]);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/fixtures/compat/pak_v3_rigged");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("game.pack"), out).unwrap();
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
    fn effect_and_timeline_nodes_survive_postcard() {
        use orbit_core::{
            AnimProp, AnimTarget, Color, Curve, Effect, Ease, Key, Loop, Node, NodeId, NodeKind,
            StoryTrack, Track, TrackData, Vec2,
        };
        let mut pack = fixture_pack();
        let effects = [
            Effect::Move { target: AnimTarget::Background, dx: 0.1, dy: -0.2, duration: 1.0, ease: Some(Curve::Preset(Ease::OutBack)), wait: true },
            Effect::Shake { target: AnimTarget::Camera, amplitude: 0.02, duration: 0.5, ease: None, wait: false },
            Effect::Zoom { factor: 1.5, duration: 1.0, ease: Some(Curve::Named("snappy".into())), wait: false },
            Effect::Flash { color: Color::WHITE, duration: 0.3, ease: None, wait: false },
            Effect::Tint { target: AnimTarget::Screen, color: Color::rgb(255, 0, 0), strength: 0.4, duration: 2.0, ease: None, wait: true },
        ];
        let scene = &mut pack.story.scenes[0].nodes;
        for e in effects {
            scene.insert(0, Node { id: NodeId::new(), kind: NodeKind::Effect(e) });
        }
        scene.insert(
            0,
            Node {
                id: NodeId::new(),
                kind: NodeKind::Timeline {
                    duration: 3.0,
                    looping: Loop::Loop(2),
                    wait: false,
                    tracks: vec![StoryTrack {
                        target: AnimTarget::Background,
                        prop: AnimProp::Offset,
                        data: TrackData::Vec2(Track::new(vec![
                            Key::new(0.0, Vec2::new(0.0, 0.0)),
                            Key::with_ease(1.5, Vec2::new(0.3, 0.0), Curve::Preset(Ease::InOutQuad)),
                            Key::new(3.0, Vec2::new(0.0, 0.0)),
                        ])),
                    }],
                },
            },
        );
        pack.story
            .curves
            .insert("snappy".into(), Curve::bezier(0.2, 0.0, 0.1, 1.4));
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
    }

    /// Written by the build of pack version 3 / schema v10, before lists and
    /// maps existed: `examples/fixtures/compat/pak_v3/game.pack`, never
    /// regenerate. Collections (schema v11) only append enum variants, so
    /// the version-3 layout is unchanged and the pack version stays 3.
    const PACK_V3: &[u8] = include_bytes!("../../../examples/fixtures/compat/pak_v3/game.pack");

    #[test]
    fn a_frozen_version_3_pack_still_loads() {
        assert_eq!(&PACK_V3[8..12], &3u32.to_le_bytes());
        let back = GamePack::from_bytes(PACK_V3).unwrap();
        assert_eq!(back.info.id, "compat");
        assert_eq!(back.ui.as_deref(), Some(&b"(screens: [])"[..]));
        let mut expected = Story::from_ron_unchecked(FIXTURE).unwrap();
        // The pack keeps the schema number it was built with.
        expected.schema_version = back.story.schema_version;
        assert_eq!(back.story, expected);
        assert!(orbit_core::validate(&back.story).errors.is_empty());
        let again = GamePack::from_bytes(&back.to_bytes().unwrap()).unwrap();
        assert_eq!(again, back);
    }

    #[test]
    fn list_and_map_variables_survive_postcard() {
        use orbit_core::{Expr, Node, NodeId, NodeKind, ScalarType, Value, VarDecl};
        let mut pack = fixture_pack();
        // A story without collections encodes exactly as before.
        let before = pack.to_bytes().unwrap();
        assert_eq!(GamePack::from_bytes(&before).unwrap(), pack);
        pack.story.variables.push(VarDecl::game(
            "bag",
            Value::list(ScalarType::Str, [Value::from("sword")]),
        ));
        pack.story.variables.push(VarDecl::persistent(
            "endings",
            Value::empty_map(ScalarType::Bool),
        ));
        for src in [
            "push(bag, \"key\")",
            "insert(endings, \"good\", contains(bag, \"key\") && len(bag) > 1)",
            "[bag[0], \"x\"]",
            "#{\"a\": true}",
            "[:float]",
        ] {
            let var = if src.contains("endings") || src.starts_with('#') { "endings" } else { "bag" };
            pack.story.scenes[0].nodes.insert(
                0,
                Node {
                    id: NodeId::new(),
                    kind: NodeKind::SetVar {
                        var: var.into(),
                        value: Expr::parse(src).unwrap(),
                    },
                },
            );
        }
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
    }

    #[test]
    fn screen_nodes_survive_postcard_without_a_version_change() {
        // UI-14 (schema v12): `OpenScreen` / `CloseScreen` and the contextual
        // operands are appended variants, the pack stays version 3.
        use orbit_core::{Node, NodeId, NodeKind, Operand, ScreenParam, Value};
        let mut pack = fixture_pack();
        let before = pack.to_bytes().unwrap();
        assert_eq!(&before[8..12], &BASE_PACK_VERSION.to_le_bytes());
        for kind in [
            NodeKind::OpenScreen {
                screen: "phone".into(),
                wait: true,
                result: Some("called".into()),
                params: vec![
                    ScreenParam {
                        name: "contact".into(),
                        value: Operand::Lit(Value::from("Aiko")),
                    },
                    ScreenParam {
                        name: "gold".into(),
                        value: Operand::Var("gold".into()),
                    },
                ],
            },
            NodeKind::CloseScreen {
                screen: "hud".into(),
            },
        ] {
            pack.story.scenes[0].nodes.insert(
                0,
                Node {
                    id: NodeId::new(),
                    kind,
                },
            );
        }
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
    }

    /// A 2D game keeps writing version 3 (older players read it); a game
    /// with 3D content writes version 4 with the same body, which this
    /// build reads and a version-3 player refuses by number.
    #[test]
    fn the_lowest_version_is_written_and_both_are_read() {
        let pack = fixture_pack();
        let v3 = pack.to_bytes().unwrap();
        let v4 = pack.to_bytes_with_3d().unwrap();
        assert_eq!(GamePack::version_of(&v3), Some(3));
        assert_eq!(GamePack::version_of(&v4), Some(4));
        assert_eq!(v3[12..], v4[12..]);
        assert_eq!(GamePack::from_bytes(&v3).unwrap(), pack);
        assert_eq!(GamePack::from_bytes(&v4).unwrap(), pack);
        assert_eq!(GamePack::version_of(b"(story)"), None);
    }

    #[test]
    fn script_nodes_and_rhai_conditions_survive_postcard_without_a_version_change() {
        // M8-01 (schema v15): `NodeKind::Script` and `Expr::Rhai` are
        // appended variants, and module sources travel as plain pak entries
        // (`scripts/<name>.rhai`): the game pack layout does not change, so
        // a pack written before them still reads and the version stays.
        use orbit_core::{Expr, Node, NodeId, NodeKind, ScriptBranch};
        let mut pack = fixture_pack();
        let before = pack.to_bytes().unwrap();
        assert_eq!(&before[8..12], &BASE_PACK_VERSION.to_le_bytes());
        let label = pack.story.scenes[0].nodes[0].id;
        for kind in [
            NodeKind::Script {
                source: "import \"dice\" as d;\nd::roll()".into(),
                branches: vec![ScriptBranch {
                    name: "win".into(),
                    target: label,
                }],
                on_error: Some(label),
            },
            NodeKind::If {
                condition: Expr::Rhai("get_var(\"gold\") > 2".into()),
                then_target: label,
                else_target: None,
            },
        ] {
            pack.story.scenes[0].nodes.push(Node { id: NodeId::new(), kind });
        }
        let bytes = pack.to_bytes().unwrap();
        assert_eq!(&bytes[8..12], &BASE_PACK_VERSION.to_le_bytes());
        assert_eq!(GamePack::from_bytes(&bytes).unwrap(), pack);
        assert!(GamePack::from_bytes(&before).is_ok());
    }

    #[test]
    fn layer_and_particle_nodes_survive_postcard_without_a_version_change() {
        // M5A-07 / M5A-08 (schema v13): `BackgroundLayers`, `Particles` and
        // `StopParticles` are appended variants, so a pack written before
        // them still reads and the pack stays version 3.
        use orbit_core::{AssetId, BgLayer, Emitter, Node, NodeId, NodeKind, ParticlePreset};
        let mut pack = fixture_pack();
        let before = pack.to_bytes().unwrap();
        assert_eq!(&before[8..12], &BASE_PACK_VERSION.to_le_bytes());
        let image = AssetId::new();
        let mut custom = Emitter::preset(ParticlePreset::Custom);
        custom.sprite = Some(image);
        custom.seed = 7;
        for kind in [
            NodeKind::BackgroundLayers {
                layers: vec![
                    BgLayer::new(image, 0.0),
                    BgLayer { asset: image, depth: -0.5, repeat_x: true },
                ],
                transition: None,
            },
            NodeKind::Particles(Emitter::preset(ParticlePreset::Rain)),
            NodeKind::Particles(custom),
            NodeKind::StopParticles { name: "rain".into() },
        ] {
            pack.story.scenes[0].nodes.insert(0, Node { id: NodeId::new(), kind });
        }
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back, pack);
        // The pack from before still decodes with the current reader.
        assert!(GamePack::from_bytes(&before).is_ok());
    }

    /// Written by the build of schema v15 (pack version 5: a rigged
    /// expression and a `Script` node): `examples/fixtures/compat/pak_v5/game.pack`,
    /// never regenerate.
    const PACK_V5: &[u8] = include_bytes!("../../../examples/fixtures/compat/pak_v5/game.pack");

    #[test]
    fn a_frozen_version_5_pack_loads_in_the_v15_layout_and_reencodes_identically() {
        assert_eq!(&PACK_V5[8..12], &5u32.to_le_bytes());
        let back = GamePack::from_bytes(PACK_V5).unwrap();
        assert_eq!(back.info.id, "compat");
        assert!(back.uses_rigs());
        assert!(back.story.achievements.is_empty());
        assert!(!back.uses_v16());
        assert!(
            back.story
                .scenes
                .iter()
                .flat_map(|s| &s.nodes)
                .any(|n| matches!(n.kind, orbit_core::NodeKind::Script { .. }))
        );
        // Nothing of schema v16 is used: the same bytes come out, readable
        // by a player of schema v15.
        assert_eq!(back.to_bytes().unwrap(), PACK_V5);
    }

    #[test]
    fn voices_and_achievements_need_version_6_and_survive_postcard() {
        use orbit_core::{
            Achievement, AchievementId, AssetEntry, AssetId, AssetKind, Node, NodeId, NodeKind,
        };
        let mut pack = fixture_pack();
        let plain = pack.to_bytes().unwrap();
        assert_eq!(GamePack::version_of(&plain), Some(3));
        let voice = AssetId::new();
        pack.story.assets.assets.push(AssetEntry {
            id: voice,
            kind: AssetKind::Sound,
            path: "voice/line.ogg".into(),
        });
        for node in pack.story.scenes.iter_mut().flat_map(|s| &mut s.nodes) {
            if let NodeKind::Say { voice: v, .. } = &mut node.kind {
                *v = Some(voice);
                break;
            }
        }
        assert!(pack.uses_v16());
        let bytes = pack.to_bytes().unwrap();
        assert_eq!(GamePack::version_of(&bytes), Some(6));
        assert_eq!(GamePack::from_bytes(&bytes).unwrap(), pack);
        assert_eq!(
            GamePack::version_of(&pack.to_bytes_with_3d().unwrap()),
            Some(6)
        );

        let mut pack = fixture_pack();
        let achievement = AchievementId::new();
        let (title_id, description_id) = (
            pack.story.strings.add("First step"),
            pack.story.strings.add("Take it."),
        );
        pack.story.achievements.push(Achievement {
            id: achievement,
            title_id,
            description_id,
            hidden: true,
            icon: None,
        });
        pack.story.scenes[0].nodes.insert(
            0,
            Node {
                id: NodeId::new(),
                kind: NodeKind::Unlock { achievement },
            },
        );
        assert_eq!(GamePack::version_of(&pack.to_bytes().unwrap()), Some(6));
        assert_eq!(GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap(), pack);

        let mut pack = fixture_pack();
        pack.translations[0].voices.insert(title_id, voice);
        assert!(pack.uses_v16());
        let back = GamePack::from_bytes(&pack.to_bytes().unwrap()).unwrap();
        assert_eq!(back.translations[0].voices.get(&title_id), Some(&voice));
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
