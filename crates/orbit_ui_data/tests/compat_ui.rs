//! UI-12: documents written with `ui_version` 1 (frozen in
//! `examples/fixtures/compat/ui_v1/`, never regenerate) must keep loading.

use orbit_ui_data::{ScreenAsset, Theme, UI_VERSION, UiBundle, from_ron};

const BUNDLE_V1: &str = include_str!("../../../examples/fixtures/compat/ui_v1/bundle.ron");
const MAIN_MENU_V1: &str = include_str!("../../../examples/fixtures/compat/ui_v1/main_menu.screen.ron");
const PREFERENCES_V1: &str =
    include_str!("../../../examples/fixtures/compat/ui_v1/preferences.screen.ron");
const THEME_V1: &str = include_str!("../../../examples/fixtures/compat/ui_v1/theme.ron");

#[test]
fn v1_fixtures_declare_version_1() {
    for src in [BUNDLE_V1, MAIN_MENU_V1, PREFERENCES_V1] {
        assert!(src.contains("ui_version: 1"));
    }
}

#[test]
fn a_v1_screen_loads_migrated_and_equals_the_stock_screen() {
    let menu: ScreenAsset = from_ron(MAIN_MENU_V1).unwrap();
    assert_eq!(menu.ui_version, UI_VERSION);
    assert_eq!(&menu, UiBundle::builtin().screen("main_menu").unwrap());
    let prefs: ScreenAsset = from_ron(PREFERENCES_V1).unwrap();
    assert_eq!(&prefs, UiBundle::builtin().screen("preferences").unwrap());
}

#[test]
fn a_v1_bundle_and_theme_load() {
    let bundle = UiBundle::from_ron(BUNDLE_V1).unwrap();
    assert_eq!(bundle, UiBundle::builtin());
    let theme = Theme::from_ron(THEME_V1).unwrap();
    assert!(theme.fonts.is_empty());
    assert_eq!(theme, Theme::default());
}

#[test]
fn new_fields_round_trip_and_stay_out_of_old_documents() {
    let src = "(ui_version: 1, id: \"s\", root: (kind: Grid(columns: 2), children: [\
               (kind: Label(text: Literal(\"a\")), layout: (col_span: 2)),\
               (kind: Dropdown(bind: Var(\"mode\"), options: [Literal(\"Easy\"), Literal(\"Hard\")], \
                values: [Str(\"easy\"), Str(\"hard\")]))]))";
    let screen = from_ron(src).unwrap();
    assert_eq!(screen.ui_version, UI_VERSION);
    assert_eq!(screen.root.children[0].layout.col_span, 2);
    assert_eq!(screen.root.children[0].layout.row_span, 1);
    let text = orbit_ui_data::to_ron(&screen).unwrap();
    assert_eq!(from_ron(&text).unwrap(), screen);
    // A default span is not written.
    assert!(!text.contains("row_span"));
}
