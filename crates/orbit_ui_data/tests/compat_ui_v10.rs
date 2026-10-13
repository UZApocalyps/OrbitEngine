//! `ui_version` 11 (in-game notifications): frozen version 10 documents
//! (`examples/fixtures/compat/ui_v10/`, never regenerate) and older themes
//! keep loading, mean the same (stock toasts) and write none of the new
//! fields.

use orbit_ui_data::{NotifyAnchor, Theme, UI_VERSION, UiAction, UiNotify, WidgetKind, from_ron, to_ron};

const PAUSE_V10: &str = include_str!("../../../examples/fixtures/compat/ui_v10/pause.screen.ron");
const THEME_V6: &str = include_str!("../../../examples/fixtures/compat/ui_v6/theme.ron");

#[test]
fn v10_screens_load_as_version_11_unchanged() {
    assert!(PAUSE_V10.contains("ui_version: 10"));
    let screen = from_ron(PAUSE_V10).unwrap();
    assert_eq!(screen.ui_version, UI_VERSION);
    let as_current = PAUSE_V10.replace("ui_version: 10", &format!("ui_version: {UI_VERSION}"));
    assert_eq!(from_ron(&as_current).unwrap(), screen);
    let text = to_ron(&screen).unwrap();
    assert!(!text.contains("Notify"), "{text}");
}

#[test]
fn an_older_theme_has_stock_notifications() {
    let theme = Theme::from_ron(THEME_V6).unwrap();
    assert!(theme.notifications.is_default());
    assert_eq!(theme.notifications.anchor, NotifyAnchor::TopRight);
    assert!(!theme.to_ron().unwrap().contains("notifications"));
}

#[test]
fn a_notify_action_round_trips() {
    let mut screen = from_ron(PAUSE_V10).unwrap();
    fn first_button(w: &mut orbit_ui_data::WidgetDef) -> Option<&mut Vec<UiAction>> {
        if matches!(w.kind, WidgetKind::Button { .. }) {
            return w.kind.actions_mut();
        }
        w.children.iter_mut().find_map(first_button)
    }
    let actions = first_button(&mut screen.root);
    let done = actions.is_some();
    if let Some(action) = actions {
        action.push(UiAction::Notify(UiNotify {
            style: Some("achievement".into()),
            duration: Some(2.0),
            ..UiNotify::text("Hello [name]")
        }));
    }
    assert!(done, "the pause screen has a button");
    let text = to_ron(&screen).unwrap();
    assert!(text.contains("Notify(") && text.contains("Literal(\"Hello [name]\")"), "{text}");
    assert!(!text.contains("icon"), "unset options are not written: {text}");
    assert_eq!(from_ron(&text).unwrap(), screen);
}
