//! Loading of the story file and of the assets its manifest declares.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use orbit_assets::{AssetHandle, AssetPoll, ImageData, PcmAudio};
use orbit_audio::Sound;
use orbit_core::{AssetKind, Story, validate};
use orbit_render2d::{Renderer2d, TextureId, TextureOptions};

/// A parsed and validated story.
#[derive(Debug)]
pub struct LoadedStory {
    pub story: Arc<Story>,
    /// Non-fatal validation warnings (already logged).
    pub warnings: Vec<String>,
}

/// Parses and validates story bytes. Every problem becomes a readable error
/// message (never a panic): RON syntax errors, unsupported schema versions
/// and the full list of validation errors.
pub fn parse_story(bytes: Vec<u8>) -> Result<LoadedStory, String> {
    let text = String::from_utf8(bytes).map_err(|e| format!("the file is not valid UTF-8: {e}"))?;
    let story = Story::from_ron_unchecked(&text).map_err(|e| e.to_string())?;
    let report = validate(&story);
    if !report.errors.is_empty() {
        let mut message = format!("{} validation error(s):", report.errors.len());
        for error in &report.errors {
            message.push_str(&format!("\n- {error}"));
        }
        return Err(message);
    }
    let warnings: Vec<String> = report.warnings.iter().map(|w| w.to_string()).collect();
    for warning in &warnings {
        log::warn!("story warning: {warning}");
    }
    Ok(LoadedStory {
        story: Arc::new(story),
        warnings,
    })
}

/// GPU textures and decoded sounds, by manifest path.
#[derive(Default)]
pub struct AssetStore {
    textures: HashMap<String, TextureId>,
    sounds: HashMap<String, Sound>,
}

impl AssetStore {
    pub fn texture(&self, path: &str) -> Option<TextureId> {
        self.textures.get(path).copied()
    }

    pub fn sound(&self, path: &str) -> Option<&Sound> {
        self.sounds.get(path)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.textures.contains_key(path) || self.sounds.contains_key(path)
    }

    /// Drops the asset at `path` (a file that changed on disk): the next
    /// loading pass fetches it again. The texture, if any, is released.
    pub fn forget(&mut self, renderer: &mut Renderer2d, path: &str) -> bool {
        let texture = self.textures.remove(path);
        if let Some(texture) = texture {
            renderer.release_texture(texture);
        }
        let sound = self.sounds.remove(path);
        texture.is_some() || sound.is_some()
    }

    /// Drops every asset (the asset root changed).
    pub fn clear(&mut self, renderer: &mut Renderer2d) {
        for (_, texture) in self.textures.drain() {
            renderer.release_texture(texture);
        }
        self.sounds.clear();
    }
}

enum Handle {
    Image(AssetHandle<ImageData>),
    Audio(AssetHandle<PcmAudio>),
}

/// One manifest asset being loaded.
pub struct PendingAsset {
    path: String,
    handle: Handle,
}

impl PendingAsset {
    pub fn is_done(&self) -> bool {
        match &self.handle {
            Handle::Image(h) => !matches!(h.poll(), AssetPoll::Loading),
            Handle::Audio(h) => !matches!(h.poll(), AssetPoll::Loading),
        }
    }
}

/// Starts loading every manifest asset that is not in `store` yet (each path
/// once).
pub fn start_loading(
    loader: &orbit_assets::AssetLoader,
    story: &Story,
    store: &AssetStore,
) -> Vec<PendingAsset> {
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    for entry in &story.assets.assets {
        if store.contains(&entry.path) || !seen.insert(entry.path.clone()) {
            continue;
        }
        let handle = match entry.kind {
            AssetKind::Image => Handle::Image(loader.load_image(&entry.path)),
            AssetKind::Music | AssetKind::Sound => Handle::Audio(loader.load_audio(&entry.path)),
        };
        pending.push(PendingAsset {
            path: entry.path.clone(),
            handle,
        });
    }
    pending
}

/// Moves finished assets into the store. A missing or undecodable asset is not
/// fatal: images become a magenta placeholder, sounds are skipped; each failure
/// is logged and returned.
pub fn finish_loading(
    renderer: &mut Renderer2d,
    store: &mut AssetStore,
    pending: Vec<PendingAsset>,
) -> Vec<String> {
    let mut failures = Vec::new();
    for asset in pending {
        match asset.handle {
            Handle::Image(h) => {
                let created = match h.poll() {
                    AssetPoll::Ready(img) => renderer.create_texture(
                        &asset.path,
                        img.width,
                        img.height,
                        &img.rgba,
                        TextureOptions::COLOR,
                    ),
                    other => {
                        failures.push(describe_failure(&asset.path, &other));
                        renderer.create_texture(
                            &asset.path,
                            1,
                            1,
                            &[255, 0, 255, 255],
                            TextureOptions::COLOR,
                        )
                    }
                };
                match created {
                    Ok(id) => {
                        store.textures.insert(asset.path, id);
                    }
                    Err(err) => failures.push(format!("texture `{}`: {err}", asset.path)),
                }
            }
            Handle::Audio(h) => match h.poll() {
                AssetPoll::Ready(pcm) => match Sound::from_pcm(&pcm) {
                    Ok(sound) => {
                        store.sounds.insert(asset.path, sound);
                    }
                    Err(err) => failures.push(format!("sound `{}`: {err}", asset.path)),
                },
                other => failures.push(describe_failure(&asset.path, &other)),
            },
        }
    }
    for failure in &failures {
        log::error!("asset problem: {failure}");
    }
    failures
}

fn describe_failure<T>(path: &str, poll: &AssetPoll<T>) -> String {
    match poll {
        AssetPoll::Failed(err) => err.to_string(),
        _ => format!("`{path}` could not be loaded"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = include_str!("../../../examples/stories/demo.ron");

    #[test]
    fn demo_story_loads_cleanly() {
        let loaded = parse_story(DEMO.as_bytes().to_vec()).expect("demo parses");
        assert_eq!(loaded.story.title, "Demo");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn syntax_errors_are_readable() {
        let err = parse_story(b"(schema_version: 2, title: ".to_vec()).unwrap_err();
        assert!(!err.is_empty());
        assert!(err.to_lowercase().contains("ron") || err.contains(':'));
    }

    #[test]
    fn invalid_utf8_is_an_error() {
        let err = parse_story(vec![0xff, 0xfe, 0x00]).unwrap_err();
        assert!(err.contains("UTF-8"));
    }

    #[test]
    fn validation_errors_are_listed() {
        // Break a reference: point a Jump/Call-like target at nothing by
        // removing a string the story uses.
        let broken = DEMO.replace(
            "\"01JBX0T0000000000000000004\": \"Hello! Welcome to OrbitEngine.\",",
            "",
        );
        assert_ne!(broken, DEMO, "test fixture drifted");
        let err = parse_story(broken.into_bytes()).unwrap_err();
        assert!(err.contains("validation error"), "{err}");
        assert!(err.contains("unknown string"), "{err}");
    }
}
