//! MCP tools for achievements (M7-05) and voice acting (M7-06).
//!
//! Everything goes through `orbit_project` (undoable commands): achievements
//! are story data (`Story::achievements`, unlocked by `Unlock` nodes, see
//! `add_command type=unlock`), voices are `Say::voice` sound assets, with
//! per-language overrides stored in the translation files.

use orbit_core::{AchievementId, AssetKind, NodeId, NodeKind, Story};
use orbit_project::achievements as ach;
use orbit_project::voices::{VOICE_EXTENSIONS, apply_voice_plan, plan_voice_import, set_say_voice};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::convert::{find_asset, parse_id};
use crate::error::ToolError;
use crate::tools::ToolCx;

fn out(value: Json) -> Result<String, ToolError> {
    Ok(serde_json::to_string_pretty(&value).unwrap_or_default())
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// An achievement by id, or by its exact title.
fn find_achievement(story: &Story, key: &str) -> Result<AchievementId, ToolError> {
    let key = key.trim();
    if let Ok(id) = key.parse::<AchievementId>()
        && story.achievements.iter().any(|a| a.id == id)
    {
        return Ok(id);
    }
    let matches: Vec<_> = story
        .achievements
        .iter()
        .filter(|a| story.text(a.title_id) == Some(key))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.id),
        [] => Err(ToolError::not_found(format!(
            "no achievement `{key}` (see list_achievements)"
        ))),
        _ => Err(ToolError::invalid(format!(
            "several achievements are titled `{key}`: use their id"
        ))),
    }
}

/// Achievement id from the `unlock` command's argument (id or title).
pub(crate) fn achievement_arg(story: &Story, key: &str) -> Result<AchievementId, ToolError> {
    find_achievement(story, key)
}

/// `list_achievements`.
pub fn list_achievements(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let story = cx.project.story();
    let achievements: Vec<Json> = story
        .achievements
        .iter()
        .map(|a| {
            json!({
                "id": a.id.to_string(),
                "title": story.text(a.title_id),
                "description": story.text(a.description_id),
                "hidden": a.hidden,
                "icon": a.icon.and_then(|i| story.asset_path(i)),
                "unlocked_by": ach::unlock_nodes(story, a.id).iter().map(ToString::to_string).collect::<Vec<_>>(),
            })
        })
        .collect();
    let (mut lines, mut voiced) = (0usize, 0usize);
    for n in story.scenes.iter().flat_map(|s| &s.nodes) {
        if let NodeKind::Say { voice, .. } = &n.kind {
            lines += 1;
            voiced += usize::from(voice.is_some());
        }
    }
    out(json!({
        "achievements": achievements,
        "voice": { "lines": lines, "voiced": voiced },
    }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAchievementArgs {
    /// Id or exact title of the achievement to edit; omit to create one.
    achievement: Option<String>,
    /// Title (required to create).
    title: Option<String>,
    /// Description.
    description: Option<String>,
    /// Hidden (placeholder text) until unlocked.
    hidden: Option<bool>,
    /// Icon: an image asset (path, name or id); an empty text removes it.
    icon: Option<String>,
}

/// `set_achievement`: create or edit.
pub fn set_achievement(cx: &mut ToolCx<'_>, a: SetAchievementArgs) -> Result<String, ToolError> {
    let story = cx.project.story();
    let icon = match a.icon.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(key) => Some(Some(find_asset(story, key, Some(AssetKind::Image))?)),
    };
    let id = match &a.achievement {
        Some(key) => {
            let id = find_achievement(story, key)?;
            ach::update_achievement(
                cx.project,
                id,
                a.title.as_deref(),
                a.description.as_deref(),
                a.hidden,
                icon,
            )?;
            id
        }
        None => {
            let title = a
                .title
                .as_deref()
                .ok_or_else(|| ToolError::invalid("`title` is required to create an achievement"))?;
            ach::add_achievement(
                cx.project,
                title,
                a.description.as_deref().unwrap_or_default(),
                a.hidden.unwrap_or(false),
                icon.flatten(),
            )?
        }
    };
    list_one(cx, id)
}

fn list_one(cx: &ToolCx<'_>, id: AchievementId) -> Result<String, ToolError> {
    let story = cx.project.story();
    let a = story
        .achievements
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| ToolError::not_found(format!("no achievement {id}")))?;
    out(json!({
        "id": a.id.to_string(),
        "title": story.text(a.title_id),
        "description": story.text(a.description_id),
        "hidden": a.hidden,
        "icon": a.icon.and_then(|i| story.asset_path(i)),
    }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AchievementArg {
    /// Id or exact title of the achievement.
    achievement: String,
}

/// `delete_achievement`: refused while an unlock node names it.
pub fn delete_achievement(cx: &mut ToolCx<'_>, a: AchievementArg) -> Result<String, ToolError> {
    let id = find_achievement(cx.project.story(), &a.achievement)?;
    ach::delete_achievement(cx.project, id)?;
    out(json!({ "deleted": id.to_string() }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetVoiceArgs {
    /// Id of a dialogue (`say`) node.
    node: String,
    /// Project-relative path of the voice file (a sound; declared in the
    /// manifest if needed, the file must exist under the project). Omit to
    /// remove the voice.
    voice: Option<String>,
}

impl SetVoiceArgs {
    pub(crate) fn new(node: String, voice: String) -> Self {
        Self {
            node,
            voice: Some(voice),
        }
    }
}

/// `set_voice`.
pub fn set_voice(cx: &mut ToolCx<'_>, a: SetVoiceArgs) -> Result<String, ToolError> {
    let node: NodeId = parse_id(&a.node, "node")?;
    if let Some(path) = a.voice.as_deref() {
        let abs = cx
            .sandbox
            .resolve(path)
            .map_err(|e| ToolError::invalid(e.to_string()))?;
        if !abs.is_file() {
            return Err(ToolError::not_found(format!("no file `{path}` in the project")));
        }
    }
    set_say_voice(cx.project, node, a.voice.as_deref().map(|p| p.trim()))?;
    let story = cx.project.story();
    let voice = match story.node(node).map(|n| &n.kind) {
        Some(NodeKind::Say { voice: Some(v), .. }) => story.asset_path(*v).map(str::to_owned),
        _ => None,
    };
    out(json!({ "node": node.to_string(), "voice": voice }))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportVoicesArgs {
    /// Project-relative folder holding the voice files, named after the
    /// lines: `<node id or text id>.ogg` (.wav, .mp3, .flac too); a folder
    /// named after a language (`fr/<id>.ogg`) gives the voice of that
    /// translation.
    folder: Option<String>,
    /// Project-relative CSV (`line,file[,language]`, optional header); files
    /// are project-relative paths.
    csv: Option<String>,
    /// Only report what would be imported (default false).
    dry_run: Option<bool>,
}

fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| VOICE_EXTENSIONS.contains(&x.to_ascii_lowercase().as_str()))
            && let Ok(rel) = p.strip_prefix(base)
        {
            out.push((p.clone(), rel.to_string_lossy().replace('\\', "/")));
        }
    }
}

/// `import_voices`: batch import by naming convention and/or CSV.
pub fn import_voices(cx: &mut ToolCx<'_>, a: ImportVoicesArgs) -> Result<String, ToolError> {
    if a.folder.is_none() && a.csv.is_none() {
        return Err(ToolError::invalid("give `folder` and/or `csv`"));
    }
    let mut names = Vec::new();
    if let Some(folder) = &a.folder {
        let dir = cx
            .sandbox
            .resolve(folder)
            .map_err(|e| ToolError::invalid(e.to_string()))?;
        if !dir.is_dir() {
            return Err(ToolError::not_found(format!("no folder `{folder}`")));
        }
        let mut files = Vec::new();
        walk(&dir, &dir, &mut files);
        // The convention reads the path relative to the folder (language
        // sub-folders); the manifest path is relative to the project.
        for (abs, rel) in files {
            let project = cx.sandbox.relative(&abs).unwrap_or(rel);
            names.push(project);
        }
    }
    let csv = match &a.csv {
        Some(path) => {
            let abs = cx
                .sandbox
                .resolve(path)
                .map_err(|e| ToolError::invalid(e.to_string()))?;
            Some(std::fs::read_to_string(&abs).map_err(|e| {
                ToolError::new(crate::ErrorKind::Io, format!("cannot read `{path}`: {e}"))
            })?)
        }
        None => None,
    };
    let plan = plan_voice_import(cx.project.story(), &names, csv.as_deref());
    let summary = |plan: &orbit_project::voices::VoicePlan| {
        json!({
            "matches": plan.matches.iter().map(|m| json!({
                "node": m.node.to_string(),
                "file": m.path,
                "language": m.language,
            })).collect::<Vec<_>>(),
            "unmatched": plan.unmatched,
        })
    };
    if a.dry_run.unwrap_or(false) {
        let mut j = summary(&plan);
        j["dry_run"] = json!(true);
        return out(j);
    }
    let report = apply_voice_plan(cx.project, &plan)?;
    let story_path = cx.sandbox.root().join(&cx.manifest.story);
    let mut stored = Vec::new();
    let mut missing = Vec::new();
    for (lang, voices) in &report.translated {
        match orbit_i18n::files::merge_voices(&story_path, lang, voices) {
            Ok(true) => stored.push(lang.clone()),
            Ok(false) => missing.push(lang.clone()),
            Err(e) => return Err(ToolError::new(crate::ErrorKind::Io, e)),
        }
    }
    let mut j = summary(&plan);
    j["source_voices"] = json!(report.source_voices);
    j["translations_updated"] = json!(stored);
    j["languages_without_file"] = json!(missing);
    out(j)
}
