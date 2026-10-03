//! MCP tools for the project's `ui/` folder (UI-17): screens, theme styles,
//! system-screen links and splashes, as `orbit_ui_data` types.
//!
//! The UI is not part of the story, so it has no `orbit_project` command:
//! these tools read and write the files of `<project>/ui/` directly. Every
//! write is validated first: a change that would leave an *error* in the
//! bundle is refused and nothing is written. Warnings are returned.

use std::collections::BTreeSet;
use std::path::PathBuf;

use orbit_ui_data::{
    SYSTEM_SCREENS, Severity, SplashContent, SplashDef, Style, UiBundle, from_ron, to_ron,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::error::ToolError;
use crate::tools::ToolCx;

fn ui_dir(cx: &ToolCx<'_>) -> PathBuf {
    cx.sandbox.root().join("ui")
}

fn load(cx: &ToolCx<'_>) -> Result<UiBundle, ToolError> {
    let dir = ui_dir(cx);
    if !dir.is_dir() {
        return Ok(UiBundle::default());
    }
    UiBundle::load_dir(&dir).map_err(|e| ToolError::invalid(format!("ui folder: {e}")))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn issues_json(bundle: &UiBundle) -> Vec<Json> {
    bundle
        .validate()
        .iter()
        .map(|i| {
            json!({
                "severity": if i.severity == Severity::Error { "error" } else { "warning" },
                "screen": i.screen, "widget": i.widget, "message": i.message,
            })
        })
        .collect()
}

/// The bundle's own issues plus the story cross-checks: `OpenScreen` /
/// `CloseScreen` naming a missing screen, and variable uses (a `Repeat`
/// source that is not a list or map...) given the declared types.
fn issues_json_with_story(cx: &ToolCx<'_>, bundle: &UiBundle) -> Vec<Json> {
    let story = cx.project.story();
    let ty_of = |name: &str| {
        story
            .variables
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.initial.value_type())
    };
    let mut out = issues_json(bundle);
    let extra = bundle
        .story_issues(story)
        .into_iter()
        .chain(bundle.variable_issues(&ty_of));
    out.extend(extra.map(|i| {
        json!({
            "severity": if i.severity == Severity::Error { "error" } else { "warning" },
            "screen": i.screen, "widget": i.widget, "message": i.message,
        })
    }));
    out
}

/// Validates then writes `bundle` into `ui/`; returns the warnings.
fn commit(cx: &ToolCx<'_>, bundle: &UiBundle) -> Result<Vec<Json>, ToolError> {
    let issues = issues_json(bundle);
    let errors: Vec<String> = issues
        .iter()
        .filter(|i| i["severity"] == "error")
        .map(|i| format!("{} {}: {}", i["screen"], i["widget"], i["message"]))
        .collect();
    if !errors.is_empty() {
        return Err(ToolError::invalid(format!(
            "refused, the UI would have errors (nothing written): {}",
            errors.join("; ")
        )));
    }
    for screen in &bundle.screens {
        if !valid_id(&screen.id) {
            return Err(ToolError::invalid(format!(
                "screen id `{}` must be letters, digits, `_` or `-`",
                screen.id
            )));
        }
    }
    let dir = ui_dir(cx);
    let io = |what: &str, e: std::io::Error| ToolError::invalid(format!("{what}: {e}"));
    std::fs::create_dir_all(&dir).map_err(|e| io("ui", e))?;
    let mut kept = BTreeSet::new();
    for screen in &bundle.screens {
        let name = format!("{}.screen.ron", screen.id);
        std::fs::write(dir.join(&name), to_ron(screen).map_err(ToolError::invalid)?)
            .map_err(|e| io(&name, e))?;
        kept.insert(name);
    }
    for entry in std::fs::read_dir(&dir).map_err(|e| io("ui", e))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".screen.ron") && !kept.contains(&name) {
            std::fs::remove_file(entry.path()).map_err(|e| io(&name, e))?;
        }
    }
    let theme = dir.join("theme.ron");
    match &bundle.theme {
        Some(t) => std::fs::write(
            &theme,
            t.to_ron().map_err(|e| ToolError::invalid(e.to_string()))?,
        )
        .map_err(|e| io("theme.ron", e))?,
        None => {
            let _ = std::fs::remove_file(&theme);
        }
    }
    let ui_ron = dir.join("ui.ron");
    if bundle.system_screens.is_empty() && bundle.splashes.is_empty() {
        let _ = std::fs::remove_file(&ui_ron);
    } else {
        let rest = UiBundle {
            system_screens: bundle.system_screens.clone(),
            splashes: bundle.splashes.clone(),
            ..UiBundle::default()
        };
        std::fs::write(&ui_ron, rest.to_ron().map_err(ToolError::invalid)?)
            .map_err(|e| io("ui.ron", e))?;
    }
    Ok(issues)
}

fn out(value: Json) -> Result<String, ToolError> {
    Ok(serde_json::to_string_pretty(&value).unwrap_or_default())
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenIdArg {
    /// Screen id.
    id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteScreenArgs {
    /// The screen as RON text (`(id: "x", root: (kind: Stack(dir: Vertical)))`);
    /// its `id` names the screen. Creates it or replaces the existing one.
    ron: String,
    /// Refuse to replace an existing screen (pure creation).
    create_only: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StyleNameArg {
    /// Style name.
    name: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteStyleArgs {
    /// Style name (created or replaced).
    name: String,
    /// The style as RON text (`(fill: ...)`).
    ron: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SystemScreenArgs {
    /// `main_menu`, `pause`, `preferences`, `save` or `load`.
    system: String,
    /// Screen standing in for it; omit to unlink.
    screen: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddSplashArgs {
    /// `text` or `image`.
    kind: String,
    /// The text, or the image asset path.
    value: String,
    /// Seconds on screen, fades included.
    duration: f32,
    fade_in: Option<f32>,
    fade_out: Option<f32>,
    skippable: Option<bool>,
    /// Position in the sequence (default: at the end).
    index: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexArg {
    /// Splash index (see `list_ui`).
    index: usize,
}

pub fn list_ui(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let b = load(cx)?;
    let screens: Vec<Json> = b
        .screens
        .iter()
        .map(|s| {
            let role = SYSTEM_SCREENS
                .iter()
                .find(|n| b.system_screens.get(**n).map_or(**n, String::as_str) == s.id);
            let tr = |t: &Option<orbit_ui_data::ScreenTransition>| {
                t.as_ref()
                    .map(|t| json!({"kind": format!("{:?}", t.kind), "duration": t.duration}))
            };
            json!({
                "id": s.id, "modal": s.modal, "hud": s.hud, "system_role": role,
                "transition_in": tr(&s.transition_in), "transition_out": tr(&s.transition_out),
            })
        })
        .collect();
    let splashes: Vec<Json> = b
        .splashes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (kind, value) = match &s.content {
                SplashContent::Text(t) => ("text", t),
                SplashContent::Image(t) => ("image", t),
                SplashContent::Video(t) => ("video", t),
            };
            json!({"index": i, "kind": kind, "value": value, "duration": s.duration})
        })
        .collect();
    out(json!({
        "folder": "ui",
        "has_theme": b.theme.is_some(),
        "styles": b.theme.as_ref().map(|t| t.styles.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "screens": screens,
        "system_screens": b.system_screens,
        "splashes": splashes,
        "issues": issues_json_with_story(cx, &b),
    }))
}

pub fn read_ui_screen(cx: &mut ToolCx<'_>, a: ScreenIdArg) -> Result<String, ToolError> {
    let b = load(cx)?;
    let s = b
        .screen(&a.id)
        .ok_or_else(|| ToolError::not_found(format!("no screen `{}`", a.id)))?;
    let issues: Vec<Json> = issues_json_with_story(cx, &b)
        .into_iter()
        .filter(|i| i["screen"] == a.id)
        .collect();
    out(json!({"id": a.id, "ron": to_ron(s).map_err(ToolError::invalid)?, "issues": issues}))
}

pub fn write_ui_screen(cx: &mut ToolCx<'_>, a: WriteScreenArgs) -> Result<String, ToolError> {
    let screen = from_ron(&a.ron).map_err(|e| ToolError::invalid(format!("screen RON: {e}")))?;
    let mut b = load(cx)?;
    let id = screen.id.clone();
    let existing = b.screens.iter().position(|s| s.id == id);
    if existing.is_some() && a.create_only == Some(true) {
        return Err(ToolError::invalid(format!("screen `{id}` already exists")));
    }
    match existing {
        Some(i) => b.screens[i] = screen,
        None => b.screens.push(screen),
    }
    let issues = commit(cx, &b)?;
    out(json!({"id": id, "created": existing.is_none(), "issues": issues}))
}

pub fn delete_ui_screen(cx: &mut ToolCx<'_>, a: ScreenIdArg) -> Result<String, ToolError> {
    let mut b = load(cx)?;
    let before = b.screens.len();
    b.screens.retain(|s| s.id != a.id);
    if b.screens.len() == before {
        return Err(ToolError::not_found(format!("no screen `{}`", a.id)));
    }
    b.system_screens.retain(|_, v| *v != a.id);
    let issues = commit(cx, &b)?;
    out(json!({"deleted": a.id, "issues": issues}))
}

pub fn read_ui_style(cx: &mut ToolCx<'_>, a: StyleNameArg) -> Result<String, ToolError> {
    let b = load(cx)?;
    let style = b
        .theme
        .as_ref()
        .and_then(|t| t.styles.get(&a.name))
        .ok_or_else(|| ToolError::not_found(format!("no style `{}`", a.name)))?;
    let ron = ron::ser::to_string_pretty(style, ron::ser::PrettyConfig::new())
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    out(json!({"name": a.name, "ron": ron}))
}

pub fn write_ui_style(cx: &mut ToolCx<'_>, a: WriteStyleArgs) -> Result<String, ToolError> {
    if a.name.trim().is_empty() {
        return Err(ToolError::invalid("style name must not be empty"));
    }
    let style: Style =
        ron::from_str(&a.ron).map_err(|e| ToolError::invalid(format!("style RON: {e}")))?;
    let mut b = load(cx)?;
    let theme = b.theme.get_or_insert_with(Default::default);
    let created = theme.styles.insert(a.name.clone(), style).is_none();
    let issues = commit(cx, &b)?;
    out(json!({"name": a.name, "created": created, "issues": issues}))
}

pub fn delete_ui_style(cx: &mut ToolCx<'_>, a: StyleNameArg) -> Result<String, ToolError> {
    let mut b = load(cx)?;
    let removed = b
        .theme
        .as_mut()
        .and_then(|t| t.styles.remove(&a.name))
        .is_some();
    if !removed {
        return Err(ToolError::not_found(format!("no style `{}`", a.name)));
    }
    let issues = commit(cx, &b)?;
    out(json!({"deleted": a.name, "issues": issues}))
}

pub fn set_ui_system_screen(
    cx: &mut ToolCx<'_>,
    a: SystemScreenArgs,
) -> Result<String, ToolError> {
    if !SYSTEM_SCREENS.contains(&a.system.as_str()) {
        return Err(ToolError::invalid(format!(
            "`{}` is not a system screen ({})",
            a.system,
            SYSTEM_SCREENS.join(", ")
        )));
    }
    let mut b = load(cx)?;
    match &a.screen {
        Some(id) => {
            b.system_screens.insert(a.system.clone(), id.clone());
        }
        None => {
            b.system_screens.remove(&a.system);
        }
    }
    let issues = commit(cx, &b)?;
    out(json!({"system": a.system, "screen": a.screen, "issues": issues}))
}

pub fn add_ui_splash(cx: &mut ToolCx<'_>, a: AddSplashArgs) -> Result<String, ToolError> {
    let content = match a.kind.as_str() {
        "text" => SplashContent::Text(a.value),
        "image" => SplashContent::Image(a.value),
        k => {
            return Err(ToolError::invalid(format!(
                "kind must be `text` or `image`, not `{k}`"
            )));
        }
    };
    let mut b = load(cx)?;
    let splash = SplashDef {
        content,
        duration: a.duration,
        fade_in: a.fade_in.unwrap_or(0.5),
        fade_out: a.fade_out.unwrap_or(0.5),
        skippable: a.skippable.unwrap_or(true),
        background: [0, 0, 0, 255],
    };
    let at = a.index.unwrap_or(b.splashes.len()).min(b.splashes.len());
    b.splashes.insert(at, splash);
    let issues = commit(cx, &b)?;
    out(json!({"index": at, "issues": issues}))
}

pub fn delete_ui_splash(cx: &mut ToolCx<'_>, a: IndexArg) -> Result<String, ToolError> {
    let mut b = load(cx)?;
    if a.index >= b.splashes.len() {
        return Err(ToolError::not_found(format!("no splash {}", a.index)));
    }
    b.splashes.remove(a.index);
    let issues = commit(cx, &b)?;
    out(json!({"deleted": a.index, "issues": issues}))
}
