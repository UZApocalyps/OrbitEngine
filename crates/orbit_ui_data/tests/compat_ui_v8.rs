//! `ui_version` 9 (menu music, editable save slots): frozen version 8
//! documents (`examples/fixtures/compat/ui_v8/`, never regenerate) keep
//! loading, mean the same and write none of the new fields.

use orbit_ui_data::{
    BindPath, Cond, OverGame, ScreenAsset, ScreenMusic, SlotField, SlotsLook, UI_VERSION, UiAction, UiBundle,
    WidgetKind, WidgetTheme, default_slot_card, from_ron, to_ron,
};

const SLOTS_V8: &str = include_str!("../../../examples/fixtures/compat/ui_v8/slots.screen.ron");

#[test]
fn v8_screens_load_as_version_9_with_the_same_meaning() {
    assert!(SLOTS_V8.contains("ui_version: 8"));
    let screen = from_ron(SLOTS_V8).unwrap();
    assert_eq!(screen.ui_version, UI_VERSION);
    assert_eq!(screen.music, None);
    let as_current = SLOTS_V8.replace("ui_version: 8", &format!("ui_version: {UI_VERSION}"));
    assert_eq!(from_ron(&as_current).unwrap(), screen);
    // A SaveSlots written before version 9 has no look and no template.
    let mut slots = 0;
    screen.root.walk(&mut |w| {
        if let WidgetKind::SaveSlots { look, .. } = &w.kind {
            assert!(look.is_default());
            assert!(w.children.is_empty(), "no card template");
            slots += 1;
        }
    });
    assert_eq!(slots, 1);
    // It writes none of the version 9 fields and reads back identical.
    let text = to_ron(&screen).unwrap();
    for new_field in ["music:", "look:", "SlotField", "SlotEmpty", "PlayMusic"] {
        assert!(!text.contains(new_field), "{new_field} written for a v8 screen:\n{text}");
    }
    assert_eq!(from_ron(&text).unwrap(), screen);
    assert!(orbit_ui_data::validate(&screen, None).is_empty());
}

#[test]
fn version_9_music_and_slot_template_round_trip() {
    let music = ScreenMusic {
        asset: "assets/music/menu.ogg".into(),
        volume: 0.8,
        fade_in_ms: 1200,
        over_game: OverGame::Replace,
        ..ScreenMusic::default()
    };
    let mut screen: ScreenAsset = from_ron(SLOTS_V8).unwrap();
    screen.music = Some(music.clone());
    screen.on_cancel.push(UiAction::StopMusic);
    screen.root.walk(&mut |_| {});
    // Turn the SaveSlots into a templated one.
    fn customize(w: &mut orbit_ui_data::WidgetDef) {
        if let WidgetKind::SaveSlots { look, .. } = &mut w.kind {
            look.card_style = Some("card".into());
            look.card_height = Some(200.0);
            w.children = default_slot_card(&WidgetTheme::default());
        }
        w.children.iter_mut().for_each(customize);
    }
    customize(&mut screen.root);
    let text = to_ron(&screen).unwrap();
    assert!(text.contains("music:") && text.contains("look:") && text.contains("SlotField(Thumbnail)"));
    let back = from_ron(&text).unwrap();
    assert_eq!(back, screen);
    assert_eq!(back.music, Some(music));

    // Slot conditions and fields only make sense inside the template.
    let mut stray: ScreenAsset = from_ron(SLOTS_V8).unwrap();
    stray.root.visible = Cond::SlotEmpty;
    assert!(
        orbit_ui_data::validate(&stray, None)
            .iter()
            .any(|i| i.message.contains("slot condition"))
    );
    assert!(BindPath::SlotField(SlotField::Date).is_item());
    assert!(SlotsLook::default().is_default());
    let ok = orbit_ui_data::validate(&screen, None);
    assert!(ok.is_empty(), "{ok:?}");
    // A button inside the card is flagged: the card itself is the button.
    let mut risky = screen.clone();
    fn add_button(w: &mut orbit_ui_data::WidgetDef) {
        if matches!(w.kind, WidgetKind::SaveSlots { .. }) {
            w.children.push(orbit_ui_data::WidgetDef::new(WidgetKind::Button {
                text: orbit_ui_data::TextRef::Literal("x".into()),
                action: Vec::new(),
            }));
        }
        w.children.iter_mut().for_each(add_button);
    }
    add_button(&mut risky.root);
    assert!(orbit_ui_data::validate(&risky, None).iter().any(|i| i.message.contains("the card is the button")));
}

#[test]
fn music_files_are_collected_for_export_and_checked() {
    let mut menu: ScreenAsset = from_ron(SLOTS_V8).unwrap();
    menu.music = Some(ScreenMusic {
        asset: "assets/music/menu.ogg".into(),
        ..ScreenMusic::default()
    });
    let mut button_screen: ScreenAsset = from_ron(SLOTS_V8).unwrap();
    button_screen.id = "other".into();
    button_screen.on_cancel.push(UiAction::PlayMusic("assets/music/other.ogg".into()));
    let bundle = UiBundle {
        screens: vec![menu, button_screen],
        ..UiBundle::default()
    };
    assert_eq!(bundle.music_paths(), ["assets/music/menu.ogg", "assets/music/other.ogg"]);
    let all = bundle.asset_paths();
    assert!(all.contains(&"assets/music/menu.ogg".to_owned()), "export packs the music");
    let missing = bundle.missing_music_issues(&|p| p == "assets/music/menu.ogg");
    assert_eq!(missing.len(), 1);
    assert!(missing[0].message.contains("other.ogg"));
    // A screen without a track is a warning, a Duck-only screen is fine.
    let mut quiet: ScreenAsset = from_ron(SLOTS_V8).unwrap();
    quiet.music = Some(ScreenMusic::default());
    assert!(orbit_ui_data::validate(&quiet, None).iter().any(|i| i.message.contains("no audio file")));
    quiet.music = Some(ScreenMusic {
        over_game: OverGame::Duck,
        ..ScreenMusic::default()
    });
    assert!(orbit_ui_data::validate(&quiet, None).is_empty());
}
