//! UI-12: documents written with `ui_version` 1 (frozen in
//! `examples/fixtures/compat/ui_v1/`, never regenerate) must keep loading.

use orbit_ui_data::{ScreenAsset, Theme, UI_VERSION, UiBundle, from_ron};

const BUNDLE_V1: &str = include_str!("../../../examples/fixtures/compat/ui_v1/bundle.ron");
const MAIN_MENU_V1: &str = include_str!("../../../examples/fixtures/compat/ui_v1/main_menu.screen.ron");
const PREFERENCES_V1: &str =
    include_str!("../../../examples/fixtures/compat/ui_v1/preferences.screen.ron");
const THEME_V1: &str = include_str!("../../../examples/fixtures/compat/ui_v1/theme.ron");

/// The stock main menu without the buttons M7-03 / M7-04 / M7-05 added.
fn stock_menu_before_m7_03() -> ScreenAsset {
    let mut menu = UiBundle::builtin().screen("main_menu").unwrap().clone();
    fn strip(w: &mut orbit_ui_data::WidgetDef) {
        w.children.retain(|c| !["gallery", "replay", "achievements"].contains(&c.id.as_str()));
        w.children.iter_mut().for_each(strip);
    }
    strip(&mut menu.root);
    menu
}

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
    assert_eq!(menu, stock_menu_before_m7_03());
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
    assert_eq!(menu, stock_menu_before_m7_03());
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
    // The stock bundle gained the gallery and replay screens (M7-03, M7-04)
    // and their main menu buttons.
    assert_eq!(bundle.screens.len() + 3, stock.screens.len());
    for old in &bundle.screens {
        assert_eq!(old.ui_version, UI_VERSION);
        match old.id.as_str() {
            "preferences" => {}
            "main_menu" => assert_eq!(*old, stock_menu_before_m7_03()),
            id => assert_eq!(Some(old), stock.screen(id), "{id}"),
        }
    }
    assert!(stock.screen("gallery").is_some() && stock.screen("replay").is_some());
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

const JOURNAL_V3: &str = include_str!("../../../examples/fixtures/compat/ui_v3/journal.screen.ron");
const JOURNAL_V4: &str = include_str!("../../../examples/fixtures/compat/ui_v4/journal.screen.ron");

/// UI-14 / UI-15 / UI-16 (`ui_version` 5): frozen version 3 and 4 documents
/// (variables, collections, accessibility settings, plain `Close`) keep
/// loading, are stamped version 5, mean the same and write no new field.
#[test]
fn v3_and_v4_screens_load_unchanged_as_version_5() {
    assert!(JOURNAL_V3.contains("ui_version: 3"));
    assert!(JOURNAL_V4.contains("ui_version: 4"));
    assert_eq!(from_ron(JOURNAL_V3).unwrap(), from_ron(JOURNAL_V4).unwrap());
    let screen = from_ron(JOURNAL_V4).unwrap();
    assert_eq!(screen.ui_version, UI_VERSION);
    assert!(UI_VERSION >= 5);
    assert!(!screen.hud);
    assert!(screen.transition_in.is_none() && screen.transition_out.is_none());
    assert_eq!(screen.on_cancel, vec![orbit_ui_data::UiAction::Close]);
    assert!(orbit_ui_data::validate(&screen, None).is_empty());
    let text = orbit_ui_data::to_ron(&screen).unwrap();
    for new_field in ["hud", "transition_in", "transition_out", "CloseWith", "Repeat"] {
        assert!(!text.contains(new_field), "{new_field}");
    }
    assert_eq!(from_ron(&text).unwrap(), screen);
}

#[test]
fn version_4_fields_round_trip() {
    let src = r#"(ui_version: 5, id: "bag", hud: true,
        transition_in: Some((kind: SlideUp, duration: 0.25, ease: Some(Preset(OutCubic)))),
        transition_out: Some((kind: Fade)),
        root: (kind: Stack(dir: Vertical), children: [
            (kind: Repeat(source: Var("bag"), dir: Grid(3), empty: Some(Literal("Empty"))), children: [
                (kind: Button(text: Bind(Item), action: [
                    VarSet((var: "bag", op: Remove, value: Item)),
                    CloseWith(Item),
                ])),
                (kind: Label(text: Bind(ItemIndex))),
            ]),
            (kind: Label(text: Bind(Param("contact")))),
            (kind: Button(text: Literal("x"), action: [CloseWith(Param("contact")), CloseWith(Lit(Int(3)))])),
        ]))"#;
    let screen = from_ron(src).unwrap();
    assert!(screen.hud && !screen.effective_pauses_game());
    let issues = orbit_ui_data::validate(&screen, None);
    assert!(issues.is_empty(), "{issues:?}");
    let text = orbit_ui_data::to_ron(&screen).unwrap();
    assert_eq!(from_ron(&text).unwrap(), screen);
}

#[test]
fn item_bindings_outside_a_repeat_and_hud_pauses_are_flagged() {
    let src = r#"(id: "s", hud: true, pauses_game: true,
        transition_in: Some((kind: Fade, duration: -1.0)),
        root: (kind: Stack(dir: Vertical), children: [
            (kind: Label(text: Bind(Item))),
            (kind: Button(text: Literal("x"), action: [CloseWith(ItemKey)])),
            (kind: Repeat(source: Setting(Language))),
        ]))"#;
    let screen = from_ron(src).unwrap();
    let issues = orbit_ui_data::validate(&screen, None);
    let has = |s: &str| issues.iter().any(|i| i.message.contains(s));
    assert!(has("never pauses"), "{issues:?}");
    assert!(has("outside a generated list"), "{issues:?}");
    assert!(has("duration"), "{issues:?}");
    assert!(has("list or map"), "{issues:?}");
}

const BAG_V5: &str = include_str!("../../../examples/fixtures/compat/ui_v5/bag.screen.ron");

/// M7-03 / M7-04 (`ui_version` 6): a frozen version 5 document (story
/// screen features: repeat, parameters, `CloseWith`, transitions) loads,
/// is stamped version 6, means the same and writes none of the new fields.
#[test]
fn v5_screens_load_unchanged_as_version_6() {
    assert!(BAG_V5.contains("ui_version: 5"));
    let screen = from_ron(BAG_V5).unwrap();
    assert_eq!(screen.ui_version, UI_VERSION);
    assert!(UI_VERSION >= 6);
    // Same document as if it had been written at version 6.
    let as_v6 = BAG_V5.replace("ui_version: 5", &format!("ui_version: {UI_VERSION}"));
    assert_eq!(from_ron(&as_v6).unwrap(), screen);
    assert!(orbit_ui_data::validate(&screen, None).is_empty());
    let text = orbit_ui_data::to_ron(&screen).unwrap();
    for new_field in ["bind", "Gallery", "ReplayScenes", "ReplayScene", "Item)"] {
        assert!(!text.contains(&format!("{new_field}:")), "{new_field}");
    }
    assert!(!text.contains("bind:"));
    assert_eq!(from_ron(&text).unwrap(), screen);
}

const GALLERY_V6: &str = include_str!("../../../examples/fixtures/compat/ui_v6/gallery.screen.ron");
const REPLAY_V6: &str = include_str!("../../../examples/fixtures/compat/ui_v6/replay.screen.ron");

/// M7-05 (`ui_version` 7): frozen version 6 documents (the stock gallery and
/// replay screens: gallery and replay sources, `Cond::Item`, row images) load,
/// are stamped version 7, mean the same and write none of the new variants.
#[test]
fn v6_screens_load_unchanged_as_version_7() {
    for src in [GALLERY_V6, REPLAY_V6] {
        assert!(src.contains("ui_version: 6"));
        let screen = from_ron(src).unwrap();
        assert_eq!(screen.ui_version, UI_VERSION);
        assert!(UI_VERSION >= 7);
        let as_v7 = src.replace("ui_version: 6", &format!("ui_version: {UI_VERSION}"));
        assert_eq!(from_ron(&as_v7).unwrap(), screen);
        assert!(orbit_ui_data::validate(&screen, None).is_empty());
        let text = orbit_ui_data::to_ron(&screen).unwrap();
        for new in ["Achievements", "Achievement(", "Unlocked", "HasAchievements"] {
            assert!(!text.contains(new), "{new}");
        }
        assert_eq!(from_ron(&text).unwrap(), screen);
    }
    // The stock screens are those documents (plus the new menu button).
    let stock = UiBundle::builtin();
    assert_eq!(from_ron(GALLERY_V6).unwrap(), *stock.screen("gallery").unwrap());
    assert_eq!(from_ron(REPLAY_V6).unwrap(), *stock.screen("replay").unwrap());
}

#[test]
fn version_7_achievement_bindings_round_trip_and_validate() {
    let src = r#"(ui_version: 7, id: "trophies", root: (kind: Stack(dir: Vertical), children: [
            (kind: Label(text: Bind(Achievement(Progress)))),
            (kind: Repeat(source: Achievements, dir: Grid(2)), children: [
                (kind: Image(bind: Some(Achievement(Icon))), visible: Unlocked),
                (kind: Label(text: Bind(Achievement(Title)))),
                (kind: Label(text: Bind(Achievement(Description))), visible: Not(Unlocked)),
            ]),
            (kind: Button(text: Literal("x"), action: [Close]), visible: HasAchievements),
        ]))"#;
    let screen = from_ron(src).unwrap();
    let issues = orbit_ui_data::validate(&screen, None);
    assert!(issues.is_empty(), "{issues:?}");
    let text = orbit_ui_data::to_ron(&screen).unwrap();
    assert_eq!(from_ron(&text).unwrap(), screen);
    // A row field outside a generated list is reported.
    let outside = r#"(ui_version: 7, id: "x", root: (kind: Label(text: Bind(Achievement(Title)))))"#;
    let issues = orbit_ui_data::validate(&from_ron(outside).unwrap(), None);
    assert!(!issues.is_empty());
    // The stock achievements screen is valid and uses them.
    let stock = UiBundle::builtin();
    let s = stock.screen("achievements").unwrap();
    assert!(orbit_ui_data::validate(s, None).is_empty());
    assert!(orbit_ui_data::to_ron(s).unwrap().contains("Achievements"));
}

#[test]
fn version_6_gallery_and_replay_round_trip_and_validate() {
    let src = r#"(ui_version: 6, id: "extras", root: (kind: Stack(dir: Vertical), children: [
            (kind: Repeat(source: Gallery(["bg/a.png", "bg/b.png"]), dir: Grid(4)), children: [
                (kind: Image(bind: Some(Item)), visible: Item),
                (kind: Label(text: Literal("locked")), visible: Not(Item)),
            ]),
            (kind: Repeat(source: ReplayScenes([])), children: [
                (kind: Button(text: Bind(Item), action: [ReplayScene(Item)])),
            ]),
            (kind: Button(text: Literal("x"), action: [ReplayScene(Lit(Str("intro")))])),
        ]))"#;
    let screen = from_ron(src).unwrap();
    let issues = orbit_ui_data::validate(&screen, None);
    assert!(issues.is_empty(), "{issues:?}");
    let text = orbit_ui_data::to_ron(&screen).unwrap();
    assert_eq!(from_ron(&text).unwrap(), screen);
    // The stock screens use them and are valid.
    let stock = UiBundle::builtin();
    for id in ["gallery", "replay"] {
        let s = stock.screen(id).unwrap();
        assert!(orbit_ui_data::validate(s, None).is_empty(), "{id}");
    }
    // Row-only values outside a list, unknown scenes and empty targets.
    let bad = r#"(id: "s", root: (kind: Stack(dir: Vertical), children: [
            (kind: Image(bind: Some(Item))),
            (kind: Button(text: Literal("x"), action: [ReplayScene(Item), ReplayScene(Lit(Str(" ")))])),
            (kind: Repeat(source: ReplayScenes(["nowhere"])), children: [(kind: Label(text: Bind(Item)))]),
        ]))"#;
    let screen = from_ron(bad).unwrap();
    let issues = orbit_ui_data::validate(&screen, None);
    let has = |s: &str| issues.iter().any(|i| i.message.contains(s));
    assert!(has("outside a generated list"), "{issues:?}");
    assert!(has("ReplayScene needs a scene"), "{issues:?}");
    let scenes = orbit_ui_data::scene_issues(&screen, &|s| s == "intro");
    assert_eq!(scenes.len(), 1, "{scenes:?}");
}
