//! Checks of a screen document beyond what parsing guarantees.

use std::collections::HashSet;

use crate::action::{BindPath, SettingKey, UiAction};
use orbit_core::ValueType;

use crate::widget::{Cond, ScreenAsset, TextRef, WidgetDef, WidgetKind};
use crate::theme::Theme;
use crate::layout::Dir;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The screen cannot be used as is.
    Error,
    /// Works, but probably not as intended.
    Warning,
}

/// One problem found in a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Screen id.
    pub screen: String,
    /// Widget id (empty: the screen itself or an unnamed widget).
    pub widget: String,
    pub message: String,
}

impl Issue {
    pub(crate) fn error(screen: &str, widget: &str, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, screen, widget, message)
    }

    pub(crate) fn warning(screen: &str, widget: &str, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, screen, widget, message)
    }

    fn new(severity: Severity, screen: &str, widget: &str, message: impl Into<String>) -> Self {
        Self {
            severity,
            screen: screen.to_owned(),
            widget: widget.to_owned(),
            message: message.into(),
        }
    }
}

/// Most slots one page of a `SaveSlots` widget can show.
pub const MAX_SLOTS_PER_PAGE: usize = 24;

/// Deepest nesting accepted (guards the recursive builders).
pub const MAX_DEPTH: usize = 32;

/// Like [`validate`], and also checks story variables against their declared
/// types (`ty_of` gives the type of a declared variable): unknown variables,
/// an operation the type does not offer, a value of another type, bindings
/// the type cannot drive (slider: number, toggle: boolean).
pub fn validate_with_vars(
    screen: &ScreenAsset,
    theme: Option<&Theme>,
    ty_of: &dyn Fn(&str) -> Option<ValueType>,
) -> Vec<Issue> {
    let mut out = validate(screen, theme);
    out.extend(variable_issues(screen, ty_of));
    out
}

/// Only the story-variable warnings of [`validate_with_vars`].
pub fn variable_issues(
    screen: &ScreenAsset,
    ty_of: &dyn Fn(&str) -> Option<ValueType>,
) -> Vec<Issue> {
    let mut out = Vec::new();
    let sid = screen.id.as_str();
    let add = |wid: &str, messages: Vec<String>, out: &mut Vec<Issue>| {
        out.extend(messages.into_iter().map(|m| Issue::warning(sid, wid, m)));
    };
    for action in &screen.on_cancel {
        if let UiAction::VarSet(op) = action {
            add("", op.issues(ty_of), &mut out);
        }
    }
    screen.root.walk(&mut |w| {
        let wid = w.id.as_str();
        for cond in [&w.visible, &w.enabled] {
            let mut stack = vec![cond];
            while let Some(c) = stack.pop() {
                match c {
                    Cond::Vars(c) => add(wid, c.issues(ty_of), &mut out),
                    Cond::Not(inner) => stack.push(inner),
                    _ => {}
                }
            }
        }
        match &w.kind {
            WidgetKind::Button { action, .. } => {
                for a in action {
                    if let UiAction::VarSet(op) = a {
                        add(wid, op.issues(ty_of), &mut out);
                    }
                }
            }
            WidgetKind::Slider { bind, .. } => {
                if let Some(name) = bind.var() {
                    match ty_of(name) {
                        None => add(wid, vec![format!("unknown variable `{name}`")], &mut out),
                        Some(t) if !t.is_numeric() => add(wid, vec![format!("a slider needs a number, `{name}` is {t}")], &mut out),
                        _ => {}
                    }
                }
            }
            WidgetKind::Toggle { bind, .. } => {
                if let Some(name) = bind.var() {
                    match ty_of(name) {
                        None => add(wid, vec![format!("unknown variable `{name}`")], &mut out),
                        Some(t) if t != ValueType::Bool => add(wid, vec![format!("a toggle needs a boolean, `{name}` is {t}")], &mut out),
                        _ => {}
                    }
                }
            }
            WidgetKind::Dropdown { bind, values, .. } => {
                if let Some(name) = bind.var() {
                    match ty_of(name) {
                        None => add(wid, vec![format!("unknown variable `{name}`")], &mut out),
                        Some(t) => {
                            if values.iter().any(|v| v.value_type() != t) {
                                add(wid, vec![format!("a dropdown value is not of the type of `{name}` ({t})")], &mut out);
                            }
                        }
                    }
                }
            }
            WidgetKind::Label { text: TextRef::Bind(b), .. } => {
                if let Some(name) = b.var()
                    && ty_of(name).is_none()
                {
                    add(wid, vec![format!("unknown variable `{name}`")], &mut out);
                }
            }
            _ => {}
        }
    });
    out
}

/// Warnings for `SceneGoto` actions whose scene is not in the story
/// (`known` says whether a scene id or name exists). Empty targets are
/// errors of [`validate`] and are not repeated here.
pub fn scene_issues(screen: &ScreenAsset, known: &dyn Fn(&str) -> bool) -> Vec<Issue> {
    let mut out = Vec::new();
    let sid = screen.id.as_str();
    let mut check = |actions: &[UiAction], wid: &str| {
        for a in actions {
            if let UiAction::SceneGoto(scene) = a
                && !scene.trim().is_empty()
                && !known(scene.trim())
            {
                out.push(Issue::warning(sid, wid, format!("jumps to `{scene}`, which is not a scene of the story")));
            }
        }
    };
    check(&screen.on_cancel, "");
    screen.root.walk(&mut |w| {
        if let WidgetKind::Button { action, .. } = &w.kind {
            check(action, &w.id);
        }
    });
    out
}

/// Checks `screen`; `theme` (when given) is used to verify style names.
pub fn validate(screen: &ScreenAsset, theme: Option<&Theme>) -> Vec<Issue> {
    let mut out = Vec::new();
    let sid = screen.id.as_str();
    if sid.trim().is_empty() {
        out.push(Issue::error(sid, "", "the screen has no id"));
    }
    for action in &screen.on_cancel {
        check_action(sid, "", action, &mut out);
    }
    let mut ids = HashSet::new();
    let mut slot_lists = 0;
    walk(&screen.root, 1, &mut |w, depth| {
        if depth > MAX_DEPTH {
            out.push(Issue::error(sid, &w.id, "widgets are nested too deeply"));
            return;
        }
        if !w.id.is_empty() && !ids.insert(w.id.clone()) {
            out.push(Issue::error(sid, &w.id, "duplicate widget id"));
        }
        if matches!(w.kind, WidgetKind::SaveSlots { .. }) {
            slot_lists += 1;
            if slot_lists == 2 {
                out.push(Issue::warning(sid, &w.id, "only one save slot list per screen keeps its own page and tab"));
            }
        }
        check_widget(sid, w, theme, &mut out);
    });
    out
}

fn walk<'a>(w: &'a WidgetDef, depth: usize, f: &mut impl FnMut(&'a WidgetDef, usize)) {
    f(w, depth);
    if depth > MAX_DEPTH {
        return;
    }
    for c in &w.children {
        walk(c, depth + 1, f);
    }
}

fn check_action(sid: &str, wid: &str, action: &UiAction, out: &mut Vec<Issue>) {
    if !action.is_supported() {
        out.push(Issue::warning(sid, wid, format!("{action:?} is not supported yet and does nothing")));
    }
    match action {
        UiAction::Open(id) | UiAction::Replace(id) if id.trim().is_empty() => {
            out.push(Issue::error(sid, wid, "Open / Replace needs a screen id"));
        }
        UiAction::SceneGoto(scene) if scene.trim().is_empty() => {
            out.push(Issue::error(sid, wid, "SceneGoto needs a scene"));
        }
        _ => {}
    }
}

fn check_bind(sid: &str, wid: &str, bind: &BindPath, out: &mut Vec<Issue>) -> Option<SettingKey> {
    if bind.var().is_some_and(|v| v.trim().is_empty()) {
        out.push(Issue::error(sid, wid, "empty variable name"));
    }
    bind.setting()
}

fn check_text(sid: &str, wid: &str, text: &TextRef, out: &mut Vec<Issue>) {
    match text {
        TextRef::Key(k) if k.trim().is_empty() => {
            out.push(Issue::error(sid, wid, "empty text key"));
        }
        TextRef::Bind(b) => {
            check_bind(sid, wid, b, out);
        }
        _ => {}
    }
}

fn check_widget(sid: &str, w: &WidgetDef, theme: Option<&Theme>, out: &mut Vec<Issue>) {
    let wid = w.id.as_str();
    if let (Some(name), Some(theme)) = (&w.style, theme)
        && !theme.styles.contains_key(name)
    {
        out.push(Issue::warning(sid, wid, format!("unknown style `{name}`")));
    }
    match &w.kind {
        WidgetKind::Grid { columns } if *columns == 0 => {
            out.push(Issue::error(sid, wid, "a grid needs at least one column"));
        }
        WidgetKind::Stack { dir: Dir::Grid(_) } | WidgetKind::Panel { dir: Dir::Grid(_), .. } => {
            out.push(Issue::warning(sid, wid, "use the Grid widget for grids"));
        }
        WidgetKind::Button { text, action } => {
            check_text(sid, wid, text, out);
            for a in action {
                check_action(sid, wid, a, out);
            }
        }
        WidgetKind::SaveSlots { columns, rows, .. } => {
            if *columns == 0 || *rows == 0 {
                out.push(Issue::error(sid, wid, "the save slots need at least one column and one row"));
            } else if columns * rows > MAX_SLOTS_PER_PAGE {
                out.push(Issue::error(sid, wid, format!("at most {MAX_SLOTS_PER_PAGE} slots per page")));
            }
        }
        WidgetKind::Label { text, .. } => check_text(sid, wid, text, out),
        WidgetKind::Slider { bind, min, max, step } => {
            if min.partial_cmp(max) != Some(std::cmp::Ordering::Less) {
                out.push(Issue::error(sid, wid, "slider min must be below max"));
            }
            if step.is_some_and(|s| s <= 0.0) {
                out.push(Issue::error(sid, wid, "slider step must be positive"));
            }
            if let Some(key) = check_bind(sid, wid, bind, out)
                && !matches!(
                    key,
                    SettingKey::MasterVolume
                        | SettingKey::MusicVolume
                        | SettingKey::SfxVolume
                        | SettingKey::VoiceVolume
                        | SettingKey::TextSpeed
                        | SettingKey::AutoDelay
                        | SettingKey::TextScale
                )
            {
                out.push(Issue::error(sid, wid, format!("{key:?} cannot be edited with a slider")));
            }
        }
        WidgetKind::Toggle { text, bind } => {
            check_text(sid, wid, text, out);
            if let Some(key) = check_bind(sid, wid, bind, out)
                && !matches!(
                    key,
                    SettingKey::Fullscreen
                        | SettingKey::ScreenReader
                        | SettingKey::HighContrast
                        | SettingKey::ReducedMotion
                )
            {
                out.push(Issue::error(sid, wid, format!("{key:?} cannot be edited with a toggle")));
            }
        }
        WidgetKind::Dropdown { bind, options, values } => {
            if bind.var().is_some() {
                check_bind(sid, wid, bind, out);
                if options.is_empty() {
                    out.push(Issue::error(sid, wid, "a variable dropdown needs at least one option"));
                } else if values.len() != options.len() {
                    out.push(Issue::error(
                        sid,
                        wid,
                        format!("{} options but {} values: each option needs a value", options.len(), values.len()),
                    ));
                } else if values.iter().any(|v| v.value_type() != values[0].value_type()) {
                    out.push(Issue::error(sid, wid, "the values of a dropdown must all have the same type"));
                }
            } else if let Some(key) = check_bind(sid, wid, bind, out) {
                match key {
                    SettingKey::Language => {}
                    SettingKey::SkipMode if options.len() == 2 => {}
                    SettingKey::SkipMode => {
                        out.push(Issue::error(sid, wid, "the skip mode dropdown needs exactly 2 options"));
                    }
                    other => out.push(Issue::error(sid, wid, format!("{other:?} cannot be edited with a dropdown"))),
                }
            }
            for o in options {
                check_text(sid, wid, o, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::from_ron;

    fn issues(root: &str) -> Vec<Issue> {
        let src = format!("(id: \"s\", root: {root})");
        validate(&from_ron(&src).unwrap(), None)
    }

    fn errors(root: &str) -> usize {
        issues(root).iter().filter(|i| i.severity == Severity::Error).count()
    }

    #[test]
    fn a_valid_screen_has_no_issue() {
        let root = "(kind: Stack(dir: Vertical), children: [\
            (id: \"a\", kind: Button(text: Key(\"ui.menu.back\"), action: [Close])),\
            (id: \"v\", kind: Slider(bind: Setting(MasterVolume), min: 0.0, max: 1.0))])";
        assert!(issues(root).is_empty(), "{:?}", issues(root));
    }

    #[test]
    fn duplicate_ids_and_bad_numbers_are_errors() {
        let dup = "(kind: Stack(), children: [(id: \"a\", kind: Stack()), (id: \"a\", kind: Stack())])";
        assert_eq!(errors(dup), 1);
        assert_eq!(errors("(kind: Slider(bind: Setting(MasterVolume), min: 1.0, max: 1.0))"), 1);
        assert_eq!(errors("(kind: Slider(bind: Setting(MasterVolume), min: 0.0, max: 1.0, step: Some(0.0)))"), 1);
        assert_eq!(errors("(kind: Grid(columns: 0))"), 1);
        assert_eq!(errors("(kind: Slider(bind: Setting(Fullscreen), min: 0.0, max: 1.0))"), 1);
        assert_eq!(errors("(kind: Toggle(text: Literal(\"x\"), bind: Setting(Language)))"), 1);
        assert_eq!(errors("(kind: Dropdown(bind: Setting(SkipMode), options: []))"), 1);
        assert_eq!(errors("(kind: Button(text: Key(\"\"), action: [Open(\"\")]))"), 2);
    }

    #[test]
    fn save_slots_are_checked() {
        assert!(issues("(kind: SaveSlots(mode: Load))").is_empty());
        assert_eq!(errors("(kind: SaveSlots(mode: Save, columns: 0))"), 1);
        assert_eq!(errors("(kind: SaveSlots(mode: Save, columns: 5, rows: 5))"), 1);
        let two = "(kind: Stack(), children: [(kind: SaveSlots(mode: Load)), (kind: SaveSlots(mode: Save))])";
        assert_eq!(issues(two).len(), 1);
    }

    #[test]
    fn scene_goto_is_checked() {
        let ok = "(kind: Button(text: Literal(\"x\"), action: [SceneGoto(\"a\")]), children: [\
                  (kind: Toggle(text: Literal(\"y\"), bind: Var(\"v\")))])";
        assert!(issues(ok).is_empty(), "SceneGoto is supported");
        assert_eq!(errors("(kind: Button(text: Literal(\"x\"), action: [SceneGoto(\" \")]))"), 1);

        let asset = from_ron(&format!("(id: \"s\", on_cancel: [SceneGoto(\"b\")], root: {ok})")).unwrap();
        let out = scene_issues(&asset, &|s| s == "a");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Warning);
        assert!(out[0].message.contains("`b`"));
        assert!(scene_issues(&asset, &|_| true).is_empty());
    }

    #[test]
    fn unknown_styles_are_warnings_and_deep_trees_errors() {
        let asset = from_ron("(id: \"s\", root: (kind: Stack(), style: Some(\"nope\")))").unwrap();
        let theme = Theme::default();
        let out = validate(&asset, Some(&theme));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Warning);

        let mut deep = WidgetDef::new(WidgetKind::Stack { dir: Dir::Vertical });
        for _ in 0..MAX_DEPTH + 2 {
            let mut parent = WidgetDef::new(WidgetKind::Stack { dir: Dir::Vertical });
            parent.children.push(deep);
            deep = parent;
        }
        let mut asset = asset;
        asset.root = deep;
        assert!(validate(&asset, None).iter().any(|i| i.severity == Severity::Error));
    }
}
