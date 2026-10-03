//! Loading of the story file and of the assets its manifest declares.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use orbit_assets::{AssetHandle, AssetPoll, ImageData, PcmAudio};
use orbit_audio::Sound;
use orbit_core::{AssetKind, NodeKind, Story, validate};
use orbit_i18n::Translation;
use orbit_pak::{GameInfo, GamePack};
use orbit_render2d::{Renderer2d, TextureId, TextureOptions};
use orbit_ui::UiBundle;

/// A parsed and validated story.
#[derive(Debug)]
pub struct LoadedStory {
    pub story: Arc<Story>,
    /// Non-fatal validation warnings (already logged).
    pub warnings: Vec<String>,
    /// Identity of the game, when the story comes from a game pack.
    pub info: Option<GameInfo>,
    /// Translations compiled into the game pack (empty for a RON story: its
    /// translation files are loaded separately).
    pub translations: Vec<Translation>,
    /// The UI bundle compiled into the game pack, when it has one and it
    /// is valid (a broken one is logged and ignored: the stock screens
    /// are used).
    pub ui: Option<UiBundle>,
}

/// Parses and validates story bytes: a RON file (development) or a game
/// pack (exported game, see `orbit_pak`). Every problem becomes a readable
/// error message (never a panic): RON syntax errors, unsupported schema
/// versions and the full list of validation errors.
pub fn parse_story(bytes: Vec<u8>) -> Result<LoadedStory, String> {
    let (story, info, translations, ui) = if GamePack::is_pack(&bytes) {
        let pack = GamePack::from_bytes(&bytes).map_err(|e| e.to_string())?;
        let ui = pack.ui.as_deref().and_then(parse_ui_bundle);
        (pack.story, Some(pack.info), pack.translations, ui)
    } else {
        let text =
            String::from_utf8(bytes).map_err(|e| format!("the file is not valid UTF-8: {e}"))?;
        let story = Story::from_ron_unchecked(&text).map_err(|e| e.to_string())?;
        (story, None, Vec::new(), None)
    };
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
        info,
        translations,
        ui,
    })
}

/// Parses and checks the RON text of a UI bundle; a bundle with errors is
/// refused (the warnings are only logged).
pub fn parse_ui_bundle(bytes: &[u8]) -> Option<UiBundle> {
    let bundle = std::str::from_utf8(bytes)
        .map_err(|e| e.to_string())
        .and_then(UiBundle::from_ron)
        .map_err(|e| log::warn!("the UI bundle is unreadable, using the stock screens: {e}"))
        .ok()?;
    check_ui_bundle(bundle)
}

/// Logs the issues of `bundle`; `None` when one is an error.
pub fn check_ui_bundle(bundle: UiBundle) -> Option<UiBundle> {
    let mut ok = true;
    for issue in bundle.validate() {
        match issue.severity {
            orbit_ui::Severity::Error => {
                ok = false;
                log::warn!("ui `{}` {}: {}", issue.screen, issue.widget, issue.message);
            }
            orbit_ui::Severity::Warning => {
                log::info!("ui `{}` {}: {}", issue.screen, issue.widget, issue.message);
            }
        }
    }
    if !ok {
        log::warn!("the UI bundle has errors, using the stock screens");
    }
    ok.then_some(bundle)
}

/// Paths of the assets scene `scene` (an index of `story.scenes`) needs:
/// backgrounds, music, sounds and the sprites of the characters it shows.
pub fn scene_asset_paths(story: &Story, scene: usize) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut add = |id: orbit_core::AssetId| {
        if let Some(entry) = story.assets.get(id)
            && !paths.contains(&entry.path)
        {
            paths.push(entry.path.clone());
        }
    };
    let Some(scene) = story.scenes.get(scene) else {
        return paths;
    };
    for node in &scene.nodes {
        match &node.kind {
            NodeKind::Background { asset, .. }
            | NodeKind::PlayMusic { asset, .. }
            | NodeKind::PlaySound { asset }
            | NodeKind::ShowScene3d { scene: asset, .. } => add(*asset),
            NodeKind::Show {
                character,
                expression,
                ..
            } => {
                if let Some(expr) = story
                    .character(*character)
                    .and_then(|c| c.expression(expression))
                {
                    for asset in expr.assets() {
                        add(asset);
                    }
                }
            }
            _ => {}
        }
    }
    paths
}

/// Scenes (indices) a scene can lead to: the targets of its jumps.
pub fn successor_scenes(story: &Story, scene: usize) -> Vec<usize> {
    let Some(from) = story.scenes.get(scene) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for node in &from.nodes {
        let by_scene = node
            .kind
            .target_scene()
            .and_then(|id| story.scenes.iter().position(|s| s.id == id));
        let by_label = node
            .kind
            .targets()
            .into_iter()
            .filter_map(|label| story.locate(label).map(|(s, _)| s));
        for target in by_scene.into_iter().chain(by_label) {
            if target != scene && !out.contains(&target) {
                out.push(target);
            }
        }
    }
    out
}

/// GPU textures and decoded sounds, by manifest path.
#[derive(Default)]
pub struct AssetStore {
    textures: HashMap<String, TextureId>,
    sounds: HashMap<String, Sound>,
    /// Parsed `.scene3d.ron` documents (feature `3d`).
    #[cfg(feature = "3d")]
    scenes3d: HashMap<String, Arc<orbit_scene::Scene3d>>,
    /// The packed 3D models (`3d/` pak entries), once loaded.
    #[cfg(feature = "3d")]
    library3d: Option<Arc<crate::library3d::Library3d>>,
    /// Set when the 3D models started loading (once per store).
    #[cfg(feature = "3d")]
    library3d_requested: std::sync::atomic::AtomicBool,
    fonts: HashSet<String>,
    new_fonts: Vec<Vec<u8>>,
}

impl AssetStore {
    pub fn texture(&self, path: &str) -> Option<TextureId> {
        self.textures.get(path).copied()
    }

    pub fn sound(&self, path: &str) -> Option<&Sound> {
        self.sounds.get(path)
    }

    #[cfg(test)]
    pub(crate) fn insert_texture(&mut self, path: &str, texture: TextureId) {
        self.textures.insert(path.to_owned(), texture);
    }

    /// A loaded 3D scene.
    #[cfg(feature = "3d")]
    pub fn scene3d(&self, path: &str) -> Option<&Arc<orbit_scene::Scene3d>> {
        self.scenes3d.get(path)
    }

    /// The packed 3D models, once loaded (an empty library when the game
    /// has none). A new `Arc` each time it is (re)loaded.
    #[cfg(feature = "3d")]
    pub fn library3d(&self) -> Option<&Arc<crate::library3d::Library3d>> {
        self.library3d.as_ref()
    }

    /// Adds (or replaces) a 3D scene under `path` (hosts that build scenes
    /// in memory, tests).
    #[cfg(feature = "3d")]
    pub fn insert_scene3d(&mut self, path: impl Into<String>, scene: orbit_scene::Scene3d) {
        self.scenes3d.insert(path.into(), Arc::new(scene));
    }

    pub fn contains(&self, path: &str) -> bool {
        #[cfg(feature = "3d")]
        if self.scenes3d.contains_key(path) {
            return true;
        }
        self.textures.contains_key(path)
            || self.sounds.contains_key(path)
            || self.fonts.contains(path)
    }

    /// Font files loaded since the last call (the runtime registers them
    /// with the text engine).
    pub fn take_new_fonts(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.new_fonts)
    }

    /// Drops the asset at `path` (a file that changed on disk): the next
    /// loading pass fetches it again. The texture, if any, is released.
    pub fn forget(&mut self, renderer: &mut Renderer2d, path: &str) -> bool {
        let texture = self.textures.remove(path);
        if let Some(texture) = texture {
            renderer.release_texture(texture);
        }
        let sound = self.sounds.remove(path);
        let font = self.fonts.remove(path);
        #[cfg(feature = "3d")]
        let font = font || self.scenes3d.remove(path).is_some();
        texture.is_some() || sound.is_some() || font
    }

    /// Drops every asset (the asset root changed).
    pub fn clear(&mut self, renderer: &mut Renderer2d) {
        for (_, texture) in self.textures.drain() {
            renderer.release_texture(texture);
        }
        self.sounds.clear();
        #[cfg(feature = "3d")]
        {
            self.scenes3d.clear();
            self.library3d = None;
            self.library3d_requested = Default::default();
        }
        self.fonts.clear();
    }
}

enum Handle {
    Image(AssetHandle<ImageData>),
    Audio(AssetHandle<PcmAudio>),
    #[cfg(feature = "3d")]
    Scene3d(AssetHandle<orbit_scene::Scene3d>),
    #[cfg(feature = "3d")]
    Library3d(Box<crate::library3d::LibraryLoad>),
    Font(AssetHandle<Vec<u8>>),
}

/// One manifest asset being loaded.
pub struct PendingAsset {
    path: String,
    handle: Handle,
}

impl PendingAsset {
    /// Manifest path of the asset.
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn is_done(&self) -> bool {
        match &self.handle {
            Handle::Image(h) => !matches!(h.poll(), AssetPoll::Loading),
            Handle::Audio(h) => !matches!(h.poll(), AssetPoll::Loading),
            #[cfg(feature = "3d")]
            Handle::Scene3d(h) => !matches!(h.poll(), AssetPoll::Loading),
            #[cfg(feature = "3d")]
            Handle::Library3d(l) => l.is_done(),
            Handle::Font(h) => !matches!(h.poll(), AssetPoll::Loading),
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
    start_loading_only(loader, story, store, None)
}

/// [`start_loading`] restricted to `only` (manifest paths) when given: the
/// lazy mode loads a scene's assets when it needs them.
pub fn start_loading_only(
    loader: &orbit_assets::AssetLoader,
    story: &Story,
    store: &AssetStore,
    only: Option<&[String]>,
) -> Vec<PendingAsset> {
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    for entry in &story.assets.assets {
        if only.is_some_and(|only| !only.contains(&entry.path))
            || store.contains(&entry.path)
            || !seen.insert(entry.path.clone())
        {
            continue;
        }
        let handle = match entry.kind {
            AssetKind::Image => Handle::Image(loader.load_image(&entry.path)),
            AssetKind::Music | AssetKind::Sound => Handle::Audio(loader.load_audio(&entry.path)),
            #[cfg(feature = "3d")]
            AssetKind::Scene3d => {
                // The packed models come with the first 3D scene.
                if !store
                    .library3d_requested
                    .swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    pending.push(PendingAsset {
                        path: orbit_scene::pack3d::INDEX_ENTRY.to_owned(),
                        handle: Handle::Library3d(Box::new(
                            crate::library3d::LibraryLoad::start(loader),
                        )),
                    });
                }
                Handle::Scene3d(loader.load_with(&entry.path, |_, bytes| {
                    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
                    orbit_scene::from_ron(&text)
                }))
            }
            // Without the `3d` feature a 3D scene is never drawn: not loaded.
            #[cfg(not(feature = "3d"))]
            AssetKind::Scene3d => {
                log::warn!(
                    "`{}` is a 3D scene, but this player is built without 3D support",
                    entry.path
                );
                continue;
            }
            AssetKind::Font => Handle::Font(loader.load_bytes(&entry.path)),
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
            #[cfg(feature = "3d")]
            Handle::Scene3d(h) => match h.poll() {
                AssetPoll::Ready(scene) => {
                    store.scenes3d.insert(asset.path, scene);
                }
                other => failures.push(describe_failure(&asset.path, &other)),
            },
            #[cfg(feature = "3d")]
            Handle::Library3d(load) => {
                let (lib, errors) = load.finish();
                failures.extend(errors);
                store.library3d = Some(Arc::new(lib));
            }
            Handle::Font(h) => match h.poll() {
                AssetPoll::Ready(bytes) => {
                    store.fonts.insert(asset.path);
                    store.new_fonts.push(bytes.to_vec());
                }
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

    const DEMO: &str = include_str!("../../../examples/stories/test_minimal.ron");

    #[test]
    fn test_fixture_story_loads_cleanly() {
        let loaded = parse_story(DEMO.as_bytes().to_vec()).expect("fixture parses");
        assert_eq!(loaded.story.title, "Test Minimal");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn a_game_pack_loads_like_the_ron_it_comes_from() {
        let story = Story::from_ron_unchecked(DEMO).unwrap();
        let pack = GamePack {
            info: GameInfo {
                name: "Demo".into(),
                id: "demo".into(),
                version: "1.0.0".into(),
                default_language: None,
            },
            story: story.clone(),
            translations: vec![Translation::new("fr", "Francais")],
            ui: Some(UiBundle::builtin().to_ron().unwrap().into_bytes()),
        };
        let loaded = parse_story(pack.to_bytes().unwrap()).expect("pack parses");
        assert_eq!(*loaded.story, story);
        assert_eq!(loaded.info.unwrap().id, "demo");
        assert_eq!(loaded.translations.len(), 1);
        assert_eq!(loaded.ui, Some(UiBundle::builtin()));
        // A damaged pack is an error, not a panic.
        let mut bytes = pack.to_bytes().unwrap();
        bytes.truncate(40);
        assert!(parse_story(bytes).is_err());
    }

    #[test]
    fn scene_assets_and_successors_of_the_demo() {
        let story = Story::from_ron_unchecked(DEMO).unwrap();
        let all: Vec<String> = story.assets.assets.iter().map(|a| a.path.clone()).collect();
        let mut seen = Vec::new();
        for scene in 0..story.scenes.len() {
            seen.extend(scene_asset_paths(&story, scene));
        }
        // Everything the demo declares is used by some scene.
        for path in &all {
            assert!(seen.contains(path), "{path} is not used by any scene");
        }
        assert!(scene_asset_paths(&story, 99).is_empty());
        assert!(successor_scenes(&story, 99).is_empty());
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
            "\"01JBX0T0000000000000000003\": \"Neo-Kyoto, 2087. Rain falls on the megatowers.\",",
            "",
        );
        assert_ne!(broken, DEMO, "test fixture drifted");
        let err = parse_story(broken.into_bytes()).unwrap_err();
        assert!(err.contains("validation error"), "{err}");
        assert!(err.contains("unknown string"), "{err}");
    }
}
