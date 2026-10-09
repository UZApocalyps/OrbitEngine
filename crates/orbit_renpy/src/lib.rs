//! Ren'Py import: converts a Ren'Py game folder into a new OrbitEngine
//! project (editor and CLI only; the runtime, the player and the web build
//! never link this crate).
//!
//! [`import`] reads `game/*.rpy` (the common script subset: characters,
//! variables, labels, jumps, calls, menus, conditions, Python assignments,
//! dialogue, `show` / `hide` / `scene` / `with`, music and sounds, pauses;
//! transforms and simple ATL as positions and timelines, script-defined
//! images, character text styles, navigation screens as hotspots or
//! choices, achievements, persistent data, the common `renpy.*` calls),
//! the images (Ren'Py naming: `images/eileen happy.png` is the expression
//! `happy` of the character `eileen`, `bg room.png` a background), the audio
//! and the translations of `game/tl/`, and writes a project that validates.
//! Whatever has no equivalent is listed in an [`ImportReport`], also written
//! to `import-report.md` in the new project.
//!
//! Labels become scenes (one per top-level label, `start` first); nested and
//! local labels become labels of their scene.

mod atl;
mod convert;
mod lexer;
mod png;
mod pyexpr;
mod screen;
mod text;
mod tl;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_core::{Expr, NodeKind, Story, ValidationError, validate};
use orbit_project::{DEFAULT_STORY_FILE, ProjectManifest, STATE_DIR};

pub use convert::{Script, image_name, parse_color};
pub use tl::language;

/// Name of the report written in the new project.
pub const REPORT_FILE: &str = "import-report.md";

/// A remark tied to a place of the Ren'Py sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Path relative to the game folder (empty: the whole game).
    pub file: String,
    /// 1-based line (0: the whole file).
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.file.is_empty(), self.line) {
            (true, _) => write!(f, "{}", self.message),
            (false, 0) => write!(f, "{}: {}", self.file, self.message),
            (false, l) => write!(f, "{}:{l}: {}", self.file, self.message),
        }
    }
}

/// Kind of a media file of the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportReport {
    pub source: PathBuf,
    pub project: PathBuf,
    pub scripts: usize,
    pub scenes: usize,
    pub labels: usize,
    pub nodes: usize,
    pub characters: usize,
    pub variables: usize,
    pub strings: usize,
    pub images: usize,
    pub audio: usize,
    /// (language tag, translated entries, unmatched entries).
    pub translations: Vec<(String, usize, usize)>,
    /// Statements with no equivalent (skipped or kept as a commented script).
    pub unsupported: Vec<Issue>,
    /// Approximations (a transition replaced by a dissolve, ...).
    pub warnings: Vec<Issue>,
    /// Files not imported (format the engine does not read, `gui/`...).
    pub skipped_files: Vec<String>,
    /// Validation of the generated story.
    pub validation_errors: Vec<String>,
    pub validation_warnings: usize,
}

impl ImportReport {
    /// One line for a toast / the console.
    pub fn summary(&self) -> String {
        format!(
            "{} scenes, {} characters, {} images, {} sounds, {} variables, {} unsupported, {} warnings",
            self.scenes,
            self.characters,
            self.images,
            self.audio,
            self.variables,
            self.unsupported.len(),
            self.warnings.len()
        )
    }

    /// The report as Markdown (`import-report.md`).
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# Ren'Py import report\n");
        let _ = writeln!(s, "- Source: `{}`", self.source.display());
        let _ = writeln!(s, "- Project: `{}`\n", self.project.display());
        let _ = writeln!(s, "## Counts\n");
        let _ = writeln!(s, "| What | Count |\n|---|---|");
        for (what, n) in [
            ("Scripts (.rpy)", self.scripts),
            ("Scenes (top-level labels)", self.scenes),
            ("Labels", self.labels),
            ("Nodes", self.nodes),
            ("Characters", self.characters),
            ("Variables", self.variables),
            ("Texts", self.strings),
            ("Images", self.images),
            ("Audio files", self.audio),
        ] {
            let _ = writeln!(s, "| {what} | {n} |");
        }
        if !self.translations.is_empty() {
            let _ = writeln!(s, "\n## Translations\n");
            for (lang, ok, missing) in &self.translations {
                let _ = writeln!(
                    s,
                    "- `{lang}`: {ok} texts translated, {missing} not matched"
                );
            }
        }
        let _ = writeln!(s, "\n## Validation\n");
        let _ = writeln!(
            s,
            "{} errors, {} warnings.",
            self.validation_errors.len(),
            self.validation_warnings
        );
        for e in &self.validation_errors {
            let _ = writeln!(s, "- {e}");
        }
        let sections = [
            ("Unsupported statements", &self.unsupported),
            ("Warnings", &self.warnings),
        ];
        for (title, issues) in sections {
            let _ = writeln!(s, "\n## {title} ({})\n", issues.len());
            if issues.is_empty() {
                let _ = writeln!(s, "None.");
            }
            for i in issues {
                let _ = writeln!(s, "- {i}");
            }
        }
        if !self.skipped_files.is_empty() {
            let _ = writeln!(
                s,
                "\n## Files not imported ({})\n",
                self.skipped_files.len()
            );
            for f in &self.skipped_files {
                let _ = writeln!(s, "- `{f}`");
            }
        }
        s
    }
}

/// The game folder of a Ren'Py project: `<dir>/game` when it exists, else
/// `dir` itself.
pub fn game_dir(dir: &Path) -> PathBuf {
    let game = dir.join("game");
    if game.is_dir() { game } else { dir.to_owned() }
}

const IMAGE_EXT: [&str; 3] = ["png", "jpg", "jpeg"];
const AUDIO_EXT: [&str; 2] = ["ogg", "wav"];
const UNSUPPORTED_MEDIA: [&str; 9] = [
    "webp", "avif", "gif", "bmp", "mp3", "opus", "flac", "m4a", "webm",
];

/// Everything read from the game folder.
struct Sources {
    scripts: Vec<Script>,
    media: Vec<(String, MediaKind)>,
    /// language -> its `.rpy` sources.
    tl: Vec<(String, Vec<String>)>,
    skipped: Vec<String>,
}

fn scan(game: &Path) -> Result<Sources, String> {
    let mut out = Sources {
        scripts: Vec::new(),
        media: Vec::new(),
        tl: Vec::new(),
        skipped: Vec::new(),
    };
    let mut files = Vec::new();
    walk(game, game, &mut files).map_err(|e| format!("{}: {e}", game.display()))?;
    files.sort();
    let mut gui_files = 0;
    for rel in files {
        let ext = rel
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .unwrap_or_default();
        let top = rel.split('/').next().unwrap_or("");
        if matches!(top, "cache" | "saves" | "python-packages") {
            continue;
        }
        if top == "tl" {
            let mut parts = rel.split('/');
            let lang = parts.nth(1).unwrap_or("");
            if ext == "rpy" && lang != "None" && !lang.is_empty() {
                let source = read(game, &rel)?;
                match out.tl.iter_mut().find(|(l, _)| l == lang) {
                    Some((_, v)) => v.push(source),
                    None => out.tl.push((lang.to_owned(), vec![source])),
                }
            }
            continue;
        }
        if top == "gui" {
            gui_files += 1;
            continue;
        }
        if ext == "rpy" {
            out.scripts.push(Script {
                source: read(game, &rel)?,
                path: rel,
            });
        } else if IMAGE_EXT.contains(&ext.as_str()) {
            out.media.push((rel, MediaKind::Image));
        } else if AUDIO_EXT.contains(&ext.as_str()) {
            out.media.push((rel, MediaKind::Audio));
        } else if UNSUPPORTED_MEDIA.contains(&ext.as_str()) {
            out.skipped.push(format!(
                "{rel} (format not supported: convert it to PNG / OGG)"
            ));
        }
    }
    if gui_files > 0 {
        out.skipped.push(format!(
            "gui/ ({gui_files} files: the Ren'Py interface is not imported, OrbitEngine screens are made in the UI editor)"
        ));
    }
    Ok(out)
}

fn read(game: &Path, rel: &str) -> Result<String, String> {
    let bytes = fs::read(game.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(rel);
        }
    }
    Ok(())
}

/// Replaces nodes the validation refuses (an expression of the wrong type,
/// a script that does not compile) by commented scripts, so that the
/// project always opens. Returns how many were replaced.
fn repair(story: &mut Story, report: &mut ImportReport) {
    for _ in 0..4 {
        let errors = validate(story).errors;
        let mut fixed = false;
        for e in errors {
            let node = match &e {
                ValidationError::ExprType { node, .. }
                | ValidationError::InvalidScript { node, .. } => *node,
                _ => continue,
            };
            let Some(n) = story.node_mut(node) else {
                continue;
            };
            let replacement = match &n.kind {
                NodeKind::If { then_target, .. } => {
                    // Keep the branch; the condition is left for the author.
                    Some(NodeKind::If {
                        condition: Expr::lit(false),
                        then_target: *then_target,
                        else_target: None,
                    })
                }
                NodeKind::Choice { options } => {
                    let mut options = options.clone();
                    options.iter_mut().for_each(|o| o.condition = None);
                    Some(NodeKind::Choice { options })
                }
                NodeKind::Script { source, .. }
                    if !source.starts_with("// Ren'Py code, not converted") =>
                {
                    Some(NodeKind::Script {
                        source: format!(
                            "// Ren'Py code, not converted:\n// {}\n",
                            source.replace('\n', "\n// ")
                        ),
                        branches: Vec::new(),
                        on_error: None,
                    })
                }
                NodeKind::SetVar { var, value } => Some(NodeKind::Script {
                    source: format!("// Ren'Py code, not converted:\n// {var} = {value}\n"),
                    branches: Vec::new(),
                    on_error: None,
                }),
                _ => None,
            };
            if let Some(kind) = replacement {
                report.warnings.push(Issue {
                    file: String::new(),
                    line: 0,
                    message: format!("node {node} disabled, it did not validate: {e}"),
                });
                n.kind = kind;
                fixed = true;
            }
        }
        if !fixed {
            break;
        }
    }
}

/// Converts the Ren'Py game in `source` (a Ren'Py project folder or its
/// `game/` folder) into a new project folder `out` (created; it must be
/// empty or absent). `progress(fraction, label)` is called along the way.
pub fn import(
    source: &Path,
    out: &Path,
    progress: &dyn Fn(f32, &str),
) -> Result<ImportReport, String> {
    let game = game_dir(source);
    if !game.is_dir() {
        return Err(format!("{}: not a folder", source.display()));
    }
    if out.exists() && out.read_dir().map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{}: the folder is not empty", out.display()));
    }
    progress(0.05, "reading the Ren'Py game");
    let sources = scan(&game)?;
    if sources.scripts.is_empty() {
        return Err(format!("{}: no .rpy script found", game.display()));
    }
    progress(0.2, "converting the scripts");
    let title = source.file_name().map_or_else(
        || "Ren'Py import".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let title = if title == "game" {
        source
            .parent()
            .and_then(Path::file_name)
            .map_or(title, |n| n.to_string_lossy().into_owned())
    } else {
        title
    };
    let converted =
        convert::Conv::new(&title, &sources.scripts, &sources.media, Some(game.clone())).run();
    let mut report = ImportReport {
        source: source.to_owned(),
        project: out.to_owned(),
        scripts: sources.scripts.len(),
        labels: converted.labels,
        unsupported: converted.unsupported,
        warnings: converted.warnings,
        skipped_files: sources.skipped,
        ..ImportReport::default()
    };
    let mut story = converted.story;
    repair(&mut story, &mut report);

    progress(0.4, "copying the assets");
    fs::create_dir_all(out.join("assets")).map_err(|e| format!("{}: {e}", out.display()))?;
    let total = converted.copies.len().max(1);
    for (i, (from, to)) in converted.copies.iter().enumerate() {
        let dest = out.join(to);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::copy(game.join(from), &dest).map_err(|e| format!("{from}: {e}"))?;
        if i % 16 == 0 {
            progress(0.4 + 0.4 * (i as f32 / total as f32), "copying the assets");
        }
    }
    for (path, bytes) in &converted.generated {
        fs::write(out.join(path), bytes).map_err(|e| format!("{path}: {e}"))?;
    }

    progress(0.85, "writing the project");
    let manifest = ProjectManifest::new(story.title.clone());
    let io = |p: &Path| {
        let p = p.display().to_string();
        move |e: std::io::Error| format!("{p}: {e}")
    };
    fs::write(
        out.join(orbit_project::MANIFEST_FILE),
        manifest.to_ron().map_err(|e| e.to_string())?,
    )
    .map_err(io(out))?;
    let story_path = out.join(DEFAULT_STORY_FILE);
    fs::write(&story_path, story.to_ron().map_err(|e| e.to_string())?).map_err(io(&story_path))?;
    fs::write(out.join(".gitignore"), format!("{STATE_DIR}/\n")).map_err(io(out))?;

    for (lang, files) in &sources.tl {
        let r = tl::translate(lang, files, &converted.text_index, &converted.wrappers);
        if r.translation.entries.is_empty() {
            continue;
        }
        let path = orbit_i18n::files::translation_path(&story_path, &r.translation.lang);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        orbit_i18n::files::save(&path, &r.translation)?;
        report
            .translations
            .push((r.translation.lang.clone(), r.matched, r.unmatched));
    }

    let check = validate(&story);
    report.validation_errors = check.errors.iter().map(ToString::to_string).collect();
    report.validation_warnings = check.warnings.len();
    report.scenes = story.scenes.len();
    report.nodes = story.scenes.iter().map(|s| s.nodes.len()).sum();
    report.characters = story.characters.len();
    report.variables = story.variables.len();
    report.strings = story.strings.entries.len();
    report.images = story
        .assets
        .assets
        .iter()
        .filter(|a| a.kind == orbit_core::AssetKind::Image)
        .count();
    report.audio = story.assets.assets.len() - report.images;
    fs::write(out.join(REPORT_FILE), report.to_markdown()).map_err(io(out))?;
    progress(1.0, "done");
    Ok(report)
}

/// The story an import would produce, without writing anything (tests,
/// previews): `scripts` are `(path, source)` pairs, `media` the media files
/// of the game folder.
pub fn convert_scripts(
    title: &str,
    scripts: &[(&str, &str)],
    media: &[(&str, MediaKind)],
) -> (Story, ImportReport) {
    let scripts: Vec<Script> = scripts
        .iter()
        .map(|(p, s)| Script {
            path: (*p).to_owned(),
            source: (*s).to_owned(),
        })
        .collect();
    let media: Vec<(String, MediaKind)> =
        media.iter().map(|(p, k)| ((*p).to_owned(), *k)).collect();
    let converted = convert::Conv::new(title, &scripts, &media, None).run();
    let mut report = ImportReport {
        scripts: scripts.len(),
        labels: converted.labels,
        unsupported: converted.unsupported,
        warnings: converted.warnings,
        ..ImportReport::default()
    };
    let mut story = converted.story;
    repair(&mut story, &mut report);
    let check = validate(&story);
    report.validation_errors = check.errors.iter().map(ToString::to_string).collect();
    report.validation_warnings = check.warnings.len();
    report.scenes = story.scenes.len();
    (story, report)
}
