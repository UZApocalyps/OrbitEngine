//! Gathers what the report needs from the project folder: the files of the
//! asset folder with their size, and the paths named by the project files
//! that are not the story (UI screens, rigs, 3D scenes, scripts) and by the
//! translations. Disk work only, no UI: it runs in a background task.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_editor_sdk::orbit_core::AssetKind;

use crate::report::{DeclaredAsset, DiskFile, Inputs, join_rel, norm_key, quoted_strings};

/// A text file bigger than this is not read for the paths it names.
const MAX_TEXT_BYTES: u64 = 8 * 1024 * 1024;
/// Folders deeper than this under `ui/` are not read.
const MAX_UI_DEPTH: u32 = 4;

/// What to gather. Everything is owned: the job runs on another thread.
pub struct Job {
    /// The project folder (`ui/` and `scripts/` are under it).
    pub root: PathBuf,
    /// The folder the manifest's paths are relative to.
    pub assets_root: PathBuf,
    /// The story file (translations are next to it).
    pub story_file: PathBuf,
    pub declared: Vec<DeclaredAsset>,
    pub story_uses: std::collections::HashMap<orbit_editor_sdk::orbit_core::AssetId, usize>,
}

/// Every file under `<assets_root>/assets`, recursively (dot files skipped),
/// with `/` separators, sorted by path. `cancelled` is polled per folder.
pub fn walk_assets(assets_root: &Path, cancelled: &dyn Fn() -> bool) -> Option<Vec<DiskFile>> {
    let mut out = Vec::new();
    let mut stack = vec![(assets_root.join("assets"), "assets".to_owned())];
    while let Some((dir, rel)) = stack.pop() {
        if cancelled() {
            return None;
        }
        let Ok(read) = fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let child = format!("{rel}/{name}");
            if meta.is_dir() {
                stack.push((entry.path(), child));
            } else if meta.is_file() {
                out.push(DiskFile { path: child, len: meta.len() });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Some(out)
}

/// Adds the strings quoted in `text` to `paths`: as written, and relative to
/// `file_dir` (the folder of the file, relative to the assets root) when the
/// file is a rig or a 3D scene, which name their pictures and models that way.
fn add_references(text: &str, file_dir: Option<&str>, paths: &mut HashSet<String>) {
    for q in quoted_strings(text) {
        if q.is_empty() || q.len() > 512 {
            continue;
        }
        paths.insert(norm_key(&q));
        if let Some(dir) = file_dir
            && let Some(joined) = join_rel(dir, &q)
        {
            paths.insert(norm_key(&joined));
        }
    }
}

fn read_small_text(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_TEXT_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// The `.ron` files under `dir`, to `MAX_UI_DEPTH` levels.
fn ron_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < MAX_UI_DEPTH {
                ron_files(&path, depth + 1, out);
            }
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ron")) {
            out.push(path);
        }
    }
}

/// Gathers the report inputs. `None` when cancelled.
pub fn gather(job: Job, cancelled: &dyn Fn() -> bool) -> Option<Inputs> {
    let disk = walk_assets(&job.assets_root, cancelled)?;
    let mut sizes: std::collections::HashMap<String, u64> =
        disk.iter().map(|f| (f.path.clone(), f.len)).collect();
    // Declared files outside `assets/` (or listed with another case).
    for a in &job.declared {
        if !sizes.contains_key(&a.path)
            && let Ok(meta) = fs::metadata(job.assets_root.join(&a.path))
            && meta.is_file()
        {
            sizes.insert(a.path.clone(), meta.len());
        }
    }
    if cancelled() {
        return None;
    }

    let mut referenced_paths = HashSet::new();

    // Rigs and 3D scenes: the declared ones and the ones lying in the folder.
    let mut structured: HashSet<String> = job
        .declared
        .iter()
        .filter(|a| matches!(a.kind, AssetKind::Rig | AssetKind::Scene3d))
        .map(|a| a.path.clone())
        .collect();
    structured.extend(
        disk.iter()
            .filter(|f| {
                let p = f.path.to_lowercase();
                p.ends_with(".rig.ron") || p.ends_with(".scene3d.ron")
            })
            .map(|f| f.path.clone()),
    );
    for rel in structured {
        if cancelled() {
            return None;
        }
        if let Some(text) = read_small_text(&job.assets_root.join(&rel)) {
            let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
            add_references(&text, Some(dir), &mut referenced_paths);
        }
    }

    // UI screens and scripts, from the project folder.
    let mut files = Vec::new();
    ron_files(&job.root.join("ui"), 0, &mut files);
    if let Ok(read) = fs::read_dir(job.root.join("scripts")) {
        files.extend(
            read.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("rhai"))),
        );
    }
    for file in files {
        if cancelled() {
            return None;
        }
        if let Some(text) = read_small_text(&file) {
            add_references(&text, None, &mut referenced_paths);
        }
    }

    // Voices of the translations reference sounds by id.
    let mut referenced_ids = HashSet::new();
    for lang in orbit_i18n::files::list_languages(&job.story_file) {
        if cancelled() {
            return None;
        }
        let path = orbit_i18n::files::translation_path(&job.story_file, &lang);
        if let Ok(Some(translation)) = orbit_i18n::files::load(&path) {
            referenced_ids.extend(translation.voices.values().copied());
        }
    }

    Some(Inputs {
        declared: job.declared,
        story_uses: job.story_uses,
        referenced_paths,
        referenced_ids,
        sizes,
        disk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Report;
    use orbit_editor_sdk::orbit_core::AssetId;

    /// A scratch folder under the workspace's `target` (never the system
    /// temp folder), removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/plugin-test-scratch")
                .join(format!("asset_report-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn declared(kind: AssetKind, path: &str) -> DeclaredAsset {
        DeclaredAsset { id: AssetId::new(), kind, path: path.into() }
    }

    #[test]
    fn walks_the_asset_folder_and_skips_dot_files() {
        let s = Scratch::new("walk");
        s.write("assets/a.png", "12345");
        s.write("assets/sub/b.ogg", "12");
        s.write("assets/.cache/c.bin", "x");
        s.write("assets/.hidden", "x");
        s.write("other/d.png", "x");
        let files = walk_assets(&s.0, &|| false).unwrap();
        let paths: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.len)).collect();
        assert_eq!(paths, vec![("assets/a.png", 5), ("assets/sub/b.ogg", 2)]);
        assert!(walk_assets(&s.0, &|| true).is_none(), "cancelled");
        // A project without an asset folder is fine.
        let empty = Scratch::new("walk_empty");
        assert!(walk_assets(&empty.0, &|| false).unwrap().is_empty());
    }

    #[test]
    fn finds_what_ui_rigs_and_scripts_name() {
        let s = Scratch::new("refs");
        s.write("assets/ui_bg.png", "1");
        s.write("assets/rigs/body.png", "1");
        s.write("assets/in_script.ogg", "1");
        s.write("assets/lonely.png", "1");
        s.write("ui/menu.ron", r#"(background: "assets/ui_bg.png")"#);
        s.write("assets/rigs/hero.rig.ron", r#"(skins: [(attachments: {"b": Region((image: "body.png"))})])"#);
        s.write("scripts/fx.rhai", r#"play("assets/in_script.ogg");"#);
        let job = Job {
            root: s.0.clone(),
            assets_root: s.0.clone(),
            story_file: s.0.join("story.orbit"),
            declared: vec![
                declared(AssetKind::Image, "assets/ui_bg.png"),
                declared(AssetKind::Image, "assets/rigs/body.png"),
                declared(AssetKind::Sound, "assets/in_script.ogg"),
                declared(AssetKind::Image, "assets/lonely.png"),
                declared(AssetKind::Image, "assets/vanished.png"),
            ],
            story_uses: Default::default(),
        };
        let report = Report::build(gather(job, &|| false).unwrap());
        let unused: Vec<_> = report.unused().map(|r| r.path.as_str()).collect();
        assert_eq!(unused, vec!["assets/lonely.png", "assets/vanished.png"]);
        assert_eq!(report.missing_count(), 1);
        // The rig file is on disk but not declared.
        assert_eq!(report.undeclared.len(), 1);
        assert_eq!(report.undeclared[0].path, "assets/rigs/hero.rig.ron");
    }

    #[test]
    fn reading_a_text_file_tolerates_missing_files_and_folders() {
        let s = Scratch::new("big");
        s.write("big.ron", "x");
        assert!(read_small_text(&s.0.join("big.ron")).is_some());
        assert!(read_small_text(&s.0.join("missing.ron")).is_none());
        assert!(read_small_text(&s.0).is_none(), "a folder");
    }
}
