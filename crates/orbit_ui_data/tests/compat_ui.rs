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
    // The stock preferences screen gained the accessibility controls in
    // version 3: the old one loads, migrated, without them.
    let prefs: ScreenAsset = from_ron(PREFERENCES_V1).unwrap();
    assert_eq!(prefs.ui_version, UI_VERSION);
    assert!(UiBundle::builtin().screen("preferences").is_some());
    assert!(orbit_ui_data::validate(&prefs, None).is_empty());
}

const MAIN_MENU_V2: &str = include_str!("../../../examples/fixtures/compat/ui_v2/main_menu.screen.ron");
const PREFERENCES_V2: &str =
    include_str!("../../../examples/fixtures/compat/ui_v2/preferences.screen.ron");

/// M7-07 (`ui_version` 3): frozen version 2 documents, written before the
/// accessibility `SettingKey`s, keep loading and are stamped version 3.
#[test]
fn v2_fixtures_load_and_migrate_to_the_current_version() {
    for src in [MAIN_MENU_V2, PREFERENCES_V2] {
        assert!(src.contains("ui_version: 2"));
    }
    let menu: ScreenAsset = from_ron(MAIN_MENU_V2).unwrap();
    assert_eq!(menu.ui_version, UI_VERSION);
    assert_eq!(&menu, UiBundle::builtin().screen("main_menu").unwrap());
    let prefs: ScreenAsset = from_ron(PREFERENCES_V2).unwrap();
    assert_eq!(prefs.ui_version, UI_VERSION);
    assert!(!PREFERENCES_V2.contains("TextScale"));
    assert!(orbit_ui_data::validate(&prefs, None).is_empty());
    // The stock screen is the old one plus the new controls.
    let stock = UiBundle::builtin();
    let stock = stock.screen("preferences").unwrap();
    assert!(stock.root.children.len() > prefs.root.children.len());
}

#[test]
fn the_accessibility_settings_are_valid_bindings() {
    let src = "(ui_version: 3, id: \"s\", root: (kind: Stack(dir: Vertical), children: [\
        (kind: Slider(bind: Setting(TextScale), min: 1.0, max: 1.6)),\
        (kind: Toggle(text: Literal(\"hc\"), bind: Setting(HighContrast))),\
        (kind: Toggle(text: Literal(\"rm\"), bind: Setting(ReducedMotion))),\
        (kind: Toggle(text: Literal(\"sr\"), bind: Setting(ScreenReader))),\
        (kind: Label(text: Bind(Setting(TextScale))))]))";
    let screen = from_ron(src).unwrap();
    assert!(orbit_ui_data::validate(&screen, None).is_empty());
    // Not every key fits every control.
    let bad = src.replace("Slider(bind: Setting(TextScale)", "Slider(bind: Setting(HighContrast)");
    let screen = from_ron(&bad).unwrap();
    assert!(!orbit_ui_data::validate(&screen, None).is_empty());
}

#[test]
fn a_v1_bundle_and_theme_load() {
    let bundle = UiBundle::from_ron(BUNDLE_V1).unwrap();
    let stock = UiBundle::builtin();
    assert_eq!(bundle.theme, stock.theme);
    assert_eq!(bundle.screens.len(), stock.screens.len());
    for (old, new) in bundle.screens.iter().zip(&stock.screens) {
        assert_eq!(old.ui_version, UI_VERSION);
        if new.id != "preferences" {
            assert_eq!(old, new, "{}", new.id);
        }
    }
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
