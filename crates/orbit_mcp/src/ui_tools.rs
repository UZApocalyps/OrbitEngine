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
    let root = cx.sandbox.root().to_path_buf();
    let extra = bundle
        .story_issues(story)
        .into_iter()
        .chain(bundle.variable_issues(&ty_of))
        .chain(bundle.missing_music_issues(&|path| root.join(path).is_file()));
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
                "music": s.music.as_ref().map(|m| &m.asset),
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

// ------------------------------------------------------------ dialogue box

/// The dialogue box part of the theme (`ui_version` 8): what
/// `read_ui_dialogue` returns and `write_ui_dialogue` takes.
#[derive(serde::Serialize, Deserialize, Default)]
#[serde(default)]
struct DialogueSection {
    textbox: orbit_ui_data::TextBoxTheme,
    choice: orbit_ui_data::ChoiceTheme,
    quick_menu: orbit_ui_data::QuickMenuTheme,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteDialogueArgs {
    /// RON `(textbox: (...), choice: (...), quick_menu: (...))`; a missing
    /// section is left unchanged, missing fields of a given section take
    /// their defaults.
    ron: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WidgetStyleArgs {
    /// Screen id.
    screen: String,
    /// Id of the widget in the screen.
    widget: String,
    /// Style name of the theme; omit to remove the style.
    style: Option<String>,
}

pub fn read_ui_dialogue(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let b = load(cx)?;
    let theme = b.theme.unwrap_or_default();
    let section = DialogueSection {
        textbox: theme.textbox,
        choice: theme.choice,
        quick_menu: theme.quick_menu,
    };
    let ron = ron::ser::to_string_pretty(&section, ron::ser::PrettyConfig::new())
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    out(json!({"ron": ron}))
}

pub fn write_ui_dialogue(cx: &mut ToolCx<'_>, a: WriteDialogueArgs) -> Result<String, ToolError> {
    let has = |key: &str| a.ron.contains(&format!("{key}:"));
    let section: DialogueSection =
        ron::from_str(&a.ron).map_err(|e| ToolError::invalid(format!("dialogue RON: {e}")))?;
    let mut b = load(cx)?;
    let theme = b.theme.get_or_insert_with(Default::default);
    if has("textbox") {
        theme.textbox = section.textbox;
    }
    if has("choice") {
        theme.choice = section.choice;
    }
    if has("quick_menu") {
        theme.quick_menu = section.quick_menu;
    }
    let issues = commit(cx, &b)?;
    out(json!({"written": true, "issues": issues}))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteNotificationsArgs {
    /// RON `NotificationTheme`: `(anchor: TopRight, offset: (16.0, 16.0),
    /// width: 320.0, max_visible: 4, gap: 8.0, direction: Auto, duration: Some(4.0),
    /// animation: Slide, animation_seconds: 0.25, sound: Some("assets/sfx/ping.ogg"),
    /// click_to_dismiss: true, skip: Show | Brief | Suppress, styles: {"achievement":
    /// (background: .., icon: .., ..)})`. Missing fields take their defaults.
    ron: String,
}

pub fn read_ui_notifications(cx: &mut ToolCx<'_>, _: NoArgs) -> Result<String, ToolError> {
    let b = load(cx)?;
    let theme = b.theme.unwrap_or_default();
    let ron = ron::ser::to_string_pretty(&theme.notifications, ron::ser::PrettyConfig::new())
        .map_err(|e| ToolError::invalid(e.to_string()))?;
    out(json!({"ron": ron}))
}

pub fn write_ui_notifications(
    cx: &mut ToolCx<'_>,
    a: WriteNotificationsArgs,
) -> Result<String, ToolError> {
    let n: orbit_ui_data::NotificationTheme = ron::from_str(&a.ron)
        .map_err(|e| ToolError::invalid(format!("notifications RON: {e}")))?;
    let mut b = load(cx)?;
    b.theme.get_or_insert_with(Default::default).notifications = n;
    let issues = commit(cx, &b)?;
    out(json!({"written": true, "issues": issues}))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenMusicArgs {
    /// Screen id.
    screen: String,
    /// The music as RON, every field optional: `(asset: "assets/music/menu.ogg",
    /// looping: true, volume: 1.0, fade_in_ms: 500, fade_out_ms: 500,
    /// keep_if_same: true, resume_game_music: true, over_game: Replace | Duck,
    /// duck_volume: 0.3)`. `over_game` only matters over a running game: Replace
    /// plays the track instead of the game's music (put back on close when
    /// `resume_game_music`), Duck keeps the game's music at `duck_volume` and plays
    /// no track. Omit to remove the screen's music.
    music: Option<String>,
}

pub fn set_ui_screen_music(cx: &mut ToolCx<'_>, a: ScreenMusicArgs) -> Result<String, ToolError> {
    let music = match &a.music {
        Some(text) => Some(
            ron::from_str::<orbit_ui_data::ScreenMusic>(text)
                .map_err(|e| ToolError::invalid(format!("music RON: {e}")))?,
        ),
        None => None,
    };
    let mut b = load(cx)?;
    let screen = b
        .screens
        .iter_mut()
        .find(|s| s.id == a.screen)
        .ok_or_else(|| ToolError::not_found(format!("no screen `{}`", a.screen)))?;
    screen.music = music;
    let issues = commit(cx, &b)?;
    let missing = {
        let root = cx.sandbox.root().to_path_buf();
        b.missing_music_issues(&|p| root.join(p).is_file())
            .into_iter()
            .filter(|i| i.screen == a.screen)
            .map(|i| i.message)
            .collect::<Vec<_>>()
    };
    out(json!({"screen": a.screen, "music": a.music, "issues": issues, "missing": missing}))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotCardArgs {
    /// Screen id.
    screen: String,
    /// Id of the SaveSlots widget (default: the first one of the screen).
    widget: Option<String>,
    /// `default`: write the stock card as an editable widget tree (the children of
    /// the SaveSlots widget, built once per slot; the look stays identical until
    /// edited). Inside it, `Bind(SlotField(Title | Number | Timestamp | Date |
    /// PlayTime | Chapter | Preview | Status | Thumbnail))` shows a slot field (bind
    /// an Image to Thumbnail) and `visible: SlotEmpty | SlotUsed | SlotLocked` tells
    /// the states apart. `stock`: remove the template (back to the stock card).
    /// Omit to leave the template as it is.
    template: Option<String>,
    /// The styles and sizes as RON `(card_style: Some("card"), empty_style: .., locked_style: ..,
    /// tab_style: .., pager_style: .., delete_style: .., confirm_style: .., confirm_button_style: ..,
    /// card_height: Some(170.0), column_gap: Some(24.0), row_gap: Some(18.0))`; replaces the
    /// whole look. Hover / pressed / focus looks come from the styles.
    look: Option<String>,
}

pub fn set_ui_save_slots(cx: &mut ToolCx<'_>, a: SlotCardArgs) -> Result<String, ToolError> {
    let look = match &a.look {
        Some(text) => Some(
            ron::from_str::<orbit_ui_data::SlotsLook>(text)
                .map_err(|e| ToolError::invalid(format!("look RON: {e}")))?,
        ),
        None => None,
    };
    let template = match a.template.as_deref() {
        None => None,
        Some("default") => Some(true),
        Some("stock") => Some(false),
        Some(other) => {
            return Err(ToolError::invalid(format!(
                "template must be `default` or `stock`, not `{other}`"
            )));
        }
    };
    fn find<'a>(
        w: &'a mut orbit_ui_data::WidgetDef,
        id: Option<&str>,
    ) -> Option<&'a mut orbit_ui_data::WidgetDef> {
        let is_match = matches!(w.kind, orbit_ui_data::WidgetKind::SaveSlots { .. })
            && id.is_none_or(|id| w.id == id);
        if is_match {
            return Some(w);
        }
        w.children.iter_mut().find_map(|c| find(c, id))
    }
    let mut b = load(cx)?;
    let widgets = b.theme.as_ref().map(|t| t.widgets.clone()).unwrap_or_default();
    let screen = b
        .screens
        .iter_mut()
        .find(|s| s.id == a.screen)
        .ok_or_else(|| ToolError::not_found(format!("no screen `{}`", a.screen)))?;
    let w = find(&mut screen.root, a.widget.as_deref())
        .ok_or_else(|| ToolError::not_found(format!("no SaveSlots widget in `{}`", a.screen)))?;
    if let Some(look) = look
        && let orbit_ui_data::WidgetKind::SaveSlots { look: slot_look, .. } = &mut w.kind
    {
        *slot_look = look;
    }
    match template {
        Some(true) => w.children = orbit_ui_data::default_slot_card(&widgets),
        Some(false) => w.children.clear(),
        None => {}
    }
    let templated = !w.children.is_empty();
    let id = w.id.clone();
    let issues = commit(cx, &b)?;
    out(json!({"screen": a.screen, "widget": id, "templated": templated, "issues": issues}))
}

pub fn set_ui_widget_style(cx: &mut ToolCx<'_>, a: WidgetStyleArgs) -> Result<String, ToolError> {
    fn visit(w: &mut orbit_ui_data::WidgetDef, id: &str, style: &Option<String>, found: &mut bool) {
        if w.id == id {
            w.style.clone_from(style);
            *found = true;
        }
        for c in &mut w.children {
            visit(c, id, style, found);
        }
    }
    let mut b = load(cx)?;
    let screen = b
        .screens
        .iter_mut()
        .find(|s| s.id == a.screen)
        .ok_or_else(|| ToolError::not_found(format!("no screen `{}`", a.screen)))?;
    let mut found = false;
    visit(&mut screen.root, &a.widget, &a.style, &mut found);
    if !found {
        return Err(ToolError::not_found(format!("no widget `{}` in `{}`", a.widget, a.screen)));
    }
    let issues = commit(cx, &b)?;
    out(json!({"screen": a.screen, "widget": a.widget, "style": a.style, "issues": issues}))
}
