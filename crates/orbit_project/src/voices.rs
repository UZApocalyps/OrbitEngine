//! Voice acting of a project (M7-06): the voice of a line, and the batch
//! import by naming convention.
//!
//! **Convention.** A voice file is named after the line it voices:
//! `<line-id>.ogg` (also `.wav`, `.mp3`, `.flac`), where the line id is the
//! id of the `Say` node or the id of its text (both are ULIDs shown by the
//! editor and `orbit` tools). A folder named after a language
//! (`voice/fr/<line-id>.ogg`) makes the file the voice of that language; the
//! source language of the story (or no language folder) sets the voice of
//! the node itself. A CSV file (`line,path[,language]`, one row per line,
//! optional header) says the same without renaming files.

use std::collections::BTreeMap;

use orbit_core::{AssetId, AssetKind, NodeId, NodeKind, Story, StringId};

use crate::ops::{is_valid_asset_path, reject_with};
use crate::project::Transaction;
use crate::{Command, Project, ProjectError};

/// Extensions of the audio files the importer takes.
pub const VOICE_EXTENSIONS: [&str; 4] = ["ogg", "wav", "mp3", "flac"];

/// Registers `path` as a sound of the manifest (an existing entry is reused)
/// inside a transaction.
pub(crate) fn ensure_sound(tx: &mut Transaction<'_>, path: &str) -> Result<AssetId, ProjectError> {
    if let Some(existing) = tx
        .project()
        .story()
        .assets
        .assets
        .iter()
        .find(|a| a.path == path)
    {
        return if existing.kind == AssetKind::Sound {
            Ok(existing.id)
        } else {
            Err(reject_with(
                "asset_other_type",
                &[("path", path.to_string())],
                format!("« {path} » est déjà déclaré d'un autre type."),
            ))
        };
    }
    if !is_valid_asset_path(path) {
        return Err(reject_with(
            "asset_bad_path",
            &[("path", path.to_string())],
            format!("« {path} » n'est pas un chemin de ressource valide."),
        ));
    }
    let id = AssetId::new();
    let index = tx.project().story().assets.assets.len();
    tx.execute(Command::SetAsset {
        id,
        entry: Some(orbit_core::AssetEntry {
            id,
            kind: AssetKind::Sound,
            path: path.to_owned(),
        }),
        index: Some(index),
    })?;
    Ok(id)
}

/// Registers the sound at `path` (if needed) and makes it the voice of the
/// `Say` node `node`; `None` removes the voice.
pub fn set_say_voice(
    project: &mut Project,
    node: NodeId,
    path: Option<&str>,
) -> Result<(), ProjectError> {
    let (speaker, text_id) = match project.story().node(node).map(|n| &n.kind) {
        Some(NodeKind::Say {
            speaker, text_id, ..
        }) => (*speaker, *text_id),
        Some(_) => return Err(crate::CommandError::NotASay(node).into()),
        None => return Err(crate::CommandError::UnknownNode(node).into()),
    };
    project.transaction_labeled("Modifier la voix", |tx| {
        let voice = match path {
            Some(p) => Some(ensure_sound(tx, p)?),
            None => None,
        };
        tx.execute(Command::SetNodeKind {
            id: node,
            kind: NodeKind::Say {
                speaker,
                text_id,
                voice,
            },
            expected_version: None,
        })
    })
}

/// One voice file matched to a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceMatch {
    /// The `Say` node, for the source language.
    pub node: NodeId,
    /// The text of that node (the key of a translation's voices).
    pub text_id: StringId,
    /// Manifest path of the file.
    pub path: String,
    /// `None`: the voice of the node; `Some(lang)`: the voice of a
    /// translation (the tag as written in the folder name or the CSV).
    pub language: Option<String>,
}

/// What an import would do (see [`plan_voice_import`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VoicePlan {
    pub matches: Vec<VoiceMatch>,
    /// Files that name no line of the story (or are not audio).
    pub unmatched: Vec<String>,
}

/// What an import did (see [`apply_voice_plan`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VoiceReport {
    /// Lines whose own voice was set.
    pub source_voices: usize,
    /// Voices of translations, to be stored in their language files
    /// (`Translation::voices`): language tag, then text id.
    pub translated: BTreeMap<String, BTreeMap<StringId, AssetId>>,
}

fn is_language_tag(s: &str) -> bool {
    let mut parts = s.split(['-', '_']);
    let first = parts.next().unwrap_or("");
    (2..=3).contains(&first.len())
        && first.chars().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// The line a (stem of a) file name or a CSV key designates: a `Say` node
/// id, else the text id of the first `Say` node showing it.
fn find_line(story: &Story, key: &str) -> Option<(NodeId, StringId)> {
    let key = key.trim();
    if let Ok(node) = key.parse::<NodeId>()
        && let Some(NodeKind::Say { text_id, .. }) = story.node(node).map(|n| &n.kind)
    {
        return Some((node, *text_id));
    }
    let text: StringId = key.parse().ok()?;
    story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .find_map(|n| match &n.kind {
            NodeKind::Say { text_id, .. } if *text_id == text => Some((n.id, *text_id)),
            _ => None,
        })
}

/// Matches voice files to the lines of the story by the naming convention of
/// this module and, when given, a CSV mapping (`line,path[,language]`). The
/// paths are those of the project (forward slashes, relative to the assets
/// root). Pure: nothing is changed (see [`apply_voice_plan`]).
pub fn plan_voice_import(story: &Story, files: &[String], csv: Option<&str>) -> VoicePlan {
    let mut plan = VoicePlan::default();
    let source_lang = story.strings.source_lang.to_ascii_lowercase();
    let push = |plan: &mut VoicePlan, node, text_id, path: &str, lang: Option<&str>| {
        let language = lang
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case(&source_lang))
            .map(str::to_owned);
        let m = VoiceMatch {
            node,
            text_id,
            path: path.to_owned(),
            language,
        };
        // A later file for the same line and language replaces the earlier.
        plan.matches
            .retain(|o| !(o.node == m.node && o.language == m.language));
        plan.matches.push(m);
    };
    for file in files {
        let path = file.replace('\\', "/");
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
        let Some((stem, ext)) = name.rsplit_once('.') else {
            plan.unmatched.push(path);
            continue;
        };
        if !VOICE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            plan.unmatched.push(path);
            continue;
        }
        let Some((node, text_id)) = find_line(story, stem) else {
            plan.unmatched.push(path);
            continue;
        };
        let folder = dir.rsplit('/').next().unwrap_or("");
        let lang = is_language_tag(folder).then_some(folder);
        push(&mut plan, node, text_id, &path, lang);
    }
    if let Some(csv) = csv {
        for (i, row) in csv.lines().enumerate() {
            let row = row.trim();
            if row.is_empty() || row.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = row
                .split([',', ';', '\t'])
                .map(|c| c.trim().trim_matches('"'))
                .collect();
            let (Some(key), Some(path)) = (cols.first(), cols.get(1)) else {
                plan.unmatched.push(format!("csv:{}: {row}", i + 1));
                continue;
            };
            match find_line(story, key) {
                Some((node, text_id)) => {
                    let path = path.replace('\\', "/");
                    push(&mut plan, node, text_id, &path, cols.get(2).copied());
                }
                // A header row names no line: not a problem.
                None if i == 0 => {}
                None => plan.unmatched.push(format!("csv:{}: {row}", i + 1)),
            }
        }
    }
    plan
}

/// Applies a plan as one undo entry: registers the sounds and sets the voice
/// of every source-language line. The voices of translations are returned for
/// the caller to store in the language files.
pub fn apply_voice_plan(project: &mut Project, plan: &VoicePlan) -> Result<VoiceReport, ProjectError> {
    let mut report = VoiceReport::default();
    project.transaction_labeled("Importer les voix", |tx| {
        for m in &plan.matches {
            let voice = ensure_sound(tx, &m.path)?;
            match &m.language {
                None => {
                    let speaker = match tx.project().story().node(m.node).map(|n| &n.kind) {
                        Some(NodeKind::Say { speaker, .. }) => *speaker,
                        Some(_) => return Err(crate::CommandError::NotASay(m.node).into()),
                        None => return Err(crate::CommandError::UnknownNode(m.node).into()),
                    };
                    tx.execute(Command::SetNodeKind {
                        id: m.node,
                        kind: NodeKind::Say {
                            speaker,
                            text_id: m.text_id,
                            voice: Some(voice),
                        },
                        expected_version: None,
                    })?;
                    report.source_voices += 1;
                }
                Some(lang) => {
                    report
                        .translated
                        .entry(lang.clone())
                        .or_default()
                        .insert(m.text_id, voice);
                }
            }
        }
        Ok(())
    })?;
    Ok(report)
}
