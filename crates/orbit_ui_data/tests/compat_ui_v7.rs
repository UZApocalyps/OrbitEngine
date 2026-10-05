//! `ui_version` 7 (image states, visual style fields, styleable dialogue
//! box): frozen version 6 documents (`examples/fixtures/compat/ui_v6/`,
//! never regenerate) keep loading, mean the same, write none of the new
//! fields, and the dialogue box / choices of an old theme keep their exact
//! layout (layout parity with the pre-v7 formulas, copied here).

use orbit_ui_data::{
    Anchor, ChoiceTheme, Fill, ImageFit, NamePlate, Style, TextBoxTheme, Theme, UI_VERSION, UiBundle, WidgetKind,
    WidgetState, from_ron, resolve_style, to_ron,
};

const THEME_V6: &str = include_str!("../../../examples/fixtures/compat/ui_v6/theme.ron");
const EXTRAS_V6: &str = include_str!("../../../examples/fixtures/compat/ui_v6/extras.screen.ron");

/// The textbox layout as `orbit_ui::DialogueBox::draw` computed it before
/// `ui_version` 7: `(panel, name origin, text origin, wrap width,
/// indicator)`.
fn legacy_textbox(t: &TextBoxTheme) -> ([f32; 4], [f32; 2], [f32; 2], f32, [f32; 4]) {
    let [x, y, w, h] = t.rect;
    let s = t.indicator_size;
    (
        [x, y, w, h],
        [x + t.padding[0], y + t.padding[1]],
        [x + t.padding[0], y + t.padding[1] + t.text_offset],
        t.rect[2] - 2.0 * t.padding[0],
        [x + w - t.padding[0] - s, y + h - t.padding[1] - s, s, s],
    )
}

/// `orbit_ui::button_rects` before `ui_version` 7.
fn legacy_choice_rects(theme: &ChoiceTheme, count: usize) -> Vec<[f32; 4]> {
    if count == 0 {
        return Vec::new();
    }
    let [w, h] = theme.button_size;
    let n = count as f32;
    let full = n * h + (n - 1.0) * theme.spacing;
    let avail = 1080.0 - 2.0 * 60.0;
    let k = if full > avail { avail / full } else { 1.0 };
    let (h, gap) = (h * k, theme.spacing * k);
    let total = n * h + (n - 1.0) * gap;
    let top = (1080.0 - total) * 0.5;
    let x = (1920.0 - w) * 0.5;
    (0..count).map(|i| [x, top + i as f32 * (h + gap), w, h]).collect()
}

/// What a widget background looked like before `ui_version` 7: the
/// nine-slice when the style had one, else `Style::background`'s color,
/// else the theme's default.
#[derive(Debug, PartialEq)]
enum Look {
    NineSlice(String),
    Color([u8; 4]),
    Default,
}

fn legacy_look(style: &Style, enabled: bool, pressed: bool, hot: bool) -> Look {
    if let Some((image, _)) = style.nine_slice() {
        return Look::NineSlice(image.to_owned());
    }
    style.background(enabled, pressed, hot).map_or(Look::Default, Look::Color)
}

fn new_look(style: &Style, state: WidgetState) -> Look {
    match style.fill_for(state) {
        Some(Fill::NineSlice { image, .. }) => Look::NineSlice(image),
        Some(Fill::Color(c)) => Look::Color(c),
        None => Look::Default,
        Some(other) => panic!("a v6 style cannot resolve to {other:?}"),
    }
}

#[test]
fn v6_theme_loads_unchanged_and_writes_no_new_field() {
    let theme = Theme::from_ron(THEME_V6).unwrap();
    assert_eq!(theme.textbox.style, None);
    assert_eq!(theme.textbox.anchor, None);
    assert!(theme.textbox.speaker_color);
    assert_eq!(theme.choice.button_style, None);
    assert!(theme.quick_menu.is_default());
    assert!(theme.styles.values().all(|s| s.fills.is_empty() && s.shadow.is_none()));
    let text = theme.to_ron().unwrap();
    for new_field in [
        "fills:", "shadow:", "text_align:", "bold:", "show_text:", "focused:", "quick_menu:", "name_plate:",
        "name_style:", "speaker_color:", "indicator_image:", "button_style:", "anchor:",
    ] {
        assert!(!text.contains(new_field), "{new_field} written for a v6 theme:\n{text}");
    }
    assert_eq!(Theme::from_ron(&text).unwrap(), theme);
    // No warning on a v6 theme (the nine-slice image is set, parents exist).
    assert!(theme.style_issues().is_empty(), "{:?}", theme.style_issues());
}

#[test]
fn v6_screens_load_as_the_current_version_with_the_same_meaning() {
    assert!(EXTRAS_V6.contains("ui_version: 6"));
    let screen = from_ron(EXTRAS_V6).unwrap();
    assert_eq!(screen.ui_version, UI_VERSION);
    assert!(UI_VERSION >= 8);
    let as_v7 = EXTRAS_V6.replace("ui_version: 6", &format!("ui_version: {UI_VERSION}"));
    assert_eq!(from_ron(&as_v7).unwrap(), screen);
    let text = to_ron(&screen).unwrap();
    assert!(!text.contains("action: []"), "an image without actions writes none");
    assert_eq!(from_ron(&text).unwrap(), screen);
    let theme = Theme::from_ron(THEME_V6).unwrap();
    assert!(orbit_ui_data::validate(&screen, Some(&theme)).is_empty());
    // A v6 image is not clickable.
    let mut clickable = 0;
    screen.root.walk(&mut |w| {
        if let WidgetKind::Image { action, .. } = &w.kind {
            clickable += usize::from(!action.is_empty());
        }
    });
    assert_eq!(clickable, 0);
}

const ACHIEVEMENTS_V7: &str = include_str!("../../../examples/fixtures/compat/ui_v7/achievements.screen.ron");

/// The previous version (7, achievements) loads as version 8 unchanged and
/// writes none of the version 8 fields.
#[test]
fn v7_screens_load_as_version_8_unchanged() {
    assert!(ACHIEVEMENTS_V7.contains("ui_version: 7"));
    let screen = from_ron(ACHIEVEMENTS_V7).unwrap();
    assert_eq!(screen.ui_version, UI_VERSION);
    let as_current = ACHIEVEMENTS_V7.replace("ui_version: 7", &format!("ui_version: {UI_VERSION}"));
    assert_eq!(from_ron(&as_current).unwrap(), screen);
    let text = to_ron(&screen).unwrap();
    for new_field in ["fills:", "shadow:", "show_text:", "bold:", "focused:"] {
        assert!(!text.contains(new_field), "{new_field}");
    }
    assert_eq!(from_ron(&text).unwrap(), screen);
}

/// Golden layout of the migrated textbox: an old theme (the default one and
/// the frozen v6 one) puts the panel, name, text and indicator exactly
/// where the pre-v7 drawing code did.
#[test]
fn migrated_textbox_and_choices_keep_their_layout() {
    for theme in [Theme::default(), Theme::from_ron(THEME_V6).unwrap()] {
        let t = &theme.textbox;
        let lay = t.layout(123.0);
        let (panel, name, text, width, indicator) = legacy_textbox(t);
        assert_eq!(lay.panel, panel);
        assert_eq!(lay.plate, None);
        assert_eq!(lay.name_origin, name);
        assert_eq!(lay.name_width, width);
        assert_eq!(lay.text_origin, text);
        assert_eq!(lay.text_width, width);
        assert_eq!(lay.indicator, indicator);
        for n in [0, 1, 2, 4, 13] {
            assert_eq!(theme.choice.button_rects(n), legacy_choice_rects(&theme.choice, n), "{n} choices");
        }
    }
    // Hard numbers for the frozen theme (a regression in both copies of the
    // formula would still be caught).
    let t = Theme::from_ron(THEME_V6).unwrap().textbox;
    let lay = t.layout(0.0);
    assert_eq!(lay.panel, [60.0, 720.0, 1800.0, 320.0]);
    assert_eq!(lay.name_origin, [124.0, 744.0]);
    assert_eq!(lay.text_origin, [124.0, 814.0]);
    assert_eq!(lay.indicator, [1770.0, 990.0, 26.0, 26.0]);
}

#[test]
fn anchored_textbox_name_plate_and_choices() {
    let mut t = TextBoxTheme {
        anchor: Some(Anchor::bottom_center(-20.0)),
        name_plate: Some(NamePlate {
            offset: [40.0, -60.0],
            size: [0.0, 64.0],
            style: None,
            padding: [20.0, 8.0],
        }),
        ..TextBoxTheme::default()
    };
    let lay = t.layout(200.0);
    // 1740 x 260 at the bottom center, 20 px above the edge.
    assert_eq!(lay.panel, [90.0, 800.0, 1740.0, 260.0]);
    assert_eq!(lay.plate, Some([130.0, 740.0, 240.0, 64.0]), "a 0 width fits the name");
    assert_eq!(lay.name_origin, [150.0, 748.0]);
    t.name_plate.as_mut().unwrap().size[0] = 300.0;
    assert_eq!(t.layout(200.0).plate.unwrap()[2], 300.0);
    let c = ChoiceTheme {
        anchor: Some(Anchor::new([1.0, 0.5], [1.0, 0.5], [-40.0, 0.0])),
        ..ChoiceTheme::default()
    };
    let rects = c.button_rects(2);
    assert_eq!(rects[0][0], 1920.0 - 40.0 - 1200.0);
    let total = 2.0 * 90.0 + 26.0;
    assert_eq!(rects[0][1], (1080.0 - total) * 0.5);
}

/// A style written before `ui_version` 7 resolves to exactly the background
/// it had in every state the widget code can be in.
#[test]
fn legacy_styles_resolve_to_the_same_background_in_every_state() {
    let theme = Theme::from_ron(THEME_V6).unwrap();
    let mut styles: Vec<Style> = ["card", "flat"]
        .iter()
        .map(|n| resolve_style(&theme.styles, n).unwrap())
        .collect();
    styles.push(Style::default());
    styles.push(Style {
        fill: Some(Fill::Color([1, 2, 3, 4])),
        ..Default::default()
    });
    for style in &styles {
        for enabled in [true, false] {
            for hover in [true, false] {
                for focused in [true, false] {
                    for pressed in [false, hover] {
                        let hot = hover || focused;
                        let old = legacy_look(style, enabled, pressed, hot);
                        let state = WidgetState::of(enabled, pressed, hover, focused);
                        assert_eq!(new_look(style, state), old, "{style:?} {state:?}");
                        // Text colors too.
                        assert_eq!(
                            style.text_color_for(state),
                            style.text_colors.pick(enabled, pressed, hot),
                            "{style:?} {state:?}"
                        );
                    }
                }
            }
        }
        // Converting the old fields to per-state fills keeps every state.
        let mut modern = style.clone();
        modern.modernize_background();
        assert!(modern.fill.is_none());
        for state in WidgetState::ALL {
            assert_eq!(modern.fill_for(state), style.fill_for(state), "{style:?} {state:?}");
        }
    }
}

#[test]
fn image_states_resolve_with_fallbacks_and_inheritance() {
    let img = |p: &str| Fill::Image {
        image: p.into(),
        fit: ImageFit::Stretch,
    };
    let mut styles = std::collections::BTreeMap::new();
    let mut base = Style::default();
    base.fills.normal = Some(img("b.png"));
    base.fills.hover = Some(img("b_hover.png"));
    base.fills.disabled = Some(Fill::Gradient {
        top: [0, 0, 0, 255],
        bottom: [9, 9, 9, 255],
    });
    base.show_text = Some(false);
    styles.insert("base".to_owned(), base);
    let mut child = Style {
        extends: Some("base".into()),
        ..Default::default()
    };
    child.fills.pressed = Some(Fill::NineSlice {
        image: "b_down.png".into(),
        insets: [4.0; 4],
    });
    child.colors.hover = Some([1, 1, 1, 1]);
    styles.insert("child".to_owned(), child);
    let s = resolve_style(&styles, "child").unwrap();
    assert_eq!(s.fill_for(WidgetState::Normal), Some(img("b.png")));
    assert_eq!(s.fill_for(WidgetState::Hover), Some(img("b_hover.png")), "a state fill wins over a state color");
    assert!(matches!(s.fill_for(WidgetState::Pressed), Some(Fill::NineSlice { .. })));
    assert_eq!(s.fill_for(WidgetState::Focused), Some(img("b_hover.png")), "focus falls back to hover");
    assert!(matches!(s.fill_for(WidgetState::Disabled), Some(Fill::Gradient { .. })));
    assert!(!s.text_visible());
    assert_eq!(s.image_paths(), ["b.png", "b_hover.png", "b_down.png"]);
    // Every image of the theme, once.
    let theme = Theme {
        styles,
        ..Theme::default()
    };
    assert_eq!(theme.image_paths(), ["b.png", "b_down.png", "b_hover.png"]);
    // Override fields are told apart from inherited ones; reset drops them.
    let mut own = theme.styles["child"].clone();
    assert!(own.sets("fills") && own.sets("colors") && !own.sets("show_text"));
    own.reset("fills");
    assert!(!own.sets("fills"));
}

#[test]
fn version_7_theme_and_screen_round_trip_and_validate() {
    let theme_src = r#"(
        styles: {
            "img": (fills: (normal: Some(Image(image: "assets/ui/b.png", fit: Cover)),
                            focused: Some(NineSlice(image: "assets/ui/f.png", insets: (6.0, 6.0, 6.0, 6.0)))),
                    shadow: Some((color: (0, 0, 0, 100), offset: (3.0, 5.0))), bold: Some(true),
                    text_align: Some(Right), text_colors: (focused: Some((1, 2, 3, 255)))),
            "broken": (extends: Some("nowhere"), fills: (hover: Some(Image(image: "")))),
        },
        textbox: (style: Some("img"), name_plate: Some((style: Some("ghost"))), speaker_color: false,
                  indicator_image: Some("assets/ui/arrow.png")),
        choice: (button_style: Some("img"), anchor: Some((canvas: (0.5, 1.0), pivot: (0.5, 1.0), offset: (0.0, -300.0)))),
        quick_menu: (button_style: Some("img"), hidden: true),
    )"#;
    let theme = Theme::from_ron(theme_src).unwrap();
    let back = Theme::from_ron(&theme.to_ron().unwrap()).unwrap();
    assert_eq!(back, theme);
    let issues = theme.style_issues();
    let has = |s: &str| issues.iter().any(|i| i.message.contains(s));
    assert!(has("unknown style `ghost`"), "{issues:?}");
    assert!(has("extends unknown style `nowhere`"), "{issues:?}");
    assert!(has("has no image"), "{issues:?}");
    assert!(theme.image_paths().contains(&"assets/ui/arrow.png".to_owned()));

    let screen_src = r#"(ui_version: 8, id: "m", root: (kind: Stack(dir: Vertical), children: [
        (id: "logo", kind: Image(asset: Some("assets/ui/logo.png"), action: [Open("m"), Close]), style: Some("img")),
        (kind: Button(text: Literal("Go")), overrides: (fills: (hover: Some(Color((9, 9, 9, 255)))), show_text: Some(false))),
    ]))"#;
    let screen = from_ron(screen_src).unwrap();
    assert!(orbit_ui_data::validate(&screen, Some(&theme)).is_empty());
    assert_eq!(from_ron(&to_ron(&screen).unwrap()).unwrap(), screen);
    let bundle = UiBundle {
        theme: Some(theme),
        screens: vec![screen],
        ..UiBundle::default()
    };
    assert!(bundle.image_paths().contains(&"assets/ui/logo.png".to_owned()));
    assert!(bundle.validate().iter().any(|i| i.message.contains("ghost")));
}
