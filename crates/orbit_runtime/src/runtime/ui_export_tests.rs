//! End to end: the files a project's `ui/` folder references are exported
//! into the pak and loaded by the runtime of the exported game before the
//! first screen shows (the editor preview reads them from the project, an
//! exported game only has its pak).

use std::path::Path;
use std::time::Duration;

use orbit_export::testing::{TempDir, fixture_project};

use super::*;

const BUTTON: &str = "assets/ui/button.png";
const BUTTON_HOVER: &str = "assets/ui/button_hover.png";
const PANEL: &str = "assets/ui/panel.png";
const LOGO: &str = "assets/ui/logo.png";
const INDICATOR: &str = "assets/ui/next.png";
const SPLASH: &str = "assets/ui/splash.png";
const FONT: &str = "assets/ui/title.ttf";
const MEDAL: &str = "assets/ach/medal.png";
const MEDAL_LOCKED: &str = "assets/ach/medal_locked.png";

fn write_png(path: &Path, rgba: [u8; 4]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba(rgba));
    img.save(path).unwrap();
}

/// A project (the fixture story) with a `ui/` folder: an image button
/// style (per-state images), a nine-slice panel, a project font, a continue
/// indicator image, a main menu with a fixed image and an image splash.
/// None of these files is declared in the story.
fn ui_project(dir: &Path) -> orbit_export::ExportProject {
    let project = fixture_project(dir);
    for (path, color) in [
        (BUTTON, [200, 0, 0, 255]),
        (BUTTON_HOVER, [0, 200, 0, 255]),
        (PANEL, [0, 0, 200, 255]),
        (LOGO, [200, 200, 0, 255]),
        (INDICATOR, [0, 200, 200, 255]),
        (SPLASH, [200, 0, 200, 255]),
        (MEDAL, [255, 215, 0, 255]),
        (MEDAL_LOCKED, [90, 90, 90, 255]),
    ] {
        write_png(&dir.join(path), color);
    }
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/Inter-Bold.ttf"),
        dir.join(FONT),
    )
    .unwrap();
    // An achievement with an unlocked and a locked icon (used by no node).
    let story_path = dir.join("story.orbit");
    let mut story =
        Story::from_ron_unchecked(&std::fs::read_to_string(&story_path).unwrap()).unwrap();
    let mut image = |path: &str| {
        let id = orbit_core::AssetId::new();
        story.assets.assets.push(orbit_core::AssetEntry {
            id,
            kind: orbit_core::AssetKind::Image,
            path: path.into(),
        });
        id
    };
    let (icon, locked) = (image(MEDAL), image(MEDAL_LOCKED));
    let title_id = story.strings.add("First step");
    let description_id = story.strings.add("Take it.");
    story.achievements.push(orbit_core::Achievement {
        id: orbit_core::AchievementId::new(),
        title_id,
        description_id,
        hidden: false,
        icon: Some(icon),
        locked_icon: Some(locked),
    });
    std::fs::write(&story_path, story.to_ron().unwrap()).unwrap();
    let ui = dir.join("ui");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(
        ui.join("theme.ron"),
        format!(
            "(textbox: (indicator_image: Some(\"{INDICATOR}\")), \
              fonts: {{\"title\": (asset: \"{FONT}\", family: \"Inter\")}}, \
              styles: {{\
                \"image_button\": (font: Some(\"title\"), show_text: Some(false), \
                    fills: (normal: Some(Image(image: \"{BUTTON}\")), \
                            hover: Some(Image(image: \"{BUTTON_HOVER}\")))), \
                \"panel\": (fill: Some(NineSlice(image: \"{PANEL}\", insets: (2.0, 2.0, 2.0, 2.0)))), \
              }})"
        ),
    )
    .unwrap();
    std::fs::write(
        ui.join("title.screen.ron"),
        format!(
            "(id: \"title\", root: (kind: Stack(dir: Vertical), style: Some(\"panel\"), children: [\
                (kind: Image(asset: Some(\"{LOGO}\"))), \
                (kind: Image(asset: Some(\"{LOGO}\"), action: [GameNew]), style: Some(\"image_button\")), \
             ]))"
        ),
    )
    .unwrap();
    std::fs::write(
        ui.join("ui.ron"),
        format!(
            "(system_screens: {{\"main_menu\": \"title\"}}, \
              splashes: [(content: Image(\"{SPLASH}\"), duration: 1.0)])"
        ),
    )
    .unwrap();
    project
}

fn renderer() -> Option<Renderer2d> {
    let gfx = pollster::block_on(orbit_render2d::orbit_gfx::Gfx::headless((64, 64))).ok()?;
    Renderer2d::new(gfx).ok()
}

#[test]
fn ui_files_are_exported_and_loaded_before_the_first_screen() {
    let dir = TempDir::new("runtime_ui_export");
    let project = ui_project(dir.path());
    let profile = project.profile(None).unwrap();
    let compiled = orbit_export::compile(&project, &profile).unwrap();
    let all = [BUTTON, BUTTON_HOVER, PANEL, LOGO, INDICATOR, SPLASH, FONT, MEDAL, MEDAL_LOCKED];

    // Every file the UI references is in the pak.
    let built =
        orbit_export::build_pak(&compiled, &profile, &mut orbit_export::Reporter::silent())
            .unwrap();
    let (_, index) = orbit_pak::container::parse_prefix(&built.bytes).unwrap();
    for path in all {
        assert!(index.read_from(&built.bytes, path).is_ok(), "`{path}` is not in the pak");
    }
    let pak_path = dir.path().join("game.pak");
    std::fs::write(&pak_path, &built.bytes).unwrap();

    if renderer().is_none() {
        eprintln!("no GPU adapter: the runtime part of the test is skipped");
        return;
    }
    for lazy in [false, true] {
        let mut config = RuntimeConfig::new("", "", "", "");
        config.pak = Some(orbit_assets::PakLocation::file(pak_path.to_string_lossy()));
        config.data_dir = Some(dir.path().join(format!("data_{lazy}")).to_string_lossy().into_owned());
        config.lazy_assets = lazy;
        let mut rt = Runtime::new(renderer().unwrap(), config).unwrap();
        for _ in 0..5_000 {
            rt.update(Duration::from_millis(16));
            if matches!(rt.phase, Phase::Ready(_) | Phase::Failed(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(rt.phase, Phase::Ready(_)), "the game is ready (lazy: {lazy})");
        // Before the first screen: every image is a texture, the font is
        // registered, the splash image is requested.
        for path in [BUTTON, BUTTON_HOVER, PANEL, LOGO, INDICATOR, MEDAL, MEDAL_LOCKED] {
            assert!(rt.store.texture(path).is_some(), "`{path}` is not loaded (lazy: {lazy})");
        }
        assert!(rt.store.contains(FONT), "the theme font is not loaded (lazy: {lazy})");
        assert!(rt.splash_assets.has_image(SPLASH), "the splash image is not requested");
        let images = theme_images(&rt.ctx.theme, &rt.store);
        for path in [BUTTON, BUTTON_HOVER, PANEL, INDICATOR] {
            assert!(images.contains_key(path), "`{path}` is not a UI image (lazy: {lazy})");
        }
        // The achievements screen binds Achievement(Icon): the locked icon
        // while locked, the icon once unlocked, both drawable textures.
        let Phase::Ready(shell) = &rt.phase else { unreachable!() };
        let story = shell.story().clone();
        let achievements = achievement_images(&story, &rt.store);
        let loc = orbit_i18n::Localizer::from_story(&story);
        let texts = orbit_ui::Texts::builtin("en");
        let id = story.achievements[0].id;
        let locked = crate::achievements::rows(&story, &loc, &texts, &Default::default());
        assert_eq!(locked[0].icon, MEDAL_LOCKED);
        let unlocked = crate::achievements::rows(&story, &loc, &texts, &[id].into());
        assert_eq!(unlocked[0].icon, MEDAL);
        assert!(achievements.contains_key(MEDAL) && achievements.contains_key(MEDAL_LOCKED));
    }
}
