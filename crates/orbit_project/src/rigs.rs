//! Rig assets of a project (ADR 0013, M5B-07): listing the `.rig.ron` files,
//! summarising them and checking what the rigged expressions of the story
//! need from them (the file, the clip, the skin, the pictures it draws).
//!
//! The story alone cannot check these (the clip and skin names live in the
//! rig file), so every tool that validates with the project's files (the
//! `orbit validate --check-files` command, the editor's Problems panel, the
//! MCP `validate` / `rig_*` tools) goes through here. Rig problems are
//! warnings: a missing visual resource never stops the game, the poster
//! image is drawn instead.

use std::path::Path;

use orbit_anim::rig::{PreparedRig, Rig, Severity, load_rig};
use orbit_core::{AssetId, AssetKind, CharacterId, Story};

/// Where new rig files go (the editor and the MCP server agree on it).
pub const RIG_DIR: &str = "assets/rigs";

/// Reads and parses a `.rig.ron` of the project (`path` relative to `root`).
pub fn read_rig(root: &Path, path: &str) -> Result<Rig, String> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.split('/').any(|p| p == "..") {
        return Err(format!("`{path}` is not a project path"));
    }
    let text = std::fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    load_rig(&text).map(|l| l.rig).map_err(|e| e.to_string())
}

/// What is wrong with a rigged expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RigProblemKind {
    /// The rig file is missing or does not parse / validate.
    Unreadable(String),
    /// The rig has no clip of that name.
    UnknownClip(String),
    /// The rig has no skin of that name.
    UnknownSkin(String),
    /// A picture the rig draws is not a file of the project.
    MissingImage(String),
    /// Budget warning of the rig (bones, vertices, ...), already worded.
    Budget(String),
}

/// A problem found in a rigged expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RigProblem {
    pub character: CharacterId,
    pub expression: String,
    /// Manifest path of the rig.
    pub rig: String,
    pub kind: RigProblemKind,
}

impl std::fmt::Display for RigProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match &self.kind {
            RigProblemKind::Unreadable(e) => format!("the rig cannot be used: {e}"),
            RigProblemKind::UnknownClip(c) => format!("the rig has no clip `{c}`"),
            RigProblemKind::UnknownSkin(s) => format!("the rig has no skin `{s}`"),
            RigProblemKind::MissingImage(p) => format!("the rig draws `{p}`, which is not a file of the project"),
            RigProblemKind::Budget(m) => m.clone(),
        };
        write!(
            f,
            "character {}, expression `{}`, rig `{}`: {what} (the still image is used)",
            self.character, self.expression, self.rig
        )
    }
}

/// The pictures a rig draws (paths of the project, no duplicates).
pub fn rig_images(rig: &Rig) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for skin in &rig.skins {
        for a in skin.attachments.values() {
            let p = a.image();
            if !out.iter().any(|x| x == p) {
                out.push(p.to_owned());
            }
        }
    }
    out
}

/// A file's length and modification time.
type FileStamp = Option<(u64, Option<std::time::SystemTime>)>;

/// Rig files read (and prepared) by [`rig_problems_cached`], kept between
/// runs: a rig is read again only when its file changed. A big rig takes
/// a while to parse and prepare, and the live validation runs after every
/// pause in the edits.
#[derive(Default)]
pub struct RigFileCache {
    files: std::collections::HashMap<String, (FileStamp, Result<(Rig, Vec<String>), String>)>,
}

impl RigFileCache {
    fn load(&mut self, root: &Path, path: &str) -> &Result<(Rig, Vec<String>), String> {
        let stamp: FileStamp = std::fs::metadata(root.join(path))
            .ok()
            .map(|m| (m.len(), m.modified().ok()));
        let fresh = matches!(self.files.get(path), Some((s, _)) if stamp.is_some() && *s == stamp);
        if !fresh {
            let loaded = read_rig(root, path).and_then(|rig| {
                let warnings = match PreparedRig::new(rig.clone()) {
                    Ok(p) => p.warnings.iter().map(|w| w.to_string()).collect(),
                    Err(issues) => {
                        return Err(issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; "));
                    }
                };
                Ok((rig, warnings))
            });
            self.files.insert(path.to_owned(), (stamp, loaded));
        }
        &self.files[path].1
    }
}

/// Checks every rigged expression of `story` against the rig files under
/// `root`. Each rig file is read once.
pub fn rig_problems(root: &Path, story: &Story) -> Vec<RigProblem> {
    rig_problems_cached(root, story, &mut RigFileCache::default())
}

/// [`rig_problems`], reusing the rigs `files` holds when their file did not
/// change.
pub fn rig_problems_cached(root: &Path, story: &Story, files: &mut RigFileCache) -> Vec<RigProblem> {
    // Pictures each rig draws that are not files, looked up once per rig.
    let mut missing_images: std::collections::HashMap<AssetId, Vec<String>> = Default::default();
    let mut out = Vec::new();
    for c in &story.characters {
        for e in &c.expressions {
            let Some(m) = e.rigged() else { continue };
            let Some(entry) = story.assets.get(m.rig).filter(|a| a.kind == AssetKind::Rig) else {
                continue; // core validation reports the asset
            };
            let problem = |kind| RigProblem {
                character: c.id,
                expression: e.name.clone(),
                rig: entry.path.clone(),
                kind,
            };
            match files.load(root, &entry.path) {
                Err(e) => out.push(problem(RigProblemKind::Unreadable(e.clone()))),
                Ok((rig, _)) => {
                    if !rig.clips.contains_key(&m.clip) {
                        out.push(problem(RigProblemKind::UnknownClip(m.clip.clone())));
                    }
                    if let Some(skin) = &m.skin
                        && !rig.skins.iter().any(|s| &s.name == skin)
                    {
                        out.push(problem(RigProblemKind::UnknownSkin(skin.clone())));
                    }
                    let missing = missing_images.entry(m.rig).or_insert_with(|| {
                        rig_images(rig)
                            .into_iter()
                            .filter(|image| !root.join(image).is_file())
                            .collect()
                    });
                    for image in missing.iter() {
                        out.push(problem(RigProblemKind::MissingImage(image.clone())));
                    }
                }
            }
        }
    }
    // Budget warnings are reported once per rig, not per expression.
    let mut seen: Vec<AssetId> = Vec::new();
    for c in &story.characters {
        for e in &c.expressions {
            let Some(m) = e.rigged() else { continue };
            if seen.contains(&m.rig) {
                continue;
            }
            seen.push(m.rig);
            let Some(entry) = story.assets.get(m.rig).filter(|a| a.kind == AssetKind::Rig) else { continue };
            if let Some((_, Ok((_, warnings)))) = files.files.get(&entry.path) {
                for w in warnings {
                    out.push(RigProblem {
                        character: c.id,
                        expression: e.name.clone(),
                        rig: entry.path.clone(),
                        kind: RigProblemKind::Budget(w.clone()),
                    });
                }
            }
        }
    }
    out
}

/// A rig file in a project.
#[derive(Debug, Clone, PartialEq)]
pub struct RigSummary {
    /// The declaring asset, `None` for a `.rig.ron` file nothing declares.
    pub asset: Option<AssetId>,
    pub path: String,
    /// `Err` with the reason when the file cannot be used.
    pub content: Result<RigContent, String>,
}

/// What a usable rig contains.
#[derive(Debug, Clone, PartialEq)]
pub struct RigContent {
    pub bones: Vec<String>,
    pub slots: Vec<String>,
    pub skins: Vec<String>,
    /// `(name, duration in seconds)`.
    pub clips: Vec<(String, f32)>,
    pub images: Vec<String>,
    /// Pictures that are not files of the project.
    pub missing_images: Vec<String>,
    /// Number of IK constraints.
    pub ik: usize,
    /// Worded issues of the rig (errors first, then warnings).
    pub issues: Vec<String>,
}

fn summarize(root: &Path, asset: Option<AssetId>, path: &str) -> RigSummary {
    let content = read_rig(root, path).map(|rig| {
        let images = rig_images(&rig);
        let missing_images = images.iter().filter(|i| !root.join(i).is_file()).cloned().collect();
        let mut issues: Vec<(Severity, String)> =
            rig.validate().into_iter().map(|i| (i.severity, i.to_string())).collect();
        issues.sort_by_key(|(s, _)| *s != Severity::Error);
        RigContent {
            bones: rig.bones.iter().map(|b| b.name.clone()).collect(),
            slots: rig.slots.iter().map(|s| s.name.clone()).collect(),
            skins: rig.skins.iter().map(|s| s.name.clone()).collect(),
            clips: rig.clips.iter().map(|(n, c)| (n.clone(), c.duration)).collect(),
            images,
            missing_images,
            ik: rig.ik.len(),
            issues: issues.into_iter().map(|(_, m)| m).collect(),
        }
    });
    RigSummary { asset, path: path.to_owned(), content }
}

/// The rig assets the story declares, then the `.rig.ron` files of
/// `assets/rigs` that nothing declares.
pub fn list_rigs(root: &Path, story: &Story) -> Vec<RigSummary> {
    let mut out: Vec<RigSummary> = story
        .assets
        .assets
        .iter()
        .filter(|a| a.kind == AssetKind::Rig)
        .map(|a| summarize(root, Some(a.id), &a.path))
        .collect();
    if let Ok(dir) = std::fs::read_dir(root.join(RIG_DIR)) {
        let mut found: Vec<String> = dir
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".rig.ron"))
            .map(|n| format!("{RIG_DIR}/{n}"))
            .filter(|p| !out.iter().any(|s| &s.path == p))
            .collect();
        found.sort();
        out.extend(found.iter().map(|p| summarize(root, None, p)));
    }
    out
}

#[cfg(test)]
mod tests;
