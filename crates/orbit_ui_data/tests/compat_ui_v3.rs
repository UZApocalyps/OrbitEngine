//! M7-02 (`ui_version` 4): documents written with `ui_version` 3 (frozen in
//! `examples/fixtures/compat/ui_v3/`, never regenerate) must keep loading.

use orbit_ui_data::{ScreenAsset, Theme, UI_VERSION, UiBundle, from_ron};

const MAIN_MENU_V3: &str = include_str!("../../../examples/fixtures/compat/ui_v3/main_menu.screen.ron");
const PREFERENCES_V3: &str =
    include_str!("../../../examples/fixtures/compat/ui_v3/preferences.screen.ron");
const THEME_V3: &str = include_str!("../../../examples/fixtures/compat/ui_v3/theme.ron");

/// Frozen version 3 documents (no per-language fonts) keep loading,
/// migrated by number only.
#[test]
fn v3_fixtures_load_and_migrate_to_the_current_version() {
    for src in [MAIN_MENU_V3, PREFERENCES_V3] {
        assert!(src.contains("ui_version: 3"));
    }
    let menu: ScreenAsset = from_ron(MAIN_MENU_V3).unwrap();
    assert_eq!(menu.ui_version, UI_VERSION);
    // The stock menu gained the gallery and replay buttons (M7-03, M7-04).
    let mut stock = UiBundle::builtin().screen("main_menu").unwrap().clone();
    fn strip(w: &mut orbit_ui_data::WidgetDef) {
        w.children.retain(|c| !["gallery", "replay", "achievements"].contains(&c.id.as_str()));
        w.children.iter_mut().for_each(strip);
    }
    strip(&mut stock.root);
    assert_eq!(menu, stock);
    let prefs: ScreenAsset = from_ron(PREFERENCES_V3).unwrap();
    assert_eq!(&prefs, UiBundle::builtin().screen("preferences").unwrap());
    let theme = Theme::from_ron(THEME_V3).unwrap();
    assert!(theme.language_fonts.is_empty());
    assert_eq!(theme, Theme::default());
}

#[test]
fn language_fonts_round_trip_and_resolve_named_fonts() {
    let src = "(fonts: {\"jp\": (asset: \"assets/fonts/jp.otf\", family: \"Noto Sans JP\")}, \
               language_fonts: {\"ja\": [\"jp\", \"Inter\"], \"ar\": [\"Noto Sans Arabic\"]})";
    let theme = Theme::from_ron(src).unwrap();
    let resolved = theme.language_font_families();
    assert_eq!(resolved["ja"], vec!["Noto Sans JP".to_owned(), "Inter".to_owned()]);
    assert_eq!(resolved["ar"], vec!["Noto Sans Arabic".to_owned()]);
    let back = Theme::from_ron(&theme.to_ron().unwrap()).unwrap();
    assert_eq!(back, theme);
    // An empty map is never written (the default theme stays byte-identical).
    assert!(!Theme::default().to_ron().unwrap().contains("language_fonts"));
}
