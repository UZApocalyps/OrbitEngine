//! Building the pak of a compiled game.

use orbit_pak::{
    Compression, ENGINE_PREFIX, GAME_PACK_ENTRY, ICON_ENTRY, PakWriter, WriteStats, crc32,
};
use orbit_project::{ExportProfile, PakCompression};

use crate::compile::Compiled;
use crate::error::{ExportError, Result};
use crate::report::Reporter;

/// The fonts of the engine, read by the player from the pak (`engine/<path>`):
/// the exported game has no engine folder.
const FONTS: [(&str, &[u8]); 3] = [
    (
        "assets/fonts/Inter-Regular.ttf",
        include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    ),
    (
        "assets/fonts/Inter-Bold.ttf",
        include_bytes!("../../../assets/fonts/Inter-Bold.ttf"),
    ),
    (
        "assets/fonts/Inter-Italic.ttf",
        include_bytes!("../../../assets/fonts/Inter-Italic.ttf"),
    ),
];

/// License of the Inter font, shipped with every export.
pub const FONT_LICENSE: &str = include_str!("../../../assets/fonts/OFL.txt");

/// A pak in memory.
/// Pak metadata key of the desktop quality tier (read by
/// `orbit_runtime::compose3d::QUALITY_META`, same value).
pub const QUALITY_META: &str = "quality";

#[derive(Debug, Clone)]
pub struct BuiltPak {
    pub bytes: Vec<u8>,
    pub stats: WriteStats,
    /// CRC-32 of the whole pak (used to version the file names of a web
    /// bundle).
    pub crc: u32,
}

/// Builds the pak: the game pack, the used assets, the engine fonts, the
/// icon and the game identity in the metadata.
pub fn build_pak(
    compiled: &Compiled,
    profile: &ExportProfile,
    rep: &mut Reporter<'_>,
) -> Result<BuiltPak> {
    let compression = match profile.pak_compression {
        PakCompression::None => Compression::None,
        PakCompression::Lz4 => Compression::Lz4,
    };
    let mut writer = PakWriter::new(compression);
    let info = &compiled.pack.info;
    writer.meta("name", info.name.clone());
    writer.meta("id", info.id.clone());
    writer.meta("version", info.version.clone());
    writer.meta("engine", env!("CARGO_PKG_VERSION"));
    if let Some(lang) = &info.default_language {
        writer.meta("default_language", lang.clone());
    }
    // 3D quality of the desktop players (M6-12). A container metadata
    // entry, not a pak format change: older players ignore it, and 2D games
    // do not get it (their pak stays byte-identical).
    let mut content3d = crate::pack3d::Content3d::default();
    if crate::scene3d::uses_3d(&compiled.pack.story) {
        writer.meta(QUALITY_META, profile.desktop_quality.tier_id());
        rep.step(0.0, "Packing the 3D models");
        content3d = crate::pack3d::build(&compiled.root, &compiled.pack.story);
        for warning in &content3d.warnings {
            rep.warn(warning.clone());
        }
    }

    // The lowest game pack version that holds the game (ADR 0014 addendum):
    // 4 only when 3D content ships, so 2D paks stay readable by older players.
    let pack = if content3d.needs_3d_pack {
        compiled.pack.to_bytes_with_3d()?
    } else {
        compiled.pack.to_bytes()?
    };
    writer.add(GAME_PACK_ENTRY, &pack)?;
    for (path, bytes) in &content3d.entries {
        writer.add(path, bytes)?;
    }
    for (path, bytes) in FONTS {
        writer.add(&format!("{ENGINE_PREFIX}/{path}"), bytes)?;
    }
    if let Some(icon) = &compiled.icon {
        writer.add(ICON_ENTRY, icon)?;
        writer.meta("icon", ICON_ENTRY);
    }
    let total = compiled.assets.len().max(1);
    for (i, asset) in compiled.assets.iter().enumerate() {
        rep.check_cancelled()?;
        let bytes = std::fs::read(&asset.source).map_err(ExportError::io(&asset.source))?;
        writer.add(&asset.path, &bytes)?;
        rep.step(
            i as f32 / total as f32,
            &format!(
                "Packing {} ({}/{})",
                asset.path,
                i + 1,
                compiled.assets.len()
            ),
        );
    }
    let (bytes, stats) = writer.to_bytes();
    let crc = crc32(&bytes);
    Ok(BuiltPak { bytes, stats, crc })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile;
    use crate::testing::{TempDir, demo_project};

    // Fixture: exact entry names and count come from test_minimal.ron.
    #[test]
    fn the_fixture_pak_holds_exactly_what_the_game_needs() {
        let dir = TempDir::new("pak_fixture");
        let project = crate::testing::fixture_project(dir.path());
        let mut profile = project.profile(None).unwrap();
        profile.version = "2.0.1".into();
        let compiled = compile(&project, &profile).unwrap();
        let mut rep = Reporter::silent();
        let built = build_pak(&compiled, &profile, &mut rep).unwrap();

        let (_, index) = orbit_pak::container::parse_prefix(&built.bytes).unwrap();
        assert_eq!(index.meta("name"), Some("Test Minimal"));
        assert_eq!(index.meta("version"), Some("2.0.1"));
        for entry in [
            "game.pack",
            "fixtures/assets/bg_room.png",
            "fixtures/assets/music_loop.wav",
            "engine/assets/fonts/Inter-Regular.ttf",
        ] {
            assert!(index.contains(entry), "{entry} is missing");
        }
        // 1 pack + 5 assets + 3 fonts.
        assert_eq!(index.entries().len(), 9);
        // The pack decodes back to the story.
        let pack =
            orbit_pak::GamePack::from_bytes(&index.read_from(&built.bytes, "game.pack").unwrap())
                .unwrap();
        assert_eq!(pack.story.title, "Test Minimal");
        assert_eq!(pack.translations.len(), 1);
        // Compression helps on the wave file and the fonts.
        assert!(built.stats.stored_bytes < built.stats.raw_bytes);
    }

    // Demo: global property only (the pak holds the pack, every compiled asset and the
    // engine fonts); no exact names or counts.
    #[test]
    fn the_demo_pak_holds_what_the_game_needs() {
        let dir = TempDir::new("pak_demo");
        let project = demo_project(dir.path());
        let profile = project.profile(None).unwrap();
        let compiled = compile(&project, &profile).unwrap();
        let built = build_pak(&compiled, &profile, &mut Reporter::silent()).unwrap();
        let (_, index) = orbit_pak::container::parse_prefix(&built.bytes).unwrap();
        assert!(index.contains("game.pack"));
        assert!(index.contains("engine/assets/fonts/Inter-Regular.ttf"));
        for asset in &compiled.assets {
            assert!(index.contains(&asset.path), "{} is missing", asset.path);
        }
    }

    #[test]
    fn no_compression_stores_everything_raw() {
        let dir = TempDir::new("pak_raw");
        let project = demo_project(dir.path());
        let mut profile = project.profile(None).unwrap();
        profile.pak_compression = PakCompression::None;
        let compiled = compile(&project, &profile).unwrap();
        let built = build_pak(&compiled, &profile, &mut Reporter::silent()).unwrap();
        assert_eq!(built.stats.stored_bytes, built.stats.raw_bytes);
    }
}
