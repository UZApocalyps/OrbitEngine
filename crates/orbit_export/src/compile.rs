//! Compiling a project: validation, used assets, translations, game pack.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use orbit_core::{Story, validate};
use orbit_i18n::Translation;
use orbit_pak::{GameInfo, GamePack};
use orbit_project::{ExportProfile, asset_users};
use orbit_ui_data::{Severity, SplashContent, UiBundle};

use crate::error::{ExportError, Result};
use crate::project::ExportProject;

/// How many unreferenced files are listed by name in the warnings.
const LISTED_FILES: usize = 8;

/// A file the game needs: where it is read, and where it lives in the pak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetFile {
    /// Manifest path (relative to the project, `/` separated).
    pub path: String,
    pub source: PathBuf,
}

/// The result of compiling a project.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub pack: GamePack,
    /// The assets the story uses (and only those).
    pub assets: Vec<AssetFile>,
    /// Manifest assets nothing uses: left out of the export.
    pub unreferenced: Vec<String>,
    /// PNG bytes of the game icon.
    pub icon: Option<Vec<u8>>,
    /// Everything worth telling the author (not blocking).
    pub warnings: Vec<String>,
}

fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(ExportError::io(path))
}

/// `(width, height)` of a PNG from its header; `None` when it is not one.
pub(crate) fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || !bytes.starts_with(SIGNATURE) || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

fn load_icon(
    project: &ExportProject,
    profile: &ExportProfile,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<u8>>> {
    let Some(icon) = &profile.icon else {
        return Ok(None);
    };
    let path = project.root.join(icon);
    let bytes = std::fs::read(&path).map_err(|e| ExportError::Icon {
        path: icon.clone(),
        message: e.to_string(),
    })?;
    let Some((w, h)) = png_size(&bytes) else {
        return Err(ExportError::Icon {
            path: icon.clone(),
            message: "it is not a PNG file".to_owned(),
        });
    };
    if w != h {
        warnings.push(format!(
            "the icon `{icon}` is {w}x{h}: a square icon looks better"
        ));
    } else if w < 64 {
        warnings.push(format!(
            "the icon `{icon}` is only {w}x{h}: 256x256 is recommended"
        ));
    }
    Ok(Some(bytes))
}

/// The project's data-driven UI (`ui/` folder: theme, screens, splashes),
/// checked and serialized as the RON text of a [`UiBundle`]. The images of
/// the splashes join `assets`. `None` when the project has no `ui/` folder.
fn load_ui(
    project: &ExportProject,
    assets: &mut Vec<AssetFile>,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<u8>>> {
    let dir = project.root.join("ui");
    if !dir.is_dir() {
        return Ok(None);
    }
    let bundle = UiBundle::load_dir(&dir).map_err(|e| ExportError::Ui(vec![e]))?;
    let mut errors = Vec::new();
    for issue in bundle.validate() {
        let text = format!("{} {}: {}", issue.screen, issue.widget, issue.message);
        match issue.severity {
            Severity::Error => errors.push(text),
            Severity::Warning => warnings.push(format!("ui {text}")),
        }
    }
    if !errors.is_empty() {
        return Err(ExportError::Ui(errors));
    }
    for splash in &bundle.splashes {
        if let SplashContent::Image(path) = &splash.content {
            let source = project.root.join(path);
            if !source.is_file() {
                warnings.push(format!(
                    "the splash image `{path}` is missing: the splash will be empty"
                ));
            } else if !assets.iter().any(|a| &a.path == path) {
                assets.push(AssetFile {
                    path: path.clone(),
                    source,
                });
            }
        }
    }
    let text = bundle.to_ron().map_err(|e| ExportError::Ui(vec![e]))?;
    Ok(Some(text.into_bytes()))
}

/// The translation files next to the story: `i18n/<story stem>.<lang>.ron`.
fn load_translations(
    project: &ExportProject,
    story: &Story,
    warnings: &mut Vec<String>,
) -> Result<Vec<Translation>> {
    let story_path = project.story_path();
    let Some(dir) = story_path.parent().map(|p| p.join("i18n")) else {
        return Ok(Vec::new());
    };
    let Ok(read_dir) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let stem = story_path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let prefix = format!("{stem}.");
    let mut files: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(&prefix) && n.ends_with(".ron") && n.len() > prefix.len() + 4
            })
        })
        .collect();
    files.sort();
    let mut out: Vec<Translation> = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let text = read_text(&file)?;
        let translation = Translation::from_ron(&text).map_err(|e| ExportError::Translation {
            file: name.clone(),
            message: e.to_string(),
        })?;
        if translation.lang.trim().is_empty() {
            return Err(ExportError::Translation {
                file: name,
                message: "the language tag (`lang`) is empty".to_owned(),
            });
        }
        if translation
            .lang
            .eq_ignore_ascii_case(&story.strings.source_lang)
        {
            warnings.push(format!(
                "translation `{name}` is in the source language of the story ({}): ignored",
                translation.lang
            ));
            continue;
        }
        if out
            .iter()
            .any(|t| t.lang.eq_ignore_ascii_case(&translation.lang))
        {
            warnings.push(format!(
                "translation `{name}`: the language {} is already provided by another file: ignored",
                translation.lang
            ));
            continue;
        }
        let report = orbit_i18n::coverage(story, &translation);
        if !report.missing.is_empty() {
            warnings.push(format!(
                "translation {}: {} of {} string(s) are missing (they fall back to the source language)",
                translation.lang,
                report.missing.len(),
                report.total
            ));
        }
        let stale = orbit_i18n::stale_entries(&story.strings, &translation).len();
        if stale > 0 {
            warnings.push(format!(
                "translation {}: {stale} string(s) were translated from a source text that has since changed (see `orbit i18n stale`)",
                translation.lang
            ));
        }
        out.push(translation);
    }
    Ok(out)
}

/// Files under `assets/` that the story does not declare, as warnings.
fn untracked_files(root: &Path, declared: &HashSet<String>) -> Option<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut files = Vec::new();
    walk(&root.join("assets"), root, &mut files);
    files.retain(|f| !declared.contains(f));
    files.sort();
    if files.is_empty() {
        return None;
    }
    let listed: Vec<&str> = files
        .iter()
        .take(LISTED_FILES)
        .map(String::as_str)
        .collect();
    let more = files.len().saturating_sub(LISTED_FILES);
    Some(format!(
        "{} file(s) under assets/ are not declared in the story and are not exported: {}{}",
        files.len(),
        listed.join(", "),
        if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        }
    ))
}

/// Compiles `project` for `profile`.
///
/// Fails when the profile is unusable, when the story has validation errors
/// or when an asset the story uses is missing from the disk; everything else
/// is a warning.
pub fn compile(project: &ExportProject, profile: &ExportProfile) -> Result<Compiled> {
    let problems = profile.problems();
    if !problems.is_empty() {
        return Err(ExportError::Profile(problems));
    }
    let story_path = project.story_path();
    let text = read_text(&story_path)?;
    let mut story = Story::from_ron_unchecked(&text)
        .map_err(|e| ExportError::Project(format!("{}: {e}", story_path.display())))?;

    let report = validate(&story);
    if !report.errors.is_empty() {
        return Err(ExportError::Invalid(
            report.errors.iter().map(ToString::to_string).collect(),
        ));
    }
    let mut warnings: Vec<String> = report.warnings.iter().map(ToString::to_string).collect();

    // Used and unused assets.
    let mut assets = Vec::new();
    let mut unreferenced = Vec::new();
    let mut missing = Vec::new();
    let mut seen = HashSet::new();
    let declared: HashSet<String> = story.assets.assets.iter().map(|a| a.path.clone()).collect();
    for entry in &story.assets.assets {
        let source = project.root.join(&entry.path);
        if asset_users(&story, entry.id).is_empty() {
            unreferenced.push(entry.path.clone());
            continue;
        }
        if !source.is_file() {
            missing.push(entry.path.clone());
        } else if seen.insert(entry.path.clone()) {
            assets.push(AssetFile {
                path: entry.path.clone(),
                source,
            });
        }
    }
    if !missing.is_empty() {
        return Err(ExportError::MissingAssets(missing));
    }
    if !unreferenced.is_empty() {
        warnings.push(format!(
            "{} asset(s) declared in the story are used by nothing and are not exported: {}",
            unreferenced.len(),
            unreferenced.join(", ")
        ));
    }
    if let Some(message) = untracked_files(&project.root, &declared) {
        warnings.push(message);
    }
    // The shipped story only knows the assets it ships.
    let keep: HashSet<&str> = assets.iter().map(|a| a.path.as_str()).collect();
    story
        .assets
        .assets
        .retain(|a| keep.contains(a.path.as_str()));

    let translations = load_translations(project, &story, &mut warnings)?;
    if let Some(lang) = &profile.default_language
        && !lang.trim().is_empty()
    {
        let known = lang.eq_ignore_ascii_case(&story.strings.source_lang)
            || translations
                .iter()
                .any(|t| t.lang.eq_ignore_ascii_case(lang));
        if !known {
            warnings.push(format!(
                "the default language `{lang}` is neither the source language ({}) nor a translation of the game: it will be ignored",
                story.strings.source_lang
            ));
        }
    }
    let icon = load_icon(project, profile, &mut warnings)?;
    let ui = load_ui(project, &mut assets, &mut warnings)?;

    let info = GameInfo {
        name: profile.game_name.clone(),
        id: profile.game_id.clone(),
        version: profile.version.clone(),
        default_language: profile
            .default_language
            .clone()
            .filter(|l| !l.trim().is_empty()),
    };
    Ok(Compiled {
        pack: GamePack {
            info,
            story,
            translations,
            ui,
        },
        assets,
        unreferenced,
        icon,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, demo_project};

    #[test]
    fn a_ui_folder_is_compiled_into_the_pack_with_its_splash_images() {
        let dir = TempDir::new("compile_ui");
        let project = demo_project(dir.path());
        let profile = project.profile(None).unwrap();
        assert!(compile(&project, &profile).unwrap().pack.ui.is_none());

        let ui = dir.path().join("ui");
        std::fs::create_dir_all(&ui).unwrap();
        std::fs::write(
            ui.join("pause.screen.ron"),
            "(id: \"pause\", root: (kind: Stack(dir: Vertical)))",
        )
        .unwrap();
        std::fs::write(
            ui.join("ui.ron"),
            "(splashes: [(content: Image(\"splash/studio.png\"), duration: 2.0)])",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("splash")).unwrap();
        std::fs::write(dir.path().join("splash/studio.png"), b"png").unwrap();
        let compiled = compile(&project, &profile).unwrap();
        let bundle = orbit_ui_data::UiBundle::from_ron(
            std::str::from_utf8(compiled.pack.ui.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(bundle.screens.len(), 1);
        assert_eq!(bundle.splashes.len(), 1);
        assert!(
            compiled
                .assets
                .iter()
                .any(|a| a.path == "splash/studio.png")
        );

        // A broken screen blocks the export.
        std::fs::write(
            ui.join("pause.screen.ron"),
            "(id: \"pause\", root: (kind: Grid(columns: 0)))",
        )
        .unwrap();
        assert!(matches!(
            compile(&project, &profile),
            Err(ExportError::Ui(_))
        ));
    }

    // Fixture: exact title, asset count and unreferenced list come from test_minimal.ron.
    #[test]
    fn the_fixture_compiles_with_its_translation() {
        let dir = TempDir::new("compile_fixture");
        let project = crate::testing::fixture_project(dir.path());
        let profile = project.profile(None).unwrap();
        let compiled = compile(&project, &profile).unwrap();
        assert_eq!(compiled.pack.story.title, "Test Minimal");
        assert_eq!(compiled.assets.len(), 5);
        assert_eq!(compiled.pack.translations.len(), 1);
        assert_eq!(compiled.pack.translations[0].lang, "fr");
        assert!(compiled.unreferenced.is_empty());
    }

    // Demo: global property only (compiles, nothing blocking, nothing unreferenced,
    // its translation is shipped); no exact counts.
    #[test]
    fn the_demo_still_compiles_with_its_translation() {
        let dir = TempDir::new("compile_demo");
        let project = demo_project(dir.path());
        let profile = project.profile(None).unwrap();
        let compiled = compile(&project, &profile).unwrap();
        assert!(!compiled.assets.is_empty());
        assert!(compiled.pack.translations.iter().any(|t| t.lang == "fr"));
        assert!(
            compiled.unreferenced.is_empty(),
            "{:?}",
            compiled.unreferenced
        );
    }

    #[test]
    fn errors_are_blocking_and_unused_assets_are_dropped() {
        let dir = TempDir::new("compile_errors");
        let project = crate::testing::fixture_project(dir.path());
        let profile = project.profile(None).unwrap();

        // A missing used asset blocks the export.
        std::fs::remove_file(dir.path().join("fixtures/assets/bg_room.png")).unwrap();
        let err = compile(&project, &profile).unwrap_err();
        assert!(
            matches!(err, ExportError::MissingAssets(ref m) if m.len() == 1),
            "{err}"
        );
        crate::testing::restore_fixture_assets(dir.path());

        // A broken story blocks it too.
        let story = std::fs::read_to_string(dir.path().join("story.orbit"))
            .unwrap()
            .replace("\r\n", "\n");
        std::fs::write(
            dir.path().join("story.orbit"),
            story.replace(
                "\"01JBX0T0000000000000000003\": \"Neo-Kyoto, 2087. Rain falls on the megatowers.\",",
                "",
            ),
        )
        .unwrap();
        assert!(matches!(
            compile(&project, &profile),
            Err(ExportError::Invalid(_))
        ));
        std::fs::write(dir.path().join("story.orbit"), &story).unwrap();

        // An asset nothing uses is warned about and left out.
        std::fs::write(
            dir.path().join("story.orbit"),
            story.replace(
                "assets: [\n",
                "assets: [\n            (id: \"01JBX0A0000000000000000099\", kind: Image, path: \"fixtures/assets/bg_street.png\"),\n",
            ),
        )
        .unwrap();
        let compiled = compile(&project, &profile).unwrap();
        assert_eq!(compiled.unreferenced, ["fixtures/assets/bg_street.png"]);
        assert!(
            compiled
                .assets
                .iter()
                .all(|a| a.path != "fixtures/assets/bg_street.png")
        );
        assert!(
            compiled
                .pack
                .story
                .assets
                .assets
                .iter()
                .all(|a| a.path != "fixtures/assets/bg_street.png")
        );
        assert!(
            compiled
                .warnings
                .iter()
                .any(|w| w.contains("bg_street.png"))
        );
    }

    #[test]
    fn a_bad_icon_or_profile_is_refused() {
        let dir = TempDir::new("compile_icon");
        let project = demo_project(dir.path());
        let mut profile = project.profile(None).unwrap();
        profile.icon = Some("assets/demo/music_theme.wav".into());
        assert!(matches!(
            compile(&project, &profile),
            Err(ExportError::Icon { .. })
        ));
        profile.icon = Some("assets/demo/bg_room.png".into());
        let compiled = compile(&project, &profile).unwrap();
        assert!(compiled.icon.is_some());
        assert!(compiled.warnings.iter().any(|w| w.contains("square")));
        profile.game_id = "Bad Id".into();
        assert!(matches!(
            compile(&project, &profile),
            Err(ExportError::Profile(_))
        ));
    }

    #[test]
    fn png_header_is_read() {
        let png = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/brand/icon-256.png"
        ))
        .unwrap();
        assert_eq!(png_size(&png), Some((256, 256)));
        assert_eq!(png_size(b"nope"), None);
    }
}
