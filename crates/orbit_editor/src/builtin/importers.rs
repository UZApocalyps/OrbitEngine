//! The built-in asset importers (PNG / JPEG images, WAV / OGG audio),
//! registered through `orbit_plugin_api` like any plugin's importers. They
//! **decode** the file: a text file renamed `.png`, a truncated audio file
//! or an absurdly large image is refused with a readable reason instead of
//! failing later in the player.

use orbit_core::AssetKind;
use orbit_plugin_api::{AssetImporter, ImportedAsset, Plugin, PluginManifest, ProjectRegistry};

use super::ids;
use crate::assets::refusal::{
    ERR_AUDIO_UNREADABLE, ERR_EMPTY_AUDIO, ERR_EMPTY_IMAGE, ERR_FORMAT, ERR_IMAGE_UNREADABLE,
    ERR_NOT_IMAGE, ERR_TOO_LARGE,
};

pub struct ImportersPlugin;

/// Longest side accepted for an image (a GPU texture limit with margin).
const MAX_IMAGE_SIDE: u32 = 16_384;
/// Audio longer than this (seconds) is imported as music, shorter as a sound.
pub const MUSIC_MIN_SECONDS: f32 = 20.0;

/// Validates a PNG / JPEG by decoding it.
pub fn import_image(bytes: &[u8]) -> Result<ImportedAsset, String> {
    let format = image::guess_format(bytes).map_err(|_| ERR_NOT_IMAGE.to_owned())?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err(format!("{}{format:?}{}", ERR_FORMAT.0, ERR_FORMAT.1));
    }
    let img = image::load_from_memory_with_format(bytes, format)
        .map_err(|e| format!("{}{e}{}", ERR_IMAGE_UNREADABLE.0, ERR_IMAGE_UNREADABLE.1))?;
    if img.width() == 0 || img.height() == 0 {
        return Err(ERR_EMPTY_IMAGE.to_owned());
    }
    if img.width() > MAX_IMAGE_SIDE || img.height() > MAX_IMAGE_SIDE {
        return Err(format!(
            "{}{}×{} px, {MAX_IMAGE_SIDE}{}",
            ERR_TOO_LARGE.0,
            img.width(),
            img.height(),
            ERR_TOO_LARGE.1
        ));
    }
    Ok(ImportedAsset {
        kind: AssetKind::Image,
        bytes: bytes.to_vec(),
    })
}

/// Validates a WAV / OGG Vorbis file by decoding it. Long files become
/// music, short ones sounds.
pub fn import_audio(bytes: &[u8]) -> Result<ImportedAsset, String> {
    let pcm = orbit_assets::decode_audio("", bytes)
        .map_err(|e| format!("{}{e}{}", ERR_AUDIO_UNREADABLE.0, ERR_AUDIO_UNREADABLE.1))?;
    if pcm.frames() == 0 {
        return Err(ERR_EMPTY_AUDIO.to_owned());
    }
    let kind = if pcm.duration_secs() >= MUSIC_MIN_SECONDS {
        AssetKind::Music
    } else {
        AssetKind::Sound
    };
    Ok(ImportedAsset {
        kind,
        bytes: bytes.to_vec(),
    })
}

impl Plugin for ImportersPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new(ids::IMPORTERS, "Importeurs de ressources", "0.1.0")
    }

    fn register(&self, registry: &mut ProjectRegistry) {
        registry.register_importer(AssetImporter {
            label: "Image PNG / JPEG".to_owned(),
            extensions: vec!["png".to_owned(), "jpg".to_owned(), "jpeg".to_owned()],
            import: import_image,
        });
        registry.register_importer(AssetImporter {
            label: "Audio WAV / OGG".to_owned(),
            extensions: vec!["wav".to_owned(), "ogg".to_owned()],
            import: import_audio,
        });
    }
}

impl orbit_editor_api::EditorPlugin for ImportersPlugin {
    fn register_ui(&self, _registry: &mut orbit_editor_api::EditorRegistry) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn wav(seconds: f32) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut out = Vec::new();
        {
            let mut w = hound::WavWriter::new(std::io::Cursor::new(&mut out), spec).unwrap();
            for i in 0..(8000.0 * seconds) as usize {
                w.write_sample(((i % 100) as i16 - 50) * 100).unwrap();
            }
            w.finalize().unwrap();
        }
        out
    }

    #[test]
    fn images_are_decoded_and_garbage_is_refused() {
        assert_eq!(import_image(&png(4, 3)).unwrap().kind, AssetKind::Image);
        assert!(import_image(b"hello, I am a text file").is_err());
        // A truncated PNG fails to decode.
        let full = png(64, 64);
        assert!(import_image(&full[..full.len() / 2]).is_err());
    }

    #[test]
    fn audio_kind_follows_the_duration() {
        assert_eq!(import_audio(&wav(0.5)).unwrap().kind, AssetKind::Sound);
        assert_eq!(import_audio(&wav(25.0)).unwrap().kind, AssetKind::Music);
        assert!(import_audio(b"RIFFxxxxnot really a wav").is_err());
        assert!(import_audio(&[]).is_err());
    }

    #[test]
    fn refusals_are_shown_in_the_editor_language() {
        use orbit_editor_api::Lang;
        let fr = crate::i18n::translator(Lang::Fr);
        let en = crate::i18n::translator(Lang::En);
        let reason = import_image(b"hello, I am a text file").unwrap_err();
        assert_eq!(
            crate::assets::refusal::localize(&fr, &reason),
            "ce n'est pas une image PNG ou JPEG"
        );
        assert_eq!(
            crate::assets::refusal::localize(&en, &reason),
            "not a PNG or JPEG image"
        );
        let reason = import_audio(&[]).unwrap_err();
        assert!(crate::assets::refusal::localize(&fr, &reason).starts_with("audio illisible ("));
        let big = format!(
            "{}20000×5 px, {MAX_IMAGE_SIDE}{}",
            ERR_TOO_LARGE.0, ERR_TOO_LARGE.1
        );
        assert_eq!(
            crate::assets::refusal::localize(&fr, &big),
            "image trop grande (20000×5 px, 16384 px au plus)"
        );
        // A plugin's own reason is left alone.
        assert_eq!(
            crate::assets::refusal::localize(&fr, "whatever"),
            "whatever"
        );
    }
}
